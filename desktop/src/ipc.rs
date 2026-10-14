use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{State, WebviewWindow};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use twitch_drops_miner_core::{
    app::{
        AppError, Application, Command,
        api::{ChannelSelection, HistoryQuery},
        catalog,
        commands::{ChannelQuery, GameQuery},
        projection::StatePatch,
    },
    dto::Snapshot,
};

use crate::Desktop;

#[derive(Debug, Serialize)]
pub struct Error {
    pub status: u16,
    pub detail: String,
}

impl From<AppError> for Error {
    fn from(value: AppError) -> Self {
        let status = match value {
            AppError::Telegram(error) => {
                if error == twitch_drops_miner_core::notifications::Error::Conflict {
                    409
                } else {
                    400
                }
            }
            AppError::ShuttingDown | AppError::LoginRequired | AppError::SettingsConflict => 409,
            AppError::InvalidRequest
            | AppError::InvalidSettings
            | AppError::InvalidChannel
            | AppError::InvalidManualDuration => 400,
            AppError::ChannelNotFound => 404,
            AppError::Unavailable => 503,
        };
        Self {
            status,
            detail: value.to_string(),
        }
    }
}

pub fn local(window: &WebviewWindow) -> Result<(), Error> {
    if window.label() != "main" || !window.url().is_ok_and(|url| crate::local_url(&url)) {
        return Err(Error {
            status: 403,
            detail: "forbidden".into(),
        });
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Request {
    AuthStatus,
    Settings,
    SaveSettings(Value),
    Telegram,
    TelegramBot,
    SaveTelegram(twitch_drops_miner_core::notifications::Update),
    TestTelegram,
    SelectChannel(ChannelSelection),
    Games(GameQuery),
    SearchChannels(ChannelQuery),
    History(HistoryQuery),
    HistoryStats,
    VerifyProxy {
        proxy: String,
    },
    Reload,
    ClearCache,
    Logout,
    #[serde(rename = "confirm_oauth")]
    ConfirmOAuth,
    ExitManual,
}

impl Request {
    async fn run(self, app: Arc<Application>) -> Result<Value, AppError> {
        if app.shutdown.is_cancelled() {
            return Err(AppError::ShuttingDown);
        }
        match self {
            Self::AuthStatus => {
                let language = app.settings.read().await.language;
                return Ok(
                    json!({"enabled":false,"authenticated":true,"translations":catalog(language)["gui"]["auth"]}),
                );
            }
            Self::Telegram => return Ok(json!(app.telegram.view())),
            Self::TelegramBot => return Ok(json!(app.telegram_bot().await)),
            Self::SaveTelegram(update) => return Ok(json!(app.save_telegram(update).await?)),
            Self::TestTelegram => app.test_telegram().await?,
            Self::Settings => return Ok(json!(app.snapshot.read().await.settings)),
            Self::SaveSettings(patch) => {
                if patch.to_string().len() > 1024 * 1024 {
                    return Err(AppError::InvalidRequest);
                }
                return Ok(json!({"success":true,"settings":app.update_settings(patch).await?}));
            }
            Self::SelectChannel(selection) => app.select_channel(selection).await?,
            Self::Games(query) => return Ok(json!(app.games(query).await?)),
            Self::SearchChannels(query) => return Ok(json!(app.search_channels(query).await?)),
            Self::History(query) => return Ok(app.history(query).await),
            Self::HistoryStats => return Ok(app.history.lock().await.stats()),
            Self::VerifyProxy { proxy } => return app.verify_proxy(&proxy).await,
            Self::Reload => app.refresh_inventory().await?,
            Self::ClearCache => app.clear_cache().await?,
            Self::Logout => app.command(Command::Logout).await?,
            Self::ConfirmOAuth => app.command(Command::ConfirmOAuth).await?,
            Self::ExitManual => app.command(Command::ExitManual).await?,
        }
        Ok(json!({"success":true}))
    }
}

#[tauri::command]
pub async fn app_request(
    window: WebviewWindow,
    state: State<'_, Desktop>,
    request: Request,
    id: String,
) -> Result<Value, Error> {
    local(&window)?;
    if id.is_empty() || id.len() > 64 {
        return Err(AppError::InvalidRequest.into());
    }
    let _task = state.tasks.token();
    let _permit = state
        .requests
        .try_acquire()
        .map_err(|_| AppError::Unavailable)?;
    let cancel = state.cancel.child_token();
    {
        let mut requests = state.pending.lock().await;
        if requests.contains_key(&id) {
            return Err(AppError::InvalidRequest.into());
        }
        requests.insert(id.clone(), cancel.clone());
    }
    let result = async {
        let app = state.application().await?;
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(AppError::ShuttingDown.into()),
            value = request.run(app) => value.map_err(Error::from),
        }
    }
    .await;
    state.pending.lock().await.remove(&id);
    result
}

#[tauri::command]
pub async fn cancel_request(
    window: WebviewWindow,
    state: State<'_, Desktop>,
    id: String,
) -> Result<(), Error> {
    local(&window)?;
    if let Some(cancel) = state.pending.lock().await.get(&id) {
        cancel.cancel();
    }
    Ok(())
}

pub struct Subscription {
    id: String,
    cancel: CancellationToken,
    previous: Mutex<Snapshot>,
}

#[derive(Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Publication {
    Snapshot(Box<Snapshot>),
    Patch(Box<StatePatch>),
}

#[tauri::command]
pub async fn state_open(
    window: WebviewWindow,
    state: State<'_, Desktop>,
    id: String,
) -> Result<Publication, Error> {
    local(&window)?;
    if id.len() > 64 || id.is_empty() {
        return Err(AppError::InvalidRequest.into());
    }
    let app = state.application().await?;
    let snapshot = app.snapshot.read().await.clone();
    let mut slot = state.subscription.lock().await;
    if let Some(previous) = slot.take() {
        previous.cancel.cancel();
    }
    *slot = Some(Arc::new(Subscription {
        id,
        cancel: app.shutdown.child_token(),
        previous: Mutex::new(snapshot.clone()),
    }));
    Ok(Publication::Snapshot(Box::new(snapshot)))
}

#[tauri::command]
pub async fn state_close(
    window: WebviewWindow,
    state: State<'_, Desktop>,
    id: String,
) -> Result<(), Error> {
    local(&window)?;
    let mut slot = state.subscription.lock().await;
    if slot.as_ref().is_some_and(|value| value.id == id)
        && let Some(value) = slot.take()
    {
        value.cancel.cancel();
    }
    Ok(())
}

#[tauri::command]
pub async fn state_next(
    window: WebviewWindow,
    state: State<'_, Desktop>,
    id: String,
    resync: bool,
) -> Result<Publication, Error> {
    local(&window)?;
    let app = state.application().await?;
    let subscription = state
        .subscription
        .lock()
        .await
        .as_ref()
        .filter(|value| value.id == id)
        .cloned()
        .ok_or(AppError::Unavailable)?;
    subscription.next(&app, resync).await
}

impl Subscription {
    async fn next(&self, app: &Application, resync: bool) -> Result<Publication, Error> {
        let mut previous = self
            .previous
            .try_lock()
            .map_err(|_| AppError::Unavailable)?;
        let mut changes = app.snapshot.subscribe();
        loop {
            if self.cancel.is_cancelled() {
                return Err(AppError::Unavailable.into());
            }
            let next = app.snapshot.read().await.clone();
            if resync || next.revision > previous.revision {
                let publication = if resync {
                    Publication::Snapshot(Box::new(next.clone()))
                } else {
                    Publication::Patch(Box::new(StatePatch::between(&previous, &next)))
                };
                *previous = next;
                return Ok(publication);
            }
            tokio::select! {
                biased;
                _ = self.cancel.cancelled() => return Err(AppError::Unavailable.into()),
                changed = changes.changed() => { changed.map_err(|_| AppError::Unavailable)?; }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn native_requests_share_validation_and_shutdown() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _commands) = Application::open(dir.path().into()).unwrap();
        assert!(serde_json::from_value::<Request>(json!({"kind":"auth_settings"})).is_err());
        assert!(serde_json::from_value::<Request>(json!({"kind":"logout","extra":true})).is_err());
        for kind in ["games", "search_channels"] {
            let request =
                serde_json::from_value::<Request>(json!({"kind":kind,"value":{"search":""}}))
                    .unwrap();
            assert!(matches!(
                request.run(app.clone()).await,
                Err(AppError::InvalidRequest)
            ));
        }
        assert!(matches!(
            Request::SearchChannels(ChannelQuery {
                search: "streamer".into()
            })
            .run(app.clone())
            .await,
            Err(AppError::Unavailable)
        ));
        let telegram = Request::Telegram.run(app.clone()).await.unwrap();
        assert_eq!(telegram["token_configured"], false);
        let bot = Request::TelegramBot.run(app.clone()).await.unwrap();
        assert_eq!(bot["revision"], telegram["revision"]);
        assert!(bot["bot"].is_null());
        let update = serde_json::from_value(json!({"revision":telegram["revision"],"enabled":true,"chat_id":"42","token":"123:private_token"})).unwrap();
        let saved = Request::SaveTelegram(update)
            .run(app.clone())
            .await
            .unwrap();
        assert_eq!(saved["token_configured"], true);
        assert!(!saved.to_string().contains("private_token"));
        let auth = Request::AuthStatus.run(app.clone()).await.unwrap();
        assert_eq!(auth["authenticated"], true);
        app.shutdown.cancel();
        assert!(matches!(
            Request::Settings.run(app).await,
            Err(AppError::ShuttingDown)
        ));
    }

    #[tokio::test]
    async fn native_subscription_coalesces_and_cancels() {
        let dir = tempfile::tempdir().unwrap();
        let (app, _commands) = Application::open(dir.path().into()).unwrap();
        let subscription = Subscription {
            id: "test".into(),
            cancel: app.shutdown.child_token(),
            previous: Mutex::new(app.snapshot.read().await.clone()),
        };
        app.snapshot.write().await.status = "first".into();
        app.snapshot.write().await.status = "second".into();
        let Publication::Patch(patch) = subscription.next(&app, false).await.unwrap() else {
            panic!()
        };
        assert_eq!(patch.base_revision, 1);
        assert_eq!(patch.revision, 3);
        assert!(matches!(
            subscription.next(&app, true).await.unwrap(),
            Publication::Snapshot(_)
        ));
        app.shutdown.cancel();
        assert!(subscription.next(&app, false).await.is_err());
    }
}

use std::{sync::Arc, time::Duration};

use chrono::{NaiveDate, NaiveDateTime};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    AppError, Application, Command,
    commands::{ChannelQuery, DirectoryRequest, GameQuery},
};
use crate::{config::GameMetadata, dto::SettingsView, store::HistoryFilter};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelSelection {
    pub channel_id: Option<u64>,
    pub channel: Option<String>,
    pub duration_minutes: Option<u32>,
}

#[derive(Deserialize, Default)]
pub struct HistoryQuery {
    pub game: Option<String>,
    pub campaign_id: Option<String>,
    pub since: Option<String>,
    pub limit: Option<usize>,
}

impl HistoryQuery {
    pub fn filter(self) -> HistoryFilter {
        let since = self.since.and_then(|s| {
            s.parse()
                .ok()
                .or_else(|| {
                    NaiveDate::parse_from_str(&s, "%Y-%m-%d")
                        .ok()
                        .and_then(|d| d.and_hms_opt(0, 0, 0))
                        .map(|d| d.and_utc())
                })
                .or_else(|| {
                    NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%S")
                        .ok()
                        .map(|d| d.and_utc())
                })
        });
        HistoryFilter {
            game: self.game.filter(|s| !s.is_empty()),
            campaign_id: self.campaign_id,
            since,
            limit: self.limit.filter(|n| *n > 0).map(|n| n.min(5000)),
        }
    }
}

impl Application {
    pub async fn games(&self, query: GameQuery) -> Result<Vec<GameMetadata>, AppError> {
        if !query.valid() {
            return Err(AppError::InvalidRequest);
        }
        let sender = self
            .directory_queries
            .read()
            .await
            .clone()
            .ok_or(AppError::Unavailable)?;
        let (complete, result) = tokio::sync::oneshot::channel();
        sender
            .try_send(DirectoryRequest::Games { query, complete })
            .map_err(|_| AppError::Unavailable)?;
        tokio::select! {
            biased;
            _ = self.shutdown.cancelled() => Err(AppError::ShuttingDown),
            result = tokio::time::timeout(Duration::from_secs(12), result) => {
                result.map_err(|_| AppError::Unavailable)?
                    .map_err(|_| AppError::Unavailable)?
                    .map_err(|_| AppError::Unavailable)
            }
        }
    }

    pub async fn search_channels(
        &self,
        query: ChannelQuery,
    ) -> Result<Vec<crate::dto::ChannelSearchResult>, AppError> {
        if !query.valid() {
            return Err(AppError::InvalidRequest);
        }
        let sender = self
            .directory_queries
            .read()
            .await
            .clone()
            .ok_or(AppError::Unavailable)?;
        let (complete, result) = tokio::sync::oneshot::channel();
        sender
            .try_send(DirectoryRequest::Channels { query, complete })
            .map_err(|_| AppError::Unavailable)?;
        tokio::select! {
            biased;
            _ = self.shutdown.cancelled() => Err(AppError::ShuttingDown),
            result = tokio::time::timeout(Duration::from_secs(12), result) => {
                result.map_err(|_| AppError::Unavailable)?
                    .map_err(|_| AppError::Unavailable)?
                    .map_err(|_| AppError::Unavailable)
            }
        }
    }

    pub async fn update_settings(self: &Arc<Self>, patch: Value) -> Result<SettingsView, AppError> {
        let permit = self
            .settings_slot
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| AppError::Unavailable)?;
        let owned = self.clone();
        // Accepted writes outlive their caller; shutdown drains them before releasing storage.
        self.writes
            .spawn(async move {
                if owned.shutdown.is_cancelled() {
                    return Err(AppError::ShuttingDown);
                }
                let result = owned
                    .change_settings(|current| {
                        if patch.get("revision").is_some_and(|v| {
                            !v.is_null() && v.as_str() != Some(current.revision.as_str())
                        }) {
                            return Err(AppError::SettingsConflict);
                        }
                        current
                            .values
                            .patched(&patch)
                            .map_err(|_| AppError::InvalidSettings)
                    })
                    .await;
                // Reconfiguration can itself append a selected game.
                drop(permit);
                if result.is_ok() {
                    owned.command(Command::SettingsChanged).await?;
                }
                result
            })
            .await
            .map_err(|_| AppError::Unavailable)?
    }

    pub async fn select_channel(&self, selection: ChannelSelection) -> Result<(), AppError> {
        if self.snapshot.read().await.login.user_id.is_none() {
            return Err(AppError::LoginRequired);
        }
        if selection
            .duration_minutes
            .is_some_and(|m| !(1..=1440).contains(&m))
        {
            return Err(AppError::InvalidManualDuration);
        }
        let duration = selection
            .duration_minutes
            .map(|m| Duration::from_secs(u64::from(m) * 60));
        if selection.channel.is_some() == selection.channel_id.is_some() {
            return Err(AppError::InvalidRequest);
        }
        if let Some(channel) = selection.channel {
            let login =
                crate::twitch::channels::channel_login(&channel).ok_or(AppError::InvalidChannel)?;
            return self.command(Command::MineChannel(login, duration)).await;
        }
        let channel_id = selection.channel_id.ok_or(AppError::InvalidRequest)?;
        if !self
            .snapshot
            .read()
            .await
            .channels
            .iter()
            .any(|c| c.id == channel_id)
        {
            return Err(AppError::ChannelNotFound);
        }
        self.command(Command::SelectChannel(channel_id, duration))
            .await
    }

    pub async fn history(&self, query: HistoryQuery) -> Value {
        let history = self.history.lock().await;
        let state = self.snapshot.read().await;
        json!({"total":history.total(),"entries":history.entries(&query.filter()),"instance":state.instance,"revision":state.history_revision,"clear_revision":state.history_clear_revision})
    }

    pub fn has_pending_writes(&self) -> bool {
        !self.writes.is_empty()
    }

    pub async fn verify_proxy(&self, proxy: &str) -> Result<Value, AppError> {
        crate::config::validate_proxy(proxy).map_err(|_| AppError::InvalidRequest)?;
        if proxy.is_empty() {
            return Ok(
                json!({"success":false,"message":self.text("gui.backend.proxy_empty",&[]).await}),
            );
        }
        let started = std::time::Instant::now();
        let client = reqwest::Client::builder()
            .no_proxy()
            .proxy(reqwest::Proxy::all(proxy).map_err(|_| AppError::InvalidRequest)?)
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| AppError::Unavailable)?;
        let response = tokio::select! {
            biased;
            _ = self.shutdown.cancelled() => return Err(AppError::ShuttingDown),
            response = client.get("https://www.twitch.tv").send() => response,
        };
        Ok(match response {
            Ok(response) if response.status().as_u16() < 500 => {
                json!({"success":true,"latency":started.elapsed().as_millis(),"message":self.text("gui.backend.proxy_connected",&[]).await})
            }
            _ => json!({"success":false,"message":self.text("gui.backend.proxy_failed",&[]).await}),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn game_queries_validate_and_use_the_owned_network_queue() {
        let directory = tempfile::tempdir().unwrap();
        let (app, _commands) = Application::open(directory.path().into()).unwrap();
        assert!(matches!(
            app.games(GameQuery::Search("".into())).await,
            Err(AppError::InvalidRequest)
        ));
        assert!(matches!(
            app.games(GameQuery::Search("Rust".into())).await,
            Err(AppError::Unavailable)
        ));
        let (sender, mut receiver) = tokio::sync::mpsc::channel(4);
        *app.directory_queries.write().await = Some(sender);
        let reply = async {
            let DirectoryRequest::Games { query, complete } = receiver.recv().await.unwrap() else {
                panic!("unexpected directory request")
            };
            assert!(matches!(query, GameQuery::Search(ref name) if name == "Rust"));
            complete.send(Ok(vec![])).unwrap();
        };
        let (result, _) = tokio::join!(app.games(GameQuery::Search("Rust".into())), reply);
        assert!(result.unwrap().is_empty());
    }
    #[tokio::test]
    async fn channel_search_uses_the_owned_queue_and_cancels_when_caller_leaves() {
        let directory = tempfile::tempdir().unwrap();
        let (app, _commands) = Application::open(directory.path().into()).unwrap();
        assert!(matches!(
            app.search_channels(ChannelQuery { search: " ".into() })
                .await,
            Err(AppError::InvalidRequest)
        ));
        let (sender, mut receiver) = tokio::sync::mpsc::channel(4);
        *app.directory_queries.write().await = Some(sender);
        let caller = app.clone();
        let read = tokio::spawn(async move {
            caller
                .search_channels(ChannelQuery {
                    search: "streamer".into(),
                })
                .await
        });
        let DirectoryRequest::Channels {
            query,
            mut complete,
        } = receiver.recv().await.unwrap()
        else {
            panic!("unexpected directory request")
        };
        assert_eq!(query.search, "streamer");
        read.abort();
        let _ = read.await;
        tokio::time::timeout(Duration::from_secs(1), complete.closed())
            .await
            .unwrap();
    }
}

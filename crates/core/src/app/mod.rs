pub mod activity;
pub mod api;
pub mod commands;
pub mod projection;
pub mod state;

use crate::{
    config::{Language, Settings},
    dto::{RefreshState, SettingsView, Snapshot},
    random_hex,
    store::{CampaignArchive, DataDirectory, History},
};
use anyhow::Result;
use chrono::Utc;
pub use commands::{AppError, Command, CommandRequest};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use state::PublishedState;
use std::{
    path::PathBuf,
    sync::{Arc, LazyLock},
};
use tokio::sync::{Mutex, RwLock, Semaphore, broadcast, mpsc, oneshot};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

pub static ENGLISH: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../../lang/English.json"))
        .expect("valid English catalog")
});

pub static RUSSIAN: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../../lang/Russian.json"))
        .expect("valid Russian catalog")
});

pub fn catalog(language: Language) -> &'static Value {
    match language {
        Language::En => &ENGLISH,
        Language::Ru => &RUSSIAN,
    }
}

pub fn message_in(language: Language, path: &str, replacements: &[(&str, &str)]) -> String {
    let mut value = catalog(language);
    for key in path.split('.') {
        value = &value[key];
    }
    let mut result = value.as_str().unwrap_or(path).to_owned();
    for (key, value) in replacements {
        result = result.replace(&format!("{{{key}}}"), value);
    }
    result
}

pub fn message(path: &str, replacements: &[(&str, &str)]) -> String {
    message_in(Language::En, path, replacements)
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Notification {
    pub title: String,
    pub message: String,
    #[serde(skip)]
    pub claim: Option<crate::notifications::Claim>,
}

pub struct Application {
    pub data: Arc<DataDirectory>,
    pub settings: RwLock<Settings>,
    pub snapshot: PublishedState,
    pub history: Mutex<History>,
    pub archive: Mutex<CampaignArchive>,
    pub shutdown: CancellationToken,
    pub(crate) writes: TaskTracker,
    pub(crate) settings_slot: Arc<Semaphore>,
    pub notifications: broadcast::Sender<Notification>,
    pub telegram: crate::notifications::Telegram,
    pub(crate) directory_queries: RwLock<Option<mpsc::Sender<commands::DirectoryRequest>>>,
    commands: mpsc::Sender<CommandRequest>,
}

impl Application {
    pub fn open(directory: PathBuf) -> Result<(Arc<Self>, mpsc::Receiver<CommandRequest>)> {
        let data = Arc::new(DataDirectory::open(directory)?);
        let settings = data.settings()?;
        let telegram = crate::notifications::Telegram::open(&data.path)?;
        let (commands, receiver) = mpsc::channel(64);
        let history = History::load(&data.path);
        let archive = CampaignArchive::load(&data.path);
        let mut snapshot = Snapshot {
            mining: crate::dto::MiningStatus {
                state: crate::dto::MiningState::AccountRequired,
                ..Default::default()
            },
            settings: SettingsView {
                values: settings.clone(),
                revision: random_hex::<16>()?,
                games_available: vec![],
                game_keys: Default::default(),
            },
            campaigns: archive.merge(vec![], Utc::now()),
            ..Snapshot::default()
        };
        snapshot.settings.refresh_game_keys();
        Ok((
            Arc::new(Self {
                data,
                settings: RwLock::new(settings),
                snapshot: PublishedState::new(snapshot)?,
                history: Mutex::new(history),
                archive: Mutex::new(archive),
                shutdown: CancellationToken::new(),
                writes: TaskTracker::new(),
                settings_slot: Arc::new(Semaphore::new(1)),
                notifications: broadcast::channel(32).0,
                telegram,
                directory_queries: RwLock::new(None),
                commands,
            }),
            receiver,
        ))
    }

    pub async fn language(&self) -> Language {
        self.settings.read().await.language
    }

    pub async fn text(&self, code: &str, args: &[(&str, &str)]) -> String {
        message_in(self.language().await, code, args)
    }

    pub async fn command(&self, command: Command) -> Result<(), AppError> {
        if self.shutdown.is_cancelled() {
            return Err(AppError::ShuttingDown);
        }
        let (complete, result) = oneshot::channel();
        self.commands
            .send(CommandRequest { command, complete })
            .await
            .map_err(|_| AppError::Unavailable)?;
        result
            .await
            .map_err(|_| AppError::Unavailable)?
            .map_err(|_| AppError::Unavailable)
    }

    // Repeated refresh requests join the current work, including work queued by the UI.
    pub async fn begin_inventory_refresh(&self) -> (u64, bool) {
        let refresh = {
            let mut snapshot = self.snapshot.write().await;
            let refresh = &mut snapshot.inventory_refresh;
            if refresh.state == RefreshState::Refreshing {
                return (refresh.sequence, false);
            }
            refresh.sequence += 1;
            refresh.state = RefreshState::Refreshing;
            refresh.error = None;
            refresh.clone()
        };
        (refresh.sequence, true)
    }

    pub async fn finish_inventory_refresh(&self, sequence: u64, error: Option<String>) {
        {
            let mut snapshot = self.snapshot.write().await;
            let refresh = &mut snapshot.inventory_refresh;
            if refresh.sequence != sequence || refresh.state != RefreshState::Refreshing {
                return;
            }
            refresh.sequence += 1;
            refresh.state = if error.is_some() {
                RefreshState::Failed
            } else {
                RefreshState::Refreshed
            };
            refresh.error = error;
        };
    }

    pub async fn refresh_inventory(self: &Arc<Self>) -> Result<(), AppError> {
        // Keep accepted work owned if the browser disconnects before the acknowledgement.
        let owned = self.clone();
        self.writes
            .spawn(async move {
                if owned.snapshot.read().await.login.user_id.is_none() {
                    return Err(AppError::LoginRequired);
                }
                let (sequence, started) = owned.begin_inventory_refresh().await;
                if started {
                    let result = owned.command(Command::Refresh { clear_cache: false }).await;
                    if result.is_err() {
                        owned
                            .finish_inventory_refresh(
                                sequence,
                                Some(owned.text("gui.auth.request_failed", &[]).await),
                            )
                            .await;
                    }
                    result?;
                }
                Ok(())
            })
            .await
            .map_err(|_| AppError::Unavailable)?
    }

    pub async fn clear_cache(self: &Arc<Self>) -> Result<(), AppError> {
        let owned = self.clone();
        self.writes
            .spawn(async move {
                if owned.shutdown.is_cancelled() {
                    return Err(AppError::ShuttingDown);
                }
                // Persist cleared IDs before refreshing, so imports and late claim receipts
                // cannot resurrect the removed history. Drain accepted work on disconnect.
                let mut history = owned.history.lock().await;
                history.clear().map_err(|_| AppError::Unavailable)?;
                let mut state = owned.snapshot.write().await;
                state.history_revision += 1;
                state.history_clear_revision += 1;
                drop(state);
                drop(history);
                owned.command(Command::Refresh { clear_cache: true }).await
            })
            .await
            .map_err(|_| AppError::Unavailable)?
    }

    pub async fn activity(&self, code: &str, args: &[(&str, &str)]) {
        use activity::{ActivityEvent, Category, Severity};
        let (category, severity) = match code {
            "gui.backend.twitch_error" => (Category::Connection, Severity::Warning),
            "gui.backend.worker_failed" | "gui.backend.session_storage" => {
                (Category::Account, Severity::Error)
            }
            "status.claimed_drop" => (Category::Claims, Severity::Info),
            code if code.starts_with("login.") => (Category::Account, Severity::Info),
            "status.catalog_unavailable" => (Category::Inventory, Severity::Warning),
            _ => (Category::Mining, Severity::Info),
        };
        let mut event = ActivityEvent::new_in(
            self.settings.read().await.language,
            code,
            category,
            severity,
            args,
        );
        if code == "status.catalog_unavailable" {
            event.args.insert("operation".into(), "inventory".into());
        }
        self.record_activity(event).await;
    }

    pub async fn console(&self, text: String) {
        self.record_activity(activity::ActivityEvent::message(text))
            .await;
    }

    pub async fn record_activity(&self, event: activity::ActivityEvent) {
        let text = event.message.clone();
        let inserted = {
            let mut state = self.snapshot.write().await;
            activity::record(&mut state, event)
        };
        if inserted {
            tracing::info!("{text}");
        }
    }

    pub async fn recover_activity(&self, scope: &activity::ActivityEvent) {
        let mut state = self.snapshot.write().await;
        activity::recover(&mut state, scope);
    }

    pub fn import_history(&self, entries: Vec<crate::dto::HistoryEntry>) -> Result<usize> {
        // Keep the history and its version indivisible to HTTP readers.
        let mut history = self.history.blocking_lock();
        let imported = history.import_claims(entries)?;
        if imported > 0 {
            self.snapshot.blocking_write().history_revision += 1;
        }
        Ok(imported)
    }

    pub async fn status(&self, text: String) {
        self.snapshot.write().await.status = text;
    }

    pub fn notify_claimed(
        &self,
        language: Language,
        account: u64,
        campaign: &crate::domain::Campaign,
        drop: &crate::domain::Drop,
    ) {
        let _ = self.notifications.send(Notification {
            title: message_in(language, "gui.backend.drop_claimed", &[]),
            message: drop.name.clone(),
            claim: Some(crate::notifications::Claim {
                account,
                drop_id: drop.id.clone(),
                game: campaign.game.name.clone(),
                campaign: campaign.name.clone(),
                reward: drop.name.clone(),
                image_url: drop.image_url(),
                epoch: self.telegram.epoch(),
            }),
        });
    }

    pub async fn drain_writes(&self) {
        self.writes.close();
        self.writes.wait().await;
    }

    pub(crate) async fn change_settings(
        &self,
        update: impl FnOnce(&SettingsView) -> Result<crate::config::Settings, AppError>,
    ) -> Result<SettingsView, AppError> {
        let current = self.snapshot.read().await.settings.clone();
        let next = update(&current)?;
        let data = self.data.clone();
        let saved = next.clone();
        let revision = random_hex::<16>().map_err(|_| AppError::Unavailable)?;
        tokio::task::spawn_blocking(move || data.save_settings(&saved))
            .await
            .map_err(|_| AppError::Unavailable)?
            .map_err(|_| AppError::Unavailable)?;
        *self.settings.write().await = next.clone();
        let mut state = self.snapshot.write().await;
        if current.values.language != next.language {
            // Retranslate stored activity so the log follows the saved language.
            // Free-text console entries keep their original wording.
            for event in &mut state.activity {
                if event.code == "status.message" {
                    continue;
                }
                let code = event.code.clone();
                let args: Vec<(&str, &str)> = event
                    .args
                    .iter()
                    .map(|(key, value)| (key.as_str(), value.as_str()))
                    .collect();
                event.message = message_in(next.language, &code, &args);
            }
        }
        state.settings.values = next;
        state.settings.revision = revision;
        state.settings.refresh_game_keys();
        Ok(state.settings.clone())
    }
}

use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tokio::{
    sync::{Semaphore, broadcast},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

use crate::{
    app::{
        Application, Notification,
        activity::{ActivityEvent, Category, Severity},
    },
    random_hex,
    store::{atomic_json, read_json},
};

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Config {
    enabled: bool,
    token: String,
    chat_id: String,
}

#[derive(Clone, Serialize)]
pub struct View {
    pub enabled: bool,
    pub token_configured: bool,
    pub chat_id: String,
    pub revision: String,
    pub unreadable: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Update {
    pub revision: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub chat_id: String,
    #[serde(default)]
    pub remove: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("telegram_invalid")]
    Invalid,
    #[error("settings_conflict")]
    Conflict,
    #[error("telegram_storage")]
    Storage,
    #[error("telegram_disabled")]
    Disabled,
    #[error("telegram_busy")]
    Busy,
    #[error("telegram_token")]
    Token,
    #[error("telegram_chat")]
    Chat,
    #[error("telegram_failed")]
    Delivery,
    #[error("telegram_cancelled")]
    Cancelled,
}

impl Error {
    pub fn code(self) -> &'static str {
        match self {
            Self::Invalid => "telegram_invalid",
            Self::Conflict => "settings_conflict",
            Self::Storage => "telegram_storage",
            Self::Disabled => "telegram_disabled",
            Self::Busy => "telegram_busy",
            Self::Token => "telegram_token",
            Self::Chat => "telegram_chat",
            Self::Delivery => "telegram_failed",
            Self::Cancelled => "telegram_cancelled",
        }
    }
}

#[derive(Clone)]
pub struct Claim {
    pub account: u64,
    pub drop_id: String,
    pub game: String,
    pub campaign: String,
    pub reward: String,
    pub epoch: u64,
}

struct State {
    config: Config,
    revision: String,
    unreadable: bool,
    epoch: u64,
    cancel: CancellationToken,
    next_send: Instant,
}

pub struct Telegram {
    path: PathBuf,
    state: Mutex<State>,
    writes: Arc<Semaphore>,
    delivery: Semaphore,
    client: Mutex<Option<(String, reqwest::Client)>>,
    endpoint: String,
    failed: AtomicBool,
}

impl Config {
    fn valid(&self) -> bool {
        let token = self.token.is_empty()
            || self.token.split_once(':').is_some_and(|(id, secret)| {
                !id.is_empty()
                    && id.bytes().all(|c| c.is_ascii_digit())
                    && !secret.is_empty()
                    && secret
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
            });
        let chat = self.chat_id.is_empty() || self.chat_id.parse::<i64>().is_ok_and(|id| id != 0);
        self.token.len() <= 256
            && self.chat_id.len() <= 24
            && token
            && chat
            && (!self.enabled || (!self.token.is_empty() && !self.chat_id.is_empty()))
    }
}

impl Telegram {
    pub fn open(directory: &std::path::Path) -> anyhow::Result<Self> {
        let path = directory.join("telegram.json");
        let loaded = read_json::<Config>(&path);
        let (config, unreadable) = match loaded {
            Ok(Some(config)) if config.valid() => (config, false),
            Ok(None) => (Config::default(), false),
            _ => (Config::default(), true),
        };
        Ok(Self {
            path,
            state: Mutex::new(State {
                config,
                revision: random_hex::<16>()?,
                unreadable,
                epoch: 0,
                cancel: CancellationToken::new(),
                next_send: Instant::now(),
            }),
            writes: Arc::new(Semaphore::new(1)),
            delivery: Semaphore::new(1),
            client: Mutex::new(None),
            endpoint: "https://api.telegram.org".into(),
            failed: AtomicBool::new(false),
        })
    }

    pub fn view(&self) -> View {
        let state = self.state.lock().unwrap();
        View {
            enabled: state.config.enabled,
            token_configured: !state.config.token.is_empty(),
            chat_id: state.config.chat_id.clone(),
            revision: state.revision.clone(),
            unreadable: state.unreadable,
        }
    }

    pub fn epoch(&self) -> u64 {
        self.state.lock().unwrap().epoch
    }

    pub fn cancel_pending(&self) {
        let mut state = self.state.lock().unwrap();
        state.cancel.cancel();
        state.cancel = CancellationToken::new();
        state.epoch = state.epoch.wrapping_add(1);
    }

    fn save(&self, update: Update) -> Result<View, Error> {
        let state = self.state.lock().unwrap();
        if update.revision != state.revision {
            return Err(Error::Conflict);
        }
        if state.unreadable {
            return Err(Error::Storage);
        }
        let config = if update.remove {
            Config::default()
        } else {
            Config {
                enabled: update.enabled,
                token: if update.token.is_empty() {
                    state.config.token.clone()
                } else {
                    update.token
                },
                chat_id: update.chat_id.trim().to_owned(),
            }
        };
        if !config.valid() {
            return Err(Error::Invalid);
        }
        let revision = random_hex::<16>().map_err(|_| Error::Storage)?;
        drop(state);
        atomic_json(&self.path, &config).map_err(|_| Error::Storage)?;
        let mut state = self.state.lock().unwrap();
        state.cancel.cancel();
        state.cancel = CancellationToken::new();
        state.epoch = state.epoch.wrapping_add(1);
        state.config = config;
        state.revision = revision;
        drop(state);
        Ok(self.view())
    }

    fn client(&self, proxy: &str) -> Result<reqwest::Client, Error> {
        let mut cached = self.client.lock().unwrap();
        if let Some((saved, client)) = &*cached
            && saved == proxy
        {
            return Ok(client.clone());
        }
        let mut builder = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10));
        if !proxy.is_empty() {
            builder = builder.proxy(reqwest::Proxy::all(proxy).map_err(|_| Error::Delivery)?);
        }
        let client = builder.build().map_err(|_| Error::Delivery)?;
        *cached = Some((proxy.to_owned(), client.clone()));
        Ok(client)
    }

    async fn send(
        &self,
        epoch: u64,
        text: &str,
        proxy: &str,
        shutdown: &CancellationToken,
        test: bool,
    ) -> Result<(), Error> {
        let (config, cancel) = {
            let state = self.state.lock().unwrap();
            if state.epoch != epoch {
                return Err(Error::Cancelled);
            }
            if state.unreadable {
                return Err(Error::Storage);
            }
            if !state.config.enabled {
                return Err(Error::Disabled);
            }
            (state.config.clone(), state.cancel.clone())
        };
        let operation = async {
            let _permit = if test {
                self.delivery.try_acquire().map_err(|_| Error::Busy)?
            } else {
                self.delivery
                    .acquire()
                    .await
                    .map_err(|_| Error::Cancelled)?
            };
            let next = self.state.lock().unwrap().next_send;
            tokio::time::sleep_until(next).await;
            let client = self.client(proxy)?;
            for attempt in 0..2 {
                self.state.lock().unwrap().next_send = Instant::now() + Duration::from_secs(3);
                // The token is in the URL: never format reqwest errors or response bodies.
                let mut response = client.post(format!("{}/bot{}/sendMessage", self.endpoint, config.token))
                    .json(&serde_json::json!({"chat_id":config.chat_id, "text": text.chars().take(1500).collect::<String>(),
                        "link_preview_options":{"is_disabled":true}}))
                    .send().await.map_err(|_| Error::Delivery)?;
                let status = response.status().as_u16();
                if status == 401 {
                    return Err(Error::Token);
                }
                if matches!(status, 400 | 403) {
                    return Err(Error::Chat);
                }
                let mut body = Vec::new();
                while let Some(chunk) = response.chunk().await.map_err(|_| Error::Delivery)? {
                    if body.len() + chunk.len() > 65536 {
                        return Err(Error::Delivery);
                    }
                    body.extend_from_slice(&chunk);
                }
                let value: serde_json::Value =
                    serde_json::from_slice(&body).map_err(|_| Error::Delivery)?;
                if status == 200 && value["ok"] == true {
                    return Ok(());
                }
                if status == 429
                    && attempt == 0
                    && let Some(delay) = value["parameters"]["retry_after"].as_u64()
                    && (1..=30).contains(&delay)
                {
                    tokio::time::sleep(Duration::from_secs(delay)).await;
                    continue;
                }
                return Err(Error::Delivery);
            }
            Err(Error::Delivery)
        };
        tokio::select! { biased;
            _ = shutdown.cancelled() => Err(Error::Cancelled),
            _ = cancel.cancelled() => Err(Error::Cancelled),
            result = tokio::time::timeout(Duration::from_secs(55), operation) => result.unwrap_or(Err(Error::Delivery)),
        }
    }
}

impl Application {
    pub async fn save_telegram(
        self: &Arc<Self>,
        update: Update,
    ) -> Result<View, super::app::AppError> {
        let permit = self
            .telegram
            .writes
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| Error::Storage)?;
        let app = self.clone();
        self.writes
            .spawn(async move {
                if app.shutdown.is_cancelled() {
                    return Err(super::app::AppError::ShuttingDown);
                }
                tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    app.telegram.save(update)
                })
                .await
                .map_err(|_| Error::Storage)?
                .map_err(Into::into)
            })
            .await
            .map_err(|_| Error::Storage)?
    }

    pub async fn test_telegram(&self) -> Result<(), super::app::AppError> {
        let proxy = self.settings.read().await.proxy.clone();
        let result = self
            .telegram
            .send(
                self.telegram.epoch(),
                "Drops Miner\nTest message",
                &proxy,
                &self.shutdown,
                true,
            )
            .await;
        if result.is_ok() {
            report(self, result).await;
        }
        result.map_err(Into::into)
    }
}

pub async fn run(app: Arc<Application>, mut events: broadcast::Receiver<Notification>) {
    let mut seen = VecDeque::new();
    loop {
        let event = tokio::select! { biased;
            _ = app.shutdown.cancelled() => break,
            event = events.recv() => event,
        };
        let claim = match event {
            Ok(Notification {
                claim: Some(claim), ..
            }) => claim,
            Err(broadcast::error::RecvError::Closed) => break,
            _ => continue,
        };
        if claim.epoch != app.telegram.epoch()
            || !app.telegram.view().enabled
            || app.snapshot.read().await.login.user_id != Some(claim.account)
        {
            continue;
        }
        let key = (claim.account, claim.drop_id.clone());
        if seen.contains(&key) {
            continue;
        }
        seen.push_back(key);
        if seen.len() > 1024 {
            seen.pop_front();
        }
        let proxy = app.settings.read().await.proxy.clone();
        let text = format!(
            "Drop claimed\n{} — {}\nReward: {}",
            claim.game, claim.campaign, claim.reward
        );
        let result = app
            .telegram
            .send(claim.epoch, &text, &proxy, &app.shutdown, false)
            .await;
        report(&app, result).await;
    }
}

async fn report(app: &Application, result: Result<(), Error>) {
    let event = ActivityEvent::new(
        "gui.backend.telegram_delivery_failed",
        Category::Connection,
        Severity::Warning,
        &[("operation", "telegram")],
    );
    match result {
        Ok(()) => {
            app.recover_activity(&event).await;
            app.telegram.failed.store(false, Ordering::SeqCst);
        }
        Err(Error::Cancelled | Error::Disabled) => {}
        Err(_) if !app.telegram.failed.swap(true, Ordering::SeqCst) => {
            app.record_activity(event).await;
        }
        Err(_) => {}
    }
}

#[cfg(test)]
mod tests;

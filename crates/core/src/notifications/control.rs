use super::{Config, Error, MessageKind, html, response_json};
use crate::{
    app::{
        AppError, Application,
        activity::{ActivityEvent, Category, Severity},
        message_in,
    },
    config::Language,
    dto::{MiningState, Snapshot},
};
use chrono::Utc;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

struct Session {
    config: Config,
    epoch: u64,
    cancel: CancellationToken,
    started: i64,
}

#[derive(Debug, PartialEq, Eq)]
enum Failure {
    Cancelled,
    Unavailable,
    Oversized,
    Conflict,
    RateLimited(u64),
}

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Help,
    Status,
    Pause,
    Resume,
    Mine(String),
}

#[derive(Default)]
struct Cursor {
    offset: Option<i64>,
    last_date: i64,
}

impl Cursor {
    fn advance(&mut self, update: &Value, now: i64) -> bool {
        let id = update["update_id"].as_i64().unwrap();
        let date = update["message"]["date"]
            .as_i64()
            .filter(|date| *date <= now + 5);
        // Telegram chooses a new random ID after a week without updates.
        let idle_reset = date.is_some_and(|date| date.saturating_sub(self.last_date) >= 7 * 86400);
        if self.offset.is_some_and(|offset| id < offset) && !idle_reset {
            return false;
        }
        self.offset = Some(id + 1);
        self.last_date = self.last_date.max(date.unwrap_or(now));
        true
    }
}

fn text(language: Language, key: &str, args: &[(&str, &str)]) -> String {
    message_in(language, &format!("telegram.{key}"), args)
}

fn command(update: &Value, owner: i64, started: i64, now: i64) -> Option<Command> {
    let m = update.get("message")?;
    let date = m["date"].as_i64()?;
    if owner <= 0
        || m["chat"]["id"].as_i64() != Some(owner)
        || m["chat"]["type"] != "private"
        || m["from"]["id"].as_i64() != Some(owner)
        || m["from"]["is_bot"] != false
        || date < started
        || date > now + 5
        || date < now - 120
        || [
            "forward_origin",
            "forward_date",
            "forward_from",
            "sender_chat",
            "via_bot",
            "edit_date",
            "business_connection_id",
        ]
        .iter()
        .any(|key| m.get(key).is_some())
    {
        return None;
    }
    let input = m["text"].as_str()?;
    if input.len() > 256 || input.chars().any(char::is_control) {
        return None;
    }
    let (name, argument) = input.trim().split_once(' ').unwrap_or((input.trim(), ""));
    let argument = argument.trim();
    match (name, argument.is_empty()) {
        ("/start" | "/help", true) => Some(Command::Help),
        ("/status", true) => Some(Command::Status),
        ("/pause", true) => Some(Command::Pause),
        ("/resume", true) => Some(Command::Resume),
        ("/mine", false) if argument.len() <= 100 => Some(Command::Mine(argument.into())),
        _ if name.starts_with('/') => Some(Command::Help),
        _ => None,
    }
}

fn status(snapshot: &Snapshot) -> String {
    let language = snapshot.settings.values.language;
    let state = if snapshot.login.user_id.is_none() {
        "account_required"
    } else if snapshot.settings.values.mining_paused {
        "paused"
    } else {
        match snapshot.mining.state {
            MiningState::Watching => "watching",
            MiningState::Paused => "paused",
            MiningState::AccountRequired => "account_required",
            MiningState::NoSelection => "no_selection",
            MiningState::Discovering => "discovering",
            MiningState::AwaitingClaim => "awaiting_claim",
            MiningState::AwaitingProgress => "awaiting_progress",
            MiningState::WaitingChannel => "waiting_channel",
            MiningState::NoRewards => "no_rewards",
            MiningState::ManualOffline => "manual_offline",
            MiningState::ManualWatching => "manual_watching",
            MiningState::Unknown => "unknown",
        }
    };
    let label = if state == "watching" {
        text(language, "watching", &[])
    } else {
        message_in(language, &format!("gui.redesign.mining_state_{state}"), &[])
    };
    let mut lines = vec![html(&label)];
    if snapshot.login.user_id.is_some() {
        if let Some(channel) = snapshot.channels.iter().find(|c| c.watching) {
            lines.push(text(
                language,
                "channel",
                &[("channel", &html(&channel.name))],
            ));
        }
        if let Some(drop) = &snapshot.current_drop {
            lines.push(format!(
                "<b>{}</b> — {}",
                html(&drop.game_name),
                html(&drop.drop_name)
            ));
            lines.push(if drop.confirmed_at.is_some() {
                text(
                    language,
                    "progress",
                    &[
                        ("current", &drop.confirmed_minutes.to_string()),
                        ("required", &drop.required_minutes.to_string()),
                    ],
                )
            } else {
                text(language, "progress_unknown", &[])
            });
        }
        if snapshot.manual_mode.active || snapshot.manual_mode.pending_channel.is_some() {
            lines.push(text(language, "manual", &[]));
        }
    }
    lines.join("\n")
}

impl Session {
    async fn api(&self, app: &Application, method: &str, body: Value) -> Result<Value, Failure> {
        let operation = async {
            let proxy = app.settings.read().await.proxy.clone();
            let client = app
                .telegram
                .client(&proxy)
                .map_err(|_| Failure::Unavailable)?;
            let response = client
                .post(format!(
                    "{}/bot{}/{method}",
                    app.telegram.endpoint, self.config.token
                ))
                .timeout(Duration::from_secs(30))
                .json(&body)
                .send()
                .await
                .map_err(|_| Failure::Unavailable)?;
            let status = response.status().as_u16();
            if status == 409 {
                return Err(Failure::Conflict);
            }
            let value = response_json(response).await.map_err(|error| match error {
                Error::Oversized if status == 200 => Failure::Oversized,
                _ => Failure::Unavailable,
            })?;
            if status == 429 {
                return Err(Failure::RateLimited(
                    value["parameters"]["retry_after"]
                        .as_u64()
                        .unwrap_or(60)
                        .clamp(1, 60),
                ));
            }
            if status != 200 || value["ok"] != true {
                return Err(Failure::Unavailable);
            }
            value.get("result").cloned().ok_or(Failure::Unavailable)
        };
        tokio::select! { biased;
            _ = app.shutdown.cancelled() => Err(Failure::Cancelled),
            _ = self.cancel.cancelled() => Err(Failure::Cancelled),
            result = operation => result,
        }
    }

    async fn execute(&self, app: &Arc<Application>, command: Command) -> Result<String, AppError> {
        let operation = async {
            // Serialize acceptance with configuration saves; accepted settings writes remain owned.
            let _permit = app
                .telegram
                .writes
                .acquire()
                .await
                .map_err(|_| Error::Cancelled)?;
            if self.cancel.is_cancelled() || app.telegram.epoch() != self.epoch {
                return Err(Error::Cancelled.into());
            }
            let snapshot = app.snapshot.read().await;
            let language = snapshot.settings.values.language;
            match command {
                Command::Help => return Ok(text(language, "help", &[])),
                Command::Status => return Ok(status(&snapshot)),
                _ => {}
            }
            if snapshot.login.user_id.is_none() {
                return Ok(text(language, "login_required", &[]));
            }
            let mut patch = json!({"revision":snapshot.settings.revision});
            let reply = match command {
                Command::Pause | Command::Resume => {
                    patch["mining_paused"] = json!(command == Command::Pause);
                    text(
                        language,
                        if command == Command::Pause {
                            "paused"
                        } else {
                            "resumed"
                        },
                        &[],
                    )
                }
                Command::Mine(name) => {
                    let key = crate::config::fold(&name);
                    let Some(game) = snapshot
                        .settings
                        .values
                        .games_to_watch
                        .iter()
                        .chain(
                            snapshot
                                .campaigns
                                .iter()
                                .filter(|c| !c.expired && !c.finished)
                                .map(|c| &c.game_name),
                        )
                        .find(|game| crate::config::fold(game) == key)
                    else {
                        return Ok(text(language, "unknown_game", &[]));
                    };
                    let mut games = snapshot.settings.values.games_to_watch.clone();
                    if !games.iter().any(|g| crate::config::fold(g) == key) {
                        games.push(game.clone());
                    }
                    patch["games_to_watch"] = json!(games);
                    text(language, "selected", &[("game", &html(game))])
                }
                _ => unreachable!(),
            };
            drop(snapshot);
            app.update_settings(patch).await?;
            Ok(reply)
        };
        tokio::select! { biased;
            _ = app.shutdown.cancelled() => Err(Error::Cancelled.into()),
            _ = self.cancel.cancelled() => Err(Error::Cancelled.into()),
            result = tokio::time::timeout(Duration::from_secs(15), operation) => result.unwrap_or(Err(AppError::Unavailable)),
        }
    }

    async fn poll(&self, app: &Arc<Application>) {
        let mut cursor = Cursor::default();
        let mut tail = false;
        loop {
            let bootstrap = cursor.offset.is_none();
            let result = self
                .api(
                    app,
                    "getUpdates",
                    json!({
                        "offset":if tail { -1 } else { cursor.offset.unwrap_or(-1) }, "limit":1,
                        "timeout":if bootstrap { 0 } else { 25 }, "allowed_updates":["message"]
                    }),
                )
                .await;
            let updates = result.and_then(|value| {
                let updates = value
                    .as_array()
                    .filter(|v| v.len() <= 1)
                    .ok_or(Failure::Unavailable)?;
                if updates.iter().any(|v| {
                    !v["update_id"]
                        .as_i64()
                        .is_some_and(|id| (0..i64::MAX).contains(&id))
                }) {
                    return Err(Failure::Unavailable);
                }
                Ok(updates.clone())
            });
            match updates {
                Err(Failure::Cancelled) => return,
                Err(failure) => {
                    // Skip the blocked queue when a later update arrives; never parse/execute a partial body.
                    tail |= failure == Failure::Oversized;
                    let conflict = failure == Failure::Conflict;
                    let language = app.settings.read().await.language;
                    app.record_activity(event(language, conflict)).await;
                    let seconds = match failure {
                        Failure::RateLimited(delay) => delay,
                        Failure::Conflict => 60,
                        _ => 10,
                    };
                    if !self.wait(app, Duration::from_secs(seconds)).await {
                        return;
                    }
                    continue;
                }
                Ok(updates) => {
                    tail = false;
                    let language = app.settings.read().await.language;
                    app.recover_activity(&event(language, false)).await;
                    app.recover_activity(&event(language, true)).await;
                    for update in updates {
                        if !cursor.advance(&update, Utc::now().timestamp()) {
                            continue;
                        }
                        // Advance before execution/reply; failed replies must never repeat a command.
                        if bootstrap {
                            continue;
                        }
                        let owner = self.config.chat_id.parse().unwrap_or(0);
                        let Some(command) =
                            command(&update, owner, self.started, Utc::now().timestamp())
                        else {
                            continue;
                        };
                        let reply = match self.execute(app, command).await {
                            Ok(reply) => reply,
                            Err(AppError::Telegram(Error::Cancelled)) => return,
                            Err(AppError::SettingsConflict) => {
                                text(app.settings.read().await.language, "conflict", &[])
                            }
                            Err(_) => text(app.settings.read().await.language, "failed", &[]),
                        };
                        let proxy = app.settings.read().await.proxy.clone();
                        let sent = app
                            .telegram
                            .send(
                                self.epoch,
                                &reply,
                                &proxy,
                                &app.shutdown,
                                MessageKind::Reply,
                                None,
                            )
                            .await;
                        super::report(app, sent).await;
                    }
                    if bootstrap {
                        cursor.offset.get_or_insert(0);
                        let language = app.settings.read().await.language;
                        let commands: Vec<_> = ["status", "pause", "resume", "mine", "help"].into_iter()
                            .map(|command| json!({"command":command, "description":text(language, &format!("menu_{command}"), &[])})).collect();
                        // A missing menu does not disable typed commands, or replace another chat's menu.
                        let _ = self.api(app, "setMyCommands", json!({"scope":{"type":"chat", "chat_id":self.config.chat_id}, "commands":commands})).await;
                    }
                }
            }
            if !self.wait(app, Duration::from_secs(1)).await {
                return;
            }
        }
    }

    async fn wait(&self, app: &Application, duration: Duration) -> bool {
        tokio::select! { biased;
            _ = app.shutdown.cancelled() => false,
            _ = self.cancel.cancelled() => false,
            _ = tokio::time::sleep(duration) => true,
        }
    }
}

fn event(language: Language, conflict: bool) -> ActivityEvent {
    ActivityEvent::new_in(
        language,
        if conflict {
            "gui.backend.telegram_controls_conflict"
        } else {
            "gui.backend.telegram_controls_failed"
        },
        Category::Connection,
        Severity::Warning,
        &[("operation", "telegram_controls")],
    )
}

pub(super) async fn run(app: Arc<Application>) {
    loop {
        if app.shutdown.is_cancelled() || app.telegram.delivery.is_closed() {
            return;
        }
        let (session, enabled) = {
            let state = app.telegram.state.lock().unwrap();
            (
                Session {
                    config: state.config.clone(),
                    epoch: state.epoch,
                    cancel: state.cancel.clone(),
                    started: Utc::now().timestamp(),
                },
                state.config.remote_control && !state.unreadable,
            )
        };
        if enabled {
            session.poll(&app).await;
        } else {
            tokio::select! {
                _ = app.shutdown.cancelled() => return,
                _ = session.cancel.cancelled() => {},
            }
        }
    }
}

#[cfg(test)]
#[path = "control_tests.rs"]
mod tests;

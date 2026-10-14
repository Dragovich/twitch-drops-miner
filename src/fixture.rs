//! Feature-gated, loopback-only browser fixture. It never constructs a Twitch client.
use std::{path::PathBuf, sync::Arc};

use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{sync::mpsc, task::JoinHandle};

use crate::{
    dto::{HistoryEntry, Login, ManualMode, OAuthCode, Snapshot},
    web::{App, Command, CommandRequest},
};

fn snapshot() -> Snapshot {
    let mut state: Snapshot = serde_json::from_str(include_str!("../frontend/tests/fixture.json"))
        .expect("valid browser fixture");
    state.settings.refresh_game_keys();
    state.activity = fixture_activity(&state.console);
    state
}

fn fixture_activity(lines: &[String]) -> Vec<crate::app::activity::ActivityEvent> {
    lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let mut event = crate::app::activity::ActivityEvent::message(line.clone());
            event.id = index as u64 + 1;
            event
        })
        .collect()
}

pub async fn create(directory: PathBuf) -> anyhow::Result<(Arc<App>, JoinHandle<()>)> {
    let (mut app, receiver) = App::open(directory, "")?;
    app.telegram.disable_for_fixture();
    Arc::get_mut(&mut app).expect("new application").fixture = true;
    reset_state(&app).await?;
    let worker = tokio::spawn(commands(app.clone(), receiver));
    Ok((app, worker))
}

async fn reset_state(app: &App) -> anyhow::Result<()> {
    app.auth.reset().await?;
    app.sockets.prune(true).await;
    let state = snapshot();
    app.data.save_settings(&state.settings.values)?;
    *app.settings.write().await = state.settings.values.clone();
    *app.snapshot.write().await = state;
    let mut history = app.history.lock().await;
    history.reset_fixture()?;
    history.record(HistoryEntry {
        id: "past-drop".into(),
        claimed_at: "2026-09-25T18:00:00Z".parse()?,
        claimed_at_is_observed: false,
        game: "Rust".into(),
        campaign: "Autumn expedition".into(),
        drop_name: "Canvas pack".into(),
        benefits: vec!["Canvas pack".into()],
        required_minutes: 30,
        campaign_id: "campaign-1".into(),
        image_url: String::new(),
    })?;
    Ok(())
}

pub fn routes(router: Router<Arc<App>>) -> Router<Arc<App>> {
    router
        .route(
            "/__test/health",
            get(|| async { Json(json!({"fixture":true})) }),
        )
        .route("/__test/reset", post(reset))
        .route("/__test/event", post(event))
        .route("/__test/reconnect", post(reconnect))
}
async fn reset(State(app): State<Arc<App>>) -> Result<Json<Value>, crate::web::ApiError> {
    app.command(Command::ExitManual).await?;
    reset_state(&app).await.map_err(|_| {
        crate::web::ApiError(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "fixture_reset_failed",
        )
    })?;
    Ok(Json(json!({"ok":true})))
}
async fn event(State(app): State<Arc<App>>, Json(body): Json<Event>) -> Json<Value> {
    apply_event(&app, &body.event, &body.data).await;
    app.sockets.emit(&body.event, &body.data).await;
    Json(json!({"ok":true}))
}

async fn apply_event(app: &App, event: &str, data: &Value) {
    if event == "console_output" {
        if let Some(text) = data["message"].as_str() {
            app.console(text.to_owned()).await;
        }
        return;
    }
    let mut state = app.snapshot.write().await;
    let mut value = serde_json::to_value(&*state).unwrap();
    match event {
        "initial_state" => {
            value = data.clone();
            if value["activity"]
                .as_array()
                .is_none_or(|events| events.is_empty())
            {
                value["activity"] = serde_json::to_value(fixture_activity(
                    &serde_json::from_value::<Vec<String>>(value["console"].clone())
                        .unwrap_or_default(),
                ))
                .unwrap();
            }
        }
        "inventory_batch_update" => value["campaigns"] = data["campaigns"].clone(),
        "channels_batch_update" => value["channels"] = data["channels"].clone(),
        "channels_clear" => value["channels"] = json!([]),
        "inventory_clear" => value["campaigns"] = json!([]),
        "drop_progress" => value["current_drop"] = data.clone(),
        "drop_progress_stop" => value["current_drop"] = Value::Null,
        "status_update" => value["status"] = data["status"].clone(),
        "login_status" => value["login"] = data.clone(),
        "login_required" => value["login"] = json!({"status":"", "user_id":null}),
        "oauth_code_required" => value["login"]["oauth_pending"] = data.clone(),
        "manual_mode_update" => value["manual_mode"] = data.clone(),
        "wanted_items_update" => value["wanted_items"] = data.clone(),
        "settings_updated" => value["settings"] = data.clone(),
        "games_available" => value["settings"]["games_available"] = data["games"].clone(),
        "inventory_status" => value["inventory_status"] = data.clone(),
        "inventory_refresh"
            if data["sequence"].as_u64().unwrap_or_default()
                >= state.inventory_refresh.sequence =>
        {
            value["inventory_refresh"] = data.clone()
        }
        "history_cleared" => {
            value["history_revision"] = (state.history_revision + 1).into();
            value["history_clear_revision"] = (state.history_clear_revision + 1).into();
        }
        "channel_update" | "channel_add" | "campaign_add" => {
            let list = if event == "campaign_add" {
                "campaigns"
            } else {
                "channels"
            };
            let list = value[list].as_array_mut().unwrap();
            if let Some(old) = list.iter_mut().find(|item| item["id"] == data["id"]) {
                *old = data.clone();
            } else {
                list.push(data.clone());
            }
        }
        "channel_remove" => value["channels"]
            .as_array_mut()
            .unwrap()
            .retain(|item| item["id"] != data["id"]),
        "channel_watching" | "channel_watching_clear" => {
            for item in value["channels"].as_array_mut().unwrap() {
                item["watching"] = (event == "channel_watching" && item["id"] == data["id"]).into();
            }
        }
        "drop_update" => {
            if let Some(campaign) = value["campaigns"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|item| item["id"] == data["campaign_id"])
            {
                if let Some(patch) = data["campaign"].as_object() {
                    campaign.as_object_mut().unwrap().extend(patch.clone());
                }
                if data["drops"].is_array() {
                    campaign["drops"] = data["drops"].clone();
                } else if data["drop"].is_object() {
                    let drops = campaign["drops"].as_array_mut().unwrap();
                    if let Some(drop) = drops
                        .iter_mut()
                        .find(|drop| drop["id"] == data["drop"]["id"])
                    {
                        *drop = data["drop"].clone();
                    } else {
                        drops.push(data["drop"].clone());
                    }
                }
            }
        }
        _ => return,
    }
    if let Ok(next) = serde_json::from_value(value) {
        *state = next;
    }
}
#[derive(Deserialize)]
struct Event {
    event: String,
    data: Value,
}
async fn reconnect(State(app): State<Arc<App>>) -> Json<Value> {
    app.snapshot.write().await.channels.clear();
    app.sockets.prune(true).await;
    Json(json!({"ok":true}))
}

async fn commands(app: Arc<App>, mut receiver: mpsc::Receiver<CommandRequest>) {
    let mut manual_until: Option<tokio::time::Instant> = None;
    loop {
        let request = tokio::select! {
            _ = app.shutdown.cancelled()=>break,
            request = receiver.recv()=>{ let Some(request)=request else{break};Some(request) },
            _ = tokio::time::sleep_until(manual_until.unwrap_or_else(tokio::time::Instant::now)), if manual_until.is_some() => None,
        };
        match request
            .as_ref()
            .map_or(Command::ExitManual, |r| r.command.clone())
        {
            Command::SelectChannel(id, duration) => {
                manual_until = duration.map(|duration| tokio::time::Instant::now() + duration);
                let mode = {
                    let mut state = app.snapshot.write().await;
                    for channel in &mut state.channels {
                        channel.watching = channel.id == id && channel.online;
                    }
                    let channel = state.channels.iter().find(|channel| channel.id == id);
                    state.manual_mode = ManualMode {
                        active: true,
                        channel_id: Some(id),
                        game_name: channel.and_then(|channel| channel.game.clone()),
                        channel_name: channel.map(|channel| channel.name.clone()),
                        expires_at: duration.map(|duration| {
                            chrono::Utc::now() + chrono::Duration::from_std(duration).unwrap()
                        }),
                        ..ManualMode::default()
                    };
                    state.manual_mode.clone()
                };
                let watching = app
                    .snapshot
                    .read()
                    .await
                    .channels
                    .iter()
                    .find(|channel| channel.watching)
                    .map(|channel| channel.id);
                app.sockets
                    .emit("channel_watching", &json!({"id":watching}))
                    .await;
                app.sockets.emit("manual_mode_update", &mode).await;
            }
            Command::ExitManual => {
                manual_until = None;
                let mode = ManualMode::default();
                app.snapshot.write().await.manual_mode = mode.clone();
                app.sockets.emit("manual_mode_update", &mode).await;
            }
            Command::MineChannel(login, duration) => {
                let mode = if login == "missing" {
                    ManualMode {
                        error: Some(app.text("gui.channels.not_found", &[]).await),
                        ..ManualMode::default()
                    }
                } else {
                    manual_until = duration.map(|duration| tokio::time::Instant::now() + duration);
                    let channels = {
                        let mut state = app.snapshot.write().await;
                        for c in &mut state.channels {
                            c.watching = false;
                        }
                        state.current_drop = None;
                        let known = state
                            .channels
                            .iter()
                            .find(|channel| channel.login == login)
                            .cloned();
                        state.channels.retain(|c| {
                            c.id != 999 && Some(c.id) != known.as_ref().map(|channel| channel.id)
                        });
                        let mut channel = known.unwrap_or(crate::dto::ChannelView {
                            id: 999,
                            login: login.clone(),
                            name: login.clone(),
                            game: Some("Other category".into()),
                            game_id: Some(999),
                            online: true,
                            drops_enabled: false,
                            watching: true,
                            ..Default::default()
                        });
                        channel.watching = channel.online;
                        state.channels.push(channel);
                        state.channels.clone()
                    };
                    app.sockets
                        .emit("channels_batch_update", &json!({"channels":channels}))
                        .await;
                    app.sockets.emit("drop_progress_stop", &json!({})).await;
                    let channel = channels.iter().find(|channel| channel.login == login);
                    ManualMode {
                        active: true,
                        channel_id: channel.map(|channel| channel.id),
                        game_name: channel.and_then(|channel| channel.game.clone()),
                        channel_name: channel.map(|channel| channel.name.clone()),
                        expires_at: duration.map(|duration| {
                            chrono::Utc::now() + chrono::Duration::from_std(duration).unwrap()
                        }),
                        ..ManualMode::default()
                    }
                };
                app.snapshot.write().await.manual_mode = mode.clone();
                app.sockets.emit("manual_mode_update", &mode).await;
            }
            Command::Logout => {
                manual_until = None;
                let login = Login {
                    status: "Logged out".into(),
                    user_id: None,
                    oauth_pending: Some(OAuthCode {
                        url: "https://www.twitch.tv/activate".into(),
                        code: "NEWCODE".into(),
                    }),
                    ..Login::default()
                };
                app.snapshot.write().await.login = login.clone();
                app.sockets.emit("login_status", &login).await;
            }
            Command::SettingsChanged => {
                let mut state = app.snapshot.write().await;
                let paused = state.settings.values.mining_paused;
                let watching = state.mining.channel_id;
                for channel in &mut state.channels {
                    channel.watching = !paused && Some(channel.id) == watching;
                }
                state.mining.state = if paused {
                    crate::dto::MiningState::Paused
                } else if state.manual_mode.active {
                    crate::dto::MiningState::ManualWatching
                } else {
                    crate::dto::MiningState::Watching
                };
            }
            Command::ConfirmOAuth | Command::Refresh { .. } | Command::Shutdown => {}
        }
        if let Some(request) = request {
            let _ = request.complete.send(Ok(()));
        }
    }
}

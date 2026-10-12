//! Offline native smoke test. This module is absent from production packages.
use std::path::PathBuf;
use tauri::Manager;
use twitch_drops_miner_core::{
    app::{AppError, Application, Command},
    dto::{MiningState, Snapshot},
    runtime::Runtime,
};

pub async fn start(directory: PathBuf) -> anyhow::Result<Runtime> {
    let (app, mut commands) = Application::open(directory)?;
    app.telegram.disable_for_fixture();
    let mut snapshot: Snapshot =
        serde_json::from_str(include_str!("../../frontend/tests/fixture.json"))?;
    snapshot.settings.refresh_game_keys();
    app.data.save_settings(&snapshot.settings.values)?;
    *app.settings.write().await = snapshot.settings.values.clone();
    *app.snapshot.write().await = snapshot;
    let owned = app.clone();
    let worker = tokio::spawn(async move {
        loop {
            let request = tokio::select! {
                biased;
                _ = owned.shutdown.cancelled() => break,
                value = commands.recv() => { let Some(value) = value else { break; }; value },
            };
            if matches!(request.command, Command::SettingsChanged) {
                let mut snapshot = owned.snapshot.write().await;
                snapshot.mining.state = if snapshot.settings.values.mining_paused {
                    MiningState::Paused
                } else {
                    MiningState::Watching
                };
            }
            let _ = request.complete.send(Ok(()));
        }
        Ok(())
    });
    Ok(Runtime::fixture(app, worker))
}

#[tauri::command]
pub fn smoke_tray_status(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> Result<Option<(String, bool)>, crate::ipc::Error> {
    crate::ipc::local(&window)?;
    let Some(status) = app.try_state::<tauri::menu::MenuItem<tauri::Wry>>() else {
        return Ok(None);
    };
    Ok(Some((
        status.text().map_err(|_| AppError::Unavailable)?,
        status.is_enabled().map_err(|_| AppError::Unavailable)?,
    )))
}

#[tauri::command]
pub async fn smoke_result(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    error: Option<String>,
) -> Result<(), crate::ipc::Error> {
    crate::ipc::local(&window)?;
    let state = app.state::<crate::Desktop>();
    state.updates.cancel.cancel();
    state.updates.tasks.close();
    state.updates.tasks.wait().await;
    state.drain().await;
    if let Some(error) = error {
        eprintln!("Native smoke failed: {error}");
        app.exit(1);
    } else {
        println!("Native smoke passed: snapshot, pause/resume, settings and inline updates");
        app.exit(0);
    }
    Ok(())
}

use std::sync::atomic::Ordering;

use serde::{Deserialize, Serialize};
use tauri::{
    Emitter, Manager, State, WebviewWindow,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_notification::NotificationExt;
use twitch_drops_miner_core::{
    app::{AppError, message, message_in},
    store::atomic_json,
};

use crate::{Desktop, ipc};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    pub close_to_tray: bool,
    pub start_minimized: bool,
    pub notifications: bool,
    pub keep_awake: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            close_to_tray: !cfg!(target_os = "linux"),
            start_minimized: false,
            notifications: true,
            keep_awake: false,
        }
    }
}

#[derive(Serialize)]
pub struct Settings {
    #[serde(flatten)]
    preferences: Preferences,
    autostart: bool,
    tray_available: bool,
    keep_awake_failed: bool,
    version: &'static str,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    CloseToTray(bool),
    StartMinimized(bool),
    Notifications(bool),
    KeepAwake(bool),
    Autostart(bool),
}

fn valid_title(title: &str) -> bool {
    !title.is_empty() && title.len() <= 1024 && !title.chars().any(char::is_control)
}

fn tray_status_text(title: &str) -> String {
    title
        .strip_suffix(" - Drops Miner")
        .unwrap_or(title)
        .replace('&', "&&")
}

#[tauri::command]
pub fn desktop_title(
    window: WebviewWindow,
    app: tauri::AppHandle,
    title: String,
) -> Result<(), ipc::Error> {
    ipc::local(&window)?;
    if !valid_title(&title) {
        return Err(AppError::InvalidRequest.into());
    }
    window
        .set_title(&title)
        .map_err(|_| AppError::Unavailable)?;
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(&title));
    }
    if let Some(status) = app.try_state::<MenuItem<tauri::Wry>>() {
        let _ = status.set_text(tray_status_text(&title));
    }
    Ok(())
}

#[tauri::command]
pub async fn desktop_settings(
    window: WebviewWindow,
    app: tauri::AppHandle,
    state: State<'_, Desktop>,
    change: Option<Change>,
) -> Result<Settings, ipc::Error> {
    ipc::local(&window)?;
    let _task = state.tasks.token();
    if state.cancel.is_cancelled() || state.startup_error.is_some() {
        return Err(AppError::Unavailable.into());
    }
    let mut preferences = state.preferences.lock().await;
    if let Some(change) = change {
        let save_preferences = !matches!(change, Change::Autostart(_));
        let mut next = preferences.clone();
        match change {
            Change::CloseToTray(value) => next.close_to_tray = value,
            Change::StartMinimized(value) => next.start_minimized = value,
            Change::Notifications(value) => next.notifications = value,
            Change::KeepAwake(value) => next.keep_awake = value,
            Change::Autostart(value) => {
                let handle = app.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    if value {
                        handle.autolaunch().enable()
                    } else {
                        handle.autolaunch().disable()
                    }
                })
                .await
                .map_err(|_| AppError::Unavailable)?
                .map_err(|_| AppError::Unavailable)?;
            }
        }
        if save_preferences {
            let path = state.preferences_path.clone();
            let saved = next.clone();
            tauri::async_runtime::spawn_blocking(move || atomic_json(&path, &saved))
                .await
                .map_err(|_| AppError::Unavailable)?
                .map_err(|_| AppError::Unavailable)?;
        }
        *preferences = next;
        state
            .close_to_tray
            .store(preferences.close_to_tray, Ordering::SeqCst);
        state.preferences_changed.notify_one();
    }
    let handle = app.clone();
    let autostart = tauri::async_runtime::spawn_blocking(move || handle.autolaunch().is_enabled())
        .await
        .map_err(|_| AppError::Unavailable)?
        .map_err(|_| AppError::Unavailable)?;
    Ok(Settings {
        preferences: preferences.clone(),
        autostart,
        tray_available: state.tray_available.load(Ordering::SeqCst),
        keep_awake_failed: state.keep_awake_failed.load(Ordering::SeqCst),
        version: env!("CARGO_PKG_VERSION"),
    })
}

pub struct TrayMenu {
    show: MenuItem<tauri::Wry>,
    pause: MenuItem<tauri::Wry>,
    updates: MenuItem<tauri::Wry>,
    quit: MenuItem<tauri::Wry>,
}

pub fn install(app: &tauri::AppHandle) -> tauri::Result<TrayMenu> {
    let label = |key: &str| message(&format!("gui.desktop.{key}"), &[]);
    let status = MenuItem::with_id(app, "status", "Drops Miner", false, None::<&str>)?;
    let status_separator = PredefinedMenuItem::separator(app)?;
    let show = MenuItem::with_id(app, "show", label("show"), true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", label("pause"), true, None::<&str>)?;
    let update = MenuItem::with_id(app, "updates", label("check_updates"), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", label("quit"), true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &status_separator,
            &show,
            &pause,
            &update,
            &separator,
            &quit,
        ],
    )?;
    let pause_action = pause.clone();
    TrayIconBuilder::with_id("main")
        .icon(tauri::image::Image::from_bytes(include_bytes!(
            "../icons/32x32.png"
        ))?)
        .tooltip("Drops Miner")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                crate::show(tray.app_handle());
            }
        })
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "show" => crate::show(app),
            "quit" => crate::quit(app),
            "updates" => {
                crate::show(app);
                let _ = app.emit_to("main", "desktop-update-open", ());
            }
            "pause" => {
                let app = app.clone();
                let control = pause_action.clone();
                app.clone().state::<Desktop>().spawn(async move {
                    let state = app.state::<Desktop>();
                    let Ok(_guard) = state.pause_action.try_lock() else {
                        return;
                    };
                    let _ = control.set_enabled(false);
                    let result = async {
                        let core = state.application().await?;
                        let paused = core.settings.read().await.mining_paused;
                        core.update_settings(serde_json::json!({"mining_paused":!paused}))
                            .await?;
                        Ok::<_, ipc::Error>(())
                    }
                    .await;
                    if result.is_err() {
                        crate::show(&app);
                        let _ = app.emit_to("main", "desktop-error", ());
                    }
                    let _ = control.set_enabled(true);
                });
            }
            _ => {}
        })
        .build(app)?;
    app.manage(status);
    app.state::<Desktop>()
        .tray_available
        .store(true, Ordering::SeqCst);
    Ok(TrayMenu {
        show,
        pause,
        updates: update,
        quit,
    })
}

pub fn observe(app: &tauri::AppHandle, menu: Option<TrayMenu>) {
    let app = app.clone();
    app.clone().state::<Desktop>().spawn(async move {
        let state = app.state::<Desktop>();
        let Ok(core) = state.application().await else {
            if let Some(menu) = menu { let _ = menu.pause.set_enabled(false); }
            return;
        };
        let mut changes = core.snapshot.subscribe();
        let mut notifications = core.notifications.subscribe();
        let awake = crate::power::start(&app);
        loop {
            let enabled = state.preferences.lock().await.keep_awake;
            let snapshot = core.snapshot.read().await;
            let paused = snapshot.settings.values.mining_paused;
            awake.send_if_modified(|desired| {
                let next = crate::power::needed(enabled, &snapshot);
                let changed = *desired != next;
                *desired = next;
                changed
            });
            drop(snapshot);
            if let Some(menu) = &menu {
                let language = core.settings.read().await.language;
                let text = |key: &str| message_in(language, &format!("gui.desktop.{key}"), &[]);
                let _ = menu.show.set_text(text("show"));
                let _ = menu.pause.set_text(text(if paused { "resume" } else { "pause" }));
                let _ = menu.updates.set_text(text("check_updates"));
                let _ = menu.quit.set_text(text("quit"));
            }
            tokio::select! {
                biased;
                _ = state.cancel.cancelled() => break,
                _ = core.shutdown.cancelled() => {
                    if let Some(menu) = &menu { let _ = menu.pause.set_enabled(false); }
                    break;
                },
                _ = state.preferences_changed.notified() => {},
                result = changes.changed() => if result.is_err() { break; },
                result = notifications.recv() => match result {
                    Ok(notification) => if state.preferences.lock().await.notifications {
                        let _ = app.notification().builder().title(notification.title).body(notification.message).show();
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {},
                    Err(_) => break,
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tray_status_preserves_title_states_and_literal_game_names() {
        for status in [
            "5% Rocket League",
            "Rocket League",
            "Watching northwind",
            "Paused",
            "Idle",
            "Disconnected",
            "Drops Miner",
            "Game - Drops Miner",
        ] {
            assert_eq!(tray_status_text(&format!("{status} - Drops Miner")), status);
        }
        assert_eq!(tray_status_text("Drops Miner"), "Drops Miner");
        assert_eq!(tray_status_text("5% S&F - Drops Miner"), "5% S&&F");
    }

    #[test]
    fn desktop_titles_allow_unicode_but_reject_control_characters_and_unbounded_input() {
        for title in [
            "5% Rocket League - Drops Miner",
            "Paused - Drops Miner",
            "龍 - Drops Miner",
        ] {
            assert!(valid_title(title));
        }
        for title in ["", "Drops\0Miner", "Drops\nMiner", &"x".repeat(1025)] {
            assert!(!valid_title(title));
        }
    }

    #[test]
    fn desktop_preferences_are_separate_and_preserve_corrupt_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("desktop.json");
        let preferences = Preferences {
            notifications: false,
            ..Preferences::default()
        };
        atomic_json(&file, &preferences).unwrap();
        let loaded: Preferences = twitch_drops_miner_core::store::read_json(&file)
            .unwrap()
            .unwrap();
        assert!(!loaded.notifications);
        assert!(!loaded.keep_awake);
        assert_eq!(loaded.close_to_tray, Preferences::default().close_to_tray);
        std::fs::write(&file, "bad").unwrap();
        assert!(twitch_drops_miner_core::store::read_json::<Preferences>(&file).is_err());
        assert_eq!(std::fs::read_to_string(file).unwrap(), "bad");
    }

    #[test]
    fn keep_awake_is_opt_in_for_existing_preferences() {
        let mut preferences: Preferences = serde_json::from_str(
            r#"{"close_to_tray":true,"start_minimized":false,"notifications":false}"#,
        )
        .unwrap();
        assert!(!preferences.keep_awake);
        preferences.keep_awake = true;
        let saved = serde_json::to_string(&preferences).unwrap();
        assert!(
            serde_json::from_str::<Preferences>(&saved)
                .unwrap()
                .keep_awake
        );
    }
}

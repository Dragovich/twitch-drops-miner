#[cfg(feature = "desktop-fixture")]
mod fixture;
mod ipc;
mod power;
mod shell;
mod updates;

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use tauri::Manager;
use tauri_plugin_opener::OpenerExt;
use tokio::sync::{Mutex, Notify, Semaphore};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use twitch_drops_miner_core::{
    app::{AppError, Application},
    logging,
    runtime::Runtime,
};

struct Desktop {
    runtime: Mutex<Option<Runtime>>,
    startup_error: Option<String>,
    requests: Semaphore,
    subscription: Mutex<Option<Arc<ipc::Subscription>>>,
    quitting: AtomicBool,
    drained: AtomicBool,
    cancel: CancellationToken,
    tasks: TaskTracker,
    pending: Mutex<HashMap<String, CancellationToken>>,
    preferences: Mutex<shell::Preferences>,
    preferences_path: PathBuf,
    preferences_changed: Notify,
    keep_awake_failed: AtomicBool,
    tray_available: AtomicBool,
    close_to_tray: AtomicBool,
    pause_action: Mutex<()>,
    updates: updates::Updates,
    installing: AtomicBool,
}

fn local_url(url: &url::Url) -> bool {
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    let bundled = matches!(
        (url.scheme(), url.host_str()),
        ("tauri", Some("localhost")) | ("http" | "https", Some("tauri.localhost"))
    ) && url.port().is_none();
    bundled
        || (cfg!(debug_assertions) && url.origin().ascii_serialization() == "http://127.0.0.1:5173")
}

fn open_external(app: &tauri::AppHandle, url: &url::Url) {
    if matches!(url.scheme(), "https" | "http")
        && url.username().is_empty()
        && url.password().is_none()
    {
        let _ = app.opener().open_url(url.as_str(), None::<&str>);
    }
}

#[tauri::command]
fn quit_app(window: tauri::WebviewWindow, app: tauri::AppHandle) -> Result<(), ipc::Error> {
    ipc::local(&window)?;
    quit(&app);
    Ok(())
}

#[tauri::command]
fn restart_app(window: tauri::WebviewWindow, app: tauri::AppHandle) -> Result<(), ipc::Error> {
    ipc::local(&window)?;
    finish(&app, true);
    Ok(())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Folder {
    Data,
    Logs,
}

#[tauri::command]
fn open_app_folder(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    folder: Folder,
) -> Result<(), ipc::Error> {
    ipc::local(&window)?;
    let path = match folder {
        Folder::Data => app.path().app_local_data_dir(),
        Folder::Logs => app.path().app_log_dir(),
    }
    .map_err(|_| AppError::Unavailable)?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|_| AppError::Unavailable)?;
    Ok(())
}

impl Desktop {
    fn spawn(&self, task: impl std::future::Future<Output = ()> + Send + 'static) {
        self.tasks
            .spawn_on(task, tauri::async_runtime::handle().inner());
    }
    async fn drain(&self) {
        self.cancel.cancel();
        self.tasks.close();
        if let Some(mut runtime) = self.runtime.lock().await.take()
            && runtime.shutdown().await.is_err()
        {
            tracing::error!("Mining task failed during shutdown");
        }
        self.tasks.wait().await;
        self.drained.store(true, Ordering::SeqCst);
    }
    async fn application(&self) -> Result<Arc<Application>, ipc::Error> {
        let runtime = self.runtime.lock().await;
        runtime
            .as_ref()
            .map(|value| value.app.clone())
            .ok_or_else(|| ipc::Error {
                status: 503,
                detail: self
                    .startup_error
                    .clone()
                    .unwrap_or_else(|| AppError::Unavailable.to_string()),
            })
    }
}

fn show(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn quit(app: &tauri::AppHandle) {
    finish(app, false);
}

fn finish(app: &tauri::AppHandle, restart: bool) {
    let state = app.state::<Desktop>();
    if state.quitting.swap(true, Ordering::SeqCst) {
        return;
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = handle.state::<Desktop>();
        state.updates.cancel.cancel();
        state.updates.tasks.close();
        state.updates.tasks.wait().await;
        state.drain().await;
        if restart {
            handle.restart();
        } else {
            handle.exit(0);
        }
    });
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| show(app)))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::all()
                        & !tauri_plugin_window_state::StateFlags::VISIBLE,
                )
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .arg("--autostart")
                .build(),
        )
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            #[cfg(not(feature = "desktop-fixture"))]
            let directory = app.path().app_local_data_dir()?;
            #[cfg(feature = "desktop-fixture")]
            let directory = {
                let temp = tempfile::tempdir()?;
                let path = temp.path().to_owned();
                app.manage(temp);
                path
            };
            #[cfg(not(feature = "desktop-fixture"))]
            let logs = app.path().app_log_dir()?;
            #[cfg(feature = "desktop-fixture")]
            let logs = directory.join("logs");
            let preferences_path = directory.join("desktop.json");
            let preferences =
                twitch_drops_miner_core::store::read_json::<shell::Preferences>(&preferences_path);
            let guard = logging::initialize(&logs, 0, false);
            let opened = if preferences.is_ok() && guard.is_ok() {
                Application::open(directory.join("data"))
            } else {
                Err(anyhow::anyhow!("desktop storage unavailable"))
            };
            app.manage(Mutex::new(guard.ok()));
            let preferences = preferences.ok().flatten().unwrap_or_default();
            let start_minimized = preferences.start_minimized;
            let close_to_tray = preferences.close_to_tray;
            #[cfg(not(feature = "desktop-fixture"))]
            let (runtime, startup_error) = match opened {
                Ok((application, commands)) => (
                    Some(tauri::async_runtime::block_on(async {
                        Runtime::start(application, commands)
                    })),
                    None,
                ),
                Err(_) => (None, Some("desktop_start_failed".into())),
            };
            #[cfg(feature = "desktop-fixture")]
            let (runtime, startup_error) = {
                drop(opened);
                (
                    Some(tauri::async_runtime::block_on(fixture::start(
                        directory.join("data"),
                    ))?),
                    None,
                )
            };
            app.manage(Desktop {
                runtime: Mutex::new(runtime),
                startup_error,
                requests: Semaphore::new(8),
                subscription: Mutex::new(None),
                quitting: AtomicBool::new(false),
                drained: AtomicBool::new(false),
                cancel: CancellationToken::new(),
                tasks: TaskTracker::new(),
                pending: Mutex::new(HashMap::new()),
                preferences: Mutex::new(preferences),
                preferences_path,
                preferences_changed: Notify::new(),
                keep_awake_failed: AtomicBool::new(false),
                tray_available: AtomicBool::new(false),
                close_to_tray: AtomicBool::new(close_to_tray),
                pause_action: Mutex::new(()),
                updates: updates::Updates::default(),
                installing: AtomicBool::new(false),
            });
            let handle = app.handle().clone();
            let popup_handle = handle.clone();
            let window =
                tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?
                    .visible(false)
                    .on_navigation(move |url| {
                        if local_url(url) {
                            return true;
                        }
                        open_external(&handle, url);
                        false
                    })
                    .on_new_window(move |url, _| {
                        open_external(&popup_handle, &url);
                        tauri::webview::NewWindowResponse::Deny
                    })
                    .on_page_load(|window, event| {
                        #[cfg(feature = "desktop-fixture")]
                        if event.event() == tauri::webview::PageLoadEvent::Finished
                            && std::env::args().any(|argument| argument == "--offline-smoke")
                        {
                            let _ = window.eval(include_str!("../smoke.js"));
                        }
                        #[cfg(not(feature = "desktop-fixture"))]
                        let _ = (window, event);
                    })
                    .build()?;
            let menu = match shell::install(app.handle()) {
                Ok(menu) => Some(menu),
                Err(_) => {
                    tracing::warn!("Tray icon unavailable; keeping the window accessible");
                    None
                }
            };
            shell::observe(app.handle(), menu);
            let state = app.state::<Desktop>();
            if !start_minimized
                || !state.tray_available.load(Ordering::SeqCst)
                || state.startup_error.is_some()
            {
                window.show()?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ipc::app_request,
            ipc::cancel_request,
            quit_app,
            restart_app,
            open_app_folder,
            shell::desktop_settings,
            shell::desktop_title,
            updates::desktop_update,
            ipc::state_open,
            ipc::state_next,
            ipc::state_close,
            #[cfg(feature = "desktop-fixture")]
            fixture::smoke_tray_status,
            #[cfg(feature = "desktop-fixture")]
            fixture::smoke_result
        ])
        .build(tauri::generate_context!())
        .expect("could not initialize Drops Miner")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if matches!(&event, tauri::RunEvent::Reopen { .. }) {
                show(app);
            }
            if let tauri::RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::CloseRequested { api, .. },
                ..
            } = &event
                && label == "main"
            {
                let state = app.state::<Desktop>();
                if state.quitting.load(Ordering::SeqCst) {
                    api.prevent_close();
                    return;
                }
                if state.tray_available.load(Ordering::SeqCst)
                    && !state.quitting.load(Ordering::SeqCst)
                    && state.close_to_tray.load(Ordering::SeqCst)
                {
                    api.prevent_close();
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.hide();
                    }
                }
            }
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if app.state::<Desktop>().installing.load(Ordering::SeqCst) {
                    api.prevent_exit();
                    return;
                }
                if !app.state::<Desktop>().drained.load(Ordering::SeqCst) {
                    api.prevent_exit();
                    quit(app);
                }
            }
        });
}

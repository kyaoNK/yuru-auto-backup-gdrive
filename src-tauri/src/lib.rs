use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt;

pub mod app_dir;
pub mod backup;
pub mod commands;
pub mod config;
pub mod drive_detector;
pub mod drive_waiter;
pub mod logger;
pub mod scheduler;

use crate::app_dir::AppDir;
use crate::backup::JobSummary;
use crate::commands::AppState;
use crate::config::ConfigStore;
use crate::logger::Logger;
use crate::scheduler::JobReporter;

struct TauriReporter {
    app: AppHandle,
    running: Arc<AtomicBool>,
}

impl JobReporter for TauriReporter {
    fn job_started(&self) {
        self.running.store(true, Ordering::SeqCst);
        let _ = self.app.emit("job-started", ());
        let _ = self.app.emit("status-changed", ());
    }

    fn job_finished(&self, summary: JobSummary) {
        self.running.store(false, Ordering::SeqCst);
        let _ = self.app.emit("job-finished", summary);
        let _ = self.app.emit("status-changed", ());
    }

    fn job_errored(&self, message: String) {
        self.running.store(false, Ordering::SeqCst);
        let _ = self.app.emit("error-occurred", message);
        let _ = self.app.emit("status-changed", ());
    }

    fn status_changed(&self) {
        let _ = self.app.emit("status-changed", ());
    }
}

pub(crate) fn sync_autostart(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let mgr = app.autolaunch();
    let currently = mgr.is_enabled().map_err(|e| e.to_string())?;
    if enabled && !currently {
        mgr.enable().map_err(|e| e.to_string())?;
    } else if !enabled && currently {
        mgr.disable().map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "設定を開く", true, None::<&str>)?;
    let run_now = MenuItem::with_id(app, "run_now", "今すぐ実行", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "終了", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &run_now, &quit])?;

    TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            "run_now" => {
                if let Some(state) = app.try_state::<AppState>() {
                    if !state.running.load(Ordering::SeqCst) {
                        let _ = state.scheduler.run_now();
                    }
                }
            }
            "quit" => {
                if let Some(state) = app.try_state::<AppState>() {
                    let handle = state
                        .scheduler_handle
                        .lock()
                        .ok()
                        .and_then(|mut guard| guard.take());
                    if let Some(handle) = handle {
                        let stopped = handle.shutdown_with_timeout(Duration::from_secs(30));
                        if !stopped {
                            state
                                .logger
                                .warn("Scheduler did not stop within 30 seconds; exiting anyway");
                        }
                    } else {
                        state
                            .logger
                            .warn("Scheduler handle was not available at shutdown");
                    }
                }
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
        })
        .build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default();

    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }));
    }

    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            let handle = app.handle().clone();

            let app_dir = AppDir::resolve().expect("failed to resolve app dir");
            app_dir.ensure_exists().expect("failed to create app dir");

            let config_store = Arc::new(ConfigStore::new(app_dir.config_path()));
            let logger =
                Arc::new(Logger::open(&app_dir.log_file()).expect("failed to open log file"));
            let running = Arc::new(AtomicBool::new(false));

            let reporter: Arc<dyn JobReporter> = Arc::new(TauriReporter {
                app: handle.clone(),
                running: running.clone(),
            });

            let scheduler_handle = scheduler::start(config_store.clone(), logger.clone(), reporter);
            let scheduler_sender = scheduler_handle.sender();

            if let Ok(cfg) = config_store.load() {
                let _ = sync_autostart(&handle, cfg.auto_start);
            }

            app.manage(AppState {
                app_dir,
                config_store,
                logger,
                scheduler: scheduler_sender,
                scheduler_handle: Mutex::new(Some(scheduler_handle)),
                running,
            });

            build_tray(&handle)?;

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::update_config,
            commands::pick_folder,
            commands::detect_drive_roots,
            commands::get_status,
            commands::run_now,
            commands::list_recent_logs,
            commands::open_app_dir,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

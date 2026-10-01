use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

pub mod app_dir;
pub mod atomic_file;
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
use crate::config::{ConfigStore, JobFailure};
use crate::logger::Logger;
use crate::scheduler::JobReporter;

struct TauriReporter {
    app: AppHandle,
    running: Arc<AtomicBool>,
    runtime_error: Arc<Mutex<Option<JobFailure>>>,
}

impl JobReporter for TauriReporter {
    fn job_started(&self) {
        self.running.store(true, Ordering::SeqCst);
        let _ = self.app.emit("job-started", ());
        self.status_changed();
    }

    fn job_finished(&self, summary: JobSummary) {
        *self.runtime_error.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.running.store(false, Ordering::SeqCst);
        let _ = self.app.emit("job-finished", summary);
        self.status_changed();
    }

    fn job_errored(&self, message: String) {
        *self.runtime_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(JobFailure {
            at: chrono::Local::now(),
            message: message.clone(),
        });
        self.running.store(false, Ordering::SeqCst);
        let _ = self.app.emit("error-occurred", message);
        self.status_changed();
    }

    fn status_changed(&self) {
        update_tray(&self.app);
        let _ = self.app.emit("status-changed", ());
    }
}

struct TrayStatus(MenuItem<tauri::Wry>);

fn update_tray(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let config = state.config_store.load();
    let failed = config
        .as_ref()
        .map(|cfg| {
            cfg.last_error.is_some()
                || cfg.last_summary.is_some_and(|summary| summary.errors > 0)
                || cfg.validate().is_err()
        })
        .unwrap_or(true)
        || state
            .runtime_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
        || state
            .service_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
        || state.logger.last_error().is_some();
    let label = if state.scheduler.is_busy() {
        "実行中".to_string()
    } else if failed {
        "エラー（画面で確認）".into()
    } else if config
        .as_ref()
        .is_ok_and(|cfg| cfg.source.is_none() || cfg.destination.is_none())
    {
        "未設定".into()
    } else if let Some(next) = state.scheduler.next_run_at() {
        format!("次回 {}", next.format("%m/%d %H:%M"))
    } else {
        "待機中／設定を確認".into()
    };
    if let Some(status) = app.try_state::<TrayStatus>() {
        let _ = status.0.set_text(&label);
    }
    if let Some(tray) = app.tray_by_id("main-tray") {
        let _ = tray.set_tooltip(Some(&label));
    }
}

fn show_settings(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        let _ = app.emit("navigate-settings", ());
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

fn show_startup_error(app: &AppHandle, message: String) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    let handle = app.clone();
    app.dialog().message(format!("設定・ログの保存先を準備できませんでした。既存データは削除せず、保存先の空き容量とアクセス権を確認してください。\n{message}"))
        .title("起動できませんでした").kind(MessageDialogKind::Error)
        .show(move |_| handle.exit(1));
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let status = MenuItem::with_id(app, "status", "待機中", false, None::<&str>)?;
    let show = MenuItem::with_id(app, "show", "設定を開く", true, None::<&str>)?;
    let run_now = MenuItem::with_id(app, "run_now", "今すぐ実行", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "終了", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&status, &show, &run_now, &quit])?;
    app.manage(TrayStatus(status));

    TrayIconBuilder::with_id("main-tray")
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                show_settings(app);
            }
            "run_now" => {
                if let Some(state) = app.try_state::<AppState>() {
                    if !state.running.load(Ordering::SeqCst) {
                        if let Err(err) = state.scheduler.run_now() {
                            state.logger.error(&err.to_string());
                        }
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
                        let app = app.clone();
                        let logger = state.logger.clone();
                        // Signal first so no new work can be accepted while the UI remains responsive.
                        let _ = state.scheduler.shutdown();
                        std::thread::spawn(move || {
                            let stopped = handle.shutdown_with_timeout(Duration::from_secs(30));
                            if !stopped {
                                logger.warn(
                                    "Scheduler did not stop within 30 seconds; exiting anyway",
                                );
                            }
                            app.exit(0);
                        });
                    }
                }
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
                show_settings(app);
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
                let _ = w.unminimize();
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

            let app_dir = match AppDir::resolve().and_then(|dir| { dir.ensure_exists()?; Ok(dir) }) {
                Ok(dir) => dir,
                Err(err) => { show_startup_error(&handle, err.to_string()); return Ok(()); }
            };

            let config_store = Arc::new(ConfigStore::new(app_dir.config_path()));
            let logger = match Logger::open(&app_dir.log_file()) {
                Ok(logger) => Arc::new(logger),
                Err(err) => { show_startup_error(&handle, err.to_string()); return Ok(()); }
            };
            let running = Arc::new(AtomicBool::new(false));
            let runtime_error = Arc::new(Mutex::new(None));
            let service_error = Arc::new(Mutex::new(None));

            let reporter: Arc<dyn JobReporter> = Arc::new(TauriReporter {
                app: handle.clone(),
                running: running.clone(),
                runtime_error: runtime_error.clone(),
            });

            let scheduler_handle = scheduler::start(config_store.clone(), logger.clone(), reporter);
            let scheduler_sender = scheduler_handle.sender();

            if let Ok(cfg) = config_store.load() {
                if let Err(err) = sync_autostart(&handle, cfg.auto_start) {
                    let message = format!("自動起動設定を反映できませんでした。設定画面から保存し直してください: {err}");
                    logger.error(&message);
                    *service_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(message);
                }
            }

            app.manage(AppState {
                app_dir,
                config_store,
                logger,
                scheduler: scheduler_sender,
                scheduler_handle: Mutex::new(Some(scheduler_handle)),
                running,
                runtime_error,
                service_error,
            });

            build_tray(&handle)?;
            update_tray(&handle);

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
            commands::preview_deletions,
            commands::list_recent_logs,
            commands::open_app_dir,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

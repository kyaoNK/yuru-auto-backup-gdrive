use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Local};
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::app_dir::AppDir;
use crate::backup::{BackupJob, DeletionPreview, JobSummary};
use crate::config::{Config, ConfigStore, JobFailure};
use crate::drive_detector::{self, DriveCandidate};
use crate::logger::Logger;
use crate::scheduler::{self, SchedulerSender};

pub struct AppState {
    pub app_dir: AppDir,
    pub config_store: Arc<ConfigStore>,
    pub logger: Arc<Logger>,
    pub scheduler: SchedulerSender,
    pub scheduler_handle: Mutex<Option<scheduler::SchedulerHandle>>,
    pub running: Arc<AtomicBool>,
    pub runtime_error: Arc<Mutex<Option<JobFailure>>>,
    pub service_error: Arc<Mutex<Option<String>>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub running: bool,
    pub last_run_at: Option<DateTime<Local>>,
    pub next_run_at: Option<DateTime<Local>>,
    pub last_summary: Option<JobSummary>,
    pub last_error: Option<JobFailure>,
    pub source: Option<PathBuf>,
    pub destination: Option<PathBuf>,
    pub schedule_time: String,
    pub auto_start: bool,
    pub service_error: Option<String>,
}

fn err_to_string<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[tauri::command]
pub async fn get_config(state: State<'_, AppState>) -> Result<Config, String> {
    let store = state.config_store.clone();
    tauri::async_runtime::spawn_blocking(move || store.load().map_err(err_to_string))
        .await
        .map_err(err_to_string)?
}

#[tauri::command]
pub async fn update_config(
    app: AppHandle,
    state: State<'_, AppState>,
    config: Config,
) -> Result<(), String> {
    let store = state.config_store.clone();
    let scheduler = state.scheduler.clone();
    let logger = state.logger.clone();
    let service_error = state.service_error.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = store.update_preferences(
            config,
            || app.autolaunch().is_enabled().map_err(err_to_string),
            |enabled| crate::sync_autostart(&app, enabled),
            || scheduler.reload().map_err(err_to_string),
        );
        if let Err(err) = &result {
            logger.error(err);
        } else {
            *service_error.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        result
    })
    .await
    .map_err(err_to_string)?
}

#[tauri::command]
pub async fn preview_deletions(state: State<'_, AppState>) -> Result<DeletionPreview, String> {
    if state.scheduler.is_busy() {
        return Err("バックアップ実行中です。完了後に確認してください。".into());
    }
    let cfg = state.config_store.load().map_err(err_to_string)?;
    let (Some(source), Some(destination)) = (cfg.source, cfg.destination) else {
        return Err("監視元と出力先を設定・保存してください。".into());
    };
    tauri::async_runtime::spawn_blocking(move || {
        BackupJob::new(source, destination)
            .with_excluded_folders(cfg.excluded_folders)
            .with_excluded_folder_names(cfg.excluded_folder_names)
            .preview_deletions()
            .map_err(err_to_string)
    })
    .await
    .map_err(err_to_string)?
}

#[tauri::command]
pub async fn pick_folder(app: AppHandle, start_dir: Option<PathBuf>) -> Option<PathBuf> {
    let mut builder = app.dialog().file();
    if let Some(dir) = start_dir {
        if dir.is_dir() {
            builder = builder.set_directory(dir);
        }
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    builder.pick_folder(move |picked| {
        let _ = tx.send(picked);
    });
    match rx.await {
        Ok(Some(fp)) => fp.into_path().ok(),
        _ => None,
    }
}

#[tauri::command]
pub async fn detect_drive_roots() -> Result<Vec<DriveCandidate>, String> {
    tauri::async_runtime::spawn_blocking(drive_detector::detect)
        .await
        .map_err(err_to_string)
}

#[tauri::command]
pub async fn get_status(state: State<'_, AppState>) -> Result<Status, String> {
    let store = state.config_store.clone();
    let scheduler = state.scheduler.clone();
    let runtime_error = state.runtime_error.clone();
    let service_error = state.service_error.clone();
    let logger = state.logger.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let cfg = store.load().map_err(err_to_string)?;
        let service_error = service_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .or_else(|| logger.last_error())
            .or_else(|| cfg.validate().err().map(|e| e.to_string()));
        let last_error = runtime_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .or(cfg.last_error);
        Ok(Status {
            running: scheduler.is_busy(),
            last_run_at: cfg.last_run_at,
            next_run_at: scheduler.next_run_at(),
            last_summary: cfg.last_summary,
            last_error,
            source: cfg.source,
            destination: cfg.destination,
            schedule_time: cfg.schedule_time,
            auto_start: cfg.auto_start,
            service_error,
        })
    })
    .await
    .map_err(err_to_string)?
}

#[tauri::command]
pub fn run_now(state: State<'_, AppState>) -> Result<bool, String> {
    state.scheduler.run_now().map_err(err_to_string)
}

#[tauri::command]
pub async fn list_recent_logs(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<String>, String> {
    let n = limit.unwrap_or(200).min(5000);
    let logger = state.logger.clone();
    tauri::async_runtime::spawn_blocking(move || logger.tail(n).map_err(err_to_string))
        .await
        .map_err(err_to_string)?
}

#[tauri::command]
pub fn open_app_dir(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let path = state.app_dir.root().to_string_lossy().to_string();
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(err_to_string)
}

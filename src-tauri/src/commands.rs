use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Local};
use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::app_dir::AppDir;
use crate::backup::JobSummary;
use crate::config::{Config, ConfigStore};
use crate::drive_detector::{self, DriveCandidate};
use crate::logger::Logger;
use crate::scheduler::{self, SchedulerSender};

pub struct AppState {
    pub app_dir: AppDir,
    pub config_store: Arc<ConfigStore>,
    pub logger: Arc<Logger>,
    pub scheduler: SchedulerSender,
    pub running: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub running: bool,
    pub last_run_at: Option<DateTime<Local>>,
    pub next_run_at: Option<DateTime<Local>>,
    pub last_summary: Option<JobSummary>,
    pub source: Option<PathBuf>,
    pub destination: Option<PathBuf>,
    pub schedule_time: String,
    pub auto_start: bool,
}

fn err_to_string<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> Result<Config, String> {
    state.config_store.load().map_err(err_to_string)
}

#[tauri::command]
pub fn update_config(state: State<'_, AppState>, config: Config) -> Result<(), String> {
    let mut cfg = config;
    let existing = state.config_store.load().unwrap_or_default();
    cfg.last_run_at = existing.last_run_at;
    cfg.last_summary = existing.last_summary;
    state.config_store.save(&cfg).map_err(err_to_string)?;
    let _ = state.scheduler.reload();
    Ok(())
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
pub fn detect_drive_roots() -> Vec<DriveCandidate> {
    drive_detector::detect()
}

#[tauri::command]
pub fn get_status(state: State<'_, AppState>) -> Result<Status, String> {
    let cfg = state.config_store.load().map_err(err_to_string)?;
    let next_run_at = scheduler::parse_schedule(&cfg.schedule_time)
        .ok()
        .map(|t| scheduler::next_run_after(Local::now(), t));

    Ok(Status {
        running: state.running.load(Ordering::SeqCst),
        last_run_at: cfg.last_run_at,
        next_run_at,
        last_summary: cfg.last_summary,
        source: cfg.source,
        destination: cfg.destination,
        schedule_time: cfg.schedule_time,
        auto_start: cfg.auto_start,
    })
}

#[tauri::command]
pub fn run_now(state: State<'_, AppState>) -> Result<(), String> {
    state.scheduler.run_now().map_err(err_to_string)
}

#[tauri::command]
pub fn list_recent_logs(state: State<'_, AppState>, limit: Option<usize>) -> Result<Vec<String>, String> {
    let n = limit.unwrap_or(200);
    state.logger.tail(n).map_err(err_to_string)
}

#[tauri::command]
pub fn open_app_dir(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let path = state.app_dir.root().to_string_lossy().to_string();
    app.opener().open_path(path, None::<&str>).map_err(err_to_string)
}

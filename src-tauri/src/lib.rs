pub mod app_dir;
pub mod backup;
pub mod config;
pub mod drive_waiter;
pub mod logger;
pub mod scheduler;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

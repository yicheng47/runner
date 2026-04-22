mod commands;
mod db;
mod error;
mod event_bus;
mod model;
mod orchestrator;
mod session;

use tauri::Manager;

pub struct AppState {
    pub db: db::DbPool,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&app_data_dir)?;
            let db_path = app_data_dir.join("runners.db");
            let pool = db::open_pool(&db_path)?;
            app.manage(AppState { db: pool });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

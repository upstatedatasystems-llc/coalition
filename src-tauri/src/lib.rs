pub mod commands;
pub mod core;
pub mod db;

use commands::AppState;
use db::DbManager;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::Mutex;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let db = DbManager::new_in_memory().expect("Failed to initialize operational database");
    let state = AppState {
        db: Mutex::new(db),
        git: Mutex::new(None),
        agy: Mutex::new(None),
        cancel_flag: Arc::new(AtomicBool::new(false)),
    };

    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            commands::get_system_diagnostics,
            commands::run_sqlite_proof,
            commands::start_builder_turn,
            commands::cancel_builder_turn,
            commands::desktop_clipboard_write,
            commands::desktop_clipboard_read,
            commands::desktop_open_url,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

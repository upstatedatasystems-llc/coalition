pub mod commands;
pub mod core;
pub mod db;

use commands::AppState;
use db::DbManager;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::Mutex;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let db = if let Ok(custom_path) = std::env::var("COALITION_DB_PATH") {
                let mut d = DbManager::open(custom_path).expect("Failed to open custom database");
                d.run_migrations().expect("Failed to run migrations");
                d
            } else {
                let app_dir = app
                    .path()
                    .app_data_dir()
                    .unwrap_or_else(|_| PathBuf::from("."));
                std::fs::create_dir_all(&app_dir).ok();
                let db_path = app_dir.join("coalition.db");
                let mut d = DbManager::open(db_path).expect("Failed to open operational database");
                d.run_migrations().expect("Failed to run migrations");
                d
            };

            let state = AppState {
                db: Mutex::new(db),
                git: Mutex::new(None),
                agy: Mutex::new(None),
                cancel_flag: Arc::new(AtomicBool::new(false)),
                active_project_id: Mutex::new(None),
            };

            app.manage(state);

            #[cfg(desktop)]
            {
                use std::str::FromStr;
                use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};
                if let Ok(shortcut_r) = Shortcut::from_str("CommandOrControl+Shift+R") {
                    let _ = app.global_shortcut().register(shortcut_r);
                }
                if let Ok(shortcut_i) = Shortcut::from_str("CommandOrControl+Shift+I") {
                    let _ = app.global_shortcut().register(shortcut_i);
                }
            }

            Ok(())
        })
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    use tauri::Emitter;
                    use tauri_plugin_global_shortcut::ShortcutState;
                    if event.state() == ShortcutState::Pressed {
                        let shortcut_str = shortcut.to_string();
                        if shortcut_str.contains("Shift+KeyR") || shortcut_str.contains("Shift+R") {
                            let _ = app.emit("coalition:shortcut-copy-relay", ());
                        } else if shortcut_str.contains("Shift+KeyI")
                            || shortcut_str.contains("Shift+I")
                        {
                            let _ = app.emit("coalition:shortcut-import-clipboard", ());
                        }
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            commands::list_projects,
            commands::register_or_open_project,
            commands::get_project_details,
            commands::refresh_project_git_state,
            commands::get_project_activity,
            commands::apply_workflow_action,
            commands::get_last_opened_project_id,
            commands::get_system_diagnostics,
            commands::run_sqlite_proof,
            commands::start_builder_turn,
            commands::cancel_builder_turn,
            commands::desktop_clipboard_write,
            commands::desktop_clipboard_read,
            commands::desktop_open_url,
            commands::open_chatgpt,
            commands::prepare_architect_relay_packet,
            commands::get_pending_relay_packet,
            commands::get_relay_history,
            commands::copy_relay_packet_to_clipboard,
            commands::import_from_clipboard,
            commands::retry_parse_import,
            commands::accept_relay_import,
            commands::reject_relay_import,
            commands::get_architecture_workspace_state,
            commands::get_artifact_content,
            commands::save_artifact_content,
            commands::set_active_project_id,
            commands::set_project_artifact_applicability,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

pub mod commands;
pub mod core;
pub mod db;

use commands::AppState;
use db::DbManager;
use std::path::PathBuf;
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
                let _ = d.reconcile_orphaned_sessions();
                let _ = crate::core::validation::ValidationService::reconcile_interrupted_runs(
                    d.connection_mut(),
                );
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
                let _ = d.reconcile_orphaned_sessions();
                let _ = crate::core::validation::ValidationService::reconcile_interrupted_runs(
                    d.connection_mut(),
                );
                d
            };

            let state = AppState {
                db: Arc::new(Mutex::new(db)),
                git: Mutex::new(None),
                agy: Mutex::new(None),
                active_project_id: Mutex::new(None),
                active_builder_registry: Arc::new(Mutex::new(
                    crate::core::builder::ActiveBuilderRegistry::new(),
                )),
                active_validation_registry: Arc::new(Mutex::new(
                    crate::core::validation::ActiveValidationRegistry::new(),
                )),
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
            commands::list_builder_models,
            commands::get_builder_session,
            commands::list_builder_sessions,
            commands::get_builder_events,
            commands::start_builder_turn,
            commands::cancel_builder_turn,
            commands::run_diagnostic_fake_agy_turn,
            commands::get_usage_telemetry,
            commands::reset_chatgpt_usage,
            commands::calibrate_chatgpt_usage,
            commands::get_permission_history,
            commands::get_icarus_state,
            commands::set_icarus_mode,
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
            commands::prepare_architecture_freeze,
            commands::confirm_architecture_freeze,
            commands::get_contract_drift,
            commands::get_drift_diff,
            commands::restore_drifted_artifact,
            commands::restore_all_drifted_artifacts,
            commands::get_builder_packet,
            commands::export_project_diagnostics,
            commands::get_validation_config,
            commands::start_validation_run,
            commands::stop_validation_command,
            commands::stop_validation_run,
            commands::get_active_validation_run,
            commands::get_validation_history,
            commands::get_validation_run_details,
            commands::get_validation_run_commands,
            commands::override_validation_gate,
            commands::submit_for_review,
            commands::prepare_review_packet,
            commands::get_latest_review_cycle,
            commands::list_review_cycles,
            commands::prepare_review_import,
            commands::confirm_review_import,
            commands::get_review_packet_content,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

use crate::core::builder::safe_sanitize_text;
use crate::db::DbManager;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

#[derive(Debug, Error)]
pub enum DiagnosticsError {
    #[error("Database error: {0}")]
    Database(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("ZIP error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Project not found: {0}")]
    ProjectNotFound(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportMetadata {
    pub application: String,
    pub version: String,
    pub project_id: String,
    pub exported_at: String,
    pub operating_system: String,
    pub bounds_applied: BoundsSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundsSummary {
    pub max_sessions: usize,
    pub max_events: usize,
    pub max_permissions: usize,
    pub max_activity: usize,
    pub prompt_strategy: String,
    pub field_max_bytes: usize,
}

pub fn export_project_diagnostics(
    db: &DbManager,
    project_id: &str,
    destination_dir: Option<&Path>,
) -> Result<PathBuf, DiagnosticsError> {
    // 1. Verify project exists
    let project_exists: bool = db
        .connection()
        .query_row(
            "SELECT 1 FROM projects WHERE project_id = ?1",
            params![project_id],
            |_| Ok(true),
        )
        .unwrap_or(false);

    if !project_exists {
        return Err(DiagnosticsError::ProjectNotFound(project_id.to_string()));
    }

    let now_dt = chrono::Utc::now();
    let timestamp_slug = now_dt.format("%Y%m%d_%H%M%S").to_string();
    let zip_filename = format!(
        "coalition-diagnostics-{}-{}.zip",
        project_id, timestamp_slug
    );

    let base_dir = match destination_dir {
        Some(d) => d.to_path_buf(),
        None => std::env::temp_dir().join("coalition-diagnostics"),
    };
    std::fs::create_dir_all(&base_dir)?;
    let zip_path = base_dir.join(zip_filename);

    let file = File::create(&zip_path)?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);

    // 2. Export Metadata
    let meta = ExportMetadata {
        application: "Coalition Governed Control Plane".to_string(),
        version: "0.1.0".to_string(),
        project_id: project_id.to_string(),
        exported_at: now_dt.to_rfc3339(),
        operating_system: std::env::consts::OS.to_string(),
        bounds_applied: BoundsSummary {
            max_sessions: 50,
            max_events: 1000,
            max_permissions: 100,
            max_activity: 200,
            prompt_strategy: "Sha256 hash with 512-byte safe excerpt".to_string(),
            field_max_bytes: 32 * 1024,
        },
    };
    zip.start_file("metadata.json", options)?;
    let meta_json = serde_json::to_string_pretty(&meta)?;
    zip.write_all(meta_json.as_bytes())?;

    // 3. Workflow State
    let wf_state: Option<(String, String)> = db
        .connection()
        .query_row(
            "SELECT state, updated_at FROM workflow_state WHERE project_id = ?1",
            params![project_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();
    zip.start_file("workflow_state.json", options)?;
    let wf_json = serde_json::to_string_pretty(&serde_json::json!({
        "project_id": project_id,
        "state": wf_state.as_ref().map(|s| &s.0),
        "updated_at": wf_state.as_ref().map(|s| &s.1),
    }))?;
    zip.write_all(wf_json.as_bytes())?;

    // 4. Bounded & Sanitized Builder Sessions
    let mut stmt = db.connection().prepare(
        "SELECT session_id, epoch_id, conversation_id, model, effort, icarus_mode, status, prompt, response_text, error_message, started_at, completed_at, duration_ms, input_tokens, output_tokens, thinking_tokens, cache_read_tokens, total_tokens
         FROM builder_sessions WHERE project_id = ?1 ORDER BY started_at DESC LIMIT 50"
    ).map_err(|e| DiagnosticsError::Database(e.to_string()))?;

    let session_rows = stmt
        .query_map(params![project_id], |r| {
            let prompt_raw: String = r.get(7)?;
            let mut hasher = Sha256::new();
            hasher.update(prompt_raw.as_bytes());
            let prompt_hash = format!("{:x}", hasher.finalize());
            let prompt_excerpt = crate::core::process::truncate_utf8_safe(&prompt_raw, 512);

            let resp_raw: Option<String> = r.get(8)?;
            let resp_sanitized = resp_raw.as_deref().map(|s| {
                let bounded = crate::core::process::truncate_utf8_safe(s, 32 * 1024);
                safe_sanitize_text(bounded)
            });

            let err_raw: Option<String> = r.get(9)?;
            let err_sanitized = err_raw.as_deref().map(|s| {
                let bounded = crate::core::process::truncate_utf8_safe(s, 16 * 1024);
                safe_sanitize_text(bounded)
            });

            Ok(serde_json::json!({
                "session_id": r.get::<_, String>(0)?,
                "epoch_id": r.get::<_, String>(1)?,
                "conversation_id": r.get::<_, Option<String>>(2)?,
                "model": r.get::<_, String>(3)?,
                "effort": r.get::<_, Option<String>>(4)?,
                "icarus_mode": r.get::<_, bool>(5)?,
                "status": r.get::<_, String>(6)?,
                "prompt_sha256": prompt_hash,
                "prompt_char_length": prompt_raw.chars().count(),
                "prompt_excerpt": prompt_excerpt,
                "response_text": resp_sanitized,
                "error_message": err_sanitized,
                "started_at": r.get::<_, String>(10)?,
                "completed_at": r.get::<_, Option<String>>(11)?,
                "duration_ms": r.get::<_, i64>(12)?,
                "usage": {
                    "input_tokens": r.get::<_, i64>(13)?,
                    "output_tokens": r.get::<_, i64>(14)?,
                    "thinking_tokens": r.get::<_, i64>(15)?,
                    "cache_read_tokens": r.get::<_, i64>(16)?,
                    "total_tokens": r.get::<_, i64>(17)?,
                }
            }))
        })
        .map_err(|e| DiagnosticsError::Database(e.to_string()))?;

    let mut sessions = Vec::new();
    for row in session_rows {
        sessions.push(row.map_err(|e| DiagnosticsError::Database(e.to_string()))?);
    }
    zip.start_file("sessions.json", options)?;
    let sessions_json = serde_json::to_string_pretty(&sessions)?;
    zip.write_all(sessions_json.as_bytes())?;

    // 5. Bounded & Sanitized Builder Events
    let mut stmt = db
        .connection()
        .prepare(
            "SELECT id, session_id, step_index, event_type, state, content, details_json, timestamp
         FROM builder_events WHERE project_id = ?1 ORDER BY id ASC LIMIT 1000",
        )
        .map_err(|e| DiagnosticsError::Database(e.to_string()))?;

    let event_rows = stmt
        .query_map(params![project_id], |r| {
            let content_raw: Option<String> = r.get(5)?;
            let content_sanitized = content_raw.as_deref().map(|c| {
                let bounded = crate::core::process::truncate_utf8_safe(c, 16 * 1024);
                safe_sanitize_text(bounded)
            });

            let details_raw: Option<String> = r.get(6)?;
            let details_sanitized = details_raw.as_deref().map(|d| {
                let bounded = crate::core::process::truncate_utf8_safe(d, 16 * 1024);
                safe_sanitize_text(bounded)
            });

            Ok(serde_json::json!({
                "id": r.get::<_, i64>(0)?,
                "session_id": r.get::<_, String>(1)?,
                "step_index": r.get::<_, Option<i64>>(2)?,
                "event_type": r.get::<_, String>(3)?,
                "state": r.get::<_, Option<String>>(4)?,
                "content": content_sanitized,
                "details_json": details_sanitized,
                "timestamp": r.get::<_, String>(7)?,
            }))
        })
        .map_err(|e| DiagnosticsError::Database(e.to_string()))?;

    let mut events = Vec::new();
    for row in event_rows {
        events.push(row.map_err(|e| DiagnosticsError::Database(e.to_string()))?);
    }
    zip.start_file("events.json", options)?;
    let events_json = serde_json::to_string_pretty(&events)?;
    zip.write_all(events_json.as_bytes())?;

    // 6. Permission History
    let mut stmt = db
        .connection()
        .prepare(
            "SELECT id, session_id, tool_name, target, risk_level, decision, reason, created_at
         FROM builder_permission_history WHERE project_id = ?1 ORDER BY id DESC LIMIT 100",
        )
        .map_err(|e| DiagnosticsError::Database(e.to_string()))?;

    let perm_rows = stmt
        .query_map(params![project_id], |r| {
            Ok(serde_json::json!({
                "id": r.get::<_, i64>(0)?,
                "session_id": r.get::<_, Option<String>>(1)?,
                "tool_name": r.get::<_, String>(2)?,
                "target": r.get::<_, Option<String>>(3)?,
                "risk_level": r.get::<_, String>(4)?,
                "decision": r.get::<_, String>(5)?,
                "reason": r.get::<_, Option<String>>(6)?,
                "created_at": r.get::<_, String>(7)?,
            }))
        })
        .map_err(|e| DiagnosticsError::Database(e.to_string()))?;

    let mut permissions = Vec::new();
    for row in perm_rows {
        permissions.push(row.map_err(|e| DiagnosticsError::Database(e.to_string()))?);
    }
    zip.start_file("permissions.json", options)?;
    let permissions_json = serde_json::to_string_pretty(&permissions)?;
    zip.write_all(permissions_json.as_bytes())?;

    // 7. Activity Log
    let mut stmt = db
        .connection()
        .prepare(
            "SELECT id, event_type, actor, summary, metadata_json, timestamp
         FROM activity_events WHERE project_id = ?1 ORDER BY id DESC LIMIT 200",
        )
        .map_err(|e| DiagnosticsError::Database(e.to_string()))?;

    let act_rows = stmt
        .query_map(params![project_id], |r| {
            let summary_raw: String = r.get(3)?;
            let summary_sanitized = safe_sanitize_text(&summary_raw);
            let meta_raw: Option<String> = r.get(4)?;
            let meta_sanitized = meta_raw.as_deref().map(safe_sanitize_text);

            Ok(serde_json::json!({
                "id": r.get::<_, i64>(0)?,
                "event_type": r.get::<_, String>(1)?,
                "actor": r.get::<_, String>(2)?,
                "summary": summary_sanitized,
                "metadata": meta_sanitized,
                "timestamp": r.get::<_, String>(5)?,
            }))
        })
        .map_err(|e| DiagnosticsError::Database(e.to_string()))?;

    let mut activities = Vec::new();
    for row in act_rows {
        activities.push(row.map_err(|e| DiagnosticsError::Database(e.to_string()))?);
    }
    zip.start_file("activity.json", options)?;
    let activity_json = serde_json::to_string_pretty(&activities)?;
    zip.write_all(activity_json.as_bytes())?;

    // Finish writing ZIP
    zip.finish()?;

    Ok(zip_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_export_diagnostics_produces_valid_zip() {
        let dir = tempdir().expect("create temp dir");
        let db_path = dir.path().join("test.db");
        let mut db = DbManager::open(&db_path).expect("init db");
        db.run_migrations().expect("run migrations");
        let project_id = "test-proj-diag";

        db.connection()
            .execute(
                "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at) VALUES (?1, ?2, ?3, ?4, ?4, ?4)",
                params![project_id, "Test Project", "/fake/repo", "2026-09-23T00:00:00Z"],
            )
            .expect("insert project");

        let export_dir = dir.path().join("exports");
        let zip_path = export_project_diagnostics(&db, project_id, Some(&export_dir))
            .expect("export diagnostics");

        assert!(zip_path.exists());
        assert!(zip_path.extension().is_some_and(|ext| ext == "zip"));

        // Verify ZIP contents
        let file = File::open(&zip_path).expect("open zip");
        let mut archive = zip::ZipArchive::new(file).expect("read zip archive");

        let expected_files = [
            "metadata.json",
            "workflow_state.json",
            "sessions.json",
            "events.json",
            "permissions.json",
            "activity.json",
        ];

        for expected in expected_files {
            assert!(
                archive.by_name(expected).is_ok(),
                "Expected file '{}' not found in diagnostics ZIP",
                expected
            );
        }
    }
}

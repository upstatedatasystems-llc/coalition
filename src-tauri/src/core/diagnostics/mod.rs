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

/// Redacts local developer filesystem paths (Windows drive paths, UNC paths, and Unix user home paths)
/// while strictly preserving ordinary architecture-relative paths such as `src/log-analyzer.js`
/// or `design/architecture.md`.
pub fn redact_local_paths(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let n = chars.len();
    let mut i = 0;

    while i < n {
        let (byte_idx, ch) = chars[i];

        // 1. Windows drive letter: [A-Za-z]:\ or [A-Za-z]:/ or escaped [A-Za-z]:\\
        let is_drive = ch.is_ascii_alphabetic()
            && i + 1 < n
            && chars[i + 1].1 == ':'
            && i + 2 < n
            && (chars[i + 2].1 == '\\' || chars[i + 2].1 == '/');

        // 2. UNC path: \\server\share or escaped \\\\server\\share
        let is_unc = ch == '\\' && i + 1 < n && chars[i + 1].1 == '\\';

        // 3. Unix user home paths: /home/ or /Users/
        let remaining = &text[byte_idx..];
        let is_unix_home = remaining.starts_with("/home/") || remaining.starts_with("/Users/");

        if is_drive || is_unc || is_unix_home {
            let mut j = i;
            while j < n {
                let (_, c) = chars[j];
                // Delimiters for path in plain text or JSON
                if c.is_whitespace()
                    || c == '"'
                    || c == '\''
                    || c == ','
                    || c == '}'
                    || c == ']'
                    || c == '{'
                    || c == '['
                    || c == '<'
                    || c == '>'
                    || c == ')'
                    || c == '('
                    || c.is_control()
                {
                    break;
                }
                j += 1;
            }

            // Strip trailing sentence punctuation
            while j > i && (chars[j - 1].1 == '.' || chars[j - 1].1 == ';' || chars[j - 1].1 == ':')
            {
                j -= 1;
            }

            if j - i >= 3 {
                result.push_str("[REDACTED_LOCAL_PATH]");
                i = j;
                continue;
            }
        }

        result.push(ch);
        i += 1;
    }

    result
}

/// Applies UTF-8 boundary safe truncation, sensitive token/header redaction,
/// and local developer filesystem path redaction to an exported diagnostic field.
pub fn sanitize_diagnostics_field(text: &str, max_bytes: usize) -> String {
    let bounded = crate::core::process::truncate_utf8_safe(text, max_bytes);
    let secrets_redacted = safe_sanitize_text(bounded);
    redact_local_paths(&secrets_redacted)
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

    // 3. Current Builder Epoch Metadata (Bounded, Non-Sensitive)
    let repo_path_res: Option<String> = db
        .connection()
        .query_row(
            "SELECT repository_path FROM projects WHERE project_id = ?1",
            params![project_id],
            |r| r.get(0),
        )
        .ok();

    let mut epoch_json_val = serde_json::json!({
        "project_id": project_id,
        "builder_epoch_id": serde_json::Value::Null,
        "architecture_version": serde_json::Value::Null,
        "contract_fingerprint": serde_json::Value::Null,
        "git_head_commit": serde_json::Value::Null,
        "git_branch": serde_json::Value::Null,
        "created_at": serde_json::Value::Null,
        "is_frozen": false,
    });

    if let Some(ref repo_str) = repo_path_res {
        let repo_p = Path::new(repo_str);
        if repo_p.exists() {
            if let Ok(packet) = crate::core::freeze::FreezeService::get_builder_packet(repo_p, None)
            {
                epoch_json_val = serde_json::json!({
                    "project_id": project_id,
                    "builder_epoch_id": packet.metadata.builder_epoch_id,
                    "architecture_version": packet.metadata.architecture_version,
                    "contract_fingerprint": packet.metadata.contract_fingerprint,
                    "git_head_commit": packet.metadata.git_head_commit,
                    "git_branch": packet.metadata.git_branch,
                    "created_at": packet.metadata.created_at,
                    "is_frozen": true,
                });
            }
        }
    }

    if epoch_json_val["builder_epoch_id"].is_null() {
        let latest_epoch: Option<String> = db
            .connection()
            .query_row(
                "SELECT epoch_id FROM builder_sessions WHERE project_id = ?1 ORDER BY started_at DESC LIMIT 1",
                params![project_id],
                |r| r.get(0),
            )
            .ok();
        if let Some(ep) = latest_epoch {
            epoch_json_val["builder_epoch_id"] = serde_json::Value::String(ep);
        }
    }

    zip.start_file("epoch.json", options)?;
    let epoch_json_str = serde_json::to_string_pretty(&epoch_json_val)?;
    zip.write_all(epoch_json_str.as_bytes())?;

    // 4. Workflow State
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

    // 5. Bounded & Sanitized Builder Sessions
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
            let prompt_excerpt = sanitize_diagnostics_field(&prompt_raw, 512);

            let resp_raw: Option<String> = r.get(8)?;
            let resp_sanitized = resp_raw
                .as_deref()
                .map(|s| sanitize_diagnostics_field(s, 32 * 1024));

            let err_raw: Option<String> = r.get(9)?;
            let err_sanitized = err_raw
                .as_deref()
                .map(|s| sanitize_diagnostics_field(s, 16 * 1024));

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

    // 6. Bounded & Sanitized Builder Events (Newest 1,000, Chronologically Restored)
    let mut stmt = db
        .connection()
        .prepare(
            "SELECT id, session_id, step_index, event_type, state, content, details_json, timestamp
         FROM builder_events WHERE project_id = ?1 ORDER BY id DESC LIMIT 1000",
        )
        .map_err(|e| DiagnosticsError::Database(e.to_string()))?;

    let event_rows = stmt
        .query_map(params![project_id], |r| {
            let content_raw: Option<String> = r.get(5)?;
            let content_sanitized = content_raw
                .as_deref()
                .map(|c| sanitize_diagnostics_field(c, 16 * 1024));

            let details_raw: Option<String> = r.get(6)?;
            let details_sanitized = details_raw
                .as_deref()
                .map(|d| sanitize_diagnostics_field(d, 16 * 1024));

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
    // Restore chronological ordering for exported output
    events.reverse();
    zip.start_file("events.json", options)?;
    let events_json = serde_json::to_string_pretty(&events)?;
    zip.write_all(events_json.as_bytes())?;

    // 7. Permission History (Bounded & Sanitized)
    let mut stmt = db
        .connection()
        .prepare(
            "SELECT id, session_id, tool_name, target, risk_level, decision, reason, created_at
         FROM builder_permission_history WHERE project_id = ?1 ORDER BY id DESC LIMIT 100",
        )
        .map_err(|e| DiagnosticsError::Database(e.to_string()))?;

    let perm_rows = stmt
        .query_map(params![project_id], |r| {
            let target_raw: Option<String> = r.get(3)?;
            let target_sanitized = target_raw
                .as_deref()
                .map(|t| sanitize_diagnostics_field(t, 4 * 1024));

            let reason_raw: Option<String> = r.get(6)?;
            let reason_sanitized = reason_raw
                .as_deref()
                .map(|reason| sanitize_diagnostics_field(reason, 4 * 1024));

            Ok(serde_json::json!({
                "id": r.get::<_, i64>(0)?,
                "session_id": r.get::<_, Option<String>>(1)?,
                "tool_name": r.get::<_, String>(2)?,
                "target": target_sanitized,
                "risk_level": r.get::<_, String>(4)?,
                "decision": r.get::<_, String>(5)?,
                "reason": reason_sanitized,
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

    // 8. Activity Log (Bounded & Sanitized)
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
            let summary_sanitized = sanitize_diagnostics_field(&summary_raw, 4 * 1024);
            let meta_raw: Option<String> = r.get(4)?;
            let meta_sanitized = meta_raw
                .as_deref()
                .map(|m| sanitize_diagnostics_field(m, 16 * 1024));

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
    use std::io::Read;
    use tempfile::tempdir;

    #[test]
    fn test_redact_local_paths_unit() {
        // Windows drive paths with backslashes
        assert_eq!(
            redact_local_paths(
                "Registered at C:\\Users\\mikea\\Documents\\coalition-stage3-acceptance."
            ),
            "Registered at [REDACTED_LOCAL_PATH]."
        );

        // Windows drive paths with forward slashes
        assert_eq!(
            redact_local_paths("Path: C:/Users/mikea/Documents/coalition-stage3-acceptance"),
            "Path: [REDACTED_LOCAL_PATH]"
        );

        // Escaped Windows drive paths in JSON
        assert_eq!(
            redact_local_paths(
                r#"{"repo": "C:\\Users\\mikea\\Documents\\coalition-stage3-acceptance"}"#
            ),
            r#"{"repo": "[REDACTED_LOCAL_PATH]"}"#
        );

        // UNC paths
        assert_eq!(
            redact_local_paths("Target: \\\\server\\share\\builds\\output.log"),
            "Target: [REDACTED_LOCAL_PATH]"
        );

        // Unix home paths
        assert_eq!(
            redact_local_paths("User dir /home/developer/code/project and /Users/alice/app.js"),
            "User dir [REDACTED_LOCAL_PATH] and [REDACTED_LOCAL_PATH]"
        );

        // Ordinary architecture-relative paths must remain untouched!
        let rel_paths = "Check src/log-analyzer.js and design/architecture.md and Cargo.toml";
        assert_eq!(redact_local_paths(rel_paths), rel_paths);
    }

    #[test]
    fn test_export_diagnostics_produces_valid_zip_with_epoch() {
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
            "epoch.json",
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

    #[test]
    fn test_export_diagnostics_strictly_redacts_local_paths_and_secrets() {
        let dir = tempdir().expect("create temp dir");
        let db_path = dir.path().join("test_redact.db");
        let mut db = DbManager::open(&db_path).expect("init db");
        db.run_migrations().expect("run migrations");
        let project_id = "proj-redact-test";

        let leak_path = r"C:\Users\mikea\Documents\coalition-stage3-acceptance";
        let leak_unix_home = "/home/mikea/private";
        let leak_unc = r"\\server\share\secret_share";
        let secret_token = "sk-live-1234567890abcdef12345";

        // Insert project
        db.connection()
            .execute(
                "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at) VALUES (?1, ?2, ?3, ?4, ?4, ?4)",
                params![project_id, "Redaction Project", leak_path, "2026-09-23T00:00:00Z"],
            )
            .expect("insert project");

        // 1. Activity event with local path in summary and metadata
        let act_summary = format!("Project registered at {} in local filesystem", leak_path);
        let act_meta = format!(r#"{{"repository_path": "{}"}}"#, leak_path);
        db.connection()
            .execute(
                "INSERT INTO activity_events (project_id, timestamp, event_type, actor, summary, metadata_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![project_id, "2026-09-23T00:00:00Z", "PROJECT_REGISTERED", "HUMAN", act_summary, act_meta],
            )
            .expect("insert activity");

        // 2. Builder session with local path and secret in prompt excerpt, response, and error
        let sess_prompt = format!(
            "Build project located at {} with Authorization: Bearer {}",
            leak_path, secret_token
        );
        let sess_resp = format!("Done. Output written to {}", leak_unc);
        let sess_err = format!("Failed checking {}", leak_unix_home);
        db.connection()
            .execute(
                "INSERT INTO builder_sessions (session_id, project_id, epoch_id, model, icarus_mode, status, prompt, response_text, error_message, started_at, duration_ms, input_tokens, output_tokens, thinking_tokens, cache_read_tokens, total_tokens) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                params![
                    "sess-redact-1",
                    project_id,
                    "epoch-redact-1",
                    "gemini-3.8-flash-high",
                    false,
                    "SUCCESS",
                    sess_prompt,
                    sess_resp,
                    sess_err,
                    "2026-09-23T00:01:00Z",
                    1000,
                    10,
                    20,
                    0,
                    0,
                    30
                ],
            )
            .expect("insert session");

        // 3. Permission record with local path in target and reason
        db.connection()
            .execute(
                "INSERT INTO builder_permission_history (project_id, session_id, tool_name, target, risk_level, decision, reason, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    project_id,
                    "sess-redact-1",
                    "write_file",
                    format!(r"{}\package.json", leak_path),
                    "MUTATING",
                    "BLOCKED",
                    format!("Denied mutation for {}", leak_unix_home),
                    "2026-09-23T00:01:05Z"
                ],
            )
            .expect("insert permission");

        // 4. Builder event with local path in content and details
        db.connection()
            .execute(
                "INSERT INTO builder_events (project_id, session_id, step_index, event_type, state, content, details_json, timestamp) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    project_id,
                    "sess-redact-1",
                    1,
                    "step_update",
                    "running",
                    format!("Accessed {}", leak_unc),
                    format!(r#"{{"path": "{}"}}"#, leak_path),
                    "2026-09-23T00:01:02Z"
                ],
            )
            .expect("insert event");

        let export_dir = dir.path().join("exports");
        let zip_path = export_project_diagnostics(&db, project_id, Some(&export_dir))
            .expect("export diagnostics");

        let file = File::open(&zip_path).expect("open zip");
        let mut archive = zip::ZipArchive::new(file).expect("read zip archive");

        // Inspect all members: literal leak paths and secrets MUST be absent from EVERY file
        for i in 0..archive.len() {
            let mut zip_file = archive.by_index(i).expect("get zip entry");
            let mut content = String::new();
            zip_file
                .read_to_string(&mut content)
                .expect("read zip entry text");

            let filename = zip_file.name().to_string();

            assert!(
                !content.contains(leak_path),
                "Literal local path '{}' found in member '{}'!\nContent:\n{}",
                leak_path,
                filename,
                content
            );
            assert!(
                !content.contains(leak_unix_home),
                "Literal Unix home path '{}' found in member '{}'!\nContent:\n{}",
                leak_unix_home,
                filename,
                content
            );
            assert!(
                !content.contains(leak_unc),
                "Literal UNC path '{}' found in member '{}'!\nContent:\n{}",
                leak_unc,
                filename,
                content
            );
            assert!(
                !content.contains(secret_token),
                "Secret token found in member '{}'!\nContent:\n{}",
                filename,
                content
            );
        }

        // Verify [REDACTED_LOCAL_PATH] placeholder exists in sanitized members
        {
            let mut act_file = archive
                .by_name("activity.json")
                .expect("open activity.json");
            let mut act_content = String::new();
            act_file
                .read_to_string(&mut act_content)
                .expect("read activity.json");
            assert!(act_content.contains("[REDACTED_LOCAL_PATH]"));
        }

        {
            let mut sess_file = archive
                .by_name("sessions.json")
                .expect("open sessions.json");
            let mut sess_content = String::new();
            sess_file
                .read_to_string(&mut sess_content)
                .expect("read sessions.json");
            assert!(sess_content.contains("[REDACTED_LOCAL_PATH]"));
        }

        {
            let mut perm_file = archive
                .by_name("permissions.json")
                .expect("open permissions.json");
            let mut perm_content = String::new();
            perm_file
                .read_to_string(&mut perm_content)
                .expect("read permissions.json");
            assert!(perm_content.contains("[REDACTED_LOCAL_PATH]"));
        }

        {
            let mut evt_file = archive.by_name("events.json").expect("open events.json");
            let mut evt_content = String::new();
            evt_file
                .read_to_string(&mut evt_content)
                .expect("read events.json");
            assert!(evt_content.contains("[REDACTED_LOCAL_PATH]"));
        }
    }
}

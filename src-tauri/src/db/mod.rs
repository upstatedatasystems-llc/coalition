use crate::core::builder::{
    AgyUsage, BuilderEventRecord, BuilderSessionRecord, ChatGptUsageSummary, IcarusState,
    PermissionRecord, PermissionRuleRecord,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DbError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Migration error: {0}")]
    Migration(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationRecord {
    pub version: i64,
    pub name: String,
    pub applied_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofResult {
    pub applied_migrations: Vec<MigrationRecord>,
    pub test_record_id: i64,
    pub test_record_message: String,
    pub total_records: i64,
}

pub struct DbManager {
    conn: Connection,
}

impl DbManager {
    pub fn new_in_memory() -> Result<Self, DbError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(Self { conn })
    }

    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, DbError> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(Self { conn })
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    pub fn connection_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    pub fn run_migrations(&mut self) -> Result<Vec<MigrationRecord>, DbError> {
        // Ensure migration tracking table exists
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS _coalition_migrations (
                version INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                applied_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );",
            [],
        )?;

        let migrations: Vec<(i64, &'static str, &'static str)> = vec![
            (
                1,
                "001_initial_operational_schema",
                "CREATE TABLE IF NOT EXISTS operational_proof_records (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    message TEXT NOT NULL,
                    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
                );
                CREATE TABLE IF NOT EXISTS builder_sessions (
                    id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL,
                    conversation_id TEXT NOT NULL,
                    model TEXT NOT NULL,
                    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
                );",
            ),
            (
                2,
                "002_phase1_core_persistence",
                "CREATE TABLE IF NOT EXISTS projects (
                    project_id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    repository_path TEXT NOT NULL UNIQUE,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    last_opened_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS workflow_state (
                    project_id TEXT PRIMARY KEY REFERENCES projects(project_id) ON DELETE CASCADE,
                    state TEXT NOT NULL,
                    resume_state TEXT,
                    revision INTEGER NOT NULL DEFAULT 1,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS activity_events (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
                    timestamp TEXT NOT NULL,
                    event_type TEXT NOT NULL,
                    actor TEXT NOT NULL,
                    summary TEXT NOT NULL,
                    metadata_json TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS app_settings (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );",
            ),
            (
                3,
                "003_phase2a_relay_persistence",
                "CREATE TABLE IF NOT EXISTS relay_packets (
                    packet_id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
                    role TEXT NOT NULL,
                    packet_type TEXT NOT NULL,
                    architecture_version TEXT NOT NULL,
                    expected_response TEXT NOT NULL,
                    prompt TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    status TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS relay_imports (
                    import_id TEXT PRIMARY KEY,
                    packet_id TEXT REFERENCES relay_packets(packet_id) ON DELETE SET NULL,
                    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
                    raw_content TEXT NOT NULL,
                    parsed_payload_json TEXT,
                    parse_status TEXT NOT NULL,
                    error_message TEXT,
                    decision TEXT,
                    imported_at TEXT NOT NULL,
                    decided_at TEXT
                );",
            ),
            (
                4,
                "004_phase2a_batch_journal_and_history",
                "CREATE TABLE IF NOT EXISTS relay_import_batch_journal (
                    import_id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
                    packet_id TEXT NOT NULL,
                    phase TEXT NOT NULL,
                    operations_json TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE UNIQUE INDEX IF NOT EXISTS idx_batch_journal_project ON relay_import_batch_journal(project_id);
                CREATE TABLE IF NOT EXISTS relay_history (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
                    packet_id TEXT,
                    import_id TEXT,
                    event_type TEXT NOT NULL,
                    summary TEXT NOT NULL,
                    details_json TEXT,
                    created_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_relay_packets_proj_status ON relay_packets(project_id, status);
                CREATE INDEX IF NOT EXISTS idx_relay_imports_proj_decision ON relay_imports(project_id, decision);
                CREATE INDEX IF NOT EXISTS idx_relay_history_proj ON relay_history(project_id, id);",
            ),
            (
                5,
                "005_stage2_freeze_and_builder_epochs",
                "CREATE TABLE IF NOT EXISTS builder_epochs (
                    epoch_id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
                    architecture_version TEXT NOT NULL,
                    git_commit TEXT,
                    git_branch TEXT,
                    created_at TEXT NOT NULL,
                    status TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_builder_epochs_proj ON builder_epochs(project_id, architecture_version);
                CREATE TABLE IF NOT EXISTS frozen_boundaries (
                    boundary_id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
                    architecture_version TEXT NOT NULL,
                    git_commit TEXT,
                    git_branch TEXT,
                    is_clean INTEGER NOT NULL,
                    staged_count INTEGER NOT NULL,
                    unstaged_count INTEGER NOT NULL,
                    untracked_count INTEGER NOT NULL,
                    dirty_fingerprint TEXT NOT NULL,
                    snapshot_path TEXT NOT NULL,
                    manifest_fingerprint TEXT NOT NULL,
                    frozen_at TEXT NOT NULL,
                    frozen_by TEXT NOT NULL
                );
                CREATE UNIQUE INDEX IF NOT EXISTS idx_frozen_boundaries_proj_ver ON frozen_boundaries(project_id, architecture_version);",
            ),
            (
                6,
                "006_stage2_freeze_previews_and_drift_journals",
                "CREATE TABLE IF NOT EXISTS freeze_previews (
                    preview_id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
                    target_version TEXT NOT NULL,
                    readiness_policy_version INTEGER NOT NULL,
                    preview_json TEXT NOT NULL,
                    status TEXT NOT NULL,
                    created_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_freeze_previews_proj_status ON freeze_previews(project_id, status);
                CREATE TABLE IF NOT EXISTS drift_restoration_journals (
                    journal_id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
                    architecture_version TEXT NOT NULL,
                    phase TEXT NOT NULL,
                    operations_json TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE UNIQUE INDEX IF NOT EXISTS idx_drift_journal_project ON drift_restoration_journals(project_id);",
            ),
            (
                7,
                "007_stage3_builder_control_plane",
                "CREATE TABLE IF NOT EXISTS builder_sessions_v2 (
                    session_id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
                    epoch_id TEXT NOT NULL,
                    conversation_id TEXT,
                    model TEXT NOT NULL,
                    effort TEXT,
                    icarus_mode INTEGER NOT NULL DEFAULT 0,
                    status TEXT NOT NULL DEFAULT 'COMPLETED',
                    prompt TEXT NOT NULL DEFAULT '',
                    response_text TEXT,
                    error_message TEXT,
                    started_at TEXT NOT NULL,
                    completed_at TEXT,
                    duration_ms INTEGER DEFAULT 0,
                    input_tokens INTEGER DEFAULT 0,
                    output_tokens INTEGER DEFAULT 0,
                    thinking_tokens INTEGER DEFAULT 0,
                    cache_read_tokens INTEGER DEFAULT 0,
                    total_tokens INTEGER DEFAULT 0
                );
                INSERT OR IGNORE INTO builder_sessions_v2 (session_id, project_id, epoch_id, conversation_id, model, started_at)
                    SELECT id, project_id, 'legacy', conversation_id, model, created_at FROM builder_sessions;
                DROP TABLE builder_sessions;
                ALTER TABLE builder_sessions_v2 RENAME TO builder_sessions;
                CREATE INDEX IF NOT EXISTS idx_builder_sessions_proj ON builder_sessions(project_id, epoch_id);

                CREATE TABLE IF NOT EXISTS builder_events (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    session_id TEXT NOT NULL REFERENCES builder_sessions(session_id) ON DELETE CASCADE,
                    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
                    step_index INTEGER,
                    event_type TEXT NOT NULL,
                    state TEXT,
                    content TEXT,
                    details_json TEXT,
                    timestamp TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_builder_events_session ON builder_events(session_id, id);

                CREATE TABLE IF NOT EXISTS chatgpt_usage_records (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
                    packet_id TEXT,
                    direction TEXT NOT NULL,
                    char_count INTEGER NOT NULL,
                    estimated_tokens INTEGER NOT NULL,
                    timestamp TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_chatgpt_usage_proj_time ON chatgpt_usage_records(project_id, timestamp);

                CREATE TABLE IF NOT EXISTS chatgpt_calibration_settings (
                    project_id TEXT PRIMARY KEY REFERENCES projects(project_id) ON DELETE CASCADE,
                    reset_at TEXT,
                    offset_tokens INTEGER DEFAULT 0,
                    notes TEXT
                );

                CREATE TABLE IF NOT EXISTS builder_permission_rules (
                    rule_id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
                    tool_name TEXT NOT NULL,
                    pattern TEXT,
                    decision TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    expires_at TEXT
                );
                CREATE INDEX IF NOT EXISTS idx_perm_rules_proj ON builder_permission_rules(project_id);

                CREATE TABLE IF NOT EXISTS builder_permission_history (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
                    session_id TEXT,
                    tool_name TEXT NOT NULL,
                    target TEXT,
                    risk_level TEXT NOT NULL,
                    decision TEXT NOT NULL,
                    reason TEXT,
                    created_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_perm_hist_proj ON builder_permission_history(project_id, id);

                CREATE TABLE IF NOT EXISTS project_icarus_state (
                    project_id TEXT PRIMARY KEY REFERENCES projects(project_id) ON DELETE CASCADE,
                    enabled INTEGER NOT NULL DEFAULT 0,
                    enabled_at TEXT,
                    enabled_by TEXT
                );",
            ),
        ];

        let mut applied = Vec::new();

        for (version, name, sql) in migrations {
            let count: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM _coalition_migrations WHERE version = ?1",
                params![version],
                |row| row.get(0),
            )?;

            if count == 0 {
                let tx = self.conn.transaction()?;
                tx.execute_batch(sql)?;
                tx.execute(
                    "INSERT INTO _coalition_migrations (version, name) VALUES (?1, ?2)",
                    params![version, name],
                )?;
                tx.commit()?;

                applied.push(MigrationRecord {
                    version,
                    name: name.to_string(),
                    applied_at: chrono::Utc::now().to_rfc3339(),
                });
            }
        }

        Ok(applied)
    }

    pub fn run_proof(&mut self) -> Result<ProofResult, DbError> {
        let applied_migrations = self.run_migrations()?;

        // Perform operational test write
        let test_message = format!("Proof write at {}", chrono::Utc::now().to_rfc3339());
        self.conn.execute(
            "INSERT INTO operational_proof_records (message) VALUES (?1)",
            params![test_message],
        )?;

        let last_id = self.conn.last_insert_rowid();

        let total_records: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM operational_proof_records",
            [],
            |row| row.get(0),
        )?;

        Ok(ProofResult {
            applied_migrations,
            test_record_id: last_id,
            test_record_message: test_message,
            total_records,
        })
    }

    // Builder Sessions CRUD
    pub fn insert_builder_session(&self, s: &BuilderSessionRecord) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO builder_sessions (
                session_id, project_id, epoch_id, conversation_id, model, effort,
                icarus_mode, status, prompt, response_text, error_message, started_at,
                completed_at, duration_ms, input_tokens, output_tokens, thinking_tokens,
                cache_read_tokens, total_tokens
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
            params![
                s.session_id,
                s.project_id,
                s.epoch_id,
                s.conversation_id,
                s.model,
                s.effort,
                if s.icarus_mode { 1 } else { 0 },
                s.status,
                s.prompt,
                s.response_text,
                s.error_message,
                s.started_at,
                s.completed_at,
                s.duration_ms as i64,
                s.usage.input_tokens as i64,
                s.usage.output_tokens as i64,
                s.usage.thinking_tokens as i64,
                s.usage.cache_read_tokens as i64,
                s.usage.total_tokens as i64,
            ],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_builder_session_status(
        &self,
        session_id: &str,
        status: &str,
        response_text: Option<&str>,
        error_message: Option<&str>,
        completed_at: Option<&str>,
        duration_ms: u64,
        usage: &AgyUsage,
        conversation_id: Option<&str>,
    ) -> Result<(), DbError> {
        self.conn.execute(
            "UPDATE builder_sessions SET
                status = ?1,
                response_text = coalesce(?2, response_text),
                error_message = coalesce(?3, error_message),
                completed_at = coalesce(?4, completed_at),
                duration_ms = ?5,
                input_tokens = ?6,
                output_tokens = ?7,
                thinking_tokens = ?8,
                cache_read_tokens = ?9,
                total_tokens = ?10,
                conversation_id = coalesce(?11, conversation_id)
            WHERE session_id = ?12",
            params![
                status,
                response_text,
                error_message,
                completed_at,
                duration_ms as i64,
                usage.input_tokens as i64,
                usage.output_tokens as i64,
                usage.thinking_tokens as i64,
                usage.cache_read_tokens as i64,
                usage.total_tokens as i64,
                conversation_id,
                session_id,
            ],
        )?;
        Ok(())
    }

    pub fn get_builder_session(
        &self,
        session_id: &str,
    ) -> Result<Option<BuilderSessionRecord>, DbError> {
        let res = self
            .conn
            .query_row(
                "SELECT session_id, project_id, epoch_id, conversation_id, model, effort,
                    icarus_mode, status, prompt, response_text, error_message, started_at,
                    completed_at, duration_ms, input_tokens, output_tokens, thinking_tokens,
                    cache_read_tokens, total_tokens
             FROM builder_sessions WHERE session_id = ?1",
                params![session_id],
                |r| {
                    let icarus: i32 = r.get(6)?;
                    let duration_ms: i64 = r.get(13)?;
                    let input_tokens: i64 = r.get(14)?;
                    let output_tokens: i64 = r.get(15)?;
                    let thinking_tokens: i64 = r.get(16)?;
                    let cache_read_tokens: i64 = r.get(17)?;
                    let total_tokens: i64 = r.get(18)?;

                    Ok(BuilderSessionRecord {
                        session_id: r.get(0)?,
                        project_id: r.get(1)?,
                        epoch_id: r.get(2)?,
                        conversation_id: r.get(3)?,
                        model: r.get(4)?,
                        effort: r.get(5)?,
                        icarus_mode: icarus != 0,
                        status: r.get(7)?,
                        prompt: r.get(8)?,
                        response_text: r.get(9)?,
                        error_message: r.get(10)?,
                        started_at: r.get(11)?,
                        completed_at: r.get(12)?,
                        duration_ms: duration_ms as u64,
                        usage: AgyUsage {
                            input_tokens: input_tokens as u64,
                            output_tokens: output_tokens as u64,
                            thinking_tokens: thinking_tokens as u64,
                            cache_read_tokens: cache_read_tokens as u64,
                            total_tokens: total_tokens as u64,
                        },
                    })
                },
            )
            .optional()?;
        Ok(res)
    }

    pub fn get_latest_builder_session(
        &self,
        project_id: &str,
    ) -> Result<Option<BuilderSessionRecord>, DbError> {
        let res = self
            .conn
            .query_row(
                "SELECT session_id, project_id, epoch_id, conversation_id, model, effort,
                    icarus_mode, status, prompt, response_text, error_message, started_at,
                    completed_at, duration_ms, input_tokens, output_tokens, thinking_tokens,
                    cache_read_tokens, total_tokens
             FROM builder_sessions WHERE project_id = ?1
             ORDER BY started_at DESC, session_id DESC LIMIT 1",
                params![project_id],
                |r| {
                    let icarus: i32 = r.get(6)?;
                    let duration_ms: i64 = r.get(13)?;
                    let input_tokens: i64 = r.get(14)?;
                    let output_tokens: i64 = r.get(15)?;
                    let thinking_tokens: i64 = r.get(16)?;
                    let cache_read_tokens: i64 = r.get(17)?;
                    let total_tokens: i64 = r.get(18)?;

                    Ok(BuilderSessionRecord {
                        session_id: r.get(0)?,
                        project_id: r.get(1)?,
                        epoch_id: r.get(2)?,
                        conversation_id: r.get(3)?,
                        model: r.get(4)?,
                        effort: r.get(5)?,
                        icarus_mode: icarus != 0,
                        status: r.get(7)?,
                        prompt: r.get(8)?,
                        response_text: r.get(9)?,
                        error_message: r.get(10)?,
                        started_at: r.get(11)?,
                        completed_at: r.get(12)?,
                        duration_ms: duration_ms as u64,
                        usage: AgyUsage {
                            input_tokens: input_tokens as u64,
                            output_tokens: output_tokens as u64,
                            thinking_tokens: thinking_tokens as u64,
                            cache_read_tokens: cache_read_tokens as u64,
                            total_tokens: total_tokens as u64,
                        },
                    })
                },
            )
            .optional()?;
        Ok(res)
    }

    pub fn list_builder_sessions(
        &self,
        project_id: &str,
        limit: usize,
    ) -> Result<Vec<BuilderSessionRecord>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id, project_id, epoch_id, conversation_id, model, effort,
                    icarus_mode, status, prompt, response_text, error_message, started_at,
                    completed_at, duration_ms, input_tokens, output_tokens, thinking_tokens,
                    cache_read_tokens, total_tokens
             FROM builder_sessions WHERE project_id = ?1
             ORDER BY started_at DESC LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![project_id, limit as i64], |r| {
            let icarus: i32 = r.get(6)?;
            let duration_ms: i64 = r.get(13)?;
            let input_tokens: i64 = r.get(14)?;
            let output_tokens: i64 = r.get(15)?;
            let thinking_tokens: i64 = r.get(16)?;
            let cache_read_tokens: i64 = r.get(17)?;
            let total_tokens: i64 = r.get(18)?;

            Ok(BuilderSessionRecord {
                session_id: r.get(0)?,
                project_id: r.get(1)?,
                epoch_id: r.get(2)?,
                conversation_id: r.get(3)?,
                model: r.get(4)?,
                effort: r.get(5)?,
                icarus_mode: icarus != 0,
                status: r.get(7)?,
                prompt: r.get(8)?,
                response_text: r.get(9)?,
                error_message: r.get(10)?,
                started_at: r.get(11)?,
                completed_at: r.get(12)?,
                duration_ms: duration_ms as u64,
                usage: AgyUsage {
                    input_tokens: input_tokens as u64,
                    output_tokens: output_tokens as u64,
                    thinking_tokens: thinking_tokens as u64,
                    cache_read_tokens: cache_read_tokens as u64,
                    total_tokens: total_tokens as u64,
                },
            })
        })?;

        let mut list = Vec::new();
        for row in rows {
            list.push(row?);
        }
        Ok(list)
    }

    /// Reconciles any orphaned sessions marked RUNNING when Coalition starts up.
    /// Returns the number of sessions reconciled.
    pub fn reconcile_orphaned_sessions(&self) -> Result<usize, DbError> {
        let now = chrono::Utc::now().to_rfc3339();
        let count = self.conn.execute(
            "UPDATE builder_sessions
             SET status = 'INTERRUPTED',
                 completed_at = ?1,
                 error_message = 'Session interrupted by application shutdown or process loss'
             WHERE status = 'RUNNING'",
            params![now],
        )?;
        Ok(count)
    }

    // Builder Events
    pub fn insert_builder_event(&self, e: &BuilderEventRecord) -> Result<i64, DbError> {
        self.conn.execute(
            "INSERT INTO builder_events (
                session_id, project_id, step_index, event_type, state, content, details_json, timestamp
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                e.session_id,
                e.project_id,
                e.step_index,
                e.event_type,
                e.state,
                e.content,
                e.details_json,
                e.timestamp,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_builder_events(
        &self,
        session_id: &str,
    ) -> Result<Vec<BuilderEventRecord>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, project_id, step_index, event_type, state, content, details_json, timestamp
             FROM builder_events WHERE session_id = ?1 ORDER BY id ASC",
        )?;

        let rows = stmt.query_map(params![session_id], |r| {
            Ok(BuilderEventRecord {
                id: r.get(0)?,
                session_id: r.get(1)?,
                project_id: r.get(2)?,
                step_index: r.get(3)?,
                event_type: r.get(4)?,
                state: r.get(5)?,
                content: r.get(6)?,
                details_json: r.get(7)?,
                timestamp: r.get(8)?,
            })
        })?;

        let mut list = Vec::new();
        for row in rows {
            list.push(row?);
        }
        Ok(list)
    }

    // ChatGPT Usage & Capacity Telemetry
    pub fn record_chatgpt_usage(
        &self,
        project_id: &str,
        packet_id: Option<&str>,
        direction: &str,
        char_count: usize,
        estimated_tokens: usize,
    ) -> Result<(), DbError> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO chatgpt_usage_records (
                project_id, packet_id, direction, char_count, estimated_tokens, timestamp
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                project_id,
                packet_id,
                direction,
                char_count as i64,
                estimated_tokens as i64,
                now
            ],
        )?;
        Ok(())
    }

    pub fn get_chatgpt_usage_summary(
        &self,
        project_id: &str,
    ) -> Result<ChatGptUsageSummary, DbError> {
        let cal_reset_at: Option<String> = self
            .conn
            .query_row(
                "SELECT reset_at FROM chatgpt_calibration_settings WHERE project_id = ?1",
                params![project_id],
                |r| r.get(0),
            )
            .optional()?;

        let reset_filter = cal_reset_at.as_deref().unwrap_or("1970-01-01T00:00:00Z");

        let five_hours_ago = (chrono::Utc::now() - chrono::Duration::hours(5)).to_rfc3339();
        let seven_days_ago = (chrono::Utc::now() - chrono::Duration::days(7)).to_rfc3339();

        let rolling_5h_tokens: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(estimated_tokens), 0) FROM chatgpt_usage_records
             WHERE project_id = ?1 AND timestamp >= ?2 AND timestamp >= ?3",
            params![project_id, five_hours_ago, reset_filter],
            |r| r.get(0),
        )?;

        let rolling_7d_tokens: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(estimated_tokens), 0) FROM chatgpt_usage_records
             WHERE project_id = ?1 AND timestamp >= ?2 AND timestamp >= ?3",
            params![project_id, seven_days_ago, reset_filter],
            |r| r.get(0),
        )?;

        let total_tokens: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(estimated_tokens), 0) FROM chatgpt_usage_records
             WHERE project_id = ?1 AND timestamp >= ?2",
            params![project_id, reset_filter],
            |r| r.get(0),
        )?;

        let total_packets_sent: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM chatgpt_usage_records
             WHERE project_id = ?1 AND direction = 'OUTBOUND_PACKET' AND timestamp >= ?2",
            params![project_id, reset_filter],
            |r| r.get(0),
        )?;

        let total_imports_received: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM chatgpt_usage_records
             WHERE project_id = ?1 AND direction = 'INBOUND_IMPORT' AND timestamp >= ?2",
            params![project_id, reset_filter],
            |r| r.get(0),
        )?;

        Ok(ChatGptUsageSummary {
            rolling_5h_tokens: rolling_5h_tokens as u64,
            rolling_7d_tokens: rolling_7d_tokens as u64,
            total_tokens: total_tokens as u64,
            total_packets_sent: total_packets_sent as u64,
            total_imports_received: total_imports_received as u64,
            last_calibrated_at: cal_reset_at,
            disclaimer: "Estimated relay throughput based on character heuristics (~4 chars/token). ChatGPT Plus message caps and rate limits are managed by OpenAI and are not provider-reported here.".to_string(),
        })
    }

    pub fn reset_chatgpt_usage(&self, project_id: &str) -> Result<(), DbError> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO chatgpt_calibration_settings (project_id, reset_at, offset_tokens, notes)
             VALUES (?1, ?2, 0, 'User reset')
             ON CONFLICT(project_id) DO UPDATE SET reset_at = excluded.reset_at, notes = excluded.notes",
            params![project_id, now],
        )?;
        Ok(())
    }

    // Permission History & Rules
    pub fn record_permission_history(&self, r: &PermissionRecord) -> Result<i64, DbError> {
        self.conn.execute(
            "INSERT INTO builder_permission_history (
                project_id, session_id, tool_name, target, risk_level, decision, reason, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                r.project_id,
                r.session_id,
                r.tool_name,
                r.target,
                r.risk_level,
                r.decision,
                r.reason,
                r.created_at,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_permission_history(
        &self,
        project_id: &str,
        limit: usize,
    ) -> Result<Vec<PermissionRecord>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, session_id, tool_name, target, risk_level, decision, reason, created_at
             FROM builder_permission_history WHERE project_id = ?1 ORDER BY id DESC LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![project_id, limit as i64], |r| {
            Ok(PermissionRecord {
                id: r.get(0)?,
                project_id: r.get(1)?,
                session_id: r.get(2)?,
                tool_name: r.get(3)?,
                target: r.get(4)?,
                risk_level: r.get(5)?,
                decision: r.get(6)?,
                reason: r.get(7)?,
                created_at: r.get(8)?,
            })
        })?;

        let mut list = Vec::new();
        for row in rows {
            list.push(row?);
        }
        Ok(list)
    }

    // Icarus Mode State
    pub fn get_icarus_state(&self, project_id: &str) -> Result<IcarusState, DbError> {
        let res = self.conn.query_row(
            "SELECT enabled, enabled_at, enabled_by FROM project_icarus_state WHERE project_id = ?1",
            params![project_id],
            |r| {
                let enabled_num: i32 = r.get(0)?;
                Ok(IcarusState {
                    project_id: project_id.to_string(),
                    enabled: enabled_num != 0,
                    enabled_at: r.get(1)?,
                    enabled_by: r.get(2)?,
                })
            },
        ).optional()?;

        Ok(res.unwrap_or(IcarusState {
            project_id: project_id.to_string(),
            enabled: false,
            enabled_at: None,
            enabled_by: None,
        }))
    }

    pub fn set_icarus_state(
        &self,
        project_id: &str,
        enabled: bool,
        actor: Option<&str>,
    ) -> Result<(), DbError> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO project_icarus_state (project_id, enabled, enabled_at, enabled_by)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(project_id) DO UPDATE SET
                enabled = excluded.enabled,
                enabled_at = excluded.enabled_at,
                enabled_by = excluded.enabled_by",
            params![
                project_id,
                if enabled { 1 } else { 0 },
                if enabled { Some(now) } else { None },
                actor
            ],
        )?;
        Ok(())
    }

    pub fn add_permission_rule(&self, rule: &PermissionRuleRecord) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO builder_permission_rules (
                rule_id, project_id, tool_name, pattern, decision, created_at, expires_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                rule.rule_id,
                rule.project_id,
                rule.tool_name,
                rule.pattern,
                rule.decision,
                rule.created_at,
                rule.expires_at,
            ],
        )?;
        Ok(())
    }

    pub fn list_permission_rules(
        &self,
        project_id: &str,
    ) -> Result<Vec<PermissionRuleRecord>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT rule_id, project_id, tool_name, pattern, decision, created_at, expires_at
             FROM builder_permission_rules WHERE project_id = ?1 ORDER BY created_at DESC",
        )?;

        let rows = stmt.query_map(params![project_id], |r| {
            Ok(PermissionRuleRecord {
                rule_id: r.get(0)?,
                project_id: r.get(1)?,
                tool_name: r.get(2)?,
                pattern: r.get(3)?,
                decision: r.get(4)?,
                created_at: r.get(5)?,
                expires_at: r.get(6)?,
            })
        })?;

        let mut list = Vec::new();
        for row in rows {
            list.push(row?);
        }
        Ok(list)
    }

    pub fn delete_permission_rule(&self, rule_id: &str) -> Result<bool, DbError> {
        let rows = self.conn.execute(
            "DELETE FROM builder_permission_rules WHERE rule_id = ?1",
            params![rule_id],
        )?;
        Ok(rows > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sqlite_in_memory_migrations_and_proof() {
        let mut db = DbManager::new_in_memory().expect("in memory db");
        let result = db.run_proof().expect("run proof");
        assert_eq!(result.applied_migrations.len(), 7);
        assert_eq!(result.applied_migrations[0].version, 1);
        assert_eq!(result.applied_migrations[1].version, 2);
        assert_eq!(result.applied_migrations[2].version, 3);
        assert_eq!(result.applied_migrations[3].version, 4);
        assert_eq!(result.applied_migrations[4].version, 5);
        assert_eq!(result.applied_migrations[5].version, 6);
        assert_eq!(result.applied_migrations[6].version, 7);
        assert_eq!(result.test_record_id, 1);
        assert_eq!(result.total_records, 1);

        // Run proof second time: migrations already applied, records increment
        let result2 = db.run_proof().expect("run proof second time");
        assert_eq!(result2.applied_migrations.len(), 0);
        assert_eq!(result2.test_record_id, 2);
        assert_eq!(result2.total_records, 2);
    }

    #[test]
    fn test_migrations_phase0_to_phase1() {
        let conn = Connection::open_in_memory().expect("raw conn");
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        // Simulate Phase 0 DB state
        conn.execute(
            "CREATE TABLE _coalition_migrations (
                version INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                applied_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE operational_proof_records (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                message TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE builder_sessions (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL,
                conversation_id TEXT NOT NULL,
                model TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO _coalition_migrations (version, name) VALUES (1, '001_initial_operational_schema')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO operational_proof_records (message) VALUES ('Phase 0 test')",
            [],
        )
        .unwrap();

        let mut db = DbManager { conn };
        let applied = db.run_migrations().expect("run forward migrations");
        assert_eq!(applied.len(), 6);
        assert_eq!(applied[0].version, 2);
        assert_eq!(applied[1].version, 3);
        assert_eq!(applied[2].version, 4);
        assert_eq!(applied[3].version, 5);
        assert_eq!(applied[4].version, 6);
        assert_eq!(applied[5].version, 7);

        // Verify Phase 0 data preserved
        let count: i64 = db
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM operational_proof_records WHERE message = 'Phase 0 test'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        // Verify Phase 1 tables exist and are usable
        db.connection()
            .execute(
                "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at)
                 VALUES ('p1', 'Test', '/path/test', '2026-09-08T00:00:00Z', '2026-09-08T00:00:00Z', '2026-09-08T00:00:00Z')",
                [],
            )
            .unwrap();

        // Verify Phase 2A tables exist and are usable
        db.connection()
            .execute(
                "INSERT INTO relay_packets (packet_id, project_id, role, packet_type, architecture_version, expected_response, prompt, created_at, status)
                 VALUES ('pkt1', 'p1', 'ARCHITECT', 'ARCHITECT_INITIAL', 'draft', 'ARCHITECT_UPDATE', 'Test prompt', '2026-09-08T00:00:00Z', 'PENDING')",
                [],
            )
            .unwrap();
    }

    #[test]
    fn test_migrations_already_migrated_is_idempotent() {
        let mut db = DbManager::new_in_memory().expect("in memory db");
        let applied1 = db.run_migrations().expect("first migration run");
        assert_eq!(applied1.len(), 7);

        let applied2 = db.run_migrations().expect("second migration run");
        assert_eq!(applied2.len(), 0);
    }

    #[test]
    fn test_foreign_keys_enforced() {
        let mut db = DbManager::new_in_memory().expect("in memory db");
        db.run_migrations().expect("migrations");

        // Attempt to insert workflow_state for non-existent project
        let res = db.connection().execute(
            "INSERT INTO workflow_state (project_id, state, revision, updated_at) VALUES ('non-existent', 'DRAFT', 1, 'now')",
            [],
        );
        assert!(
            res.is_err(),
            "Foreign key constraint must reject orphan workflow_state"
        );

        // Attempt to insert activity_event for non-existent project
        let res2 = db.connection().execute(
            "INSERT INTO activity_events (project_id, timestamp, event_type, actor, summary, metadata_json)
             VALUES ('non-existent', 'now', 'PROJECT_REGISTERED', 'HUMAN', 'Registered', '{}')",
            [],
        );
        assert!(
            res2.is_err(),
            "Foreign key constraint must reject orphan activity_events"
        );
    }

    #[test]
    fn test_reconcile_orphaned_sessions() {
        let mut db = DbManager::new_in_memory().expect("in memory db");
        db.run_migrations().expect("migrations");

        // Insert project
        db.connection()
            .execute(
                "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at)
                 VALUES ('p1', 'Test', '/path/test', 'now', 'now', 'now')",
                [],
            )
            .unwrap();

        let s = BuilderSessionRecord {
            session_id: "s-1".to_string(),
            project_id: "p1".to_string(),
            epoch_id: "e-1".to_string(),
            conversation_id: Some("c-1".to_string()),
            model: "gemini-3.8-flash-high".to_string(),
            effort: None,
            icarus_mode: false,
            status: "RUNNING".to_string(),
            prompt: "build something".to_string(),
            response_text: None,
            error_message: None,
            started_at: "2026-09-22T00:00:00Z".to_string(),
            completed_at: None,
            duration_ms: 0,
            usage: AgyUsage::default(),
        };
        db.insert_builder_session(&s).unwrap();

        let reconciled = db.reconcile_orphaned_sessions().unwrap();
        assert_eq!(reconciled, 1);

        let s_after = db.get_builder_session("s-1").unwrap().unwrap();
        assert_eq!(s_after.status, "INTERRUPTED");
        assert!(s_after.completed_at.is_some());
        assert!(s_after
            .error_message
            .unwrap()
            .contains("Session interrupted"));
    }

    #[test]
    fn test_chatgpt_usage_telemetry_and_reset() {
        let mut db = DbManager::new_in_memory().expect("in memory db");
        db.run_migrations().expect("migrations");

        db.connection()
            .execute(
                "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at)
                 VALUES ('p1', 'Test', '/path/test', 'now', 'now', 'now')",
                [],
            )
            .unwrap();

        db.record_chatgpt_usage("p1", Some("pkt-1"), "OUTBOUND_PACKET", 4000, 1000)
            .unwrap();
        db.record_chatgpt_usage("p1", Some("pkt-1"), "INBOUND_IMPORT", 2000, 500)
            .unwrap();

        let summary = db.get_chatgpt_usage_summary("p1").unwrap();
        assert_eq!(summary.rolling_5h_tokens, 1500);
        assert_eq!(summary.rolling_7d_tokens, 1500);
        assert_eq!(summary.total_tokens, 1500);
        assert_eq!(summary.total_packets_sent, 1);
        assert_eq!(summary.total_imports_received, 1);
        assert!(summary.disclaimer.contains("Estimated relay throughput"));

        // Reset
        db.reset_chatgpt_usage("p1").unwrap();
        let summary_after = db.get_chatgpt_usage_summary("p1").unwrap();
        assert_eq!(summary_after.rolling_5h_tokens, 0);
        assert_eq!(summary_after.total_tokens, 0);
        assert!(summary_after.last_calibrated_at.is_some());
    }

    #[test]
    fn test_icarus_state_persistence() {
        let mut db = DbManager::new_in_memory().expect("in memory db");
        db.run_migrations().expect("migrations");

        db.connection()
            .execute(
                "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at)
                 VALUES ('p1', 'Test', '/path/test', 'now', 'now', 'now')",
                [],
            )
            .unwrap();

        let state0 = db.get_icarus_state("p1").unwrap();
        assert!(!state0.enabled);

        db.set_icarus_state("p1", true, Some("HUMAN")).unwrap();
        let state1 = db.get_icarus_state("p1").unwrap();
        assert!(state1.enabled);
        assert_eq!(state1.enabled_by.as_deref(), Some("HUMAN"));

        db.set_icarus_state("p1", false, Some("HUMAN")).unwrap();
        let state2 = db.get_icarus_state("p1").unwrap();
        assert!(!state2.enabled);
    }
}

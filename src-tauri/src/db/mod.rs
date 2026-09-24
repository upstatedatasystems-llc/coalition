use crate::core::builder::{
    AgyUsage, BuilderEventRecord, BuilderSessionRecord, ChatGptUsageEstimator, ChatGptUsageSummary,
    IcarusState, PermissionRecord, PermissionRuleRecord,
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
            (
                8,
                "008_stage3_estimator_calibration",
                "ALTER TABLE chatgpt_calibration_settings ADD COLUMN version INTEGER NOT NULL DEFAULT 1;
                ALTER TABLE chatgpt_calibration_settings ADD COLUMN chars_per_token REAL NOT NULL DEFAULT 4.0;
                ALTER TABLE chatgpt_calibration_settings ADD COLUMN sample_count INTEGER NOT NULL DEFAULT 0;
                ALTER TABLE chatgpt_calibration_settings ADD COLUMN last_calibrated_at TEXT;",
            ),
            (
                9,
                "009_stage3_provenance_and_normalization",
                "ALTER TABLE chatgpt_usage_records ADD COLUMN estimator_version INTEGER NOT NULL DEFAULT 1;
                ALTER TABLE chatgpt_usage_records ADD COLUMN chars_per_token REAL NOT NULL DEFAULT 4.0;
                UPDATE builder_sessions SET status = 'SUCCESS' WHERE status = 'COMPLETED';",
            ),
            (
                10,
                "010_stage4_validation_engine",
                "CREATE TABLE IF NOT EXISTS validation_runs (
                    run_id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
                    architecture_version TEXT NOT NULL,
                    epoch_id TEXT,
                    trigger_source TEXT NOT NULL,
                    status TEXT NOT NULL,
                    is_gate_passed INTEGER NOT NULL DEFAULT 0,
                    has_override INTEGER NOT NULL DEFAULT 0,
                    git_head TEXT,
                    git_dirty_fingerprint TEXT,
                    config_fingerprint TEXT,
                    log_path TEXT,
                    started_at TEXT NOT NULL,
                    completed_at TEXT,
                    duration_ms INTEGER DEFAULT 0
                );
                CREATE INDEX IF NOT EXISTS idx_validation_runs_proj ON validation_runs(project_id, started_at);

                CREATE TABLE IF NOT EXISTS validation_commands (
                    execution_id TEXT PRIMARY KEY,
                    run_id TEXT NOT NULL REFERENCES validation_runs(run_id) ON DELETE CASCADE,
                    command_id TEXT NOT NULL,
                    name TEXT NOT NULL,
                    command_str TEXT NOT NULL,
                    working_dir TEXT,
                    required INTEGER NOT NULL DEFAULT 1,
                    status TEXT NOT NULL,
                    exit_code INTEGER,
                    duration_ms INTEGER DEFAULT 0,
                    is_truncated INTEGER NOT NULL DEFAULT 0,
                    log_path TEXT,
                    started_at TEXT,
                    completed_at TEXT
                );
                CREATE INDEX IF NOT EXISTS idx_validation_commands_run ON validation_commands(run_id);

                CREATE TABLE IF NOT EXISTS validation_gate_overrides (
                    override_id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
                    run_id TEXT NOT NULL REFERENCES validation_runs(run_id) ON DELETE CASCADE,
                    git_fingerprint TEXT NOT NULL,
                    reason TEXT NOT NULL,
                    authorized_by TEXT NOT NULL,
                    created_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_validation_overrides_proj ON validation_gate_overrides(project_id, run_id);",
            ),
            (
                11,
                "011_stage4_review_correction_loop",
                "CREATE TABLE IF NOT EXISTS review_cycles (
                    cycle_id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
                    cycle_number INTEGER NOT NULL,
                    architecture_version TEXT NOT NULL,
                    epoch_id TEXT,
                    validation_run_id TEXT,
                    status TEXT NOT NULL,
                    verdict TEXT,
                    reviewer_type TEXT NOT NULL,
                    git_head TEXT,
                    git_dirty_fingerprint TEXT,
                    review_packet_hash TEXT NOT NULL,
                    corrections_packet TEXT,
                    summary TEXT,
                    started_at TEXT NOT NULL,
                    completed_at TEXT,
                    created_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_review_cycles_proj ON review_cycles(project_id, cycle_number);

                CREATE TABLE IF NOT EXISTS review_findings (
                    finding_id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
                    first_cycle_id TEXT NOT NULL REFERENCES review_cycles(cycle_id) ON DELETE CASCADE,
                    last_cycle_id TEXT NOT NULL REFERENCES review_cycles(cycle_id) ON DELETE CASCADE,
                    fingerprint TEXT NOT NULL,
                    severity TEXT NOT NULL,
                    status TEXT NOT NULL,
                    file_path TEXT,
                    line_range TEXT,
                    title TEXT NOT NULL,
                    description TEXT NOT NULL,
                    suggested_fix TEXT,
                    resolution_cycle_id TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_review_findings_proj ON review_findings(project_id, status);

                CREATE TABLE IF NOT EXISTS review_cycle_findings (
                    cycle_id TEXT NOT NULL REFERENCES review_cycles(cycle_id) ON DELETE CASCADE,
                    finding_id TEXT NOT NULL REFERENCES review_findings(finding_id) ON DELETE CASCADE,
                    is_repeat INTEGER NOT NULL DEFAULT 0,
                    PRIMARY KEY (cycle_id, finding_id)
                );
                CREATE INDEX IF NOT EXISTS idx_cycle_findings_cycle ON review_cycle_findings(cycle_id);",
            ),
            (
                12,
                "012_stage4_review_identity_hardening",
                "CREATE TABLE IF NOT EXISTS review_import_previews (
                    preview_id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
                    cycle_id TEXT NOT NULL REFERENCES review_cycles(cycle_id) ON DELETE CASCADE,
                    verdict TEXT NOT NULL,
                    summary TEXT NOT NULL,
                    raw_response TEXT NOT NULL,
                    raw_response_hash TEXT NOT NULL,
                    git_head TEXT,
                    git_dirty_fingerprint TEXT,
                    corrections_preview TEXT,
                    findings_json TEXT NOT NULL,
                    created_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_review_previews_proj ON review_import_previews(project_id, cycle_id);",
            ),
            (
                13,
                "013_stage4_review_journal_and_findings",
                "ALTER TABLE review_findings ADD COLUMN reviewer_source_id TEXT;
                ALTER TABLE review_findings ADD COLUMN requirement_references TEXT;
                ALTER TABLE review_findings ADD COLUMN problem_statement TEXT;
                ALTER TABLE review_findings ADD COLUMN required_change TEXT;
                ALTER TABLE review_findings ADD COLUMN required_test TEXT;

                CREATE TABLE IF NOT EXISTS review_acceptance_journal (
                    cycle_id TEXT PRIMARY KEY,
                    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE CASCADE,
                    preview_id TEXT NOT NULL,
                    phase TEXT NOT NULL,
                    staging_dir TEXT NOT NULL,
                    target_dir TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_review_acceptance_journal_proj ON review_acceptance_journal(project_id);",
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
    /// Records BUILDER_SESSION_INTERRUPTED activity events and returns the number of sessions reconciled.
    pub fn reconcile_orphaned_sessions(&self) -> Result<usize, DbError> {
        let now = chrono::Utc::now().to_rfc3339();
        let mut stmt = self.conn.prepare(
            "SELECT session_id, project_id FROM builder_sessions WHERE status = 'RUNNING'",
        )?;
        let orphaned: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();

        for (session_id, project_id) in &orphaned {
            let meta = serde_json::json!({
                "session_id": session_id,
                "reason": "startup_reconciliation"
            });
            let _ = crate::core::activity::ActivityManager::record_event(
                &self.conn,
                project_id,
                "BUILDER_SESSION_INTERRUPTED",
                "System",
                &format!(
                    "Builder session {} was interrupted by application shutdown or process loss",
                    session_id
                ),
                Some(&meta),
            );
        }

        self.conn.execute(
            "UPDATE builder_sessions
             SET status = 'INTERRUPTED',
                 completed_at = ?1,
                 error_message = 'Session interrupted by application shutdown or process loss'
             WHERE status = 'RUNNING'",
            params![now],
        )?;
        Ok(orphaned.len())
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
        limit: Option<usize>,
    ) -> Result<Vec<BuilderEventRecord>, DbError> {
        let max_records = limit.unwrap_or(1000);
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, project_id, step_index, event_type, state, content, details_json, timestamp
             FROM (
                 SELECT id, session_id, project_id, step_index, event_type, state, content, details_json, timestamp
                 FROM builder_events
                 WHERE session_id = ?1
                 ORDER BY id DESC
                 LIMIT ?2
             ) sub
             ORDER BY id ASC",
        )?;

        let rows = stmt.query_map(params![session_id, max_records as i64], |r| {
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
    #[allow(clippy::too_many_arguments)]
    pub fn record_chatgpt_usage(
        &self,
        project_id: &str,
        packet_id: Option<&str>,
        direction: &str,
        char_count: usize,
        estimated_tokens: usize,
        estimator_version: u32,
        chars_per_token: f64,
    ) -> Result<(), DbError> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO chatgpt_usage_records (
                project_id, packet_id, direction, char_count, estimated_tokens, timestamp, estimator_version, chars_per_token
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                project_id,
                packet_id,
                direction,
                char_count as i64,
                estimated_tokens as i64,
                now,
                estimator_version,
                chars_per_token,
            ],
        )?;
        Ok(())
    }

    pub fn get_chatgpt_usage_summary(
        &self,
        project_id: &str,
    ) -> Result<ChatGptUsageSummary, DbError> {
        type CalibrationRow = (Option<String>, u32, f64, u32, Option<String>);
        let cal_row: Option<CalibrationRow> = self
            .conn
            .query_row(
                "SELECT reset_at, version, chars_per_token, sample_count, last_calibrated_at
                 FROM chatgpt_calibration_settings WHERE project_id = ?1",
                params![project_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;

        let (cal_reset_at, estimator_version, chars_per_token, sample_count, last_calibrated_at) =
            cal_row.unwrap_or((None, 1, 4.0, 0, None));

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

        // Do not fabricate hardcoded 80,000 / 500,000 subscription plan allowances.
        let estimated_5h_capacity_pct: Option<f64> = None;
        let estimated_weekly_capacity_pct: Option<f64> = None;

        let disclaimer = format!(
            "Estimated relay throughput based on character heuristics (~{:.1} chars/token). Model limits and tier quotas are managed by OpenAI and are not provider-reported here.",
            chars_per_token
        );

        Ok(ChatGptUsageSummary {
            rolling_5h_tokens: rolling_5h_tokens as u64,
            rolling_7d_tokens: rolling_7d_tokens as u64,
            total_tokens: total_tokens as u64,
            total_packets_sent: total_packets_sent as u64,
            total_imports_received: total_imports_received as u64,
            last_calibrated_at: last_calibrated_at.or(cal_reset_at),
            disclaimer,
            estimator_version,
            chars_per_token,
            sample_count,
            estimated_5h_capacity_pct,
            estimated_weekly_capacity_pct,
        })
    }

    pub fn calibrate_chatgpt_estimator(
        &self,
        project_id: &str,
        sample_tokens: u64,
        sample_chars: usize,
    ) -> Result<ChatGptUsageEstimator, DbError> {
        let cal_row: Option<(u32, f64, u32, Option<String>)> = self
            .conn
            .query_row(
                "SELECT version, chars_per_token, sample_count, last_calibrated_at
                 FROM chatgpt_calibration_settings WHERE project_id = ?1",
                params![project_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;

        let mut estimator = match cal_row {
            Some((v, cpt, cnt, last_cal)) => ChatGptUsageEstimator::new(v, cpt, cnt, last_cal),
            None => ChatGptUsageEstimator::default(),
        };

        estimator.calibrate(sample_tokens, sample_chars);

        self.conn.execute(
            "INSERT INTO chatgpt_calibration_settings (project_id, version, chars_per_token, sample_count, last_calibrated_at, notes)
             VALUES (?1, ?2, ?3, ?4, ?5, 'Calibrated via sample')
             ON CONFLICT(project_id) DO UPDATE SET
                 version = excluded.version,
                 chars_per_token = excluded.chars_per_token,
                 sample_count = excluded.sample_count,
                 last_calibrated_at = excluded.last_calibrated_at,
                 notes = excluded.notes",
            params![
                project_id,
                estimator.version,
                estimator.chars_per_token,
                estimator.sample_count,
                estimator.last_calibrated_at,
            ],
        )?;

        Ok(estimator)
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

    pub fn insert_validation_run(
        &self,
        record: &crate::core::validation::ValidationRunRecord,
    ) -> Result<(), DbError> {
        crate::core::validation::insert_validation_run(&self.conn, record)
            .map_err(|e| DbError::Migration(e.to_string()))
    }

    pub fn update_validation_run(
        &self,
        record: &crate::core::validation::ValidationRunRecord,
    ) -> Result<(), DbError> {
        crate::core::validation::update_validation_run(&self.conn, record)
            .map_err(|e| DbError::Migration(e.to_string()))
    }

    pub fn get_validation_run(
        &self,
        run_id: &str,
    ) -> Result<Option<crate::core::validation::ValidationRunRecord>, DbError> {
        crate::core::validation::get_validation_run(&self.conn, run_id)
            .map_err(|e| DbError::Migration(e.to_string()))
    }

    pub fn get_latest_validation_run(
        &self,
        project_id: &str,
    ) -> Result<Option<crate::core::validation::ValidationRunRecord>, DbError> {
        crate::core::validation::get_latest_validation_run(&self.conn, project_id)
            .map_err(|e| DbError::Migration(e.to_string()))
    }

    pub fn list_validation_runs(
        &self,
        project_id: &str,
        limit: usize,
    ) -> Result<Vec<crate::core::validation::ValidationRunRecord>, DbError> {
        crate::core::validation::list_validation_runs_for_project(&self.conn, project_id, limit)
            .map_err(|e| DbError::Migration(e.to_string()))
    }

    pub fn insert_validation_override(
        &self,
        record: &crate::core::validation::ValidationGateOverrideRecord,
    ) -> Result<(), DbError> {
        crate::core::validation::insert_validation_override(&self.conn, record)
            .map_err(|e| DbError::Migration(e.to_string()))
    }

    pub fn reconcile_interrupted_validation_runs(&mut self) -> Result<usize, DbError> {
        crate::core::validation::ValidationService::reconcile_interrupted_runs(&mut self.conn)
            .map_err(|e| DbError::Migration(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sqlite_in_memory_migrations_and_proof() {
        let mut db = DbManager::new_in_memory().expect("in memory db");
        let result = db.run_proof().expect("run proof");
        assert_eq!(result.applied_migrations.len(), 13);
        assert_eq!(result.applied_migrations[0].version, 1);
        assert_eq!(result.applied_migrations[1].version, 2);
        assert_eq!(result.applied_migrations[2].version, 3);
        assert_eq!(result.applied_migrations[3].version, 4);
        assert_eq!(result.applied_migrations[4].version, 5);
        assert_eq!(result.applied_migrations[5].version, 6);
        assert_eq!(result.applied_migrations[6].version, 7);
        assert_eq!(result.applied_migrations[7].version, 8);
        assert_eq!(result.applied_migrations[8].version, 9);
        assert_eq!(result.applied_migrations[9].version, 10);
        assert_eq!(result.applied_migrations[10].version, 11);
        assert_eq!(result.applied_migrations[11].version, 12);
        assert_eq!(result.applied_migrations[12].version, 13);
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
        assert_eq!(applied.len(), 12);
        assert_eq!(applied[0].version, 2);
        assert_eq!(applied[1].version, 3);
        assert_eq!(applied[2].version, 4);
        assert_eq!(applied[3].version, 5);
        assert_eq!(applied[4].version, 6);
        assert_eq!(applied[5].version, 7);
        assert_eq!(applied[6].version, 8);
        assert_eq!(applied[7].version, 9);
        assert_eq!(applied[8].version, 10);
        assert_eq!(applied[9].version, 11);
        assert_eq!(applied[10].version, 12);
        assert_eq!(applied[11].version, 13);

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
        assert_eq!(applied1.len(), 13);

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

        // Verify BUILDER_SESSION_INTERRUPTED activity event was recorded
        let event_count: i64 = db
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM activity_events WHERE project_id = 'p1' AND event_type = 'BUILDER_SESSION_INTERRUPTED'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(event_count, 1);
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

        db.record_chatgpt_usage("p1", Some("pkt-1"), "OUTBOUND_PACKET", 4000, 1000, 1, 4.0)
            .unwrap();
        db.record_chatgpt_usage("p1", Some("pkt-1"), "INBOUND_IMPORT", 2000, 500, 1, 4.0)
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

    #[test]
    fn test_calibrate_chatgpt_estimator() {
        let mut db = DbManager::new_in_memory().expect("in memory db");
        db.run_migrations().expect("migrations");

        db.connection()
            .execute(
                "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at)
                 VALUES ('p1', 'Test', '/path/test', 'now', 'now', 'now')",
                [],
            )
            .unwrap();

        // Default before calibration: chars_per_token = 4.0, version = 1, sample_count = 0
        let summary0 = db.get_chatgpt_usage_summary("p1").unwrap();
        assert_eq!(summary0.chars_per_token, 4.0);
        assert_eq!(summary0.estimator_version, 1);
        assert_eq!(summary0.sample_count, 0);

        // Calibrate with sample: 1000 tokens for 3500 chars (observed: 3.5 chars/token)
        let est = db.calibrate_chatgpt_estimator("p1", 1000, 3500).unwrap();
        assert_eq!(est.version, 2);
        assert_eq!(est.sample_count, 1);
        assert_eq!(est.chars_per_token, 3.5);

        let summary1 = db.get_chatgpt_usage_summary("p1").unwrap();
        assert_eq!(summary1.chars_per_token, 3.5);
        assert_eq!(summary1.estimator_version, 2);
        assert_eq!(summary1.sample_count, 1);
        assert!(summary1.last_calibrated_at.is_some());
    }

    #[test]
    fn test_builder_sessions_all_statuses_round_trip() {
        let mut db = DbManager::new_in_memory().expect("in memory db");
        db.run_migrations().expect("migrations");

        db.connection()
            .execute(
                "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at)
                 VALUES ('p1', 'Test', '/path/test', 'now', 'now', 'now')",
                [],
            )
            .unwrap();

        let statuses = [
            crate::core::builder::STATUS_RUNNING,
            crate::core::builder::STATUS_SUCCESS,
            crate::core::builder::STATUS_FAILED,
            crate::core::builder::STATUS_CANCELLED,
            crate::core::builder::STATUS_TIMEOUT,
            crate::core::builder::STATUS_INTERRUPTED,
        ];

        for (i, status) in statuses.iter().enumerate() {
            let session_id = format!("sess-{}", i);
            let session = crate::core::builder::BuilderSessionRecord {
                session_id: session_id.clone(),
                project_id: "p1".to_string(),
                epoch_id: "epoch-1".to_string(),
                conversation_id: None,
                model: "gemini-3.8-flash-high".to_string(),
                effort: Some("medium".to_string()),
                icarus_mode: false,
                status: status.to_string(),
                prompt: "test prompt".to_string(),
                response_text: None,
                error_message: None,
                started_at: "2026-09-22T10:00:00Z".to_string(),
                completed_at: None,
                duration_ms: 1000,
                usage: crate::core::builder::AgyUsage::default(),
            };

            db.insert_builder_session(&session).unwrap();
            let retrieved = db.get_builder_session(&session_id).unwrap().unwrap();
            assert_eq!(retrieved.status, *status);
        }
    }

    #[test]
    fn test_list_builder_events_bounded_chronological() {
        let mut db = DbManager::new_in_memory().expect("in memory db");
        db.run_migrations().expect("migrations");

        db.connection()
            .execute(
                "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at)
                 VALUES ('p1', 'Test', '/path/test', 'now', 'now', 'now')",
                [],
            )
            .unwrap();

        db.insert_builder_session(&crate::core::builder::BuilderSessionRecord {
            session_id: "sess-bounded".to_string(),
            project_id: "p1".to_string(),
            epoch_id: "ep-1".to_string(),
            conversation_id: None,
            model: "m1".to_string(),
            effort: None,
            icarus_mode: false,
            status: "RUNNING".to_string(),
            prompt: "p".to_string(),
            response_text: None,
            error_message: None,
            started_at: "now".to_string(),
            completed_at: None,
            duration_ms: 0,
            usage: crate::core::builder::AgyUsage::default(),
        })
        .unwrap();

        // Insert 1,050 events
        let total_events = 1050;
        for i in 1..=total_events {
            let rec = crate::core::builder::BuilderEventRecord {
                id: 0,
                session_id: "sess-bounded".to_string(),
                project_id: "p1".to_string(),
                step_index: Some(i),
                event_type: "STEP_UPDATE".to_string(),
                state: Some("running".to_string()),
                content: Some(format!("Step content #{}", i)),
                details_json: None,
                timestamp: format!(
                    "2026-09-22T{:02}:{:02}:{:02}Z",
                    (i / 3600) % 24,
                    (i / 60) % 60,
                    i % 60
                ),
            };
            db.insert_builder_event(&rec).unwrap();
        }

        // Query with default limit (1,000)
        let retrieved = db.list_builder_events("sess-bounded", None).unwrap();
        assert_eq!(retrieved.len(), 1000);

        // Verify it returned the LATEST 1,000 records (51 to 1050)
        assert_eq!(retrieved.first().unwrap().step_index, Some(51));
        assert_eq!(retrieved.last().unwrap().step_index, Some(1050));

        // Verify chronological ascending order
        for idx in 0..retrieved.len() - 1 {
            assert!(retrieved[idx].id < retrieved[idx + 1].id);
        }
    }
}

use rusqlite::{params, Connection};
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

        let now = chrono::Utc::now().to_rfc3339();
        let test_message = format!("Phase 0 SQLite Proof executed at {}", now);

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sqlite_in_memory_migrations_and_proof() {
        let mut db = DbManager::new_in_memory().expect("in memory db");
        let result = db.run_proof().expect("run proof");
        assert_eq!(result.applied_migrations.len(), 3);
        assert_eq!(result.applied_migrations[0].version, 1);
        assert_eq!(result.applied_migrations[1].version, 2);
        assert_eq!(result.applied_migrations[2].version, 3);
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
        assert_eq!(applied.len(), 2);
        assert_eq!(applied[0].version, 2);
        assert_eq!(applied[1].version, 3);

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
        assert_eq!(applied1.len(), 3);

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
}

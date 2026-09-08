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
        Ok(Self { conn })
    }

    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, DbError> {
        let conn = Connection::open(path)?;
        Ok(Self { conn })
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

        let migrations: Vec<(i64, &'static str, &'static str)> = vec![(
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
        )];

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
        assert_eq!(result.applied_migrations.len(), 1);
        assert_eq!(result.applied_migrations[0].version, 1);
        assert_eq!(result.test_record_id, 1);
        assert_eq!(result.total_records, 1);

        // Run proof second time: migration already applied, records increment
        let result2 = db.run_proof().expect("run proof second time");
        assert_eq!(result2.applied_migrations.len(), 0);
        assert_eq!(result2.test_record_id, 2);
        assert_eq!(result2.total_records, 2);
    }
}

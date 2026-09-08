use rusqlite::{params, Connection, Transaction};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivityEventRecord {
    pub id: i64,
    pub project_id: String,
    pub timestamp: String,
    pub event_type: String,
    pub actor: String,
    pub summary: String,
    pub metadata_json: String,
}

#[derive(Error, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActivityError {
    #[error("Database error: {0}")]
    Database(String),
}

pub struct ActivityManager;

impl ActivityManager {
    pub fn record_event(
        conn: &Connection,
        project_id: &str,
        event_type: &str,
        actor: &str,
        summary: &str,
        metadata: Option<&serde_json::Value>,
    ) -> Result<ActivityEventRecord, ActivityError> {
        let timestamp = chrono::Utc::now().to_rfc3339();
        let meta_str = metadata
            .map(|m| m.to_string())
            .unwrap_or_else(|| "{}".to_string());

        conn.execute(
            "INSERT INTO activity_events (project_id, timestamp, event_type, actor, summary, metadata_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![project_id, timestamp, event_type, actor, summary, meta_str],
        )
        .map_err(|e| ActivityError::Database(e.to_string()))?;

        let id = conn.last_insert_rowid();

        Ok(ActivityEventRecord {
            id,
            project_id: project_id.to_string(),
            timestamp,
            event_type: event_type.to_string(),
            actor: actor.to_string(),
            summary: summary.to_string(),
            metadata_json: meta_str,
        })
    }

    pub fn record_event_tx(
        tx: &Transaction,
        project_id: &str,
        event_type: &str,
        actor: &str,
        summary: &str,
        metadata: Option<&serde_json::Value>,
    ) -> Result<ActivityEventRecord, ActivityError> {
        let timestamp = chrono::Utc::now().to_rfc3339();
        let meta_str = metadata
            .map(|m| m.to_string())
            .unwrap_or_else(|| "{}".to_string());

        tx.execute(
            "INSERT INTO activity_events (project_id, timestamp, event_type, actor, summary, metadata_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![project_id, timestamp, event_type, actor, summary, meta_str],
        )
        .map_err(|e| ActivityError::Database(e.to_string()))?;

        let id = tx.last_insert_rowid();

        Ok(ActivityEventRecord {
            id,
            project_id: project_id.to_string(),
            timestamp,
            event_type: event_type.to_string(),
            actor: actor.to_string(),
            summary: summary.to_string(),
            metadata_json: meta_str,
        })
    }

    pub fn get_project_activity(
        conn: &Connection,
        project_id: &str,
        limit: Option<usize>,
    ) -> Result<Vec<ActivityEventRecord>, ActivityError> {
        let max_events = limit.unwrap_or(50).min(200);

        let mut stmt = conn
            .prepare(
                "SELECT id, project_id, timestamp, event_type, actor, summary, metadata_json
                 FROM activity_events
                 WHERE project_id = ?1
                 ORDER BY id DESC
                 LIMIT ?2",
            )
            .map_err(|e| ActivityError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![project_id, max_events as i64], |row| {
                Ok(ActivityEventRecord {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    timestamp: row.get(2)?,
                    event_type: row.get(3)?,
                    actor: row.get(4)?,
                    summary: row.get(5)?,
                    metadata_json: row.get(6)?,
                })
            })
            .map_err(|e| ActivityError::Database(e.to_string()))?;

        let mut events = Vec::new();
        for r in rows {
            events.push(r.map_err(|e| ActivityError::Database(e.to_string()))?);
        }

        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbManager;

    #[test]
    fn test_record_and_query_activity_events() {
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();

        let pid = "p-activity-test";
        let now = chrono::Utc::now().to_rfc3339();
        db.connection()
            .execute(
                "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at)
                 VALUES (?1, 'Activity Test', '/tmp/repo', ?2, ?2, ?2)",
                params![pid, now],
            )
            .unwrap();

        let e1 = ActivityManager::record_event(
            db.connection(),
            pid,
            "PROJECT_REGISTERED",
            "HUMAN",
            "Project registered",
            Some(&serde_json::json!({ "foo": "bar" })),
        )
        .unwrap();

        assert_eq!(e1.id, 1);
        assert_eq!(e1.event_type, "PROJECT_REGISTERED");

        let e2 = ActivityManager::record_event(
            db.connection(),
            pid,
            "GIT_STATE_REFRESHED",
            "COALITION",
            "Git refreshed",
            None,
        )
        .unwrap();

        assert_eq!(e2.id, 2);

        let events = ActivityManager::get_project_activity(db.connection(), pid, Some(10)).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].id, 2); // Ordered descending by id
        assert_eq!(events[1].id, 1);
    }
}

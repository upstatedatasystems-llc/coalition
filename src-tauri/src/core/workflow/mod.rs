use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkflowState {
    Draft,
    Architecting,
    ReadyToFreeze,
    Frozen,
    Building,
    Validating,
    WaitingForReview,
    CorrectionsRequired,
    Blocked,
    ArchitectureConcern,
    ReviewAccepted,
    FinalValidation,
    ReadyForHumanReview,
    HumanAccepted,
    ArchitectureChange,
    ArchitectingRevision,
    ReadyToRefreeze,
    Paused,
    Interrupted,
}

impl fmt::Display for WorkflowState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Draft => "DRAFT",
            Self::Architecting => "ARCHITECTING",
            Self::ReadyToFreeze => "READY_TO_FREEZE",
            Self::Frozen => "FROZEN",
            Self::Building => "BUILDING",
            Self::Validating => "VALIDATING",
            Self::WaitingForReview => "WAITING_FOR_REVIEW",
            Self::CorrectionsRequired => "CORRECTIONS_REQUIRED",
            Self::Blocked => "BLOCKED",
            Self::ArchitectureConcern => "ARCHITECTURE_CONCERN",
            Self::ReviewAccepted => "REVIEW_ACCEPTED",
            Self::FinalValidation => "FINAL_VALIDATION",
            Self::ReadyForHumanReview => "READY_FOR_HUMAN_REVIEW",
            Self::HumanAccepted => "HUMAN_ACCEPTED",
            Self::ArchitectureChange => "ARCHITECTURE_CHANGE",
            Self::ArchitectingRevision => "ARCHITECTING_REVISION",
            Self::ReadyToRefreeze => "READY_TO_REFREEZE",
            Self::Paused => "PAUSED",
            Self::Interrupted => "INTERRUPTED",
        };
        write!(f, "{}", s)
    }
}

impl std::str::FromStr for WorkflowState {
    type Err = WorkflowError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "DRAFT" => Ok(Self::Draft),
            "ARCHITECTING" => Ok(Self::Architecting),
            "READY_TO_FREEZE" => Ok(Self::ReadyToFreeze),
            "FROZEN" => Ok(Self::Frozen),
            "BUILDING" => Ok(Self::Building),
            "VALIDATING" => Ok(Self::Validating),
            "WAITING_FOR_REVIEW" => Ok(Self::WaitingForReview),
            "CORRECTIONS_REQUIRED" => Ok(Self::CorrectionsRequired),
            "BLOCKED" => Ok(Self::Blocked),
            "ARCHITECTURE_CONCERN" => Ok(Self::ArchitectureConcern),
            "REVIEW_ACCEPTED" => Ok(Self::ReviewAccepted),
            "FINAL_VALIDATION" => Ok(Self::FinalValidation),
            "READY_FOR_HUMAN_REVIEW" => Ok(Self::ReadyForHumanReview),
            "HUMAN_ACCEPTED" => Ok(Self::HumanAccepted),
            "ARCHITECTURE_CHANGE" => Ok(Self::ArchitectureChange),
            "ARCHITECTING_REVISION" => Ok(Self::ArchitectingRevision),
            "READY_TO_REFREEZE" => Ok(Self::ReadyToRefreeze),
            "PAUSED" => Ok(Self::Paused),
            "INTERRUPTED" => Ok(Self::Interrupted),
            _ => Err(WorkflowError::InvalidStateString(s.to_string())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkflowAction {
    StartArchitecting,
    MarkReadyToFreeze,
    ReturnToArchitecting,
    Freeze,
    StartBuild,
    StartValidation,
    SubmitForReview,
    RequestCorrections,
    AcceptReview,
    StartFinalValidation,
    MarkReadyForHumanReview,
    AcceptHuman,
    RequestArchitectureChange,
    StartArchitectingRevision,
    MarkReadyToRefreeze,
    ReturnToArchitectingRevision,
    Refreeze,
    Pause,
    Resume,
    Interrupt,
    Block,
    Unblock,
    RaiseArchitectureConcern,
    ResolveArchitectureConcern,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowStateRecord {
    pub project_id: String,
    pub state: WorkflowState,
    pub resume_state: Option<WorkflowState>,
    pub revision: i64,
    pub updated_at: String,
}

#[derive(Error, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorkflowError {
    #[error("Cannot perform action {action:?} while project is in state {current:?}. {reason}")]
    InvalidTransition {
        current: WorkflowState,
        action: WorkflowAction,
        reason: String,
    },
    #[error("Terminal state {0:?} cannot accept further transitions")]
    TerminalState(WorkflowState),
    #[error("Missing resume state for state {0:?}")]
    MissingResumeState(WorkflowState),
    #[error("Unknown workflow state string: {0}")]
    InvalidStateString(String),
    #[error("Workflow state record not found for project {0}")]
    NotFound(String),
    #[error("Database error: {0}")]
    Database(String),
}

/// Computes the next state and resume state given the current state and action.
/// Pure deterministic function implementing the exact Coalition transition matrix.
pub fn compute_transition(
    current: WorkflowState,
    current_resume_state: Option<WorkflowState>,
    action: WorkflowAction,
) -> Result<(WorkflowState, Option<WorkflowState>), WorkflowError> {
    if current == WorkflowState::HumanAccepted {
        return Err(WorkflowError::TerminalState(WorkflowState::HumanAccepted));
    }

    match (current, action) {
        // Mainline transitions
        (WorkflowState::Draft, WorkflowAction::StartArchitecting) => {
            Ok((WorkflowState::Architecting, None))
        }
        (WorkflowState::Architecting, WorkflowAction::MarkReadyToFreeze) => {
            Ok((WorkflowState::ReadyToFreeze, None))
        }
        (WorkflowState::ReadyToFreeze, WorkflowAction::ReturnToArchitecting) => {
            Ok((WorkflowState::Architecting, None))
        }
        (WorkflowState::ReadyToFreeze, WorkflowAction::Freeze) => Ok((WorkflowState::Frozen, None)),
        (WorkflowState::Frozen, WorkflowAction::StartBuild) => Ok((WorkflowState::Building, None)),
        (WorkflowState::Building, WorkflowAction::StartValidation) => {
            Ok((WorkflowState::Validating, None))
        }
        (WorkflowState::Validating, WorkflowAction::SubmitForReview) => {
            Ok((WorkflowState::WaitingForReview, None))
        }
        (WorkflowState::WaitingForReview, WorkflowAction::AcceptReview) => {
            Ok((WorkflowState::ReviewAccepted, None))
        }
        (WorkflowState::ReviewAccepted, WorkflowAction::StartFinalValidation) => {
            Ok((WorkflowState::FinalValidation, None))
        }
        (WorkflowState::FinalValidation, WorkflowAction::MarkReadyForHumanReview) => {
            Ok((WorkflowState::ReadyForHumanReview, None))
        }
        (WorkflowState::ReadyForHumanReview, WorkflowAction::AcceptHuman) => {
            Ok((WorkflowState::HumanAccepted, None))
        }

        // Review Correction loop
        (WorkflowState::WaitingForReview, WorkflowAction::RequestCorrections) => {
            Ok((WorkflowState::CorrectionsRequired, None))
        }
        (WorkflowState::CorrectionsRequired, WorkflowAction::StartBuild) => {
            Ok((WorkflowState::Building, None))
        }

        // Architecture Revision Path (Permitted states)
        (
            WorkflowState::Frozen
            | WorkflowState::Building
            | WorkflowState::Validating
            | WorkflowState::WaitingForReview
            | WorkflowState::CorrectionsRequired
            | WorkflowState::Blocked
            | WorkflowState::ArchitectureConcern
            | WorkflowState::ReviewAccepted
            | WorkflowState::FinalValidation
            | WorkflowState::ReadyForHumanReview,
            WorkflowAction::RequestArchitectureChange,
        ) => Ok((WorkflowState::ArchitectureChange, None)),

        (WorkflowState::ArchitectureChange, WorkflowAction::StartArchitectingRevision) => {
            Ok((WorkflowState::ArchitectingRevision, None))
        }
        (WorkflowState::ArchitectingRevision, WorkflowAction::MarkReadyToRefreeze) => {
            Ok((WorkflowState::ReadyToRefreeze, None))
        }
        (WorkflowState::ReadyToRefreeze, WorkflowAction::ReturnToArchitectingRevision) => {
            Ok((WorkflowState::ArchitectingRevision, None))
        }
        (WorkflowState::ReadyToRefreeze, WorkflowAction::Refreeze) => {
            Ok((WorkflowState::Frozen, None))
        }

        // Pause & Resume
        (
            WorkflowState::Building
            | WorkflowState::Validating
            | WorkflowState::Architecting
            | WorkflowState::ArchitectingRevision,
            WorkflowAction::Pause,
        ) => Ok((WorkflowState::Paused, Some(current))),
        (WorkflowState::Paused, WorkflowAction::Resume) => {
            let target = current_resume_state
                .ok_or(WorkflowError::MissingResumeState(WorkflowState::Paused))?;
            Ok((target, None))
        }

        // Interrupt & Resume
        (WorkflowState::Building | WorkflowState::Validating, WorkflowAction::Interrupt) => {
            Ok((WorkflowState::Interrupted, Some(current)))
        }
        (WorkflowState::Interrupted, WorkflowAction::Resume) => {
            let target = current_resume_state.ok_or(WorkflowError::MissingResumeState(
                WorkflowState::Interrupted,
            ))?;
            Ok((target, None))
        }

        // Block & Unblock
        (
            WorkflowState::Building
            | WorkflowState::Validating
            | WorkflowState::CorrectionsRequired,
            WorkflowAction::Block,
        ) => Ok((WorkflowState::Blocked, Some(current))),
        (WorkflowState::Blocked, WorkflowAction::Unblock) => {
            let target = current_resume_state
                .ok_or(WorkflowError::MissingResumeState(WorkflowState::Blocked))?;
            Ok((target, None))
        }

        // Architecture Concern
        (
            WorkflowState::Building
            | WorkflowState::Validating
            | WorkflowState::WaitingForReview
            | WorkflowState::CorrectionsRequired,
            WorkflowAction::RaiseArchitectureConcern,
        ) => Ok((WorkflowState::ArchitectureConcern, Some(current))),
        (WorkflowState::ArchitectureConcern, WorkflowAction::ResolveArchitectureConcern) => {
            let target = current_resume_state.ok_or(WorkflowError::MissingResumeState(
                WorkflowState::ArchitectureConcern,
            ))?;
            Ok((target, None))
        }

        // Prohibited transitions
        _ => {
            let reason = match action {
                WorkflowAction::AcceptHuman => {
                    format!("Human acceptance is only allowed from READY_FOR_HUMAN_REVIEW (current is {})", current)
                }
                WorkflowAction::RequestArchitectureChange => {
                    format!(
                        "Architecture change request is not permitted from preliminary state {}",
                        current
                    )
                }
                WorkflowAction::Resume => {
                    format!(
                        "Cannot resume from non-paused/interrupted state {}",
                        current
                    )
                }
                _ => format!(
                    "No valid transition defined from state {} for action {:?}",
                    current, action
                ),
            };
            Err(WorkflowError::InvalidTransition {
                current,
                action,
                reason,
            })
        }
    }
}

/// Applies a workflow action atomically in SQLite:
/// 1. Reads current state and revision
/// 2. Validates and computes next state
/// 3. Increments revision
/// 4. Updates workflow_state table
/// 5. Writes WORKFLOW_TRANSITION event to activity_events table
/// 6. Commits transaction and returns new record
pub fn apply_workflow_action(
    conn: &mut Connection,
    project_id: &str,
    action: WorkflowAction,
    actor: &str,
) -> Result<WorkflowStateRecord, WorkflowError> {
    let tx = conn
        .transaction()
        .map_err(|e| WorkflowError::Database(e.to_string()))?;

    let record = apply_workflow_action_tx(&tx, project_id, action, actor)?;

    tx.commit()
        .map_err(|e| WorkflowError::Database(e.to_string()))?;

    Ok(record)
}

/// Internal transactional helper
pub fn apply_workflow_action_tx(
    tx: &Transaction,
    project_id: &str,
    action: WorkflowAction,
    actor: &str,
) -> Result<WorkflowStateRecord, WorkflowError> {
    let row: Option<(String, Option<String>, i64)> = tx
        .query_row(
            "SELECT state, resume_state, revision FROM workflow_state WHERE project_id = ?1",
            params![project_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| WorkflowError::Database(e.to_string()))?;

    let (state_str, resume_str, revision) =
        row.ok_or_else(|| WorkflowError::NotFound(project_id.to_string()))?;

    let current_state: WorkflowState = state_str.parse()?;
    let current_resume_state: Option<WorkflowState> = match resume_str {
        Some(s) if !s.is_empty() => Some(s.parse()?),
        _ => None,
    };

    let (new_state, new_resume_state) =
        compute_transition(current_state, current_resume_state, action)?;

    let new_revision = revision + 1;
    let now = chrono::Utc::now().to_rfc3339();
    let new_resume_str = new_resume_state.map(|s| s.to_string());

    tx.execute(
        "UPDATE workflow_state SET state = ?1, resume_state = ?2, revision = ?3, updated_at = ?4 WHERE project_id = ?5",
        params![new_state.to_string(), new_resume_str, new_revision, now, project_id],
    )
    .map_err(|e| WorkflowError::Database(e.to_string()))?;

    let summary = format!(
        "Workflow transitioned from {} to {} via action {:?}",
        current_state, new_state, action
    );
    let metadata = serde_json::json!({
        "previous_state": current_state.to_string(),
        "new_state": new_state.to_string(),
        "action": format!("{:?}", action),
        "revision": new_revision,
    });

    tx.execute(
        "INSERT INTO activity_events (project_id, timestamp, event_type, actor, summary, metadata_json)
         VALUES (?1, ?2, 'WORKFLOW_TRANSITION', ?3, ?4, ?5)",
        params![project_id, now, actor, summary, metadata.to_string()],
    )
    .map_err(|e| WorkflowError::Database(e.to_string()))?;

    Ok(WorkflowStateRecord {
        project_id: project_id.to_string(),
        state: new_state,
        resume_state: new_resume_state,
        revision: new_revision,
        updated_at: now,
    })
}

pub fn get_workflow_state(
    conn: &Connection,
    project_id: &str,
) -> Result<WorkflowStateRecord, WorkflowError> {
    let row = conn
        .query_row(
            "SELECT project_id, state, resume_state, revision, updated_at FROM workflow_state WHERE project_id = ?1",
            params![project_id],
            |r| {
                let pid: String = r.get(0)?;
                let state_str: String = r.get(1)?;
                let resume_str: Option<String> = r.get(2)?;
                let rev: i64 = r.get(3)?;
                let updated: String = r.get(4)?;
                Ok((pid, state_str, resume_str, rev, updated))
            },
        )
        .optional()
        .map_err(|e| WorkflowError::Database(e.to_string()))?;

    match row {
        Some((pid, s_str, res_str, rev, updated)) => {
            let state: WorkflowState = s_str.parse()?;
            let resume_state = match res_str {
                Some(s) if !s.is_empty() => Some(s.parse()?),
                _ => None,
            };
            Ok(WorkflowStateRecord {
                project_id: pid,
                state,
                resume_state,
                revision: rev,
                updated_at: updated,
            })
        }
        None => Err(WorkflowError::NotFound(project_id.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbManager;

    fn setup_test_db_with_project(initial_state: WorkflowState) -> (DbManager, String) {
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();
        let project_id = "proj-workflow-test".to_string();
        let now = chrono::Utc::now().to_rfc3339();

        db.connection()
            .execute(
                "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at)
                 VALUES (?1, 'Workflow Test', '/tmp/repo', ?2, ?2, ?2)",
                params![project_id, now],
            )
            .unwrap();

        db.connection()
            .execute(
                "INSERT INTO workflow_state (project_id, state, resume_state, revision, updated_at)
                 VALUES (?1, ?2, NULL, 1, ?3)",
                params![project_id, initial_state.to_string(), now],
            )
            .unwrap();

        (db, project_id)
    }

    #[test]
    fn test_mainline_transitions() {
        let (mut db, pid) = setup_test_db_with_project(WorkflowState::Draft);

        // DRAFT -> ARCHITECTING
        let r = apply_workflow_action(
            db.connection_mut(),
            &pid,
            WorkflowAction::StartArchitecting,
            "HUMAN",
        )
        .unwrap();
        assert_eq!(r.state, WorkflowState::Architecting);
        assert_eq!(r.revision, 2);

        // ARCHITECTING -> READY_TO_FREEZE
        let r = apply_workflow_action(
            db.connection_mut(),
            &pid,
            WorkflowAction::MarkReadyToFreeze,
            "HUMAN",
        )
        .unwrap();
        assert_eq!(r.state, WorkflowState::ReadyToFreeze);

        // READY_TO_FREEZE -> ReturnToArchitecting -> ARCHITECTING
        let r = apply_workflow_action(
            db.connection_mut(),
            &pid,
            WorkflowAction::ReturnToArchitecting,
            "HUMAN",
        )
        .unwrap();
        assert_eq!(r.state, WorkflowState::Architecting);

        // Back to READY_TO_FREEZE -> Freeze -> FROZEN
        let _ = apply_workflow_action(
            db.connection_mut(),
            &pid,
            WorkflowAction::MarkReadyToFreeze,
            "HUMAN",
        )
        .unwrap();
        let r = apply_workflow_action(db.connection_mut(), &pid, WorkflowAction::Freeze, "HUMAN")
            .unwrap();
        assert_eq!(r.state, WorkflowState::Frozen);

        // FROZEN -> BUILDING
        let r = apply_workflow_action(
            db.connection_mut(),
            &pid,
            WorkflowAction::StartBuild,
            "COALITION",
        )
        .unwrap();
        assert_eq!(r.state, WorkflowState::Building);

        // BUILDING -> VALIDATING
        let r = apply_workflow_action(
            db.connection_mut(),
            &pid,
            WorkflowAction::StartValidation,
            "COALITION",
        )
        .unwrap();
        assert_eq!(r.state, WorkflowState::Validating);

        // VALIDATING -> WAITING_FOR_REVIEW
        let r = apply_workflow_action(
            db.connection_mut(),
            &pid,
            WorkflowAction::SubmitForReview,
            "COALITION",
        )
        .unwrap();
        assert_eq!(r.state, WorkflowState::WaitingForReview);

        // WAITING_FOR_REVIEW -> REVIEW_ACCEPTED
        let r = apply_workflow_action(
            db.connection_mut(),
            &pid,
            WorkflowAction::AcceptReview,
            "HUMAN",
        )
        .unwrap();
        assert_eq!(r.state, WorkflowState::ReviewAccepted);

        // REVIEW_ACCEPTED -> FINAL_VALIDATION
        let r = apply_workflow_action(
            db.connection_mut(),
            &pid,
            WorkflowAction::StartFinalValidation,
            "COALITION",
        )
        .unwrap();
        assert_eq!(r.state, WorkflowState::FinalValidation);

        // FINAL_VALIDATION -> READY_FOR_HUMAN_REVIEW
        let r = apply_workflow_action(
            db.connection_mut(),
            &pid,
            WorkflowAction::MarkReadyForHumanReview,
            "COALITION",
        )
        .unwrap();
        assert_eq!(r.state, WorkflowState::ReadyForHumanReview);

        // READY_FOR_HUMAN_REVIEW -> HUMAN_ACCEPTED
        let r = apply_workflow_action(
            db.connection_mut(),
            &pid,
            WorkflowAction::AcceptHuman,
            "HUMAN",
        )
        .unwrap();
        assert_eq!(r.state, WorkflowState::HumanAccepted);

        // Terminal state rejects further actions
        let err = apply_workflow_action(
            db.connection_mut(),
            &pid,
            WorkflowAction::StartBuild,
            "COALITION",
        )
        .unwrap_err();
        assert_eq!(
            err,
            WorkflowError::TerminalState(WorkflowState::HumanAccepted)
        );
    }

    #[test]
    fn test_correction_loop() {
        let (mut db, pid) = setup_test_db_with_project(WorkflowState::WaitingForReview);

        let r = apply_workflow_action(
            db.connection_mut(),
            &pid,
            WorkflowAction::RequestCorrections,
            "HUMAN",
        )
        .unwrap();
        assert_eq!(r.state, WorkflowState::CorrectionsRequired);

        let r = apply_workflow_action(
            db.connection_mut(),
            &pid,
            WorkflowAction::StartBuild,
            "COALITION",
        )
        .unwrap();
        assert_eq!(r.state, WorkflowState::Building);
    }

    #[test]
    fn test_architecture_revision_path_from_all_permitted_states() {
        let permitted_states = [
            WorkflowState::Frozen,
            WorkflowState::Building,
            WorkflowState::Validating,
            WorkflowState::WaitingForReview,
            WorkflowState::CorrectionsRequired,
            WorkflowState::Blocked,
            WorkflowState::ArchitectureConcern,
            WorkflowState::ReviewAccepted,
            WorkflowState::FinalValidation,
            WorkflowState::ReadyForHumanReview,
        ];

        for st in permitted_states {
            let (mut db, pid) = setup_test_db_with_project(st);
            let r = apply_workflow_action(
                db.connection_mut(),
                &pid,
                WorkflowAction::RequestArchitectureChange,
                "HUMAN",
            )
            .unwrap();
            assert_eq!(r.state, WorkflowState::ArchitectureChange);

            let r = apply_workflow_action(
                db.connection_mut(),
                &pid,
                WorkflowAction::StartArchitectingRevision,
                "HUMAN",
            )
            .unwrap();
            assert_eq!(r.state, WorkflowState::ArchitectingRevision);

            let r = apply_workflow_action(
                db.connection_mut(),
                &pid,
                WorkflowAction::MarkReadyToRefreeze,
                "HUMAN",
            )
            .unwrap();
            assert_eq!(r.state, WorkflowState::ReadyToRefreeze);

            // Reversible check: return to architecting revision
            let r = apply_workflow_action(
                db.connection_mut(),
                &pid,
                WorkflowAction::ReturnToArchitectingRevision,
                "HUMAN",
            )
            .unwrap();
            assert_eq!(r.state, WorkflowState::ArchitectingRevision);

            let _ = apply_workflow_action(
                db.connection_mut(),
                &pid,
                WorkflowAction::MarkReadyToRefreeze,
                "HUMAN",
            )
            .unwrap();
            let r =
                apply_workflow_action(db.connection_mut(), &pid, WorkflowAction::Refreeze, "HUMAN")
                    .unwrap();
            assert_eq!(r.state, WorkflowState::Frozen);
        }
    }

    #[test]
    fn test_pause_and_resume() {
        let (mut db, pid) = setup_test_db_with_project(WorkflowState::Building);

        // Pause from BUILDING
        let r = apply_workflow_action(db.connection_mut(), &pid, WorkflowAction::Pause, "HUMAN")
            .unwrap();
        assert_eq!(r.state, WorkflowState::Paused);
        assert_eq!(r.resume_state, Some(WorkflowState::Building));

        // Resume restores BUILDING
        let r = apply_workflow_action(db.connection_mut(), &pid, WorkflowAction::Resume, "HUMAN")
            .unwrap();
        assert_eq!(r.state, WorkflowState::Building);
        assert_eq!(r.resume_state, None);
    }

    #[test]
    fn test_interrupt_and_resume() {
        let (mut db, pid) = setup_test_db_with_project(WorkflowState::Validating);

        let r = apply_workflow_action(
            db.connection_mut(),
            &pid,
            WorkflowAction::Interrupt,
            "SYSTEM",
        )
        .unwrap();
        assert_eq!(r.state, WorkflowState::Interrupted);
        assert_eq!(r.resume_state, Some(WorkflowState::Validating));

        let r = apply_workflow_action(db.connection_mut(), &pid, WorkflowAction::Resume, "HUMAN")
            .unwrap();
        assert_eq!(r.state, WorkflowState::Validating);
        assert_eq!(r.resume_state, None);
    }

    #[test]
    fn test_block_and_unblock() {
        let (mut db, pid) = setup_test_db_with_project(WorkflowState::CorrectionsRequired);

        let r = apply_workflow_action(
            db.connection_mut(),
            &pid,
            WorkflowAction::Block,
            "COALITION",
        )
        .unwrap();
        assert_eq!(r.state, WorkflowState::Blocked);
        assert_eq!(r.resume_state, Some(WorkflowState::CorrectionsRequired));

        let r = apply_workflow_action(db.connection_mut(), &pid, WorkflowAction::Unblock, "HUMAN")
            .unwrap();
        assert_eq!(r.state, WorkflowState::CorrectionsRequired);
        assert_eq!(r.resume_state, None);
    }

    #[test]
    fn test_architecture_concern_and_resolve() {
        let (mut db, pid) = setup_test_db_with_project(WorkflowState::Building);

        let r = apply_workflow_action(
            db.connection_mut(),
            &pid,
            WorkflowAction::RaiseArchitectureConcern,
            "COALITION",
        )
        .unwrap();
        assert_eq!(r.state, WorkflowState::ArchitectureConcern);
        assert_eq!(r.resume_state, Some(WorkflowState::Building));

        let r = apply_workflow_action(
            db.connection_mut(),
            &pid,
            WorkflowAction::ResolveArchitectureConcern,
            "HUMAN",
        )
        .unwrap();
        assert_eq!(r.state, WorkflowState::Building);
        assert_eq!(r.resume_state, None);
    }

    #[test]
    fn test_illegal_transitions() {
        let (mut db, pid) = setup_test_db_with_project(WorkflowState::Draft);

        // Cannot start build from draft
        let err = apply_workflow_action(
            db.connection_mut(),
            &pid,
            WorkflowAction::StartBuild,
            "COALITION",
        )
        .unwrap_err();
        match err {
            WorkflowError::InvalidTransition {
                current, action, ..
            } => {
                assert_eq!(current, WorkflowState::Draft);
                assert_eq!(action, WorkflowAction::StartBuild);
            }
            _ => panic!("Expected InvalidTransition, got {:?}", err),
        }

        // Cannot accept human from draft
        let err = apply_workflow_action(
            db.connection_mut(),
            &pid,
            WorkflowAction::AcceptHuman,
            "HUMAN",
        )
        .unwrap_err();
        assert!(matches!(err, WorkflowError::InvalidTransition { .. }));

        // Cannot request architecture change from draft
        let err = apply_workflow_action(
            db.connection_mut(),
            &pid,
            WorkflowAction::RequestArchitectureChange,
            "HUMAN",
        )
        .unwrap_err();
        assert!(matches!(err, WorkflowError::InvalidTransition { .. }));

        // Verify activity events count is still 0 (no event written on failed transitions)
        let event_count: i64 = db
            .connection()
            .query_row("SELECT COUNT(*) FROM activity_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(event_count, 0);
    }

    #[test]
    fn test_successful_transition_writes_exactly_one_event() {
        let (mut db, pid) = setup_test_db_with_project(WorkflowState::Draft);

        let _ = apply_workflow_action(
            db.connection_mut(),
            &pid,
            WorkflowAction::StartArchitecting,
            "HUMAN",
        )
        .unwrap();

        let event_count: i64 = db
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM activity_events WHERE project_id = ?1",
                params![pid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(event_count, 1);

        let (event_type, actor): (String, String) = db
            .connection()
            .query_row(
                "SELECT event_type, actor FROM activity_events WHERE project_id = ?1",
                params![pid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(event_type, "WORKFLOW_TRANSITION");
        assert_eq!(actor, "HUMAN");
    }
}

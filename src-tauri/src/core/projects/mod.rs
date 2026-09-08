use crate::core::activity::{ActivityError, ActivityManager};
use crate::core::artifacts::{ArchitectureState, ArtifactError, ArtifactManager, ProjectYaml};
use crate::core::git::{GitAdapter, GitError, GitRepoInfo};
use crate::core::workflow::{self, WorkflowError, WorkflowState, WorkflowStateRecord};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectRecord {
    pub project_id: String,
    pub name: String,
    pub repository_path: String,
    pub created_at: String,
    pub updated_at: String,
    pub last_opened_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub project_id: String,
    pub name: String,
    pub repository_path: String,
    pub workflow_state: Option<WorkflowState>,
    pub git_branch: Option<String>,
    pub is_clean: Option<bool>,
    pub last_opened_at: String,
    pub is_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectDetails {
    pub project: ProjectRecord,
    pub workflow_state: WorkflowStateRecord,
    pub artifact: Option<ProjectYaml>,
    pub git: Option<GitRepoInfo>,
    pub is_available: bool,
}

#[derive(Error, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectError {
    #[error("Not a Git repository: {0}")]
    NotAGitRepository(String),
    #[error("Durable contract missing at '{path}' for registered project '{project_id}'")]
    DurableContractMissing { path: String, project_id: String },
    #[error("Artifact error: {0}")]
    Artifact(String),
    #[error("Recovery required: {0}")]
    RecoveryRequired(String),
    #[error("Workflow error: {0}")]
    Workflow(String),
    #[error("Git error: {0}")]
    Git(String),
    #[error("Project not found: {0}")]
    NotFound(String),
    #[error("Project repository is unavailable at {0}")]
    RepositoryUnavailable(String),
    #[error("Project identity conflict: {0}")]
    IdentityConflict(String),
    #[error("Authoritative state corruption: {0}")]
    CorruptedState(String),
    #[error("Database error: {0}")]
    Database(String),
}

impl From<ArtifactError> for ProjectError {
    fn from(e: ArtifactError) -> Self {
        match e {
            ArtifactError::RecoveryRequired(msg) => Self::RecoveryRequired(msg),
            other => Self::Artifact(other.to_string()),
        }
    }
}

impl From<WorkflowError> for ProjectError {
    fn from(e: WorkflowError) -> Self {
        Self::Workflow(e.to_string())
    }
}

impl From<ActivityError> for ProjectError {
    fn from(e: ActivityError) -> Self {
        Self::Database(e.to_string())
    }
}

impl From<GitError> for ProjectError {
    fn from(e: GitError) -> Self {
        match e {
            GitError::NotAGitRepository(msg) => Self::NotAGitRepository(msg),
            other => Self::Git(other.to_string()),
        }
    }
}

impl From<rusqlite::Error> for ProjectError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Database(e.to_string())
    }
}

pub struct ProjectService;

impl ProjectService {
    /// Registers a new repository or opens an existing one.
    /// Handles durable contract initialization, SQLite operational records,
    /// rehydration after SQLite deletion, and Git status inspection.
    pub fn register_or_open_project<P: AsRef<Path>>(
        conn: &mut Connection,
        git: &GitAdapter,
        input_path: P,
    ) -> Result<ProjectDetails, ProjectError> {
        let repo_root = git.resolve_repo_root(input_path)?;
        let canonical_path_str = repo_root.to_string_lossy().to_string();

        let default_name = repo_root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project");

        // 1. Query SQLite operational state by canonical path BEFORE durable identity generation
        let record_by_path: Option<ProjectRecord> = conn
            .query_row(
                "SELECT project_id, name, repository_path, created_at, updated_at, last_opened_at
                 FROM projects
                 WHERE repository_path = ?1",
                params![canonical_path_str],
                |r| {
                    Ok(ProjectRecord {
                        project_id: r.get(0)?,
                        name: r.get(1)?,
                        repository_path: r.get(2)?,
                        created_at: r.get(3)?,
                        updated_at: r.get(4)?,
                        last_opened_at: r.get(5)?,
                    })
                },
            )
            .optional()?;

        // 2. Inspect durable/recovery artifacts WITHOUT mutation
        let inspection = ArtifactManager::inspect_project_artifacts(&repo_root)?;
        let coalition_dir = repo_root.join(".coalition");
        let canonical_yaml_path = coalition_dir.join("project.yaml");

        // Check for ambiguous or corrupted recovery state when canonical is missing
        if inspection.canonical.is_none() && inspection.ambiguous_or_invalid_recovery_state {
            return Err(ProjectError::RecoveryRequired(
                "Ambiguous or corrupted backup state in .coalition requires manual recovery"
                    .to_string(),
            ));
        }

        // Check for canonical missing with only temp files present
        if inspection.canonical.is_none()
            && inspection.valid_backups.is_empty()
            && inspection.temp_files_present
        {
            return Err(ProjectError::RecoveryRequired(
                "Incomplete write detected in .coalition (only temp files present); manual recovery required".to_string(),
            ));
        }

        // 3. Reconcile identity and authorize backup promotion or new contract initialization
        let (project_yaml, created_new_artifact, was_rehydrated) = if let Some(canonical_yaml) =
            inspection.canonical
        {
            // Canonical exists! Clean any stale temp files.
            ArtifactManager::clean_stale_temp_files(&coalition_dir);
            (canonical_yaml, false, false)
        } else if inspection.valid_backups.len() == 1 {
            let candidate = &inspection.valid_backups[0];
            let candidate_id = &candidate.project.project_id;

            if let Some(ref by_path) = record_by_path {
                // Known path in SQLite
                if by_path.project_id == *candidate_id {
                    // Identity matches! Authorize promotion
                    ArtifactManager::promote_backup_to_canonical(
                        &candidate.path,
                        &canonical_yaml_path,
                    )?;
                    (candidate.project.clone(), false, false)
                } else {
                    // Conflicting ID! Reject without promoting
                    return Err(ProjectError::IdentityConflict(format!(
                        "Repository path '{}' is registered with project_id '{}', but backup contract specifies conflicting project_id '{}'",
                        canonical_path_str, by_path.project_id, candidate_id
                    )));
                }
            } else {
                // Unknown path in SQLite. Query SQLite by candidate ID.
                let record_by_id: Option<ProjectRecord> = conn
                    .query_row(
                        "SELECT project_id, name, repository_path, created_at, updated_at, last_opened_at
                         FROM projects
                         WHERE project_id = ?1",
                        params![candidate_id],
                        |r| {
                            Ok(ProjectRecord {
                                project_id: r.get(0)?,
                                name: r.get(1)?,
                                repository_path: r.get(2)?,
                                created_at: r.get(3)?,
                                updated_at: r.get(4)?,
                                last_opened_at: r.get(5)?,
                            })
                        },
                    )
                    .optional()?;

                if let Some(ref by_id) = record_by_id {
                    let old_path = PathBuf::from(&by_id.repository_path);
                    if old_path.exists() {
                        // Active checkout collision! Reject without promoting
                        return Err(ProjectError::IdentityConflict(format!(
                            "Project ID '{}' from backup is already registered at active path '{}'. Cannot register duplicate checkout at '{}'",
                            candidate_id, by_id.repository_path, canonical_path_str
                        )));
                    }
                    // Moved repository! Authorize promotion
                    ArtifactManager::promote_backup_to_canonical(
                        &candidate.path,
                        &canonical_yaml_path,
                    )?;
                    (candidate.project.clone(), false, false)
                } else {
                    // Neither path nor ID is known to SQLite (e.g. SQLite was deleted / fresh DB).
                    // Authorize promotion as durable rehydration.
                    ArtifactManager::promote_backup_to_canonical(
                        &candidate.path,
                        &canonical_yaml_path,
                    )?;
                    (candidate.project.clone(), false, true)
                }
            }
        } else {
            // No canonical, no backups, no temp files
            if let Some(ref by_path) = record_by_path {
                // Known path in SQLite, but no durable contract: Case D!
                return Err(ProjectError::DurableContractMissing {
                    path: canonical_path_str,
                    project_id: by_path.project_id.clone(),
                });
            }

            // Case A: Neither SQLite nor disk knows this repository.
            let new_yaml = ArtifactManager::initialize_new_project(&repo_root, default_name)?;
            (new_yaml, true, false)
        };

        let now = chrono::Utc::now().to_rfc3339();

        // 4. Query SQLite by project ID for operational state persistence
        let record_by_id: Option<ProjectRecord> = conn
            .query_row(
                "SELECT project_id, name, repository_path, created_at, updated_at, last_opened_at
                 FROM projects
                 WHERE project_id = ?1",
                params![project_yaml.project_id],
                |r| {
                    Ok(ProjectRecord {
                        project_id: r.get(0)?,
                        name: r.get(1)?,
                        repository_path: r.get(2)?,
                        created_at: r.get(3)?,
                        updated_at: r.get(4)?,
                        last_opened_at: r.get(5)?,
                    })
                },
            )
            .optional()?;

        // Perform operational state persistence in an explicit SQLite transaction
        let tx = conn.transaction()?;

        let (project_record, is_rehydration) = match (record_by_id, record_by_path) {
            (Some(mut by_id), Some(by_path)) => {
                if by_id.project_id != by_path.project_id {
                    return Err(ProjectError::IdentityConflict(format!(
                        "Path '{}' is registered to project '{}', but durable contract has project_id '{}' which is registered at '{}'",
                        canonical_path_str, by_path.project_id, project_yaml.project_id, by_id.repository_path
                    )));
                }
                // Same project reopening at same path
                by_id.last_opened_at = now.clone();
                by_id.updated_at = now.clone();
                by_id.name = project_yaml.name.clone();
                by_id.repository_path = canonical_path_str.clone();

                tx.execute(
                    "UPDATE projects
                     SET last_opened_at = ?1, updated_at = ?2, name = ?3, repository_path = ?4
                     WHERE project_id = ?5",
                    params![
                        by_id.last_opened_at,
                        by_id.updated_at,
                        by_id.name,
                        by_id.repository_path,
                        by_id.project_id
                    ],
                )?;

                (by_id, false)
            }
            (None, Some(by_path)) => {
                // The path is already registered under a different project ID
                return Err(ProjectError::IdentityConflict(format!(
                    "Repository path '{}' is already registered with project_id '{}', but durable contract specifies project_id '{}'",
                    canonical_path_str, by_path.project_id, project_yaml.project_id
                )));
            }
            (Some(mut by_id), None) => {
                // The project ID was registered at an older path
                let old_path = PathBuf::from(&by_id.repository_path);
                if old_path.exists() {
                    // Old directory still exists: reject collision/duplicate checkout
                    return Err(ProjectError::IdentityConflict(format!(
                        "Project ID '{}' is already registered at active path '{}'. Cannot register duplicate checkout at '{}'",
                        project_yaml.project_id, by_id.repository_path, canonical_path_str
                    )));
                } else {
                    // Old directory was moved or renamed; update registered path
                    by_id.last_opened_at = now.clone();
                    by_id.updated_at = now.clone();
                    by_id.name = project_yaml.name.clone();
                    by_id.repository_path = canonical_path_str.clone();

                    tx.execute(
                        "UPDATE projects
                         SET last_opened_at = ?1, updated_at = ?2, name = ?3, repository_path = ?4
                         WHERE project_id = ?5",
                        params![
                            by_id.last_opened_at,
                            by_id.updated_at,
                            by_id.name,
                            by_id.repository_path,
                            by_id.project_id
                        ],
                    )?;

                    (by_id, false)
                }
            }
            (None, None) => {
                // New registration or rehydration into empty DB
                let record = ProjectRecord {
                    project_id: project_yaml.project_id.clone(),
                    name: project_yaml.name.clone(),
                    repository_path: canonical_path_str.clone(),
                    created_at: project_yaml.created_at.clone(),
                    updated_at: now.clone(),
                    last_opened_at: now.clone(),
                };

                tx.execute(
                    "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        record.project_id,
                        record.name,
                        record.repository_path,
                        record.created_at,
                        record.updated_at,
                        record.last_opened_at,
                    ],
                )?;

                (record, was_rehydrated || !created_new_artifact)
            }
        };

        // 3. Ensure workflow_state exists
        let wf_state_exists: bool = tx
            .query_row(
                "SELECT COUNT(*) FROM workflow_state WHERE project_id = ?1",
                params![project_record.project_id],
                |r| r.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);

        let workflow_record = if wf_state_exists {
            workflow::get_workflow_state(&tx, &project_record.project_id)?
        } else {
            let initial_state = match project_yaml.architecture_state {
                ArchitectureState::Frozen => WorkflowState::Frozen,
                ArchitectureState::Draft => WorkflowState::Draft,
            };

            tx.execute(
                "INSERT INTO workflow_state (project_id, state, resume_state, revision, updated_at)
                 VALUES (?1, ?2, NULL, 1, ?3)",
                params![project_record.project_id, initial_state.to_string(), now],
            )?;

            WorkflowStateRecord {
                project_id: project_record.project_id.clone(),
                state: initial_state,
                resume_state: None,
                revision: 1,
                updated_at: now.clone(),
            }
        };

        // 4. Record corresponding activity event
        let (event_type, summary) = if created_new_artifact {
            (
                "PROJECT_REGISTERED",
                format!(
                    "Governed new project '{}' at {}",
                    project_record.name, project_record.repository_path
                ),
            )
        } else if is_rehydration {
            (
                "PROJECT_REHYDRATED",
                format!(
                    "Rehydrated project '{}' operational state from .coalition contract",
                    project_record.name
                ),
            )
        } else {
            (
                "PROJECT_OPENED",
                format!("Opened project '{}'", project_record.name),
            )
        };

        ActivityManager::record_event(
            &tx,
            &project_record.project_id,
            event_type,
            "HUMAN",
            &summary,
            Some(&serde_json::json!({
                "repository_path": project_record.repository_path,
                "workflow_state": workflow_record.state.to_string(),
            })),
        )?;

        // 5. Update last opened project in app_settings
        Self::set_app_setting(&tx, "last_opened_project_id", &project_record.project_id)?;

        // 6. Re-validate complete .coalition hierarchy, recreating any missing standard
        // directories and rejecting symlink/junction reparse points that escape the repository.
        ArtifactManager::validate_coalition_layout(&repo_root, &coalition_dir)?;

        // Commit transaction
        tx.commit()?;

        // 6. Query live Git state
        let git_info = git.inspect_repo(&repo_root).ok();

        Ok(ProjectDetails {
            project: project_record,
            workflow_state: workflow_record,
            artifact: Some(project_yaml),
            git: git_info,
            is_available: true,
        })
    }

    /// Lists all registered projects from SQLite, checking repository availability on disk.
    /// If a repository was moved or deleted, it is flagged as available: false without being silently removed.
    pub fn list_projects(
        conn: &Connection,
        git: Option<&GitAdapter>,
    ) -> Result<Vec<ProjectSummary>, ProjectError> {
        let mut stmt = conn.prepare(
            "SELECT p.project_id, p.name, p.repository_path, p.last_opened_at, w.state
             FROM projects p
             LEFT JOIN workflow_state w ON p.project_id = w.project_id
             ORDER BY p.last_opened_at DESC",
        )?;

        let rows = stmt.query_map([], |r| {
            let pid: String = r.get(0)?;
            let name: String = r.get(1)?;
            let path: String = r.get(2)?;
            let last_opened: String = r.get(3)?;
            let state_str: Option<String> = r.get(4)?;

            Ok((pid, name, path, last_opened, state_str))
        })?;

        let mut summaries = Vec::new();

        for r in rows {
            let (pid, name, path_str, last_opened, state_str) = r?;
            let path = PathBuf::from(&path_str);
            let is_available = path.exists() && path.is_dir();

            let wf_state = match state_str {
                Some(s) => match s.parse::<WorkflowState>() {
                    Ok(st) => Some(st),
                    Err(e) => {
                        return Err(ProjectError::CorruptedState(format!(
                            "Corrupted workflow state '{}' for project {}: {}",
                            s, pid, e
                        )));
                    }
                },
                None => None,
            };

            let (branch, is_clean) = if is_available {
                if let Some(g) = git {
                    if let Ok(info) = g.inspect_repo(&path) {
                        (info.current_branch, Some(info.status.is_clean))
                    } else {
                        (None, None)
                    }
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            };

            summaries.push(ProjectSummary {
                project_id: pid,
                name,
                repository_path: path_str,
                workflow_state: wf_state,
                git_branch: branch,
                is_clean,
                last_opened_at: last_opened,
                is_available,
            });
        }

        Ok(summaries)
    }

    /// Fetches project details for a specific project ID.
    pub fn get_project_details(
        conn: &Connection,
        git: Option<&GitAdapter>,
        project_id: &str,
    ) -> Result<ProjectDetails, ProjectError> {
        let project_row: Option<ProjectRecord> = conn
            .query_row(
                "SELECT project_id, name, repository_path, created_at, updated_at, last_opened_at
                 FROM projects
                 WHERE project_id = ?1",
                params![project_id],
                |r| {
                    Ok(ProjectRecord {
                        project_id: r.get(0)?,
                        name: r.get(1)?,
                        repository_path: r.get(2)?,
                        created_at: r.get(3)?,
                        updated_at: r.get(4)?,
                        last_opened_at: r.get(5)?,
                    })
                },
            )
            .optional()?;

        let project = project_row.ok_or_else(|| ProjectError::NotFound(project_id.to_string()))?;
        let repo_path = PathBuf::from(&project.repository_path);
        let is_available = repo_path.exists() && repo_path.is_dir();

        let workflow_state = workflow::get_workflow_state(conn, project_id)?;

        let (artifact, git_info) = if is_available {
            let project_yaml_path = repo_path.join(".coalition").join("project.yaml");
            let art = ArtifactManager::read_project_yaml(&project_yaml_path)?;
            let g_info = git.and_then(|g| g.inspect_repo(&repo_path).ok());
            (Some(art), g_info)
        } else {
            (None, None)
        };

        Ok(ProjectDetails {
            project,
            workflow_state,
            artifact,
            git: git_info,
            is_available,
        })
    }

    pub fn get_app_setting(conn: &Connection, key: &str) -> Result<Option<String>, ProjectError> {
        let val: Option<String> = conn
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()?;
        Ok(val)
    }

    pub fn set_app_setting(conn: &Connection, key: &str, value: &str) -> Result<(), ProjectError> {
        conn.execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbManager;
    use std::fs;
    use std::process::Command;
    use tempfile::tempdir;

    fn init_test_git_repo(path: &Path) {
        Command::new("git")
            .args(["init"])
            .current_dir(path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(path)
            .output()
            .unwrap();
        fs::write(path.join("README.md"), "# Test Repo\n").unwrap();
        Command::new("git")
            .args(["add", "README.md"])
            .current_dir(path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial commit"])
            .current_dir(path)
            .output()
            .unwrap();
    }

    #[test]
    fn test_project_registration_and_reopen_idempotency() {
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();
        let git = GitAdapter::new().unwrap();

        let repo_dir = tempdir().unwrap();
        init_test_git_repo(repo_dir.path());

        // 1. Initial registration
        let details =
            ProjectService::register_or_open_project(db.connection_mut(), &git, repo_dir.path())
                .unwrap();
        assert_eq!(details.workflow_state.state, WorkflowState::Draft);
        assert!(details.is_available);
        assert_eq!(details.artifact.as_ref().unwrap().schema_version, 1);
        assert!(!details.project.project_id.is_empty());

        let coalition_dir = repo_dir.path().join(".coalition");
        assert!(coalition_dir.join("project.yaml").is_file());

        // 2. Idempotent reopen of same path
        let details2 =
            ProjectService::register_or_open_project(db.connection_mut(), &git, repo_dir.path())
                .unwrap();
        assert_eq!(details2.project.project_id, details.project.project_id);

        let projects = ProjectService::list_projects(db.connection(), Some(&git)).unwrap();
        assert_eq!(
            projects.len(),
            1,
            "Must not create duplicate SQLite projects"
        );

        // 3. App setting last_opened_project_id is updated
        let last_opened =
            ProjectService::get_app_setting(db.connection(), "last_opened_project_id").unwrap();
        assert_eq!(last_opened, Some(details.project.project_id));
    }

    #[test]
    fn test_rehydration_after_sqlite_loss() {
        let git = GitAdapter::new().unwrap();
        let repo_dir = tempdir().unwrap();
        init_test_git_repo(repo_dir.path());

        let durable_pid: String;

        // Session 1 with DB 1
        {
            let mut db1 = DbManager::new_in_memory().unwrap();
            db1.run_migrations().unwrap();
            let details = ProjectService::register_or_open_project(
                db1.connection_mut(),
                &git,
                repo_dir.path(),
            )
            .unwrap();
            durable_pid = details.project.project_id;
            // Transition to ARCHITECTING
            let _ = workflow::apply_workflow_action(
                db1.connection_mut(),
                &durable_pid,
                workflow::WorkflowAction::StartArchitecting,
                "HUMAN",
            )
            .unwrap();
        } // db1 dropped / deleted

        // Session 2 with fresh empty DB 2
        let mut db2 = DbManager::new_in_memory().unwrap();
        db2.run_migrations().unwrap();

        // Reopen against fresh DB
        let rehydrated =
            ProjectService::register_or_open_project(db2.connection_mut(), &git, repo_dir.path())
                .unwrap();
        assert_eq!(
            rehydrated.project.project_id, durable_pid,
            "Must preserve durable project ID"
        );
        assert_eq!(
            rehydrated.workflow_state.state,
            WorkflowState::Draft,
            "Draft state rehydrated from draft project.yaml"
        );

        // Verify activity log recorded PROJECT_REHYDRATED
        let activity =
            ActivityManager::get_project_activity(db2.connection(), &durable_pid, Some(10))
                .unwrap();
        assert_eq!(activity[0].event_type, "PROJECT_REHYDRATED");
    }

    #[test]
    fn test_rehydration_preserves_frozen_durable_architecture_state() {
        let git = GitAdapter::new().unwrap();
        let repo_dir = tempdir().unwrap();
        init_test_git_repo(repo_dir.path());

        // Create .coalition with architecture_state: frozen and non-empty version
        let (mut proj_yaml, _) =
            ArtifactManager::initialize_or_load_project(repo_dir.path(), "frozen-proj").unwrap();
        proj_yaml.architecture_state = ArchitectureState::Frozen;
        proj_yaml.current_architecture_version = Some("1.0".to_string());
        ArtifactManager::write_project_yaml_atomic(
            repo_dir.path().join(".coalition").join("project.yaml"),
            &proj_yaml,
        )
        .unwrap();

        // Fresh DB rehydration
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();

        let rehydrated =
            ProjectService::register_or_open_project(db.connection_mut(), &git, repo_dir.path())
                .unwrap();
        assert_eq!(
            rehydrated.workflow_state.state,
            WorkflowState::Frozen,
            "Rehydration must respect durable frozen state"
        );
    }

    #[test]
    fn test_unavailable_project_is_retained_not_deleted() {
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();
        let git = GitAdapter::new().unwrap();

        let repo_dir = tempdir().unwrap();
        init_test_git_repo(repo_dir.path());

        let details =
            ProjectService::register_or_open_project(db.connection_mut(), &git, repo_dir.path())
                .unwrap();
        let pid = details.project.project_id;

        // Delete the repository directory to simulate moved/deleted folder
        drop(repo_dir); // TempDir cleans up folder on drop

        let projects = ProjectService::list_projects(db.connection(), Some(&git)).unwrap();
        assert_eq!(
            projects.len(),
            1,
            "Unavailable project must NOT be silently deleted"
        );
        assert_eq!(projects[0].project_id, pid);
        assert!(
            !projects[0].is_available,
            "Project must be marked unavailable"
        );

        let fetched =
            ProjectService::get_project_details(db.connection(), Some(&git), &pid).unwrap();
        assert!(!fetched.is_available);
        assert!(
            fetched.artifact.is_none(),
            "Unavailable project must have artifact: None (never fabricated Draft)"
        );
    }

    #[test]
    fn test_reject_non_git_directory() {
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();
        let git = GitAdapter::new().unwrap();

        let normal_dir = tempdir().unwrap();
        let res =
            ProjectService::register_or_open_project(db.connection_mut(), &git, normal_dir.path());
        assert!(matches!(res, Err(ProjectError::NotAGitRepository(_))));
    }

    #[test]
    fn test_identity_conflict_detection() {
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();
        let git = GitAdapter::new().unwrap();

        let dir1 = tempdir().unwrap();
        init_test_git_repo(dir1.path());

        let dir2 = tempdir().unwrap();
        init_test_git_repo(dir2.path());

        // 1. Register dir1
        let d1 = ProjectService::register_or_open_project(db.connection_mut(), &git, dir1.path())
            .unwrap();

        // 2. Conflict: dir2 has the SAME project_id as dir1 while dir1 is still active
        fs::create_dir_all(dir2.path().join(".coalition")).unwrap();
        let mut d2_yaml = d1.artifact.unwrap();
        d2_yaml.name = "cloned-dir".to_string();
        ArtifactManager::write_project_yaml_atomic(
            dir2.path().join(".coalition").join("project.yaml"),
            &d2_yaml,
        )
        .unwrap();

        let err = ProjectService::register_or_open_project(db.connection_mut(), &git, dir2.path())
            .unwrap_err();
        assert!(
            matches!(err, ProjectError::IdentityConflict(_)),
            "Expected IdentityConflict when same project_id registered at distinct active path, got {:?}",
            err
        );

        // 3. Conflict: same path registered with a different project ID
        let new_uuid = uuid::Uuid::new_v4().to_string();
        let mut conflicting_yaml = d2_yaml;
        conflicting_yaml.project_id = new_uuid;
        ArtifactManager::write_project_yaml_atomic(
            dir1.path().join(".coalition").join("project.yaml"),
            &conflicting_yaml,
        )
        .unwrap();

        let err2 = ProjectService::register_or_open_project(db.connection_mut(), &git, dir1.path())
            .unwrap_err();
        assert!(
            matches!(err2, ProjectError::IdentityConflict(_)),
            "Expected IdentityConflict when path already registered under different project_id, got {:?}",
            err2
        );
    }

    #[test]
    fn test_moved_project_path_update() {
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();
        let git = GitAdapter::new().unwrap();

        let base_dir = tempdir().unwrap();
        let old_path = base_dir.path().join("old_loc");
        fs::create_dir_all(&old_path).unwrap();
        init_test_git_repo(&old_path);

        let d1 =
            ProjectService::register_or_open_project(db.connection_mut(), &git, &old_path).unwrap();
        let pid = d1.project.project_id;

        // Move folder from old_path to new_path (old_path ceases to exist)
        let new_path = base_dir.path().join("new_loc");
        fs::rename(&old_path, &new_path).unwrap();
        assert!(!old_path.exists());
        assert!(new_path.exists());

        // Open project from new_path
        let reopened =
            ProjectService::register_or_open_project(db.connection_mut(), &git, &new_path).unwrap();
        assert_eq!(reopened.project.project_id, pid);

        // Verify SQLite record was updated to new_path
        let canonical_new = git.resolve_repo_root(&new_path).unwrap();
        let rec = ProjectService::get_project_details(db.connection(), Some(&git), &pid).unwrap();
        assert_eq!(rec.project.repository_path, canonical_new.to_string_lossy());
    }

    #[test]
    fn test_registration_operational_transaction_rollback() {
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();
        let git = GitAdapter::new().unwrap();

        let repo_dir = tempdir().unwrap();
        init_test_git_repo(repo_dir.path());

        // Install a trigger that aborts on activity_events insert
        db.connection()
            .execute(
                "CREATE TRIGGER abort_activity BEFORE INSERT ON activity_events
                 BEGIN
                     SELECT RAISE(ABORT, 'Simulated failure during activity recording');
                 END;",
                [],
            )
            .unwrap();

        // Registration should fail because activity logging fails
        let res =
            ProjectService::register_or_open_project(db.connection_mut(), &git, repo_dir.path());
        assert!(res.is_err(), "Registration must fail on transaction error");

        // Assert that projects table has 0 rows (transaction was rolled back)
        let project_count: i64 = db
            .connection()
            .query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            project_count, 0,
            "Projects table must have 0 rows after rollback"
        );

        // Assert workflow_state has 0 rows
        let wf_count: i64 = db
            .connection()
            .query_row("SELECT COUNT(*) FROM workflow_state", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            wf_count, 0,
            "workflow_state table must have 0 rows after rollback"
        );

        // Assert app_settings has no last_opened_project_id
        let last_opened =
            ProjectService::get_app_setting(db.connection(), "last_opened_project_id").unwrap();
        assert_eq!(last_opened, None, "App settings must not have been updated");
    }

    #[test]
    fn test_list_projects_corrupted_workflow_state() {
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();
        let git = GitAdapter::new().unwrap();

        let repo_dir = tempdir().unwrap();
        init_test_git_repo(repo_dir.path());

        let details =
            ProjectService::register_or_open_project(db.connection_mut(), &git, repo_dir.path())
                .unwrap();
        let pid = details.project.project_id;

        // Manually corrupt workflow_state
        db.connection()
            .execute(
                "UPDATE workflow_state SET state = 'CORRUPTED_GARBAGE' WHERE project_id = ?1",
                params![pid],
            )
            .unwrap();

        let res = ProjectService::list_projects(db.connection(), Some(&git));
        assert!(
            matches!(res, Err(ProjectError::CorruptedState(_))),
            "Corrupted workflow state must return Err(ProjectError::CorruptedState), got {:?}",
            res
        );
    }

    #[test]
    fn test_case_d_durable_contract_missing_zero_mutation() {
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();
        let git = GitAdapter::new().unwrap();

        let repo_dir = tempdir().unwrap();
        init_test_git_repo(repo_dir.path());

        // Initial registration
        let details =
            ProjectService::register_or_open_project(db.connection_mut(), &git, repo_dir.path())
                .unwrap();
        let original_pid = details.project.project_id.clone();

        // Delete .coalition/project.yaml from disk to simulate Case D
        let project_yaml_path = repo_dir.path().join(".coalition").join("project.yaml");
        fs::remove_file(&project_yaml_path).unwrap();
        assert!(!project_yaml_path.exists());

        // Attempting to register/open must return DurableContractMissing
        let err =
            ProjectService::register_or_open_project(db.connection_mut(), &git, repo_dir.path())
                .unwrap_err();

        match err {
            ProjectError::DurableContractMissing { path, project_id } => {
                assert_eq!(project_id, original_pid);
                let canonical = git.resolve_repo_root(repo_dir.path()).unwrap();
                assert_eq!(path, canonical.to_string_lossy());
            }
            other => panic!("Expected DurableContractMissing, got {:?}", other),
        }

        // CRITICAL: Ensure zero durable identity mutation occurred on disk
        assert!(
            !project_yaml_path.exists(),
            "Must NOT write a new project.yaml when durable contract is missing for a known path"
        );
    }

    #[test]
    fn test_case_e_identity_conflict_zero_mutation() {
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();
        let git = GitAdapter::new().unwrap();

        let repo_dir = tempdir().unwrap();
        init_test_git_repo(repo_dir.path());

        // Register project
        let details =
            ProjectService::register_or_open_project(db.connection_mut(), &git, repo_dir.path())
                .unwrap();
        let original_pid = details.project.project_id.clone();

        // Overwrite project.yaml on disk with a brand new project_id
        let mut rogue_project = details.artifact.unwrap();
        rogue_project.project_id = uuid::Uuid::new_v4().to_string();
        ArtifactManager::write_project_yaml_atomic(
            repo_dir.path().join(".coalition").join("project.yaml"),
            &rogue_project,
        )
        .unwrap();

        // Attempting to reopen must return IdentityConflict
        let err =
            ProjectService::register_or_open_project(db.connection_mut(), &git, repo_dir.path())
                .unwrap_err();
        assert!(matches!(err, ProjectError::IdentityConflict(_)));

        // SQLite must still retain the original project ID
        let rec = ProjectService::get_project_details(db.connection(), Some(&git), &original_pid)
            .unwrap();
        assert_eq!(rec.project.project_id, original_pid);
    }

    #[test]
    fn test_valid_canonical_plus_stale_backup_canonical_wins() {
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();
        let git = GitAdapter::new().unwrap();

        let repo_dir = tempdir().unwrap();
        init_test_git_repo(repo_dir.path());

        // Register project
        let details =
            ProjectService::register_or_open_project(db.connection_mut(), &git, repo_dir.path())
                .unwrap();
        let pid = details.project.project_id.clone();
        let coalition = repo_dir.path().join(".coalition");

        // Plant a stale backup with a different project ID
        let stale_backup = coalition.join("project.yaml.bak.old");
        let stale_proj = ProjectYaml {
            schema_version: 1,
            project_id: uuid::Uuid::new_v4().to_string(),
            name: "stale-backup".to_string(),
            current_architecture_version: None,
            architecture_state: ArchitectureState::Draft,
            created_at: "2026-09-08T00:00:00Z".to_string(),
        };
        fs::write(&stale_backup, serde_yaml::to_string(&stale_proj).unwrap()).unwrap();

        // Reopen project: canonical must win!
        let reopened =
            ProjectService::register_or_open_project(db.connection_mut(), &git, repo_dir.path())
                .unwrap();
        assert_eq!(reopened.project.project_id, pid);
        assert_eq!(reopened.artifact.unwrap().project_id, pid);
    }

    #[test]
    fn test_known_path_matching_backup_promoted() {
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();
        let git = GitAdapter::new().unwrap();

        let repo_dir = tempdir().unwrap();
        init_test_git_repo(repo_dir.path());

        // Register project
        let details =
            ProjectService::register_or_open_project(db.connection_mut(), &git, repo_dir.path())
                .unwrap();
        let pid = details.project.project_id.clone();
        let coalition = repo_dir.path().join(".coalition");

        // Remove canonical project.yaml and place single valid backup with SAME project_id
        let canonical_path = coalition.join("project.yaml");
        fs::remove_file(&canonical_path).unwrap();

        let backup_path = coalition.join("project.yaml.bak.matching");
        let mut backup_proj = details.artifact.unwrap();
        backup_proj.name = "promoted-from-backup".to_string();
        fs::write(&backup_path, serde_yaml::to_string(&backup_proj).unwrap()).unwrap();

        // Reopen project: matching backup must be promoted to canonical!
        let reopened =
            ProjectService::register_or_open_project(db.connection_mut(), &git, repo_dir.path())
                .unwrap();
        assert_eq!(reopened.project.project_id, pid);
        assert!(
            canonical_path.exists(),
            "Canonical project.yaml must now exist"
        );
        let on_disk = ArtifactManager::read_project_yaml(&canonical_path).unwrap();
        assert_eq!(on_disk.project_id, pid);
        assert_eq!(on_disk.name, "promoted-from-backup");
    }

    #[test]
    fn test_known_path_conflicting_backup_rejected_no_mutation() {
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();
        let git = GitAdapter::new().unwrap();

        let repo_dir = tempdir().unwrap();
        init_test_git_repo(repo_dir.path());

        // Register project
        let _details =
            ProjectService::register_or_open_project(db.connection_mut(), &git, repo_dir.path())
                .unwrap();
        let coalition = repo_dir.path().join(".coalition");

        // Remove canonical project.yaml and place single valid backup with CONFLICTING project_id
        let canonical_path = coalition.join("project.yaml");
        fs::remove_file(&canonical_path).unwrap();

        let backup_path = coalition.join("project.yaml.bak.conflicting");
        let conflicting_proj = ProjectYaml {
            schema_version: 1,
            project_id: uuid::Uuid::new_v4().to_string(),
            name: "conflicting".to_string(),
            current_architecture_version: None,
            architecture_state: ArchitectureState::Draft,
            created_at: "2026-09-08T00:00:00Z".to_string(),
        };
        fs::write(
            &backup_path,
            serde_yaml::to_string(&conflicting_proj).unwrap(),
        )
        .unwrap();

        // Reopen project: must return IdentityConflict
        let err =
            ProjectService::register_or_open_project(db.connection_mut(), &git, repo_dir.path())
                .unwrap_err();
        assert!(matches!(err, ProjectError::IdentityConflict(_)));

        // CRITICAL: Ensure zero promotion mutation occurred
        assert!(
            !canonical_path.exists(),
            "Must NOT promote conflicting backup"
        );
        assert!(backup_path.exists(), "Backup must remain untouched");
    }

    #[test]
    fn test_unknown_path_backup_registered_at_active_path_rejected() {
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();
        let git = GitAdapter::new().unwrap();

        // Repo 1 is registered and active
        let repo1 = tempdir().unwrap();
        init_test_git_repo(repo1.path());
        let details1 =
            ProjectService::register_or_open_project(db.connection_mut(), &git, repo1.path())
                .unwrap();

        // Repo 2 at unknown path has no canonical, but has backup with active_pid
        let repo2 = tempdir().unwrap();
        init_test_git_repo(repo2.path());
        let coalition2 = repo2.path().join(".coalition");
        fs::create_dir_all(&coalition2).unwrap();

        let backup_path = coalition2.join("project.yaml.bak.duplicate");
        let mut duplicate_proj = details1.artifact.unwrap();
        duplicate_proj.name = "duplicate-checkout".to_string();
        fs::write(
            &backup_path,
            serde_yaml::to_string(&duplicate_proj).unwrap(),
        )
        .unwrap();

        // Registering repo 2 must detect active checkout collision and reject
        let err = ProjectService::register_or_open_project(db.connection_mut(), &git, repo2.path())
            .unwrap_err();
        assert!(matches!(err, ProjectError::IdentityConflict(_)));
        assert!(
            !coalition2.join("project.yaml").exists(),
            "Must not promote duplicate backup"
        );
    }

    #[test]
    fn test_unknown_path_backup_from_moved_path_recovered() {
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();
        let git = GitAdapter::new().unwrap();

        // Register original project
        let repo1 = tempdir().unwrap();
        init_test_git_repo(repo1.path());
        let details1 =
            ProjectService::register_or_open_project(db.connection_mut(), &git, repo1.path())
                .unwrap();
        let pid = details1.project.project_id.clone();
        let backup_content = details1.artifact.unwrap();

        // Drop repo1 (simulating move)
        drop(repo1);

        // Repo 2 at new path has backup with same ID
        let repo2 = tempdir().unwrap();
        init_test_git_repo(repo2.path());
        let coalition2 = repo2.path().join(".coalition");
        fs::create_dir_all(&coalition2).unwrap();
        let backup_path = coalition2.join("project.yaml.bak.moved");
        fs::write(
            &backup_path,
            serde_yaml::to_string(&backup_content).unwrap(),
        )
        .unwrap();

        // Registering repo 2 succeeds as moved repository recovery
        let details2 =
            ProjectService::register_or_open_project(db.connection_mut(), &git, repo2.path())
                .unwrap();
        assert_eq!(details2.project.project_id, pid);
        assert!(
            coalition2.join("project.yaml").exists(),
            "Backup must be promoted"
        );

        // SQLite path updated to repo2
        let canonical2 = git.resolve_repo_root(repo2.path()).unwrap();
        assert_eq!(
            details2.project.repository_path,
            canonical2.to_string_lossy()
        );
    }

    #[test]
    fn test_fresh_db_single_valid_backup_rehydrated() {
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();
        let git = GitAdapter::new().unwrap();

        let repo_dir = tempdir().unwrap();
        init_test_git_repo(repo_dir.path());
        let coalition = repo_dir.path().join(".coalition");
        fs::create_dir_all(&coalition).unwrap();

        let pid = uuid::Uuid::new_v4().to_string();
        let backup_proj = ProjectYaml {
            schema_version: 1,
            project_id: pid.clone(),
            name: "rehydrated-from-backup".to_string(),
            current_architecture_version: None,
            architecture_state: ArchitectureState::Draft,
            created_at: "2026-09-08T00:00:00Z".to_string(),
        };
        fs::write(
            coalition.join("project.yaml.bak.1"),
            serde_yaml::to_string(&backup_proj).unwrap(),
        )
        .unwrap();

        // Register in empty DB
        let details =
            ProjectService::register_or_open_project(db.connection_mut(), &git, repo_dir.path())
                .unwrap();
        assert_eq!(details.project.project_id, pid);
        assert!(
            coalition.join("project.yaml").exists(),
            "Backup must be promoted"
        );

        // Verify PROJECT_REHYDRATED logged
        let events =
            ActivityManager::get_project_activity(db.connection(), &pid, Some(10)).unwrap();
        assert!(
            events.iter().any(|e| e.event_type == "PROJECT_REHYDRATED"),
            "Must be logged as PROJECT_REHYDRATED"
        );
    }

    #[test]
    fn test_multiple_backups_recovery_required_no_mutation() {
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();
        let git = GitAdapter::new().unwrap();

        let repo_dir = tempdir().unwrap();
        init_test_git_repo(repo_dir.path());
        let coalition = repo_dir.path().join(".coalition");
        fs::create_dir_all(&coalition).unwrap();

        let proj1 = ProjectYaml {
            schema_version: 1,
            project_id: uuid::Uuid::new_v4().to_string(),
            name: "backup-1".to_string(),
            current_architecture_version: None,
            architecture_state: ArchitectureState::Draft,
            created_at: "2026-09-08T00:00:00Z".to_string(),
        };
        let proj2 = ProjectYaml {
            schema_version: 1,
            project_id: uuid::Uuid::new_v4().to_string(),
            name: "backup-2".to_string(),
            current_architecture_version: None,
            architecture_state: ArchitectureState::Draft,
            created_at: "2026-09-08T00:00:00Z".to_string(),
        };

        fs::write(
            coalition.join("project.yaml.bak.1"),
            serde_yaml::to_string(&proj1).unwrap(),
        )
        .unwrap();
        fs::write(
            coalition.join("project.yaml.bak.2"),
            serde_yaml::to_string(&proj2).unwrap(),
        )
        .unwrap();

        let err =
            ProjectService::register_or_open_project(db.connection_mut(), &git, repo_dir.path())
                .unwrap_err();
        assert!(matches!(err, ProjectError::RecoveryRequired(_)));
        assert!(
            !coalition.join("project.yaml").exists(),
            "Must NOT create project.yaml"
        );
        assert!(coalition.join("project.yaml.bak.1").exists());
        assert!(coalition.join("project.yaml.bak.2").exists());
    }

    #[test]
    fn test_only_temp_file_missing_canonical_recovery_required() {
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();
        let git = GitAdapter::new().unwrap();

        let repo_dir = tempdir().unwrap();
        init_test_git_repo(repo_dir.path());
        let coalition = repo_dir.path().join(".coalition");
        fs::create_dir_all(&coalition).unwrap();

        // Plant only a temp file
        let temp_file = coalition.join("project.yaml.tmp.interrupted");
        fs::write(&temp_file, "interrupted write data").unwrap();

        let err =
            ProjectService::register_or_open_project(db.connection_mut(), &git, repo_dir.path())
                .unwrap_err();
        assert!(matches!(err, ProjectError::RecoveryRequired(_)));
        assert!(
            !coalition.join("project.yaml").exists(),
            "Must NOT create project.yaml"
        );
        assert!(
            temp_file.exists(),
            "Temp file must remain for recovery inspection"
        );
    }

    #[test]
    fn test_reopen_recreates_missing_standard_subdirectories() {
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();
        let git = GitAdapter::new().unwrap();

        let repo_dir = tempdir().unwrap();
        init_test_git_repo(repo_dir.path());

        // 1. Register project initially
        let details =
            ProjectService::register_or_open_project(db.connection_mut(), &git, repo_dir.path())
                .unwrap();
        let pid = details.project.project_id;
        let coalition = repo_dir.path().join(".coalition");

        // 2. Remove several empty standard subdirectories
        let design_dir = coalition.join("design");
        let reviews_dir = coalition.join("reviews");
        let evidence_dir = coalition.join("evidence");
        fs::remove_dir(&design_dir).unwrap();
        fs::remove_dir(&reviews_dir).unwrap();
        fs::remove_dir(&evidence_dir).unwrap();
        assert!(!design_dir.exists());
        assert!(!reviews_dir.exists());
        assert!(!evidence_dir.exists());

        // 3. Reopen project
        let reopened =
            ProjectService::register_or_open_project(db.connection_mut(), &git, repo_dir.path())
                .unwrap();
        assert_eq!(reopened.project.project_id, pid);

        // 4. Missing standard directories must be recreated safely
        assert!(design_dir.is_dir(), "design directory must be recreated");
        assert!(reviews_dir.is_dir(), "reviews directory must be recreated");
        assert!(
            evidence_dir.is_dir(),
            "evidence directory must be recreated"
        );
    }

    #[test]
    fn test_reopen_rejects_escaping_symlink_or_junction() {
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();
        let git = GitAdapter::new().unwrap();

        let repo_dir = tempdir().unwrap();
        init_test_git_repo(repo_dir.path());

        // 1. Register project
        let _details =
            ProjectService::register_or_open_project(db.connection_mut(), &git, repo_dir.path())
                .unwrap();
        let coalition = repo_dir.path().join(".coalition");
        let changes_dir = coalition.join("changes");

        // 2. Create external directory outside repository root
        let external_dir = tempdir().unwrap();
        let external_target = external_dir.path().join("external_target");
        fs::create_dir_all(&external_target).unwrap();

        // 3. Replace standard subdirectory with a junction / symlink pointing outside repo
        fs::remove_dir(&changes_dir).unwrap();

        #[cfg(windows)]
        {
            let status = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(changes_dir.as_os_str())
                .arg(external_target.as_os_str())
                .status()
                .unwrap();
            assert!(status.success(), "mklink /J must succeed");
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&external_target, &changes_dir).unwrap();
        }

        // 4. Reopen project: must reject before Coalition writes through that path!
        let err =
            ProjectService::register_or_open_project(db.connection_mut(), &git, repo_dir.path())
                .unwrap_err();
        match err {
            ProjectError::Artifact(ref msg) => {
                assert!(
                    msg.contains("outside repository root") || msg.contains("Path traversal"),
                    "Expected reparse point or traversal error message, got: {}",
                    msg
                );
            }
            other => panic!("Expected ProjectError::Artifact, got {:?}", other),
        }

        // Windows cleanup of junction so tempdir destructor can delete it without issue
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("cmd")
                .args(["/C", "rmdir"])
                .arg(changes_dir.as_os_str())
                .status();
        }
    }
}

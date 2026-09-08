use crate::core::activity::ActivityManager;
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
    pub artifact: ProjectYaml,
    pub git: Option<GitRepoInfo>,
    pub is_available: bool,
}

#[derive(Error, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectError {
    #[error("Not a Git repository: {0}")]
    NotAGitRepository(String),
    #[error("Artifact error: {0}")]
    Artifact(String),
    #[error("Workflow error: {0}")]
    Workflow(String),
    #[error("Git error: {0}")]
    Git(String),
    #[error("Project not found: {0}")]
    NotFound(String),
    #[error("Project repository is unavailable at {0}")]
    RepositoryUnavailable(String),
    #[error("Database error: {0}")]
    Database(String),
}

impl From<ArtifactError> for ProjectError {
    fn from(e: ArtifactError) -> Self {
        Self::Artifact(e.to_string())
    }
}

impl From<WorkflowError> for ProjectError {
    fn from(e: WorkflowError) -> Self {
        Self::Workflow(e.to_string())
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

        // 1. Durable artifact initialization / loading
        let (project_yaml, created_new_artifact) =
            ArtifactManager::initialize_or_load_project(&repo_root, default_name)?;

        let now = chrono::Utc::now().to_rfc3339();

        // 2. Check existing operational SQLite state by project_id or canonical path
        let existing_project: Option<ProjectRecord> = conn
            .query_row(
                "SELECT project_id, name, repository_path, created_at, updated_at, last_opened_at
                 FROM projects
                 WHERE project_id = ?1 OR repository_path = ?2",
                params![project_yaml.project_id, canonical_path_str],
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

        let (project_record, was_rehydrated) = match existing_project {
            Some(mut p) => {
                // Update last_opened_at and ensure canonical path and name match durable artifact
                p.last_opened_at = now.clone();
                p.updated_at = now.clone();
                p.name = project_yaml.name.clone();
                p.repository_path = canonical_path_str.clone();

                conn.execute(
                    "UPDATE projects
                     SET last_opened_at = ?1, updated_at = ?2, name = ?3, repository_path = ?4
                     WHERE project_id = ?5",
                    params![
                        p.last_opened_at,
                        p.updated_at,
                        p.name,
                        p.repository_path,
                        p.project_id
                    ],
                )?;

                (p, false)
            }
            None => {
                // Insert new operational record
                let record = ProjectRecord {
                    project_id: project_yaml.project_id.clone(),
                    name: project_yaml.name.clone(),
                    repository_path: canonical_path_str.clone(),
                    created_at: project_yaml.created_at.clone(),
                    updated_at: now.clone(),
                    last_opened_at: now.clone(),
                };

                conn.execute(
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

                // Rehydration occurs if the durable artifact already existed but SQLite had no row
                let is_rehydration = !created_new_artifact;
                (record, is_rehydration)
            }
        };

        // 3. Ensure workflow_state exists
        let wf_state_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM workflow_state WHERE project_id = ?1",
                params![project_record.project_id],
                |r| r.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);

        let workflow_record = if wf_state_exists {
            workflow::get_workflow_state(conn, &project_record.project_id)?
        } else {
            // Determine initial workflow state according to durable metadata
            let initial_state = match project_yaml.architecture_state {
                ArchitectureState::Frozen => WorkflowState::Frozen,
                ArchitectureState::Draft => WorkflowState::Draft,
            };

            conn.execute(
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
        } else if was_rehydrated {
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

        let _ = ActivityManager::record_event(
            conn,
            &project_record.project_id,
            event_type,
            "HUMAN",
            &summary,
            Some(&serde_json::json!({
                "repository_path": project_record.repository_path,
                "workflow_state": workflow_record.state.to_string(),
            })),
        );

        // 5. Update last opened project in app_settings
        let _ = Self::set_app_setting(conn, "last_opened_project_id", &project_record.project_id);

        // 6. Query live Git state
        let git_info = git.inspect_repo(&repo_root).ok();

        Ok(ProjectDetails {
            project: project_record,
            workflow_state: workflow_record,
            artifact: project_yaml,
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

            let wf_state: Option<WorkflowState> = state_str.and_then(|s| s.parse().ok());

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
            (art, g_info)
        } else {
            // Unavailable project: construct dummy or cached metadata
            let fallback_artifact = ProjectYaml {
                schema_version: 1,
                project_id: project.project_id.clone(),
                name: project.name.clone(),
                current_architecture_version: None,
                architecture_state: ArchitectureState::Draft,
                created_at: project.created_at.clone(),
            };
            (fallback_artifact, None)
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
        assert_eq!(details.artifact.schema_version, 1);
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

        // Create .coalition with architecture_state: frozen
        let (mut proj_yaml, _) =
            ArtifactManager::initialize_or_load_project(repo_dir.path(), "frozen-proj").unwrap();
        proj_yaml.architecture_state = ArchitectureState::Frozen;
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
}

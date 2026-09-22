use crate::core::activity::{ActivityEventRecord, ActivityManager};
use crate::core::artifacts::{ArtifactApplicability, ArtifactError};
use crate::core::builder::{
    AgyEvent, AgyUsage, AntigravityCliAdapter, BuilderError, BuilderEventRecord,
    BuilderSessionRecord, BuilderTurnRequest, BuilderTurnResponse, ChatGptUsageSummary,
    IcarusState, ModelInfo, PermissionRecord, UsageTelemetryReport,
};
use crate::core::git::{GitAdapter, GitError, GitRepoInfo};
use crate::core::projects::{ProjectDetails, ProjectError, ProjectService, ProjectSummary};
use crate::core::relay::readiness::ReadinessReport;
use crate::core::relay::{
    ArtifactContentDetails, ImportPreview, RelayError, RelayHistoryItem, RelayPacket, RelayService,
    WorkspaceState,
};
use crate::core::workflow::{self, WorkflowAction, WorkflowError, WorkflowStateRecord};
use crate::db::{DbError, DbManager, ProofResult};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::{mpsc, Mutex};

pub struct AppState {
    pub db: Mutex<DbManager>,
    pub git: Mutex<Option<GitAdapter>>,
    pub agy: Mutex<Option<AntigravityCliAdapter>>,
    pub cancel_flag: Arc<AtomicBool>,
    pub active_project_id: Mutex<Option<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandError {
    pub code: String,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

impl CommandError {
    pub fn new<S: Into<String>, M: Into<String>>(code: S, message: M) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details<S: Into<String>, M: Into<String>>(
        code: S,
        message: M,
        details: serde_json::Value,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: Some(details),
        }
    }
}

impl From<ProjectError> for CommandError {
    fn from(e: ProjectError) -> Self {
        match e {
            ProjectError::NotAGitRepository(msg) => Self::new("NOT_A_GIT_REPOSITORY", msg),
            ProjectError::DurableContractMissing { path, project_id } => Self::with_details(
                "DURABLE_CONTRACT_MISSING",
                format!(
                    "Durable contract missing at '{}' for registered project '{}'",
                    path, project_id
                ),
                serde_json::json!({ "path": path, "project_id": project_id }),
            ),
            ProjectError::NotFound(msg) => Self::new("PROJECT_NOT_FOUND", msg),
            ProjectError::RepositoryUnavailable(msg) => Self::new("REPOSITORY_UNAVAILABLE", msg),
            ProjectError::IdentityConflict(msg) => Self::new("PROJECT_IDENTITY_CONFLICT", msg),
            ProjectError::RecoveryRequired(msg) => Self::new("ARTIFACT_RECOVERY_REQUIRED", msg),
            ProjectError::CorruptedState(msg) => Self::new("CORRUPTED_STATE", msg),
            ProjectError::Artifact(msg) => Self::new("ARTIFACT_ERROR", msg),
            ProjectError::Workflow(msg) => Self::new("WORKFLOW_ERROR", msg),
            ProjectError::Git(msg) => Self::new("GIT_ERROR", msg),
            ProjectError::Database(msg) => Self::new("DATABASE_ERROR", msg),
        }
    }
}

impl From<ArtifactError> for CommandError {
    fn from(e: ArtifactError) -> Self {
        match e {
            ArtifactError::UnsupportedSchemaVersion(v) => Self::with_details(
                "UNSUPPORTED_SCHEMA_VERSION",
                format!(
                    "Unsupported schema version {}. Only version 1 is supported",
                    v
                ),
                serde_json::json!({ "version": v }),
            ),
            ArtifactError::UnsupportedReadinessPolicyVersion(v) => Self::with_details(
                "UNSUPPORTED_READINESS_POLICY_VERSION",
                format!(
                    "Unsupported readiness policy version {}. Only version 1 is supported",
                    v
                ),
                serde_json::json!({ "version": v }),
            ),
            ArtifactError::UnknownReadinessArtifactPath(p) => Self::with_details(
                "UNKNOWN_READINESS_ARTIFACT_PATH",
                format!("Unknown artifact path in readiness applicability: {}", p),
                serde_json::json!({ "path": p }),
            ),
            ArtifactError::InvalidProjectId(msg) => Self::new("INVALID_PROJECT_ID", msg),
            ArtifactError::EmptyProjectName => {
                Self::new("EMPTY_PROJECT_NAME", "Project name cannot be empty")
            }
            ArtifactError::InvalidTimestamp(msg) => Self::new("INVALID_TIMESTAMP", msg),
            ArtifactError::InvalidArchitectureState(msg) => {
                Self::new("INVALID_ARCHITECTURE_STATE", msg)
            }
            ArtifactError::PathTraversal { path, root } => Self::with_details(
                "PATH_TRAVERSAL_DETECTED",
                format!("Path traversal detected: {} escapes root {}", path, root),
                serde_json::json!({ "path": path, "root": root }),
            ),
            ArtifactError::UnsafeReparsePoint(msg) => Self::new("UNSAFE_REPARSE_POINT", msg),
            ArtifactError::InvalidYaml(msg) => Self::new("INVALID_YAML", msg),
            ArtifactError::NotFound(msg) => Self::new("ARTIFACT_NOT_FOUND", msg),
            ArtifactError::RecoveryRequired(msg) => Self::new("ARTIFACT_RECOVERY_REQUIRED", msg),
            ArtifactError::Io(msg) => Self::new("IO_ERROR", msg),
        }
    }
}

impl From<WorkflowError> for CommandError {
    fn from(e: WorkflowError) -> Self {
        match e {
            WorkflowError::InvalidTransition {
                current,
                action,
                reason,
            } => Self::with_details(
                "INVALID_WORKFLOW_TRANSITION",
                format!(
                    "Cannot perform action {:?} while project is in state {}. {}",
                    action, current, reason
                ),
                serde_json::json!({
                    "current_state": current.to_string(),
                    "action": format!("{:?}", action),
                    "reason": reason,
                }),
            ),
            WorkflowError::TerminalState(st) => Self::with_details(
                "TERMINAL_WORKFLOW_STATE",
                format!("State {} is terminal and rejects further transitions", st),
                serde_json::json!({ "state": st.to_string() }),
            ),
            WorkflowError::MissingResumeState(st) => Self::with_details(
                "MISSING_RESUME_STATE",
                format!("State {} has no recorded resume state", st),
                serde_json::json!({ "state": st.to_string() }),
            ),
            WorkflowError::InvalidStateString(s) => Self::new(
                "INVALID_STATE_STRING",
                format!("Unknown state string: {}", s),
            ),
            WorkflowError::NotFound(pid) => Self::new(
                "WORKFLOW_STATE_NOT_FOUND",
                format!("Workflow state for project {} not found", pid),
            ),
            WorkflowError::Database(msg) => Self::new("DATABASE_ERROR", msg),
        }
    }
}

impl From<GitError> for CommandError {
    fn from(e: GitError) -> Self {
        match e {
            GitError::NotFound(msg) => Self::new("GIT_NOT_FOUND", msg),
            GitError::NotAGitRepository(msg) => Self::new("NOT_A_GIT_REPOSITORY", msg),
            GitError::ExecutionFailed(msg) => Self::new("GIT_EXECUTION_FAILED", msg),
            GitError::Io(err) => Self::new("IO_ERROR", err.to_string()),
        }
    }
}

impl From<DbError> for CommandError {
    fn from(e: DbError) -> Self {
        Self::new("DATABASE_ERROR", e.to_string())
    }
}

impl From<RelayError> for CommandError {
    fn from(e: RelayError) -> Self {
        match e {
            RelayError::ProjectNotFound(msg) => Self::new("PROJECT_NOT_FOUND", msg),
            RelayError::Database(msg) => Self::new("DATABASE_ERROR", msg),
            RelayError::Artifact(msg) => Self::new("ARTIFACT_ERROR", msg),
            RelayError::Workflow(msg) => Self::new("WORKFLOW_ERROR", msg),
            RelayError::IllegalWorkflowState { current, operation } => Self::with_details(
                "ILLEGAL_WORKFLOW_STATE",
                format!(
                    "Cannot perform operation '{}' while project is in workflow state {}",
                    operation, current
                ),
                serde_json::json!({ "current": current, "operation": operation }),
            ),
            RelayError::NoPendingPacket(p) => Self::new(
                "NO_PENDING_PACKET",
                format!("No pending relay packet for project {}", p),
            ),
            RelayError::PacketAlreadyImported(p) => Self::new(
                "PACKET_ALREADY_IMPORTED",
                format!("Packet {} has already been imported", p),
            ),
            RelayError::PacketSuperseded(p) => Self::new(
                "PACKET_SUPERSEDED",
                format!("Packet {} has been superseded", p),
            ),
            RelayError::ImportAlreadyPending(id) => Self::with_details(
                "IMPORT_ALREADY_PENDING",
                format!("An import is already pending review: {}", id),
                serde_json::json!({ "import_id": id }),
            ),
            RelayError::ImportAlreadyAccepted(id) => Self::new(
                "IMPORT_ALREADY_ACCEPTED",
                format!("Import {} has already been accepted", id),
            ),
            RelayError::DuplicateImport(msg) => Self::new("DUPLICATE_IMPORT", msg),
            RelayError::UnrelatedContent(msg) => Self::new("UNRELATED_CLIPBOARD_CONTENT", msg),
            RelayError::ParseError(msg) => Self::new("RELAY_PARSE_ERROR", msg),
            RelayError::SchemaError(msg) => Self::new("RELAY_SCHEMA_ERROR", msg),
            RelayError::ProjectIdMismatch { expected, actual } => Self::with_details(
                "PROJECT_ID_MISMATCH",
                format!(
                    "Import project ID mismatch: expected {}, got {}",
                    expected, actual
                ),
                serde_json::json!({ "expected": expected, "actual": actual }),
            ),
            RelayError::PacketIdMismatch { expected, actual } => Self::with_details(
                "PACKET_ID_MISMATCH",
                format!(
                    "Import packet ID mismatch: expected {}, got {}",
                    expected, actual
                ),
                serde_json::json!({ "expected": expected, "actual": actual }),
            ),
            RelayError::DisallowedArtifactPath(p) => Self::with_details(
                "DISALLOWED_ARTIFACT_PATH",
                format!(
                    "Proposed artifact path is outside the allowed architecture package: {}",
                    p
                ),
                serde_json::json!({ "path": p }),
            ),
            RelayError::StaleImportPreview { path, details } => Self::with_details(
                "STALE_IMPORT_PREVIEW",
                format!(
                    "Target '{}' changed on disk since preview was generated: {}",
                    path, details
                ),
                serde_json::json!({ "path": path, "details": details }),
            ),
            RelayError::StaleArtifactContent { path, details } => Self::with_details(
                "STALE_ARTIFACT_CONTENT",
                format!(
                    "Artifact '{}' was modified since last read: {}",
                    path, details
                ),
                serde_json::json!({ "path": path, "details": details }),
            ),
            RelayError::ActionSemanticConflict { path, details } => Self::with_details(
                "ACTION_SEMANTIC_CONFLICT",
                format!("Action semantic conflict for '{}': {}", path, details),
                serde_json::json!({ "path": path, "details": details }),
            ),
            RelayError::InvalidYamlContent { path, error } => Self::with_details(
                "INVALID_YAML_CONTENT",
                format!("Invalid YAML syntax in '{}': {}", path, error),
                serde_json::json!({ "path": path, "error": error }),
            ),
            RelayError::BatchRecoveryRequired(msg) => Self::new("BATCH_RECOVERY_REQUIRED", msg),
            RelayError::ParseFailure {
                import_id,
                raw_content,
                message,
            } => Self::with_details(
                "RELAY_PARSE_FAILURE",
                format!("Failed to parse response: {}", message),
                serde_json::json!({ "import_id": import_id, "raw_content": raw_content, "message": message }),
            ),
            RelayError::ImportNotFound(id) => {
                Self::new("IMPORT_NOT_FOUND", format!("Import {} not found", id))
            }
            RelayError::DuplicateArtifactPath(path) => Self::with_details(
                "DUPLICATE_ARTIFACT_PATH",
                format!("Duplicate artifact path in response: {}", path),
                serde_json::json!({ "path": path }),
            ),
            RelayError::OpenQuestionsDirectArtifactRejected(path) => Self::with_details(
                "OPEN_QUESTIONS_DIRECT_ARTIFACT_REJECTED",
                "Direct modification of 'design/open-questions.md' in artifacts is disallowed; use top-level 'open_questions:' field instead",
                serde_json::json!({ "path": path }),
            ),
            RelayError::AlreadyDecided(s) => {
                Self::new("IMPORT_ALREADY_DECIDED", format!("Import already {}", s))
            }
            RelayError::UnsupportedReadinessPolicyVersion(v) => Self::with_details(
                "UNSUPPORTED_READINESS_POLICY_VERSION",
                format!(
                    "Unsupported readiness policy version {}. Only version 1 is supported",
                    v
                ),
                serde_json::json!({ "version": v }),
            ),
            RelayError::UnknownReadinessArtifactPath(p) => Self::with_details(
                "UNKNOWN_READINESS_ARTIFACT_PATH",
                format!("Unknown artifact path in readiness applicability: {}", p),
                serde_json::json!({ "path": p }),
            ),
            #[cfg(test)]
            RelayError::InjectedFailure(msg) => Self::new("INJECTED_FAILURE", msg),
        }
    }
}

impl From<crate::core::freeze::FreezeError> for CommandError {
    fn from(e: crate::core::freeze::FreezeError) -> Self {
        use crate::core::freeze::FreezeError;
        match e {
            FreezeError::ReadinessIncomplete(msg) => Self::new("READINESS_INCOMPLETE", msg),
            FreezeError::IllegalWorkflowState { current } => Self::with_details(
                "ILLEGAL_WORKFLOW_STATE",
                format!("Illegal workflow state for freeze: {}", current),
                serde_json::json!({ "current": current }),
            ),
            FreezeError::NoHeadCommit => Self::new(
                "NO_HEAD_COMMIT",
                "Repository has no valid HEAD commit (unborn repository). A valid Git commit boundary is required before freezing architecture",
            ),
            FreezeError::StaleFreezePreview(msg) => Self::new("STALE_FREEZE_PREVIEW", msg),
            FreezeError::VersionAlreadyExists(v) => Self::with_details(
                "ARCHITECTURE_VERSION_ALREADY_EXISTS",
                format!("Architecture version '{}' already exists and is immutable", v),
                serde_json::json!({ "version": v }),
            ),
            FreezeError::FrozenSnapshotCorrupt { version, reason } => Self::with_details(
                "FROZEN_SNAPSHOT_CORRUPT",
                format!("Frozen snapshot for version '{}' is corrupt: {}", version, reason),
                serde_json::json!({ "version": version, "reason": reason }),
            ),
            FreezeError::FreezeRecoveryRequired(msg) => Self::new("FREEZE_RECOVERY_REQUIRED", msg),
            FreezeError::DriftRestorationRecoveryRequired(msg) => {
                Self::new("DRIFT_RESTORATION_RECOVERY_REQUIRED", msg)
            }
            FreezeError::InvalidArchitectureVersion(v) => Self::with_details(
                "INVALID_ARCHITECTURE_VERSION",
                format!("Invalid architecture version: {}", v),
                serde_json::json!({ "version": v }),
            ),
            FreezeError::Artifact(msg) => Self::new("ARTIFACT_ERROR", msg),
            FreezeError::Git(msg) => Self::new("GIT_ERROR", msg),
            FreezeError::Workflow(msg) => Self::new("WORKFLOW_ERROR", msg),
            FreezeError::Database(msg) => Self::new("DATABASE_ERROR", msg),
            FreezeError::Io(msg) => Self::new("IO_ERROR", msg),
        }
    }
}

impl From<BuilderError> for CommandError {
    fn from(e: BuilderError) -> Self {
        match e {
            BuilderError::NotFound(msg) => Self::new("AGY_NOT_FOUND", msg),
            BuilderError::ExecutionFailed(msg) => Self::new("BUILDER_EXECUTION_FAILED", msg),
            BuilderError::ParseError(msg) => Self::new("BUILDER_PARSE_ERROR", msg),
            BuilderError::Io(err) => Self::new("IO_ERROR", err.to_string()),
            BuilderError::Canceled => Self::new("BUILDER_CANCELED", "Session was canceled"),
            BuilderError::Timeout => Self::new("BUILDER_TIMEOUT", "Session timed out"),
            BuilderError::NotFrozen(msg) => Self::new("NOT_FROZEN", msg),
            BuilderError::DriftDetected(msg) => Self::new("DRIFT_DETECTED", msg),
            BuilderError::Database(msg) => Self::new("DATABASE_ERROR", msg),
        }
    }
}

impl From<String> for CommandError {
    fn from(msg: String) -> Self {
        Self::new("GENERAL_ERROR", msg)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemDiagnosticInfo {
    pub current_dir: String,
    pub git: Option<GitRepoInfo>,
    pub agy_detected: bool,
    pub agy_path: Option<String>,
    pub agy_version: Option<String>,
    pub agy_models: Vec<ModelInfo>,
    pub agy_error: Option<String>,
}

// ----------------------------------------------------------------------------
// Phase 1 Project Commands
// ----------------------------------------------------------------------------

#[tauri::command]
pub async fn list_projects(
    state: State<'_, AppState>,
) -> Result<Vec<ProjectSummary>, CommandError> {
    let db = state.db.lock().await;
    let mut git_lock = state.git.lock().await;
    if git_lock.is_none() {
        if let Ok(adapter) = GitAdapter::new() {
            *git_lock = Some(adapter);
        }
    }

    ProjectService::list_projects(db.connection(), git_lock.as_ref()).map_err(CommandError::from)
}

#[tauri::command]
pub async fn register_or_open_project(
    state: State<'_, AppState>,
    path: String,
) -> Result<ProjectDetails, CommandError> {
    let mut git_lock = state.git.lock().await;
    if git_lock.is_none() {
        *git_lock = Some(GitAdapter::new().map_err(CommandError::from)?);
    }
    let git = git_lock.as_ref().unwrap();

    let mut db = state.db.lock().await;
    let details = ProjectService::register_or_open_project(db.connection_mut(), git, path)
        .map_err(CommandError::from)?;

    let details = if details.is_available {
        let repo_path = PathBuf::from(&details.project.repository_path);
        let reconciled = RelayService::reconcile_interrupted_batches(
            db.connection(),
            &repo_path,
            &details.project.project_id,
        )
        .map_err(CommandError::from)?;

        if reconciled {
            ProjectService::get_project_details(
                db.connection(),
                Some(git),
                &details.project.project_id,
            )
            .map_err(CommandError::from)?
        } else {
            details
        }
    } else {
        details
    };

    *state.active_project_id.lock().await = Some(details.project.project_id.clone());

    Ok(details)
}

#[tauri::command]
pub async fn get_project_details(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<ProjectDetails, CommandError> {
    let db = state.db.lock().await;
    let mut git_lock = state.git.lock().await;
    if git_lock.is_none() {
        if let Ok(adapter) = GitAdapter::new() {
            *git_lock = Some(adapter);
        }
    }

    let details =
        ProjectService::get_project_details(db.connection(), git_lock.as_ref(), &project_id)
            .map_err(CommandError::from)?;

    let details = if details.is_available {
        let repo_path = PathBuf::from(&details.project.repository_path);
        let reconciled = RelayService::reconcile_interrupted_batches(
            db.connection(),
            &repo_path,
            &details.project.project_id,
        )
        .map_err(CommandError::from)?;

        if reconciled {
            ProjectService::get_project_details(
                db.connection(),
                git_lock.as_ref(),
                &details.project.project_id,
            )
            .map_err(CommandError::from)?
        } else {
            details
        }
    } else {
        details
    };

    *state.active_project_id.lock().await = Some(details.project.project_id.clone());

    Ok(details)
}

#[tauri::command]
pub async fn refresh_project_git_state(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<ProjectDetails, CommandError> {
    let mut git_lock = state.git.lock().await;
    if git_lock.is_none() {
        *git_lock = Some(GitAdapter::new().map_err(CommandError::from)?);
    }
    let git = git_lock.as_ref().unwrap();

    let mut db = state.db.lock().await;
    let details = ProjectService::get_project_details(db.connection(), Some(git), &project_id)?;

    if details.is_available {
        let _ = ActivityManager::record_event(
            db.connection_mut(),
            &project_id,
            "GIT_STATE_REFRESHED",
            "COALITION",
            "Git repository state refreshed",
            None,
        );
    }

    ProjectService::get_project_details(db.connection(), Some(git), &project_id)
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn get_project_activity(
    state: State<'_, AppState>,
    project_id: String,
    limit: Option<usize>,
) -> Result<Vec<ActivityEventRecord>, CommandError> {
    let db = state.db.lock().await;
    ActivityManager::get_project_activity(db.connection(), &project_id, limit)
        .map_err(|e| CommandError::new("DATABASE_ERROR", e.to_string()))
}

pub fn apply_workflow_action_impl(
    db: &mut DbManager,
    project_id: &str,
    action: WorkflowAction,
) -> Result<WorkflowStateRecord, CommandError> {
    // Invariant: Builder execution cannot start if repository resolution, restoration reconciliation,
    // frozen snapshot integrity, or contract drift checks fail.
    if action == WorkflowAction::StartBuild {
        let repo_path = get_repo_path_for_project_sync(db, project_id)?;
        if !repo_path.exists() || !repo_path.is_dir() {
            return Err(CommandError::new(
                "REPOSITORY_UNAVAILABLE",
                format!(
                    "Repository directory does not exist or is unavailable: {:?}",
                    repo_path
                ),
            ));
        }

        crate::core::freeze::FreezeService::reconcile_drift_restoration(
            &repo_path,
            project_id,
            db.connection_mut(),
        )
        .map_err(CommandError::from)?;

        let coalition_dir =
            crate::core::artifacts::ArtifactManager::resolve_coalition_dir(&repo_path)
                .map_err(CommandError::from)?;
        let project_yaml = crate::core::artifacts::ArtifactManager::read_project_yaml(
            coalition_dir.join("project.yaml"),
        )
        .map_err(CommandError::from)?;
        let version = project_yaml
            .current_architecture_version
            .as_deref()
            .unwrap_or("1.0");

        crate::core::freeze::FreezeService::verify_snapshot_integrity(
            &repo_path,
            version,
            project_yaml.active_manifest_fingerprint.as_deref(),
        )
        .map_err(CommandError::from)?;

        let drift =
            crate::core::freeze::FreezeService::check_contract_drift(&repo_path, project_id)
                .map_err(CommandError::from)?;
        if drift.has_drift {
            return Err(CommandError::with_details(
                "FROZEN_CONTRACT_DRIFT_DETECTED",
                "Cannot start build: architecture contract has drifted from frozen snapshot. Restore artifacts or complete governed architecture change first.",
                serde_json::to_value(&drift).unwrap_or_default(),
            ));
        }
    }

    workflow::apply_workflow_action(db.connection_mut(), project_id, action, "HUMAN")
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn apply_workflow_action(
    state: State<'_, AppState>,
    project_id: String,
    action: WorkflowAction,
) -> Result<WorkflowStateRecord, CommandError> {
    let mut db = state.db.lock().await;
    apply_workflow_action_impl(&mut db, &project_id, action)
}

#[tauri::command]
pub async fn get_last_opened_project_id(
    state: State<'_, AppState>,
) -> Result<Option<String>, CommandError> {
    let db = state.db.lock().await;
    ProjectService::get_app_setting(db.connection(), "last_opened_project_id")
        .map_err(CommandError::from)
}

// ----------------------------------------------------------------------------
// Phase 0 Diagnostic Commands (Preserved with structured CommandError)
// ----------------------------------------------------------------------------

#[tauri::command]
pub async fn get_system_diagnostics(
    state: State<'_, AppState>,
) -> Result<SystemDiagnosticInfo, CommandError> {
    let current_dir = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .to_string_lossy()
        .to_string();

    let mut git_lock = state.git.lock().await;
    if git_lock.is_none() {
        if let Ok(adapter) = GitAdapter::new() {
            *git_lock = Some(adapter);
        }
    }

    let git_info = if let Some(ref git) = *git_lock {
        git.inspect_repo(&current_dir).ok()
    } else {
        None
    };

    let mut agy_lock = state.agy.lock().await;
    if agy_lock.is_none() {
        if let Ok(adapter) = AntigravityCliAdapter::discover() {
            *agy_lock = Some(adapter);
        }
    }

    let mut agy_detected = false;
    let mut agy_path = None;
    let mut agy_version = None;
    let mut agy_models = Vec::new();
    let mut agy_error = None;

    if let Some(ref agy) = *agy_lock {
        agy_detected = true;
        agy_path = Some(agy.binary_path().to_string_lossy().to_string());
        match agy.get_version() {
            Ok(v) => agy_version = Some(v),
            Err(e) => agy_error = Some(e.to_string()),
        }
        match agy.list_models() {
            Ok(m) => agy_models = m,
            Err(e) => {
                if agy_error.is_none() {
                    agy_error = Some(e.to_string());
                }
            }
        }
    } else {
        agy_error = Some(
            "Antigravity CLI (agy) not found on PATH or standard local app directories".to_string(),
        );
    }

    Ok(SystemDiagnosticInfo {
        current_dir,
        git: git_info,
        agy_detected,
        agy_path,
        agy_version,
        agy_models,
        agy_error,
    })
}

#[tauri::command]
pub async fn run_sqlite_proof(state: State<'_, AppState>) -> Result<ProofResult, CommandError> {
    let mut db = state.db.lock().await;
    db.run_proof().map_err(CommandError::from)
}

fn get_fake_agy_path() -> Result<PathBuf, CommandError> {
    let fake_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests")
        .join("fake-commands")
        .join(if cfg!(windows) {
            "fake-agy.cmd"
        } else {
            "fake-agy"
        });

    if !fake_path.exists() {
        return Err(CommandError::new(
            "FAKE_AGY_NOT_FOUND",
            format!("fake-agy executable not found at {:?}", fake_path),
        ));
    }
    Ok(fake_path)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartBuilderTurnPayload {
    #[serde(alias = "project_id")]
    pub project_id: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    #[serde(alias = "follow_up_prompt")]
    pub follow_up_prompt: Option<String>,
    #[serde(alias = "use_fake_agy")]
    pub use_fake_agy: bool,
}

#[tauri::command]
pub async fn list_builder_models(
    state: State<'_, AppState>,
    use_fake_agy: Option<bool>,
) -> Result<Vec<ModelInfo>, CommandError> {
    let adapter = if use_fake_agy.unwrap_or(false) {
        let fake_path = get_fake_agy_path()?;
        AntigravityCliAdapter::with_path(fake_path)
    } else {
        let lock = state.agy.lock().await;
        if let Some(ref a) = *lock {
            AntigravityCliAdapter::with_path(a.binary_path())
        } else {
            AntigravityCliAdapter::discover()
                .map_err(|e| CommandError::new("AGY_DISCOVERY_ERROR", e.to_string()))?
        }
    };
    adapter
        .list_models()
        .map_err(|e| CommandError::new("AGY_MODELS_ERROR", e.to_string()))
}

#[tauri::command]
pub async fn get_builder_session(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<Option<BuilderSessionRecord>, CommandError> {
    let db = state.db.lock().await;
    db.get_latest_builder_session(&project_id)
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn list_builder_sessions(
    state: State<'_, AppState>,
    project_id: String,
    limit: Option<usize>,
) -> Result<Vec<BuilderSessionRecord>, CommandError> {
    let db = state.db.lock().await;
    db.list_builder_sessions(&project_id, limit.unwrap_or(20))
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn get_builder_events(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Vec<BuilderEventRecord>, CommandError> {
    let db = state.db.lock().await;
    db.list_builder_events(&session_id)
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn start_builder_turn(
    app: AppHandle,
    state: State<'_, AppState>,
    payload: StartBuilderTurnPayload,
) -> Result<BuilderTurnResponse, CommandError> {
    state.cancel_flag.store(false, Ordering::Relaxed);

    let repo_path = {
        let db = state.db.lock().await;
        get_repo_path_for_project_sync(&db, &payload.project_id)?
    };

    if !repo_path.exists() {
        return Err(CommandError::new(
            "REPOSITORY_UNAVAILABLE",
            format!("Repository at {:?} is unavailable on disk", repo_path),
        ));
    }

    // Check workflow state: must be FROZEN, BUILDING, or CORRECTIONS_REQUIRED
    let current_wf_state = {
        let db = state.db.lock().await;
        db.connection()
            .query_row(
                "SELECT state FROM workflow_state WHERE project_id = ?1",
                rusqlite::params![payload.project_id],
                |r| r.get::<_, String>(0),
            )
            .map_err(|e| CommandError::new("WORKFLOW_STATE_NOT_FOUND", e.to_string()))?
    };

    if current_wf_state != "FROZEN"
        && current_wf_state != "BUILDING"
        && current_wf_state != "CORRECTIONS_REQUIRED"
    {
        return Err(CommandError::new(
            "INVALID_WORKFLOW_STATE",
            format!(
                "Cannot start Builder turn while project is in {} state. Architecture must be FROZEN first.",
                current_wf_state
            ),
        ));
    }

    // If in FROZEN or CORRECTIONS_REQUIRED, transition to BUILDING
    if current_wf_state == "FROZEN" || current_wf_state == "CORRECTIONS_REQUIRED" {
        let mut db = state.db.lock().await;
        workflow::apply_workflow_action(
            db.connection_mut(),
            &payload.project_id,
            WorkflowAction::StartBuild,
            "HUMAN",
        )
        .map_err(CommandError::from)?;
    }

    // Check drift
    let drift_report =
        crate::core::freeze::FreezeService::check_contract_drift(&repo_path, &payload.project_id)
            .map_err(CommandError::from)?;
    if drift_report.has_drift {
        return Err(CommandError::new(
            "DRIFT_DETECTED",
            "Cannot run Builder: frozen architecture contract drift detected. Please reconcile drift before proceeding.",
        ));
    }

    // Authoritative builder packet from Stage 2 contract
    let builder_packet = crate::core::freeze::FreezeService::get_builder_packet(&repo_path, None)
        .map_err(CommandError::from)?;

    let epoch_id = builder_packet.metadata.builder_epoch_id.clone();

    // Check for existing conversation in this epoch and Icarus state
    let (existing_conv_id, icarus_mode) = {
        let db = state.db.lock().await;
        let latest_session = db
            .get_latest_builder_session(&payload.project_id)
            .map_err(CommandError::from)?;
        let cid = latest_session.and_then(|s| {
            if s.epoch_id == epoch_id {
                s.conversation_id
            } else {
                None
            }
        });
        let icarus = db
            .get_icarus_state(&payload.project_id)
            .map_err(CommandError::from)?
            .enabled;
        (cid, icarus)
    };

    // Construct authoritative prompt
    let turn_prompt = if let Some(ref follow_up) = payload.follow_up_prompt {
        if existing_conv_id.is_some() {
            follow_up.clone()
        } else {
            format!(
                "{}\n\nAdditional Guidance:\n{}",
                builder_packet.prompt, follow_up
            )
        }
    } else {
        builder_packet.prompt.clone()
    };

    let model_name = payload
        .model
        .clone()
        .unwrap_or_else(|| "gemini-3.8-flash-high".to_string());
    let effort_level = payload.effort.clone();

    let session_id = uuid::Uuid::new_v4().to_string();
    let started_at = chrono::Utc::now().to_rfc3339();

    // Insert initial session record
    {
        let db = state.db.lock().await;
        let session_rec = BuilderSessionRecord {
            session_id: session_id.clone(),
            project_id: payload.project_id.clone(),
            epoch_id: epoch_id.clone(),
            conversation_id: existing_conv_id.clone(),
            model: model_name.clone(),
            effort: effort_level.clone(),
            icarus_mode,
            status: "RUNNING".to_string(),
            prompt: turn_prompt.clone(),
            response_text: None,
            error_message: None,
            started_at: started_at.clone(),
            completed_at: None,
            duration_ms: 0,
            usage: AgyUsage::default(),
        };
        db.insert_builder_session(&session_rec)
            .map_err(CommandError::from)?;

        let meta = serde_json::json!({
            "session_id": session_id,
            "epoch_id": epoch_id,
            "model": model_name,
            "effort": effort_level,
            "icarus_mode": icarus_mode,
            "conversation_id": existing_conv_id,
        });
        let _ = ActivityManager::record_event(
            db.connection(),
            &payload.project_id,
            "BUILDER_TURN_STARTED",
            "BUILDER",
            &format!(
                "Started Builder turn with model {} (epoch: {}){}",
                model_name,
                epoch_id,
                if icarus_mode { " [ICARUS MODE]" } else { "" }
            ),
            Some(&meta),
        );
    }

    let adapter = if payload.use_fake_agy {
        let fake_path = get_fake_agy_path()?;
        AntigravityCliAdapter::with_path(fake_path)
    } else {
        let lock = state.agy.lock().await;
        if let Some(ref a) = *lock {
            AntigravityCliAdapter::with_path(a.binary_path())
        } else {
            AntigravityCliAdapter::discover()
                .map_err(|e| CommandError::new("AGY_DISCOVERY_ERROR", e.to_string()))?
        }
    };

    let (tx, mut rx) = mpsc::channel::<AgyEvent>(200);

    let app_clone = app.clone();
    let session_id_clone = session_id.clone();
    let project_id_clone = payload.project_id.clone();

    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            let _ = app_clone.emit(
                "coalition:builder-event",
                &serde_json::json!({
                    "session_id": session_id_clone,
                    "project_id": project_id_clone,
                    "event": event,
                }),
            );
        }
    });

    let request = BuilderTurnRequest {
        prompt: turn_prompt,
        conversation_id: existing_conv_id,
        model: Some(model_name.clone()),
        effort: effort_level,
        icarus_mode,
        working_dir: Some(repo_path.to_string_lossy().to_string()),
    };

    let turn_start = tokio::time::Instant::now();
    let execution_result = adapter
        .run_turn(request, state.cancel_flag.clone(), Some(tx))
        .await;
    let duration_ms = turn_start.elapsed().as_millis() as u64;
    let completed_at = chrono::Utc::now().to_rfc3339();

    match execution_result {
        Ok(resp) => {
            let db = state.db.lock().await;
            let _ = db.update_builder_session_status(
                &session_id,
                &resp.status,
                Some(&resp.text_response),
                None,
                Some(&completed_at),
                duration_ms,
                &resp.cumulative_usage,
                resp.conversation_id.as_deref(),
            );

            let meta = serde_json::json!({
                "session_id": session_id,
                "status": resp.status,
                "duration_ms": duration_ms,
                "tokens": resp.cumulative_usage.total_tokens,
                "conversation_id": resp.conversation_id,
            });
            let _ = ActivityManager::record_event(
                db.connection(),
                &payload.project_id,
                if resp.was_canceled {
                    "BUILDER_TURN_CANCELED"
                } else {
                    "BUILDER_TURN_COMPLETED"
                },
                "BUILDER",
                &format!(
                    "Builder turn {} with status: {}",
                    if resp.was_canceled {
                        "canceled"
                    } else {
                        "completed"
                    },
                    resp.status
                ),
                Some(&meta),
            );

            if resp.status == "ERROR" || resp.stderr.contains("permission denied") {
                let perm_rec = PermissionRecord {
                    id: 0,
                    project_id: payload.project_id.clone(),
                    session_id: Some(session_id.clone()),
                    tool_name: "external_action".to_string(),
                    target: None,
                    risk_level: "HIGH".to_string(),
                    decision: "BLOCKED".to_string(),
                    reason: Some(
                        "Action requires review or permission was denied in headless execution"
                            .to_string(),
                    ),
                    created_at: completed_at.clone(),
                };
                let _ = db.record_permission_history(&perm_rec);
            }

            Ok(resp)
        }
        Err(e) => {
            let err_str = e.to_string();
            let db = state.db.lock().await;
            let _ = db.update_builder_session_status(
                &session_id,
                "FAILED",
                None,
                Some(&err_str),
                Some(&completed_at),
                duration_ms,
                &AgyUsage::default(),
                None,
            );

            let meta = serde_json::json!({
                "session_id": session_id,
                "error": err_str,
                "duration_ms": duration_ms,
            });
            let _ = ActivityManager::record_event(
                db.connection(),
                &payload.project_id,
                "BUILDER_TURN_FAILED",
                "BUILDER",
                &format!("Builder turn failed: {}", err_str),
                Some(&meta),
            );

            Err(CommandError::new("BUILDER_TURN_FAILED", err_str))
        }
    }
}

#[tauri::command]
pub async fn cancel_builder_turn(
    state: State<'_, AppState>,
    project_id: Option<String>,
) -> Result<(), CommandError> {
    state.cancel_flag.store(true, Ordering::Relaxed);
    if let Some(pid) = project_id {
        let db = state.db.lock().await;
        if let Ok(Some(session)) = db.get_latest_builder_session(&pid) {
            if session.status == "RUNNING" {
                let now = chrono::Utc::now().to_rfc3339();
                let _ = db.update_builder_session_status(
                    &session.session_id,
                    "CANCELED",
                    None,
                    None,
                    Some(&now),
                    session.duration_ms,
                    &session.usage,
                    session.conversation_id.as_deref(),
                );
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn get_usage_telemetry(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<UsageTelemetryReport, CommandError> {
    let db = state.db.lock().await;
    let chatgpt_summary = db
        .get_chatgpt_usage_summary(&project_id)
        .map_err(CommandError::from)?;
    let latest_session = db
        .get_latest_builder_session(&project_id)
        .map_err(CommandError::from)?;
    let (usage, active_model, active_effort) = match latest_session {
        Some(s) => (s.usage, Some(s.model), s.effort),
        None => (AgyUsage::default(), None, None),
    };

    Ok(UsageTelemetryReport {
        provider_antigravity_usage: usage,
        active_model,
        active_effort,
        chatgpt_estimated_usage: chatgpt_summary,
        context_window_note:
            "Context window remaining is not exposed by Antigravity CLI v1.1.27. No fabrication is performed."
                .to_string(),
        quota_buckets_note:
            "Account quota buckets and reset timers are not exposed by Antigravity CLI v1.1.27."
                .to_string(),
    })
}

#[tauri::command]
pub async fn reset_chatgpt_usage(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<ChatGptUsageSummary, CommandError> {
    let db = state.db.lock().await;
    db.reset_chatgpt_usage(&project_id)
        .map_err(CommandError::from)?;
    db.get_chatgpt_usage_summary(&project_id)
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn get_permission_history(
    state: State<'_, AppState>,
    project_id: String,
    limit: Option<usize>,
) -> Result<Vec<PermissionRecord>, CommandError> {
    let db = state.db.lock().await;
    db.list_permission_history(&project_id, limit.unwrap_or(50))
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn get_icarus_state(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<IcarusState, CommandError> {
    let db = state.db.lock().await;
    db.get_icarus_state(&project_id).map_err(CommandError::from)
}

#[tauri::command]
pub async fn set_icarus_mode(
    state: State<'_, AppState>,
    project_id: String,
    enabled: bool,
) -> Result<IcarusState, CommandError> {
    let db = state.db.lock().await;
    db.set_icarus_state(&project_id, enabled, Some("HUMAN"))
        .map_err(CommandError::from)?;
    let summary = if enabled {
        "Icarus Mode ENABLED: Antigravity will run with --dangerously-skip-permissions".to_string()
    } else {
        "Icarus Mode DISABLED: Standard permissions restored".to_string()
    };
    let meta = serde_json::json!({ "enabled": enabled });
    let _ = ActivityManager::record_event(
        db.connection(),
        &project_id,
        if enabled {
            "ICARUS_ENABLED"
        } else {
            "ICARUS_DISABLED"
        },
        "HUMAN",
        &summary,
        Some(&meta),
    );
    db.get_icarus_state(&project_id).map_err(CommandError::from)
}

#[tauri::command]
pub async fn desktop_clipboard_write(text: String) -> Result<(), CommandError> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|e| CommandError::new("CLIPBOARD_ERROR", e.to_string()))?;
    clipboard
        .set_text(text)
        .map_err(|e| CommandError::new("CLIPBOARD_ERROR", e.to_string()))?;
    Ok(())
}

#[tauri::command]
pub async fn desktop_clipboard_read() -> Result<String, CommandError> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|e| CommandError::new("CLIPBOARD_ERROR", e.to_string()))?;
    clipboard
        .get_text()
        .map_err(|e| CommandError::new("CLIPBOARD_ERROR", e.to_string()))
}

#[tauri::command]
pub async fn desktop_open_url(url: String) -> Result<(), CommandError> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(CommandError::new(
            "INVALID_URL",
            "Only HTTP and HTTPS URLs are allowed",
        ));
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", &url])
            .spawn()
            .map_err(|e| CommandError::new("URL_OPEN_ERROR", e.to_string()))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&url)
            .spawn()
            .map_err(|e| CommandError::new("URL_OPEN_ERROR", e.to_string()))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .map_err(|e| CommandError::new("URL_OPEN_ERROR", e.to_string()))?;
    }

    Ok(())
}

/// Narrows opening to strictly ChatGPT without arbitrary URL-opening capability.
#[tauri::command]
pub async fn open_chatgpt() -> Result<(), CommandError> {
    let url = "https://chatgpt.com";
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
            .map_err(|e| CommandError::new("URL_OPEN_ERROR", e.to_string()))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|e| CommandError::new("URL_OPEN_ERROR", e.to_string()))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|e| CommandError::new("URL_OPEN_ERROR", e.to_string()))?;
    }

    Ok(())
}

// ----------------------------------------------------------------------------
// Stage 2A ChatGPT Relay & Architecture Commands
// ----------------------------------------------------------------------------

fn get_repo_path_for_project_sync(
    db: &DbManager,
    project_id: &str,
) -> Result<PathBuf, CommandError> {
    let path_str: String = db
        .connection()
        .query_row(
            "SELECT repository_path FROM projects WHERE project_id = ?1",
            rusqlite::params![project_id],
            |r| r.get(0),
        )
        .map_err(|_| {
            CommandError::new(
                "PROJECT_NOT_FOUND",
                format!("Project {} not found", project_id),
            )
        })?;

    Ok(PathBuf::from(path_str))
}

#[tauri::command]
pub async fn prepare_architect_relay_packet(
    state: State<'_, AppState>,
    project_id: String,
    custom_notes: Option<String>,
) -> Result<RelayPacket, CommandError> {
    let mut db = state.db.lock().await;
    let repo_path = get_repo_path_for_project_sync(&db, &project_id)?;
    let packet = RelayService::prepare_architect_packet(
        db.connection_mut(),
        &repo_path,
        &project_id,
        custom_notes.as_deref(),
    )
    .map_err(CommandError::from)?;

    let char_count = packet.prompt.len();
    let estimated_tokens = char_count / 4;
    let _ = db.record_chatgpt_usage(
        &project_id,
        Some(&packet.metadata.packet_id),
        "OUTBOUND_PACKET",
        char_count,
        estimated_tokens,
    );

    Ok(packet)
}

#[tauri::command]
pub async fn get_pending_relay_packet(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<Option<RelayPacket>, CommandError> {
    let db = state.db.lock().await;
    RelayService::get_pending_packet(db.connection(), &project_id).map_err(CommandError::from)
}

#[tauri::command]
pub async fn get_relay_history(
    state: State<'_, AppState>,
    project_id: String,
    limit: Option<usize>,
) -> Result<Vec<RelayHistoryItem>, CommandError> {
    let db = state.db.lock().await;
    RelayService::get_relay_history(db.connection(), &project_id, limit.unwrap_or(20))
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn copy_relay_packet_to_clipboard(
    state: State<'_, AppState>,
    packet_id: String,
) -> Result<(), CommandError> {
    let db = state.db.lock().await;
    let prompt: String = db
        .connection()
        .query_row(
            "SELECT prompt FROM relay_packets WHERE packet_id = ?1",
            rusqlite::params![packet_id],
            |r| r.get(0),
        )
        .map_err(|_| {
            CommandError::new(
                "PACKET_NOT_FOUND",
                format!("Packet {} not found", packet_id),
            )
        })?;

    drop(db);

    desktop_clipboard_write(prompt).await
}

#[tauri::command]
pub async fn import_from_clipboard(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<ImportPreview, CommandError> {
    let clipboard_text = desktop_clipboard_read().await?;
    let db = state.db.lock().await;
    let repo_path = get_repo_path_for_project_sync(&db, &project_id)?;
    let preview =
        RelayService::process_import(db.connection(), &repo_path, &project_id, &clipboard_text)
            .map_err(CommandError::from)?;

    let char_count = clipboard_text.len();
    let estimated_tokens = char_count / 4;
    let _ = db.record_chatgpt_usage(
        &project_id,
        Some(&preview.packet_id),
        "INBOUND_IMPORT",
        char_count,
        estimated_tokens,
    );

    Ok(preview)
}

#[tauri::command]
pub async fn retry_parse_import(
    state: State<'_, AppState>,
    project_id: String,
    import_id: String,
    edited_raw_text: String,
) -> Result<ImportPreview, CommandError> {
    let db = state.db.lock().await;
    let repo_path = get_repo_path_for_project_sync(&db, &project_id)?;
    RelayService::retry_parse_import(
        db.connection(),
        &repo_path,
        &project_id,
        &import_id,
        &edited_raw_text,
    )
    .map_err(CommandError::from)
}

#[tauri::command]
pub async fn accept_relay_import(
    state: State<'_, AppState>,
    project_id: String,
    import_id: String,
) -> Result<ReadinessReport, CommandError> {
    let db = state.db.lock().await;
    let repo_path = get_repo_path_for_project_sync(&db, &project_id)?;
    RelayService::accept_import(db.connection(), &repo_path, &project_id, &import_id)
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn reject_relay_import(
    state: State<'_, AppState>,
    project_id: String,
    import_id: String,
) -> Result<(), CommandError> {
    let db = state.db.lock().await;
    RelayService::reject_import(db.connection(), &project_id, &import_id)
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn get_architecture_workspace_state(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<WorkspaceState, CommandError> {
    let db = state.db.lock().await;
    let repo_path = get_repo_path_for_project_sync(&db, &project_id)?;
    RelayService::get_workspace_state(db.connection(), &repo_path, &project_id)
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn get_artifact_content(
    state: State<'_, AppState>,
    project_id: String,
    artifact_path: String,
) -> Result<ArtifactContentDetails, CommandError> {
    let db = state.db.lock().await;
    let repo_path = get_repo_path_for_project_sync(&db, &project_id)?;
    RelayService::get_artifact_content(&repo_path, &artifact_path).map_err(CommandError::from)
}

#[tauri::command]
pub async fn save_artifact_content(
    state: State<'_, AppState>,
    project_id: String,
    artifact_path: String,
    content: String,
    expected_fingerprint: Option<String>,
) -> Result<ReadinessReport, CommandError> {
    let db = state.db.lock().await;
    let repo_path = get_repo_path_for_project_sync(&db, &project_id)?;
    RelayService::save_artifact_content(
        db.connection(),
        &repo_path,
        &project_id,
        &artifact_path,
        &content,
        expected_fingerprint.as_deref(),
    )
    .map_err(CommandError::from)
}

#[tauri::command]
pub async fn set_active_project_id(
    state: State<'_, AppState>,
    project_id: Option<String>,
) -> Result<(), CommandError> {
    *state.active_project_id.lock().await = project_id;
    Ok(())
}

#[tauri::command]
pub async fn set_project_artifact_applicability(
    state: State<'_, AppState>,
    project_id: String,
    artifact_path: String,
    applicability: ArtifactApplicability,
) -> Result<WorkspaceState, CommandError> {
    let db = state.db.lock().await;
    let repo_path = get_repo_path_for_project_sync(&db, &project_id)?;
    RelayService::set_project_artifact_applicability(
        db.connection(),
        &repo_path,
        &project_id,
        &artifact_path,
        applicability,
    )
    .map_err(CommandError::from)
}

// ----------------------------------------------------------------------------
// Stage 2B Architecture Freeze & Git Boundaries Commands
// ----------------------------------------------------------------------------

#[tauri::command]
pub async fn prepare_architecture_freeze(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<crate::core::freeze::FreezePreview, CommandError> {
    let db = state.db.lock().await;
    let repo_path = get_repo_path_for_project_sync(&db, &project_id)?;

    let mut git_lock = state.git.lock().await;
    if git_lock.is_none() {
        *git_lock = Some(GitAdapter::new().map_err(CommandError::from)?);
    }
    let git = git_lock.as_ref().unwrap();

    crate::core::freeze::FreezeService::prepare_freeze_preview(
        &repo_path,
        &project_id,
        git,
        db.connection(),
    )
    .map_err(CommandError::from)
}

#[tauri::command]
pub async fn confirm_architecture_freeze(
    state: State<'_, AppState>,
    project_id: String,
    preview_id: String,
) -> Result<crate::core::freeze::FreezeResult, CommandError> {
    let mut db = state.db.lock().await;
    let repo_path = get_repo_path_for_project_sync(&db, &project_id)?;

    let mut git_lock = state.git.lock().await;
    if git_lock.is_none() {
        *git_lock = Some(GitAdapter::new().map_err(CommandError::from)?);
    }
    let git = git_lock.as_ref().unwrap();

    crate::core::freeze::FreezeService::confirm_freeze(
        &repo_path,
        &project_id,
        &preview_id,
        git,
        db.connection_mut(),
    )
    .map_err(CommandError::from)
}

#[tauri::command]
pub async fn get_contract_drift(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<crate::core::freeze::DriftReport, CommandError> {
    let mut db = state.db.lock().await;
    let repo_path = get_repo_path_for_project_sync(&db, &project_id)?;
    crate::core::freeze::FreezeService::reconcile_drift_restoration(
        &repo_path,
        &project_id,
        db.connection_mut(),
    )
    .map_err(CommandError::from)?;
    crate::core::freeze::FreezeService::check_contract_drift(&repo_path, &project_id)
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn get_drift_diff(
    state: State<'_, AppState>,
    project_id: String,
    artifact_path: String,
) -> Result<crate::core::freeze::DriftDiff, CommandError> {
    let db = state.db.lock().await;
    let repo_path = get_repo_path_for_project_sync(&db, &project_id)?;
    crate::core::freeze::FreezeService::get_drift_diff(&repo_path, &project_id, &artifact_path)
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn restore_drifted_artifact(
    state: State<'_, AppState>,
    project_id: String,
    artifact_path: String,
) -> Result<crate::core::freeze::DriftReport, CommandError> {
    let mut db = state.db.lock().await;
    let repo_path = get_repo_path_for_project_sync(&db, &project_id)?;
    crate::core::freeze::FreezeService::restore_drifted_artifact(
        &repo_path,
        &project_id,
        &artifact_path,
        db.connection_mut(),
    )
    .map_err(CommandError::from)
}

#[tauri::command]
pub async fn restore_all_drifted_artifacts(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<crate::core::freeze::DriftReport, CommandError> {
    let mut db = state.db.lock().await;
    let repo_path = get_repo_path_for_project_sync(&db, &project_id)?;
    crate::core::freeze::FreezeService::restore_all_drifted_artifacts(
        &repo_path,
        &project_id,
        db.connection_mut(),
    )
    .map_err(CommandError::from)
}

#[tauri::command]
pub async fn get_builder_packet(
    state: State<'_, AppState>,
    project_id: String,
    version: Option<String>,
) -> Result<crate::core::freeze::BuilderPacket, CommandError> {
    let db = state.db.lock().await;
    let repo_path = get_repo_path_for_project_sync(&db, &project_id)?;
    if let Some(ref v) = version {
        crate::core::freeze::validate_architecture_version(v).map_err(CommandError::from)?;
    }
    crate::core::freeze::FreezeService::get_builder_packet(&repo_path, version.as_deref())
        .map_err(CommandError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::artifacts::ArtifactManager;
    use crate::core::freeze::FreezeService;
    use crate::core::git::GitAdapter;
    use crate::db::DbManager;
    use rusqlite::params;
    use std::fs;
    use tempfile::tempdir;

    fn setup_test_git_repo(path: &std::path::Path) {
        std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(path)
            .output()
            .expect("git init");
        std::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(path)
            .output()
            .expect("git config user.name");
        std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(path)
            .output()
            .expect("git config user.email");
        fs::write(path.join("README.md"), "# Init").unwrap();
        std::process::Command::new("git")
            .args(["add", "README.md"])
            .current_dir(path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "Initial"])
            .current_dir(path)
            .output()
            .unwrap();
    }

    fn populate_artifacts(root: &std::path::Path) {
        for path in crate::core::artifacts::CANONICAL_ARCHITECTURE_ARTIFACTS {
            let full = root.join(".coalition").join(path);
            if let Some(p) = full.parent() {
                fs::create_dir_all(p).unwrap();
            }
            if path.ends_with(".yaml") {
                fs::write(&full, "version: 1\nschema_version: 1\nitems: []\n").unwrap();
            } else {
                fs::write(
                    &full,
                    format!("# {}\n\nSubstantive content for {}\n", path, path),
                )
                .unwrap();
            }
        }
    }

    #[test]
    fn test_start_build_fails_on_missing_project() {
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();

        let err =
            apply_workflow_action_impl(&mut db, "non-existent-proj", WorkflowAction::StartBuild)
                .unwrap_err();
        assert_eq!(err.code, "PROJECT_NOT_FOUND");
    }

    #[test]
    fn test_start_build_fails_on_unavailable_repo_path() {
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();

        db.connection().execute(
            "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at)
             VALUES ('proj-1', 'Test', 'C:/non_existent_dir_99999', '2026-01-01', '2026-01-01', '2026-01-01')",
            [],
        ).unwrap();

        let err =
            apply_workflow_action_impl(&mut db, "proj-1", WorkflowAction::StartBuild).unwrap_err();
        assert_eq!(err.code, "REPOSITORY_UNAVAILABLE");
    }

    #[test]
    fn test_start_build_fails_on_corrupt_frozen_snapshot_and_drift() {
        let dir = tempdir().unwrap();
        setup_test_git_repo(dir.path());
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();

        let py = ArtifactManager::initialize_new_project(dir.path(), "start-build-test").unwrap();
        let proj_id = &py.project_id;

        db.connection().execute(
            "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at)
             VALUES (?1, ?2, ?3, '2026-01-01', '2026-01-01', '2026-01-01')",
            params![proj_id, "Test", dir.path().to_string_lossy().to_string()],
        ).unwrap();
        db.connection()
            .execute(
                "INSERT INTO workflow_state (project_id, state, revision, updated_at)
             VALUES (?1, 'READY_TO_FREEZE', 1, '2026-01-01')",
                params![proj_id],
            )
            .unwrap();

        populate_artifacts(dir.path());

        let git = GitAdapter::new().unwrap();
        let preview =
            FreezeService::prepare_freeze_preview(dir.path(), proj_id, &git, db.connection())
                .unwrap();

        FreezeService::confirm_freeze(
            dir.path(),
            proj_id,
            &preview.preview_id,
            &git,
            db.connection_mut(),
        )
        .unwrap();

        // Workflow state is now FROZEN.
        // 1. Mutate contract artifact -> drift detected -> StartBuild fails closed!
        ArtifactManager::write_artifact_atomic(
            dir.path(),
            "design/product-vision.md",
            "# Tampered Content\n",
        )
        .unwrap();

        let drift_err =
            apply_workflow_action_impl(&mut db, proj_id, WorkflowAction::StartBuild).unwrap_err();
        assert_eq!(drift_err.code, "FROZEN_CONTRACT_DRIFT_DETECTED");

        // 2. Corrupt frozen snapshot -> StartBuild fails closed!
        let manifest_path = dir
            .path()
            .join(".coalition")
            .join("architecture-versions")
            .join("v1.0")
            .join("contract-manifest.yaml");
        fs::write(&manifest_path, "corrupt yaml: [}").unwrap();

        let corrupt_err =
            apply_workflow_action_impl(&mut db, proj_id, WorkflowAction::StartBuild).unwrap_err();
        assert_eq!(corrupt_err.code, "FROZEN_SNAPSHOT_CORRUPT");
    }

    #[test]
    fn test_start_build_fails_on_unresolved_restoration_journal() {
        let dir = tempdir().unwrap();
        setup_test_git_repo(dir.path());
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();

        let py = ArtifactManager::initialize_new_project(dir.path(), "start-build-journal-test")
            .unwrap();
        let proj_id = &py.project_id;

        db.connection().execute(
            "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at)
             VALUES (?1, ?2, ?3, '2026-01-01', '2026-01-01', '2026-01-01')",
            params![proj_id, "Test", dir.path().to_string_lossy().to_string()],
        ).unwrap();
        db.connection()
            .execute(
                "INSERT INTO workflow_state (project_id, state, revision, updated_at)
             VALUES (?1, 'READY_TO_FREEZE', 1, '2026-01-01')",
                params![proj_id],
            )
            .unwrap();

        populate_artifacts(dir.path());

        let git = GitAdapter::new().unwrap();
        let preview =
            FreezeService::prepare_freeze_preview(dir.path(), proj_id, &git, db.connection())
                .unwrap();

        FreezeService::confirm_freeze(
            dir.path(),
            proj_id,
            &preview.preview_id,
            &git,
            db.connection_mut(),
        )
        .unwrap();

        // Place a corrupt/unresolvable journal on disk
        let rec_dir = dir.path().join(".coalition").join("recovery");
        fs::create_dir_all(&rec_dir).unwrap();
        fs::write(
            rec_dir.join("drift-restoration-journal.json"),
            "{ invalid json",
        )
        .unwrap();

        let err =
            apply_workflow_action_impl(&mut db, proj_id, WorkflowAction::StartBuild).unwrap_err();
        assert_eq!(err.code, "DRIFT_RESTORATION_RECOVERY_REQUIRED");
    }
}

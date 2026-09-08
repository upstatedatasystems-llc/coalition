use crate::core::activity::{ActivityEventRecord, ActivityManager};
use crate::core::artifacts::ArtifactError;
use crate::core::builder::{
    AgyEvent, AntigravityCliAdapter, BuilderTurnRequest, BuilderTurnResponse, ModelInfo,
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
            RelayError::AlreadyDecided(s) => {
                Self::new("IMPORT_ALREADY_DECIDED", format!("Import already {}", s))
            }
            #[cfg(test)]
            RelayError::InjectedFailure(msg) => Self::new("INJECTED_FAILURE", msg),
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

    if details.is_available {
        let repo_path = PathBuf::from(&details.project.repository_path);
        let _ = RelayService::reconcile_interrupted_batches(
            db.connection(),
            &repo_path,
            &details.project.project_id,
        );
    }

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

    if details.is_available {
        let repo_path = PathBuf::from(&details.project.repository_path);
        let _ = RelayService::reconcile_interrupted_batches(
            db.connection(),
            &repo_path,
            &details.project.project_id,
        );
    }

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

#[tauri::command]
pub async fn apply_workflow_action(
    state: State<'_, AppState>,
    project_id: String,
    action: WorkflowAction,
) -> Result<WorkflowStateRecord, CommandError> {
    let mut db = state.db.lock().await;
    workflow::apply_workflow_action(db.connection_mut(), &project_id, action, "HUMAN")
        .map_err(CommandError::from)
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartBuilderTurnPayload {
    pub prompt: String,
    pub conversation_id: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub icarus_mode: bool,
    pub use_fake_agy: bool,
}

#[tauri::command]
pub async fn start_builder_turn(
    app: AppHandle,
    state: State<'_, AppState>,
    payload: StartBuilderTurnPayload,
) -> Result<BuilderTurnResponse, CommandError> {
    state.cancel_flag.store(false, Ordering::Relaxed);

    let adapter = if payload.use_fake_agy {
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

    let (tx, mut rx) = mpsc::channel::<AgyEvent>(100);

    let app_clone = app.clone();
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            let _ = app_clone.emit("agy-stream-event", &event);
        }
    });

    let current_dir = std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().to_string());

    let request = BuilderTurnRequest {
        prompt: payload.prompt,
        conversation_id: payload.conversation_id,
        model: payload.model,
        effort: payload.effort,
        icarus_mode: payload.icarus_mode,
        working_dir: current_dir,
    };

    let result = adapter
        .run_turn(request, state.cancel_flag.clone(), Some(tx))
        .await
        .map_err(|e| CommandError::new("BUILDER_TURN_FAILED", e.to_string()))?;

    Ok(result)
}

#[tauri::command]
pub async fn cancel_builder_turn(state: State<'_, AppState>) -> Result<(), CommandError> {
    state.cancel_flag.store(true, Ordering::Relaxed);
    Ok(())
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
    RelayService::prepare_architect_packet(
        db.connection_mut(),
        &repo_path,
        &project_id,
        custom_notes.as_deref(),
    )
    .map_err(CommandError::from)
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
    RelayService::process_import(db.connection(), &repo_path, &project_id, &clipboard_text)
        .map_err(CommandError::from)
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

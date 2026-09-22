use crate::core::activity::{ActivityError, ActivityManager};
use crate::core::artifacts::{
    ArchitectureState, ArtifactApplicability, ArtifactError, ArtifactManager,
    CANONICAL_ARCHITECTURE_ARTIFACTS,
};
use crate::core::git::{GitAdapter, GitError};
use crate::core::relay::readiness::{OverallReadiness, ReadinessEvaluator};
use crate::core::workflow::{self, WorkflowAction, WorkflowError, WorkflowState};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;
use thiserror::Error;
use uuid::Uuid;

pub const FREEZE_SCHEMA_VERSION: u32 = 1;
pub const MANIFEST_VERSION: u32 = 1;
pub const INITIAL_ARCHITECTURE_VERSION: &str = "1.0";
pub const BUILDER_PACKET_MAX_BYTES: usize = 200 * 1024; // 200 KB budget

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FrozenGitBoundary {
    pub head_commit: String,
    pub branch: Option<String>,
    pub is_detached: bool,
    pub is_clean: bool,
    pub staged_count: usize,
    pub unstaged_count: usize,
    pub untracked_count: usize,
    pub dirty_fingerprint: String,
    pub porcelain_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BuilderPacketSummary {
    pub total_artifacts: usize,
    pub total_characters: usize,
    pub prompt_bytes: usize,
    pub estimated_tokens: usize,
    pub is_truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FreezePreview {
    pub preview_id: String,
    pub project_id: String,
    pub target_version: String,
    pub readiness_policy_version: u32,
    pub ready_required_count: usize,
    pub total_required_count: usize,
    pub unresolved_open_questions_count: usize,
    pub artifact_baselines: BTreeMap<String, String>,
    pub git_boundary: FrozenGitBoundary,
    pub builder_packet_summary: BuilderPacketSummary,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractArtifactEntry {
    pub relative_path: String,
    pub fingerprint: String,
    pub size_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContractManifest {
    pub schema_version: u32,
    pub manifest_version: u32,
    pub project_id: String,
    pub architecture_version: String,
    pub frozen_at: String,
    pub frozen_by: String,
    pub builder_epoch_id: String,
    pub readiness_policy_version: u32,
    pub applicability: BTreeMap<String, ArtifactApplicability>,
    pub unresolved_open_questions_count: usize,
    pub git_boundary: FrozenGitBoundary,
    pub contract_fingerprint: String,
    pub artifacts: Vec<ContractArtifactEntry>,
    pub builder_packet_fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BuilderPacketMetadata {
    pub schema_version: u32,
    pub packet_id: String,
    pub project_id: String,
    pub project_name: String,
    pub architecture_version: String,
    pub builder_epoch_id: String,
    pub created_at: String,
    pub contract_fingerprint: String,
    pub git_head_commit: String,
    pub git_branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BuilderPacketArtifact {
    pub path: String,
    pub title: String,
    pub content: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BuilderPacket {
    pub metadata: BuilderPacketMetadata,
    pub summary: String,
    pub builder_rules: String,
    pub artifacts: Vec<BuilderPacketArtifact>,
    pub is_truncated: bool,
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FreezeResult {
    pub project_id: String,
    pub architecture_version: String,
    pub epoch_id: String,
    pub frozen_at: String,
    pub manifest_fingerprint: String,
    pub contract_fingerprint: String,
    pub git_boundary: FrozenGitBoundary,
    pub snapshot_path: String,
    pub builder_packet: BuilderPacket,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DriftType {
    Modified,
    Deleted,
    Added,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DriftedArtifact {
    pub path: String,
    pub drift_type: DriftType,
    pub frozen_fingerprint: Option<String>,
    pub active_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DriftReport {
    pub has_drift: bool,
    pub is_frozen: bool,
    pub architecture_version: Option<String>,
    pub drifted_artifacts: Vec<DriftedArtifact>,
    pub checked_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DriftDiff {
    pub path: String,
    pub drift_type: DriftType,
    pub frozen_content: Option<String>,
    pub active_content: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RestorationPhase {
    Staged,
    Committing,
    Committed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RestorationOp {
    pub path: String,
    pub drift_type: DriftType,
    pub staged_file_rel: Option<String>,
    pub quarantine_dest_rel: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DriftRestorationJournal {
    pub journal_id: String,
    pub project_id: String,
    pub architecture_version: String,
    pub phase: RestorationPhase,
    pub operations: Vec<RestorationOp>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Error, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FreezeError {
    #[error(
        "Readiness incomplete: cannot freeze project because required artifacts are not ready: {0}"
    )]
    ReadinessIncomplete(String),
    #[error("Illegal workflow state for freeze: project is in state '{current}', but must be in READY_TO_FREEZE")]
    IllegalWorkflowState { current: String },
    #[error("Repository has no valid HEAD commit (unborn repository). A valid Git commit boundary is required before freezing architecture")]
    NoHeadCommit,
    #[error("Stale freeze preview: {0}. Please refresh preview and confirm again")]
    StaleFreezePreview(String),
    #[error("Architecture version '{0}' already exists and is immutable")]
    VersionAlreadyExists(String),
    #[error("Frozen snapshot for version '{version}' is corrupt: {reason}")]
    FrozenSnapshotCorrupt { version: String, reason: String },
    #[error("Freeze recovery required: {0}")]
    FreezeRecoveryRequired(String),
    #[error("Drift restoration recovery required: {0}")]
    DriftRestorationRecoveryRequired(String),
    #[error("Invalid architecture version: {0}")]
    InvalidArchitectureVersion(String),
    #[error("Artifact error: {0}")]
    Artifact(String),
    #[error("Git error: {0}")]
    Git(String),
    #[error("Workflow error: {0}")]
    Workflow(String),
    #[error("Database error: {0}")]
    Database(String),
    #[error("IO error: {0}")]
    Io(String),
}

impl From<ArtifactError> for FreezeError {
    fn from(e: ArtifactError) -> Self {
        match e {
            ArtifactError::RecoveryRequired(r) => Self::FreezeRecoveryRequired(r),
            _ => Self::Artifact(e.to_string()),
        }
    }
}

impl From<GitError> for FreezeError {
    fn from(e: GitError) -> Self {
        Self::Git(e.to_string())
    }
}

impl From<WorkflowError> for FreezeError {
    fn from(e: WorkflowError) -> Self {
        Self::Workflow(e.to_string())
    }
}

impl From<ActivityError> for FreezeError {
    fn from(e: ActivityError) -> Self {
        Self::Database(e.to_string())
    }
}

impl From<rusqlite::Error> for FreezeError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Database(e.to_string())
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InjectedFreezeSeam {
    None,
    PreStaging,
    MidStaging,
    PostStagingPreFinalize,
    PostFinalizePreCommit,
    PostCommitPreDb,
    MidDbTransaction,
}

#[cfg(test)]
thread_local! {
    pub static INJECTED_FREEZE_SEAM: std::cell::Cell<InjectedFreezeSeam> = const { std::cell::Cell::new(InjectedFreezeSeam::None) };
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InjectedRestoreSeam {
    None,
    BeforeFirstRestoreMutation,
    AfterFirstRestoreMutation,
    MidRestoreBatch,
    AfterAllMutationsBeforeVerification,
    DuringAddedArtifactQuarantine,
    AfterVerificationBeforeJournalCleanup,
}

#[cfg(test)]
thread_local! {
    pub static INJECTED_RESTORE_SEAM: std::cell::Cell<InjectedRestoreSeam> = const { std::cell::Cell::new(InjectedRestoreSeam::None) };
}

/// Finds the largest byte index <= max_bytes that falls on a UTF-8 character boundary.
pub fn floor_char_boundary(s: &str, max_bytes: usize) -> usize {
    if max_bytes >= s.len() {
        return s.len();
    }
    let mut index = max_bytes;
    while index > 0 && !s.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// Durably synchronizes directory metadata to disk where supported.
/// On POSIX systems, opens the directory and invokes `sync_all()`.
/// On Windows NTFS, directory entries and renames are journaled into the transactional
/// NTFS metadata log ($LogFile); where supported, an explicit FlushFileBuffers is issued
/// on the directory handle opened with backup semantics.
pub fn durable_directory_sync(dir: &Path) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    {
        let f = fs::File::open(dir)?;
        f.sync_all()?;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x02000000;
        const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
        const FILE_READ_ATTRIBUTES: u32 = 0x0080;
        const FILE_SHARE_READ: u32 = 0x00000001;
        const FILE_SHARE_WRITE: u32 = 0x00000002;
        const FILE_SHARE_DELETE: u32 = 0x00000004;

        let f = match fs::OpenOptions::new()
            .access_mode(FILE_WRITE_ATTRIBUTES | FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(dir)
        {
            Ok(f) => f,
            Err(_) => {
                // Fall back to read handle
                fs::OpenOptions::new()
                    .read(true)
                    .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                    .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                    .open(dir)?
            }
        };

        if let Err(e) = f.sync_all() {
            match e.raw_os_error() {
                Some(1) | Some(5) | Some(50) => {}
                _ => return Err(e),
            }
        }
    }
    Ok(())
}

/// Strictly validates a path string from an untrusted restoration journal.
/// Rejects empty paths, NUL/control characters, parent directory traversals ('..'),
/// absolute paths, and drive-letter / UNC prefixes.
pub fn validate_journal_path(path: &str) -> Result<String, FreezeError> {
    if path.trim().is_empty() {
        return Err(FreezeError::DriftRestorationRecoveryRequired(
            "Restoration journal contains an empty path".to_string(),
        ));
    }
    if path.chars().any(|c| c.is_control() || c == '\0') {
        return Err(FreezeError::DriftRestorationRecoveryRequired(format!(
            "Restoration journal path contains NUL or control characters: {:?}",
            path
        )));
    }
    let normalized = path.replace('\\', "/");
    if normalized.starts_with('/')
        || normalized.contains("..")
        || Path::new(&normalized).is_absolute()
        || (normalized.len() >= 2 && normalized.as_bytes()[1] == b':')
        || normalized.starts_with("//")
        || normalized.starts_with("\\\\")
    {
        return Err(FreezeError::DriftRestorationRecoveryRequired(format!(
            "Restoration journal path contains forbidden traversal or absolute path prefix: '{}'",
            path
        )));
    }
    Ok(normalized)
}

/// Strictly validates all paths, identifiers, and versions in a restoration journal
/// before any filesystem operations are attempted.
pub fn validate_restoration_journal(
    coalition_dir: &Path,
    journal: &DriftRestorationJournal,
) -> Result<(), FreezeError> {
    // 1. Validate journal ID
    let jid = journal.journal_id.trim();
    if jid.is_empty()
        || jid.len() > 64
        || jid
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '/' || c == '\\' || c == '.')
    {
        return Err(FreezeError::DriftRestorationRecoveryRequired(format!(
            "Restoration journal ID '{}' contains forbidden characters",
            journal.journal_id
        )));
    }

    // 2. Validate architecture version
    validate_architecture_version(&journal.architecture_version).map_err(|e| {
        FreezeError::DriftRestorationRecoveryRequired(format!(
            "Restoration journal architecture version '{}' is invalid: {}",
            journal.architecture_version, e
        ))
    })?;

    // 3. Validate operations
    let expected_staged_prefix = format!("recovery/.staging-restore-{}/", jid);
    let expected_quarantine_prefix = "recovery/quarantine-";

    for op in &journal.operations {
        let norm_path = validate_journal_path(&op.path)?;
        if !ArtifactManager::is_valid_architecture_artifact_path(&norm_path) {
            return Err(FreezeError::DriftRestorationRecoveryRequired(format!(
                "Restoration operation target path '{}' is not an allowed governed architecture path",
                op.path
            )));
        }

        if let Some(ref staged_rel) = op.staged_file_rel {
            let norm_staged = validate_journal_path(staged_rel)?;
            if !norm_staged.starts_with(&expected_staged_prefix) {
                return Err(FreezeError::DriftRestorationRecoveryRequired(format!(
                    "Staged file path '{}' does not reside beneath expected staging dir '{}'",
                    staged_rel, expected_staged_prefix
                )));
            }
        }

        if let Some(ref q_rel) = op.quarantine_dest_rel {
            let norm_q = validate_journal_path(q_rel)?;
            if !norm_q.starts_with(expected_quarantine_prefix) {
                return Err(FreezeError::DriftRestorationRecoveryRequired(format!(
                    "Quarantine destination path '{}' does not reside beneath '{}'",
                    q_rel, expected_quarantine_prefix
                )));
            }
        }

        // Canonical containment check for target path
        let target_full = coalition_dir.join(&norm_path);
        if let Some(parent) = target_full.parent() {
            if parent.exists() {
                if let (Ok(canon_coalition), Ok(canon_parent)) =
                    (coalition_dir.canonicalize(), parent.canonicalize())
                {
                    if !canon_parent.starts_with(&canon_coalition) {
                        return Err(FreezeError::DriftRestorationRecoveryRequired(format!(
                            "Target path {:?} escapes .coalition directory",
                            target_full
                        )));
                    }
                }
            }
        }
    }

    Ok(())
}

/// Strictly validates an architecture version string.
/// Allows 1-32 chars of ASCII alphanumeric, dots, and hyphens (e.g. "1.0", "1.0-alpha").
/// Strictly rejects empty strings, path traversals (".."), slashes, whitespace, and control chars.
pub fn validate_architecture_version(version: &str) -> Result<(), FreezeError> {
    let trimmed = version.trim();
    if trimmed.is_empty() || trimmed.len() > 32 {
        return Err(FreezeError::InvalidArchitectureVersion(format!(
            "Architecture version must be between 1 and 32 characters, got '{}'",
            version
        )));
    }
    if trimmed.contains("..")
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(FreezeError::InvalidArchitectureVersion(format!(
            "Architecture version contains forbidden characters or path traversal: '{}'",
            version
        )));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return Err(FreezeError::InvalidArchitectureVersion(format!(
            "Architecture version has invalid format: '{}'",
            version
        )));
    }
    Ok(())
}

pub struct FreezeService;

impl FreezeService {
    /// Strictly validates an architecture version string.
    pub fn validate_architecture_version(version: &str) -> Result<(), FreezeError> {
        validate_architecture_version(version)
    }

    /// Strictly validates a restoration journal before performing any mutations.
    pub fn validate_restoration_journal(
        coalition_dir: &Path,
        journal: &DriftRestorationJournal,
    ) -> Result<(), FreezeError> {
        validate_restoration_journal(coalition_dir, journal)
    }

    /// Computes SHA-256 hex string for given bytes.
    pub fn sha256_hex(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        format!("{:x}", hasher.finalize())
    }

    /// Captures the authoritative Git boundary. Fails if repo has no valid HEAD commit.
    pub fn capture_git_boundary<P: AsRef<Path>>(
        git: &GitAdapter,
        repo_root: P,
    ) -> Result<FrozenGitBoundary, FreezeError> {
        let root = repo_root.as_ref();
        let info = git.inspect_repo(root)?;
        let head_commit = info.head_commit.ok_or(FreezeError::NoHeadCommit)?;

        if head_commit.trim().is_empty() {
            return Err(FreezeError::NoHeadCommit);
        }

        let detailed = git.compute_detailed_dirty_state(root)?;

        Ok(FrozenGitBoundary {
            head_commit,
            branch: info.current_branch,
            is_detached: info.is_detached,
            is_clean: detailed.is_clean,
            staged_count: detailed.staged_count,
            unstaged_count: detailed.unstaged_count,
            untracked_count: detailed.untracked_count,
            dirty_fingerprint: detailed.composite_fingerprint,
            porcelain_status: detailed.raw_porcelain,
        })
    }

    /// Scans repo root for all currently active governed architecture artifacts.
    pub fn list_active_governed_artifacts<P: AsRef<Path>>(
        repo_root: P,
    ) -> Result<Vec<String>, FreezeError> {
        let root = repo_root.as_ref();
        let coalition_dir = ArtifactManager::resolve_coalition_dir(root)?;
        let mut active = Vec::new();

        // 1. Canonical architecture artifacts
        for rel in CANONICAL_ARCHITECTURE_ARTIFACTS {
            let target = coalition_dir.join(rel);
            if target.exists() {
                active.push(rel.to_string());
            }
        }

        // 2. Open questions if present
        let oq = coalition_dir.join("design/open-questions.md");
        if oq.exists() && !active.contains(&"design/open-questions.md".to_string()) {
            active.push("design/open-questions.md".to_string());
        }

        // 3. Managed ADRs (decisions/ADR-*.md)
        let decisions_dir = coalition_dir.join("decisions");
        if decisions_dir.is_dir() {
            if let Ok(entries) = fs::read_dir(&decisions_dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with("ADR-") && name.ends_with(".md") {
                        let rel = format!("decisions/{}", name);
                        if !active.contains(&rel) {
                            active.push(rel);
                        }
                    }
                }
            }
        }

        active.sort();
        Ok(active)
    }

    /// Computes baseline fingerprints for all active governed contract artifacts.
    pub fn compute_active_contract_baselines<P: AsRef<Path>>(
        repo_root: P,
    ) -> Result<BTreeMap<String, String>, FreezeError> {
        let root = repo_root.as_ref();
        let active_paths = Self::list_active_governed_artifacts(root)?;
        let mut baselines = BTreeMap::new();

        for rel in active_paths {
            if let Some(fp) = ArtifactManager::compute_file_fingerprint(root, &rel)? {
                baselines.insert(rel, fp);
            }
        }

        Ok(baselines)
    }

    /// Preflight check that prepares an authoritative FreezePreview for human confirmation.
    /// Persists the immutable Rust-owned preview record in the database under `preview_id`.
    /// Supersedes any existing `PENDING` freeze previews for this project.
    pub fn prepare_freeze_preview<P: AsRef<Path>>(
        repo_root: P,
        project_id: &str,
        git: &GitAdapter,
        conn: &Connection,
    ) -> Result<FreezePreview, FreezeError> {
        let root = repo_root.as_ref();
        let coalition_dir = ArtifactManager::resolve_coalition_dir(root)?;
        let project_yaml_path = coalition_dir.join("project.yaml");

        if !project_yaml_path.exists() {
            return Err(FreezeError::Artifact(format!(
                "project.yaml missing at {:?}",
                project_yaml_path
            )));
        }

        let project_yaml = ArtifactManager::read_project_yaml(&project_yaml_path)?;
        if project_yaml.project_id != project_id {
            return Err(FreezeError::Artifact(format!(
                "Project ID mismatch: requested '{}', found '{}'",
                project_id, project_yaml.project_id
            )));
        }

        if project_yaml.architecture_state != ArchitectureState::Draft {
            return Err(FreezeError::IllegalWorkflowState {
                current: project_yaml.architecture_state.to_string(),
            });
        }

        // 1. Evaluate readiness
        let readiness = ReadinessEvaluator::evaluate(root)?;
        if readiness.overall_readiness != OverallReadiness::ReadyToFreeze {
            return Err(FreezeError::ReadinessIncomplete(format!(
                "{}/{} required artifacts ready",
                readiness.ready_required_count, readiness.total_required_count
            )));
        }

        // 2. Capture Git boundary (fails if unborn / no HEAD commit)
        let git_boundary = Self::capture_git_boundary(git, root)?;

        // 3. Compute baseline fingerprints of all active governed contract artifacts
        let artifact_baselines = Self::compute_active_contract_baselines(root)?;

        // 4. Summarize Builder packet using the exact shared prompt builder
        let mut prospective_manifest_artifacts = Vec::new();
        let mut total_characters = 0;
        for (rel, fp) in &artifact_baselines {
            let content = ArtifactManager::read_artifact(root, rel)?.unwrap_or_default();
            total_characters += content.len();
            prospective_manifest_artifacts.push(ContractArtifactEntry {
                relative_path: rel.clone(),
                fingerprint: fp.clone(),
                size_bytes: content.len(),
            });
        }

        let mut contract_hasher = Sha256::new();
        for entry in &prospective_manifest_artifacts {
            contract_hasher.update(entry.relative_path.as_bytes());
            contract_hasher.update(entry.fingerprint.as_bytes());
            contract_hasher.update(entry.size_bytes.to_le_bytes());
        }
        let prospective_contract_fingerprint = format!("{:x}", contract_hasher.finalize());

        let (_prompt, _rules, _arts, is_truncated, prompt_bytes) =
            Self::build_builder_packet_prompt_and_artifacts(
                INITIAL_ARCHITECTURE_VERSION,
                &project_yaml.name,
                project_id,
                &git_boundary.head_commit,
                &prospective_contract_fingerprint,
                &prospective_manifest_artifacts,
                |rel| {
                    ArtifactManager::read_artifact(root, rel)?
                        .ok_or_else(|| FreezeError::Artifact(format!("Artifact missing: {}", rel)))
                },
            )?;

        let summary = BuilderPacketSummary {
            total_artifacts: artifact_baselines.len(),
            total_characters,
            prompt_bytes,
            estimated_tokens: (prompt_bytes / 4).max(1),
            is_truncated,
        };

        let preview = FreezePreview {
            preview_id: Uuid::new_v4().to_string(),
            project_id: project_id.to_string(),
            target_version: INITIAL_ARCHITECTURE_VERSION.to_string(),
            readiness_policy_version: readiness.policy_version,
            ready_required_count: readiness.ready_required_count,
            total_required_count: readiness.total_required_count,
            unresolved_open_questions_count: readiness.unresolved_open_questions_count,
            artifact_baselines,
            git_boundary,
            builder_packet_summary: summary,
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        // Persist server-owned preview record: supersede older pending previews first
        conn.execute(
            "UPDATE freeze_previews SET status = 'SUPERSEDED' WHERE project_id = ?1 AND status = 'PENDING'",
            params![project_id],
        )?;

        let preview_json = serde_json::to_string(&preview).map_err(|e| {
            FreezeError::Artifact(format!("Failed to serialize freeze preview: {}", e))
        })?;

        conn.execute(
            "INSERT INTO freeze_previews (preview_id, project_id, target_version, readiness_policy_version, preview_json, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'PENDING', ?6)",
            params![
                preview.preview_id,
                project_id,
                preview.target_version,
                preview.readiness_policy_version as i64,
                preview_json,
                preview.created_at,
            ],
        )?;

        Ok(preview)
    }

    /// Builds the prompt and artifacts for a Builder implementation packet using a shared calculation
    /// between preflight preview and snapshot finalization.
    /// Strictly enforces the 200 KB context budget against the full prompt context transmitted to the Builder.
    pub fn build_builder_packet_prompt_and_artifacts<F>(
        architecture_version: &str,
        project_name: &str,
        project_id: &str,
        git_head_commit: &str,
        contract_fingerprint: &str,
        manifest_artifacts: &[ContractArtifactEntry],
        load_content: F,
    ) -> Result<(String, String, Vec<BuilderPacketArtifact>, bool, usize), FreezeError>
    where
        F: Fn(&str) -> Result<String, FreezeError>,
    {
        let builder_rules = r#"=== COALITION BUILDER INSTRUCTIONS ===
1. You are Google Antigravity acting as the Builder for this project.
2. The architecture specifications in this packet are FROZEN and IMMUTABLE.
3. Do not silently change, weaken, or omit requirements or constraints.
4. If an implementation contradiction is discovered, pause and surface an ARCHITECTURE_CONCERN rather than inventing undocumented workarounds.
5. Implement all software strictly according to the acceptance criteria and test plan.
"#.to_string();

        let prompt_header = format!(
            "# Coalition Builder Milestone Task: Architecture v{}\n\nProject: {}\nProject ID: {}\nGit HEAD: {}\nContract Fingerprint: `{}`\n\n{}\n## Frozen Contract Specifications\n\n",
            architecture_version,
            project_name,
            project_id,
            git_head_commit,
            contract_fingerprint,
            builder_rules
        );

        let mut prompt = prompt_header;
        let mut artifacts = Vec::new();
        let mut is_truncated = false;

        for entry in manifest_artifacts {
            let content = load_content(&entry.relative_path)?;

            let title = entry
                .relative_path
                .split('/')
                .next_back()
                .unwrap_or(&entry.relative_path)
                .trim_end_matches(".md")
                .trim_end_matches(".yaml")
                .replace('-', " ");

            let heading = format!(
                "### {}\nPath: `{}`\nSHA-256: `{}`\n\n```markdown\n",
                title, entry.relative_path, entry.fingerprint
            );
            let footer = "\n```\n\n";
            let overhead = heading.len() + footer.len();
            let remaining_budget = BUILDER_PACKET_MAX_BYTES.saturating_sub(prompt.len());

            if remaining_budget <= overhead + 80 {
                // Not enough room for substantive content
                is_truncated = true;
                let notice = format!(
                    "### {}\nPath: `{}`\nSHA-256: `{}`\n\n[TRUNCATED: Excluded due to Builder Packet budget limit]\n\n",
                    title, entry.relative_path, entry.fingerprint
                );
                if prompt.len() + notice.len() <= BUILDER_PACKET_MAX_BYTES {
                    prompt.push_str(&notice);
                }
                artifacts.push(BuilderPacketArtifact {
                    path: entry.relative_path.clone(),
                    title,
                    content: "[TRUNCATED: Excluded due to Builder Packet budget limit]".to_string(),
                    fingerprint: entry.fingerprint.clone(),
                });
            } else {
                let available_content_bytes = remaining_budget - overhead;
                if content.len() <= available_content_bytes {
                    prompt.push_str(&heading);
                    prompt.push_str(&content);
                    prompt.push_str(footer);
                    artifacts.push(BuilderPacketArtifact {
                        path: entry.relative_path.clone(),
                        title,
                        content,
                        fingerprint: entry.fingerprint.clone(),
                    });
                } else {
                    is_truncated = true;
                    let notice_suffix =
                        "\n\n[TRUNCATED: Document exceeds Builder Packet budget limit]";
                    let text_budget = available_content_bytes.saturating_sub(notice_suffix.len());
                    let safe_cut = floor_char_boundary(&content, text_budget);
                    let effective_content = format!("{}{}", &content[..safe_cut], notice_suffix);

                    prompt.push_str(&heading);
                    prompt.push_str(&effective_content);
                    prompt.push_str(footer);
                    artifacts.push(BuilderPacketArtifact {
                        path: entry.relative_path.clone(),
                        title,
                        content: effective_content,
                        fingerprint: entry.fingerprint.clone(),
                    });
                }
            }
        }

        let prompt_bytes = prompt.len();
        Ok((prompt, builder_rules, artifacts, is_truncated, prompt_bytes))
    }

    /// Assembles the bounded Builder packet from the staged frozen contract files.
    fn build_builder_packet(
        metadata: BuilderPacketMetadata,
        staged_contract_dir: &Path,
        manifest_artifacts: &[ContractArtifactEntry],
    ) -> Result<BuilderPacket, FreezeError> {
        let (prompt, builder_rules, artifacts, is_truncated, _prompt_bytes) =
            Self::build_builder_packet_prompt_and_artifacts(
                &metadata.architecture_version,
                &metadata.project_name,
                &metadata.project_id,
                &metadata.git_head_commit,
                &metadata.contract_fingerprint,
                manifest_artifacts,
                |rel| {
                    let file_path = staged_contract_dir.join(rel);
                    fs::read_to_string(&file_path).map_err(|e| {
                        FreezeError::Io(format!(
                            "Failed to read staged contract file {:?}: {}",
                            file_path, e
                        ))
                    })
                },
            )?;

        let summary = format!(
            "Architecture v{} implementation contract containing {} frozen specifications",
            metadata.architecture_version,
            artifacts.len()
        );

        Ok(BuilderPacket {
            metadata,
            summary,
            builder_rules,
            artifacts,
            is_truncated,
            prompt,
        })
    }

    /// Executes the crash-safe freeze transaction upon explicit human confirmation of a Rust-owned preview ID.
    pub fn confirm_freeze<P: AsRef<Path>>(
        repo_root: P,
        project_id: &str,
        preview_id: &str,
        git: &GitAdapter,
        conn: &mut Connection,
    ) -> Result<FreezeResult, FreezeError> {
        let root = repo_root.as_ref();
        let coalition_dir = ArtifactManager::resolve_coalition_dir(root)?;
        let project_yaml_path = coalition_dir.join("project.yaml");

        // 1. Reload and validate the authoritative Rust-owned preview from database
        let preview_row: Result<(String, String, i64, String, String), rusqlite::Error> = conn
            .query_row(
                "SELECT preview_id, project_id, readiness_policy_version, preview_json, status
                 FROM freeze_previews
                 WHERE preview_id = ?1",
                params![preview_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            );

        let (_p_id, p_proj_id, _p_policy_ver, preview_json, status) = match preview_row {
            Ok(row) => row,
            Err(_) => {
                return Err(FreezeError::StaleFreezePreview(format!(
                    "Freeze preview '{}' not found. Please refresh preview and confirm again.",
                    preview_id
                )));
            }
        };

        if status != "PENDING" {
            return Err(FreezeError::StaleFreezePreview(format!(
                "Freeze preview '{}' is in status '{}' (expected PENDING). Please refresh preview.",
                preview_id, status
            )));
        }

        if p_proj_id != project_id {
            return Err(FreezeError::StaleFreezePreview(format!(
                "Freeze preview '{}' was prepared for project '{}', not '{}'",
                preview_id, p_proj_id, project_id
            )));
        }

        let preview: FreezePreview = serde_json::from_str(&preview_json).map_err(|e| {
            FreezeError::Artifact(format!("Failed to parse stored preview record: {}", e))
        })?;

        // 2. Verify project descriptor & workflow state
        if !project_yaml_path.exists() {
            return Err(FreezeError::Artifact(format!(
                "project.yaml missing at {:?}",
                project_yaml_path
            )));
        }

        let current_project_yaml = ArtifactManager::read_project_yaml(&project_yaml_path)?;
        if current_project_yaml.project_id != project_id {
            return Err(FreezeError::Artifact(format!(
                "Project ID mismatch: requested '{}', found '{}'",
                project_id, current_project_yaml.project_id
            )));
        }

        if current_project_yaml.architecture_state != ArchitectureState::Draft {
            return Err(FreezeError::IllegalWorkflowState {
                current: current_project_yaml.architecture_state.to_string(),
            });
        }

        let current_wf_state = workflow::get_workflow_state(conn, project_id)?;
        if current_wf_state.state != WorkflowState::ReadyToFreeze {
            return Err(FreezeError::IllegalWorkflowState {
                current: current_wf_state.state.to_string(),
            });
        }

        // 3. Revalidate readiness against disk
        let current_readiness = ReadinessEvaluator::evaluate(root)?;
        if current_readiness.overall_readiness != OverallReadiness::ReadyToFreeze {
            return Err(FreezeError::ReadinessIncomplete(format!(
                "Current readiness is incomplete: {}/{} required artifacts ready",
                current_readiness.ready_required_count, current_readiness.total_required_count
            )));
        }

        // 4. Revalidate Git boundary against disk
        let current_git_boundary = Self::capture_git_boundary(git, root)?;
        if current_git_boundary.head_commit != preview.git_boundary.head_commit {
            return Err(FreezeError::StaleFreezePreview(format!(
                "Git HEAD commit changed from '{}' to '{}'",
                preview.git_boundary.head_commit, current_git_boundary.head_commit
            )));
        }
        if current_git_boundary.dirty_fingerprint != preview.git_boundary.dirty_fingerprint {
            return Err(FreezeError::StaleFreezePreview(
                "Git working tree status changed since preview".to_string(),
            ));
        }

        // 5. Revalidate all artifact baselines against disk
        let current_baselines = Self::compute_active_contract_baselines(root)?;
        if current_baselines != preview.artifact_baselines {
            return Err(FreezeError::StaleFreezePreview(
                "One or more active architecture contract files were modified or added since preview".to_string(),
            ));
        }

        #[cfg(test)]
        {
            let seam = INJECTED_FREEZE_SEAM.with(|c| c.get());
            if seam == InjectedFreezeSeam::PreStaging {
                return Err(FreezeError::Io("Injected failure: PreStaging".to_string()));
            }
        }

        // 6. Staging Phase
        let staging_id = Uuid::new_v4().to_string();
        let arch_versions_dir = coalition_dir.join("architecture-versions");
        if !arch_versions_dir.exists() {
            fs::create_dir_all(&arch_versions_dir).map_err(|e| {
                FreezeError::Io(format!("Failed to create architecture-versions dir: {}", e))
            })?;
        }

        // Clean any leftover stale staging directories; fail closed if cleanup fails
        ArtifactManager::clean_stale_freeze_staging(&coalition_dir)?;

        let staging_dir = arch_versions_dir.join(format!(".staging-v1.0-{}", staging_id));
        let staging_contract_dir = staging_dir.join("contract");

        fs::create_dir_all(&staging_contract_dir).map_err(|e| {
            FreezeError::Io(format!("Failed to create staging contract dir: {}", e))
        })?;

        let mut manifest_artifacts = Vec::new();
        for (rel, expected_hash) in &preview.artifact_baselines {
            let active_path = coalition_dir.join(rel);
            let content = fs::read_to_string(&active_path).map_err(|e| {
                let _ = fs::remove_dir_all(&staging_dir);
                FreezeError::Io(format!(
                    "Failed to read active file {:?}: {}",
                    active_path, e
                ))
            })?;

            let current_hash = Self::sha256_hex(content.as_bytes());
            if current_hash != *expected_hash {
                let _ = fs::remove_dir_all(&staging_dir);
                return Err(FreezeError::StaleFreezePreview(format!(
                    "Artifact '{}' changed while staging snapshot",
                    rel
                )));
            }

            let dest_path = staging_contract_dir.join(rel);
            if let Some(parent) = dest_path.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    let _ = fs::remove_dir_all(&staging_dir);
                    FreezeError::Io(format!("Failed to create staging parent dir: {}", e))
                })?;
            }

            // Write staged contract file through explicit file handle and sync to disk
            {
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&dest_path)
                    .map_err(|e| {
                        let _ = fs::remove_dir_all(&staging_dir);
                        FreezeError::Io(format!(
                            "Failed to open staged artifact {:?}: {}",
                            dest_path, e
                        ))
                    })?;
                file.write_all(content.as_bytes()).map_err(|e| {
                    let _ = fs::remove_dir_all(&staging_dir);
                    FreezeError::Io(format!(
                        "Failed to write staged artifact {:?}: {}",
                        dest_path, e
                    ))
                })?;
                file.sync_all().map_err(|e| {
                    let _ = fs::remove_dir_all(&staging_dir);
                    FreezeError::Io(format!(
                        "Failed to sync staged artifact {:?}: {}",
                        dest_path, e
                    ))
                })?;
            }

            manifest_artifacts.push(ContractArtifactEntry {
                relative_path: rel.clone(),
                fingerprint: current_hash,
                size_bytes: content.len(),
            });

            #[cfg(test)]
            {
                let seam = INJECTED_FREEZE_SEAM.with(|c| c.get());
                if seam == InjectedFreezeSeam::MidStaging {
                    let _ = fs::remove_dir_all(&staging_dir);
                    return Err(FreezeError::Io("Injected failure: MidStaging".to_string()));
                }
            }
        }

        // 7. Compute deterministic contract identity
        let mut contract_hasher = Sha256::new();
        contract_hasher.update(project_id.as_bytes());
        contract_hasher.update(INITIAL_ARCHITECTURE_VERSION.as_bytes());
        contract_hasher.update(current_git_boundary.head_commit.as_bytes());
        contract_hasher.update(current_git_boundary.dirty_fingerprint.as_bytes());
        for entry in &manifest_artifacts {
            contract_hasher.update(entry.relative_path.as_bytes());
            contract_hasher.update(entry.fingerprint.as_bytes());
            contract_hasher.update(entry.size_bytes.to_le_bytes());
        }
        let contract_fingerprint = format!("{:x}", contract_hasher.finalize());

        // 8. Build, serialize, and persist finalized Builder Packet
        let freeze_timestamp = chrono::Utc::now().to_rfc3339();
        let epoch_id = Uuid::new_v4().to_string();
        let packet_id = Uuid::new_v4().to_string();
        let packet_metadata = BuilderPacketMetadata {
            schema_version: FREEZE_SCHEMA_VERSION,
            packet_id,
            project_id: project_id.to_string(),
            project_name: current_project_yaml.name.clone(),
            architecture_version: INITIAL_ARCHITECTURE_VERSION.to_string(),
            builder_epoch_id: epoch_id.clone(),
            created_at: freeze_timestamp.clone(),
            contract_fingerprint: contract_fingerprint.clone(),
            git_head_commit: current_git_boundary.head_commit.clone(),
            git_branch: current_git_boundary.branch.clone(),
        };

        let builder_packet = Self::build_builder_packet(
            packet_metadata,
            &staging_contract_dir,
            &manifest_artifacts,
        )?;

        let packet_bytes = serde_json::to_vec_pretty(&builder_packet).map_err(|e| {
            let _ = fs::remove_dir_all(&staging_dir);
            FreezeError::Artifact(format!("Failed to serialize Builder packet: {}", e))
        })?;

        let packet_fingerprint = Self::sha256_hex(&packet_bytes);
        let staged_packet_path = staging_dir.join("builder-packet.json");
        {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&staged_packet_path)
                .map_err(|e| {
                    let _ = fs::remove_dir_all(&staging_dir);
                    FreezeError::Io(format!("Failed to create staged builder packet: {}", e))
                })?;
            file.write_all(&packet_bytes).map_err(|e| {
                let _ = fs::remove_dir_all(&staging_dir);
                FreezeError::Io(format!("Failed to write staged builder packet: {}", e))
            })?;
            file.sync_all().map_err(|e| {
                let _ = fs::remove_dir_all(&staging_dir);
                FreezeError::Io(format!("Failed to sync staged builder packet: {}", e))
            })?;
        }

        // 9. Build, serialize, and persist finalized Contract Manifest
        let applicability = current_project_yaml
            .readiness
            .as_ref()
            .map(|r| r.applicability.clone())
            .unwrap_or_default();

        let manifest = ContractManifest {
            schema_version: FREEZE_SCHEMA_VERSION,
            manifest_version: MANIFEST_VERSION,
            project_id: project_id.to_string(),
            architecture_version: INITIAL_ARCHITECTURE_VERSION.to_string(),
            frozen_at: freeze_timestamp.clone(),
            frozen_by: "HUMAN".to_string(),
            builder_epoch_id: epoch_id.clone(),
            readiness_policy_version: preview.readiness_policy_version,
            applicability,
            unresolved_open_questions_count: preview.unresolved_open_questions_count,
            git_boundary: current_git_boundary.clone(),
            contract_fingerprint: contract_fingerprint.clone(),
            artifacts: manifest_artifacts,
            builder_packet_fingerprint: packet_fingerprint,
        };

        let manifest_yaml_str = serde_yaml::to_string(&manifest).map_err(|e| {
            let _ = fs::remove_dir_all(&staging_dir);
            FreezeError::Artifact(format!("Failed to serialize manifest: {}", e))
        })?;

        let manifest_fingerprint = Self::sha256_hex(manifest_yaml_str.as_bytes());
        let staged_manifest_path = staging_dir.join("contract-manifest.yaml");
        {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&staged_manifest_path)
                .map_err(|e| {
                    let _ = fs::remove_dir_all(&staging_dir);
                    FreezeError::Io(format!("Failed to create staged manifest: {}", e))
                })?;
            file.write_all(manifest_yaml_str.as_bytes()).map_err(|e| {
                let _ = fs::remove_dir_all(&staging_dir);
                FreezeError::Io(format!("Failed to write staged manifest: {}", e))
            })?;
            file.sync_all().map_err(|e| {
                let _ = fs::remove_dir_all(&staging_dir);
                FreezeError::Io(format!("Failed to sync staged manifest: {}", e))
            })?;
        }

        #[cfg(test)]
        {
            let seam = INJECTED_FREEZE_SEAM.with(|c| c.get());
            if seam == InjectedFreezeSeam::PostStagingPreFinalize {
                let _ = fs::remove_dir_all(&staging_dir);
                return Err(FreezeError::Io(
                    "Injected failure: PostStagingPreFinalize".to_string(),
                ));
            }
        }

        // 10. Directory Promotion: promote staging dir to target v1.0 using filesystem rename
        // Note: directory rename across the same volume provides filesystem directory relocation.
        let target_v1_dir = arch_versions_dir.join(format!("v{}", INITIAL_ARCHITECTURE_VERSION));
        if target_v1_dir.exists() {
            let _ = fs::remove_dir_all(&staging_dir);
            return Err(FreezeError::VersionAlreadyExists(
                INITIAL_ARCHITECTURE_VERSION.to_string(),
            ));
        }

        if let Err(e) = fs::rename(&staging_dir, &target_v1_dir) {
            let _ = fs::remove_dir_all(&staging_dir);
            return Err(FreezeError::Io(format!(
                "Failed to finalize snapshot directory to {:?}: {}",
                target_v1_dir, e
            )));
        }

        // Durably synchronize directory metadata
        durable_directory_sync(&arch_versions_dir).map_err(|e| {
            let _ = fs::remove_dir_all(&target_v1_dir);
            FreezeError::Io(format!("Failed to sync architecture-versions dir: {}", e))
        })?;
        durable_directory_sync(&target_v1_dir).map_err(|e| {
            let _ = fs::remove_dir_all(&target_v1_dir);
            FreezeError::Io(format!("Failed to sync snapshot dir: {}", e))
        })?;

        #[cfg(test)]
        {
            let seam = INJECTED_FREEZE_SEAM.with(|c| c.get());
            if seam == InjectedFreezeSeam::PostFinalizePreCommit {
                return Err(FreezeError::Io(
                    "Injected failure: PostFinalizePreCommit".to_string(),
                ));
            }
        }

        // Final stale-preview revalidation immediately before the durable commit point
        // 1. Re-evaluate Git boundary
        let commit_git_boundary = Self::capture_git_boundary(git, root)?;
        if commit_git_boundary.head_commit != preview.git_boundary.head_commit
            || commit_git_boundary.dirty_fingerprint != preview.git_boundary.dirty_fingerprint
        {
            let _ = fs::remove_dir_all(&target_v1_dir);
            let _ = durable_directory_sync(&arch_versions_dir);
            return Err(FreezeError::StaleFreezePreview(
                "Git repository state changed right before commit point (HEAD commit or dirty state modified)".to_string(),
            ));
        }

        // 2. Re-evaluate artifact baselines
        for (rel, expected_hash) in &preview.artifact_baselines {
            let active_path = coalition_dir.join(rel);
            let content = fs::read_to_string(&active_path).map_err(|e| {
                let _ = fs::remove_dir_all(&target_v1_dir);
                let _ = durable_directory_sync(&arch_versions_dir);
                FreezeError::Io(format!(
                    "Failed to read active file {:?} before commit: {}",
                    active_path, e
                ))
            })?;
            let current_hash = Self::sha256_hex(content.as_bytes());
            if current_hash != *expected_hash {
                let _ = fs::remove_dir_all(&target_v1_dir);
                let _ = durable_directory_sync(&arch_versions_dir);
                return Err(FreezeError::StaleFreezePreview(format!(
                    "Artifact '{}' modified right before commit point",
                    rel
                )));
            }
        }

        // 3. Re-evaluate readiness
        let commit_readiness = ReadinessEvaluator::evaluate(root)?;
        if commit_readiness.overall_readiness != OverallReadiness::ReadyToFreeze
            || commit_readiness.ready_required_count != preview.ready_required_count
            || commit_readiness.unresolved_open_questions_count
                != preview.unresolved_open_questions_count
        {
            let _ = fs::remove_dir_all(&target_v1_dir);
            let _ = durable_directory_sync(&arch_versions_dir);
            return Err(FreezeError::StaleFreezePreview(
                "Readiness state changed right before commit point".to_string(),
            ));
        }

        // 11. DURABLE COMMIT POINT: Atomically update project.yaml
        let mut activated_yaml = current_project_yaml.clone();
        activated_yaml.architecture_state = ArchitectureState::Frozen;
        activated_yaml.current_architecture_version =
            Some(INITIAL_ARCHITECTURE_VERSION.to_string());
        activated_yaml.active_manifest_fingerprint = Some(manifest_fingerprint.clone());

        if let Err(e) =
            ArtifactManager::write_project_yaml_atomic(&project_yaml_path, &activated_yaml)
        {
            let _ = fs::remove_dir_all(&target_v1_dir);
            return Err(FreezeError::Artifact(format!(
                "Failed to commit project.yaml during freeze: {}",
                e
            )));
        }

        // --- FROM THIS EXACT POINT ONWARD, ARCHITECTURE V1.0 IS PERMANENTLY ACTIVE ---

        #[cfg(test)]
        {
            let seam = INJECTED_FREEZE_SEAM.with(|c| c.get());
            if seam == InjectedFreezeSeam::PostCommitPreDb {
                return Err(FreezeError::Io(
                    "Injected failure: PostCommitPreDb".to_string(),
                ));
            }
        }

        // 12. Operational SQLite Updates
        let tx_res = (|| -> Result<(), FreezeError> {
            let tx = conn.transaction()?;

            // A. Invalidate single-use preview
            tx.execute(
                "UPDATE freeze_previews SET status = 'CONSUMED' WHERE preview_id = ?1",
                params![preview_id],
            )?;

            // B. Authoritative workflow transition
            workflow::apply_workflow_action_tx(&tx, project_id, WorkflowAction::Freeze, "HUMAN")?;

            #[cfg(test)]
            {
                let seam = INJECTED_FREEZE_SEAM.with(|c| c.get());
                if seam == InjectedFreezeSeam::MidDbTransaction {
                    return Err(FreezeError::Database(
                        "Injected failure: MidDbTransaction".to_string(),
                    ));
                }
            }

            // C. Insert minimal PENDING builder epoch
            tx.execute(
                "INSERT INTO builder_epochs (epoch_id, project_id, architecture_version, git_commit, git_branch, created_at, status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    epoch_id,
                    project_id,
                    INITIAL_ARCHITECTURE_VERSION,
                    current_git_boundary.head_commit,
                    current_git_boundary.branch,
                    freeze_timestamp,
                    "PENDING",
                ],
            )?;

            // D. Insert frozen boundary operational index
            let boundary_id = Uuid::new_v4().to_string();
            let snapshot_path_rel = format!(
                ".coalition/architecture-versions/v{}/",
                INITIAL_ARCHITECTURE_VERSION
            );
            tx.execute(
                "INSERT INTO frozen_boundaries (
                    boundary_id, project_id, architecture_version, git_commit, git_branch,
                    is_clean, staged_count, unstaged_count, untracked_count, dirty_fingerprint,
                    snapshot_path, manifest_fingerprint, frozen_at, frozen_by
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    boundary_id,
                    project_id,
                    INITIAL_ARCHITECTURE_VERSION,
                    current_git_boundary.head_commit,
                    current_git_boundary.branch,
                    if current_git_boundary.is_clean { 1 } else { 0 },
                    current_git_boundary.staged_count as i64,
                    current_git_boundary.unstaged_count as i64,
                    current_git_boundary.untracked_count as i64,
                    current_git_boundary.dirty_fingerprint,
                    snapshot_path_rel,
                    manifest_fingerprint,
                    freeze_timestamp,
                    "HUMAN",
                ],
            )?;

            // E. Activity event
            let short_commit = if current_git_boundary.head_commit.len() >= 8 {
                &current_git_boundary.head_commit[..8]
            } else {
                &current_git_boundary.head_commit
            };
            let event_meta = serde_json::json!({
                "architecture_version": INITIAL_ARCHITECTURE_VERSION,
                "epoch_id": epoch_id,
                "manifest_fingerprint": manifest_fingerprint,
                "contract_fingerprint": contract_fingerprint,
                "git_commit": current_git_boundary.head_commit,
                "git_branch": current_git_boundary.branch,
                "is_clean": current_git_boundary.is_clean,
                "frozen_at": freeze_timestamp,
            });
            ActivityManager::record_event(
                &tx,
                project_id,
                "ARCHITECTURE_FROZEN",
                "HUMAN",
                &format!(
                    "Frozen Architecture v{} at Git commit {} (clean: {})",
                    INITIAL_ARCHITECTURE_VERSION, short_commit, current_git_boundary.is_clean
                ),
                Some(&event_meta),
            )?;

            tx.commit()?;
            Ok(())
        })();

        tx_res?;

        Ok(FreezeResult {
            project_id: project_id.to_string(),
            architecture_version: INITIAL_ARCHITECTURE_VERSION.to_string(),
            epoch_id,
            frozen_at: freeze_timestamp,
            manifest_fingerprint,
            contract_fingerprint,
            git_boundary: current_git_boundary,
            snapshot_path: format!(
                ".coalition/architecture-versions/v{}/",
                INITIAL_ARCHITECTURE_VERSION
            ),
            builder_packet,
        })
    }

    /// Reconciles project truth and operational SQLite state on reopen / restart.
    /// Handles crash recovery:
    /// - Cleans stale `.staging-v*` directories; fails closed on error.
    /// - Reconciles any interrupted drift restoration journals.
    /// - If `project.yaml` is DRAFT but `v1.0/` exists, removes orphaned `v1.0/` to allow clean retry.
    /// - If `project.yaml` is FROZEN, ensures SQLite workflow state is `FROZEN`, and epoch and boundary exist.
    pub fn reconcile_freeze_state<P: AsRef<Path>>(
        repo_root: P,
        project_id: &str,
        conn: &mut Connection,
    ) -> Result<(), FreezeError> {
        let root = repo_root.as_ref();
        let coalition_dir = ArtifactManager::resolve_coalition_dir(root)?;
        let project_yaml_path = coalition_dir.join("project.yaml");

        if !project_yaml_path.exists() {
            return Ok(());
        }

        // 1. Clean stale staging directories (fails closed if deletion fails)
        ArtifactManager::clean_stale_freeze_staging(&coalition_dir)?;
        ArtifactManager::clean_stale_restore_staging(&coalition_dir)?;

        // 2. Reconcile any interrupted drift restoration journals
        Self::reconcile_drift_restoration(root, project_id, conn)?;

        let project_yaml = ArtifactManager::read_project_yaml(&project_yaml_path)?;
        let target_v1 = coalition_dir
            .join("architecture-versions")
            .join(format!("v{}", INITIAL_ARCHITECTURE_VERSION));

        // 3. Crash recovery: if project.yaml is DRAFT but target_v1 exists (crash at PostFinalizePreCommit)
        if project_yaml.architecture_state == ArchitectureState::Draft && target_v1.exists() {
            fs::remove_dir_all(&target_v1).map_err(|e| {
                FreezeError::FreezeRecoveryRequired(format!(
                    "Failed to clean orphaned uncommitted snapshot directory {:?}: {}",
                    target_v1, e
                ))
            })?;
        }

        // 4. If project.yaml is FROZEN, reconcile operational state
        if project_yaml.architecture_state == ArchitectureState::Frozen {
            let version = project_yaml
                .current_architecture_version
                .as_deref()
                .unwrap_or(INITIAL_ARCHITECTURE_VERSION);

            // Verify snapshot integrity first
            Self::verify_snapshot_integrity(
                root,
                version,
                project_yaml.active_manifest_fingerprint.as_deref(),
            )?;

            // Reconstruct operational records inside an explicit transaction
            let tx = conn.transaction()?;

            // Invalidate any single-use freeze preview
            tx.execute(
                "UPDATE freeze_previews SET status = 'CONSUMED' WHERE project_id = ?1 AND status = 'PENDING'",
                params![project_id],
            )?;

            // Reconcile SQLite workflow state
            let wf = workflow::get_workflow_state(&tx, project_id);
            match wf {
                Ok(rec) => {
                    if rec.state != WorkflowState::Frozen {
                        let now = chrono::Utc::now().to_rfc3339();
                        tx.execute(
                            "UPDATE workflow_state SET state = 'FROZEN', updated_at = ?1 WHERE project_id = ?2",
                            params![now, project_id],
                        )?;
                    }
                }
                Err(_) => {
                    let now = chrono::Utc::now().to_rfc3339();
                    tx.execute(
                        "INSERT INTO workflow_state (project_id, state, resume_state, revision, updated_at)
                         VALUES (?1, 'FROZEN', NULL, 1, ?2)",
                        params![project_id, now],
                    )?;
                }
            }

            // Ensure builder_epoch exists for this version
            let epoch_count: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM builder_epochs WHERE project_id = ?1 AND architecture_version = ?2",
                    params![project_id, version],
                    |r| r.get(0),
                )
                .unwrap_or(0);

            if epoch_count == 0 {
                let manifest = Self::read_contract_manifest(root, version)?;
                tx.execute(
                    "INSERT INTO builder_epochs (epoch_id, project_id, architecture_version, git_commit, git_branch, created_at, status)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        manifest.builder_epoch_id,
                        project_id,
                        version,
                        manifest.git_boundary.head_commit,
                        manifest.git_boundary.branch,
                        manifest.frozen_at,
                        "PENDING",
                    ],
                )?;
            }

            // Ensure frozen_boundary exists for this version independently
            let boundary_count: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM frozen_boundaries WHERE project_id = ?1 AND architecture_version = ?2",
                    params![project_id, version],
                    |r| r.get(0),
                )
                .unwrap_or(0);

            if boundary_count == 0 {
                let manifest = Self::read_contract_manifest(root, version)?;
                let boundary_id = Uuid::new_v4().to_string();
                let snapshot_path_rel = format!(".coalition/architecture-versions/v{}/", version);
                let manifest_fp = project_yaml
                    .active_manifest_fingerprint
                    .clone()
                    .unwrap_or_default();
                tx.execute(
                    "INSERT INTO frozen_boundaries (
                        boundary_id, project_id, architecture_version, git_commit, git_branch,
                        is_clean, staged_count, unstaged_count, untracked_count, dirty_fingerprint,
                        snapshot_path, manifest_fingerprint, frozen_at, frozen_by
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                    params![
                        boundary_id,
                        project_id,
                        version,
                        manifest.git_boundary.head_commit,
                        manifest.git_boundary.branch,
                        if manifest.git_boundary.is_clean { 1 } else { 0 },
                        manifest.git_boundary.staged_count as i64,
                        manifest.git_boundary.unstaged_count as i64,
                        manifest.git_boundary.untracked_count as i64,
                        manifest.git_boundary.dirty_fingerprint,
                        snapshot_path_rel,
                        manifest_fp,
                        manifest.frozen_at,
                        manifest.frozen_by,
                    ],
                )?;
            }

            // Ensure activity event exists for this freeze
            let event_count: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM activity_events WHERE project_id = ?1 AND event_type = 'ARCHITECTURE_FROZEN'",
                    params![project_id],
                    |r| r.get(0),
                )
                .unwrap_or(0);

            if event_count == 0 {
                let manifest = Self::read_contract_manifest(root, version)?;
                let short_commit = if manifest.git_boundary.head_commit.len() >= 8 {
                    &manifest.git_boundary.head_commit[..8]
                } else {
                    &manifest.git_boundary.head_commit
                };
                let event_meta = serde_json::json!({
                    "architecture_version": version,
                    "epoch_id": manifest.builder_epoch_id,
                    "manifest_fingerprint": project_yaml.active_manifest_fingerprint,
                    "contract_fingerprint": manifest.contract_fingerprint,
                    "git_commit": manifest.git_boundary.head_commit,
                    "git_branch": manifest.git_boundary.branch,
                    "is_clean": manifest.git_boundary.is_clean,
                    "frozen_at": manifest.frozen_at,
                });
                ActivityManager::record_event(
                    &tx,
                    project_id,
                    "ARCHITECTURE_FROZEN",
                    "HUMAN",
                    &format!(
                        "Frozen Architecture v{} at Git commit {} (clean: {})",
                        version, short_commit, manifest.git_boundary.is_clean
                    ),
                    Some(&event_meta),
                )?;
            }

            tx.commit().map_err(|e| {
                FreezeError::FreezeRecoveryRequired(format!(
                    "Failed to commit operational reconciliation transaction: {}",
                    e
                ))
            })?;
        }

        Ok(())
    }

    /// Reads and parses the contract manifest for a frozen version.
    pub fn read_contract_manifest<P: AsRef<Path>>(
        repo_root: P,
        version: &str,
    ) -> Result<ContractManifest, FreezeError> {
        Self::validate_architecture_version(version)?;
        let root = repo_root.as_ref();
        let coalition_dir = ArtifactManager::resolve_coalition_dir(root)?;
        let arch_versions_dir = coalition_dir.join("architecture-versions");
        let version_dir = arch_versions_dir.join(format!("v{}", version));

        if let (Ok(canon_root), Ok(canon_ver)) =
            (arch_versions_dir.canonicalize(), version_dir.canonicalize())
        {
            if !canon_ver.starts_with(&canon_root) {
                return Err(FreezeError::InvalidArchitectureVersion(format!(
                    "Architecture version '{}' directory escapes architecture-versions root",
                    version
                )));
            }
        }

        let manifest_path = version_dir.join("contract-manifest.yaml");

        if !manifest_path.exists() {
            return Err(FreezeError::FrozenSnapshotCorrupt {
                version: version.to_string(),
                reason: format!("contract-manifest.yaml missing at {:?}", manifest_path),
            });
        }

        let content =
            fs::read_to_string(&manifest_path).map_err(|e| FreezeError::FrozenSnapshotCorrupt {
                version: version.to_string(),
                reason: format!("Failed to read contract-manifest.yaml: {}", e),
            })?;

        serde_yaml::from_str(&content).map_err(|e| FreezeError::FrozenSnapshotCorrupt {
            version: version.to_string(),
            reason: format!("Failed to parse contract-manifest.yaml: {}", e),
        })
    }

    /// Verifies that the frozen snapshot directory has not been tampered with or corrupted.
    /// Strictly verifies:
    /// - Manifest fingerprint matches expected fingerprint if provided.
    /// - Every manifested artifact exists in `contract/` and matches its SHA-256 fingerprint.
    /// - No unexpected or unmanifested files exist inside `contract/`.
    /// - `builder-packet.json` exists and matches `manifest.builder_packet_fingerprint`.
    pub fn verify_snapshot_integrity<P: AsRef<Path>>(
        repo_root: P,
        version: &str,
        expected_manifest_fingerprint: Option<&str>,
    ) -> Result<ContractManifest, FreezeError> {
        Self::validate_architecture_version(version)?;

        fn check_no_symlinks(path: &Path, version: &str) -> Result<(), FreezeError> {
            let meta =
                fs::symlink_metadata(path).map_err(|e| FreezeError::FrozenSnapshotCorrupt {
                    version: version.to_string(),
                    reason: format!("Failed to read metadata for {:?}: {}", path, e),
                })?;
            if meta.file_type().is_symlink() {
                return Err(FreezeError::FrozenSnapshotCorrupt {
                    version: version.to_string(),
                    reason: format!("Snapshot path {:?} is a symlink", path),
                });
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
                if (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
                    return Err(FreezeError::FrozenSnapshotCorrupt {
                        version: version.to_string(),
                        reason: format!("Snapshot path {:?} is a reparse point", path),
                    });
                }
            }
            Ok(())
        }

        let root = repo_root.as_ref();
        let coalition_dir = ArtifactManager::resolve_coalition_dir(root)?;
        let arch_versions_dir = coalition_dir.join("architecture-versions");
        let version_dir = arch_versions_dir.join(format!("v{}", version));

        if !version_dir.exists() {
            return Err(FreezeError::FrozenSnapshotCorrupt {
                version: version.to_string(),
                reason: format!("Snapshot directory {:?} missing", version_dir),
            });
        }

        check_no_symlinks(&version_dir, version)?;

        if let (Ok(canon_root), Ok(canon_ver)) =
            (arch_versions_dir.canonicalize(), version_dir.canonicalize())
        {
            if !canon_ver.starts_with(&canon_root) {
                return Err(FreezeError::FrozenSnapshotCorrupt {
                    version: version.to_string(),
                    reason: format!(
                        "Snapshot directory {:?} escapes architecture-versions root",
                        version_dir
                    ),
                });
            }
        }

        let manifest_path = version_dir.join("contract-manifest.yaml");
        if !manifest_path.exists() {
            return Err(FreezeError::FrozenSnapshotCorrupt {
                version: version.to_string(),
                reason: "contract-manifest.yaml missing in snapshot".to_string(),
            });
        }
        check_no_symlinks(&manifest_path, version)?;

        let manifest_content =
            fs::read_to_string(&manifest_path).map_err(|e| FreezeError::FrozenSnapshotCorrupt {
                version: version.to_string(),
                reason: format!("Failed to read contract-manifest.yaml: {}", e),
            })?;

        // 1. Verify manifest fingerprint against project.yaml active_manifest_fingerprint if provided
        if let Some(expected_fp) = expected_manifest_fingerprint {
            let actual_fp = Self::sha256_hex(manifest_content.as_bytes());
            if actual_fp != expected_fp {
                return Err(FreezeError::FrozenSnapshotCorrupt {
                    version: version.to_string(),
                    reason: format!(
                        "contract-manifest.yaml hash '{}' does not match active_manifest_fingerprint '{}'",
                        actual_fp, expected_fp
                    ),
                });
            }
        }

        let manifest: ContractManifest = serde_yaml::from_str(&manifest_content).map_err(|e| {
            FreezeError::FrozenSnapshotCorrupt {
                version: version.to_string(),
                reason: format!("Failed to parse contract-manifest.yaml: {}", e),
            }
        })?;

        let contract_dir = version_dir.join("contract");
        if !contract_dir.exists() {
            return Err(FreezeError::FrozenSnapshotCorrupt {
                version: version.to_string(),
                reason: "contract/ subfolder missing in snapshot".to_string(),
            });
        }
        check_no_symlinks(&contract_dir, version)?;

        // 2. Verify each frozen contract file against its manifest hash
        for entry in &manifest.artifacts {
            let file_path = contract_dir.join(&entry.relative_path);
            if !file_path.exists() {
                return Err(FreezeError::FrozenSnapshotCorrupt {
                    version: version.to_string(),
                    reason: format!("Frozen file {:?} missing from snapshot contract", file_path),
                });
            }
            check_no_symlinks(&file_path, version)?;

            let bytes = fs::read(&file_path).map_err(|e| FreezeError::FrozenSnapshotCorrupt {
                version: version.to_string(),
                reason: format!("Failed to read snapshot file {:?}: {}", file_path, e),
            })?;

            let actual_hash = Self::sha256_hex(&bytes);
            if actual_hash != entry.fingerprint {
                return Err(FreezeError::FrozenSnapshotCorrupt {
                    version: version.to_string(),
                    reason: format!(
                        "Snapshot file {:?} hash mismatch (expected {}, got {})",
                        entry.relative_path, entry.fingerprint, actual_hash
                    ),
                });
            }
        }

        // 3. Strict check: detect unexpected unmanifested files inside contract/ directory
        fn scan_dir_files(
            dir: &Path,
            base: &Path,
            version: &str,
            files: &mut Vec<String>,
        ) -> Result<(), FreezeError> {
            if dir.is_dir() {
                let entries =
                    fs::read_dir(dir).map_err(|e| FreezeError::FrozenSnapshotCorrupt {
                        version: version.to_string(),
                        reason: format!("Failed to read snapshot directory {:?}: {}", dir, e),
                    })?;
                for entry in entries {
                    let entry = entry.map_err(|e| FreezeError::FrozenSnapshotCorrupt {
                        version: version.to_string(),
                        reason: format!("Failed to read snapshot directory entry: {}", e),
                    })?;
                    let path = entry.path();
                    check_no_symlinks(&path, version)?;
                    if path.is_dir() {
                        scan_dir_files(&path, base, version, files)?;
                    } else if path.is_file() {
                        if let Ok(rel) = path.strip_prefix(base) {
                            let rel_str = rel.to_string_lossy().replace('\\', "/");
                            files.push(rel_str);
                        }
                    }
                }
            }
            Ok(())
        }

        let mut actual_files = Vec::new();
        scan_dir_files(&contract_dir, &contract_dir, version, &mut actual_files)?;

        for actual in actual_files {
            if !manifest.artifacts.iter().any(|a| a.relative_path == actual) {
                return Err(FreezeError::FrozenSnapshotCorrupt {
                    version: version.to_string(),
                    reason: format!(
                        "Unexpected unmanifested file found in snapshot contract: '{}'",
                        actual
                    ),
                });
            }
        }

        // 4. Verify builder-packet.json against manifest hash
        let packet_path = version_dir.join("builder-packet.json");
        if !packet_path.exists() {
            return Err(FreezeError::FrozenSnapshotCorrupt {
                version: version.to_string(),
                reason: "builder-packet.json missing in snapshot".to_string(),
            });
        }
        check_no_symlinks(&packet_path, version)?;

        let packet_bytes =
            fs::read(&packet_path).map_err(|e| FreezeError::FrozenSnapshotCorrupt {
                version: version.to_string(),
                reason: format!("Failed to read builder-packet.json: {}", e),
            })?;

        let actual_packet_hash = Self::sha256_hex(&packet_bytes);
        if actual_packet_hash != manifest.builder_packet_fingerprint {
            return Err(FreezeError::FrozenSnapshotCorrupt {
                version: version.to_string(),
                reason: format!(
                    "builder-packet.json hash mismatch (expected {}, got {})",
                    manifest.builder_packet_fingerprint, actual_packet_hash
                ),
            });
        }

        Ok(manifest)
    }

    /// Authoritatively validates an artifact path for drift operations:
    /// - Rejects parent directory traversal (..)
    /// - Rejects absolute paths and drive letters
    /// - Rejects paths outside the managed architecture artifact package
    /// - Rejects paths not present in the verified DriftReport
    pub fn validate_drift_artifact_path<P: AsRef<Path>>(
        repo_root: P,
        project_id: &str,
        artifact_path: &str,
    ) -> Result<DriftedArtifact, FreezeError> {
        let root = repo_root.as_ref();
        let normalized = artifact_path.replace('\\', "/");

        if normalized.starts_with('/')
            || normalized.contains("..")
            || Path::new(&normalized).is_absolute()
            || (normalized.len() >= 2 && normalized.as_bytes()[1] == b':')
        {
            return Err(FreezeError::Artifact(format!(
                "Invalid path traversal or absolute path: '{}'",
                artifact_path
            )));
        }

        if !ArtifactManager::is_valid_architecture_artifact_path(&normalized) {
            return Err(FreezeError::Artifact(format!(
                "Path '{}' is not an allowed architecture contract artifact",
                artifact_path
            )));
        }

        let report = Self::check_contract_drift(root, project_id)?;
        let drifted = report
            .drifted_artifacts
            .into_iter()
            .find(|a| a.path == normalized)
            .ok_or_else(|| {
                FreezeError::Artifact(format!(
                    "Path '{}' is not present in the active drift report for project '{}'",
                    artifact_path, project_id
                ))
            })?;

        Ok(drifted)
    }

    /// Checks the active architecture contract files against the frozen snapshot to detect drift.
    pub fn check_contract_drift<P: AsRef<Path>>(
        repo_root: P,
        _project_id: &str,
    ) -> Result<DriftReport, FreezeError> {
        let root = repo_root.as_ref();
        let coalition_dir = ArtifactManager::resolve_coalition_dir(root)?;
        let project_yaml_path = coalition_dir.join("project.yaml");

        if !project_yaml_path.exists() {
            return Ok(DriftReport {
                has_drift: false,
                is_frozen: false,
                architecture_version: None,
                drifted_artifacts: Vec::new(),
                checked_at: chrono::Utc::now().to_rfc3339(),
            });
        }

        let project_yaml = ArtifactManager::read_project_yaml(&project_yaml_path)?;
        if project_yaml.architecture_state != ArchitectureState::Frozen {
            return Ok(DriftReport {
                has_drift: false,
                is_frozen: false,
                architecture_version: None,
                drifted_artifacts: Vec::new(),
                checked_at: chrono::Utc::now().to_rfc3339(),
            });
        }

        let version = project_yaml
            .current_architecture_version
            .as_deref()
            .unwrap_or(INITIAL_ARCHITECTURE_VERSION);

        // 1. Verify snapshot integrity first! (Fails closed if snapshot is corrupted)
        let manifest = Self::verify_snapshot_integrity(
            root,
            version,
            project_yaml.active_manifest_fingerprint.as_deref(),
        )?;

        let mut drifted = Vec::new();
        let manifest_map: BTreeMap<String, String> = manifest
            .artifacts
            .iter()
            .map(|a| (a.relative_path.clone(), a.fingerprint.clone()))
            .collect();

        // 2. Check for MODIFIED or DELETED active files
        for (rel, frozen_hash) in &manifest_map {
            let active_path = coalition_dir.join(rel);
            if !active_path.exists() {
                drifted.push(DriftedArtifact {
                    path: rel.clone(),
                    drift_type: DriftType::Deleted,
                    frozen_fingerprint: Some(frozen_hash.clone()),
                    active_fingerprint: None,
                });
            } else {
                let active_hash =
                    ArtifactManager::compute_file_fingerprint(root, rel)?.unwrap_or_default();
                if active_hash != *frozen_hash {
                    drifted.push(DriftedArtifact {
                        path: rel.clone(),
                        drift_type: DriftType::Modified,
                        frozen_fingerprint: Some(frozen_hash.clone()),
                        active_fingerprint: Some(active_hash),
                    });
                }
            }
        }

        // 3. Check for ADDED active files (unexpected architecture files in design/impl/decisions)
        let active_files = Self::list_active_governed_artifacts(root)?;
        for rel in active_files {
            if !manifest_map.contains_key(&rel) {
                let active_hash =
                    ArtifactManager::compute_file_fingerprint(root, &rel)?.unwrap_or_default();
                drifted.push(DriftedArtifact {
                    path: rel,
                    drift_type: DriftType::Added,
                    frozen_fingerprint: None,
                    active_fingerprint: Some(active_hash),
                });
            }
        }

        let has_drift = !drifted.is_empty();
        Ok(DriftReport {
            has_drift,
            is_frozen: true,
            architecture_version: Some(version.to_string()),
            drifted_artifacts: drifted,
            checked_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    /// Inspects the diff between the frozen snapshot content and current active content.
    pub fn get_drift_diff<P: AsRef<Path>>(
        repo_root: P,
        project_id: &str,
        artifact_path: &str,
    ) -> Result<DriftDiff, FreezeError> {
        let root = repo_root.as_ref();
        let drifted_meta = Self::validate_drift_artifact_path(root, project_id, artifact_path)?;

        let coalition_dir = ArtifactManager::resolve_coalition_dir(root)?;
        let project_yaml_path = coalition_dir.join("project.yaml");
        let project_yaml = ArtifactManager::read_project_yaml(&project_yaml_path)?;

        let version = project_yaml
            .current_architecture_version
            .as_deref()
            .unwrap_or(INITIAL_ARCHITECTURE_VERSION);

        Self::validate_architecture_version(version)?;

        let snapshot_file = coalition_dir
            .join("architecture-versions")
            .join(format!("v{}", version))
            .join("contract")
            .join(&drifted_meta.path);

        let frozen_content = if snapshot_file.exists() {
            Some(fs::read_to_string(&snapshot_file).map_err(|e| {
                FreezeError::Io(format!(
                    "Failed to read snapshot file {:?}: {}",
                    snapshot_file, e
                ))
            })?)
        } else {
            None
        };

        let active_path = coalition_dir.join(&drifted_meta.path);
        let active_content = if active_path.exists() {
            Some(fs::read_to_string(&active_path).map_err(|e| {
                FreezeError::Io(format!(
                    "Failed to read active file {:?}: {}",
                    active_path, e
                ))
            })?)
        } else {
            None
        };

        Ok(DriftDiff {
            path: drifted_meta.path,
            drift_type: drifted_meta.drift_type,
            frozen_content,
            active_content,
        })
    }

    /// Restores a single drifted artifact back to match the frozen snapshot.
    /// Non-destructive: if artifact is ADDED, it is quarantined rather than deleted.
    pub fn restore_drifted_artifact<P: AsRef<Path>>(
        repo_root: P,
        project_id: &str,
        artifact_path: &str,
        conn: &mut Connection,
    ) -> Result<DriftReport, FreezeError> {
        let root = repo_root.as_ref();
        Self::reconcile_drift_restoration(root, project_id, conn)?;
        let drifted = Self::validate_drift_artifact_path(root, project_id, artifact_path)?;

        let coalition_dir = ArtifactManager::resolve_coalition_dir(root)?;
        let project_yaml_path = coalition_dir.join("project.yaml");
        let project_yaml = ArtifactManager::read_project_yaml(&project_yaml_path)?;

        let version = project_yaml
            .current_architecture_version
            .as_deref()
            .unwrap_or(INITIAL_ARCHITECTURE_VERSION);

        Self::validate_architecture_version(version)?;

        // Verify snapshot integrity first
        Self::verify_snapshot_integrity(
            root,
            version,
            project_yaml.active_manifest_fingerprint.as_deref(),
        )?;

        let snapshot_file = coalition_dir
            .join("architecture-versions")
            .join(format!("v{}", version))
            .join("contract")
            .join(&drifted.path);

        if snapshot_file.exists() {
            let content = fs::read_to_string(&snapshot_file).map_err(|e| {
                FreezeError::Io(format!(
                    "Failed to read snapshot file {:?}: {}",
                    snapshot_file, e
                ))
            })?;
            ArtifactManager::write_artifact_atomic(root, &drifted.path, &content)?;
        } else {
            ArtifactManager::quarantine_added_artifact(root, &drifted.path)?;
        }

        Self::check_contract_drift(root, project_id)
    }

    /// Reconciles an interrupted drift restoration batch journal.
    pub fn reconcile_drift_restoration<P: AsRef<Path>>(
        repo_root: P,
        project_id: &str,
        conn: &mut Connection,
    ) -> Result<(), FreezeError> {
        let root = repo_root.as_ref();
        let coalition_dir = ArtifactManager::resolve_coalition_dir(root)?;
        let recovery_dir = coalition_dir.join("recovery");
        let disk_journal_path = recovery_dir.join("drift-restoration-journal.json");

        // Clean orphan restore staging directories
        ArtifactManager::clean_stale_restore_staging(&coalition_dir)?;

        // 1. Read disk journal if present (strict fail-closed on corrupt content)
        let disk_journal: Option<DriftRestorationJournal> = if disk_journal_path.exists() {
            let content = fs::read_to_string(&disk_journal_path).map_err(|e| {
                FreezeError::DriftRestorationRecoveryRequired(format!(
                    "Failed to read drift journal from disk {:?}: {}",
                    disk_journal_path, e
                ))
            })?;
            let parsed: DriftRestorationJournal = serde_json::from_str(&content).map_err(|e| {
                FreezeError::DriftRestorationRecoveryRequired(format!(
                    "Corrupt disk drift restoration journal {:?}: {}",
                    disk_journal_path, e
                ))
            })?;
            Some(parsed)
        } else {
            None
        };

        // 2. Read SQLite journal if present
        let db_row_opt: Option<(String, String, String, String, String, String)> = conn
            .query_row(
                "SELECT journal_id, architecture_version, phase, operations_json, created_at, updated_at
                 FROM drift_restoration_journals
                 WHERE project_id = ?1",
                params![project_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .optional()?;

        let db_journal: Option<DriftRestorationJournal> =
            if let Some((jid, ver, ph, ops, cat, uat)) = db_row_opt {
                let phase = match ph.as_str() {
                    "STAGED" => RestorationPhase::Staged,
                    "COMMITTING" => RestorationPhase::Committing,
                    "COMMITTED" => RestorationPhase::Committed,
                    other => {
                        return Err(FreezeError::DriftRestorationRecoveryRequired(format!(
                            "Unknown or unexpected phase '{}' in SQLite drift restoration journal",
                            other
                        )));
                    }
                };
                let operations: Vec<RestorationOp> = serde_json::from_str(&ops).map_err(|e| {
                    FreezeError::DriftRestorationRecoveryRequired(format!(
                        "Corrupt operations JSON in SQLite drift restoration journal: {}",
                        e
                    ))
                })?;
                Some(DriftRestorationJournal {
                    journal_id: jid,
                    project_id: project_id.to_string(),
                    architecture_version: ver,
                    phase,
                    operations,
                    created_at: cat,
                    updated_at: uat,
                })
            } else {
                None
            };

        // 3. Reconcile dual journals (disk and SQLite)
        let authoritative_journal: Option<DriftRestorationJournal> = match (
            disk_journal,
            db_journal,
        ) {
            (Some(disk_j), Some(db_j)) => {
                if disk_j.journal_id != db_j.journal_id
                    || disk_j.architecture_version != db_j.architecture_version
                    || disk_j.project_id != db_j.project_id
                    || disk_j.operations != db_j.operations
                {
                    return Err(FreezeError::DriftRestorationRecoveryRequired(format!(
                        "Conflicting disk and SQLite restoration journals for project '{}'",
                        project_id
                    )));
                }

                // Check lifecycle phase compatibility
                let effective_phase = match (disk_j.phase, db_j.phase) {
                    (RestorationPhase::Staged, RestorationPhase::Staged) => {
                        RestorationPhase::Staged
                    }
                    (RestorationPhase::Committing, RestorationPhase::Staged)
                    | (RestorationPhase::Staged, RestorationPhase::Committing)
                    | (RestorationPhase::Committing, RestorationPhase::Committing) => {
                        RestorationPhase::Committing
                    }
                    (RestorationPhase::Committing, RestorationPhase::Committed)
                    | (RestorationPhase::Committed, RestorationPhase::Committed) => {
                        RestorationPhase::Committed
                    }
                    (disk_p, db_p) => {
                        return Err(FreezeError::DriftRestorationRecoveryRequired(format!(
                            "Incompatible restoration journal phases: disk={:?}, db={:?}",
                            disk_p, db_p
                        )));
                    }
                };

                Some(DriftRestorationJournal {
                    phase: effective_phase,
                    ..disk_j
                })
            }
            (Some(disk_j), None) => {
                // SQLite record missing or deleted; disk journal is durable authority
                Some(disk_j)
            }
            (None, Some(db_j)) => {
                if db_j.phase == RestorationPhase::Committing {
                    // Check if contract is already clean
                    let report = Self::check_contract_drift(root, project_id)?;
                    if !report.has_drift {
                        // Disk journal was removed after clean restore, clean up SQLite
                        conn.execute(
                            "DELETE FROM drift_restoration_journals WHERE project_id = ?1",
                            params![project_id],
                        )?;
                        return Ok(());
                    } else {
                        return Err(FreezeError::DriftRestorationRecoveryRequired(
                            "SQLite indicates COMMITTING drift restoration but disk journal is missing while drift exists".to_string(),
                        ));
                    }
                } else {
                    // STAGED and disk journal missing: safely clean SQLite row
                    conn.execute(
                        "DELETE FROM drift_restoration_journals WHERE project_id = ?1",
                        params![project_id],
                    )?;
                    return Ok(());
                }
            }
            (None, None) => None,
        };

        if let Some(ref j) = authoritative_journal {
            Self::validate_restoration_journal(&coalition_dir, j)?;
        }

        if let Some(j) = authoritative_journal {
            match j.phase {
                RestorationPhase::Staged => {
                    // Safe rollback: mutations never began
                    for op in &j.operations {
                        if let Some(ref rel) = op.staged_file_rel {
                            let p = coalition_dir.join(rel);
                            if p.exists() {
                                fs::remove_file(&p).map_err(|e| {
                                    FreezeError::Io(format!(
                                        "Failed to clean staged restore file during rollback: {}",
                                        e
                                    ))
                                })?;
                            }
                        }
                    }
                    let staging_batch_dir =
                        recovery_dir.join(format!(".staging-restore-{}", j.journal_id));
                    if staging_batch_dir.exists() {
                        fs::remove_dir_all(&staging_batch_dir).map_err(|e| {
                            FreezeError::Io(format!(
                                "Failed to clean staging restore dir during rollback: {}",
                                e
                            ))
                        })?;
                    }
                    if disk_journal_path.exists() {
                        fs::remove_file(&disk_journal_path).map_err(|e| {
                            FreezeError::Io(format!(
                                "Failed to clean disk drift journal during rollback: {}",
                                e
                            ))
                        })?;
                    }
                    durable_directory_sync(&recovery_dir).map_err(|e| {
                        FreezeError::Io(format!(
                            "Failed to sync recovery dir during rollback: {}",
                            e
                        ))
                    })?;
                    conn.execute(
                        "DELETE FROM drift_restoration_journals WHERE project_id = ?1",
                        params![project_id],
                    )?;
                }
                RestorationPhase::Committing => {
                    // Mid-mutation crash: replay all operations deterministically from staged/snapshot files
                    for op in &j.operations {
                        match op.drift_type {
                            DriftType::Modified | DriftType::Deleted => {
                                let target_p = coalition_dir.join(&op.path);
                                let mut staged_applied = false;

                                if let Some(ref rel) = op.staged_file_rel {
                                    let staged_p = coalition_dir.join(rel);
                                    if staged_p.exists() {
                                        ArtifactManager::replace_file_atomically(
                                            &staged_p, &target_p,
                                        )?;
                                        staged_applied = true;
                                    }
                                }

                                if !staged_applied {
                                    let snapshot_p = coalition_dir
                                        .join("architecture-versions")
                                        .join(format!("v{}", j.architecture_version))
                                        .join("contract")
                                        .join(&op.path);

                                    if snapshot_p.exists() {
                                        let content = fs::read_to_string(&snapshot_p).map_err(|e| {
                                            FreezeError::Io(format!(
                                                "Failed to read snapshot file {:?} during reconcile: {}",
                                                snapshot_p, e
                                            ))
                                        })?;
                                        ArtifactManager::write_artifact_atomic(
                                            root, &op.path, &content,
                                        )?;
                                    } else {
                                        return Err(FreezeError::DriftRestorationRecoveryRequired(format!(
                                            "Snapshot file missing at {:?} during drift restoration reconcile",
                                            snapshot_p
                                        )));
                                    }
                                }
                            }
                            DriftType::Added => {
                                let active_p = coalition_dir.join(&op.path);
                                if active_p.exists() {
                                    if let Some(ref q_rel) = op.quarantine_dest_rel {
                                        let q_p = coalition_dir.join(q_rel);
                                        if let Some(parent) = q_p.parent() {
                                            fs::create_dir_all(parent).map_err(|e| {
                                                FreezeError::Io(format!(
                                                    "Failed to create quarantine parent: {}",
                                                    e
                                                ))
                                            })?;
                                        }
                                        if let Err(e) = fs::rename(&active_p, &q_p) {
                                            fs::copy(&active_p, &q_p).map_err(|ce| {
                                                FreezeError::Io(format!(
                                                    "Failed to quarantine added file: rename: {}, copy: {}",
                                                    e, ce
                                                ))
                                            })?;
                                            fs::remove_file(&active_p).map_err(|re| {
                                                FreezeError::Io(format!(
                                                    "Failed to remove quarantined file: {}",
                                                    re
                                                ))
                                            })?;
                                        }
                                        if let Some(parent) = q_p.parent() {
                                            durable_directory_sync(parent).map_err(|e| {
                                                FreezeError::Io(format!(
                                                    "Failed to sync quarantine dir: {}",
                                                    e
                                                ))
                                            })?;
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Verify clean contract drift
                    let report = Self::check_contract_drift(root, project_id)?;
                    if report.has_drift {
                        return Err(FreezeError::DriftRestorationRecoveryRequired(format!(
                            "Reconciliation of drift restoration failed: {} artifacts still drifted",
                            report.drifted_artifacts.len()
                        )));
                    }

                    // Clean up after successful verification
                    let staging_batch_dir =
                        recovery_dir.join(format!(".staging-restore-{}", j.journal_id));
                    if staging_batch_dir.exists() {
                        fs::remove_dir_all(&staging_batch_dir).map_err(|e| {
                            FreezeError::Io(format!("Failed to remove staging restore dir: {}", e))
                        })?;
                    }
                    if disk_journal_path.exists() {
                        fs::remove_file(&disk_journal_path).map_err(|e| {
                            FreezeError::Io(format!("Failed to remove disk drift journal: {}", e))
                        })?;
                    }
                    durable_directory_sync(&recovery_dir).map_err(|e| {
                        FreezeError::Io(format!("Failed to sync recovery dir: {}", e))
                    })?;
                    conn.execute(
                        "DELETE FROM drift_restoration_journals WHERE project_id = ?1",
                        params![project_id],
                    )?;
                }
                RestorationPhase::Committed => {
                    // Clean up any lingering files
                    let staging_batch_dir =
                        recovery_dir.join(format!(".staging-restore-{}", j.journal_id));
                    if staging_batch_dir.exists() {
                        fs::remove_dir_all(&staging_batch_dir).map_err(|e| {
                            FreezeError::Io(format!("Failed to remove staging restore dir: {}", e))
                        })?;
                    }
                    if disk_journal_path.exists() {
                        fs::remove_file(&disk_journal_path).map_err(|e| {
                            FreezeError::Io(format!("Failed to remove disk drift journal: {}", e))
                        })?;
                    }
                    durable_directory_sync(&recovery_dir).map_err(|e| {
                        FreezeError::Io(format!("Failed to sync recovery dir: {}", e))
                    })?;
                    conn.execute(
                        "DELETE FROM drift_restoration_journals WHERE project_id = ?1",
                        params![project_id],
                    )?;
                }
            }
        }

        Ok(())
    }

    /// Restores all drifted artifacts back to exact frozen baseline using a crash-safe batch restoration protocol.
    pub fn restore_all_drifted_artifacts<P: AsRef<Path>>(
        repo_root: P,
        project_id: &str,
        conn: &mut Connection,
    ) -> Result<DriftReport, FreezeError> {
        let root = repo_root.as_ref();
        Self::reconcile_drift_restoration(root, project_id, conn)?;
        let report = Self::check_contract_drift(root, project_id)?;

        if !report.has_drift {
            return Ok(report);
        }

        let coalition_dir = ArtifactManager::resolve_coalition_dir(root)?;
        ArtifactManager::clean_stale_restore_staging(&coalition_dir)?;

        let project_yaml_path = coalition_dir.join("project.yaml");
        let project_yaml = ArtifactManager::read_project_yaml(&project_yaml_path)?;
        let version = project_yaml
            .current_architecture_version
            .as_deref()
            .unwrap_or(INITIAL_ARCHITECTURE_VERSION);

        Self::validate_architecture_version(version)?;

        // 1. Verify snapshot integrity first
        Self::verify_snapshot_integrity(
            root,
            version,
            project_yaml.active_manifest_fingerprint.as_deref(),
        )?;

        let batch_id = Uuid::new_v4().to_string();
        let recovery_dir = coalition_dir.join("recovery");
        fs::create_dir_all(&recovery_dir)
            .map_err(|e| FreezeError::Io(format!("Failed to create recovery dir: {}", e)))?;
        let staging_batch_dir = recovery_dir.join(format!(".staging-restore-{}", batch_id));
        fs::create_dir_all(&staging_batch_dir)
            .map_err(|e| FreezeError::Io(format!("Failed to create staging restore dir: {}", e)))?;
        durable_directory_sync(&recovery_dir)
            .map_err(|e| FreezeError::Io(format!("Failed to sync recovery dir: {}", e)))?;

        // 2. Deterministically order operations
        let mut sorted_drift = report.drifted_artifacts.clone();
        sorted_drift.sort_by(|a, b| a.path.cmp(&b.path));

        // 3. Stage restoration files and pre-allocate relative quarantine paths
        let mut operations = Vec::new();
        for drifted in &sorted_drift {
            match drifted.drift_type {
                DriftType::Modified | DriftType::Deleted => {
                    let snapshot_file = coalition_dir
                        .join("architecture-versions")
                        .join(format!("v{}", version))
                        .join("contract")
                        .join(&drifted.path);

                    let content = fs::read_to_string(&snapshot_file).map_err(|e| {
                        let _ = fs::remove_dir_all(&staging_batch_dir);
                        FreezeError::Io(format!(
                            "Failed to read snapshot file {:?}: {}",
                            snapshot_file, e
                        ))
                    })?;

                    let staged_filename = format!("{}.staged", drifted.path.replace('/', "_"));
                    let staged_file = staging_batch_dir.join(&staged_filename);
                    {
                        let mut file = fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&staged_file)
                            .map_err(|e| {
                                let _ = fs::remove_dir_all(&staging_batch_dir);
                                FreezeError::Io(format!(
                                    "Failed to create staged restore file: {}",
                                    e
                                ))
                            })?;
                        file.write_all(content.as_bytes()).map_err(|e| {
                            let _ = fs::remove_dir_all(&staging_batch_dir);
                            FreezeError::Io(format!("Failed to write staged restore file: {}", e))
                        })?;
                        file.sync_all().map_err(|e| {
                            let _ = fs::remove_dir_all(&staging_batch_dir);
                            FreezeError::Io(format!("Failed to sync staged restore file: {}", e))
                        })?;
                    }

                    let staged_file_rel =
                        format!("recovery/.staging-restore-{}/{}", batch_id, staged_filename);

                    operations.push(RestorationOp {
                        path: drifted.path.clone(),
                        drift_type: drifted.drift_type,
                        staged_file_rel: Some(staged_file_rel),
                        quarantine_dest_rel: None,
                    });
                }
                DriftType::Added => {
                    let quarantine_dest_rel = format!(
                        "recovery/quarantine-{}-{}/{}",
                        chrono::Utc::now().format("%Y%m%d%H%M%S"),
                        batch_id,
                        drifted.path
                    );

                    operations.push(RestorationOp {
                        path: drifted.path.clone(),
                        drift_type: drifted.drift_type,
                        staged_file_rel: None,
                        quarantine_dest_rel: Some(quarantine_dest_rel),
                    });
                }
            }
        }

        durable_directory_sync(&staging_batch_dir)
            .map_err(|e| FreezeError::Io(format!("Failed to sync staging batch dir: {}", e)))?;

        // 4. Record journal durably on disk (STAGED phase)
        let now = chrono::Utc::now().to_rfc3339();
        let mut journal = DriftRestorationJournal {
            journal_id: batch_id.clone(),
            project_id: project_id.to_string(),
            architecture_version: version.to_string(),
            phase: RestorationPhase::Staged,
            operations: operations.clone(),
            created_at: now.clone(),
            updated_at: now.clone(),
        };

        let disk_journal_path = recovery_dir.join("drift-restoration-journal.json");
        {
            let journal_json = serde_json::to_string_pretty(&journal).map_err(|e| {
                let _ = fs::remove_dir_all(&staging_batch_dir);
                FreezeError::Artifact(format!("Failed to serialize drift journal: {}", e))
            })?;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&disk_journal_path)
                .map_err(|e| {
                    let _ = fs::remove_dir_all(&staging_batch_dir);
                    FreezeError::Io(format!("Failed to open disk drift journal: {}", e))
                })?;
            file.write_all(journal_json.as_bytes()).map_err(|e| {
                let _ = fs::remove_dir_all(&staging_batch_dir);
                FreezeError::Io(format!("Failed to write disk drift journal: {}", e))
            })?;
            file.sync_all().map_err(|e| {
                let _ = fs::remove_dir_all(&staging_batch_dir);
                FreezeError::Io(format!("Failed to sync disk drift journal: {}", e))
            })?;
        }
        durable_directory_sync(&recovery_dir)
            .map_err(|e| FreezeError::Io(format!("Failed to sync recovery dir: {}", e)))?;

        conn.execute(
            "INSERT INTO drift_restoration_journals (journal_id, project_id, architecture_version, phase, operations_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'STAGED', ?4, ?5, ?6)",
            params![
                journal.journal_id,
                project_id,
                version,
                serde_json::to_string(&operations).unwrap_or_default(),
                now,
                now,
            ],
        )?;

        // 5. Transition to COMMITTING phase on disk and database before first mutation
        let committing_time = chrono::Utc::now().to_rfc3339();
        journal.phase = RestorationPhase::Committing;
        journal.updated_at = committing_time.clone();

        {
            let journal_json = serde_json::to_string_pretty(&journal).map_err(|e| {
                FreezeError::Artifact(format!(
                    "Failed to serialize committing drift journal: {}",
                    e
                ))
            })?;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&disk_journal_path)
                .map_err(|e| {
                    FreezeError::Io(format!(
                        "Failed to open committing disk drift journal: {}",
                        e
                    ))
                })?;
            file.write_all(journal_json.as_bytes()).map_err(|e| {
                FreezeError::Io(format!(
                    "Failed to write committing disk drift journal: {}",
                    e
                ))
            })?;
            file.sync_all().map_err(|e| {
                FreezeError::Io(format!(
                    "Failed to sync committing disk drift journal: {}",
                    e
                ))
            })?;
        }
        durable_directory_sync(&recovery_dir).map_err(|e| {
            FreezeError::Io(format!("Failed to sync recovery dir for committing: {}", e))
        })?;

        conn.execute(
            "UPDATE drift_restoration_journals SET phase = 'COMMITTING', updated_at = ?1 WHERE journal_id = ?2",
            params![committing_time, journal.journal_id],
        )?;

        #[cfg(test)]
        {
            let seam = INJECTED_RESTORE_SEAM.with(|c| c.get());
            if seam == InjectedRestoreSeam::BeforeFirstRestoreMutation {
                return Err(FreezeError::Io(
                    "Injected restore seam: BeforeFirstRestoreMutation".to_string(),
                ));
            }
        }

        // 6. Execute mutations in deterministic order
        #[cfg(test)]
        let op_count = operations.len();
        for (idx, op) in operations.iter().enumerate() {
            #[cfg(not(test))]
            let _ = idx;
            match op.drift_type {
                DriftType::Modified | DriftType::Deleted => {
                    if let Some(ref staged_rel) = op.staged_file_rel {
                        let staged_p = coalition_dir.join(staged_rel);
                        let target_p = coalition_dir.join(&op.path);
                        if staged_p.exists() {
                            ArtifactManager::replace_file_atomically(&staged_p, &target_p)?;
                        } else {
                            let snapshot_p = coalition_dir
                                .join("architecture-versions")
                                .join(format!("v{}", version))
                                .join("contract")
                                .join(&op.path);
                            let content = fs::read_to_string(&snapshot_p).map_err(|e| {
                                FreezeError::Io(format!(
                                    "Failed to read snapshot file during restore: {}",
                                    e
                                ))
                            })?;
                            ArtifactManager::write_artifact_atomic(root, &op.path, &content)?;
                        }
                    }
                }
                DriftType::Added => {
                    #[cfg(test)]
                    {
                        let seam = INJECTED_RESTORE_SEAM.with(|c| c.get());
                        if seam == InjectedRestoreSeam::DuringAddedArtifactQuarantine {
                            return Err(FreezeError::Io(
                                "Injected restore seam: DuringAddedArtifactQuarantine".to_string(),
                            ));
                        }
                    }

                    let active_p = coalition_dir.join(&op.path);
                    if active_p.exists() {
                        if let Some(ref q_rel) = op.quarantine_dest_rel {
                            let q_p = coalition_dir.join(q_rel);
                            if let Some(parent) = q_p.parent() {
                                fs::create_dir_all(parent).map_err(|e| {
                                    FreezeError::Io(format!(
                                        "Failed to create quarantine parent: {}",
                                        e
                                    ))
                                })?;
                            }
                            if let Err(e) = fs::rename(&active_p, &q_p) {
                                fs::copy(&active_p, &q_p).map_err(|ce| {
                                    FreezeError::Io(format!(
                                        "Failed to quarantine added file: rename: {}, copy: {}",
                                        e, ce
                                    ))
                                })?;
                                fs::remove_file(&active_p).map_err(|re| {
                                    FreezeError::Io(format!(
                                        "Failed to remove quarantined source file: {}",
                                        re
                                    ))
                                })?;
                            }
                            if let Some(parent) = q_p.parent() {
                                durable_directory_sync(parent).map_err(|e| {
                                    FreezeError::Io(format!("Failed to sync quarantine dir: {}", e))
                                })?;
                            }
                        }
                    }
                }
            }

            #[cfg(test)]
            {
                let seam = INJECTED_RESTORE_SEAM.with(|c| c.get());
                if idx == 0 && seam == InjectedRestoreSeam::AfterFirstRestoreMutation {
                    return Err(FreezeError::Io(
                        "Injected restore seam: AfterFirstRestoreMutation".to_string(),
                    ));
                }
                if op_count > 1
                    && idx == op_count / 2
                    && seam == InjectedRestoreSeam::MidRestoreBatch
                {
                    return Err(FreezeError::Io(
                        "Injected restore seam: MidRestoreBatch".to_string(),
                    ));
                }
            }
        }

        #[cfg(test)]
        {
            let seam = INJECTED_RESTORE_SEAM.with(|c| c.get());
            if seam == InjectedRestoreSeam::AfterAllMutationsBeforeVerification {
                return Err(FreezeError::Io(
                    "Injected restore seam: AfterAllMutationsBeforeVerification".to_string(),
                ));
            }
        }

        // 7. Verify contract drift is completely resolved before cleaning journals
        let post_report = Self::check_contract_drift(root, project_id)?;
        if post_report.has_drift {
            return Err(FreezeError::DriftRestorationRecoveryRequired(format!(
                "Drift restoration failed to restore clean contract: {} artifacts remain drifted",
                post_report.drifted_artifacts.len()
            )));
        }

        #[cfg(test)]
        {
            let seam = INJECTED_RESTORE_SEAM.with(|c| c.get());
            if seam == InjectedRestoreSeam::AfterVerificationBeforeJournalCleanup {
                return Err(FreezeError::Io(
                    "Injected restore seam: AfterVerificationBeforeJournalCleanup".to_string(),
                ));
            }
        }

        // 8. Cleanup journal and staging files (fails closed without swallowing errors)
        if staging_batch_dir.exists() {
            fs::remove_dir_all(&staging_batch_dir).map_err(|e| {
                FreezeError::Io(format!("Failed to remove staging restore dir: {}", e))
            })?;
        }
        if disk_journal_path.exists() {
            fs::remove_file(&disk_journal_path).map_err(|e| {
                FreezeError::Io(format!("Failed to remove disk drift journal: {}", e))
            })?;
        }
        durable_directory_sync(&recovery_dir).map_err(|e| {
            FreezeError::Io(format!("Failed to sync recovery dir after cleanup: {}", e))
        })?;

        conn.execute(
            "DELETE FROM drift_restoration_journals WHERE journal_id = ?1",
            params![journal.journal_id],
        )?;

        Ok(post_report)
    }

    /// Retrieves the bounded Builder implementation packet for a frozen version.
    pub fn get_builder_packet<P: AsRef<Path>>(
        repo_root: P,
        version_override: Option<&str>,
    ) -> Result<BuilderPacket, FreezeError> {
        let root = repo_root.as_ref();
        let coalition_dir = ArtifactManager::resolve_coalition_dir(root)?;
        let project_yaml_path = coalition_dir.join("project.yaml");
        let project_yaml = ArtifactManager::read_project_yaml(&project_yaml_path)?;

        let version = version_override
            .or(project_yaml.current_architecture_version.as_deref())
            .unwrap_or(INITIAL_ARCHITECTURE_VERSION);

        Self::validate_architecture_version(version)?;

        // Verify snapshot integrity
        Self::verify_snapshot_integrity(
            root,
            version,
            project_yaml.active_manifest_fingerprint.as_deref(),
        )?;

        let packet_path = coalition_dir
            .join("architecture-versions")
            .join(format!("v{}", version))
            .join("builder-packet.json");

        if !packet_path.exists() {
            return Err(FreezeError::FrozenSnapshotCorrupt {
                version: version.to_string(),
                reason: "builder-packet.json missing in snapshot".to_string(),
            });
        }

        let content = fs::read_to_string(&packet_path)
            .map_err(|e| FreezeError::Io(format!("Failed to read builder-packet.json: {}", e)))?;

        serde_json::from_str(&content).map_err(|e| FreezeError::Artifact(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::artifacts::ArtifactManager;
    use crate::core::git::GitAdapter;
    use crate::db::DbManager;
    use tempfile::tempdir;

    fn setup_git_repo(path: &Path) {
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
    }

    fn commit_file(path: &Path, file_name: &str, content: &str, message: &str) {
        fs::write(path.join(file_name), content).unwrap();
        std::process::Command::new("git")
            .args(["add", file_name])
            .current_dir(path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", message])
            .current_dir(path)
            .output()
            .unwrap();
    }

    fn insert_test_project_and_workflow(
        db: &mut DbManager,
        project_id: &str,
        repo_path: &Path,
        state: &str,
    ) {
        let now = chrono::Utc::now().to_rfc3339();
        db.connection()
            .execute(
                "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at)
                 VALUES (?1, 'Test Proj', ?2, ?3, ?3, ?3)",
                params![project_id, repo_path.to_string_lossy().to_string(), now],
            )
            .unwrap();
        db.connection()
            .execute(
                "INSERT INTO workflow_state (project_id, state, revision, updated_at) VALUES (?1, ?2, 1, ?3)",
                params![project_id, state, now],
            )
            .unwrap();
    }

    fn populate_ready_artifacts(repo_root: &Path) {
        let files = [
            (
                "design/product-vision.md",
                "# Product Vision\n\nSubstantive product vision content for test project.",
            ),
            (
                "design/requirements.md",
                "# Requirements\n\nSubstantive requirements content for test project.",
            ),
            (
                "design/architecture.md",
                "# Architecture\n\nSubstantive architecture content for test project.",
            ),
            (
                "design/constraints.md",
                "# Constraints\n\nSubstantive constraints content for test project.",
            ),
            (
                "design/interfaces.md",
                "# Interfaces\n\nSubstantive interfaces content for test project.",
            ),
            (
                "design/security.md",
                "# Security\n\nSubstantive security content for test project.",
            ),
            (
                "implementation/implementation-plan.md",
                "# Implementation Plan\n\nSubstantive implementation plan content.",
            ),
            (
                "implementation/acceptance-criteria.yaml",
                "schema_version: 1\ncriteria:\n  - id: AC-1\n    description: Must pass tests\n",
            ),
            (
                "implementation/test-plan.md",
                "# Test Plan\n\nSubstantive test plan content.",
            ),
            (
                "design/open-questions.md",
                "# Open Questions\n\nAll questions addressed.",
            ),
        ];

        for (rel, content) in files {
            ArtifactManager::write_artifact_atomic(repo_root, rel, content).unwrap();
        }
    }

    #[test]
    fn test_freeze_fails_on_unborn_repository_no_head_commit() {
        let dir = tempdir().unwrap();
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();

        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        let project_yaml =
            ArtifactManager::initialize_new_project(dir.path(), "test-unborn").unwrap();
        insert_test_project_and_workflow(
            &mut db,
            &project_yaml.project_id,
            dir.path(),
            "READY_TO_FREEZE",
        );
        populate_ready_artifacts(dir.path());

        let git = GitAdapter::new().unwrap();
        let res = FreezeService::prepare_freeze_preview(
            dir.path(),
            &project_yaml.project_id,
            &git,
            db.connection(),
        );
        assert_eq!(res.unwrap_err(), FreezeError::NoHeadCommit);
    }

    #[test]
    fn test_freeze_fails_when_readiness_is_incomplete() {
        let dir = tempdir().unwrap();
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();

        setup_git_repo(dir.path());
        commit_file(dir.path(), "README.md", "# Test", "Initial commit");

        let project_yaml =
            ArtifactManager::initialize_new_project(dir.path(), "test-incomplete").unwrap();
        insert_test_project_and_workflow(
            &mut db,
            &project_yaml.project_id,
            dir.path(),
            "READY_TO_FREEZE",
        );

        let git = GitAdapter::new().unwrap();
        let res = FreezeService::prepare_freeze_preview(
            dir.path(),
            &project_yaml.project_id,
            &git,
            db.connection(),
        );
        assert!(matches!(
            res.unwrap_err(),
            FreezeError::ReadinessIncomplete(_)
        ));
    }

    #[test]
    fn test_stale_freeze_preview_rejection_on_artifact_or_git_change() {
        let dir = tempdir().unwrap();
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();

        setup_git_repo(dir.path());
        commit_file(dir.path(), "README.md", "# Test", "Initial commit");

        let project_yaml =
            ArtifactManager::initialize_new_project(dir.path(), "test-stale-preview").unwrap();
        insert_test_project_and_workflow(
            &mut db,
            &project_yaml.project_id,
            dir.path(),
            "READY_TO_FREEZE",
        );
        populate_ready_artifacts(dir.path());

        let git = GitAdapter::new().unwrap();
        let preview = FreezeService::prepare_freeze_preview(
            dir.path(),
            &project_yaml.project_id,
            &git,
            db.connection(),
        )
        .unwrap();

        // 1. Stale due to artifact modification
        ArtifactManager::write_artifact_atomic(
            dir.path(),
            "design/product-vision.md",
            "# Modified Vision\n\nSubstantive modified product vision content for testing.",
        )
        .unwrap();

        let res = FreezeService::confirm_freeze(
            dir.path(),
            &project_yaml.project_id,
            &preview.preview_id,
            &git,
            db.connection_mut(),
        );
        assert!(matches!(
            res.unwrap_err(),
            FreezeError::StaleFreezePreview(_)
        ));

        // Restore original content
        populate_ready_artifacts(dir.path());

        // Refresh preview
        let preview2 = FreezeService::prepare_freeze_preview(
            dir.path(),
            &project_yaml.project_id,
            &git,
            db.connection(),
        )
        .unwrap();

        // Old preview is now SUPERSEDED
        let res_old = FreezeService::confirm_freeze(
            dir.path(),
            &project_yaml.project_id,
            &preview.preview_id,
            &git,
            db.connection_mut(),
        );
        assert!(matches!(
            res_old.unwrap_err(),
            FreezeError::StaleFreezePreview(_)
        ));

        // 2. Stale due to Git boundary change (new commit)
        commit_file(dir.path(), "extra.txt", "content", "Extra commit");
        let res2 = FreezeService::confirm_freeze(
            dir.path(),
            &project_yaml.project_id,
            &preview2.preview_id,
            &git,
            db.connection_mut(),
        );
        assert!(matches!(
            res2.unwrap_err(),
            FreezeError::StaleFreezePreview(_)
        ));
    }

    #[test]
    fn test_successful_freeze_journey_and_packet_identity_equality() {
        let dir = tempdir().unwrap();
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();

        setup_git_repo(dir.path());
        commit_file(dir.path(), "README.md", "# Test", "Initial commit");

        let project_yaml =
            ArtifactManager::initialize_new_project(dir.path(), "test-freeze-success").unwrap();
        insert_test_project_and_workflow(
            &mut db,
            &project_yaml.project_id,
            dir.path(),
            "READY_TO_FREEZE",
        );
        populate_ready_artifacts(dir.path());

        let git = GitAdapter::new().unwrap();
        let preview = FreezeService::prepare_freeze_preview(
            dir.path(),
            &project_yaml.project_id,
            &git,
            db.connection(),
        )
        .unwrap();

        let result = FreezeService::confirm_freeze(
            dir.path(),
            &project_yaml.project_id,
            &preview.preview_id,
            &git,
            db.connection_mut(),
        )
        .unwrap();

        assert_eq!(result.architecture_version, "1.0");
        assert_eq!(
            result.git_boundary.head_commit,
            preview.git_boundary.head_commit
        );

        // REGRESSION TEST: FreezeResult.builder_packet == get_builder_packet() durable representation
        let durable_packet = FreezeService::get_builder_packet(dir.path(), None).unwrap();
        assert_eq!(
            result.builder_packet.metadata, durable_packet.metadata,
            "Authoritative metadata must match byte-for-byte between returned and durable packet"
        );
        assert_eq!(
            result.builder_packet.prompt, durable_packet.prompt,
            "Prompt must match byte-for-byte"
        );
        assert_eq!(
            result.builder_packet.artifacts.len(),
            durable_packet.artifacts.len()
        );
        assert_eq!(
            result.contract_fingerprint,
            durable_packet.metadata.contract_fingerprint
        );

        // Verify preview cannot be reused (single-use check)
        let second_confirm = FreezeService::confirm_freeze(
            dir.path(),
            &project_yaml.project_id,
            &preview.preview_id,
            &git,
            db.connection_mut(),
        );
        assert!(matches!(
            second_confirm.unwrap_err(),
            FreezeError::StaleFreezePreview(_)
        ));
    }

    #[test]
    fn test_drift_detection_and_non_destructive_restoration() {
        let dir = tempdir().unwrap();
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();

        setup_git_repo(dir.path());
        commit_file(dir.path(), "README.md", "# Test", "Initial commit");

        let project_yaml =
            ArtifactManager::initialize_new_project(dir.path(), "test-drift").unwrap();
        insert_test_project_and_workflow(
            &mut db,
            &project_yaml.project_id,
            dir.path(),
            "READY_TO_FREEZE",
        );
        populate_ready_artifacts(dir.path());

        let git = GitAdapter::new().unwrap();
        let preview = FreezeService::prepare_freeze_preview(
            dir.path(),
            &project_yaml.project_id,
            &git,
            db.connection(),
        )
        .unwrap();

        FreezeService::confirm_freeze(
            dir.path(),
            &project_yaml.project_id,
            &preview.preview_id,
            &git,
            db.connection_mut(),
        )
        .unwrap();

        // 1. Initial drift report is clean
        let drift_init =
            FreezeService::check_contract_drift(dir.path(), &project_yaml.project_id).unwrap();
        assert!(!drift_init.has_drift);

        // 2. Introduce MODIFIED, DELETED, and ADDED drift
        ArtifactManager::write_artifact_atomic(
            dir.path(),
            "design/product-vision.md",
            "# Drifted Vision\nTampered",
        )
        .unwrap();
        ArtifactManager::delete_artifact(dir.path(), "design/constraints.md").unwrap();
        ArtifactManager::write_artifact_atomic(
            dir.path(),
            "decisions/ADR-009-unfrozen.md",
            "# Unfrozen ADR",
        )
        .unwrap();

        let drift_report =
            FreezeService::check_contract_drift(dir.path(), &project_yaml.project_id).unwrap();
        assert!(drift_report.has_drift);
        assert_eq!(drift_report.drifted_artifacts.len(), 3);

        // Test diff inspection
        let diff = FreezeService::get_drift_diff(
            dir.path(),
            &project_yaml.project_id,
            "design/product-vision.md",
        )
        .unwrap();
        assert_eq!(diff.drift_type, DriftType::Modified);
        assert!(diff
            .frozen_content
            .unwrap()
            .contains("Substantive product vision"));
        assert!(diff.active_content.unwrap().contains("Drifted Vision"));

        // 3. Restore all drifted artifacts
        let restored_report = FreezeService::restore_all_drifted_artifacts(
            dir.path(),
            &project_yaml.project_id,
            db.connection_mut(),
        )
        .unwrap();
        assert!(!restored_report.has_drift);

        // Verify quarantined ADDED file is preserved in .coalition/recovery/
        let recovery_dir = dir.path().join(".coalition").join("recovery");
        assert!(recovery_dir.exists());
        let entries: Vec<_> = fs::read_dir(&recovery_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("quarantine-"))
            .collect();
        assert!(!entries.is_empty(), "Quarantined directory must exist");
    }

    #[test]
    fn test_drift_path_traversal_and_unmanaged_rejection() {
        let dir = tempdir().unwrap();
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();

        setup_git_repo(dir.path());
        commit_file(dir.path(), "README.md", "# Test", "Initial commit");

        let project_yaml =
            ArtifactManager::initialize_new_project(dir.path(), "test-drift-security").unwrap();
        insert_test_project_and_workflow(
            &mut db,
            &project_yaml.project_id,
            dir.path(),
            "READY_TO_FREEZE",
        );
        populate_ready_artifacts(dir.path());

        let git = GitAdapter::new().unwrap();
        let preview = FreezeService::prepare_freeze_preview(
            dir.path(),
            &project_yaml.project_id,
            &git,
            db.connection(),
        )
        .unwrap();

        FreezeService::confirm_freeze(
            dir.path(),
            &project_yaml.project_id,
            &preview.preview_id,
            &git,
            db.connection_mut(),
        )
        .unwrap();

        let evil_drift_paths = [
            "../README.md",
            "../../outside",
            "project.yaml",
            ".git/config",
            "/etc/passwd",
            "C:\\windows\\system32\\evil.dll",
            "design/unmanaged.txt",
        ];

        for &bad in &evil_drift_paths {
            let res_diff = FreezeService::get_drift_diff(dir.path(), &project_yaml.project_id, bad);
            assert!(
                res_diff.is_err(),
                "get_drift_diff must reject malicious path '{}'",
                bad
            );

            let res_restore = FreezeService::restore_drifted_artifact(
                dir.path(),
                &project_yaml.project_id,
                bad,
                db.connection_mut(),
            );
            assert!(
                res_restore.is_err(),
                "restore_drifted_artifact must reject malicious path '{}'",
                bad
            );
        }
    }

    #[test]
    fn test_strict_snapshot_integrity_rejects_unmanifested_files() {
        let dir = tempdir().unwrap();
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();

        setup_git_repo(dir.path());
        commit_file(dir.path(), "README.md", "# Test", "Initial commit");

        let project_yaml =
            ArtifactManager::initialize_new_project(dir.path(), "test-strict-snapshot").unwrap();
        insert_test_project_and_workflow(
            &mut db,
            &project_yaml.project_id,
            dir.path(),
            "READY_TO_FREEZE",
        );
        populate_ready_artifacts(dir.path());

        let git = GitAdapter::new().unwrap();
        let preview = FreezeService::prepare_freeze_preview(
            dir.path(),
            &project_yaml.project_id,
            &git,
            db.connection(),
        )
        .unwrap();

        FreezeService::confirm_freeze(
            dir.path(),
            &project_yaml.project_id,
            &preview.preview_id,
            &git,
            db.connection_mut(),
        )
        .unwrap();

        // Drop an unmanifested rogue file into v1.0/contract/
        let rogue_path = dir
            .path()
            .join(".coalition")
            .join("architecture-versions")
            .join("v1.0")
            .join("contract")
            .join("rogue.md");
        fs::write(&rogue_path, "unmanifested file inserted into snapshot").unwrap();

        let verify_res = FreezeService::verify_snapshot_integrity(dir.path(), "1.0", None);
        assert!(matches!(
            verify_res.unwrap_err(),
            FreezeError::FrozenSnapshotCorrupt { .. }
        ));
    }

    #[test]
    fn test_builder_packet_budgeting_and_utf8_char_boundary_truncation() {
        let text = "Hello 🦀 World";
        // '🦀' starts at index 6 and ends at index 10
        assert_eq!(floor_char_boundary(text, 6), 6);
        assert_eq!(floor_char_boundary(text, 7), 6); // Inside 🦀
        assert_eq!(floor_char_boundary(text, 8), 6); // Inside 🦀
        assert_eq!(floor_char_boundary(text, 9), 6); // Inside 🦀
        assert_eq!(floor_char_boundary(text, 10), 10); // After 🦀
    }

    #[test]
    fn test_all_freeze_failure_injection_seams() {
        let seams = [
            InjectedFreezeSeam::PreStaging,
            InjectedFreezeSeam::MidStaging,
            InjectedFreezeSeam::PostStagingPreFinalize,
            InjectedFreezeSeam::PostFinalizePreCommit,
            InjectedFreezeSeam::PostCommitPreDb,
            InjectedFreezeSeam::MidDbTransaction,
        ];

        for &seam in &seams {
            let dir = tempdir().unwrap();
            let mut db = DbManager::new_in_memory().unwrap();
            db.run_migrations().unwrap();

            setup_git_repo(dir.path());
            commit_file(dir.path(), "README.md", "# Test", "Initial commit");

            let project_yaml =
                ArtifactManager::initialize_new_project(dir.path(), "test-seams").unwrap();
            insert_test_project_and_workflow(
                &mut db,
                &project_yaml.project_id,
                dir.path(),
                "READY_TO_FREEZE",
            );
            populate_ready_artifacts(dir.path());

            let git = GitAdapter::new().unwrap();
            let preview = FreezeService::prepare_freeze_preview(
                dir.path(),
                &project_yaml.project_id,
                &git,
                db.connection(),
            )
            .unwrap();

            INJECTED_FREEZE_SEAM.with(|c| c.set(seam));
            let res = FreezeService::confirm_freeze(
                dir.path(),
                &project_yaml.project_id,
                &preview.preview_id,
                &git,
                db.connection_mut(),
            );
            INJECTED_FREEZE_SEAM.with(|c| c.set(InjectedFreezeSeam::None));

            assert!(res.is_err(), "Seam {:?} must fail freeze", seam);

            if seam == InjectedFreezeSeam::MidDbTransaction {
                // Assert that DB transaction rolled back completely
                let preview_status: String = db
                    .connection()
                    .query_row(
                        "SELECT status FROM freeze_previews WHERE preview_id = ?1",
                        params![preview.preview_id],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert_eq!(
                    preview_status, "PENDING",
                    "Preview must remain PENDING on rollback"
                );

                let pre_wf =
                    workflow::get_workflow_state(db.connection(), &project_yaml.project_id)
                        .unwrap();
                assert_eq!(
                    pre_wf.state,
                    WorkflowState::ReadyToFreeze,
                    "Workflow must remain READY_TO_FREEZE on rollback"
                );

                let epoch_count: i64 = db
                    .connection()
                    .query_row(
                        "SELECT COUNT(*) FROM builder_epochs WHERE project_id = ?1",
                        params![project_yaml.project_id],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert_eq!(epoch_count, 0, "No epochs must be committed on rollback");
            }

            // Reconcile and test safe convergence
            FreezeService::reconcile_freeze_state(
                dir.path(),
                &project_yaml.project_id,
                db.connection_mut(),
            )
            .unwrap();

            let post_yaml = ArtifactManager::read_project_yaml(
                dir.path().join(".coalition").join("project.yaml"),
            )
            .unwrap();
            let wf_state =
                workflow::get_workflow_state(db.connection(), &project_yaml.project_id).unwrap();

            match seam {
                InjectedFreezeSeam::PreStaging
                | InjectedFreezeSeam::MidStaging
                | InjectedFreezeSeam::PostStagingPreFinalize
                | InjectedFreezeSeam::PostFinalizePreCommit => {
                    // Converges to safe unfrozen retry
                    assert_eq!(post_yaml.architecture_state, ArchitectureState::Draft);
                    assert_eq!(wf_state.state, WorkflowState::ReadyToFreeze);
                }
                InjectedFreezeSeam::PostCommitPreDb | InjectedFreezeSeam::MidDbTransaction => {
                    // Converges to fully frozen v1.0 with operational records reconstructed
                    assert_eq!(post_yaml.architecture_state, ArchitectureState::Frozen);
                    assert_eq!(wf_state.state, WorkflowState::Frozen);

                    let epoch_count: i64 = db
                        .connection()
                        .query_row(
                            "SELECT COUNT(*) FROM builder_epochs WHERE project_id = ?1",
                            params![project_yaml.project_id],
                            |r| r.get(0),
                        )
                        .unwrap();
                    assert_eq!(epoch_count, 1, "Epoch must be reconstructed");

                    let boundary_count: i64 = db
                        .connection()
                        .query_row(
                            "SELECT COUNT(*) FROM frozen_boundaries WHERE project_id = ?1",
                            params![project_yaml.project_id],
                            |r| r.get(0),
                        )
                        .unwrap();
                    assert_eq!(boundary_count, 1, "Boundary must be reconstructed");

                    let preview_status: String = db
                        .connection()
                        .query_row(
                            "SELECT status FROM freeze_previews WHERE preview_id = ?1",
                            params![preview.preview_id],
                            |r| r.get(0),
                        )
                        .unwrap();
                    assert_eq!(
                        preview_status, "CONSUMED",
                        "Preview must be reconciled to CONSUMED"
                    );

                    let event_count: i64 = db
                        .connection()
                        .query_row(
                            "SELECT COUNT(*) FROM activity_events WHERE project_id = ?1 AND event_type = 'ARCHITECTURE_FROZEN'",
                            params![project_yaml.project_id],
                            |r| r.get(0),
                        )
                        .unwrap();
                    assert_eq!(
                        event_count, 1,
                        "ARCHITECTURE_FROZEN activity event must be reconciled"
                    );

                    let packet = FreezeService::get_builder_packet(dir.path(), None);
                    assert!(packet.is_ok(), "Builder packet must be retrievable");
                }
                InjectedFreezeSeam::None => unreachable!(),
            }
        }
    }

    #[test]
    fn test_all_restoration_failure_injection_seams() {
        let seams = [
            InjectedRestoreSeam::BeforeFirstRestoreMutation,
            InjectedRestoreSeam::AfterFirstRestoreMutation,
            InjectedRestoreSeam::MidRestoreBatch,
            InjectedRestoreSeam::DuringAddedArtifactQuarantine,
            InjectedRestoreSeam::AfterAllMutationsBeforeVerification,
            InjectedRestoreSeam::AfterVerificationBeforeJournalCleanup,
        ];

        for &seam in &seams {
            let dir = tempdir().unwrap();
            let mut db = DbManager::new_in_memory().unwrap();
            db.run_migrations().unwrap();

            setup_git_repo(dir.path());
            commit_file(dir.path(), "README.md", "# Test", "Initial commit");

            let project_yaml =
                ArtifactManager::initialize_new_project(dir.path(), "test-restore-seams").unwrap();
            insert_test_project_and_workflow(
                &mut db,
                &project_yaml.project_id,
                dir.path(),
                "READY_TO_FREEZE",
            );
            populate_ready_artifacts(dir.path());

            let git = GitAdapter::new().unwrap();
            let preview = FreezeService::prepare_freeze_preview(
                dir.path(),
                &project_yaml.project_id,
                &git,
                db.connection(),
            )
            .unwrap();

            FreezeService::confirm_freeze(
                dir.path(),
                &project_yaml.project_id,
                &preview.preview_id,
                &git,
                db.connection_mut(),
            )
            .unwrap();

            // Introduce 3 kinds of drift: modified, deleted, added
            ArtifactManager::write_artifact_atomic(
                dir.path(),
                "design/product-vision.md",
                "# Drifted Vision\n\nModified content.",
            )
            .unwrap();

            let target_req = dir.path().join(".coalition").join("design/requirements.md");
            fs::remove_file(target_req).unwrap();

            let rogue_added = dir
                .path()
                .join(".coalition")
                .join("decisions/ADR-999-rogue.md");
            fs::create_dir_all(dir.path().join(".coalition").join("decisions")).unwrap();
            fs::write(rogue_added, "# Rogue ADR\n\nUnexpected added decision.").unwrap();

            let drift =
                FreezeService::check_contract_drift(dir.path(), &project_yaml.project_id).unwrap();
            assert!(drift.has_drift);
            assert_eq!(drift.drifted_artifacts.len(), 3);

            // Inject seam and call restore
            INJECTED_RESTORE_SEAM.with(|c| c.set(seam));
            let res = FreezeService::restore_all_drifted_artifacts(
                dir.path(),
                &project_yaml.project_id,
                db.connection_mut(),
            );
            INJECTED_RESTORE_SEAM.with(|c| c.set(InjectedRestoreSeam::None));

            assert!(res.is_err(), "Seam {:?} must fail restore", seam);

            // Reconcile and assert clean convergence
            FreezeService::reconcile_drift_restoration(
                dir.path(),
                &project_yaml.project_id,
                db.connection_mut(),
            )
            .unwrap();

            let post_drift =
                FreezeService::check_contract_drift(dir.path(), &project_yaml.project_id).unwrap();
            assert!(
                !post_drift.has_drift,
                "Seam {:?} must safely converge to clean contract after reconcile",
                seam
            );

            // Verify disk journal is cleaned
            let disk_journal = dir
                .path()
                .join(".coalition")
                .join("recovery")
                .join("drift-restoration-journal.json");
            assert!(!disk_journal.exists(), "Disk journal must be cleaned up");

            // Verify SQLite journal row is cleaned
            let journal_count: i64 = db
                .connection()
                .query_row(
                    "SELECT COUNT(*) FROM drift_restoration_journals WHERE project_id = ?1",
                    params![project_yaml.project_id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(journal_count, 0, "SQLite journal record must be cleaned up");
        }
    }

    #[test]
    fn test_restoration_recovery_with_sqlite_loss_during_committing() {
        let dir = tempdir().unwrap();
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();

        setup_git_repo(dir.path());
        commit_file(dir.path(), "README.md", "# Test", "Initial commit");

        let project_yaml =
            ArtifactManager::initialize_new_project(dir.path(), "test-restore-loss").unwrap();
        insert_test_project_and_workflow(
            &mut db,
            &project_yaml.project_id,
            dir.path(),
            "READY_TO_FREEZE",
        );
        populate_ready_artifacts(dir.path());

        let git = GitAdapter::new().unwrap();
        let preview = FreezeService::prepare_freeze_preview(
            dir.path(),
            &project_yaml.project_id,
            &git,
            db.connection(),
        )
        .unwrap();

        FreezeService::confirm_freeze(
            dir.path(),
            &project_yaml.project_id,
            &preview.preview_id,
            &git,
            db.connection_mut(),
        )
        .unwrap();

        // Mutate product-vision
        ArtifactManager::write_artifact_atomic(
            dir.path(),
            "design/product-vision.md",
            "# Corrupted Vision\n\nModified content.",
        )
        .unwrap();

        // Fail restore at BeforeFirstRestoreMutation (disk journal is durably COMMITTING)
        INJECTED_RESTORE_SEAM.with(|c| c.set(InjectedRestoreSeam::BeforeFirstRestoreMutation));
        let res = FreezeService::restore_all_drifted_artifacts(
            dir.path(),
            &project_yaml.project_id,
            db.connection_mut(),
        );
        INJECTED_RESTORE_SEAM.with(|c| c.set(InjectedRestoreSeam::None));
        assert!(res.is_err());

        // Verify disk journal is COMMITTING
        let disk_journal = dir
            .path()
            .join(".coalition")
            .join("recovery")
            .join("drift-restoration-journal.json");
        assert!(disk_journal.exists(), "Disk journal must exist on disk");
        let content = fs::read_to_string(&disk_journal).unwrap();
        let j: DriftRestorationJournal = serde_json::from_str(&content).unwrap();
        assert_eq!(j.phase, RestorationPhase::Committing);

        // Simulate complete SQLite loss: create a brand new clean database
        let mut fresh_db = DbManager::new_in_memory().unwrap();
        fresh_db.run_migrations().unwrap();

        // Run reconcile_drift_restoration on fresh DB with no prior records
        FreezeService::reconcile_drift_restoration(
            dir.path(),
            &project_yaml.project_id,
            fresh_db.connection_mut(),
        )
        .unwrap();

        // Contract must be completely restored from disk journal alone!
        let post_drift =
            FreezeService::check_contract_drift(dir.path(), &project_yaml.project_id).unwrap();
        assert!(
            !post_drift.has_drift,
            "Contract must be restored cleanly from disk journal alone"
        );
        assert!(!disk_journal.exists(), "Disk journal must be cleaned up");
    }

    #[test]
    fn test_architecture_version_validation_and_containment() {
        // 1. Version format validation
        assert!(validate_architecture_version("1.0").is_ok());
        assert!(validate_architecture_version("1.0-alpha").is_ok());
        assert!(validate_architecture_version("v2.0").is_ok());
        assert!(validate_architecture_version("3.14-beta.1").is_ok());

        assert!(matches!(
            validate_architecture_version("").unwrap_err(),
            FreezeError::InvalidArchitectureVersion(_)
        ));
        assert!(matches!(
            validate_architecture_version("   ").unwrap_err(),
            FreezeError::InvalidArchitectureVersion(_)
        ));
        assert!(matches!(
            validate_architecture_version("1.0/../escape").unwrap_err(),
            FreezeError::InvalidArchitectureVersion(_)
        ));
        assert!(matches!(
            validate_architecture_version("1.0\\escape").unwrap_err(),
            FreezeError::InvalidArchitectureVersion(_)
        ));
        assert!(matches!(
            validate_architecture_version("1.0\0bad").unwrap_err(),
            FreezeError::InvalidArchitectureVersion(_)
        ));
        assert!(matches!(
            validate_architecture_version("123456789012345678901234567890123").unwrap_err(),
            FreezeError::InvalidArchitectureVersion(_)
        ));

        // 2. Traversal version rejected by verify_snapshot_integrity
        let dir = tempdir().unwrap();
        let res = FreezeService::verify_snapshot_integrity(dir.path(), "../escape", None);
        assert!(matches!(
            res.unwrap_err(),
            FreezeError::InvalidArchitectureVersion(_)
        ));
    }

    #[test]
    fn test_prompt_overhead_budget_truncation() {
        let manifest_artifacts = vec![
            ContractArtifactEntry {
                relative_path: "design/product-vision.md".to_string(),
                fingerprint: "abc".to_string(),
                size_bytes: 104_000,
            },
            ContractArtifactEntry {
                relative_path: "design/architecture.md".to_string(),
                fingerprint: "def".to_string(),
                size_bytes: 100_500, // Total content = 204,500 (< 204,800 bytes = 200 KB)
            },
        ];

        let (prompt, _rules, artifacts, is_truncated, prompt_bytes) =
            FreezeService::build_builder_packet_prompt_and_artifacts(
                "1.0",
                "Test Project",
                "test-proj-123",
                "0123456789abcdef0123456789abcdef01234567",
                "contractfp123",
                &manifest_artifacts,
                |rel| {
                    if rel == "design/product-vision.md" {
                        Ok("A".repeat(104_000))
                    } else {
                        Ok("B".repeat(100_500))
                    }
                },
            )
            .unwrap();

        // Because prompt header, markdown headings, and footers push total prompt over 200 KB:
        assert!(
            is_truncated,
            "Must be truncated due to prompt context budget"
        );
        assert!(
            prompt_bytes <= BUILDER_PACKET_MAX_BYTES,
            "Prompt bytes must not exceed 200 KB"
        );
        assert!(prompt.len() <= BUILDER_PACKET_MAX_BYTES);
        assert_eq!(artifacts.len(), 2);
        assert!(artifacts[1].content.contains("[TRUNCATED"));
    }

    #[test]
    fn test_validate_restoration_journal_untrusted_input_rejection() {
        let dir = tempdir().unwrap();
        let coalition_dir = dir.path().join(".coalition");
        fs::create_dir_all(&coalition_dir).unwrap();

        let valid_jid = "test-journal-123";
        let valid_op = RestorationOp {
            path: "design/product-vision.md".to_string(),
            drift_type: DriftType::Modified,
            staged_file_rel: Some(format!(
                "recovery/.staging-restore-{}/design_product-vision.md.staged",
                valid_jid
            )),
            quarantine_dest_rel: None,
        };

        // 1. Valid journal must pass
        let valid_journal = DriftRestorationJournal {
            journal_id: valid_jid.to_string(),
            project_id: "proj-1".to_string(),
            architecture_version: "1.0".to_string(),
            phase: RestorationPhase::Committing,
            operations: vec![valid_op.clone()],
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };
        assert!(
            FreezeService::validate_restoration_journal(&coalition_dir, &valid_journal).is_ok()
        );

        // 2. Malicious journal_id with traversal or separators
        let bad_jid_journal = DriftRestorationJournal {
            journal_id: "../escape".to_string(),
            ..valid_journal.clone()
        };
        assert!(matches!(
            FreezeService::validate_restoration_journal(&coalition_dir, &bad_jid_journal)
                .unwrap_err(),
            FreezeError::DriftRestorationRecoveryRequired(_)
        ));

        // 3. Invalid architecture version in journal
        let bad_ver_journal = DriftRestorationJournal {
            architecture_version: "1.0/../v2".to_string(),
            ..valid_journal.clone()
        };
        assert!(matches!(
            FreezeService::validate_restoration_journal(&coalition_dir, &bad_ver_journal)
                .unwrap_err(),
            FreezeError::DriftRestorationRecoveryRequired(_)
        ));

        // 4. Malicious op.path with traversal
        let bad_path_journal = DriftRestorationJournal {
            operations: vec![RestorationOp {
                path: "../README.md".to_string(),
                ..valid_op.clone()
            }],
            ..valid_journal.clone()
        };
        assert!(matches!(
            FreezeService::validate_restoration_journal(&coalition_dir, &bad_path_journal)
                .unwrap_err(),
            FreezeError::DriftRestorationRecoveryRequired(_)
        ));

        // 5. Malicious op.path with drive letter
        let drive_journal = DriftRestorationJournal {
            operations: vec![RestorationOp {
                path: "C:/Windows/System32/cmd.exe".to_string(),
                ..valid_op.clone()
            }],
            ..valid_journal.clone()
        };
        assert!(matches!(
            FreezeService::validate_restoration_journal(&coalition_dir, &drive_journal)
                .unwrap_err(),
            FreezeError::DriftRestorationRecoveryRequired(_)
        ));

        // 6. Malicious op.path with control chars
        let ctrl_journal = DriftRestorationJournal {
            operations: vec![RestorationOp {
                path: "design/product\0vision.md".to_string(),
                ..valid_op.clone()
            }],
            ..valid_journal.clone()
        };
        assert!(matches!(
            FreezeService::validate_restoration_journal(&coalition_dir, &ctrl_journal).unwrap_err(),
            FreezeError::DriftRestorationRecoveryRequired(_)
        ));

        // 7. Malicious op.path not an allowed architecture contract path
        let unmanaged_journal = DriftRestorationJournal {
            operations: vec![RestorationOp {
                path: "src/main.rs".to_string(),
                ..valid_op.clone()
            }],
            ..valid_journal.clone()
        };
        assert!(matches!(
            FreezeService::validate_restoration_journal(&coalition_dir, &unmanaged_journal)
                .unwrap_err(),
            FreezeError::DriftRestorationRecoveryRequired(_)
        ));

        // 8. Malicious staged_file_rel escaping staging directory
        let bad_staged_journal = DriftRestorationJournal {
            operations: vec![RestorationOp {
                staged_file_rel: Some("recovery/.staging-restore-OTHER/file.staged".to_string()),
                ..valid_op.clone()
            }],
            ..valid_journal.clone()
        };
        assert!(matches!(
            FreezeService::validate_restoration_journal(&coalition_dir, &bad_staged_journal)
                .unwrap_err(),
            FreezeError::DriftRestorationRecoveryRequired(_)
        ));

        // 9. Malicious quarantine_dest_rel escaping quarantine prefix
        let bad_quarantine_journal = DriftRestorationJournal {
            operations: vec![RestorationOp {
                path: "design/product-vision.md".to_string(),
                drift_type: DriftType::Added,
                staged_file_rel: None,
                quarantine_dest_rel: Some("design/product-vision.md".to_string()),
            }],
            ..valid_journal.clone()
        };
        assert!(matches!(
            FreezeService::validate_restoration_journal(&coalition_dir, &bad_quarantine_journal)
                .unwrap_err(),
            FreezeError::DriftRestorationRecoveryRequired(_)
        ));
    }

    #[test]
    fn test_conflicting_disk_and_sqlite_restoration_journals() {
        let dir = tempdir().unwrap();
        setup_git_repo(dir.path());
        commit_file(dir.path(), "README.md", "# Test", "Initial commit");
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();

        let project_yaml =
            ArtifactManager::initialize_new_project(dir.path(), "test-conflict-journals").unwrap();
        insert_test_project_and_workflow(
            &mut db,
            &project_yaml.project_id,
            dir.path(),
            "READY_TO_FREEZE",
        );
        populate_ready_artifacts(dir.path());

        let git = GitAdapter::new().unwrap();
        let preview = FreezeService::prepare_freeze_preview(
            dir.path(),
            &project_yaml.project_id,
            &git,
            db.connection(),
        )
        .unwrap();
        FreezeService::confirm_freeze(
            dir.path(),
            &project_yaml.project_id,
            &preview.preview_id,
            &git,
            db.connection_mut(),
        )
        .unwrap();

        let rec_dir = dir.path().join(".coalition").join("recovery");
        fs::create_dir_all(&rec_dir).unwrap();
        let disk_journal_path = rec_dir.join("drift-restoration-journal.json");

        let disk_j = DriftRestorationJournal {
            journal_id: "j-disk-1".to_string(),
            project_id: project_yaml.project_id.clone(),
            architecture_version: "1.0".to_string(),
            phase: RestorationPhase::Committing,
            operations: vec![RestorationOp {
                path: "design/product-vision.md".to_string(),
                drift_type: DriftType::Modified,
                staged_file_rel: Some(
                    "recovery/.staging-restore-j-disk-1/design_product-vision.md.staged"
                        .to_string(),
                ),
                quarantine_dest_rel: None,
            }],
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };
        fs::write(&disk_journal_path, serde_json::to_string(&disk_j).unwrap()).unwrap();

        // 1. Conflicting operations between SQLite and disk
        let conflicting_ops = vec![RestorationOp {
            path: "design/architecture.md".to_string(),
            drift_type: DriftType::Modified,
            staged_file_rel: Some(
                "recovery/.staging-restore-j-disk-1/design_architecture.md.staged".to_string(),
            ),
            quarantine_dest_rel: None,
        }];
        db.connection().execute(
            "INSERT INTO drift_restoration_journals (journal_id, project_id, architecture_version, phase, operations_json, created_at, updated_at)
             VALUES ('j-disk-1', ?1, '1.0', 'COMMITTING', ?2, '2026-01-01', '2026-01-01')",
            params![project_yaml.project_id, serde_json::to_string(&conflicting_ops).unwrap()],
        ).unwrap();

        let err = FreezeService::reconcile_drift_restoration(
            dir.path(),
            &project_yaml.project_id,
            db.connection_mut(),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            FreezeError::DriftRestorationRecoveryRequired(_)
        ));

        // 2. Unknown phase in SQLite journal fails closed
        db.connection().execute(
            "UPDATE drift_restoration_journals SET phase = 'UNKNOWN_PHASE', operations_json = ?1 WHERE journal_id = 'j-disk-1'",
            params![serde_json::to_string(&disk_j.operations).unwrap()],
        ).unwrap();

        let err2 = FreezeService::reconcile_drift_restoration(
            dir.path(),
            &project_yaml.project_id,
            db.connection_mut(),
        )
        .unwrap_err();
        assert!(matches!(
            err2,
            FreezeError::DriftRestorationRecoveryRequired(_)
        ));
    }

    #[test]
    fn test_read_contract_manifest_and_drift_diff_architecture_version_validation() {
        let dir = tempdir().unwrap();

        // Invalid version rejected in read_contract_manifest
        let err1 = FreezeService::read_contract_manifest(dir.path(), "../v2").unwrap_err();
        assert!(matches!(err1, FreezeError::InvalidArchitectureVersion(_)));

        let err2 = FreezeService::read_contract_manifest(dir.path(), "1.0\\escape").unwrap_err();
        assert!(matches!(err2, FreezeError::InvalidArchitectureVersion(_)));

        // Invalid version in get_drift_diff rejected
        let coalition_dir = dir.path().join(".coalition");
        fs::create_dir_all(&coalition_dir).unwrap();
        let proj_uuid = Uuid::new_v4().to_string();
        let py = crate::core::artifacts::ProjectYaml {
            schema_version: 1,
            project_id: proj_uuid.clone(),
            name: "Test".to_string(),
            current_architecture_version: Some("../escape".to_string()),
            architecture_state: ArchitectureState::Frozen,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            active_manifest_fingerprint: None,
            readiness: None,
        };
        fs::write(
            coalition_dir.join("project.yaml"),
            serde_yaml::to_string(&py).unwrap(),
        )
        .unwrap();

        let err3 =
            FreezeService::get_drift_diff(dir.path(), &proj_uuid, "design/product-vision.md")
                .unwrap_err();
        assert!(matches!(err3, FreezeError::InvalidArchitectureVersion(_)));
    }

    #[test]
    fn test_clean_stale_restore_staging() {
        let dir = tempdir().unwrap();
        let coalition_dir = dir.path().join(".coalition");
        let rec_dir = coalition_dir.join("recovery");
        fs::create_dir_all(&rec_dir).unwrap();

        let stale1 = rec_dir.join(".staging-restore-batch1");
        let stale2 = rec_dir.join(".staging-restore-batch2");
        let keeper_quarantine = rec_dir.join("quarantine-20260101-1");
        let keeper_file = rec_dir.join("drift-restoration-journal.json");

        fs::create_dir_all(&stale1).unwrap();
        fs::write(stale1.join("file1.staged"), "staged").unwrap();
        fs::create_dir_all(&stale2).unwrap();
        fs::write(stale2.join("file2.staged"), "staged").unwrap();
        fs::create_dir_all(&keeper_quarantine).unwrap();
        fs::write(keeper_quarantine.join("quarantined.md"), "data").unwrap();
        fs::write(&keeper_file, "{}").unwrap();

        ArtifactManager::clean_stale_restore_staging(&coalition_dir).unwrap();

        assert!(!stale1.exists(), "stale1 must be cleaned");
        assert!(!stale2.exists(), "stale2 must be cleaned");
        assert!(keeper_quarantine.exists(), "quarantine must not be touched");
        assert!(keeper_file.exists(), "journal file must not be touched");
    }
}

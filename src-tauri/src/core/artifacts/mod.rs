use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

pub const STANDARD_SUBDIRECTORIES: &[&str] = &[
    "design",
    "implementation",
    "decisions",
    "architecture-versions",
    "changes",
    "reviews",
    "evidence",
];

pub const CANONICAL_ARCHITECTURE_ARTIFACTS: &[&str] = &[
    "design/product-vision.md",
    "design/requirements.md",
    "design/architecture.md",
    "design/constraints.md",
    "design/interfaces.md",
    "design/security.md",
    "design/open-questions.md",
    "implementation/implementation-plan.md",
    "implementation/acceptance-criteria.yaml",
    "implementation/validation.yaml",
    "implementation/test-plan.md",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArchitectureState {
    Draft,
    Frozen,
}

impl std::fmt::Display for ArchitectureState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Draft => write!(f, "draft"),
            Self::Frozen => write!(f, "frozen"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectYaml {
    pub schema_version: u32,
    pub project_id: String,
    pub name: String,
    pub current_architecture_version: Option<String>,
    pub architecture_state: ArchitectureState,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryCandidate {
    pub path: PathBuf,
    pub project: ProjectYaml,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectArtifactInspection {
    pub canonical: Option<ProjectYaml>,
    pub valid_backups: Vec<RecoveryCandidate>,
    pub temp_files_present: bool,
    pub ambiguous_or_invalid_recovery_state: bool,
}

#[derive(Error, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArtifactError {
    #[error("Path traversal detected: {path} escapes repository root {root}")]
    PathTraversal { path: String, root: String },
    #[error("Unsafe symlink or reparse point detected at {0}")]
    UnsafeReparsePoint(String),
    #[error("Unsupported project schema version: {0}. Only version 1 is supported")]
    UnsupportedSchemaVersion(u32),
    #[error("Invalid project ID '{0}': must be a valid UUID")]
    InvalidProjectId(String),
    #[error("Invalid timestamp '{0}': must be RFC3339 formatted")]
    InvalidTimestamp(String),
    #[error("Project name cannot be empty")]
    EmptyProjectName,
    #[error("Invalid architecture state: {0}")]
    InvalidArchitectureState(String),
    #[error("Invalid YAML in project.yaml: {0}")]
    InvalidYaml(String),
    #[error("Project descriptor project.yaml not found at {0}")]
    NotFound(String),
    #[error("Ambiguous or corrupted backup state requires manual recovery: {0}")]
    RecoveryRequired(String),
    #[error("IO error: {0}")]
    Io(String),
}

pub struct ArtifactManager;

impl ArtifactManager {
    /// Validates an in-memory ProjectYaml descriptor against schema v1 invariants.
    pub fn validate_project_yaml(project: &ProjectYaml) -> Result<(), ArtifactError> {
        if project.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(ArtifactError::UnsupportedSchemaVersion(
                project.schema_version,
            ));
        }

        match Uuid::parse_str(&project.project_id) {
            Ok(parsed) => {
                if parsed.get_version() != Some(uuid::Version::Random) {
                    return Err(ArtifactError::InvalidProjectId(format!(
                        "{}: only UUID v4 is supported",
                        project.project_id
                    )));
                }
            }
            Err(_) => {
                return Err(ArtifactError::InvalidProjectId(project.project_id.clone()));
            }
        }

        if project.name.trim().is_empty() {
            return Err(ArtifactError::EmptyProjectName);
        }

        if chrono::DateTime::parse_from_rfc3339(&project.created_at).is_err() {
            return Err(ArtifactError::InvalidTimestamp(project.created_at.clone()));
        }

        match project.architecture_state {
            ArchitectureState::Draft => {
                if project.current_architecture_version.is_some() {
                    return Err(ArtifactError::InvalidArchitectureState(
                        "Draft project cannot declare an active architecture version".to_string(),
                    ));
                }
            }
            ArchitectureState::Frozen => match project.current_architecture_version {
                Some(ref ver) if !ver.trim().is_empty() => (),
                _ => {
                    return Err(ArtifactError::InvalidArchitectureState(
                        "Frozen project must declare a non-empty current_architecture_version"
                            .to_string(),
                    ));
                }
            },
        }

        Ok(())
    }

    /// Canonicalizes repo root and returns path to .coalition directory, ensuring it does not escape.
    pub fn resolve_coalition_dir<P: AsRef<Path>>(repo_root: P) -> Result<PathBuf, ArtifactError> {
        let canonical_root = repo_root
            .as_ref()
            .canonicalize()
            .map_err(|e| ArtifactError::Io(format!("Failed to canonicalize repo root: {}", e)))?;

        let coalition_dir = canonical_root.join(".coalition");

        if coalition_dir.exists() {
            Self::validate_safe_path(&canonical_root, &coalition_dir)?;
        }

        Ok(coalition_dir)
    }

    pub fn normalize_path(path: &Path) -> PathBuf {
        let s = path.to_string_lossy();
        if let Some(stripped) = s.strip_prefix(r"\\?\") {
            PathBuf::from(stripped)
        } else {
            path.to_path_buf()
        }
    }

    /// Verifies that a path remains inside canonical_root and does not follow symlinks/junctions
    /// that point outside the repository.
    pub fn validate_safe_path(canonical_root: &Path, target: &Path) -> Result<(), ArtifactError> {
        let clean_root = Self::normalize_path(
            &canonical_root
                .canonicalize()
                .unwrap_or_else(|_| canonical_root.to_path_buf()),
        );
        let meta = fs::symlink_metadata(target).map_err(|e| {
            ArtifactError::Io(format!(
                "Failed to inspect metadata for {:?}: {}",
                target, e
            ))
        })?;

        // Check for symlinks or Windows junctions / reparse points
        if meta.file_type().is_symlink() {
            let canonical_target = target.canonicalize().map_err(|e| {
                ArtifactError::Io(format!(
                    "Failed to resolve symlink target {:?}: {}",
                    target, e
                ))
            })?;
            let clean_target = Self::normalize_path(&canonical_target);

            if !clean_target.starts_with(&clean_root) {
                return Err(ArtifactError::UnsafeReparsePoint(format!(
                    "Path {:?} resolves to {:?} outside repository root {:?}",
                    target, clean_target, clean_root
                )));
            }
        }

        let canonical_target = target.canonicalize().map_err(|e| {
            ArtifactError::Io(format!("Failed to canonicalize target {:?}: {}", target, e))
        })?;
        let clean_target = Self::normalize_path(&canonical_target);

        if !clean_target.starts_with(&clean_root) {
            return Err(ArtifactError::PathTraversal {
                path: target.to_string_lossy().to_string(),
                root: canonical_root.to_string_lossy().to_string(),
            });
        }

        Ok(())
    }

    /// Validates the complete .coalition layout, ensuring every standard subdirectory
    /// exists, is safe, does not escape via symlink/reparse point, and that project.yaml is valid.
    pub fn validate_coalition_layout(
        canonical_root: &Path,
        coalition_dir: &Path,
    ) -> Result<(), ArtifactError> {
        Self::validate_safe_path(canonical_root, coalition_dir)?;

        for sub in STANDARD_SUBDIRECTORIES {
            let sub_path = coalition_dir.join(sub);
            if !sub_path.exists() {
                fs::create_dir_all(&sub_path).map_err(|e| {
                    ArtifactError::Io(format!(
                        "Failed to create .coalition/{} directory: {}",
                        sub, e
                    ))
                })?;
            }
            Self::validate_safe_path(canonical_root, &sub_path)?;
        }

        let project_yaml_path = coalition_dir.join("project.yaml");
        if project_yaml_path.exists() {
            Self::validate_safe_path(canonical_root, &project_yaml_path)?;
            let _ = Self::read_project_yaml(&project_yaml_path)?;
        }

        Ok(())
    }

    /// Cleans up any stale project.yaml.tmp.* files left over from crashes or aborted writes.
    pub fn clean_stale_temp_files(coalition_dir: &Path) {
        if let Ok(entries) = fs::read_dir(coalition_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("project.yaml.tmp.") {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
    }

    /// Non-mutating inspection of .coalition artifacts.
    /// Identifies canonical project.yaml, valid backup candidates, presence of temp files,
    /// and whether the directory is in an ambiguous or corrupted recovery state.
    /// Does NOT mutate, promote, or delete any files.
    pub fn inspect_project_artifacts<P: AsRef<Path>>(
        repo_root: P,
    ) -> Result<ProjectArtifactInspection, ArtifactError> {
        let canonical_root = repo_root
            .as_ref()
            .canonicalize()
            .map_err(|e| ArtifactError::Io(format!("Failed to canonicalize repo root: {}", e)))?;

        let coalition_dir = canonical_root.join(".coalition");
        if !coalition_dir.exists() {
            return Ok(ProjectArtifactInspection {
                canonical: None,
                valid_backups: Vec::new(),
                temp_files_present: false,
                ambiguous_or_invalid_recovery_state: false,
            });
        }

        Self::validate_safe_path(&canonical_root, &coalition_dir)?;

        let project_yaml_path = coalition_dir.join("project.yaml");
        let canonical = if project_yaml_path.exists() {
            Self::validate_safe_path(&canonical_root, &project_yaml_path)?;
            Some(Self::read_project_yaml(&project_yaml_path)?)
        } else {
            None
        };

        let mut valid_backups = Vec::new();
        let mut temp_files_present = false;
        let mut ambiguous_or_invalid_recovery_state = false;

        if let Ok(entries) = fs::read_dir(&coalition_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let path = entry.path();
                if name.starts_with("project.yaml.tmp.") {
                    temp_files_present = true;
                } else if name.starts_with("project.yaml.bak.") {
                    if let Ok(()) = Self::validate_safe_path(&canonical_root, &path) {
                        match Self::read_project_yaml(&path) {
                            Ok(project) => {
                                valid_backups.push(RecoveryCandidate { path, project });
                            }
                            Err(_) => {
                                ambiguous_or_invalid_recovery_state = true;
                            }
                        }
                    } else {
                        ambiguous_or_invalid_recovery_state = true;
                    }
                }
            }
        }

        if valid_backups.len() > 1 {
            ambiguous_or_invalid_recovery_state = true;
        }

        Ok(ProjectArtifactInspection {
            canonical,
            valid_backups,
            temp_files_present,
            ambiguous_or_invalid_recovery_state,
        })
    }

    /// Authoritatively promotes an authorized backup to canonical project.yaml.
    pub fn promote_backup_to_canonical(
        backup_path: &Path,
        canonical_path: &Path,
    ) -> Result<(), ArtifactError> {
        Self::replace_file_atomically(backup_path, canonical_path)
    }

    /// Creates .coalition directory, ensures standard subdirectories, and writes a brand new project.yaml.
    /// Errors if project.yaml already exists.
    pub fn initialize_new_project<P: AsRef<Path>>(
        repo_root: P,
        default_name: &str,
    ) -> Result<ProjectYaml, ArtifactError> {
        let canonical_root = repo_root
            .as_ref()
            .canonicalize()
            .map_err(|e| ArtifactError::Io(format!("Failed to canonicalize repo root: {}", e)))?;

        let coalition_dir = canonical_root.join(".coalition");
        let project_yaml_path = coalition_dir.join("project.yaml");

        if project_yaml_path.exists() {
            return Err(ArtifactError::Io(
                "project.yaml already exists; cannot initialize new project".to_string(),
            ));
        }

        fs::create_dir_all(&coalition_dir).map_err(|e| {
            ArtifactError::Io(format!("Failed to create .coalition directory: {}", e))
        })?;

        Self::validate_coalition_layout(&canonical_root, &coalition_dir)?;

        let new_project = ProjectYaml {
            schema_version: CURRENT_SCHEMA_VERSION,
            project_id: Uuid::new_v4().to_string(),
            name: default_name.trim().to_string(),
            current_architecture_version: None,
            architecture_state: ArchitectureState::Draft,
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        Self::write_project_yaml_atomic(&project_yaml_path, &new_project)?;

        Ok(new_project)
    }

    /// Initializes standard durable .coalition layout if not present, and loads project.yaml if already present.
    /// Initializes standard durable .coalition layout if not present, and loads project.yaml if already present.
    pub fn initialize_or_load_project<P: AsRef<Path>>(
        repo_root: P,
        default_name: &str,
    ) -> Result<(ProjectYaml, bool), ArtifactError> {
        let canonical_root = repo_root
            .as_ref()
            .canonicalize()
            .map_err(|e| ArtifactError::Io(format!("Failed to canonicalize repo root: {}", e)))?;

        let coalition_dir = canonical_root.join(".coalition");
        if coalition_dir.exists() {
            Self::validate_safe_path(&canonical_root, &coalition_dir)?;
            let inspection = Self::inspect_project_artifacts(&canonical_root)?;
            if let Some(existing) = inspection.canonical {
                Self::validate_coalition_layout(&canonical_root, &coalition_dir)?;
                return Ok((existing, false));
            }

            // Option A: If recovery artifacts exist and canonical is absent, never generate a new project identity
            if !inspection.valid_backups.is_empty()
                || inspection.temp_files_present
                || inspection.ambiguous_or_invalid_recovery_state
            {
                return Err(ArtifactError::RecoveryRequired(
                    "Recovery artifacts detected in .coalition; cannot initialize new project identity while recovery evidence exists".to_string(),
                ));
            }
        }

        let created = Self::initialize_new_project(&canonical_root, default_name)?;
        Ok((created, true))
    }

    pub fn read_project_yaml<P: AsRef<Path>>(path: P) -> Result<ProjectYaml, ArtifactError> {
        let p = path.as_ref();
        if !p.exists() {
            return Err(ArtifactError::NotFound(p.to_string_lossy().to_string()));
        }

        let content = fs::read_to_string(p)
            .map_err(|e| ArtifactError::Io(format!("Failed to read project.yaml: {}", e)))?;

        let parsed: ProjectYaml = serde_yaml::from_str(&content)
            .map_err(|e| ArtifactError::InvalidYaml(e.to_string()))?;

        Self::validate_project_yaml(&parsed)?;

        Ok(parsed)
    }

    /// Performs genuinely crash-safe atomic file replacement.
    pub fn replace_file_atomically(temp: &Path, destination: &Path) -> Result<(), ArtifactError> {
        #[cfg(test)]
        {
            if INJECTED_REPLACEMENT_FAILURE.with(|f| f.get()) {
                return Err(ArtifactError::Io(
                    "Injected atomic replacement failure for test".to_string(),
                ));
            }
            if INJECTED_SEAM.with(|f| f.get()) == InjectedSeam::PreCallFailure {
                return Err(ArtifactError::Io(
                    "Injected pre-call atomic replacement failure for test".to_string(),
                ));
            }
        }

        replace_file_atomically_impl(temp, destination)
    }

    /// Writes project.yaml using genuinely crash-safe atomic replacement:
    /// 1. Validates the descriptor before any I/O.
    /// 2. Writes to a temporary file in the same directory and flushes/syncs to disk.
    /// 3. Atomically replaces target using platform-native atomic replacement.
    /// 4. If replacement fails with RecoveryRequired, preserves all recovery artifacts intact.
    ///    On ordinary safe failures, cleans up the staged temporary file.
    pub fn write_project_yaml_atomic<P: AsRef<Path>>(
        path: P,
        project: &ProjectYaml,
    ) -> Result<(), ArtifactError> {
        Self::validate_project_yaml(project)?;

        let p = path.as_ref();
        let parent = p.parent().ok_or_else(|| {
            ArtifactError::Io("Target project.yaml path has no parent directory".to_string())
        })?;

        let yaml_str = serde_yaml::to_string(project)
            .map_err(|e| ArtifactError::InvalidYaml(e.to_string()))?;

        let temp_file_name = format!("project.yaml.tmp.{}", Uuid::new_v4());
        let temp_path = parent.join(&temp_file_name);

        // 1. Write to temp file and sync to durable storage
        {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)
                .map_err(|e| {
                    ArtifactError::Io(format!("Failed to create temp file {:?}: {}", temp_path, e))
                })?;

            file.write_all(yaml_str.as_bytes()).map_err(|e| {
                let _ = fs::remove_file(&temp_path);
                ArtifactError::Io(format!("Failed to write to temp file: {}", e))
            })?;

            file.sync_all().map_err(|e| {
                let _ = fs::remove_file(&temp_path);
                ArtifactError::Io(format!("Failed to sync temp file: {}", e))
            })?;
        }

        // 2. Atomically replace target using platform native replacement
        if let Err(e) = Self::replace_file_atomically(&temp_path, p) {
            match &e {
                ArtifactError::RecoveryRequired(_) => {
                    // Crucial: preserve any remaining recognized temp/backup/canonical artifacts
                    // for manual or administrative recovery inspection.
                }
                _ => {
                    // Ordinary known-safe failures: clean up temp file if not already cleaned.
                    let _ = fs::remove_file(&temp_path);
                }
            }
            return Err(e);
        }

        Ok(())
    }

    /// Checks if a relative artifact path belongs to the allowed architecture package.
    pub fn is_valid_architecture_artifact_path(relative_path: &str) -> bool {
        let normalized = relative_path.replace('\\', "/");
        if normalized.starts_with('/') || normalized.contains("..") {
            return false;
        }

        if CANONICAL_ARCHITECTURE_ARTIFACTS.contains(&normalized.as_str()) {
            return true;
        }

        if normalized.starts_with("decisions/ADR-") && normalized.ends_with(".md") {
            return true;
        }

        false
    }

    /// Writes an architecture artifact atomically to `.coalition/<relative_path>`.
    /// Enforces strict path allowlist and repository boundary checks.
    pub fn write_artifact_atomic<P: AsRef<Path>>(
        repo_root: P,
        relative_path: &str,
        content: &str,
    ) -> Result<(), ArtifactError> {
        let normalized = relative_path.replace('\\', "/");
        if !Self::is_valid_architecture_artifact_path(&normalized) {
            return Err(ArtifactError::PathTraversal {
                path: relative_path.to_string(),
                root: repo_root.as_ref().to_string_lossy().to_string(),
            });
        }

        let coalition_dir = Self::resolve_coalition_dir(repo_root.as_ref())?;
        let target_path = coalition_dir.join(&normalized);

        let parent = target_path.parent().ok_or_else(|| {
            ArtifactError::Io("Target artifact path has no parent directory".to_string())
        })?;

        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| {
                ArtifactError::Io(format!("Failed to create directory {:?}: {}", parent, e))
            })?;
        }

        Self::validate_safe_path(repo_root.as_ref(), parent)?;

        let file_name = target_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("artifact");
        let temp_file_name = format!("{}.tmp.{}", file_name, Uuid::new_v4());
        let temp_path = parent.join(&temp_file_name);

        {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)
                .map_err(|e| {
                    ArtifactError::Io(format!("Failed to create temp file {:?}: {}", temp_path, e))
                })?;

            file.write_all(content.as_bytes()).map_err(|e| {
                let _ = fs::remove_file(&temp_path);
                ArtifactError::Io(format!("Failed to write to temp file: {}", e))
            })?;

            file.sync_all().map_err(|e| {
                let _ = fs::remove_file(&temp_path);
                ArtifactError::Io(format!("Failed to sync temp file: {}", e))
            })?;
        }

        if let Err(e) = Self::replace_file_atomically(&temp_path, &target_path) {
            match &e {
                ArtifactError::RecoveryRequired(_) => {}
                _ => {
                    let _ = fs::remove_file(&temp_path);
                }
            }
            return Err(e);
        }

        Self::validate_safe_path(repo_root.as_ref(), &target_path)?;

        Ok(())
    }

    /// Reads an architecture artifact from `.coalition/<relative_path>` if it exists.
    pub fn read_artifact<P: AsRef<Path>>(
        repo_root: P,
        relative_path: &str,
    ) -> Result<Option<String>, ArtifactError> {
        let normalized = relative_path.replace('\\', "/");
        if !Self::is_valid_architecture_artifact_path(&normalized) {
            return Err(ArtifactError::PathTraversal {
                path: relative_path.to_string(),
                root: repo_root.as_ref().to_string_lossy().to_string(),
            });
        }

        let coalition_dir = Self::resolve_coalition_dir(repo_root.as_ref())?;
        let target_path = coalition_dir.join(&normalized);

        if !target_path.exists() {
            return Ok(None);
        }

        Self::validate_safe_path(repo_root.as_ref(), &target_path)?;

        let content = fs::read_to_string(&target_path).map_err(|e| {
            ArtifactError::Io(format!("Failed to read artifact {:?}: {}", target_path, e))
        })?;

        Ok(Some(content))
    }

    /// Deletes an architecture artifact from `.coalition/<relative_path>` if it exists.
    pub fn delete_artifact<P: AsRef<Path>>(
        repo_root: P,
        relative_path: &str,
    ) -> Result<bool, ArtifactError> {
        let normalized = relative_path.replace('\\', "/");
        if !Self::is_valid_architecture_artifact_path(&normalized) {
            return Err(ArtifactError::PathTraversal {
                path: relative_path.to_string(),
                root: repo_root.as_ref().to_string_lossy().to_string(),
            });
        }

        let coalition_dir = Self::resolve_coalition_dir(repo_root.as_ref())?;
        let target_path = coalition_dir.join(&normalized);

        if !target_path.exists() {
            return Ok(false);
        }

        Self::validate_safe_path(repo_root.as_ref(), &target_path)?;
        fs::remove_file(&target_path).map_err(|e| {
            ArtifactError::Io(format!(
                "Failed to delete artifact {:?}: {}",
                target_path, e
            ))
        })?;

        Ok(true)
    }

    /// Computes SHA-256 fingerprint hex of an architecture artifact, or returns None if file does not exist.
    pub fn compute_file_fingerprint<P: AsRef<Path>>(
        repo_root: P,
        relative_path: &str,
    ) -> Result<Option<String>, ArtifactError> {
        let normalized = relative_path.replace('\\', "/");
        if !Self::is_valid_architecture_artifact_path(&normalized) {
            return Err(ArtifactError::PathTraversal {
                path: relative_path.to_string(),
                root: repo_root.as_ref().to_string_lossy().to_string(),
            });
        }

        let coalition_dir = Self::resolve_coalition_dir(repo_root.as_ref())?;
        let target_path = coalition_dir.join(&normalized);

        if !target_path.exists() {
            return Ok(None);
        }

        Self::validate_safe_path(repo_root.as_ref(), &target_path)?;
        let bytes = fs::read(&target_path).map_err(|e| {
            ArtifactError::Io(format!(
                "Failed to read artifact {:?} for fingerprint: {}",
                target_path, e
            ))
        })?;

        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let hash = format!("{:x}", hasher.finalize());
        Ok(Some(hash))
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InjectedSeam {
    None,
    PreCallFailure,
    SimulateAmbiguousRecovery,
    SimulateReplaceFileWError(u32),
    SimulateMoveFileExWError(u32),
}

#[cfg(test)]
thread_local! {
    pub static INJECTED_SEAM: std::cell::Cell<InjectedSeam> = const { std::cell::Cell::new(InjectedSeam::None) };
    pub static INJECTED_REPLACEMENT_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(windows)]
fn replace_file_atomically_impl(temp: &Path, destination: &Path) -> Result<(), ArtifactError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, ReplaceFileW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let temp_wide: Vec<u16> = temp
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let dest_wide: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let destination_existed_at_start = destination.exists();

    if destination_existed_at_start {
        let parent = destination.parent().ok_or_else(|| {
            ArtifactError::Io("Destination path has no parent directory".to_string())
        })?;
        let is_project_yaml = destination
            .file_name()
            .map(|n| n == "project.yaml")
            .unwrap_or(false);
        let dest_name = destination
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file");
        let backup_path = parent.join(format!("{}.bak.{}", dest_name, Uuid::new_v4()));
        let backup_wide: Vec<u16> = backup_path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        #[cfg(test)]
        let (res, err) = match INJECTED_SEAM.with(|f| f.get()) {
            InjectedSeam::SimulateAmbiguousRecovery => {
                // Simulate an interrupted/ambiguous crash where destination was unlinked
                // but no valid backup was committed.
                let _ = fs::remove_file(destination);
                (0, 31) // ERROR_GEN_FAILURE
            }
            InjectedSeam::SimulateReplaceFileWError(code) => (0, code),
            _ => {
                let r = unsafe {
                    ReplaceFileW(
                        dest_wide.as_ptr(),
                        temp_wide.as_ptr(),
                        backup_wide.as_ptr(),
                        0, // dwReplaceFlags = 0 (REPLACEFILE_WRITE_THROUGH is unsupported for ReplaceFileW)
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    )
                };
                let e = if r == 0 { unsafe { GetLastError() } } else { 0 };
                (r, e)
            }
        };

        #[cfg(not(test))]
        let (res, err) = {
            let r = unsafe {
                ReplaceFileW(
                    dest_wide.as_ptr(),
                    temp_wide.as_ptr(),
                    backup_wide.as_ptr(),
                    0, // dwReplaceFlags = 0 (REPLACEFILE_WRITE_THROUGH is unsupported for ReplaceFileW)
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            let e = if r == 0 { unsafe { GetLastError() } } else { 0 };
            (r, e)
        };

        if res != 0 {
            // ReplaceFileW succeeded from the OS perspective.
            // If destination is project.yaml, validate that the destination contains a valid project descriptor before cleaning backup.
            if is_project_yaml {
                match ArtifactManager::read_project_yaml(destination) {
                    Ok(_) => {
                        // Valid! Safely remove the generated backup.
                        let _ = fs::remove_file(&backup_path);
                        return Ok(());
                    }
                    Err(read_err) => {
                        // Canonical file validation failed after ReplaceFileW.
                        // If backup exists, attempt to restore it before reporting error.
                        if backup_path.exists() {
                            let _ = unsafe {
                                MoveFileExW(
                                    backup_wide.as_ptr(),
                                    dest_wide.as_ptr(),
                                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                                )
                            };
                        }
                        return Err(ArtifactError::RecoveryRequired(format!(
                            "ReplaceFileW succeeded but destination validation failed: {}. Preserved recovery artifacts.",
                            read_err
                        )));
                    }
                }
            } else {
                // For non-project.yaml artifacts, verify destination exists before removing backup.
                if destination.exists() {
                    let _ = fs::remove_file(&backup_path);
                    return Ok(());
                } else {
                    return Err(ArtifactError::Io(format!(
                        "ReplaceFileW reported success but destination {:?} does not exist",
                        destination
                    )));
                }
            }
        }

        // ReplaceFileW failed. Inspect actual filesystem state.
        // Crucial: do NOT fall through into MoveFileExW creation path!
        let dest_exists = destination.exists();
        let backup_exists = backup_path.exists();

        if !dest_exists && backup_exists {
            // The destination was moved/renamed to backup before failure occurred!
            // Restore destination from backup.
            let restore_res = unsafe {
                MoveFileExW(
                    backup_wide.as_ptr(),
                    dest_wide.as_ptr(),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            };
            if restore_res != 0 && ArtifactManager::read_project_yaml(destination).is_ok() {
                let _ = fs::remove_file(temp);
                return Err(ArtifactError::Io(format!(
                    "ReplaceFileW failed (OS error {}). Destination was restored from backup.",
                    err
                )));
            } else {
                return Err(ArtifactError::RecoveryRequired(format!(
                    "ReplaceFileW failed (OS error {}). Destination lost and backup restore failed. Manual recovery required.",
                    err
                )));
            }
        } else if dest_exists && ArtifactManager::read_project_yaml(destination).is_ok() {
            // Destination is still intact and valid.
            let _ = fs::remove_file(temp);
            if backup_exists {
                let _ = fs::remove_file(&backup_path);
            }
            return Err(ArtifactError::Io(format!(
                "ReplaceFileW failed with OS error code {}. Original destination remains intact.",
                err
            )));
        } else {
            // Ambiguous or corrupted state (e.g. destination missing and backup missing, or destination corrupted)
            return Err(ArtifactError::RecoveryRequired(format!(
                "ReplaceFileW failed with OS error code {}. Filesystem in ambiguous recovery state.",
                err
            )));
        }
    }

    // Destination was genuinely absent at the start of the operation: use MoveFileExW creation path
    #[cfg(test)]
    let (res, err) = match INJECTED_SEAM.with(|f| f.get()) {
        InjectedSeam::SimulateMoveFileExWError(code) => (0, code),
        _ => {
            let r = unsafe {
                MoveFileExW(
                    temp_wide.as_ptr(),
                    dest_wide.as_ptr(),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            };
            let e = if r == 0 { unsafe { GetLastError() } } else { 0 };
            (r, e)
        }
    };

    #[cfg(not(test))]
    let (res, err) = {
        let r = unsafe {
            MoveFileExW(
                temp_wide.as_ptr(),
                dest_wide.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        let e = if r == 0 { unsafe { GetLastError() } } else { 0 };
        (r, e)
    };

    if res != 0 {
        Ok(())
    } else {
        // Crucial: NEVER delete source `temp` here; callers own source argument cleanup!
        Err(ArtifactError::Io(format!(
            "MoveFileExW creation failed with OS error code {}",
            err
        )))
    }
}

#[cfg(not(windows))]
fn replace_file_atomically_impl(temp: &Path, destination: &Path) -> Result<(), ArtifactError> {
    #[cfg(test)]
    {
        if INJECTED_SEAM.with(|f| f.get()) == InjectedSeam::SimulateAmbiguousRecovery {
            let _ = fs::remove_file(destination);
            return Err(ArtifactError::RecoveryRequired(
                "Simulated ambiguous recovery state on POSIX".to_string(),
            ));
        }
        if let InjectedSeam::SimulateMoveFileExWError(code) = INJECTED_SEAM.with(|f| f.get()) {
            return Err(ArtifactError::Io(format!(
                "Simulated MoveFileExW creation failure on POSIX with code {}",
                code
            )));
        }
    }

    fs::rename(temp, destination)
        .map_err(|e| ArtifactError::Io(format!("rename failed: {}", e)))?;
    if let Some(parent) = destination.parent() {
        if let Ok(dir) = fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_initialize_and_read_project_yaml() {
        let dir = tempdir().unwrap();
        let (project, created) =
            ArtifactManager::initialize_or_load_project(dir.path(), "my-test-proj").unwrap();

        assert!(created);
        assert_eq!(project.schema_version, 1);
        assert_eq!(project.name, "my-test-proj");
        assert_eq!(project.architecture_state, ArchitectureState::Draft);
        assert!(project.current_architecture_version.is_none());
        assert!(!project.project_id.is_empty());

        let coalition = dir.path().join(".coalition");
        assert!(coalition.is_dir());
        assert!(coalition.join("project.yaml").is_file());
        assert!(coalition.join("design").is_dir());
        assert!(coalition.join("implementation").is_dir());
        assert!(coalition.join("decisions").is_dir());
        assert!(coalition.join("architecture-versions").is_dir());
        assert!(coalition.join("changes").is_dir());
        assert!(coalition.join("reviews").is_dir());
        assert!(coalition.join("evidence").is_dir());

        // Reopen existing
        let (loaded, created_again) =
            ArtifactManager::initialize_or_load_project(dir.path(), "different-name").unwrap();
        assert!(!created_again);
        assert_eq!(loaded.project_id, project.project_id);
        assert_eq!(loaded.name, "my-test-proj");
    }

    #[test]
    fn test_successful_safe_replacement() {
        let dir = tempdir().unwrap();
        let (mut project, _) =
            ArtifactManager::initialize_or_load_project(dir.path(), "initial-name").unwrap();
        let yaml_path = dir.path().join(".coalition").join("project.yaml");

        project.architecture_state = ArchitectureState::Frozen;
        project.current_architecture_version = Some("1.0".to_string());

        ArtifactManager::write_project_yaml_atomic(&yaml_path, &project).unwrap();

        let reloaded = ArtifactManager::read_project_yaml(&yaml_path).unwrap();
        assert_eq!(reloaded.architecture_state, ArchitectureState::Frozen);
        assert_eq!(
            reloaded.current_architecture_version,
            Some("1.0".to_string())
        );

        // Ensure no temporary or backup files remained
        let entries: Vec<_> = fs::read_dir(dir.path().join(".coalition"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(!entries
            .iter()
            .any(|f| f.contains(".tmp.") || f.contains(".bak.")));
    }

    #[test]
    fn test_failed_validation_preserves_existing_file() {
        let dir = tempdir().unwrap();
        let (project, _) =
            ArtifactManager::initialize_or_load_project(dir.path(), "valid-proj").unwrap();
        let yaml_path = dir.path().join(".coalition").join("project.yaml");

        // Create invalid descriptor: frozen without version
        let invalid_project = ProjectYaml {
            schema_version: 1,
            project_id: project.project_id.clone(),
            name: "valid-proj".to_string(),
            current_architecture_version: None,
            architecture_state: ArchitectureState::Frozen,
            created_at: project.created_at.clone(),
        };

        let err =
            ArtifactManager::write_project_yaml_atomic(&yaml_path, &invalid_project).unwrap_err();
        assert!(matches!(err, ArtifactError::InvalidArchitectureState(_)));

        // Original file must remain intact and valid
        let intact = ArtifactManager::read_project_yaml(&yaml_path).unwrap();
        assert_eq!(intact.architecture_state, ArchitectureState::Draft);
    }

    #[test]
    fn test_rejects_schema_version_0_and_future_versions() {
        let dir = tempdir().unwrap();
        let coalition = dir.path().join(".coalition");
        fs::create_dir_all(&coalition).unwrap();

        // Version 0
        let v0_yaml = "schema_version: 0\nproject_id: 'a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d'\nname: 'V0 Proj'\narchitecture_state: draft\ncreated_at: '2026-09-08T00:00:00Z'\n";
        fs::write(coalition.join("project.yaml"), v0_yaml).unwrap();
        let err = ArtifactManager::read_project_yaml(coalition.join("project.yaml")).unwrap_err();
        assert_eq!(err, ArtifactError::UnsupportedSchemaVersion(0));

        // Version 99
        let v99_yaml = "schema_version: 99\nproject_id: 'a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d'\nname: 'V99 Proj'\narchitecture_state: draft\ncreated_at: '2026-09-08T00:00:00Z'\n";
        fs::write(coalition.join("project.yaml"), v99_yaml).unwrap();
        let err2 = ArtifactManager::read_project_yaml(coalition.join("project.yaml")).unwrap_err();
        assert_eq!(err2, ArtifactError::UnsupportedSchemaVersion(99));
    }

    #[test]
    fn test_rejects_invalid_project_id() {
        let dir = tempdir().unwrap();
        let coalition = dir.path().join(".coalition");
        fs::create_dir_all(&coalition).unwrap();

        let bad_id_yaml = "schema_version: 1\nproject_id: 'not-a-valid-uuid'\nname: 'Bad ID Proj'\narchitecture_state: draft\ncreated_at: '2026-09-08T00:00:00Z'\n";
        fs::write(coalition.join("project.yaml"), bad_id_yaml).unwrap();
        let err = ArtifactManager::read_project_yaml(coalition.join("project.yaml")).unwrap_err();
        assert!(matches!(err, ArtifactError::InvalidProjectId(_)));
    }

    #[test]
    fn test_rejects_empty_project_name() {
        let dir = tempdir().unwrap();
        let coalition = dir.path().join(".coalition");
        fs::create_dir_all(&coalition).unwrap();

        let empty_name_yaml = "schema_version: 1\nproject_id: 'a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d'\nname: '   '\narchitecture_state: draft\ncreated_at: '2026-09-08T00:00:00Z'\n";
        fs::write(coalition.join("project.yaml"), empty_name_yaml).unwrap();
        let err = ArtifactManager::read_project_yaml(coalition.join("project.yaml")).unwrap_err();
        assert_eq!(err, ArtifactError::EmptyProjectName);
    }

    #[test]
    fn test_rejects_invalid_timestamp() {
        let dir = tempdir().unwrap();
        let coalition = dir.path().join(".coalition");
        fs::create_dir_all(&coalition).unwrap();

        let bad_ts_yaml = "schema_version: 1\nproject_id: 'a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d'\nname: 'Proj'\narchitecture_state: draft\ncreated_at: 'yesterday'\n";
        fs::write(coalition.join("project.yaml"), bad_ts_yaml).unwrap();
        let err = ArtifactManager::read_project_yaml(coalition.join("project.yaml")).unwrap_err();
        assert!(matches!(err, ArtifactError::InvalidTimestamp(_)));
    }

    #[test]
    fn test_rejects_draft_with_architecture_version() {
        let dir = tempdir().unwrap();
        let coalition = dir.path().join(".coalition");
        fs::create_dir_all(&coalition).unwrap();

        let draft_with_ver = "schema_version: 1\nproject_id: 'a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d'\nname: 'Proj'\narchitecture_state: draft\ncurrent_architecture_version: '1.0'\ncreated_at: '2026-09-08T00:00:00Z'\n";
        fs::write(coalition.join("project.yaml"), draft_with_ver).unwrap();
        let err = ArtifactManager::read_project_yaml(coalition.join("project.yaml")).unwrap_err();
        assert!(matches!(err, ArtifactError::InvalidArchitectureState(_)));

        // Empty string version in draft must also be rejected
        let draft_with_empty_ver = "schema_version: 1\nproject_id: 'a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d'\nname: 'Proj'\narchitecture_state: draft\ncurrent_architecture_version: ''\ncreated_at: '2026-09-08T00:00:00Z'\n";
        fs::write(coalition.join("project.yaml"), draft_with_empty_ver).unwrap();
        let err2 = ArtifactManager::read_project_yaml(coalition.join("project.yaml")).unwrap_err();
        assert!(matches!(err2, ArtifactError::InvalidArchitectureState(_)));
    }

    #[test]
    fn test_rejects_frozen_without_architecture_version() {
        let dir = tempdir().unwrap();
        let coalition = dir.path().join(".coalition");
        fs::create_dir_all(&coalition).unwrap();

        let frozen_no_ver = "schema_version: 1\nproject_id: 'a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d'\nname: 'Proj'\narchitecture_state: frozen\ncreated_at: '2026-09-08T00:00:00Z'\n";
        fs::write(coalition.join("project.yaml"), frozen_no_ver).unwrap();
        let err = ArtifactManager::read_project_yaml(coalition.join("project.yaml")).unwrap_err();
        assert!(matches!(err, ArtifactError::InvalidArchitectureState(_)));
    }

    #[test]
    fn test_accepts_valid_frozen_descriptor() {
        let dir = tempdir().unwrap();
        let coalition = dir.path().join(".coalition");
        fs::create_dir_all(&coalition).unwrap();

        let valid_frozen = "schema_version: 1\nproject_id: 'a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d'\nname: 'Proj'\narchitecture_state: frozen\ncurrent_architecture_version: '1.0'\ncreated_at: '2026-09-08T00:00:00Z'\n";
        fs::write(coalition.join("project.yaml"), valid_frozen).unwrap();
        let project = ArtifactManager::read_project_yaml(coalition.join("project.yaml")).unwrap();
        assert_eq!(project.architecture_state, ArchitectureState::Frozen);
        assert_eq!(
            project.current_architecture_version,
            Some("1.0".to_string())
        );
    }

    #[test]
    fn test_uuid_v4_validation() {
        let make_project = |id: &str| ProjectYaml {
            schema_version: 1,
            project_id: id.to_string(),
            name: "uuid-test".to_string(),
            current_architecture_version: None,
            architecture_state: ArchitectureState::Draft,
            created_at: "2026-09-08T00:00:00Z".to_string(),
        };

        // Valid UUID v4
        let valid_v4 = make_project("a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d");
        assert!(ArtifactManager::validate_project_yaml(&valid_v4).is_ok());

        // Generated v4
        let generated_v4 = make_project(&Uuid::new_v4().to_string());
        assert!(ArtifactManager::validate_project_yaml(&generated_v4).is_ok());

        // Nil UUID (rejected)
        let nil_uuid = make_project("00000000-0000-0000-0000-000000000000");
        let err_nil = ArtifactManager::validate_project_yaml(&nil_uuid).unwrap_err();
        assert!(matches!(err_nil, ArtifactError::InvalidProjectId(_)));

        // UUID v1 (rejected)
        let v1_uuid = make_project("6ba7b810-9dad-11d1-80b4-00c04fd430c8");
        let err_v1 = ArtifactManager::validate_project_yaml(&v1_uuid).unwrap_err();
        assert!(matches!(err_v1, ArtifactError::InvalidProjectId(_)));

        // UUID v3 (rejected)
        let v3_uuid = make_project("6ba7b811-9dad-31d1-80b4-00c04fd430c8");
        let err_v3 = ArtifactManager::validate_project_yaml(&v3_uuid).unwrap_err();
        assert!(matches!(err_v3, ArtifactError::InvalidProjectId(_)));

        // UUID v5 (rejected)
        let v5_uuid = make_project("6ba7b811-9dad-51d1-80b4-00c04fd430c8");
        let err_v5 = ArtifactManager::validate_project_yaml(&v5_uuid).unwrap_err();
        assert!(matches!(err_v5, ArtifactError::InvalidProjectId(_)));

        // Non-UUID string (rejected)
        let malformed = make_project("not-even-a-uuid");
        let err_malformed = ArtifactManager::validate_project_yaml(&malformed).unwrap_err();
        assert!(matches!(err_malformed, ArtifactError::InvalidProjectId(_)));
    }

    #[test]
    fn test_draft_architecture_version_exact_invariant() {
        let mut proj = ProjectYaml {
            schema_version: 1,
            project_id: Uuid::new_v4().to_string(),
            name: "draft-inv-test".to_string(),
            current_architecture_version: None,
            architecture_state: ArchitectureState::Draft,
            created_at: "2026-09-08T00:00:00Z".to_string(),
        };

        // Draft + None => valid
        assert!(ArtifactManager::validate_project_yaml(&proj).is_ok());

        // Draft + Some("") => rejected
        proj.current_architecture_version = Some("".to_string());
        assert!(matches!(
            ArtifactManager::validate_project_yaml(&proj).unwrap_err(),
            ArtifactError::InvalidArchitectureState(_)
        ));

        // Draft + Some("   ") => rejected
        proj.current_architecture_version = Some("   ".to_string());
        assert!(matches!(
            ArtifactManager::validate_project_yaml(&proj).unwrap_err(),
            ArtifactError::InvalidArchitectureState(_)
        ));

        // Draft + Some("1.0") => rejected
        proj.current_architecture_version = Some("1.0".to_string());
        assert!(matches!(
            ArtifactManager::validate_project_yaml(&proj).unwrap_err(),
            ArtifactError::InvalidArchitectureState(_)
        ));

        // Frozen + None => rejected
        proj.architecture_state = ArchitectureState::Frozen;
        proj.current_architecture_version = None;
        assert!(matches!(
            ArtifactManager::validate_project_yaml(&proj).unwrap_err(),
            ArtifactError::InvalidArchitectureState(_)
        ));

        // Frozen + Some("") => rejected
        proj.current_architecture_version = Some("".to_string());
        assert!(matches!(
            ArtifactManager::validate_project_yaml(&proj).unwrap_err(),
            ArtifactError::InvalidArchitectureState(_)
        ));

        // Frozen + Some("   ") => rejected
        proj.current_architecture_version = Some("   ".to_string());
        assert!(matches!(
            ArtifactManager::validate_project_yaml(&proj).unwrap_err(),
            ArtifactError::InvalidArchitectureState(_)
        ));

        // Frozen + Some("1.0") => valid
        proj.current_architecture_version = Some("1.0".to_string());
        assert!(ArtifactManager::validate_project_yaml(&proj).is_ok());
    }

    #[test]
    fn test_replacement_failure_preserves_old_file_authoritatively() {
        let dir = tempdir().unwrap();
        let (original_proj, _) =
            ArtifactManager::initialize_or_load_project(dir.path(), "preserve-me").unwrap();
        let yaml_path = dir.path().join(".coalition").join("project.yaml");

        let mut updated = original_proj.clone();
        updated.name = "mutated-name".to_string();

        // Inject replacement failure
        INJECTED_REPLACEMENT_FAILURE.with(|f| f.set(true));

        let err = ArtifactManager::write_project_yaml_atomic(&yaml_path, &updated).unwrap_err();
        assert!(matches!(err, ArtifactError::Io(_)));

        // Reset injection flag
        INJECTED_REPLACEMENT_FAILURE.with(|f| f.set(false));

        // Verify the original file on disk is completely intact and unaltered
        let current_on_disk = ArtifactManager::read_project_yaml(&yaml_path).unwrap();
        assert_eq!(current_on_disk.name, "preserve-me");
        assert_eq!(current_on_disk.project_id, original_proj.project_id);

        // Verify no leftover temp files
        let entries: Vec<_> = fs::read_dir(dir.path().join(".coalition"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(!entries.iter().any(|f| f.contains(".tmp.")));
    }

    #[test]
    fn test_stale_temp_file_cleanup() {
        let dir = tempdir().unwrap();
        let (project, _) =
            ArtifactManager::initialize_or_load_project(dir.path(), "cleanup-test").unwrap();
        let coalition = dir.path().join(".coalition");

        // Plant stale temp file
        let stale_temp = coalition.join("project.yaml.tmp.old-crash-file");
        fs::write(&stale_temp, "temporary stale data").unwrap();
        assert!(stale_temp.exists());

        // Calling inspect_project_artifacts detects temp file without mutating
        let inspection = ArtifactManager::inspect_project_artifacts(dir.path()).unwrap();
        assert_eq!(
            inspection.canonical.as_ref().unwrap().project_id,
            project.project_id
        );
        assert!(inspection.temp_files_present);
        assert!(stale_temp.exists(), "Inspection must be non-mutating");

        // Clean stale temp files
        ArtifactManager::clean_stale_temp_files(&coalition);
        assert!(!stale_temp.exists(), "stale temp file must be cleaned up");
    }

    #[test]
    fn test_single_valid_backup_inspection_and_promotion() {
        let dir = tempdir().unwrap();
        let coalition = dir.path().join(".coalition");
        fs::create_dir_all(&coalition).unwrap();

        let backup_project = ProjectYaml {
            schema_version: 1,
            project_id: Uuid::new_v4().to_string(),
            name: "recovered-project".to_string(),
            current_architecture_version: None,
            architecture_state: ArchitectureState::Draft,
            created_at: "2026-09-08T00:00:00Z".to_string(),
        };

        let backup_file = coalition.join("project.yaml.bak.20260908");
        let backup_yaml = serde_yaml::to_string(&backup_project).unwrap();
        fs::write(&backup_file, backup_yaml).unwrap();

        let project_yaml_path = coalition.join("project.yaml");
        assert!(!project_yaml_path.exists());

        // inspect_project_artifacts identifies the single valid backup WITHOUT mutating
        let inspection = ArtifactManager::inspect_project_artifacts(dir.path()).unwrap();
        assert!(inspection.canonical.is_none());
        assert_eq!(inspection.valid_backups.len(), 1);
        assert_eq!(
            inspection.valid_backups[0].project.project_id,
            backup_project.project_id
        );
        assert!(!inspection.ambiguous_or_invalid_recovery_state);
        // Canonical project.yaml must NOT exist yet (inspection is non-mutating)
        assert!(!project_yaml_path.exists());
        assert!(backup_file.exists());

        // Now perform authorized promotion
        ArtifactManager::promote_backup_to_canonical(&backup_file, &project_yaml_path).unwrap();

        // Canonical project.yaml must now exist on disk
        assert!(project_yaml_path.exists());
        let on_disk = ArtifactManager::read_project_yaml(&project_yaml_path).unwrap();
        assert_eq!(on_disk.project_id, backup_project.project_id);
    }

    #[test]
    fn test_ambiguous_backups_require_recovery() {
        let dir = tempdir().unwrap();
        let coalition = dir.path().join(".coalition");
        fs::create_dir_all(&coalition).unwrap();

        let proj1 = ProjectYaml {
            schema_version: 1,
            project_id: Uuid::new_v4().to_string(),
            name: "backup-1".to_string(),
            current_architecture_version: None,
            architecture_state: ArchitectureState::Draft,
            created_at: "2026-09-08T00:00:00Z".to_string(),
        };
        let proj2 = ProjectYaml {
            schema_version: 1,
            project_id: Uuid::new_v4().to_string(),
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

        let inspection = ArtifactManager::inspect_project_artifacts(dir.path()).unwrap();
        assert!(inspection.canonical.is_none());
        assert!(inspection.ambiguous_or_invalid_recovery_state);

        // Neither backup should be deleted
        assert!(coalition.join("project.yaml.bak.1").exists());
        assert!(coalition.join("project.yaml.bak.2").exists());
        assert!(!coalition.join("project.yaml").exists());
    }

    #[cfg(windows)]
    #[test]
    fn test_windows_native_replace_file_w_success() {
        let dir = tempdir().unwrap();
        let canonical_root = dir.path().canonicalize().unwrap();
        let coalition = canonical_root.join(".coalition");
        fs::create_dir_all(&coalition).unwrap();

        let initial_project = ProjectYaml {
            schema_version: 1,
            project_id: Uuid::new_v4().to_string(),
            name: "initial-replace-w".to_string(),
            current_architecture_version: None,
            architecture_state: ArchitectureState::Draft,
            created_at: "2026-09-08T00:00:00Z".to_string(),
        };
        let project_yaml_path = coalition.join("project.yaml");
        fs::write(
            &project_yaml_path,
            serde_yaml::to_string(&initial_project).unwrap(),
        )
        .unwrap();

        let mut updated_project = initial_project.clone();
        updated_project.name = "updated-replace-w".to_string();

        // Exercise the actual ReplaceFileW path (destination exists, flags = 0, backup path used)
        ArtifactManager::write_project_yaml_atomic(&project_yaml_path, &updated_project).unwrap();

        // 1. Verify destination replaced with new content
        let read_back = ArtifactManager::read_project_yaml(&project_yaml_path).unwrap();
        assert_eq!(read_back.name, "updated-replace-w");
        assert_eq!(read_back.project_id, initial_project.project_id);

        // 2. Verify no stale generated backup or temp files remain after confirmed success
        let entries: Vec<_> = fs::read_dir(&coalition)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(
            !entries.iter().any(|f| f.starts_with("project.yaml.bak.")),
            "Generated backup must be cleaned up on confirmed success: found {:?}",
            entries
        );
        assert!(
            !entries.iter().any(|f| f.starts_with("project.yaml.tmp.")),
            "Temporary file must be cleaned up on confirmed success: found {:?}",
            entries
        );
    }

    #[test]
    fn test_layout_validation_checks_subdirectories() {
        let dir = tempdir().unwrap();
        let canonical_root = dir.path().canonicalize().unwrap();
        let coalition = canonical_root.join(".coalition");
        fs::create_dir_all(&coalition).unwrap();

        // Valid layout created
        ArtifactManager::validate_coalition_layout(&canonical_root, &coalition).unwrap();

        for sub in STANDARD_SUBDIRECTORIES {
            assert!(coalition.join(sub).is_dir());
        }

        // Place a symlink inside .coalition pointing outside root
        let outside_dir = tempdir().unwrap();
        let link_path = coalition.join("decisions_link");

        #[cfg(windows)]
        {
            let _ = std::os::windows::fs::symlink_dir(outside_dir.path(), &link_path);
        }
        #[cfg(unix)]
        {
            let _ = std::os::unix::fs::symlink(outside_dir.path(), &link_path);
        }

        if link_path.exists() {
            let err = ArtifactManager::validate_safe_path(&canonical_root, &link_path).unwrap_err();
            assert!(matches!(
                err,
                ArtifactError::UnsafeReparsePoint(_) | ArtifactError::PathTraversal { .. }
            ));
        }
    }

    #[test]
    fn test_rejects_malformed_yaml() {
        let dir = tempdir().unwrap();
        let coalition = dir.path().join(".coalition");
        fs::create_dir_all(&coalition).unwrap();

        let broken_yaml = "schema_version: [broken yaml: : :";
        fs::write(coalition.join("project.yaml"), broken_yaml).unwrap();

        let err = ArtifactManager::read_project_yaml(coalition.join("project.yaml")).unwrap_err();
        assert!(matches!(err, ArtifactError::InvalidYaml(_)));
    }

    #[test]
    fn test_no_absolute_paths_in_project_yaml() {
        let dir = tempdir().unwrap();
        let (project, _) =
            ArtifactManager::initialize_or_load_project(dir.path(), "path-test").unwrap();
        let serialized = serde_yaml::to_string(&project).unwrap();

        let dir_str = dir.path().to_string_lossy().to_string();
        assert!(
            !serialized.contains(&dir_str),
            "project.yaml must never contain local absolute paths"
        );
    }

    #[test]
    fn test_ambiguous_replacement_preserves_temp_file() {
        let dir = tempdir().unwrap();
        let canonical_root = dir.path().canonicalize().unwrap();
        let coalition = canonical_root.join(".coalition");
        fs::create_dir_all(&coalition).unwrap();

        let initial_project = ProjectYaml {
            schema_version: 1,
            project_id: Uuid::new_v4().to_string(),
            name: "initial-ambiguous-test".to_string(),
            current_architecture_version: None,
            architecture_state: ArchitectureState::Draft,
            created_at: "2026-09-08T00:00:00Z".to_string(),
        };
        let project_yaml_path = coalition.join("project.yaml");
        fs::write(
            &project_yaml_path,
            serde_yaml::to_string(&initial_project).unwrap(),
        )
        .unwrap();

        let mut updated = initial_project.clone();
        updated.name = "mutated-ambiguous-name".to_string();

        // Force ambiguous recovery condition via seam
        INJECTED_SEAM.with(|f| f.set(InjectedSeam::SimulateAmbiguousRecovery));

        let err =
            ArtifactManager::write_project_yaml_atomic(&project_yaml_path, &updated).unwrap_err();
        assert!(
            matches!(err, ArtifactError::RecoveryRequired(_)),
            "Ambiguous replacement must return RecoveryRequired error, got: {:?}",
            err
        );

        // Reset injection seam
        INJECTED_SEAM.with(|f| f.set(InjectedSeam::None));

        // Distinct from pre-call failure (which removes temp files),
        // ambiguous recovery MUST preserve the staged temporary file for recovery inspection!
        let entries: Vec<_> = fs::read_dir(&coalition)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        let temp_found = entries.iter().any(|f| f.starts_with("project.yaml.tmp."));
        assert!(
            temp_found,
            "Ambiguous replacement failure must preserve staged temporary file: found {:?}",
            entries
        );

        // Non-mutating inspection must detect temp_files_present
        let inspection = ArtifactManager::inspect_project_artifacts(&canonical_root).unwrap();
        assert!(
            inspection.temp_files_present,
            "Inspection must report temp_files_present"
        );
    }

    #[cfg(windows)]
    #[test]
    fn test_failed_existing_file_replacement_does_not_fall_through_to_creation() {
        let dir = tempdir().unwrap();
        let canonical_root = dir.path().canonicalize().unwrap();
        let coalition = canonical_root.join(".coalition");
        fs::create_dir_all(&coalition).unwrap();

        let initial_project = ProjectYaml {
            schema_version: 1,
            project_id: Uuid::new_v4().to_string(),
            name: "original-before-replace".to_string(),
            current_architecture_version: None,
            architecture_state: ArchitectureState::Draft,
            created_at: "2026-09-08T00:00:00Z".to_string(),
        };
        let project_yaml_path = coalition.join("project.yaml");
        fs::write(
            &project_yaml_path,
            serde_yaml::to_string(&initial_project).unwrap(),
        )
        .unwrap();

        let mut updated = initial_project.clone();
        updated.name = "mutated-should-not-overwrite".to_string();

        // Simulate ReplaceFileW failing with ERROR_FILE_NOT_FOUND (code 2)
        INJECTED_SEAM.with(|f| f.set(InjectedSeam::SimulateReplaceFileWError(2)));

        let res = ArtifactManager::write_project_yaml_atomic(&project_yaml_path, &updated);
        assert!(
            res.is_err(),
            "Must NOT fall through to MoveFileExW creation path and succeed!"
        );

        // Reset injection seam
        INJECTED_SEAM.with(|f| f.set(InjectedSeam::None));

        // Verify original destination remains intact with original content
        let on_disk = ArtifactManager::read_project_yaml(&project_yaml_path).unwrap();
        assert_eq!(
            on_disk.name, "original-before-replace",
            "Destination must not have been overwritten by MoveFileExW creation fallback"
        );
    }

    #[test]
    fn test_initialize_or_load_project_backup_only_returns_recovery_required() {
        let dir = tempdir().unwrap();
        let coalition = dir.path().join(".coalition");
        fs::create_dir_all(&coalition).unwrap();

        let backup_proj = ProjectYaml {
            schema_version: 1,
            project_id: Uuid::new_v4().to_string(),
            name: "backup-only-proj".to_string(),
            current_architecture_version: None,
            architecture_state: ArchitectureState::Draft,
            created_at: "2026-09-08T00:00:00Z".to_string(),
        };
        fs::write(
            coalition.join("project.yaml.bak.1"),
            serde_yaml::to_string(&backup_proj).unwrap(),
        )
        .unwrap();

        // Must return RecoveryRequired and NOT initialize a new project identity
        let err = ArtifactManager::initialize_or_load_project(dir.path(), "new-should-not-create")
            .unwrap_err();
        assert!(
            matches!(err, ArtifactError::RecoveryRequired(_)),
            "Expected RecoveryRequired, got: {:?}",
            err
        );
        assert!(
            !coalition.join("project.yaml").exists(),
            "Must NOT create project.yaml while recovery artifacts exist"
        );
    }

    #[test]
    fn test_initialize_or_load_project_temp_only_returns_recovery_required() {
        let dir = tempdir().unwrap();
        let coalition = dir.path().join(".coalition");
        fs::create_dir_all(&coalition).unwrap();

        fs::write(
            coalition.join("project.yaml.tmp.incomplete"),
            "temporary write data",
        )
        .unwrap();

        // Must return RecoveryRequired and NOT initialize a new project identity
        let err = ArtifactManager::initialize_or_load_project(dir.path(), "new-should-not-create")
            .unwrap_err();
        assert!(
            matches!(err, ArtifactError::RecoveryRequired(_)),
            "Expected RecoveryRequired, got: {:?}",
            err
        );
        assert!(
            !coalition.join("project.yaml").exists(),
            "Must NOT create project.yaml while temp files exist"
        );
    }

    #[test]
    fn test_failed_backup_promotion_preserves_backup_intact() {
        let dir = tempdir().unwrap();
        let canonical_root = dir.path().canonicalize().unwrap();
        let coalition = canonical_root.join(".coalition");
        fs::create_dir_all(&coalition).unwrap();

        let backup_project = ProjectYaml {
            schema_version: 1,
            project_id: Uuid::new_v4().to_string(),
            name: "recovery-promotion-test".to_string(),
            current_architecture_version: None,
            architecture_state: ArchitectureState::Draft,
            created_at: "2026-09-08T00:00:00Z".to_string(),
        };

        let backup_file = coalition.join("project.yaml.bak.promotion_fail");
        let backup_yaml = serde_yaml::to_string(&backup_project).unwrap();
        fs::write(&backup_file, &backup_yaml).unwrap();
        let original_bytes = fs::read(&backup_file).unwrap();

        let canonical_path = coalition.join("project.yaml");
        assert!(!canonical_path.exists());

        // Inject MoveFileExW failure for destination-absent move
        INJECTED_SEAM.with(|f| f.set(InjectedSeam::SimulateMoveFileExWError(5)));

        let res = ArtifactManager::promote_backup_to_canonical(&backup_file, &canonical_path);
        assert!(res.is_err(), "Promotion must fail when MoveFileExW fails");

        // Reset injection seam
        INJECTED_SEAM.with(|f| f.set(InjectedSeam::None));

        // 1. Canonical must remain absent
        assert!(
            !canonical_path.exists(),
            "Canonical project.yaml must remain absent after failed promotion"
        );

        // 2. Authoritative backup must remain byte-for-byte intact (NOT deleted)
        assert!(
            backup_file.exists(),
            "Backup file must NOT be deleted when promotion fails!"
        );
        let current_bytes = fs::read(&backup_file).unwrap();
        assert_eq!(
            current_bytes, original_bytes,
            "Backup content must remain byte-for-byte identical"
        );

        // 3. inspect_project_artifacts must still discover the same valid backup
        let inspection = ArtifactManager::inspect_project_artifacts(dir.path()).unwrap();
        assert!(inspection.canonical.is_none());
        assert_eq!(inspection.valid_backups.len(), 1);
        assert_eq!(
            inspection.valid_backups[0].project.project_id,
            backup_project.project_id
        );
    }

    #[test]
    fn test_architecture_artifact_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        ArtifactManager::initialize_new_project(dir.path(), "test-arch").unwrap();

        let path = "design/product-vision.md";
        let content = "# Product Vision\nBuilding a governed AI desktop control plane.\n";

        // 1. Initially does not exist
        let initial = ArtifactManager::read_artifact(dir.path(), path).unwrap();
        assert!(initial.is_none());

        // 2. Write atomically
        ArtifactManager::write_artifact_atomic(dir.path(), path, content).unwrap();

        // 3. Read back
        let read_back = ArtifactManager::read_artifact(dir.path(), path).unwrap();
        assert_eq!(read_back, Some(content.to_string()));

        // 4. Update
        let updated = "# Product Vision\nUpdated version 2.\n";
        ArtifactManager::write_artifact_atomic(dir.path(), path, updated).unwrap();
        assert_eq!(
            ArtifactManager::read_artifact(dir.path(), path).unwrap(),
            Some(updated.to_string())
        );

        // 5. Delete
        let deleted = ArtifactManager::delete_artifact(dir.path(), path).unwrap();
        assert!(deleted);
        assert!(ArtifactManager::read_artifact(dir.path(), path)
            .unwrap()
            .is_none());
    }

    #[test]
    fn test_architecture_artifact_path_traversal_rejection() {
        let dir = tempfile::tempdir().unwrap();
        ArtifactManager::initialize_new_project(dir.path(), "test-traversal").unwrap();

        let evil_paths = &[
            "../secret.txt",
            "..\\evil.txt",
            "/etc/passwd",
            "C:\\windows\\system32\\evil.dll",
            "project.yaml",
            ".git/config",
            "src/main.rs",
            "design/../../escape.txt",
        ];

        for &bad in evil_paths {
            assert!(
                !ArtifactManager::is_valid_architecture_artifact_path(bad),
                "Should reject invalid path: {}",
                bad
            );

            let res = ArtifactManager::write_artifact_atomic(dir.path(), bad, "evil");
            assert!(res.is_err(), "write_artifact_atomic must reject: {}", bad);

            let read_res = ArtifactManager::read_artifact(dir.path(), bad);
            assert!(read_res.is_err(), "read_artifact must reject: {}", bad);
        }
    }
}

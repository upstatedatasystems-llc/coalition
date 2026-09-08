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

    /// Inspects the project state in .coalition or recovers from a single valid backup.
    /// Precedence:
    /// 1. If project.yaml exists and is valid: cleans stale temp files and returns Some(project).
    /// 2. If project.yaml is missing:
    ///    - Checks for project.yaml.bak.*
    ///    - If exactly one valid backup exists: promotes it atomically to project.yaml, cleans stale temp files, returns Some(recovered).
    ///    - If multiple backups or corrupted backup: returns Err(ArtifactError::RecoveryRequired).
    ///    - If no backups: returns Ok(None).
    pub fn inspect_or_recover_project<P: AsRef<Path>>(
        repo_root: P,
    ) -> Result<Option<ProjectYaml>, ArtifactError> {
        let canonical_root = repo_root
            .as_ref()
            .canonicalize()
            .map_err(|e| ArtifactError::Io(format!("Failed to canonicalize repo root: {}", e)))?;

        let coalition_dir = canonical_root.join(".coalition");
        if !coalition_dir.exists() {
            return Ok(None);
        }

        Self::validate_safe_path(&canonical_root, &coalition_dir)?;

        let project_yaml_path = coalition_dir.join("project.yaml");
        if project_yaml_path.exists() {
            Self::validate_safe_path(&canonical_root, &project_yaml_path)?;
            let project = Self::read_project_yaml(&project_yaml_path)?;
            Self::clean_stale_temp_files(&coalition_dir);
            return Ok(Some(project));
        }

        // project.yaml is missing; check for backup files
        let mut backups = Vec::new();
        if let Ok(entries) = fs::read_dir(&coalition_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("project.yaml.bak.") {
                    backups.push(entry.path());
                }
            }
        }

        if backups.is_empty() {
            Self::clean_stale_temp_files(&coalition_dir);
            return Ok(None);
        }

        if backups.len() == 1 {
            let backup_path = &backups[0];
            Self::validate_safe_path(&canonical_root, backup_path)?;
            match Self::read_project_yaml(backup_path) {
                Ok(recovered) => {
                    Self::replace_file_atomically(backup_path, &project_yaml_path)?;
                    Self::clean_stale_temp_files(&coalition_dir);
                    Ok(Some(recovered))
                }
                Err(e) => Err(ArtifactError::RecoveryRequired(format!(
                    "Found single backup file '{:?}', but it is corrupted/invalid: {}",
                    backup_path.file_name().unwrap_or_default(),
                    e
                ))),
            }
        } else {
            Err(ArtifactError::RecoveryRequired(format!(
                "Multiple project.yaml backup files (count: {}) found in .coalition; manual recovery required to avoid selecting wrong state",
                backups.len()
            )))
        }
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

    /// Initializes standard durable .coalition layout if not present, and loads/recovers project.yaml.
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
            Self::validate_coalition_layout(&canonical_root, &coalition_dir)?;
            if let Some(existing) = Self::inspect_or_recover_project(&canonical_root)? {
                return Ok((existing, false));
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
        if INJECTED_REPLACEMENT_FAILURE.with(|f| f.get()) {
            return Err(ArtifactError::Io(
                "Injected atomic replacement failure for test".to_string(),
            ));
        }

        replace_file_atomically_impl(temp, destination)
    }

    /// Writes project.yaml using genuinely crash-safe atomic replacement:
    /// 1. Validates the descriptor before any I/O.
    /// 2. Writes to a temporary file in the same directory and flushes/syncs to disk.
    /// 3. Atomically replaces target using platform-native atomic replacement.
    /// 4. If replacement fails, cleans up temporary file and leaves target completely intact.
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
            let _ = fs::remove_file(&temp_path);
            return Err(e);
        }

        Ok(())
    }
}

#[cfg(test)]
thread_local! {
    pub static INJECTED_REPLACEMENT_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(windows)]
fn replace_file_atomically_impl(temp: &Path, destination: &Path) -> Result<(), ArtifactError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{
        GetLastError, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, ReplaceFileW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        REPLACEFILE_WRITE_THROUGH,
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

    if destination.exists() {
        let res = unsafe {
            ReplaceFileW(
                dest_wide.as_ptr(),
                temp_wide.as_ptr(),
                std::ptr::null(),
                REPLACEFILE_WRITE_THROUGH,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };

        if res != 0 {
            return Ok(());
        }

        let err = unsafe { GetLastError() };
        if err != ERROR_FILE_NOT_FOUND && err != ERROR_PATH_NOT_FOUND {
            return Err(ArtifactError::Io(format!(
                "ReplaceFileW failed with OS error code {}",
                err
            )));
        }
    }

    let res = unsafe {
        MoveFileExW(
            temp_wide.as_ptr(),
            dest_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };

    if res != 0 {
        Ok(())
    } else {
        let err = unsafe { GetLastError() };
        Err(ArtifactError::Io(format!(
            "MoveFileExW failed with OS error code {}",
            err
        )))
    }
}

#[cfg(not(windows))]
fn replace_file_atomically_impl(temp: &Path, destination: &Path) -> Result<(), ArtifactError> {
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

        // Calling inspect_or_recover_project cleans stale temp files
        let inspected = ArtifactManager::inspect_or_recover_project(dir.path())
            .unwrap()
            .expect("should find project");
        assert_eq!(inspected.project_id, project.project_id);
        assert!(!stale_temp.exists(), "stale temp file must be cleaned up");
    }

    #[test]
    fn test_single_valid_backup_recovery() {
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

        // inspect_or_recover_project recovers the single valid backup
        let recovered = ArtifactManager::inspect_or_recover_project(dir.path())
            .unwrap()
            .expect("should recover project");
        assert_eq!(recovered.project_id, backup_project.project_id);
        assert_eq!(recovered.name, "recovered-project");

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

        let err = ArtifactManager::inspect_or_recover_project(dir.path()).unwrap_err();
        assert!(matches!(err, ArtifactError::RecoveryRequired(_)));

        // Neither backup should be deleted
        assert!(coalition.join("project.yaml.bak.1").exists());
        assert!(coalition.join("project.yaml.bak.2").exists());
        assert!(!coalition.join("project.yaml").exists());
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
}

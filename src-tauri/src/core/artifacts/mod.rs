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

        if Uuid::parse_str(&project.project_id).is_err() {
            return Err(ArtifactError::InvalidProjectId(project.project_id.clone()));
        }

        if project.name.trim().is_empty() {
            return Err(ArtifactError::EmptyProjectName);
        }

        if chrono::DateTime::parse_from_rfc3339(&project.created_at).is_err() {
            return Err(ArtifactError::InvalidTimestamp(project.created_at.clone()));
        }

        match project.architecture_state {
            ArchitectureState::Draft => {
                if let Some(ref ver) = project.current_architecture_version {
                    if !ver.trim().is_empty() {
                        return Err(ArtifactError::InvalidArchitectureState(
                            "Draft project cannot declare an active architecture version"
                                .to_string(),
                        ));
                    }
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

    /// Initializes standard durable .coalition layout if not present, and writes new project.yaml.
    /// If .coalition/project.yaml already exists, validates the entire layout and loads project.yaml without overwriting.
    pub fn initialize_or_load_project<P: AsRef<Path>>(
        repo_root: P,
        default_name: &str,
    ) -> Result<(ProjectYaml, bool), ArtifactError> {
        let canonical_root = repo_root
            .as_ref()
            .canonicalize()
            .map_err(|e| ArtifactError::Io(format!("Failed to canonicalize repo root: {}", e)))?;

        let coalition_dir = canonical_root.join(".coalition");
        let project_yaml_path = coalition_dir.join("project.yaml");

        if project_yaml_path.exists() {
            Self::validate_coalition_layout(&canonical_root, &coalition_dir)?;
            let project_yaml = Self::read_project_yaml(&project_yaml_path)?;
            return Ok((project_yaml, false));
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

        Ok((new_project, true))
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

    /// Writes project.yaml using a Windows-safe / cross-platform replacement strategy:
    /// 1. Validates the descriptor before any I/O.
    /// 2. Writes to a temporary file in the same directory and flushes/syncs to disk.
    /// 3. If target file already exists, renames target to a temporary backup file.
    /// 4. Renames temporary file to target.
    /// 5. If renaming temp to target fails, restores the original file from backup.
    /// 6. Cleans up temporary/backup files on success or failure.
    ///
    /// This ensures the existing valid project.yaml is never prematurely deleted.
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

        // 1. Write to temp file and sync
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

        // 2. Safe replacement without premature deletion
        if p.exists() {
            let backup_file_name = format!("project.yaml.bak.{}", Uuid::new_v4());
            let backup_path = parent.join(&backup_file_name);

            if let Err(e) = fs::rename(p, &backup_path) {
                let _ = fs::remove_file(&temp_path);
                return Err(ArtifactError::Io(format!(
                    "Failed to stage temporary backup of existing project.yaml: {}",
                    e
                )));
            }

            if let Err(e) = fs::rename(&temp_path, p) {
                // Restore original file from backup immediately
                let _ = fs::rename(&backup_path, p);
                let _ = fs::remove_file(&temp_path);
                return Err(ArtifactError::Io(format!(
                    "Failed to commit new project.yaml; original file was restored: {}",
                    e
                )));
            }

            // Successfully committed new file; remove temporary backup
            let _ = fs::remove_file(&backup_path);
        } else if let Err(e) = fs::rename(&temp_path, p) {
            let _ = fs::remove_file(&temp_path);
            return Err(ArtifactError::Io(format!(
                "Failed to rename temp file to project.yaml: {}",
                e
            )));
        }

        Ok(())
    }
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
        let v0_yaml = "schema_version: 0\nproject_id: '00000000-0000-0000-0000-000000000000'\nname: 'V0 Proj'\narchitecture_state: draft\ncreated_at: '2026-09-08T00:00:00Z'\n";
        fs::write(coalition.join("project.yaml"), v0_yaml).unwrap();
        let err = ArtifactManager::read_project_yaml(coalition.join("project.yaml")).unwrap_err();
        assert_eq!(err, ArtifactError::UnsupportedSchemaVersion(0));

        // Version 99
        let v99_yaml = "schema_version: 99\nproject_id: '00000000-0000-0000-0000-000000000000'\nname: 'V99 Proj'\narchitecture_state: draft\ncreated_at: '2026-09-08T00:00:00Z'\n";
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

        let empty_name_yaml = "schema_version: 1\nproject_id: '00000000-0000-0000-0000-000000000000'\nname: '   '\narchitecture_state: draft\ncreated_at: '2026-09-08T00:00:00Z'\n";
        fs::write(coalition.join("project.yaml"), empty_name_yaml).unwrap();
        let err = ArtifactManager::read_project_yaml(coalition.join("project.yaml")).unwrap_err();
        assert_eq!(err, ArtifactError::EmptyProjectName);
    }

    #[test]
    fn test_rejects_invalid_timestamp() {
        let dir = tempdir().unwrap();
        let coalition = dir.path().join(".coalition");
        fs::create_dir_all(&coalition).unwrap();

        let bad_ts_yaml = "schema_version: 1\nproject_id: '00000000-0000-0000-0000-000000000000'\nname: 'Proj'\narchitecture_state: draft\ncreated_at: 'yesterday'\n";
        fs::write(coalition.join("project.yaml"), bad_ts_yaml).unwrap();
        let err = ArtifactManager::read_project_yaml(coalition.join("project.yaml")).unwrap_err();
        assert!(matches!(err, ArtifactError::InvalidTimestamp(_)));
    }

    #[test]
    fn test_rejects_draft_with_architecture_version() {
        let dir = tempdir().unwrap();
        let coalition = dir.path().join(".coalition");
        fs::create_dir_all(&coalition).unwrap();

        let draft_with_ver = "schema_version: 1\nproject_id: '00000000-0000-0000-0000-000000000000'\nname: 'Proj'\narchitecture_state: draft\ncurrent_architecture_version: '1.0'\ncreated_at: '2026-09-08T00:00:00Z'\n";
        fs::write(coalition.join("project.yaml"), draft_with_ver).unwrap();
        let err = ArtifactManager::read_project_yaml(coalition.join("project.yaml")).unwrap_err();
        assert!(matches!(err, ArtifactError::InvalidArchitectureState(_)));
    }

    #[test]
    fn test_rejects_frozen_without_architecture_version() {
        let dir = tempdir().unwrap();
        let coalition = dir.path().join(".coalition");
        fs::create_dir_all(&coalition).unwrap();

        let frozen_no_ver = "schema_version: 1\nproject_id: '00000000-0000-0000-0000-000000000000'\nname: 'Proj'\narchitecture_state: frozen\ncreated_at: '2026-09-08T00:00:00Z'\n";
        fs::write(coalition.join("project.yaml"), frozen_no_ver).unwrap();
        let err = ArtifactManager::read_project_yaml(coalition.join("project.yaml")).unwrap_err();
        assert!(matches!(err, ArtifactError::InvalidArchitectureState(_)));
    }

    #[test]
    fn test_accepts_valid_frozen_descriptor() {
        let dir = tempdir().unwrap();
        let coalition = dir.path().join(".coalition");
        fs::create_dir_all(&coalition).unwrap();

        let valid_frozen = "schema_version: 1\nproject_id: '00000000-0000-0000-0000-000000000000'\nname: 'Proj'\narchitecture_state: frozen\ncurrent_architecture_version: '1.0'\ncreated_at: '2026-09-08T00:00:00Z'\n";
        fs::write(coalition.join("project.yaml"), valid_frozen).unwrap();
        let project = ArtifactManager::read_project_yaml(coalition.join("project.yaml")).unwrap();
        assert_eq!(project.architecture_state, ArchitectureState::Frozen);
        assert_eq!(
            project.current_architecture_version,
            Some("1.0".to_string())
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
}

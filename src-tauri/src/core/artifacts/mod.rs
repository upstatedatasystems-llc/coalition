use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

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
    #[error("Unsupported project schema version: {0}. Maximum supported version is 1")]
    UnsupportedSchemaVersion(u32),
    #[error("Invalid YAML in project.yaml: {0}")]
    InvalidYaml(String),
    #[error("Project descriptor project.yaml not found at {0}")]
    NotFound(String),
    #[error("IO error: {0}")]
    Io(String),
}

pub struct ArtifactManager;

impl ArtifactManager {
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

    /// Verifies that a path remains inside canonical_root and does not follow symlinks/junctions
    /// that point outside the repository.
    pub fn validate_safe_path(canonical_root: &Path, target: &Path) -> Result<(), ArtifactError> {
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

            if !canonical_target.starts_with(canonical_root) {
                return Err(ArtifactError::UnsafeReparsePoint(format!(
                    "Path {:?} resolves to {:?} outside repository root {:?}",
                    target, canonical_target, canonical_root
                )));
            }
        }

        let canonical_target = target.canonicalize().map_err(|e| {
            ArtifactError::Io(format!("Failed to canonicalize target {:?}: {}", target, e))
        })?;

        if !canonical_target.starts_with(canonical_root) {
            return Err(ArtifactError::PathTraversal {
                path: target.to_string_lossy().to_string(),
                root: canonical_root.to_string_lossy().to_string(),
            });
        }

        Ok(())
    }

    /// Initializes standard durable .coalition layout if not present, and writes new project.yaml.
    /// If .coalition/project.yaml already exists, loads and returns it without overwriting.
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
            Self::validate_safe_path(&canonical_root, &project_yaml_path)?;
            let project_yaml = Self::read_project_yaml(&project_yaml_path)?;
            return Ok((project_yaml, false));
        }

        // Create directory hierarchy
        let subdirs = [
            "design",
            "implementation",
            "decisions",
            "architecture-versions",
            "changes",
            "reviews",
            "evidence",
        ];

        fs::create_dir_all(&coalition_dir).map_err(|e| {
            ArtifactError::Io(format!("Failed to create .coalition directory: {}", e))
        })?;

        Self::validate_safe_path(&canonical_root, &coalition_dir)?;

        for sub in &subdirs {
            let sub_path = coalition_dir.join(sub);
            fs::create_dir_all(&sub_path).map_err(|e| {
                ArtifactError::Io(format!(
                    "Failed to create .coalition/{} directory: {}",
                    sub, e
                ))
            })?;
        }

        let new_project = ProjectYaml {
            schema_version: CURRENT_SCHEMA_VERSION,
            project_id: Uuid::new_v4().to_string(),
            name: default_name.to_string(),
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

        if parsed.schema_version > CURRENT_SCHEMA_VERSION {
            return Err(ArtifactError::UnsupportedSchemaVersion(
                parsed.schema_version,
            ));
        }

        Ok(parsed)
    }

    /// Writes project.yaml atomically using tempfile + rename suitable for Windows.
    pub fn write_project_yaml_atomic<P: AsRef<Path>>(
        path: P,
        project: &ProjectYaml,
    ) -> Result<(), ArtifactError> {
        let p = path.as_ref();
        let parent = p.parent().ok_or_else(|| {
            ArtifactError::Io("Target project.yaml path has no parent directory".to_string())
        })?;

        let yaml_str = serde_yaml::to_string(project)
            .map_err(|e| ArtifactError::InvalidYaml(e.to_string()))?;

        let temp_file_name = format!("project.yaml.tmp.{}", Uuid::new_v4());
        let temp_path = parent.join(&temp_file_name);

        fs::write(&temp_path, yaml_str.as_bytes())
            .map_err(|e| ArtifactError::Io(format!("Failed to write temp project.yaml: {}", e)))?;

        // On Windows std::fs::rename can fail if the destination exists, so we remove the target first if it exists
        if p.exists() {
            let _ = fs::remove_file(p);
        }

        fs::rename(&temp_path, p).map_err(|e| {
            let _ = fs::remove_file(&temp_path);
            ArtifactError::Io(format!("Failed to atomically rename project.yaml: {}", e))
        })?;

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
    fn test_rejects_unsupported_schema_version() {
        let dir = tempdir().unwrap();
        let coalition = dir.path().join(".coalition");
        fs::create_dir_all(&coalition).unwrap();

        let unsupported_yaml = "schema_version: 99\nproject_id: 'pid-123'\nname: 'Future Proj'\narchitecture_state: draft\ncreated_at: '2026-09-08T00:00:00Z'\n";
        fs::write(coalition.join("project.yaml"), unsupported_yaml).unwrap();

        let err = ArtifactManager::read_project_yaml(coalition.join("project.yaml")).unwrap_err();
        match err {
            ArtifactError::UnsupportedSchemaVersion(v) => assert_eq!(v, 99),
            _ => panic!("Expected UnsupportedSchemaVersion, got {:?}", err),
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
    fn test_rejects_path_escaping_root() {
        let dir1 = tempdir().unwrap();
        let dir2 = tempdir().unwrap();

        // dir2 is outside dir1
        let err = ArtifactManager::validate_safe_path(dir1.path(), dir2.path()).unwrap_err();
        assert!(matches!(err, ArtifactError::PathTraversal { .. }));
    }

    #[test]
    fn test_rejects_symlink_pointing_outside_root() {
        let root_dir = tempdir().unwrap();
        let outside_dir = tempdir().unwrap();

        let symlink_path = root_dir.path().join("outside_link");

        #[cfg(windows)]
        {
            // Windows directory junction / symlink
            let _ = std::os::windows::fs::symlink_dir(outside_dir.path(), &symlink_path);
        }
        #[cfg(unix)]
        {
            let _ = std::os::unix::fs::symlink(outside_dir.path(), &symlink_path);
        }

        if symlink_path.exists() {
            let err =
                ArtifactManager::validate_safe_path(root_dir.path(), &symlink_path).unwrap_err();
            assert!(matches!(
                err,
                ArtifactError::UnsafeReparsePoint(_) | ArtifactError::PathTraversal { .. }
            ));
        }
    }
}

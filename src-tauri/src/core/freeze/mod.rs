use crate::core::activity::{ActivityError, ActivityManager};
use crate::core::artifacts::{
    ArchitectureState, ArtifactApplicability, ArtifactError, ArtifactManager,
    CANONICAL_ARCHITECTURE_ARTIFACTS,
};
use crate::core::git::{GitAdapter, GitError};
use crate::core::relay::readiness::{OverallReadiness, ReadinessEvaluator};
use crate::core::workflow::{self, WorkflowAction, WorkflowError, WorkflowState};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
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
    pub manifest_fingerprint: String,
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
        Self::Artifact(e.to_string())
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

pub struct FreezeService;

impl FreezeService {
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
        let info = git.inspect_repo(&repo_root)?;
        let head_commit = info.head_commit.ok_or(FreezeError::NoHeadCommit)?;

        if head_commit.trim().is_empty() {
            return Err(FreezeError::NoHeadCommit);
        }

        // Compute deterministic dirty-state fingerprint
        let dirty_input = format!(
            "clean:{}:staged:{}:unstaged:{}:untracked:{}:diff:{}",
            info.status.is_clean,
            info.status.staged,
            info.status.unstaged,
            info.status.untracked,
            info.diff_summary
        );
        let dirty_fingerprint = Self::sha256_hex(dirty_input.as_bytes());

        let porcelain = format!(
            "staged={}, unstaged={}, untracked={}",
            info.status.staged, info.status.unstaged, info.status.untracked
        );

        Ok(FrozenGitBoundary {
            head_commit,
            branch: info.current_branch,
            is_detached: info.is_detached,
            is_clean: info.status.is_clean,
            staged_count: info.status.staged,
            unstaged_count: info.status.unstaged,
            untracked_count: info.status.untracked,
            dirty_fingerprint,
            porcelain_status: porcelain,
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
    pub fn prepare_freeze_preview<P: AsRef<Path>>(
        repo_root: P,
        project_id: &str,
        git: &GitAdapter,
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

        // 4. Summarize Builder packet
        let mut total_characters = 0;
        for rel in artifact_baselines.keys() {
            if let Ok(Some(content)) = ArtifactManager::read_artifact(root, rel) {
                total_characters += content.len();
            }
        }
        let estimated_tokens = (total_characters / 4).max(1);
        let is_truncated = total_characters > BUILDER_PACKET_MAX_BYTES;

        let summary = BuilderPacketSummary {
            total_artifacts: artifact_baselines.len(),
            total_characters,
            estimated_tokens,
            is_truncated,
        };

        Ok(FreezePreview {
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
        })
    }

    /// Assembles the bounded Builder packet from the staged frozen contract files.
    fn build_builder_packet(
        metadata: BuilderPacketMetadata,
        staged_contract_dir: &Path,
        manifest_artifacts: &[ContractArtifactEntry],
    ) -> Result<BuilderPacket, FreezeError> {
        let mut artifacts = Vec::new();
        let mut total_chars = 0;
        let mut is_truncated = false;

        for entry in manifest_artifacts {
            let file_path = staged_contract_dir.join(&entry.relative_path);
            let content = fs::read_to_string(&file_path).map_err(|e| {
                FreezeError::Io(format!(
                    "Failed to read staged contract file {:?}: {}",
                    file_path, e
                ))
            })?;

            let title = entry
                .relative_path
                .split('/')
                .next_back()
                .unwrap_or(&entry.relative_path)
                .trim_end_matches(".md")
                .trim_end_matches(".yaml")
                .replace('-', " ");

            let content_len = content.len();
            let effective_content = if total_chars + content_len > BUILDER_PACKET_MAX_BYTES {
                is_truncated = true;
                let available = BUILDER_PACKET_MAX_BYTES.saturating_sub(total_chars);
                if available > 100 {
                    let mut truncated = content[..available.min(content.len())].to_string();
                    truncated
                        .push_str("\n\n[TRUNCATED: Document exceeds Builder Packet size budget]\n");
                    total_chars += truncated.len();
                    truncated
                } else {
                    total_chars += 60;
                    "[TRUNCATED: Excluded due to Builder Packet size budget]\n".to_string()
                }
            } else {
                total_chars += content_len;
                content
            };

            artifacts.push(BuilderPacketArtifact {
                path: entry.relative_path.clone(),
                title,
                content: effective_content,
                fingerprint: entry.fingerprint.clone(),
            });
        }

        let summary = format!(
            "Architecture v{} implementation contract containing {} frozen specifications",
            metadata.architecture_version,
            artifacts.len()
        );

        let builder_rules = r#"=== COALITION BUILDER INSTRUCTIONS ===
1. You are Google Antigravity acting as the Builder for this project.
2. The architecture specifications in this packet are FROZEN and IMMUTABLE.
3. Do not silently change, weaken, or omit requirements or constraints.
4. If an implementation contradiction is discovered, pause and surface an ARCHITECTURE_CONCERN rather than inventing undocumented workarounds.
5. Implement all software strictly according to the acceptance criteria and test plan.
"#.to_string();

        let mut prompt = format!(
            "# Coalition Builder Milestone Task: Architecture v{}\n\nProject: {}\nProject ID: {}\nGit HEAD: {}\n\n{}\n## Frozen Contract Specifications\n\n",
            metadata.architecture_version,
            metadata.project_name,
            metadata.project_id,
            metadata.git_head_commit,
            builder_rules
        );

        for art in &artifacts {
            prompt.push_str(&format!(
                "### {}\nPath: `{}`\nSHA-256: `{}`\n\n```markdown\n{}\n```\n\n",
                art.title, art.path, art.fingerprint, art.content
            ));
        }

        Ok(BuilderPacket {
            metadata,
            summary,
            builder_rules,
            artifacts,
            is_truncated,
            prompt,
        })
    }

    /// Executes the crash-safe freeze transaction upon explicit human confirmation.
    pub fn confirm_freeze<P: AsRef<Path>>(
        repo_root: P,
        project_id: &str,
        preview: &FreezePreview,
        git: &GitAdapter,
        conn: &mut Connection,
    ) -> Result<FreezeResult, FreezeError> {
        let root = repo_root.as_ref();
        let coalition_dir = ArtifactManager::resolve_coalition_dir(root)?;
        let project_yaml_path = coalition_dir.join("project.yaml");

        // 1. Verify project descriptor & workflow state
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

        // 2. Revalidate readiness against disk
        let current_readiness = ReadinessEvaluator::evaluate(root)?;
        if current_readiness.overall_readiness != OverallReadiness::ReadyToFreeze {
            return Err(FreezeError::ReadinessIncomplete(format!(
                "Current readiness is incomplete: {}/{} required artifacts ready",
                current_readiness.ready_required_count, current_readiness.total_required_count
            )));
        }

        // 3. Revalidate Git boundary against disk
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

        // 4. Revalidate all artifact baselines against disk
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

        // 5. Staging Phase
        let staging_id = Uuid::new_v4().to_string();
        let arch_versions_dir = coalition_dir.join("architecture-versions");
        if !arch_versions_dir.exists() {
            fs::create_dir_all(&arch_versions_dir).map_err(|e| {
                FreezeError::Io(format!("Failed to create architecture-versions dir: {}", e))
            })?;
        }

        // Clean any leftover stale staging directories
        ArtifactManager::clean_stale_freeze_staging(&coalition_dir);

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

            fs::write(&dest_path, content.as_bytes()).map_err(|e| {
                let _ = fs::remove_dir_all(&staging_dir);
                FreezeError::Io(format!(
                    "Failed to write staged artifact {:?}: {}",
                    dest_path, e
                ))
            })?;

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

        // 6. Build and write Builder Packet into staging
        let epoch_id = Uuid::new_v4().to_string();
        let packet_id = Uuid::new_v4().to_string();
        let packet_metadata = BuilderPacketMetadata {
            schema_version: FREEZE_SCHEMA_VERSION,
            packet_id,
            project_id: project_id.to_string(),
            project_name: current_project_yaml.name.clone(),
            architecture_version: INITIAL_ARCHITECTURE_VERSION.to_string(),
            builder_epoch_id: epoch_id.clone(),
            created_at: preview.created_at.clone(),
            manifest_fingerprint: String::new(), // Will update once manifest is serialized
            git_head_commit: current_git_boundary.head_commit.clone(),
            git_branch: current_git_boundary.branch.clone(),
        };

        let mut builder_packet = Self::build_builder_packet(
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
        fs::write(&staged_packet_path, &packet_bytes).map_err(|e| {
            let _ = fs::remove_dir_all(&staging_dir);
            FreezeError::Io(format!("Failed to write staged builder packet: {}", e))
        })?;

        // 7. Build and write Contract Manifest into staging
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
            frozen_at: preview.created_at.clone(),
            frozen_by: "HUMAN".to_string(),
            builder_epoch_id: epoch_id.clone(),
            readiness_policy_version: preview.readiness_policy_version,
            applicability,
            unresolved_open_questions_count: preview.unresolved_open_questions_count,
            git_boundary: current_git_boundary.clone(),
            artifacts: manifest_artifacts,
            builder_packet_fingerprint: packet_fingerprint,
        };

        let manifest_yaml_str = serde_yaml::to_string(&manifest).map_err(|e| {
            let _ = fs::remove_dir_all(&staging_dir);
            FreezeError::Artifact(format!("Failed to serialize manifest: {}", e))
        })?;

        let manifest_fingerprint = Self::sha256_hex(manifest_yaml_str.as_bytes());
        builder_packet.metadata.manifest_fingerprint = manifest_fingerprint.clone();

        let staged_manifest_path = staging_dir.join("contract-manifest.yaml");
        fs::write(&staged_manifest_path, manifest_yaml_str.as_bytes()).map_err(|e| {
            let _ = fs::remove_dir_all(&staging_dir);
            FreezeError::Io(format!("Failed to write staged manifest: {}", e))
        })?;

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

        // 8. Finalization Phase: Atomic rename of staging dir to target v1.0
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

        #[cfg(test)]
        {
            let seam = INJECTED_FREEZE_SEAM.with(|c| c.get());
            if seam == InjectedFreezeSeam::PostFinalizePreCommit {
                // At this point snapshot dir exists on disk, but project.yaml has NOT been committed!
                return Err(FreezeError::Io(
                    "Injected failure: PostFinalizePreCommit".to_string(),
                ));
            }
        }

        // 9. DURABLE COMMIT POINT: Atomically update project.yaml
        let mut activated_yaml = current_project_yaml.clone();
        activated_yaml.architecture_state = ArchitectureState::Frozen;
        activated_yaml.current_architecture_version =
            Some(INITIAL_ARCHITECTURE_VERSION.to_string());
        activated_yaml.active_manifest_fingerprint = Some(manifest_fingerprint.clone());

        if let Err(e) =
            ArtifactManager::write_project_yaml_atomic(&project_yaml_path, &activated_yaml)
        {
            // If project.yaml update failed, clean up the uncommitted snapshot dir
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

        // 10. Operational SQLite Updates
        let tx_res = (|| -> Result<(), FreezeError> {
            let tx = conn.transaction()?;

            #[cfg(test)]
            {
                let seam = INJECTED_FREEZE_SEAM.with(|c| c.get());
                if seam == InjectedFreezeSeam::MidDbTransaction {
                    return Err(FreezeError::Database(
                        "Injected failure: MidDbTransaction".to_string(),
                    ));
                }
            }

            // A. Authoritative workflow transition
            workflow::apply_workflow_action_tx(&tx, project_id, WorkflowAction::Freeze, "HUMAN")?;

            // B. Insert minimal PENDING builder epoch
            tx.execute(
                "INSERT INTO builder_epochs (epoch_id, project_id, architecture_version, git_commit, git_branch, created_at, status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    epoch_id,
                    project_id,
                    INITIAL_ARCHITECTURE_VERSION,
                    current_git_boundary.head_commit,
                    current_git_boundary.branch,
                    preview.created_at,
                    "PENDING",
                ],
            )?;

            // C. Insert frozen boundary operational index
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
                    preview.created_at,
                    "HUMAN",
                ],
            )?;

            // D. Activity event
            let short_commit = if current_git_boundary.head_commit.len() >= 8 {
                &current_git_boundary.head_commit[..8]
            } else {
                &current_git_boundary.head_commit
            };
            let event_meta = serde_json::json!({
                "architecture_version": INITIAL_ARCHITECTURE_VERSION,
                "epoch_id": epoch_id,
                "manifest_fingerprint": manifest_fingerprint,
                "git_commit": current_git_boundary.head_commit,
                "git_branch": current_git_boundary.branch,
                "is_clean": current_git_boundary.is_clean,
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

        if let Err(e) = tx_res {
            // Note: Project truth is safely committed in project.yaml. Operational state can be reconciled on next open.
            eprintln!("Warning: SQLite operational write failed during freeze: {}. Will reconcile on reopen.", e);
        }

        Ok(FreezeResult {
            project_id: project_id.to_string(),
            architecture_version: INITIAL_ARCHITECTURE_VERSION.to_string(),
            epoch_id,
            frozen_at: preview.created_at.clone(),
            manifest_fingerprint,
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
    /// - Cleans stale `.staging-v*` directories.
    /// - If `project.yaml` is DRAFT but `v1.0/` exists, removes orphaned `v1.0/` to allow clean retry.
    /// - If `project.yaml` is FROZEN, ensures SQLite workflow state is `FROZEN` and epoch exists.
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

        // 1. Clean stale staging directories
        ArtifactManager::clean_stale_freeze_staging(&coalition_dir);

        let project_yaml = ArtifactManager::read_project_yaml(&project_yaml_path)?;
        let target_v1 = coalition_dir
            .join("architecture-versions")
            .join(format!("v{}", INITIAL_ARCHITECTURE_VERSION));

        // 2. Crash recovery: if project.yaml is DRAFT but target_v1 exists (crash at PostFinalizePreCommit)
        if project_yaml.architecture_state == ArchitectureState::Draft && target_v1.exists() {
            let _ = fs::remove_dir_all(&target_v1);
        }

        // 3. If project.yaml is FROZEN, reconcile operational state
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

            // Reconcile SQLite workflow state
            let wf = workflow::get_workflow_state(conn, project_id);
            match wf {
                Ok(rec) => {
                    if rec.state == WorkflowState::Draft
                        || rec.state == WorkflowState::ReadyToFreeze
                        || rec.state == WorkflowState::Architecting
                    {
                        // Project is frozen in durable truth, but SQLite lagged behind! Reconcile to FROZEN.
                        let now = chrono::Utc::now().to_rfc3339();
                        conn.execute(
                            "UPDATE workflow_state SET state = 'FROZEN', updated_at = ?1 WHERE project_id = ?2",
                            params![now, project_id],
                        )?;
                    }
                }
                Err(_) => {
                    // Operational record missing, recreate
                    let now = chrono::Utc::now().to_rfc3339();
                    conn.execute(
                        "INSERT INTO workflow_state (project_id, state, resume_state, revision, updated_at)
                         VALUES (?1, 'FROZEN', NULL, 1, ?2)",
                        params![project_id, now],
                    )?;
                }
            }

            // Ensure builder_epoch exists for this version
            let epoch_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM builder_epochs WHERE project_id = ?1 AND architecture_version = ?2",
                    params![project_id, version],
                    |r| r.get(0),
                )
                .unwrap_or(0);

            if epoch_count == 0 {
                // Load manifest to reconstruct epoch record
                let manifest = Self::read_contract_manifest(root, version)?;
                let _now = chrono::Utc::now().to_rfc3339();
                conn.execute(
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

                let boundary_id = Uuid::new_v4().to_string();
                let snapshot_path_rel = format!(".coalition/architecture-versions/v{}/", version);
                let manifest_fp = project_yaml
                    .active_manifest_fingerprint
                    .clone()
                    .unwrap_or_default();
                conn.execute(
                    "INSERT OR REPLACE INTO frozen_boundaries (
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
        }

        Ok(())
    }

    /// Reads and parses the contract manifest for a frozen version.
    pub fn read_contract_manifest<P: AsRef<Path>>(
        repo_root: P,
        version: &str,
    ) -> Result<ContractManifest, FreezeError> {
        let root = repo_root.as_ref();
        let coalition_dir = ArtifactManager::resolve_coalition_dir(root)?;
        let manifest_path = coalition_dir
            .join("architecture-versions")
            .join(format!("v{}", version))
            .join("contract-manifest.yaml");

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
    pub fn verify_snapshot_integrity<P: AsRef<Path>>(
        repo_root: P,
        version: &str,
        expected_manifest_fingerprint: Option<&str>,
    ) -> Result<ContractManifest, FreezeError> {
        let root = repo_root.as_ref();
        let coalition_dir = ArtifactManager::resolve_coalition_dir(root)?;
        let version_dir = coalition_dir
            .join("architecture-versions")
            .join(format!("v{}", version));

        if !version_dir.exists() {
            return Err(FreezeError::FrozenSnapshotCorrupt {
                version: version.to_string(),
                reason: format!("Snapshot directory {:?} missing", version_dir),
            });
        }

        let manifest_path = version_dir.join("contract-manifest.yaml");
        if !manifest_path.exists() {
            return Err(FreezeError::FrozenSnapshotCorrupt {
                version: version.to_string(),
                reason: "contract-manifest.yaml missing in snapshot".to_string(),
            });
        }

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

        // 2. Verify each frozen contract file against its manifest hash
        for entry in &manifest.artifacts {
            let file_path = contract_dir.join(&entry.relative_path);
            if !file_path.exists() {
                return Err(FreezeError::FrozenSnapshotCorrupt {
                    version: version.to_string(),
                    reason: format!("Frozen file {:?} missing from snapshot contract", file_path),
                });
            }

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

        // 3. Verify builder-packet.json against manifest hash
        let packet_path = version_dir.join("builder-packet.json");
        if !packet_path.exists() {
            return Err(FreezeError::FrozenSnapshotCorrupt {
                version: version.to_string(),
                reason: "builder-packet.json missing in snapshot".to_string(),
            });
        }

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
        artifact_path: &str,
    ) -> Result<DriftDiff, FreezeError> {
        let root = repo_root.as_ref();
        let coalition_dir = ArtifactManager::resolve_coalition_dir(root)?;
        let project_yaml_path = coalition_dir.join("project.yaml");
        let project_yaml = ArtifactManager::read_project_yaml(&project_yaml_path)?;

        let version = project_yaml
            .current_architecture_version
            .as_deref()
            .unwrap_or(INITIAL_ARCHITECTURE_VERSION);

        // Verify snapshot integrity
        Self::verify_snapshot_integrity(
            root,
            version,
            project_yaml.active_manifest_fingerprint.as_deref(),
        )?;

        let snapshot_file = coalition_dir
            .join("architecture-versions")
            .join(format!("v{}", version))
            .join("contract")
            .join(artifact_path);

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

        let active_path = coalition_dir.join(artifact_path);
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

        let drift_type = match (&frozen_content, &active_content) {
            (Some(_), None) => DriftType::Deleted,
            (None, Some(_)) => DriftType::Added,
            (Some(f), Some(a)) => {
                if f == a {
                    // Not drifted, but treat as modified if requested
                    DriftType::Modified
                } else {
                    DriftType::Modified
                }
            }
            (None, None) => {
                return Err(FreezeError::Artifact(format!(
                    "Artifact '{}' exists in neither snapshot nor active workspace",
                    artifact_path
                )))
            }
        };

        Ok(DriftDiff {
            path: artifact_path.to_string(),
            drift_type,
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
    ) -> Result<DriftReport, FreezeError> {
        let root = repo_root.as_ref();
        let coalition_dir = ArtifactManager::resolve_coalition_dir(root)?;
        let project_yaml_path = coalition_dir.join("project.yaml");
        let project_yaml = ArtifactManager::read_project_yaml(&project_yaml_path)?;

        let version = project_yaml
            .current_architecture_version
            .as_deref()
            .unwrap_or(INITIAL_ARCHITECTURE_VERSION);

        // Verify snapshot integrity first! (Never restore from corrupt snapshot)
        Self::verify_snapshot_integrity(
            root,
            version,
            project_yaml.active_manifest_fingerprint.as_deref(),
        )?;

        let snapshot_file = coalition_dir
            .join("architecture-versions")
            .join(format!("v{}", version))
            .join("contract")
            .join(artifact_path);

        if snapshot_file.exists() {
            // MODIFIED or DELETED artifact: restore from snapshot using safe atomic write
            let content = fs::read_to_string(&snapshot_file).map_err(|e| {
                FreezeError::Io(format!(
                    "Failed to read snapshot file {:?}: {}",
                    snapshot_file, e
                ))
            })?;
            ArtifactManager::write_artifact_atomic(root, artifact_path, &content)?;
        } else {
            // ADDED artifact: quarantine it into .coalition/recovery/quarantine-...
            ArtifactManager::quarantine_added_artifact(root, artifact_path)?;
        }

        Self::check_contract_drift(root, project_id)
    }

    /// Restores all drifted artifacts back to exact frozen baseline.
    pub fn restore_all_drifted_artifacts<P: AsRef<Path>>(
        repo_root: P,
        project_id: &str,
    ) -> Result<DriftReport, FreezeError> {
        let root = repo_root.as_ref();
        let report = Self::check_contract_drift(root, project_id)?;

        if !report.has_drift {
            return Ok(report);
        }

        for drifted in &report.drifted_artifacts {
            Self::restore_drifted_artifact(root, project_id, &drifted.path)?;
        }

        Self::check_contract_drift(root, project_id)
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
        ];

        for (rel, content) in files {
            ArtifactManager::write_artifact_atomic(repo_root, rel, content).unwrap();
        }
    }

    #[test]
    fn test_freeze_fails_on_unborn_repository_no_head_commit() {
        let dir = tempdir().unwrap();
        setup_git_repo(dir.path());
        // Git repo has 0 commits (unborn HEAD)
        let project = ArtifactManager::initialize_new_project(dir.path(), "Test").unwrap();
        let pid = project.project_id;
        populate_ready_artifacts(dir.path());

        let git = GitAdapter::new().unwrap();
        let err = FreezeService::prepare_freeze_preview(dir.path(), &pid, &git).unwrap_err();
        assert_eq!(err, FreezeError::NoHeadCommit);
    }

    #[test]
    fn test_freeze_fails_when_readiness_is_incomplete() {
        let dir = tempdir().unwrap();
        setup_git_repo(dir.path());
        commit_file(dir.path(), "README.md", "# Test", "initial commit");

        let project = ArtifactManager::initialize_new_project(dir.path(), "Test").unwrap();
        let pid = project.project_id;
        let git = GitAdapter::new().unwrap();

        // No artifacts written yet -> readiness incomplete!
        let err = FreezeService::prepare_freeze_preview(dir.path(), &pid, &git).unwrap_err();
        assert!(matches!(err, FreezeError::ReadinessIncomplete(_)));
    }

    #[test]
    fn test_successful_freeze_journey_end_to_end() {
        let dir = tempdir().unwrap();
        setup_git_repo(dir.path());
        commit_file(dir.path(), "README.md", "# Test", "initial commit");

        let project = ArtifactManager::initialize_new_project(dir.path(), "Test").unwrap();
        let pid = project.project_id;
        populate_ready_artifacts(dir.path());

        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();

        // Set workflow state to READY_TO_FREEZE in SQLite
        db.connection().execute(
            "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at)
             VALUES (?1, 'Test', ?2, 'now', 'now', 'now')",
            params![pid, dir.path().to_string_lossy().to_string()],
        ).unwrap();
        db.connection()
            .execute(
                "INSERT INTO workflow_state (project_id, state, resume_state, revision, updated_at)
             VALUES (?1, 'READY_TO_FREEZE', NULL, 1, 'now')",
                params![pid],
            )
            .unwrap();

        let git = GitAdapter::new().unwrap();

        // 1. Prepare Freeze Preview
        let preview = FreezeService::prepare_freeze_preview(dir.path(), &pid, &git).unwrap();
        assert_eq!(preview.target_version, "1.0");
        assert_eq!(preview.ready_required_count, preview.total_required_count);
        assert!(!preview.git_boundary.head_commit.is_empty());
        assert_eq!(preview.artifact_baselines.len(), 9);

        // 2. Confirm Freeze
        let result =
            FreezeService::confirm_freeze(dir.path(), &pid, &preview, &git, db.connection_mut())
                .unwrap();

        assert_eq!(result.architecture_version, "1.0");
        assert_eq!(
            result.git_boundary.head_commit,
            preview.git_boundary.head_commit
        );

        // 3. Verify durable state: project.yaml is frozen at 1.0 with manifest fingerprint
        let updated_yaml =
            ArtifactManager::read_project_yaml(dir.path().join(".coalition/project.yaml")).unwrap();
        assert_eq!(updated_yaml.architecture_state, ArchitectureState::Frozen);
        assert_eq!(
            updated_yaml.current_architecture_version,
            Some("1.0".to_string())
        );
        assert!(updated_yaml.active_manifest_fingerprint.is_some());

        // 4. Verify snapshot directory structure
        let snapshot_dir = dir.path().join(".coalition/architecture-versions/v1.0");
        assert!(snapshot_dir.exists());
        assert!(snapshot_dir
            .join("contract/design/product-vision.md")
            .exists());
        assert!(snapshot_dir.join("contract-manifest.yaml").exists());
        assert!(snapshot_dir.join("builder-packet.json").exists());

        // 5. Verify SQLite operational state
        let wf = workflow::get_workflow_state(db.connection(), &pid).unwrap();
        assert_eq!(wf.state, WorkflowState::Frozen);

        let epoch_status: String = db.connection().query_row(
            "SELECT status FROM builder_epochs WHERE project_id = ?1 AND architecture_version = '1.0'",
            params![pid],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(epoch_status, "PENDING");

        // 6. Verify Builder packet can be read
        let packet = FreezeService::get_builder_packet(dir.path(), None).unwrap();
        assert_eq!(packet.metadata.architecture_version, "1.0");
        assert!(!packet.artifacts.is_empty());

        // 7. Verify no initial drift
        let drift = FreezeService::check_contract_drift(dir.path(), &pid).unwrap();
        assert!(!drift.has_drift);
        assert!(drift.is_frozen);
    }

    #[test]
    fn test_stale_freeze_preview_rejection_on_artifact_or_git_change() {
        let dir = tempdir().unwrap();
        setup_git_repo(dir.path());
        commit_file(dir.path(), "README.md", "# Test", "initial commit");

        let project = ArtifactManager::initialize_new_project(dir.path(), "Test").unwrap();
        let pid = project.project_id;
        populate_ready_artifacts(dir.path());

        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();
        db.connection().execute(
            "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at)
             VALUES (?1, 'Test', ?2, 'now', 'now', 'now')",
            params![pid, dir.path().to_string_lossy().to_string()],
        ).unwrap();
        db.connection()
            .execute(
                "INSERT INTO workflow_state (project_id, state, resume_state, revision, updated_at)
             VALUES (?1, 'READY_TO_FREEZE', NULL, 1, 'now')",
                params![pid],
            )
            .unwrap();

        let git = GitAdapter::new().unwrap();
        let preview = FreezeService::prepare_freeze_preview(dir.path(), &pid, &git).unwrap();

        // Subversively modify an active artifact before human confirms preview
        ArtifactManager::write_artifact_atomic(
            dir.path(),
            "design/product-vision.md",
            "# Modified Vision\n\nSubstantive sneaky changes that alter the architecture baseline contract.",
        ).unwrap();

        // Confirm freeze should reject with StaleFreezePreview!
        let err =
            FreezeService::confirm_freeze(dir.path(), &pid, &preview, &git, db.connection_mut())
                .unwrap_err();

        assert!(matches!(err, FreezeError::StaleFreezePreview(_)));
    }

    #[test]
    fn test_drift_detection_and_non_destructive_restoration() {
        let dir = tempdir().unwrap();
        setup_git_repo(dir.path());
        commit_file(dir.path(), "README.md", "# Test", "initial commit");

        let project = ArtifactManager::initialize_new_project(dir.path(), "Test").unwrap();
        let pid = project.project_id;
        populate_ready_artifacts(dir.path());

        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();
        db.connection().execute(
            "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at)
             VALUES (?1, 'Test', ?2, 'now', 'now', 'now')",
            params![pid, dir.path().to_string_lossy().to_string()],
        ).unwrap();
        db.connection()
            .execute(
                "INSERT INTO workflow_state (project_id, state, resume_state, revision, updated_at)
             VALUES (?1, 'READY_TO_FREEZE', NULL, 1, 'now')",
                params![pid],
            )
            .unwrap();

        let git = GitAdapter::new().unwrap();
        let preview = FreezeService::prepare_freeze_preview(dir.path(), &pid, &git).unwrap();
        FreezeService::confirm_freeze(dir.path(), &pid, &preview, &git, db.connection_mut())
            .unwrap();

        // 1. Introduce external MODIFIED drift
        fs::write(
            dir.path().join(".coalition/design/product-vision.md"),
            "Drifted text!",
        )
        .unwrap();

        // 2. Introduce external DELETED drift
        fs::remove_file(dir.path().join(".coalition/design/security.md")).unwrap();

        // 3. Introduce external ADDED drift
        ArtifactManager::write_artifact_atomic(
            dir.path(),
            "decisions/ADR-001-test.md",
            "# ADR-001\nAdded decision record.",
        )
        .unwrap();

        let drift = FreezeService::check_contract_drift(dir.path(), &pid).unwrap();
        assert!(drift.has_drift);
        assert_eq!(drift.drifted_artifacts.len(), 3);

        // 4. Test diff inspection
        let diff = FreezeService::get_drift_diff(dir.path(), "design/product-vision.md").unwrap();
        assert_eq!(diff.drift_type, DriftType::Modified);
        assert!(diff
            .frozen_content
            .unwrap()
            .contains("Substantive product vision"));
        assert_eq!(diff.active_content.unwrap(), "Drifted text!");

        // 5. Restore all drifted artifacts
        let restored_report =
            FreezeService::restore_all_drifted_artifacts(dir.path(), &pid).unwrap();
        assert!(!restored_report.has_drift);

        // 6. Verify restored content matches frozen snapshot
        let restored_vision =
            ArtifactManager::read_artifact(dir.path(), "design/product-vision.md")
                .unwrap()
                .unwrap();
        assert!(restored_vision.contains("Substantive product vision"));

        let restored_security = ArtifactManager::read_artifact(dir.path(), "design/security.md")
            .unwrap()
            .unwrap();
        assert!(restored_security.contains("Substantive security content"));

        // 7. Verify ADDED artifact was quarantined (not permanently lost)
        assert!(!dir
            .path()
            .join(".coalition/decisions/ADR-001-test.md")
            .exists());
        let recovery_dir = dir.path().join(".coalition/recovery");
        let quarantined = fs::read_dir(&recovery_dir).unwrap().count();
        assert!(
            quarantined > 0,
            "Added artifact must be preserved in quarantine"
        );
    }

    #[test]
    fn test_corrupted_snapshot_triggers_frozen_snapshot_corrupt_and_blocks_restore() {
        let dir = tempdir().unwrap();
        setup_git_repo(dir.path());
        commit_file(dir.path(), "README.md", "# Test", "initial commit");

        let project = ArtifactManager::initialize_new_project(dir.path(), "Test").unwrap();
        let pid = project.project_id;
        populate_ready_artifacts(dir.path());

        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();
        db.connection().execute(
            "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at)
             VALUES (?1, 'Test', ?2, 'now', 'now', 'now')",
            params![pid, dir.path().to_string_lossy().to_string()],
        ).unwrap();
        db.connection()
            .execute(
                "INSERT INTO workflow_state (project_id, state, resume_state, revision, updated_at)
             VALUES (?1, 'READY_TO_FREEZE', NULL, 1, 'now')",
                params![pid],
            )
            .unwrap();

        let git = GitAdapter::new().unwrap();
        let preview = FreezeService::prepare_freeze_preview(dir.path(), &pid, &git).unwrap();
        FreezeService::confirm_freeze(dir.path(), &pid, &preview, &git, db.connection_mut())
            .unwrap();

        // Corrupt a frozen snapshot file directly!
        let frozen_vision = dir
            .path()
            .join(".coalition/architecture-versions/v1.0/contract/design/product-vision.md");
        fs::write(&frozen_vision, "Corrupted snapshot file!!").unwrap();

        // Drift check must fail with FrozenSnapshotCorrupt
        let err = FreezeService::check_contract_drift(dir.path(), &pid).unwrap_err();
        assert!(matches!(err, FreezeError::FrozenSnapshotCorrupt { .. }));

        // Restore attempt must also fail with FrozenSnapshotCorrupt
        let restore_err =
            FreezeService::restore_drifted_artifact(dir.path(), &pid, "design/product-vision.md")
                .unwrap_err();
        assert!(matches!(
            restore_err,
            FreezeError::FrozenSnapshotCorrupt { .. }
        ));
    }

    #[test]
    fn test_rehydration_after_sqlite_wipe_reconstructs_frozen_state_and_epoch() {
        let dir = tempdir().unwrap();
        setup_git_repo(dir.path());
        commit_file(dir.path(), "README.md", "# Test", "initial commit");

        let project = ArtifactManager::initialize_new_project(dir.path(), "Test").unwrap();
        let pid = project.project_id;
        populate_ready_artifacts(dir.path());

        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();
        db.connection().execute(
            "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at)
             VALUES (?1, 'Test', ?2, 'now', 'now', 'now')",
            params![pid, dir.path().to_string_lossy().to_string()],
        ).unwrap();
        db.connection()
            .execute(
                "INSERT INTO workflow_state (project_id, state, resume_state, revision, updated_at)
             VALUES (?1, 'READY_TO_FREEZE', NULL, 1, 'now')",
                params![pid],
            )
            .unwrap();

        let git = GitAdapter::new().unwrap();
        let preview = FreezeService::prepare_freeze_preview(dir.path(), &pid, &git).unwrap();
        FreezeService::confirm_freeze(dir.path(), &pid, &preview, &git, db.connection_mut())
            .unwrap();

        // WIPE SQLITE: create completely fresh in-memory database!
        let mut fresh_db = DbManager::new_in_memory().unwrap();
        fresh_db.run_migrations().unwrap();

        // Register/rehydrate project into fresh SQLite database
        let details = crate::core::projects::ProjectService::register_or_open_project(
            fresh_db.connection_mut(),
            &git,
            dir.path(),
        )
        .unwrap();

        assert_eq!(details.workflow_state.state, WorkflowState::Frozen);

        // Reconcile freeze state
        FreezeService::reconcile_freeze_state(dir.path(), &pid, fresh_db.connection_mut()).unwrap();

        // Verify builder_epoch was restored from contract manifest into fresh DB!
        let epoch_ver: String = fresh_db
            .connection()
            .query_row(
                "SELECT architecture_version FROM builder_epochs WHERE project_id = ?1",
                params![pid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(epoch_ver, "1.0");
    }

    #[test]
    fn test_failure_injection_seams() {
        let dir = tempdir().unwrap();
        setup_git_repo(dir.path());
        commit_file(dir.path(), "README.md", "# Test", "initial commit");

        let project = ArtifactManager::initialize_new_project(dir.path(), "Test").unwrap();
        let pid = project.project_id;
        populate_ready_artifacts(dir.path());

        let git = GitAdapter::new().unwrap();

        // 1. Failure at MidStaging: should clean up staging and leave project in draft
        INJECTED_FREEZE_SEAM.with(|c| c.set(InjectedFreezeSeam::MidStaging));
        let mut db1 = DbManager::new_in_memory().unwrap();
        db1.run_migrations().unwrap();
        db1.connection().execute(
            "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at)
             VALUES (?1, 'Test', ?2, 'now', 'now', 'now')",
            params![pid, dir.path().to_string_lossy().to_string()],
        ).unwrap();
        db1.connection()
            .execute(
                "INSERT INTO workflow_state (project_id, state, resume_state, revision, updated_at)
             VALUES (?1, 'READY_TO_FREEZE', NULL, 1, 'now')",
                params![pid],
            )
            .unwrap();

        let preview1 = FreezeService::prepare_freeze_preview(dir.path(), &pid, &git).unwrap();
        let err1 =
            FreezeService::confirm_freeze(dir.path(), &pid, &preview1, &git, db1.connection_mut())
                .unwrap_err();
        assert!(err1.to_string().contains("MidStaging"));

        // Verify project is still Draft, target v1.0 does not exist
        let yaml1 =
            ArtifactManager::read_project_yaml(dir.path().join(".coalition/project.yaml")).unwrap();
        assert_eq!(yaml1.architecture_state, ArchitectureState::Draft);
        assert!(!dir
            .path()
            .join(".coalition/architecture-versions/v1.0")
            .exists());

        // 2. Failure at PostFinalizePreCommit: snapshot finalized on disk, but project.yaml NOT committed
        INJECTED_FREEZE_SEAM.with(|c| c.set(InjectedFreezeSeam::PostFinalizePreCommit));
        let preview2 = FreezeService::prepare_freeze_preview(dir.path(), &pid, &git).unwrap();
        let err2 =
            FreezeService::confirm_freeze(dir.path(), &pid, &preview2, &git, db1.connection_mut())
                .unwrap_err();
        assert!(err2.to_string().contains("PostFinalizePreCommit"));

        // Reconcile: since project.yaml is still Draft, reconcile removes uncommitted v1.0 allowing safe retry!
        FreezeService::reconcile_freeze_state(dir.path(), &pid, db1.connection_mut()).unwrap();
        assert!(!dir
            .path()
            .join(".coalition/architecture-versions/v1.0")
            .exists());

        // 3. Reset seam to None and verify clean freeze succeeds
        INJECTED_FREEZE_SEAM.with(|c| c.set(InjectedFreezeSeam::None));
        let preview3 = FreezeService::prepare_freeze_preview(dir.path(), &pid, &git).unwrap();
        let res =
            FreezeService::confirm_freeze(dir.path(), &pid, &preview3, &git, db1.connection_mut())
                .unwrap();
        assert_eq!(res.architecture_version, "1.0");
    }
}

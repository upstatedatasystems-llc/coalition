use crate::core::activity::ActivityManager;
use crate::core::builder::safe_sanitize_text;
use crate::core::git::GitAdapter;
use crate::core::process::{ProcessOutputKind, ProcessOutputLine, ProcessRunner};
use crate::core::projects::ProjectService;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Error, Debug)]
pub enum ValidationError {
    #[error("Unsupported validation schema version {0}. Expected 1")]
    UnsupportedSchemaVersion(u32),
    #[error("Duplicate validation command ID: '{0}'")]
    DuplicateCommandId(String),
    #[error("Invalid working directory '{0}': must be a relative path within repository bounds")]
    InvalidWorkingDirectory(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("YAML parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("Database error: {0}")]
    Database(String),
    #[error("Git error: {0}")]
    Git(String),
    #[error("Validation execution error: {0}")]
    Execution(String),
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("Validation run not found: {0}")]
    NotFound(String),
    #[error("Stale evidence: {0}")]
    StaleEvidence(String),
}

fn default_true() -> bool {
    true
}

fn default_one() -> u32 {
    1
}

fn default_timeout() -> u64 {
    600
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationPolicy {
    #[serde(default = "default_true")]
    pub gate_review_on_required_failure: bool,
    #[serde(default = "default_true")]
    pub allow_manual_runs: bool,
    #[serde(default = "default_true")]
    pub allow_builder_requested_runs: bool,
}

impl Default for ValidationPolicy {
    fn default() -> Self {
        Self {
            gate_review_on_required_failure: true,
            allow_manual_runs: true,
            allow_builder_requested_runs: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationTriggersConfig {
    #[serde(default = "default_true")]
    pub allow_manual_runs: bool,
    #[serde(default = "default_true")]
    pub allow_builder_requested_runs: bool,
}

impl Default for ValidationTriggersConfig {
    fn default() -> Self {
        Self {
            allow_manual_runs: true,
            allow_builder_requested_runs: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationCommandConfig {
    pub id: String,
    pub name: String,
    pub command: String,
    pub working_directory: Option<String>,
    #[serde(default = "default_true")]
    pub required: bool,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationConfig {
    #[serde(default = "default_one")]
    pub schema_version: u32,
    pub enabled: bool,
    #[serde(default)]
    pub policy: ValidationPolicy,
    #[serde(default)]
    pub triggers: ValidationTriggersConfig,
    #[serde(default)]
    pub commands: Vec<ValidationCommandConfig>,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            enabled: false,
            policy: ValidationPolicy::default(),
            triggers: ValidationTriggersConfig::default(),
            commands: Vec::new(),
        }
    }
}

impl ValidationConfig {
    pub fn is_manual_run_allowed(&self) -> bool {
        self.policy.allow_manual_runs && self.triggers.allow_manual_runs
    }

    pub fn is_builder_requested_run_allowed(&self) -> bool {
        self.policy.allow_builder_requested_runs && self.triggers.allow_builder_requested_runs
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ValidationMode {
    Disabled,
    DiagnosticOnly,
    RequiredGate,
    Mixed,
}

impl ValidationConfig {
    pub fn mode(&self) -> ValidationMode {
        if !self.enabled {
            ValidationMode::Disabled
        } else if !self.policy.gate_review_on_required_failure {
            ValidationMode::DiagnosticOnly
        } else {
            let has_required = self.commands.iter().any(|c| c.required);
            let has_optional = self.commands.iter().any(|c| !c.required);
            if has_required && has_optional {
                ValidationMode::Mixed
            } else {
                ValidationMode::RequiredGate
            }
        }
    }

    pub fn fingerprint(&self) -> String {
        let serialized = serde_json::to_string(self).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(serialized.as_bytes());
        format!("{:x}", hasher.finalize())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ValidationTriggerSource {
    Manual,
    PostBuild,
    BuilderRequested,
    PreReview,
    FinalValidation,
}

impl std::fmt::Display for ValidationTriggerSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Manual => write!(f, "MANUAL"),
            Self::PostBuild => write!(f, "POST_BUILD"),
            Self::BuilderRequested => write!(f, "BUILDER_REQUESTED"),
            Self::PreReview => write!(f, "PRE_REVIEW"),
            Self::FinalValidation => write!(f, "FINAL_VALIDATION"),
        }
    }
}

impl std::str::FromStr for ValidationTriggerSource {
    type Err = ValidationError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "MANUAL" => Ok(Self::Manual),
            "POST_BUILD" => Ok(Self::PostBuild),
            "BUILDER_REQUESTED" => Ok(Self::BuilderRequested),
            "PRE_REVIEW" => Ok(Self::PreReview),
            "FINAL_VALIDATION" => Ok(Self::FinalValidation),
            _ => Ok(Self::Manual),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ValidationRunStatus {
    Queued,
    Running,
    Pass,
    Fail,
    Timeout,
    Canceled,
    Interrupted,
}

impl std::fmt::Display for ValidationRunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Queued => write!(f, "QUEUED"),
            Self::Running => write!(f, "RUNNING"),
            Self::Pass => write!(f, "PASS"),
            Self::Fail => write!(f, "FAIL"),
            Self::Timeout => write!(f, "TIMEOUT"),
            Self::Canceled => write!(f, "CANCELED"),
            Self::Interrupted => write!(f, "INTERRUPTED"),
        }
    }
}

impl std::str::FromStr for ValidationRunStatus {
    type Err = ValidationError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "QUEUED" => Ok(Self::Queued),
            "RUNNING" => Ok(Self::Running),
            "PASS" => Ok(Self::Pass),
            "FAIL" => Ok(Self::Fail),
            "TIMEOUT" => Ok(Self::Timeout),
            "CANCELED" => Ok(Self::Canceled),
            "INTERRUPTED" => Ok(Self::Interrupted),
            _ => Err(ValidationError::Execution(format!(
                "Unknown run status: {}",
                s
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ValidationCommandStatus {
    Queued,
    Running,
    Pass,
    Fail,
    Timeout,
    Canceled,
    Interrupted,
    Skipped,
}

impl std::fmt::Display for ValidationCommandStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Queued => write!(f, "QUEUED"),
            Self::Running => write!(f, "RUNNING"),
            Self::Pass => write!(f, "PASS"),
            Self::Fail => write!(f, "FAIL"),
            Self::Timeout => write!(f, "TIMEOUT"),
            Self::Canceled => write!(f, "CANCELED"),
            Self::Interrupted => write!(f, "INTERRUPTED"),
            Self::Skipped => write!(f, "SKIPPED"),
        }
    }
}

impl std::str::FromStr for ValidationCommandStatus {
    type Err = ValidationError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "QUEUED" => Ok(Self::Queued),
            "RUNNING" => Ok(Self::Running),
            "PASS" => Ok(Self::Pass),
            "FAIL" => Ok(Self::Fail),
            "TIMEOUT" => Ok(Self::Timeout),
            "CANCELED" => Ok(Self::Canceled),
            "INTERRUPTED" => Ok(Self::Interrupted),
            "SKIPPED" => Ok(Self::Skipped),
            _ => Err(ValidationError::Execution(format!(
                "Unknown command status: {}",
                s
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationCommandExecutionRecord {
    pub execution_id: String,
    pub run_id: String,
    pub command_id: String,
    pub name: String,
    pub command_str: String,
    pub working_dir: Option<String>,
    pub required: bool,
    pub status: ValidationCommandStatus,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub is_truncated: bool,
    pub log_path: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationRunRecord {
    pub run_id: String,
    pub project_id: String,
    pub architecture_version: String,
    pub epoch_id: Option<String>,
    pub trigger_source: ValidationTriggerSource,
    pub status: ValidationRunStatus,
    pub is_gate_passed: bool,
    pub has_override: bool,
    pub git_head: Option<String>,
    pub git_dirty_fingerprint: Option<String>,
    pub config_fingerprint: Option<String>,
    pub log_path: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub duration_ms: u64,
    pub commands: Vec<ValidationCommandExecutionRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationGateOverrideRecord {
    pub override_id: String,
    pub project_id: String,
    pub run_id: String,
    pub git_fingerprint: String,
    pub reason: String,
    pub authorized_by: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationEvidenceSummary {
    pub schema_version: u32,
    pub run_id: String,
    pub trigger: String,
    pub architecture_version: String,
    pub epoch_id: Option<String>,
    pub validation_config_fingerprint: String,
    pub git_head: Option<String>,
    pub working_tree_fingerprint: String,
    pub overall_status: String,
    pub is_gate_passed: bool,
    pub has_override: bool,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub duration_ms: u64,
    pub commands: Vec<ValidationEvidenceCommandSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationEvidenceCommandSummary {
    pub command_id: String,
    pub name: String,
    pub required: bool,
    pub status: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
}

#[derive(Clone)]
pub struct ActiveValidationExecution {
    pub project_id: String,
    pub run_id: String,
    pub current_cancel_flag: Arc<AtomicBool>,
    pub all_cancel_flag: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct ActiveValidationRegistry {
    executions: HashMap<String, ActiveValidationExecution>,
}

impl ActiveValidationRegistry {
    pub fn new() -> Self {
        Self {
            executions: HashMap::new(),
        }
    }

    pub fn register(
        &mut self,
        project_id: &str,
        run_id: &str,
    ) -> Result<(Arc<AtomicBool>, Arc<AtomicBool>), ValidationError> {
        if let Some(existing) = self.executions.get(project_id) {
            return Err(ValidationError::Execution(format!(
                "Validation run '{}' is already actively running for project '{}'. Concurrent validation is forbidden.",
                existing.run_id, project_id
            )));
        }

        let current_cancel_flag = Arc::new(AtomicBool::new(false));
        let all_cancel_flag = Arc::new(AtomicBool::new(false));

        self.executions.insert(
            project_id.to_string(),
            ActiveValidationExecution {
                project_id: project_id.to_string(),
                run_id: run_id.to_string(),
                current_cancel_flag: current_cancel_flag.clone(),
                all_cancel_flag: all_cancel_flag.clone(),
            },
        );

        Ok((current_cancel_flag, all_cancel_flag))
    }

    pub fn unregister(&mut self, project_id: &str, run_id: &str) {
        if let Some(existing) = self.executions.get(project_id) {
            if existing.run_id == run_id {
                self.executions.remove(project_id);
            }
        }
    }

    pub fn stop_current(&self, target_id: &str) -> Result<String, ValidationError> {
        let exec = self
            .executions
            .values()
            .find(|e| e.project_id == target_id || e.run_id == target_id)
            .ok_or_else(|| {
                ValidationError::Execution(format!(
                    "No active validation run found matching '{}' to stop current command.",
                    target_id
                ))
            })?;
        exec.current_cancel_flag.store(true, Ordering::Relaxed);
        Ok(exec.run_id.clone())
    }

    pub fn stop_all(&self, target_id: &str) -> Result<String, ValidationError> {
        let exec = self
            .executions
            .values()
            .find(|e| e.project_id == target_id || e.run_id == target_id)
            .ok_or_else(|| {
                ValidationError::Execution(format!(
                    "No active validation run found matching '{}' to stop.",
                    target_id
                ))
            })?;
        exec.all_cancel_flag.store(true, Ordering::Relaxed);
        exec.current_cancel_flag.store(true, Ordering::Relaxed);
        Ok(exec.run_id.clone())
    }

    pub fn is_running(&self, project_id: &str) -> bool {
        self.executions.contains_key(project_id)
    }

    pub fn get_active_run_id(&self, project_id: &str) -> Option<String> {
        self.executions.get(project_id).map(|e| e.run_id.clone())
    }
}

pub type ValidationEventSink = Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>;

pub struct ValidationService;

impl ValidationService {
    /// Reads and validates `.coalition/implementation/validation.yaml`.
    /// If the file does not exist, returns `ValidationConfig` with `enabled = false` for backwards compatibility.
    pub fn read_validation_config<P: AsRef<Path>>(
        repo_path: P,
    ) -> Result<ValidationConfig, ValidationError> {
        let root = repo_path.as_ref();
        let config_path = root
            .join(".coalition")
            .join("implementation")
            .join("validation.yaml");

        if !config_path.exists() {
            return Ok(ValidationConfig::default());
        }

        let content = std::fs::read_to_string(&config_path)?;
        let config: ValidationConfig = serde_yaml::from_str(&content)?;

        if config.schema_version != 1 {
            return Err(ValidationError::UnsupportedSchemaVersion(
                config.schema_version,
            ));
        }

        // Validate command IDs are unique
        let mut seen_ids = HashSet::new();
        for cmd in &config.commands {
            if !seen_ids.insert(cmd.id.clone()) {
                return Err(ValidationError::DuplicateCommandId(cmd.id.clone()));
            }

            // Validate working directory safety
            if let Some(ref rel_dir) = cmd.working_directory {
                let trimmed = rel_dir.trim().replace('\\', "/");
                if trimmed.starts_with('/') || trimmed.starts_with("..") || trimmed.contains("/../")
                {
                    return Err(ValidationError::InvalidWorkingDirectory(rel_dir.clone()));
                }
                let target = root.join(&trimmed);
                // Target directory must be within root
                if let Ok(canon_root) = root.canonicalize() {
                    if let Ok(canon_target) = target.canonicalize() {
                        if !canon_target.starts_with(&canon_root) {
                            return Err(ValidationError::InvalidWorkingDirectory(rel_dir.clone()));
                        }
                    }
                }
            }
        }

        Ok(config)
    }

    /// Reconciles interrupted validation runs on startup/restart.
    pub fn reconcile_interrupted_runs(conn: &mut Connection) -> Result<usize, ValidationError> {
        let mut stmt = conn
            .prepare("SELECT run_id, project_id FROM validation_runs WHERE status = 'RUNNING'")
            .map_err(|e| ValidationError::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| ValidationError::Database(e.to_string()))?;

        let mut interrupted_runs = Vec::new();
        for pair in rows.flatten() {
            interrupted_runs.push(pair);
        }

        let now = chrono::Utc::now().to_rfc3339();
        let count = interrupted_runs.len();

        for (run_id, project_id) in interrupted_runs {
            conn.execute(
                "UPDATE validation_runs SET status = 'INTERRUPTED', completed_at = ?1 WHERE run_id = ?2",
                params![now, run_id],
            )
            .map_err(|e| ValidationError::Database(e.to_string()))?;

            conn.execute(
                "UPDATE validation_commands SET status = 'INTERRUPTED', completed_at = ?1 WHERE run_id = ?2 AND status = 'RUNNING'",
                params![now, run_id],
            )
            .map_err(|e| ValidationError::Database(e.to_string()))?;

            let meta = serde_json::json!({
                "run_id": run_id,
                "previous_status": "RUNNING",
                "terminal_status": "INTERRUPTED"
            });
            let _ = ActivityManager::record_event(
                conn,
                &project_id,
                "VALIDATION_RECONCILED",
                "RECOVERY",
                &format!(
                    "Reconciled orphaned validation run {} to INTERRUPTED",
                    run_id
                ),
                Some(&meta),
            );
        }

        Ok(count)
    }

    /// Executes a governed validation run.
    #[allow(clippy::too_many_arguments)]
    pub async fn execute_validation_run(
        db_conn: Arc<Mutex<crate::db::DbManager>>,
        registry: Arc<Mutex<ActiveValidationRegistry>>,
        project_id: &str,
        repo_path: &Path,
        trigger: ValidationTriggerSource,
        requested_command_ids: Option<Vec<String>>,
        event_sink: Option<ValidationEventSink>,
        app_data_dir: &Path,
    ) -> Result<ValidationRunRecord, ValidationError> {
        let config = Self::read_validation_config(repo_path)?;

        let git = GitAdapter::new().map_err(|e| ValidationError::Git(e.to_string()))?;
        let git_info = git
            .inspect_repo(repo_path)
            .map_err(|e| ValidationError::Git(e.to_string()))?;
        let git_head = git_info.head_commit;
        let dirty_fingerprint = git
            .compute_implementation_fingerprint(repo_path)
            .map_err(|e| ValidationError::Git(e.to_string()))?;

        let (arch_version, epoch_id) = {
            let db = db_conn.lock().await;
            let arch_ver = ProjectService::get_project_details(db.connection(), None, project_id)
                .map_err(|e| ValidationError::Database(e.to_string()))?
                .artifact
                .and_then(|a| a.current_architecture_version)
                .unwrap_or_else(|| "1.0".to_string());

            let ep_id: Option<String> = db
                .connection()
                .query_row(
                    "SELECT epoch_id FROM builder_epochs WHERE project_id = ?1 AND architecture_version = ?2 ORDER BY created_at DESC LIMIT 1",
                    params![project_id, &arch_ver],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| ValidationError::Database(e.to_string()))?;

            (arch_ver, ep_id)
        };

        let run_id = format!("val-{}", Uuid::new_v4());
        let started_at = chrono::Utc::now().to_rfc3339();
        let config_fp = config.fingerprint();

        // Enforce trigger permissions (Item 7)
        match trigger {
            ValidationTriggerSource::Manual if !config.is_manual_run_allowed() => {
                return Err(ValidationError::Execution(
                    "Manual validation runs are disabled in project configuration".to_string(),
                ));
            }
            ValidationTriggerSource::BuilderRequested
                if !config.is_builder_requested_run_allowed() =>
            {
                return Err(ValidationError::Execution(
                    "Builder-requested validation runs are disabled in project configuration"
                        .to_string(),
                ));
            }
            _ => {}
        }

        // If validation is disabled, record a zero-command pass run immediately
        if !config.enabled {
            let record = ValidationRunRecord {
                run_id: run_id.clone(),
                project_id: project_id.to_string(),
                architecture_version: arch_version.clone(),
                epoch_id: epoch_id.clone(),
                trigger_source: trigger,
                status: ValidationRunStatus::Pass,
                is_gate_passed: true,
                has_override: false,
                git_head: git_head.clone(),
                git_dirty_fingerprint: Some(dirty_fingerprint.clone()),
                config_fingerprint: Some(config_fp.clone()),
                log_path: None,
                started_at: started_at.clone(),
                completed_at: Some(chrono::Utc::now().to_rfc3339()),
                duration_ms: 0,
                commands: Vec::new(),
            };

            {
                let mut db = db_conn.lock().await;
                insert_validation_run(db.connection_mut(), &record)?;
                let meta = serde_json::json!({
                    "run_id": run_id,
                    "trigger": trigger.to_string(),
                });
                let _ = ActivityManager::record_event(
                    db.connection_mut(),
                    project_id,
                    "VALIDATION_DISABLED_BYPASSED",
                    "VALIDATION_SERVICE",
                    "Validation is disabled in project configuration; checks skipped",
                    Some(&meta),
                );
            }
            return Ok(record);
        }

        // Filter commands if specific IDs were requested
        let commands_to_run: Vec<ValidationCommandConfig> = match requested_command_ids {
            Some(ref req_ids) if !req_ids.is_empty() => {
                let id_set: HashSet<&String> = req_ids.iter().collect();
                config
                    .commands
                    .iter()
                    .filter(|c| id_set.contains(&c.id))
                    .cloned()
                    .collect()
            }
            _ => config.commands.clone(),
        };

        // Register in ActiveValidationRegistry
        let (current_cancel_flag, all_cancel_flag) = {
            let mut reg = registry.lock().await;
            reg.register(project_id, &run_id)?
        };

        // Guard unregistration on drop/exit
        struct RegistryGuard {
            registry: Arc<Mutex<ActiveValidationRegistry>>,
            project_id: String,
            run_id: String,
        }
        impl Drop for RegistryGuard {
            fn drop(&mut self) {
                let reg = self.registry.clone();
                let pid = self.project_id.clone();
                let rid = self.run_id.clone();
                tokio::spawn(async move {
                    let mut lock = reg.lock().await;
                    lock.unregister(&pid, &rid);
                });
            }
        }
        let _guard = RegistryGuard {
            registry: registry.clone(),
            project_id: project_id.to_string(),
            run_id: run_id.clone(),
        };

        // Initial operational log dir
        let run_log_dir = app_data_dir.join("validation-logs").join(&run_id);
        let _ = tokio::fs::create_dir_all(&run_log_dir).await;

        let mut initial_command_records = Vec::new();
        for cmd in &commands_to_run {
            initial_command_records.push(ValidationCommandExecutionRecord {
                execution_id: format!("exec-{}", Uuid::new_v4()),
                run_id: run_id.clone(),
                command_id: cmd.id.clone(),
                name: cmd.name.clone(),
                command_str: cmd.command.clone(),
                working_dir: cmd.working_directory.clone(),
                required: cmd.required,
                status: ValidationCommandStatus::Queued,
                exit_code: None,
                duration_ms: 0,
                is_truncated: false,
                log_path: Some(
                    run_log_dir
                        .join(format!("{}.log", cmd.id))
                        .to_string_lossy()
                        .to_string(),
                ),
                started_at: None,
                completed_at: None,
            });
        }

        let initial_run_record = ValidationRunRecord {
            run_id: run_id.clone(),
            project_id: project_id.to_string(),
            architecture_version: arch_version.clone(),
            epoch_id: epoch_id.clone(),
            trigger_source: trigger,
            status: ValidationRunStatus::Running,
            is_gate_passed: false,
            has_override: false,
            git_head: git_head.clone(),
            git_dirty_fingerprint: Some(dirty_fingerprint.clone()),
            config_fingerprint: Some(config_fp.clone()),
            log_path: Some(run_log_dir.to_string_lossy().to_string()),
            started_at: started_at.clone(),
            completed_at: None,
            duration_ms: 0,
            commands: initial_command_records.clone(),
        };

        {
            let mut db = db_conn.lock().await;
            insert_validation_run(db.connection_mut(), &initial_run_record)?;
            for cmd_rec in &initial_command_records {
                insert_validation_command(db.connection_mut(), cmd_rec)?;
            }
            let meta = serde_json::json!({
                "run_id": run_id,
                "trigger": trigger.to_string(),
                "command_count": commands_to_run.len()
            });
            let _ = ActivityManager::record_event(
                db.connection_mut(),
                project_id,
                "VALIDATION_STARTED",
                "VALIDATION_SERVICE",
                &format!(
                    "Started validation run {} with {} commands",
                    run_id,
                    commands_to_run.len()
                ),
                Some(&meta),
            );
        }

        if let Some(ref sink) = event_sink {
            sink(
                "validation://status",
                serde_json::json!({
                    "project_id": project_id,
                    "run_id": run_id,
                    "status": "RUNNING",
                    "architecture_version": arch_version,
                    "epoch_id": epoch_id,
                    "trigger_source": trigger.to_string(),
                    "git_head": git_head,
                    "commands": initial_command_records
                }),
            );
        }

        let run_start_instant = tokio::time::Instant::now();
        let mut executed_commands = Vec::new();
        let mut had_cancellation = false;

        for (idx, cmd) in commands_to_run.iter().enumerate() {
            let mut cmd_rec = initial_command_records[idx].clone();

            // If Stop All was requested, mark remaining queued commands as SKIPPED
            if all_cancel_flag.load(Ordering::Relaxed) {
                cmd_rec.status = ValidationCommandStatus::Skipped;
                cmd_rec.completed_at = Some(chrono::Utc::now().to_rfc3339());
                executed_commands.push(cmd_rec.clone());

                let mut db = db_conn.lock().await;
                update_validation_command(db.connection_mut(), &cmd_rec)?;
                continue;
            }

            // Reset current cancel flag for this command (so Stop Current only applied to the previous command)
            current_cancel_flag.store(false, Ordering::Relaxed);

            cmd_rec.status = ValidationCommandStatus::Running;
            cmd_rec.started_at = Some(chrono::Utc::now().to_rfc3339());

            {
                let mut db = db_conn.lock().await;
                update_validation_command(db.connection_mut(), &cmd_rec)?;
            }

            if let Some(ref sink) = event_sink {
                sink(
                    "validation://command-start",
                    serde_json::json!({
                        "project_id": project_id,
                        "run_id": run_id,
                        "command_id": cmd.id,
                        "name": cmd.name
                    }),
                );
            }

            let log_file_path = run_log_dir.join(format!("{}.log", cmd.id));
            let working_dir = match &cmd.working_directory {
                Some(rel) => repo_path.join(rel),
                None => repo_path.to_path_buf(),
            };

            #[cfg(target_os = "windows")]
            let (prog, args) = (
                PathBuf::from("cmd.exe"),
                vec!["/C".to_string(), cmd.command.clone()],
            );
            #[cfg(not(target_os = "windows"))]
            let (prog, args) = (
                PathBuf::from("sh"),
                vec!["-c".to_string(), cmd.command.clone()],
            );

            let (tx, mut rx) = tokio::sync::mpsc::channel::<ProcessOutputLine>(200);
            let sink_clone = event_sink.clone();
            let cmd_id_clone = cmd.id.clone();
            let run_id_clone = run_id.clone();
            let project_id_clone = project_id.to_string();

            tokio::spawn(async move {
                while let Some(line) = rx.recv().await {
                    if let Some(ref sink) = sink_clone {
                        sink(
                            "validation://output",
                            serde_json::json!({
                                "project_id": project_id_clone,
                                "run_id": run_id_clone,
                                "command_id": cmd_id_clone,
                                "kind": match line.kind {
                                    ProcessOutputKind::Stdout => "stdout",
                                    ProcessOutputKind::Stderr => "stderr",
                                },
                                "line": line.line,
                                "timestamp_ms": line.timestamp_ms
                            }),
                        );
                    }
                }
            });

            let proc_res = ProcessRunner::run_turn_stream_advanced(
                &prog,
                &args,
                Some(&working_dir),
                None,
                Duration::from_secs(cmd.timeout_seconds),
                current_cancel_flag.clone(),
                Some(tx),
                Some(&log_file_path),
                Duration::from_millis(1500),
            )
            .await;

            let (status, exit_code, is_trunc, dur_ms) = match proc_res {
                Ok(res) => {
                    let st = if res.canceled {
                        had_cancellation = true;
                        ValidationCommandStatus::Canceled
                    } else if res.timed_out {
                        had_cancellation = true;
                        ValidationCommandStatus::Timeout
                    } else if res.exit_code == Some(0) {
                        ValidationCommandStatus::Pass
                    } else {
                        ValidationCommandStatus::Fail
                    };
                    (st, res.exit_code, res.is_truncated, res.duration_ms)
                }
                Err(e) => {
                    eprintln!("Error executing validation command {}: {}", cmd.id, e);
                    (ValidationCommandStatus::Fail, None, false, 0)
                }
            };

            cmd_rec.status = status;
            cmd_rec.exit_code = exit_code;
            cmd_rec.is_truncated = is_trunc;
            cmd_rec.duration_ms = dur_ms;
            cmd_rec.completed_at = Some(chrono::Utc::now().to_rfc3339());

            {
                let mut db = db_conn.lock().await;
                update_validation_command(db.connection_mut(), &cmd_rec)?;
            }

            if let Some(ref sink) = event_sink {
                sink(
                    "validation://command-finish",
                    serde_json::json!({
                        "project_id": project_id,
                        "run_id": run_id,
                        "command_id": cmd.id,
                        "status": status.to_string(),
                        "exit_code": exit_code,
                        "duration_ms": dur_ms
                    }),
                );
            }

            executed_commands.push(cmd_rec);
        }

        let total_duration_ms = run_start_instant.elapsed().as_millis() as u64;

        // Run evaluation logic (Directives 1 & 7)
        let overall_status = if all_cancel_flag.load(Ordering::Relaxed) {
            ValidationRunStatus::Canceled
        } else if had_cancellation {
            // Any canceled or timed out command means the overall run can NEVER be PASS
            ValidationRunStatus::Canceled
        } else if executed_commands
            .iter()
            .all(|c| c.status == ValidationCommandStatus::Pass)
        {
            ValidationRunStatus::Pass
        } else {
            ValidationRunStatus::Fail
        };

        // Gate evaluation - requires ALL configured required commands to have executed and passed (Item 6)
        let required_ids: std::collections::HashSet<&str> = config
            .commands
            .iter()
            .filter(|c| c.required)
            .map(|c| c.id.as_str())
            .collect();
        let passed_required_ids: std::collections::HashSet<&str> = executed_commands
            .iter()
            .filter(|c| c.required && c.status == ValidationCommandStatus::Pass)
            .map(|c| c.command_id.as_str())
            .collect();
        let all_required_executed_and_passed = required_ids.is_subset(&passed_required_ids);

        let is_gate_passed = if !config.policy.gate_review_on_required_failure {
            // Diagnostic-only mode allows progression regardless of failure (Directive 7)
            true
        } else if overall_status == ValidationRunStatus::Canceled {
            false
        } else {
            all_required_executed_and_passed
        };

        let completed_at = chrono::Utc::now().to_rfc3339();

        let final_record = ValidationRunRecord {
            run_id: run_id.clone(),
            project_id: project_id.to_string(),
            architecture_version: arch_version.clone(),
            epoch_id: epoch_id.clone(),
            trigger_source: trigger,
            status: overall_status,
            is_gate_passed,
            has_override: false,
            git_head: git_head.clone(),
            git_dirty_fingerprint: Some(dirty_fingerprint.clone()),
            config_fingerprint: Some(config_fp.clone()),
            log_path: Some(run_log_dir.to_string_lossy().to_string()),
            started_at: started_at.clone(),
            completed_at: Some(completed_at.clone()),
            duration_ms: total_duration_ms,
            commands: executed_commands.clone(),
        };

        {
            let mut db = db_conn.lock().await;
            update_validation_run(db.connection_mut(), &final_record)?;
            let meta = serde_json::json!({
                "run_id": run_id,
                "status": overall_status.to_string(),
                "gate_passed": is_gate_passed,
                "duration_ms": total_duration_ms
            });
            let _ = ActivityManager::record_event(
                db.connection_mut(),
                project_id,
                "VALIDATION_COMPLETED",
                "VALIDATION_SERVICE",
                &format!(
                    "Validation run {} completed with status {}",
                    run_id, overall_status
                ),
                Some(&meta),
            );
        }

        // Write durable crash-safe evidence into .coalition/evidence/validation-<run-id>/summary.yaml
        Self::write_durable_evidence(repo_path, &final_record)?;

        if let Some(ref sink) = event_sink {
            sink(
                "validation://finish",
                serde_json::json!({
                    "project_id": project_id,
                    "run_id": run_id,
                    "status": overall_status.to_string(),
                    "is_gate_passed": is_gate_passed,
                    "duration_ms": total_duration_ms,
                    "commands": executed_commands
                }),
            );
        }

        {
            let mut lock = registry.lock().await;
            lock.unregister(project_id, &run_id);
        }

        Ok(final_record)
    }

    /// Atomically writes portable, sanitized validation evidence into `.coalition/evidence/validation-<run-id>/summary.yaml`.
    pub fn write_durable_evidence(
        repo_path: &Path,
        record: &ValidationRunRecord,
    ) -> Result<PathBuf, ValidationError> {
        let evidence_dir = repo_path
            .join(".coalition")
            .join("evidence")
            .join(format!("validation-{}", record.run_id));
        std::fs::create_dir_all(&evidence_dir)?;

        let summary = ValidationEvidenceSummary {
            schema_version: 1,
            run_id: record.run_id.clone(),
            trigger: record.trigger_source.to_string(),
            architecture_version: record.architecture_version.clone(),
            epoch_id: record.epoch_id.clone(),
            validation_config_fingerprint: record.config_fingerprint.clone().unwrap_or_default(),
            git_head: record.git_head.clone(),
            working_tree_fingerprint: record.git_dirty_fingerprint.clone().unwrap_or_default(),
            overall_status: record.status.to_string(),
            is_gate_passed: record.is_gate_passed,
            has_override: record.has_override,
            started_at: record.started_at.clone(),
            completed_at: record.completed_at.clone(),
            duration_ms: record.duration_ms,
            commands: record
                .commands
                .iter()
                .map(|c| ValidationEvidenceCommandSummary {
                    command_id: c.command_id.clone(),
                    name: c.name.clone(),
                    required: c.required,
                    status: c.status.to_string(),
                    exit_code: c.exit_code,
                    duration_ms: c.duration_ms,
                })
                .collect(),
        };

        let raw_yaml = serde_yaml::to_string(&summary)?;
        let sanitized = safe_sanitize_text(&raw_yaml);

        // Crash-safe atomic write pattern
        let final_path = evidence_dir.join("summary.yaml");
        let temp_path = evidence_dir.join(format!(".summary-{}.tmp", Uuid::new_v4()));

        std::fs::write(&temp_path, sanitized)?;
        std::fs::rename(&temp_path, &final_path)?;

        Ok(final_path)
    }

    /// Overrides a failed validation gate for a specific run.
    /// Fails closed if the implementation has changed since validation ran (Directive 4).
    pub fn override_validation_gate(
        conn: &mut Connection,
        repo_path: &Path,
        project_id: &str,
        run_id: &str,
        reason: &str,
        authorized_by: &str,
    ) -> Result<ValidationGateOverrideRecord, ValidationError> {
        let run = get_validation_run(conn, run_id)?
            .ok_or_else(|| ValidationError::NotFound(run_id.to_string()))?;

        if run.project_id != project_id {
            return Err(ValidationError::Validation(format!(
                "Validation run {} does not belong to project {}",
                run_id, project_id
            )));
        }

        if run.status != ValidationRunStatus::Fail {
            return Err(ValidationError::Validation(format!(
                "Cannot override validation run {} because it is not in Fail status (current status: {})",
                run_id, run.status
            )));
        }

        let current_arch_version = crate::core::artifacts::ArtifactManager::resolve_coalition_dir(repo_path)
            .ok()
            .and_then(|dir| {
                crate::core::artifacts::ArtifactManager::read_project_yaml(dir.join("project.yaml"))
                    .ok()
            })
            .and_then(|py| py.current_architecture_version)
            .unwrap_or_else(|| {
                conn.query_row(
                    "SELECT architecture_version FROM builder_epochs WHERE project_id = ?1 ORDER BY created_at DESC LIMIT 1",
                    params![project_id],
                    |r| r.get(0),
                )
                .unwrap_or_else(|_| "1.0".to_string())
            });

        if run.architecture_version != current_arch_version {
            return Err(ValidationError::StaleEvidence(format!(
                "Cannot override validation gate: run architecture version ({}) does not match current project architecture version ({})",
                run.architecture_version, current_arch_version
            )));
        }

        let current_epoch_id: Option<String> = crate::core::freeze::FreezeService::get_builder_packet(
            repo_path,
            Some(&current_arch_version),
        )
        .map(|bp| Some(bp.metadata.builder_epoch_id))
        .unwrap_or_else(|_| {
            conn.query_row(
                "SELECT epoch_id FROM builder_epochs WHERE project_id = ?1 AND architecture_version = ?2 ORDER BY created_at DESC LIMIT 1",
                params![project_id, &current_arch_version],
                |r| r.get(0),
            )
            .optional()
            .unwrap_or(None)
        });

        if let Some(ref ep) = current_epoch_id {
            if run.epoch_id.as_deref() != Some(ep) {
                return Err(ValidationError::StaleEvidence(format!(
                    "Cannot override validation gate: run Builder epoch ({:?}) does not match current Builder epoch ({})",
                    run.epoch_id, ep
                )));
            }
        }

        let config = Self::read_validation_config(repo_path)?;
        if run.config_fingerprint.as_deref() != Some(&config.fingerprint()) {
            return Err(ValidationError::StaleEvidence(format!(
                "Cannot override validation gate: configuration fingerprint has changed from {:?} to {}",
                run.config_fingerprint, config.fingerprint()
            )));
        }

        let git = GitAdapter::new().map_err(|e| ValidationError::Git(e.to_string()))?;
        let current_info = git
            .inspect_repo(repo_path)
            .map_err(|e| ValidationError::Git(e.to_string()))?;
        if run.git_head != current_info.head_commit {
            return Err(ValidationError::StaleEvidence(format!(
                "Cannot override validation gate: Git HEAD has changed from {:?} to {:?}",
                run.git_head, current_info.head_commit
            )));
        }

        let current_dirty = git
            .compute_implementation_fingerprint(repo_path)
            .map_err(|e| ValidationError::Git(e.to_string()))?;

        // Invariant: required validation evidence must match the code being reviewed (Directive 4)
        if let Some(ref recorded_dirty) = run.git_dirty_fingerprint {
            if recorded_dirty != &current_dirty {
                return Err(ValidationError::StaleEvidence(format!(
                    "Cannot override validation gate: working tree fingerprint has changed from {} to {}",
                    recorded_dirty, current_dirty
                )));
            }
        }

        let override_id = format!("ovr-{}", Uuid::new_v4());
        let created_at = chrono::Utc::now().to_rfc3339();

        let rec = ValidationGateOverrideRecord {
            override_id: override_id.clone(),
            project_id: project_id.to_string(),
            run_id: run_id.to_string(),
            git_fingerprint: current_dirty,
            reason: reason.to_string(),
            authorized_by: authorized_by.to_string(),
            created_at,
        };

        insert_validation_override(conn, &rec)?;

        // Update run record in DB
        conn.execute(
            "UPDATE validation_runs SET has_override = 1, is_gate_passed = 1 WHERE run_id = ?1",
            params![run_id],
        )
        .map_err(|e| ValidationError::Database(e.to_string()))?;

        let meta = serde_json::json!({
            "override_id": override_id,
            "run_id": run_id,
            "reason": reason,
            "authorized_by": authorized_by
        });
        let _ = ActivityManager::record_event(
            conn,
            project_id,
            "VALIDATION_GATE_OVERRIDDEN",
            authorized_by,
            &format!(
                "Human authorized validation gate override for run {}: {}",
                run_id, reason
            ),
            Some(&meta),
        );

        // Authoritatively verify review gate passes before transitioning workflow
        let epoch_to_check = current_epoch_id
            .as_deref()
            .or(run.epoch_id.as_deref())
            .unwrap_or("");
        Self::check_review_gate(
            conn,
            repo_path,
            project_id,
            &current_arch_version,
            epoch_to_check,
        )?;

        // If workflow state is Validating, advance it to WaitingForReview via SubmitForReview
        if let Ok(current_wf) = crate::core::workflow::get_workflow_state(conn, project_id) {
            if current_wf.state == crate::core::workflow::WorkflowState::Validating {
                let _ = crate::core::workflow::apply_workflow_action(
                    conn,
                    project_id,
                    crate::core::workflow::WorkflowAction::SubmitForReview,
                    authorized_by,
                );
            }
        }

        Ok(rec)
    }

    /// Evaluates if validation satisfies the review gate for the current project state (Directive 4).
    pub fn check_review_gate(
        conn: &Connection,
        repo_path: &Path,
        project_id: &str,
        arch_version: &str,
        epoch_id: &str,
    ) -> Result<Option<ValidationRunRecord>, ValidationError> {
        let config = Self::read_validation_config(repo_path)?;
        if !config.enabled || !config.policy.gate_review_on_required_failure {
            // Validation is disabled or diagnostic-only: review gate is automatically satisfied
            return Ok(None);
        }

        let git = GitAdapter::new().map_err(|e| ValidationError::Git(e.to_string()))?;
        let current_info = git
            .inspect_repo(repo_path)
            .map_err(|e| ValidationError::Git(e.to_string()))?;
        let current_dirty = git
            .compute_implementation_fingerprint(repo_path)
            .map_err(|e| ValidationError::Git(e.to_string()))?;

        let latest_run = get_latest_validation_run(conn, project_id)?;
        let run = match latest_run {
            Some(r) => r,
            None => {
                return Err(ValidationError::Execution(
                    "Required validation gate enabled, but no validation runs exist for this project"
                        .to_string(),
                ));
            }
        };

        // Check bindings (Directive 4)
        if run.architecture_version != arch_version {
            return Err(ValidationError::StaleEvidence(format!(
                "Latest validation run architecture version ({}) does not match current ({})",
                run.architecture_version, arch_version
            )));
        }

        if run.epoch_id.as_deref() != Some(epoch_id) {
            return Err(ValidationError::StaleEvidence(format!(
                "Latest validation run epoch ID ({:?}) does not match current ({})",
                run.epoch_id, epoch_id
            )));
        }

        if run.config_fingerprint.as_deref() != Some(&config.fingerprint()) {
            return Err(ValidationError::StaleEvidence(format!(
                "Latest validation run configuration fingerprint ({:?}) does not match current ({})",
                run.config_fingerprint, config.fingerprint()
            )));
        }

        if run.git_head != current_info.head_commit {
            return Err(ValidationError::StaleEvidence(format!(
                "Latest validation run Git HEAD ({:?}) does not match current Git HEAD ({:?})",
                run.git_head, current_info.head_commit
            )));
        }

        if run.git_dirty_fingerprint.as_deref() != Some(&current_dirty) {
            return Err(ValidationError::StaleEvidence(format!(
                "Latest validation run implementation fingerprint ({:?}) does not match current ({})",
                run.git_dirty_fingerprint, current_dirty
            )));
        }

        if !run.is_gate_passed && !run.has_override {
            return Err(ValidationError::Execution(format!(
                "Latest validation run {} failed required checks and has no human override",
                run.run_id
            )));
        }

        Ok(Some(run))
    }

    /// Generates a bounded, sanitized validation diagnostic packet for Antigravity.
    pub fn generate_diagnostic_packet(run: &ValidationRunRecord, max_tail_bytes: usize) -> String {
        let mut out = String::new();
        out.push_str("# Coalition Validation Diagnostic Report\n\n");
        out.push_str(&format!("- **Run ID**: `{}`\n", run.run_id));
        out.push_str(&format!(
            "- **Architecture Version**: `{}`\n",
            run.architecture_version
        ));
        if let Some(ref epoch) = run.epoch_id {
            out.push_str(&format!("- **Builder Epoch**: `{}`\n", epoch));
        }
        if let Some(ref head) = run.git_head {
            out.push_str(&format!("- **Git HEAD**: `{}`\n", head));
        }
        out.push_str(&format!("- **Overall Status**: `{}`\n\n", run.status));

        out.push_str("## Failed Commands\n\n");
        let failed_cmds: Vec<&ValidationCommandExecutionRecord> = run
            .commands
            .iter()
            .filter(|c| {
                c.status == ValidationCommandStatus::Fail
                    || c.status == ValidationCommandStatus::Timeout
            })
            .collect();

        if failed_cmds.is_empty() {
            out.push_str("No failed commands reported.\n");
        } else {
            for cmd in failed_cmds {
                out.push_str(&format!(
                    "### Command: `{}` ({})\n\n",
                    cmd.command_id, cmd.name
                ));
                out.push_str(&format!("- **Required**: {}\n", cmd.required));
                out.push_str(&format!("- **Status**: {}\n", cmd.status));
                out.push_str(&format!("- **Exit Code**: {:?}\n", cmd.exit_code));
                out.push_str(&format!(
                    "- **Configured Command**: `{}`\n",
                    cmd.command_str
                ));
                if let Some(ref wd) = cmd.working_dir {
                    out.push_str(&format!("- **Working Directory**: `{}`\n", wd));
                }

                // Read output tail from disk log if available
                if let Some(ref lp) = cmd.log_path {
                    if let Ok(log_content) = std::fs::read_to_string(lp) {
                        let tail = if log_content.len() > max_tail_bytes {
                            let start = log_content.len() - max_tail_bytes;
                            &log_content[start..]
                        } else {
                            &log_content
                        };
                        let sanitized_tail = safe_sanitize_text(tail);
                        out.push_str("\n```text\n");
                        out.push_str(&sanitized_tail);
                        if !sanitized_tail.ends_with('\n') {
                            out.push('\n');
                        }
                        out.push_str("```\n\n");
                    }
                }
            }
        }

        out
    }

    pub fn generate_diagnostic_packet_for_run(
        conn: &Connection,
        run_id: &str,
        max_tail_bytes: usize,
    ) -> Result<String, ValidationError> {
        let run = get_validation_run(conn, run_id)?
            .ok_or_else(|| ValidationError::NotFound(run_id.to_string()))?;
        Ok(Self::generate_diagnostic_packet(&run, max_tail_bytes))
    }

    /// Orchestrates post-build validation workflow transition (Building -> Validating -> WaitingForReview).
    pub async fn run_post_build_validation(
        db_conn: Arc<tokio::sync::Mutex<crate::db::DbManager>>,
        registry: Arc<tokio::sync::Mutex<ActiveValidationRegistry>>,
        project_id: &str,
        repo_path: &Path,
        event_sink: Option<ValidationEventSink>,
        app_data_dir: &Path,
    ) -> Result<Option<ValidationRunRecord>, ValidationError> {
        let config = Self::read_validation_config(repo_path)?;
        if !config.enabled {
            return Ok(None);
        }

        // 1. Transition workflow from Building to Validating if currently Building
        {
            let mut db = db_conn.lock().await;
            if let Ok(st) = crate::core::workflow::get_workflow_state(db.connection(), project_id) {
                if st.state == crate::core::workflow::WorkflowState::Building {
                    crate::core::workflow::apply_workflow_action(
                        db.connection_mut(),
                        project_id,
                        crate::core::workflow::WorkflowAction::StartValidation,
                        "system",
                    )
                    .map_err(|e| ValidationError::Execution(e.to_string()))?;
                }
            }
        }

        // 2. Start validation run with ValidationTriggerSource::PostBuild
        let record = Self::execute_validation_run(
            db_conn.clone(),
            registry,
            project_id,
            repo_path,
            ValidationTriggerSource::PostBuild,
            None,
            event_sink,
            app_data_dir,
        )
        .await?;

        // 3. If gate passed, transition workflow from Validating to WaitingForReview
        if record.is_gate_passed {
            let mut db = db_conn.lock().await;
            if let Ok(st) = crate::core::workflow::get_workflow_state(db.connection(), project_id) {
                if st.state == crate::core::workflow::WorkflowState::Validating {
                    crate::core::workflow::apply_workflow_action(
                        db.connection_mut(),
                        project_id,
                        crate::core::workflow::WorkflowAction::SubmitForReview,
                        "system",
                    )
                    .map_err(|e| ValidationError::Execution(e.to_string()))?;
                }
            }
        }

        Ok(Some(record))
    }
}

// Database helper functions

pub fn insert_validation_run(
    conn: &Connection,
    record: &ValidationRunRecord,
) -> Result<(), ValidationError> {
    conn.execute(
        "INSERT INTO validation_runs (
            run_id, project_id, architecture_version, epoch_id, trigger_source,
            status, is_gate_passed, has_override, git_head, git_dirty_fingerprint,
            config_fingerprint, log_path, started_at, completed_at, duration_ms
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            record.run_id,
            record.project_id,
            record.architecture_version,
            record.epoch_id,
            record.trigger_source.to_string(),
            record.status.to_string(),
            if record.is_gate_passed { 1 } else { 0 },
            if record.has_override { 1 } else { 0 },
            record.git_head,
            record.git_dirty_fingerprint,
            record.config_fingerprint,
            record.log_path,
            record.started_at,
            record.completed_at,
            record.duration_ms as i64,
        ],
    )
    .map_err(|e| ValidationError::Database(e.to_string()))?;
    Ok(())
}

pub fn update_validation_run(
    conn: &Connection,
    record: &ValidationRunRecord,
) -> Result<(), ValidationError> {
    conn.execute(
        "UPDATE validation_runs SET
            status = ?1, is_gate_passed = ?2, has_override = ?3,
            completed_at = ?4, duration_ms = ?5
        WHERE run_id = ?6",
        params![
            record.status.to_string(),
            if record.is_gate_passed { 1 } else { 0 },
            if record.has_override { 1 } else { 0 },
            record.completed_at,
            record.duration_ms as i64,
            record.run_id,
        ],
    )
    .map_err(|e| ValidationError::Database(e.to_string()))?;
    Ok(())
}

pub fn insert_validation_command(
    conn: &Connection,
    record: &ValidationCommandExecutionRecord,
) -> Result<(), ValidationError> {
    conn.execute(
        "INSERT INTO validation_commands (
            execution_id, run_id, command_id, name, command_str,
            working_dir, required, status, exit_code, duration_ms,
            is_truncated, log_path, started_at, completed_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            record.execution_id,
            record.run_id,
            record.command_id,
            record.name,
            record.command_str,
            record.working_dir,
            if record.required { 1 } else { 0 },
            record.status.to_string(),
            record.exit_code,
            record.duration_ms as i64,
            if record.is_truncated { 1 } else { 0 },
            record.log_path,
            record.started_at,
            record.completed_at,
        ],
    )
    .map_err(|e| ValidationError::Database(e.to_string()))?;
    Ok(())
}

pub fn update_validation_command(
    conn: &Connection,
    record: &ValidationCommandExecutionRecord,
) -> Result<(), ValidationError> {
    conn.execute(
        "UPDATE validation_commands SET
            status = ?1, exit_code = ?2, duration_ms = ?3,
            is_truncated = ?4, started_at = ?5, completed_at = ?6
        WHERE execution_id = ?7",
        params![
            record.status.to_string(),
            record.exit_code,
            record.duration_ms as i64,
            if record.is_truncated { 1 } else { 0 },
            record.started_at,
            record.completed_at,
            record.execution_id,
        ],
    )
    .map_err(|e| ValidationError::Database(e.to_string()))?;
    Ok(())
}

pub fn insert_validation_override(
    conn: &Connection,
    record: &ValidationGateOverrideRecord,
) -> Result<(), ValidationError> {
    conn.execute(
        "INSERT INTO validation_gate_overrides (
            override_id, project_id, run_id, git_fingerprint,
            reason, authorized_by, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            record.override_id,
            record.project_id,
            record.run_id,
            record.git_fingerprint,
            record.reason,
            record.authorized_by,
            record.created_at,
        ],
    )
    .map_err(|e| ValidationError::Database(e.to_string()))?;
    Ok(())
}

pub fn get_validation_run(
    conn: &Connection,
    run_id: &str,
) -> Result<Option<ValidationRunRecord>, ValidationError> {
    let mut stmt = conn
        .prepare(
            "SELECT run_id, project_id, architecture_version, epoch_id, trigger_source,
                    status, is_gate_passed, has_override, git_head, git_dirty_fingerprint,
                    config_fingerprint, log_path, started_at, completed_at, duration_ms
             FROM validation_runs WHERE run_id = ?1",
        )
        .map_err(|e| ValidationError::Database(e.to_string()))?;

    let run_opt = stmt
        .query_row(params![run_id], |row| {
            let trigger_str: String = row.get(4)?;
            let status_str: String = row.get(5)?;
            Ok(ValidationRunRecord {
                run_id: row.get(0)?,
                project_id: row.get(1)?,
                architecture_version: row.get(2)?,
                epoch_id: row.get(3)?,
                trigger_source: trigger_str
                    .parse()
                    .unwrap_or(ValidationTriggerSource::Manual),
                status: status_str.parse().unwrap_or(ValidationRunStatus::Fail),
                is_gate_passed: row.get::<_, i64>(6)? != 0,
                has_override: row.get::<_, i64>(7)? != 0,
                git_head: row.get(8)?,
                git_dirty_fingerprint: row.get(9)?,
                config_fingerprint: row.get(10)?,
                log_path: row.get(11)?,
                started_at: row.get(12)?,
                completed_at: row.get(13)?,
                duration_ms: row.get::<_, i64>(14)? as u64,
                commands: Vec::new(),
            })
        })
        .optional()
        .map_err(|e| ValidationError::Database(e.to_string()))?;

    if let Some(mut run) = run_opt {
        run.commands = list_validation_commands_for_run(conn, &run.run_id)?;
        Ok(Some(run))
    } else {
        Ok(None)
    }
}

pub fn get_latest_validation_run(
    conn: &Connection,
    project_id: &str,
) -> Result<Option<ValidationRunRecord>, ValidationError> {
    let latest_id: Option<String> = conn
        .query_row(
            "SELECT run_id FROM validation_runs WHERE project_id = ?1 ORDER BY started_at DESC LIMIT 1",
            params![project_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| ValidationError::Database(e.to_string()))?;

    if let Some(id) = latest_id {
        get_validation_run(conn, &id)
    } else {
        Ok(None)
    }
}

pub fn list_validation_runs_for_project(
    conn: &Connection,
    project_id: &str,
    limit: usize,
) -> Result<Vec<ValidationRunRecord>, ValidationError> {
    let mut stmt = conn
        .prepare(
            "SELECT run_id, project_id, architecture_version, epoch_id, trigger_source,
                    status, is_gate_passed, has_override, git_head, git_dirty_fingerprint,
                    config_fingerprint, log_path, started_at, completed_at, duration_ms
             FROM validation_runs WHERE project_id = ?1 ORDER BY started_at DESC LIMIT ?2",
        )
        .map_err(|e| ValidationError::Database(e.to_string()))?;

    let rows = stmt
        .query_map(params![project_id, limit as i64], |row| {
            let trigger_str: String = row.get(4)?;
            let status_str: String = row.get(5)?;
            Ok(ValidationRunRecord {
                run_id: row.get(0)?,
                project_id: row.get(1)?,
                architecture_version: row.get(2)?,
                epoch_id: row.get(3)?,
                trigger_source: trigger_str
                    .parse()
                    .unwrap_or(ValidationTriggerSource::Manual),
                status: status_str.parse().unwrap_or(ValidationRunStatus::Fail),
                is_gate_passed: row.get::<_, i64>(6)? != 0,
                has_override: row.get::<_, i64>(7)? != 0,
                git_head: row.get(8)?,
                git_dirty_fingerprint: row.get(9)?,
                config_fingerprint: row.get(10)?,
                log_path: row.get(11)?,
                started_at: row.get(12)?,
                completed_at: row.get(13)?,
                duration_ms: row.get::<_, i64>(14)? as u64,
                commands: Vec::new(),
            })
        })
        .map_err(|e| ValidationError::Database(e.to_string()))?;

    let mut list = Vec::new();
    for r in rows {
        let mut rec = r.map_err(|e| ValidationError::Database(e.to_string()))?;
        rec.commands = list_validation_commands_for_run(conn, &rec.run_id)?;
        list.push(rec);
    }
    Ok(list)
}

pub fn list_validation_commands_for_run(
    conn: &Connection,
    run_id: &str,
) -> Result<Vec<ValidationCommandExecutionRecord>, ValidationError> {
    let mut stmt = conn
        .prepare(
            "SELECT execution_id, run_id, command_id, name, command_str,
                    working_dir, required, status, exit_code, duration_ms,
                    is_truncated, log_path, started_at, completed_at
             FROM validation_commands WHERE run_id = ?1 ORDER BY rowid ASC",
        )
        .map_err(|e| ValidationError::Database(e.to_string()))?;

    let rows = stmt
        .query_map(params![run_id], |row| {
            let status_str: String = row.get(7)?;
            Ok(ValidationCommandExecutionRecord {
                execution_id: row.get(0)?,
                run_id: row.get(1)?,
                command_id: row.get(2)?,
                name: row.get(3)?,
                command_str: row.get(4)?,
                working_dir: row.get(5)?,
                required: row.get::<_, i64>(6)? != 0,
                status: status_str.parse().unwrap_or(ValidationCommandStatus::Fail),
                exit_code: row.get(8)?,
                duration_ms: row.get::<_, i64>(9)? as u64,
                is_truncated: row.get::<_, i64>(10)? != 0,
                log_path: row.get(11)?,
                started_at: row.get(12)?,
                completed_at: row.get(13)?,
            })
        })
        .map_err(|e| ValidationError::Database(e.to_string()))?;

    let mut commands = Vec::new();
    for r in rows {
        commands.push(r.map_err(|e| ValidationError::Database(e.to_string()))?);
    }
    Ok(commands)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_validation_config_parsing_and_schema_validation() {
        let dir = tempdir().unwrap();
        let repo_path = dir.path();
        let impl_dir = repo_path.join(".coalition").join("implementation");
        std::fs::create_dir_all(&impl_dir).unwrap();

        // 1. Absent config returns disabled config safely
        let absent = ValidationService::read_validation_config(repo_path).unwrap();
        assert!(!absent.enabled);
        assert_eq!(absent.mode(), ValidationMode::Disabled);

        // 2. Valid config
        let valid_yaml = r#"
schema_version: 1
enabled: true
policy:
  gate_review_on_required_failure: true
  allow_manual_runs: true
  allow_builder_requested_runs: true
commands:
  - id: test-unit
    name: Unit tests
    command: cargo test
    required: true
    timeout_seconds: 300
  - id: test-lint
    name: Linter
    command: cargo clippy
    required: false
    timeout_seconds: 120
"#;
        std::fs::write(impl_dir.join("validation.yaml"), valid_yaml).unwrap();

        let cfg = ValidationService::read_validation_config(repo_path).unwrap();
        assert!(cfg.enabled);
        assert_eq!(cfg.commands.len(), 2);
        assert_eq!(cfg.mode(), ValidationMode::Mixed);
        assert_eq!(cfg.commands[0].id, "test-unit");
        assert!(cfg.commands[0].required);
        assert!(!cfg.commands[1].required);

        // 3. Unsupported schema version
        let invalid_ver_yaml = valid_yaml.replace("schema_version: 1", "schema_version: 2");
        std::fs::write(impl_dir.join("validation.yaml"), invalid_ver_yaml).unwrap();
        let err_ver = ValidationService::read_validation_config(repo_path).unwrap_err();
        assert!(matches!(
            err_ver,
            ValidationError::UnsupportedSchemaVersion(2)
        ));

        // 4. Duplicate command ID
        let dup_id_yaml = r#"
schema_version: 1
enabled: true
commands:
  - id: test-dup
    name: First
    command: cargo test
  - id: test-dup
    name: Second
    command: cargo test
"#;
        std::fs::write(impl_dir.join("validation.yaml"), dup_id_yaml).unwrap();
        let err_dup = ValidationService::read_validation_config(repo_path).unwrap_err();
        assert!(matches!(err_dup, ValidationError::DuplicateCommandId(_)));

        // 5. Escaping working directory
        let escape_dir_yaml = r#"
schema_version: 1
enabled: true
commands:
  - id: test-escape
    name: Escaping
    command: cargo test
    working_directory: "../outside"
"#;
        std::fs::write(impl_dir.join("validation.yaml"), escape_dir_yaml).unwrap();
        let err_esc = ValidationService::read_validation_config(repo_path).unwrap_err();
        assert!(matches!(
            err_esc,
            ValidationError::InvalidWorkingDirectory(_)
        ));
    }

    #[test]
    fn test_validation_mode_detection() {
        let mut cfg = ValidationConfig::default();
        assert_eq!(cfg.mode(), ValidationMode::Disabled);

        cfg.enabled = true;
        cfg.policy.gate_review_on_required_failure = false;
        assert_eq!(cfg.mode(), ValidationMode::DiagnosticOnly);

        cfg.policy.gate_review_on_required_failure = true;
        cfg.commands.push(ValidationCommandConfig {
            id: "c1".to_string(),
            name: "C1".to_string(),
            command: "echo 1".to_string(),
            working_directory: None,
            required: true,
            timeout_seconds: 60,
        });
        assert_eq!(cfg.mode(), ValidationMode::RequiredGate);

        cfg.commands.push(ValidationCommandConfig {
            id: "c2".to_string(),
            name: "C2".to_string(),
            command: "echo 2".to_string(),
            working_directory: None,
            required: false,
            timeout_seconds: 60,
        });
        assert_eq!(cfg.mode(), ValidationMode::Mixed);
    }

    #[test]
    fn test_active_validation_registry_concurrency_and_cancellation() {
        let mut reg = ActiveValidationRegistry::new();
        assert!(!reg.is_running("proj-1"));

        let (cur1, all1) = reg.register("proj-1", "run-1").unwrap();
        assert!(reg.is_running("proj-1"));
        assert_eq!(reg.get_active_run_id("proj-1"), Some("run-1".to_string()));

        // Concurrency violation: cannot register second active run for same project
        let err = reg.register("proj-1", "run-2").unwrap_err();
        assert!(matches!(err, ValidationError::Execution(_)));

        // Test stop_current
        assert!(!cur1.load(Ordering::Relaxed));
        reg.stop_current("proj-1").unwrap();
        assert!(cur1.load(Ordering::Relaxed));
        assert!(!all1.load(Ordering::Relaxed));

        // Test stop_all
        reg.stop_all("run-1").unwrap();
        assert!(all1.load(Ordering::Relaxed));

        // Unregister
        reg.unregister("proj-1", "run-1");
        assert!(!reg.is_running("proj-1"));
    }

    #[test]
    fn test_reconcile_interrupted_runs_on_startup() {
        let mut db = crate::db::DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();

        db.connection()
            .execute(
                "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    "proj-orphaned",
                    "Test Project",
                    "/tmp/test",
                    chrono::Utc::now().to_rfc3339(),
                    chrono::Utc::now().to_rfc3339(),
                    chrono::Utc::now().to_rfc3339()
                ],
            )
            .unwrap();

        let run_rec = ValidationRunRecord {
            run_id: "run-orphaned".to_string(),
            project_id: "proj-orphaned".to_string(),
            architecture_version: "1.0".to_string(),
            epoch_id: Some("epoch-1".to_string()),
            trigger_source: ValidationTriggerSource::Manual,
            status: ValidationRunStatus::Running,
            is_gate_passed: false,
            has_override: false,
            git_head: Some("head-123".to_string()),
            git_dirty_fingerprint: Some("dirty-123".to_string()),
            config_fingerprint: Some("cfg-123".to_string()),
            log_path: None,
            started_at: chrono::Utc::now().to_rfc3339(),
            completed_at: None,
            duration_ms: 0,
            commands: vec![],
        };
        insert_validation_run(db.connection(), &run_rec).unwrap();

        let count = ValidationService::reconcile_interrupted_runs(db.connection_mut()).unwrap();
        assert_eq!(count, 1);

        let reconciled = get_validation_run(db.connection(), "run-orphaned")
            .unwrap()
            .unwrap();
        assert_eq!(reconciled.status, ValidationRunStatus::Interrupted);
        assert!(reconciled.completed_at.is_some());
    }

    #[test]
    fn test_validation_evidence_crash_safe_write_and_rehydration() {
        let dir = tempdir().unwrap();
        let repo_path = dir.path();

        let run_rec = ValidationRunRecord {
            run_id: "run-persist-1".to_string(),
            project_id: "proj-1".to_string(),
            architecture_version: "1.0".to_string(),
            epoch_id: Some("epoch-1".to_string()),
            trigger_source: ValidationTriggerSource::Manual,
            status: ValidationRunStatus::Pass,
            is_gate_passed: true,
            has_override: false,
            git_head: Some("commit-abc".to_string()),
            git_dirty_fingerprint: Some("fingerprint-def".to_string()),
            config_fingerprint: Some("cfg-xyz".to_string()),
            log_path: None,
            started_at: chrono::Utc::now().to_rfc3339(),
            completed_at: Some(chrono::Utc::now().to_rfc3339()),
            duration_ms: 450,
            commands: vec![ValidationCommandExecutionRecord {
                execution_id: "exec-1".to_string(),
                run_id: "run-persist-1".to_string(),
                command_id: "c1".to_string(),
                name: "Build".to_string(),
                command_str: "cargo check".to_string(),
                working_dir: None,
                required: true,
                status: ValidationCommandStatus::Pass,
                exit_code: Some(0),
                duration_ms: 450,
                is_truncated: false,
                log_path: None,
                started_at: Some(chrono::Utc::now().to_rfc3339()),
                completed_at: Some(chrono::Utc::now().to_rfc3339()),
            }],
        };

        let summary_file = ValidationService::write_durable_evidence(repo_path, &run_rec).unwrap();
        assert!(summary_file.exists());

        let content = std::fs::read_to_string(&summary_file).unwrap();
        let parsed: ValidationEvidenceSummary = serde_yaml::from_str(&content).unwrap();
        assert_eq!(parsed.run_id, "run-persist-1");
        assert_eq!(parsed.overall_status, "PASS");
        assert_eq!(parsed.commands.len(), 1);
        assert_eq!(parsed.commands[0].status, "PASS");
    }

    #[test]
    fn test_check_review_gate_and_override_behavior() {
        let dir = tempdir().unwrap();
        let repo_path = dir.path();

        // 1. Setup git repo
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "TestUser"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        let impl_dir = repo_path.join(".coalition").join("implementation");
        std::fs::create_dir_all(&impl_dir).unwrap();

        let val_yaml = r#"
schema_version: 1
enabled: true
policy:
  gate_review_on_required_failure: true
  allow_manual_runs: true
  allow_builder_requested_runs: true
commands:
  - id: t1
    name: Test 1
    command: echo test
    required: true
"#;
        std::fs::write(impl_dir.join("validation.yaml"), val_yaml).unwrap();

        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(repo_path)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        let git = GitAdapter::new().unwrap();
        let head = git.inspect_repo(repo_path).unwrap().head_commit.unwrap();
        let dirty = git.compute_implementation_fingerprint(repo_path).unwrap();
        let cfg = ValidationService::read_validation_config(repo_path).unwrap();
        let cfg_fp = cfg.fingerprint();

        let mut db = crate::db::DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();

        let project_id = "proj-test-gate";
        db.connection()
            .execute(
                "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    project_id,
                    "Test Project",
                    repo_path.to_str().unwrap(),
                    chrono::Utc::now().to_rfc3339(),
                    chrono::Utc::now().to_rfc3339(),
                    chrono::Utc::now().to_rfc3339()
                ],
            )
            .unwrap();

        // 2. check_review_gate fails when no runs exist
        let err_no_runs = ValidationService::check_review_gate(
            db.connection(),
            repo_path,
            project_id,
            "1.0",
            "epoch-1",
        )
        .unwrap_err();
        assert!(matches!(err_no_runs, ValidationError::Execution(_)));

        // 3. Insert failed run
        let run_fail = ValidationRunRecord {
            run_id: "run-failed".to_string(),
            project_id: project_id.to_string(),
            architecture_version: "1.0".to_string(),
            epoch_id: Some("epoch-1".to_string()),
            trigger_source: ValidationTriggerSource::Manual,
            status: ValidationRunStatus::Fail,
            is_gate_passed: false,
            has_override: false,
            git_head: Some(head.clone()),
            git_dirty_fingerprint: Some(dirty.clone()),
            config_fingerprint: Some(cfg_fp),
            log_path: None,
            started_at: chrono::Utc::now().to_rfc3339(),
            completed_at: Some(chrono::Utc::now().to_rfc3339()),
            duration_ms: 100,
            commands: vec![],
        };
        insert_validation_run(db.connection(), &run_fail).unwrap();

        // 4. check_review_gate fails closed on failed run without override
        let err_failed = ValidationService::check_review_gate(
            db.connection(),
            repo_path,
            project_id,
            "1.0",
            "epoch-1",
        )
        .unwrap_err();
        assert!(matches!(err_failed, ValidationError::Execution(_)));

        // 5. Authorize override
        let ovr = ValidationService::override_validation_gate(
            db.connection_mut(),
            repo_path,
            project_id,
            "run-failed",
            "Known defect accepted for review",
            "HUMAN",
        )
        .unwrap();
        assert_eq!(ovr.run_id, "run-failed");

        // 6. check_review_gate passes after override
        let run_after_ovr = ValidationService::check_review_gate(
            db.connection(),
            repo_path,
            project_id,
            "1.0",
            "epoch-1",
        )
        .unwrap()
        .unwrap();
        assert!(run_after_ovr.has_override);

        // 7. Modifying code produces stale evidence and fails closed!
        std::fs::write(repo_path.join("file.txt"), "mutation").unwrap();
        let err_stale = ValidationService::check_review_gate(
            db.connection(),
            repo_path,
            project_id,
            "1.0",
            "epoch-1",
        )
        .unwrap_err();
        assert!(matches!(err_stale, ValidationError::StaleEvidence(_)));
    }
}

use crate::core::process::{ProcessOutputKind, ProcessOutputLine, ProcessRunner};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::mpsc;

#[derive(Error, Debug)]
pub enum BuilderError {
    #[error("Antigravity CLI binary not found: {0}")]
    NotFound(String),
    #[error("CLI execution failed: {0}")]
    ExecutionFailed(String),
    #[error("Parse error: {0}")]
    ParseError(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Session canceled")]
    Canceled,
    #[error("Session timed out")]
    Timeout,
    #[error("Architecture not frozen or contract missing: {0}")]
    NotFrozen(String),
    #[error("Contract drift detected: {0}")]
    DriftDetected(String),
    #[error("Database error: {0}")]
    Database(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
}

fn deserialize_null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + serde::Deserialize<'de>,
{
    let opt = Option::<T>::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

pub const STATUS_RUNNING: &str = "RUNNING";
pub const STATUS_SUCCESS: &str = "SUCCESS";
pub const STATUS_FAILED: &str = "FAILED";
pub const STATUS_CANCELLED: &str = "CANCELLED";
pub const STATUS_TIMEOUT: &str = "TIMEOUT";
pub const STATUS_INTERRUPTED: &str = "INTERRUPTED";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct AgyUsage {
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub input_tokens: u64,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub output_tokens: u64,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub thinking_tokens: u64,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub cache_read_tokens: u64,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgyInitData {
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgyStepUpdateData {
    pub conversation_id: Option<String>,
    pub step_index: Option<i64>,
    pub state: Option<String>,
    pub step_type: Option<String>,
    pub text_delta: Option<String>,
    pub duration_seconds: Option<f64>,
    pub usage: Option<AgyUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgyResultData {
    pub conversation_id: Option<String>,
    pub status: String,
    pub response: Option<String>,
    pub error: Option<String>,
    pub duration_seconds: Option<f64>,
    pub num_turns: Option<i64>,
    pub usage: Option<AgyUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum AgyEvent {
    #[serde(rename = "init")]
    Init {
        conversation_id: Option<String>,
        init: AgyInitData,
    },
    #[serde(rename = "step_update")]
    StepUpdate { step_update: AgyStepUpdateData },
    #[serde(rename = "result")]
    Result { result: AgyResultData },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuilderTurnRequest {
    pub prompt: String,
    pub conversation_id: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub icarus_mode: bool,
    pub working_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuilderTurnResponse {
    pub conversation_id: Option<String>,
    pub status: String,
    pub text_response: String,
    pub cumulative_usage: AgyUsage,
    pub was_canceled: bool,
    pub stderr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuilderSessionRecord {
    pub session_id: String,
    pub project_id: String,
    pub epoch_id: String,
    pub conversation_id: Option<String>,
    pub model: String,
    pub effort: Option<String>,
    pub icarus_mode: bool,
    pub status: String,
    pub prompt: String,
    pub response_text: Option<String>,
    pub error_message: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub duration_ms: u64,
    pub usage: AgyUsage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuilderEventRecord {
    pub id: i64,
    pub session_id: String,
    pub project_id: String,
    pub step_index: Option<i64>,
    pub event_type: String,
    pub state: Option<String>,
    pub content: Option<String>,
    pub details_json: Option<String>,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatGptUsageEstimator {
    pub version: u32,
    pub chars_per_token: f64,
    pub sample_count: u32,
    pub last_calibrated_at: Option<String>,
}

impl Default for ChatGptUsageEstimator {
    fn default() -> Self {
        Self {
            version: 1,
            chars_per_token: 4.0,
            sample_count: 0,
            last_calibrated_at: None,
        }
    }
}

impl ChatGptUsageEstimator {
    pub fn new(
        version: u32,
        chars_per_token: f64,
        sample_count: u32,
        last_calibrated_at: Option<String>,
    ) -> Self {
        Self {
            version: if version > 0 { version } else { 1 },
            chars_per_token: if chars_per_token > 0.0 {
                chars_per_token
            } else {
                4.0
            },
            sample_count,
            last_calibrated_at,
        }
    }

    pub fn estimate_tokens(&self, char_count: usize) -> u64 {
        if self.chars_per_token <= 0.0 {
            return (char_count / 4) as u64;
        }
        (char_count as f64 / self.chars_per_token).round() as u64
    }

    pub fn calibrate(&mut self, sample_tokens: u64, sample_chars: usize) {
        if sample_tokens > 0 && sample_chars > 0 {
            let observed = sample_chars as f64 / sample_tokens as f64;
            let n = self.sample_count as f64;
            self.chars_per_token = (self.chars_per_token * n + observed) / (n + 1.0);
            self.sample_count += 1;
            self.version += 1;
            self.last_calibrated_at = Some(chrono::Utc::now().to_rfc3339());
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatGptUsageSummary {
    pub rolling_5h_tokens: u64,
    pub rolling_7d_tokens: u64,
    pub total_tokens: u64,
    pub total_packets_sent: u64,
    pub total_imports_received: u64,
    pub last_calibrated_at: Option<String>,
    pub disclaimer: String,
    pub estimator_version: u32,
    pub chars_per_token: f64,
    pub sample_count: u32,
    pub estimated_5h_capacity_pct: Option<f64>,
    pub estimated_weekly_capacity_pct: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRecord {
    pub id: i64,
    pub project_id: String,
    pub session_id: Option<String>,
    pub tool_name: String,
    pub target: Option<String>,
    pub risk_level: String,
    pub decision: String,
    pub reason: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRuleRecord {
    pub rule_id: String,
    pub project_id: String,
    pub tool_name: String,
    pub pattern: Option<String>,
    pub decision: String,
    pub created_at: String,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IcarusState {
    pub project_id: String,
    pub enabled: bool,
    pub enabled_at: Option<String>,
    pub enabled_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageTelemetryReport {
    pub provider_antigravity_usage: AgyUsage,
    pub active_model: Option<String>,
    pub active_effort: Option<String>,
    pub chatgpt_estimated_usage: ChatGptUsageSummary,
    pub context_window_note: String,
    pub quota_buckets_note: String,
}

pub fn evaluate_tool_risk(tool_name: &str, content: &str) -> (&'static str, &'static str) {
    match tool_name {
        "run_command" => {
            if content.contains("rm -rf")
                || content.contains("del /f")
                || content.contains("git reset --hard")
            {
                (
                    "CRITICAL",
                    "Potentially destructive shell command execution",
                )
            } else {
                ("HIGH_RISK", "Arbitrary command line process execution")
            }
        }
        "write_to_file" => ("MUTATING", "File modification or creation"),
        "replace_file_content" => ("MUTATING", "In-place file mutation"),
        "view_file" => ("READ_ONLY", "Read-only file inspection"),
        "list_dir" => ("READ_ONLY", "Directory listing"),
        "grep_search" => ("READ_ONLY", "Read-only pattern search"),
        "find_by_name" => ("READ_ONLY", "File search"),
        _ => (
            "HIGH_RISK",
            "External tool invocation with unclassified risk",
        ),
    }
}

/// Redacts sensitive credentials, tokens, and authorization headers from strings before persistent logging.
pub fn sanitize_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for word in text.split_whitespace() {
        if word.starts_with("sk-") && word.len() > 10 {
            out.push_str("[REDACTED_API_KEY]");
        } else if word.starts_with("AIza") && word.len() > 20 {
            out.push_str("[REDACTED_KEY]");
        } else if word.to_lowercase().starts_with("bearer") && word.len() > 6 {
            out.push_str("[REDACTED_TOKEN]");
        } else {
            out.push_str(word);
        }
        out.push(' ');
    }
    if !out.is_empty() {
        out.pop();
    }
    out
}

pub fn sanitize_and_bound_event_record(
    event: &AgyEvent,
    session_id: &str,
    project_id: &str,
) -> BuilderEventRecord {
    use crate::core::process::truncate_utf8_safe;

    let (event_type, step_idx, state_str, content_str, details_json) = match event {
        AgyEvent::Init {
            conversation_id: _,
            init,
        } => {
            let mut sanitized_init = init.clone();
            sanitized_init.cwd = None; // Strip developer machine-local path!
            let details = serde_json::to_string(&sanitized_init)
                .ok()
                .map(|d| truncate_utf8_safe(&d, 32 * 1024).to_string());
            (
                "INIT".to_string(),
                None,
                init.permission_mode.clone(),
                init.model.clone(),
                details,
            )
        }
        AgyEvent::StepUpdate { step_update } => {
            let mut bounded_step = step_update.clone();
            let bounded_delta = bounded_step.text_delta.as_ref().map(|d| {
                let safe = truncate_utf8_safe(d, 16 * 1024);
                sanitize_text(safe)
            });
            bounded_step.text_delta = bounded_delta.clone();

            let details = serde_json::to_string(&bounded_step)
                .ok()
                .map(|d| truncate_utf8_safe(&d, 32 * 1024).to_string());

            (
                step_update
                    .step_type
                    .clone()
                    .unwrap_or_else(|| "STEP_UPDATE".to_string()),
                step_update.step_index,
                step_update.state.clone(),
                bounded_delta,
                details,
            )
        }
        AgyEvent::Result { result } => {
            let mut bounded_result = result.clone();
            bounded_result.response = bounded_result.response.as_ref().map(|r| {
                let safe = truncate_utf8_safe(r, 16 * 1024);
                sanitize_text(safe)
            });
            bounded_result.error = bounded_result.error.as_ref().map(|e| {
                let safe = truncate_utf8_safe(e, 16 * 1024);
                sanitize_text(safe)
            });

            let details = serde_json::to_string(&bounded_result)
                .ok()
                .map(|d| truncate_utf8_safe(&d, 32 * 1024).to_string());

            (
                "RESULT".to_string(),
                None,
                Some(result.status.clone()),
                bounded_result.response,
                details,
            )
        }
        AgyEvent::Unknown => ("UNKNOWN".to_string(), None, None, None, None),
    };

    BuilderEventRecord {
        id: 0,
        session_id: session_id.to_string(),
        project_id: project_id.to_string(),
        step_index: step_idx,
        event_type,
        state: state_str,
        content: content_str,
        details_json,
        timestamp: chrono::Utc::now().to_rfc3339(),
    }
}

#[derive(Debug, Clone)]
pub struct AntigravityCliAdapter {
    bin_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ActiveBuilderExecution {
    pub project_id: String,
    pub session_id: String,
    pub epoch_id: String,
    pub conversation_id: Option<String>,
    pub model: String,
    pub effort: Option<String>,
    pub icarus_mode: bool,
    pub started_at: String,
    pub cancel_flag: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct ActiveBuilderRegistry {
    executions: std::collections::HashMap<String, ActiveBuilderExecution>,
}

impl ActiveBuilderRegistry {
    pub fn new() -> Self {
        Self {
            executions: std::collections::HashMap::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn register(
        &mut self,
        project_id: &str,
        session_id: &str,
        epoch_id: &str,
        conversation_id: Option<String>,
        model: &str,
        effort: Option<String>,
        icarus_mode: bool,
    ) -> Result<Arc<AtomicBool>, BuilderError> {
        if let Some(existing) = self.executions.get(project_id) {
            return Err(BuilderError::ExecutionFailed(format!(
                "Concurrent build forbidden: Builder session '{}' is already actively running for project '{}'.",
                existing.session_id, project_id
            )));
        }

        let cancel_flag = Arc::new(AtomicBool::new(false));
        self.executions.insert(
            project_id.to_string(),
            ActiveBuilderExecution {
                project_id: project_id.to_string(),
                session_id: session_id.to_string(),
                epoch_id: epoch_id.to_string(),
                conversation_id,
                model: model.to_string(),
                effort,
                icarus_mode,
                started_at: chrono::Utc::now().to_rfc3339(),
                cancel_flag: cancel_flag.clone(),
            },
        );
        Ok(cancel_flag)
    }

    pub fn unregister(&mut self, project_id: &str, session_id: &str) {
        if let Some(existing) = self.executions.get(project_id) {
            if existing.session_id == session_id {
                self.executions.remove(project_id);
            }
        }
    }

    pub fn cancel_project(&self, project_id: &str) -> Result<String, BuilderError> {
        if let Some(execution) = self.executions.get(project_id) {
            execution
                .cancel_flag
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(execution.session_id.clone())
        } else {
            Err(BuilderError::ExecutionFailed(format!(
                "No active Builder session found for project '{}'",
                project_id
            )))
        }
    }

    pub fn cancel_session(&self, session_id: &str) -> Result<String, BuilderError> {
        for (proj_id, execution) in &self.executions {
            if execution.session_id == session_id {
                execution
                    .cancel_flag
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                return Ok(proj_id.clone());
            }
        }
        Err(BuilderError::ExecutionFailed(format!(
            "No active Builder session found with session_id '{}'",
            session_id
        )))
    }

    pub fn get_active_execution(&self, project_id: &str) -> Option<ActiveBuilderExecution> {
        self.executions.get(project_id).cloned()
    }

    pub fn is_active(&self, project_id: &str) -> bool {
        self.executions.contains_key(project_id)
    }
}

impl AntigravityCliAdapter {
    pub fn discover() -> Result<Self, BuilderError> {
        // 1. PATH lookup for 'agy'
        if let Ok(path) = which::which("agy") {
            return Ok(Self { bin_path: path });
        }

        // 2. Windows fallback: check %LOCALAPPDATA%\agy\bin\agy.exe dynamically
        #[cfg(target_os = "windows")]
        if let Some(local_appdata) = std::env::var_os("LOCALAPPDATA") {
            let candidate = PathBuf::from(local_appdata)
                .join("agy")
                .join("bin")
                .join("agy.exe");
            if candidate.exists() {
                return Ok(Self {
                    bin_path: candidate,
                });
            }
        }

        Err(BuilderError::NotFound(
            "Could not locate official 'agy' CLI on PATH or standard local application directories"
                .to_string(),
        ))
    }

    pub fn with_path<P: AsRef<Path>>(path: P) -> Self {
        Self {
            bin_path: path.as_ref().to_path_buf(),
        }
    }

    pub fn binary_path(&self) -> &Path {
        &self.bin_path
    }

    pub fn get_version(&self) -> Result<String, BuilderError> {
        let output = ProcessRunner::run_sync_bounded(
            &self.bin_path,
            &["--version"],
            None,
            Duration::from_secs(10),
        )
        .map_err(|e| BuilderError::ExecutionFailed(e.to_string()))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            Err(BuilderError::ExecutionFailed(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ))
        }
    }

    pub fn list_models(&self) -> Result<Vec<ModelInfo>, BuilderError> {
        let output = ProcessRunner::run_sync_bounded(
            &self.bin_path,
            &["models"],
            None,
            Duration::from_secs(15),
        )
        .map_err(|e| BuilderError::ExecutionFailed(e.to_string()))?;

        if !output.status.success() {
            return Err(BuilderError::ExecutionFailed(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ));
        }

        let text = String::from_utf8_lossy(&output.stdout);
        let mut models = Vec::new();

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("Fetching") {
                continue;
            }

            if let Some((id, name)) = line.split_once('\t') {
                models.push(ModelInfo {
                    id: id.trim().to_string(),
                    name: name.trim().to_string(),
                });
            } else if let Some((id, name)) = line.split_once("  ") {
                models.push(ModelInfo {
                    id: id.trim().to_string(),
                    name: name.trim().to_string(),
                });
            } else {
                models.push(ModelInfo {
                    id: line.to_string(),
                    name: line.to_string(),
                });
            }
        }

        Ok(models)
    }

    pub async fn run_turn(
        &self,
        request: BuilderTurnRequest,
        cancel_flag: Arc<AtomicBool>,
        event_sender: Option<mpsc::Sender<AgyEvent>>,
    ) -> Result<BuilderTurnResponse, BuilderError> {
        let mut args = vec![
            "--input-format".to_string(),
            "stream-json".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
        ];

        if let Some(ref conv_id) = request.conversation_id {
            args.push("--conversation".to_string());
            args.push(conv_id.clone());
        }

        if let Some(ref model) = request.model {
            args.push("--model".to_string());
            args.push(model.clone());
        }

        if let Some(ref effort) = request.effort {
            args.push("--effort".to_string());
            args.push(effort.clone());
        }

        if request.icarus_mode {
            args.push("--dangerously-skip-permissions".to_string());
        }

        let input_msg = serde_json::json!({
            "event": "user",
            "message": {
                "content": request.prompt
            }
        });
        let mut input_line = serde_json::to_string(&input_msg)
            .map_err(|e| BuilderError::ParseError(e.to_string()))?;
        input_line.push('\n');

        let bin_path = self.bin_path.clone();
        let working_path = request.working_dir.map(PathBuf::from);

        let (line_tx, mut line_rx) = mpsc::channel::<ProcessOutputLine>(500);

        let runner_cancel = cancel_flag.clone();
        let runner_task = tokio::spawn(async move {
            ProcessRunner::run_turn_stream(
                &bin_path,
                &args,
                working_path.as_deref(),
                Some(&input_line),
                Duration::from_secs(600), // bounded turn limit
                runner_cancel,
                Some(line_tx),
            )
            .await
        });

        let mut active_conversation_id = request.conversation_id.clone();
        let mut final_text = String::new();
        let mut final_status = "UNKNOWN".to_string();
        let mut cumulative_usage = AgyUsage::default();
        let mut stderr_buffer = String::new();

        while let Some(out_line) = line_rx.recv().await {
            match out_line.kind {
                ProcessOutputKind::Stdout => {
                    if let Ok(event) = serde_json::from_str::<AgyEvent>(&out_line.line) {
                        match &event {
                            AgyEvent::Init {
                                conversation_id, ..
                            } => {
                                if let Some(cid) = conversation_id {
                                    active_conversation_id = Some(cid.clone());
                                }
                            }
                            AgyEvent::StepUpdate { step_update } => {
                                if let Some(ref delta) = step_update.text_delta {
                                    if final_text.len() < 5 * 1024 * 1024 {
                                        let remaining = 5 * 1024 * 1024 - final_text.len();
                                        let safe_slice = crate::core::process::truncate_utf8_safe(
                                            delta, remaining,
                                        );
                                        final_text.push_str(safe_slice);
                                    }
                                }
                                if let Some(ref u) = step_update.usage {
                                    cumulative_usage = u.clone();
                                }
                            }
                            AgyEvent::Result { result } => {
                                final_status = result.status.clone();
                                if let Some(ref u) = result.usage {
                                    cumulative_usage = u.clone();
                                }
                                if let Some(ref resp) = result.response {
                                    if final_text.is_empty() {
                                        final_text = crate::core::process::truncate_utf8_safe(
                                            resp,
                                            5 * 1024 * 1024,
                                        )
                                        .to_string();
                                    }
                                }
                                if let Some(ref err) = result.error {
                                    if !err.is_empty() && stderr_buffer.len() < 5 * 1024 * 1024 {
                                        let remaining = 5 * 1024 * 1024 - stderr_buffer.len();
                                        let safe_slice = crate::core::process::truncate_utf8_safe(
                                            err, remaining,
                                        );
                                        stderr_buffer.push_str(safe_slice);
                                        if stderr_buffer.len() < 5 * 1024 * 1024 {
                                            stderr_buffer.push('\n');
                                        }
                                    }
                                }
                            }
                            AgyEvent::Unknown => {}
                        }

                        if let Some(tx) = &event_sender {
                            let _ = tx.send(event).await;
                        }
                    }
                }
                ProcessOutputKind::Stderr => {
                    if stderr_buffer.len() < 5 * 1024 * 1024 {
                        let remaining = 5 * 1024 * 1024 - stderr_buffer.len();
                        let safe_slice =
                            crate::core::process::truncate_utf8_safe(&out_line.line, remaining);
                        stderr_buffer.push_str(safe_slice);
                        if stderr_buffer.len() < 5 * 1024 * 1024 {
                            stderr_buffer.push('\n');
                        }
                    }
                }
            }
        }

        let proc_res = match runner_task.await {
            Ok(Ok(res)) => res,
            Ok(Err(e)) => return Err(BuilderError::ExecutionFailed(e.to_string())),
            Err(e) => {
                return Err(BuilderError::ExecutionFailed(format!(
                    "Task join error: {}",
                    e
                )))
            }
        };

        if proc_res.canceled {
            return Ok(BuilderTurnResponse {
                conversation_id: active_conversation_id,
                status: STATUS_CANCELLED.to_string(),
                text_response: final_text,
                cumulative_usage,
                was_canceled: true,
                stderr: stderr_buffer,
            });
        }

        if proc_res.timed_out {
            return Ok(BuilderTurnResponse {
                conversation_id: active_conversation_id,
                status: STATUS_TIMEOUT.to_string(),
                text_response: final_text,
                cumulative_usage,
                was_canceled: false,
                stderr: stderr_buffer,
            });
        }

        if let Some(code) = proc_res.exit_code {
            if code != 0 {
                if stderr_buffer.contains("unavailable or not found") {
                    return Err(BuilderError::ExecutionFailed(format!(
                        "Model unavailable: {}",
                        stderr_buffer.trim()
                    )));
                }
                if final_status == "UNKNOWN" || final_status == STATUS_SUCCESS {
                    final_status = STATUS_FAILED.to_string();
                }
            }
        }

        if final_status == "SUCCESS" {
            final_status = STATUS_SUCCESS.to_string();
        } else if final_status == "ERROR" {
            final_status = STATUS_FAILED.to_string();
        } else if final_status == "CANCELED" {
            final_status = STATUS_CANCELLED.to_string();
        }

        Ok(BuilderTurnResponse {
            conversation_id: active_conversation_id,
            status: final_status,
            text_response: final_text,
            cumulative_usage,
            was_canceled: false,
            stderr: stderr_buffer,
        })
    }
}

pub fn validate_builder_preflight(
    db: &mut crate::db::DbManager,
    project_id: &str,
) -> Result<(PathBuf, String, crate::core::freeze::BuilderPacket), BuilderError> {
    let repo_path_str: String = db
        .connection()
        .query_row(
            "SELECT repository_path FROM projects WHERE project_id = ?1",
            rusqlite::params![project_id],
            |r| r.get(0),
        )
        .map_err(|e| BuilderError::ExecutionFailed(format!("Project repository error: {}", e)))?;
    let repo_path = PathBuf::from(repo_path_str);

    if !repo_path.exists() || !repo_path.is_dir() {
        return Err(BuilderError::ExecutionFailed(format!(
            "Repository directory does not exist or is unavailable: {:?}",
            repo_path
        )));
    }

    crate::core::freeze::FreezeService::reconcile_drift_restoration(
        &repo_path,
        project_id,
        db.connection_mut(),
    )
    .map_err(|e| {
        BuilderError::ExecutionFailed(format!("Drift restoration recovery required: {}", e))
    })?;

    let coalition_dir = crate::core::artifacts::ArtifactManager::resolve_coalition_dir(&repo_path)
        .map_err(|e| BuilderError::ExecutionFailed(format!("Artifact resolution error: {}", e)))?;

    let project_yaml = crate::core::artifacts::ArtifactManager::read_project_yaml(
        coalition_dir.join("project.yaml"),
    )
    .map_err(|e| BuilderError::ExecutionFailed(format!("Cannot read project.yaml: {}", e)))?;

    if project_yaml.architecture_state != crate::core::artifacts::ArchitectureState::Frozen {
        return Err(BuilderError::NotFrozen(format!(
            "Project architecture state is '{}', but must be 'frozen' to execute Builder",
            project_yaml.architecture_state
        )));
    }

    let version = project_yaml
        .current_architecture_version
        .as_deref()
        .unwrap_or("1.0")
        .to_string();

    crate::core::freeze::FreezeService::verify_snapshot_integrity(
        &repo_path,
        &version,
        project_yaml.active_manifest_fingerprint.as_deref(),
    )
    .map_err(|e| BuilderError::ExecutionFailed(format!("Frozen snapshot corrupt: {}", e)))?;

    let drift = crate::core::freeze::FreezeService::check_contract_drift(&repo_path, project_id)
        .map_err(|e| BuilderError::ExecutionFailed(format!("Contract drift check error: {}", e)))?;

    if drift.has_drift {
        return Err(BuilderError::DriftDetected(
            "Cannot start build: architecture contract has drifted from frozen snapshot. Restore artifacts or complete governed architecture change first.".to_string(),
        ));
    }

    let builder_packet = crate::core::freeze::FreezeService::get_builder_packet(&repo_path, None)
        .map_err(|e| {
        BuilderError::ExecutionFailed(format!("Builder packet read error: {}", e))
    })?;

    Ok((repo_path, version, builder_packet))
}

pub type BuilderEventSink = Arc<dyn Fn(&str, &serde_json::Value) + Send + Sync>;

pub struct BuilderService;

impl BuilderService {
    pub async fn start_governed_turn(
        db_arc: Arc<tokio::sync::Mutex<crate::db::DbManager>>,
        registry_arc: Arc<tokio::sync::Mutex<ActiveBuilderRegistry>>,
        event_sink: Option<BuilderEventSink>,
        project_id: &str,
        model: Option<String>,
        effort: Option<String>,
        custom_adapter: Option<AntigravityCliAdapter>,
    ) -> Result<BuilderTurnResponse, BuilderError> {
        // 1. Check current workflow state: must be FROZEN, BUILDING, or CORRECTIONS_REQUIRED
        let current_wf_state = {
            let db = db_arc.lock().await;
            db.connection()
                .query_row(
                    "SELECT state FROM workflow_state WHERE project_id = ?1",
                    rusqlite::params![project_id],
                    |r| r.get::<_, String>(0),
                )
                .map_err(|e| {
                    BuilderError::ExecutionFailed(format!("Workflow state not found: {}", e))
                })?
        };

        if current_wf_state != "FROZEN"
            && current_wf_state != "BUILDING"
            && current_wf_state != "CORRECTIONS_REQUIRED"
        {
            return Err(BuilderError::ExecutionFailed(format!(
                "Cannot start Builder turn while project is in {} state. Architecture must be FROZEN first.",
                current_wf_state
            )));
        }

        // 2. Perform authoritative preflights BEFORE mutating workflow state.
        // If preflight fails (drift, snapshot corrupt, missing repo), workflow state remains untouched!
        let (repo_path, _arch_version, builder_packet) = {
            let mut db = db_arc.lock().await;
            validate_builder_preflight(&mut db, project_id)?
        };

        let epoch_id = builder_packet.metadata.builder_epoch_id.clone();

        // 3. Concurrency check via ActiveBuilderRegistry: strictly 1 active execution per project
        {
            let registry = registry_arc.lock().await;
            if let Some(existing) = registry.get_active_execution(project_id) {
                return Err(BuilderError::ExecutionFailed(format!(
                    "Concurrent build forbidden: session '{}' is already actively running for project '{}'.",
                    existing.session_id, project_id
                )));
            }
        }

        // 4. Preflights have succeeded. If workflow state was FROZEN or CORRECTIONS_REQUIRED, transition to BUILDING.
        if current_wf_state == "FROZEN" || current_wf_state == "CORRECTIONS_REQUIRED" {
            let mut db = db_arc.lock().await;
            crate::core::workflow::apply_workflow_action(
                db.connection_mut(),
                project_id,
                crate::core::workflow::WorkflowAction::StartBuild,
                "HUMAN",
            )
            .map_err(|e| BuilderError::ExecutionFailed(e.to_string()))?;
        }

        // 5. Invariant: Stage 3 Builder input comes 100% from the frozen Builder Packet.
        let turn_prompt = builder_packet.prompt.clone();

        // 6. Check for existing conversation in this epoch only and Icarus state
        let (existing_conv_id, icarus_mode) = {
            let db = db_arc.lock().await;
            let latest_session = db
                .get_latest_builder_session(project_id)
                .map_err(|e| BuilderError::ExecutionFailed(e.to_string()))?;
            let cid = latest_session.and_then(|s| {
                if s.epoch_id == epoch_id {
                    s.conversation_id
                } else {
                    None
                }
            });
            let icarus = db
                .get_icarus_state(project_id)
                .map_err(|e| BuilderError::ExecutionFailed(e.to_string()))?
                .enabled;
            (cid, icarus)
        };

        // 7. Resolve adapter
        let adapter = if let Some(a) = custom_adapter {
            a
        } else {
            AntigravityCliAdapter::discover()?
        };

        let model_name = match model {
            Some(ref m) if !m.trim().is_empty() => m.clone(),
            _ => {
                let models = adapter.list_models()?;
                match models.first() {
                    Some(m) => m.id.clone(),
                    None => {
                        return Err(BuilderError::ExecutionFailed(
                            "No models are reported available by Antigravity CLI. Please check `agy models`."
                                .to_string(),
                        ));
                    }
                }
            }
        };
        let effort_level = effort.clone();

        let session_id = uuid::Uuid::new_v4().to_string();
        let started_at = chrono::Utc::now().to_rfc3339();

        // 8. Register active execution in registry (returns per-session cancel flag)
        let session_cancel_flag = {
            let mut registry = registry_arc.lock().await;
            registry.register(
                project_id,
                &session_id,
                &epoch_id,
                existing_conv_id.clone(),
                &model_name,
                effort_level.clone(),
                icarus_mode,
            )?
        };

        // 9. Insert initial session record
        {
            let db = db_arc.lock().await;
            let session_rec = BuilderSessionRecord {
                session_id: session_id.clone(),
                project_id: project_id.to_string(),
                epoch_id: epoch_id.clone(),
                conversation_id: existing_conv_id.clone(),
                model: model_name.clone(),
                effort: effort_level.clone(),
                icarus_mode,
                status: STATUS_RUNNING.to_string(),
                prompt: turn_prompt.clone(),
                response_text: None,
                error_message: None,
                started_at: started_at.clone(),
                completed_at: None,
                duration_ms: 0,
                usage: AgyUsage::default(),
            };
            db.insert_builder_session(&session_rec)
                .map_err(|e| BuilderError::ExecutionFailed(e.to_string()))?;

            let meta = serde_json::json!({
                "session_id": session_id,
                "epoch_id": epoch_id,
                "model": model_name,
                "effort": effort_level,
                "icarus_mode": icarus_mode,
                "conversation_id": existing_conv_id,
            });
            let _ = crate::core::activity::ActivityManager::record_event(
                db.connection(),
                project_id,
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

        // 10. Channel and event streaming: emit to frontend and persist to builder_events table
        let (tx, mut rx) = mpsc::channel::<AgyEvent>(200);

        let sink_clone = event_sink.clone();
        let session_id_clone = session_id.clone();
        let project_id_clone = project_id.to_string();
        let db_clone = db_arc.clone();

        let persist_handle = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let Some(ref sink) = sink_clone {
                    sink(
                        "coalition:builder-event",
                        &serde_json::json!({
                            "session_id": session_id_clone,
                            "project_id": project_id_clone,
                            "event": event,
                        }),
                    );
                }

                let rec =
                    sanitize_and_bound_event_record(&event, &session_id_clone, &project_id_clone);
                let db = db_clone.lock().await;
                let _ = db.insert_builder_event(&rec);
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
            .run_turn(request, session_cancel_flag, Some(tx))
            .await;
        let duration_ms = turn_start.elapsed().as_millis() as u64;
        let completed_at = chrono::Utc::now().to_rfc3339();

        // Await persistence drain before proceeding to ensure no trailing events are lost
        let _ = persist_handle.await;

        // 11. Unregister active execution from registry
        {
            let mut registry = registry_arc.lock().await;
            registry.unregister(project_id, &session_id);
        }

        match execution_result {
            Ok(resp) => {
                let db = db_arc.lock().await;
                let final_status = if resp.was_canceled {
                    STATUS_CANCELLED
                } else if resp.status == "SUCCESS" || resp.status == "COMPLETED" {
                    STATUS_SUCCESS
                } else if resp.status == "TIMEOUT" {
                    STATUS_TIMEOUT
                } else {
                    &resp.status
                };

                let _ = db.update_builder_session_status(
                    &session_id,
                    final_status,
                    Some(&resp.text_response),
                    None,
                    Some(&completed_at),
                    duration_ms,
                    &resp.cumulative_usage,
                    resp.conversation_id.as_deref(),
                );

                let meta = serde_json::json!({
                    "session_id": session_id,
                    "status": final_status,
                    "duration_ms": duration_ms,
                    "tokens": resp.cumulative_usage.total_tokens,
                    "conversation_id": resp.conversation_id,
                });
                let _ = crate::core::activity::ActivityManager::record_event(
                    db.connection(),
                    project_id,
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
                        final_status
                    ),
                    Some(&meta),
                );

                // Honest permission inspection:
                let lower_stderr = resp.stderr.to_lowercase();
                if lower_stderr.contains("permission denied")
                    || lower_stderr.contains("tool execution denied")
                    || lower_stderr.contains("confirmation rejected")
                {
                    let perm_rec = PermissionRecord {
                        id: 0,
                        project_id: project_id.to_string(),
                        session_id: Some(session_id.clone()),
                        tool_name: "UNCLASSIFIED_EXTERNAL_ACTION".to_string(),
                        target: None,
                        risk_level: "HIGH_RISK".to_string(),
                        decision: "BLOCKED".to_string(),
                        reason: Some(
                            "Antigravity CLI reported a permission denial for an unclassified external action in headless mode. Review security requirements or authorize Icarus mode to grant autonomous tool execution.".to_string(),
                        ),
                        created_at: completed_at.clone(),
                    };
                    let _ = db.record_permission_history(&perm_rec);
                }

                Ok(resp)
            }
            Err(e) => {
                let err_str = e.to_string();
                let db = db_arc.lock().await;
                let _ = db.update_builder_session_status(
                    &session_id,
                    STATUS_FAILED,
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
                let _ = crate::core::activity::ActivityManager::record_event(
                    db.connection(),
                    project_id,
                    "BUILDER_TURN_FAILED",
                    "BUILDER",
                    &format!("Builder turn failed: {}", err_str),
                    Some(&meta),
                );

                Err(e)
            }
        }
    }

    pub async fn cancel_turn(
        registry_arc: Arc<tokio::sync::Mutex<ActiveBuilderRegistry>>,
        db_arc: Arc<tokio::sync::Mutex<crate::db::DbManager>>,
        project_id: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<String, BuilderError> {
        let registry = registry_arc.lock().await;
        let (_canceled_project_id, target_session_id) = if let Some(sid) = session_id {
            let pid = registry.cancel_session(sid)?;
            (pid, sid.to_string())
        } else if let Some(pid) = project_id {
            let sid = registry.cancel_project(pid)?;
            (pid.to_string(), sid)
        } else {
            return Err(BuilderError::ExecutionFailed(
                "Either project_id or session_id must be provided to cancel execution".to_string(),
            ));
        };
        drop(registry);

        let db = db_arc.lock().await;
        let now = chrono::Utc::now().to_rfc3339();
        if let Ok(Some(session)) = db.get_builder_session(&target_session_id) {
            if session.status == STATUS_RUNNING {
                let _ = db.update_builder_session_status(
                    &session.session_id,
                    STATUS_CANCELLED,
                    None,
                    None,
                    Some(&now),
                    session.duration_ms,
                    &session.usage,
                    session.conversation_id.as_deref(),
                );
            }
        }

        Ok(target_session_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ndjson_event_deserialization() {
        let init_json = r#"{"event":"init","conversation_id":"c-123","init":{"model":"gemini-3.8-flash-high","cwd":"/path","tools":["view_file"],"permission_mode":"request-review"}}"#;
        let event: AgyEvent = serde_json::from_str(init_json).expect("deserialize init");
        match event {
            AgyEvent::Init {
                conversation_id,
                init,
            } => {
                assert_eq!(conversation_id, Some("c-123".to_string()));
                assert_eq!(init.model, Some("gemini-3.8-flash-high".to_string()));
                assert_eq!(init.permission_mode, Some("request-review".to_string()));
            }
            _ => panic!("Expected Init event"),
        }

        let step_json = r#"{"event":"step_update","step_update":{"conversation_id":"c-123","step_index":1,"state":"DONE","step_type":"agent_response","text_delta":"hello world","duration_seconds":0.5,"usage":{"input_tokens":100,"output_tokens":10,"thinking_tokens":5,"cache_read_tokens":50,"total_tokens":110}}}"#;
        let step_event: AgyEvent = serde_json::from_str(step_json).expect("deserialize step");
        match step_event {
            AgyEvent::StepUpdate { step_update } => {
                assert_eq!(step_update.text_delta, Some("hello world".to_string()));
                let usage = step_update.usage.unwrap();
                assert_eq!(usage.total_tokens, 110);
            }
            _ => panic!("Expected StepUpdate event"),
        }

        let result_json = r#"{"event":"result","result":{"conversation_id":"c-123","status":"SUCCESS","response":"complete","duration_seconds":1.2,"num_turns":1,"usage":{"input_tokens":100,"output_tokens":10,"thinking_tokens":5,"cache_read_tokens":50,"total_tokens":110}}}"#;
        let result_event: AgyEvent = serde_json::from_str(result_json).expect("deserialize result");
        match result_event {
            AgyEvent::Result { result } => {
                assert_eq!(result.status, "SUCCESS");
                assert_eq!(result.response, Some("complete".to_string()));
            }
            _ => panic!("Expected Result event"),
        }
    }

    #[tokio::test]
    async fn test_fake_agy_execution() {
        let fake_agy_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("tests")
            .join("fake-commands")
            .join(if cfg!(windows) {
                "fake-agy.cmd"
            } else {
                "fake-agy"
            });

        if !fake_agy_path.exists() {
            eprintln!("Skipping fake agy test: path not found {:?}", fake_agy_path);
            return;
        }

        let adapter = AntigravityCliAdapter::with_path(fake_agy_path);
        let version = adapter.get_version().expect("get fake-agy version");
        assert!(version.contains("fake"));

        let models = adapter.list_models().expect("list fake-agy models");
        assert!(!models.is_empty());
        assert_eq!(models[0].id, "gemini-3.8-flash-high");

        let cancel = Arc::new(AtomicBool::new(false));
        let response = adapter
            .run_turn(
                BuilderTurnRequest {
                    prompt: "test prompt".to_string(),
                    conversation_id: None,
                    model: Some("gemini-3.8-flash-high".to_string()),
                    effort: None,
                    icarus_mode: false,
                    working_dir: None,
                },
                cancel,
                None,
            )
            .await
            .expect("run turn on fake-agy");

        assert_eq!(response.status, "SUCCESS");
        assert!(response.text_response.contains("test prompt"));
        assert!(response.cumulative_usage.total_tokens > 0);
        assert!(!response.was_canceled);
    }

    #[tokio::test]
    async fn test_fake_agy_event_stream_count_and_accuracy() {
        let fake_agy_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("tests")
            .join("fake-commands")
            .join(if cfg!(windows) {
                "fake-agy.cmd"
            } else {
                "fake-agy"
            });

        if !fake_agy_path.exists() {
            eprintln!("Skipping fake agy test: path not found {:?}", fake_agy_path);
            return;
        }

        let adapter = AntigravityCliAdapter::with_path(fake_agy_path);
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, mut rx) = mpsc::channel::<AgyEvent>(100);

        let response = adapter
            .run_turn(
                BuilderTurnRequest {
                    prompt: "verify-non-duplicate".to_string(),
                    conversation_id: None,
                    model: Some("gemini-3.8-flash-high".to_string()),
                    effort: None,
                    icarus_mode: false,
                    working_dir: None,
                },
                cancel,
                Some(tx),
            )
            .await
            .expect("run turn on fake-agy");

        assert_eq!(
            response.conversation_id,
            Some("fake-conv-uuid-12345".to_string())
        );
        assert_eq!(response.status, "SUCCESS");
        assert_eq!(response.cumulative_usage.total_tokens, 550);
        assert!(!response.was_canceled);

        let mut received_events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            received_events.push(event);
        }

        assert_eq!(
            received_events.len(),
            4,
            "Expected exactly 4 stream events, received {}",
            received_events.len()
        );
    }

    #[tokio::test]
    async fn test_fake_agy_unavailable_model_fails_visibly() {
        let fake_agy_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("tests")
            .join("fake-commands")
            .join(if cfg!(windows) {
                "fake-agy.cmd"
            } else {
                "fake-agy"
            });

        if !fake_agy_path.exists() {
            return;
        }

        let adapter = AntigravityCliAdapter::with_path(fake_agy_path);
        let cancel = Arc::new(AtomicBool::new(false));

        let res = adapter
            .run_turn(
                BuilderTurnRequest {
                    prompt: "test".to_string(),
                    conversation_id: None,
                    model: Some("unavailable-pinned-model".to_string()),
                    effort: None,
                    icarus_mode: false,
                    working_dir: None,
                },
                cancel,
                None,
            )
            .await;

        assert!(
            res.is_err(),
            "Unavailable model must fail visibly and return an error"
        );
        let err_str = res.unwrap_err().to_string();
        assert!(
            err_str.contains("Model unavailable") || err_str.contains("unavailable"),
            "Error must indicate model is unavailable: {}",
            err_str
        );
    }

    #[tokio::test]
    async fn test_fake_agy_permission_denied_detection() {
        let fake_agy_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("tests")
            .join("fake-commands")
            .join(if cfg!(windows) {
                "fake-agy.cmd"
            } else {
                "fake-agy"
            });

        if !fake_agy_path.exists() {
            return;
        }

        let adapter = AntigravityCliAdapter::with_path(fake_agy_path);
        let cancel = Arc::new(AtomicBool::new(false));

        let res = adapter
            .run_turn(
                BuilderTurnRequest {
                    prompt: "trigger_permission_denial".to_string(),
                    conversation_id: None,
                    model: Some("gemini-3.8-flash-high".to_string()),
                    effort: None,
                    icarus_mode: false,
                    working_dir: None,
                },
                cancel,
                None,
            )
            .await
            .expect("should return turn response with status");

        assert!(res.status == STATUS_FAILED || res.status == "ERROR");
        assert!(res.stderr.contains("permission denied"));
    }
}

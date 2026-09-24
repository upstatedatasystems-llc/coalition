use crate::core::process::{ProcessOutputKind, ProcessOutputLine, ProcessRunner};
use rusqlite::OptionalExtension;
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
    #[error("Session not found: {0}")]
    SessionNotFound(String),
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
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, serde_json::Value>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum BuilderInstructionSource {
    #[serde(rename = "INITIAL_FROZEN", alias = "InitialFrozen")]
    InitialFrozen,
    #[serde(rename = "REVIEW_CORRECTION", alias = "ReviewCorrection")]
    ReviewCorrection { review_cycle_id: String },
    #[serde(rename = "VALIDATION_DIAGNOSTIC", alias = "ValidationDiagnostic")]
    ValidationDiagnostic { validation_run_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuilderTurnResponse {
    pub conversation_id: Option<String>,
    pub status: String,
    #[serde(default)]
    pub provider_status: Option<String>,
    pub text_response: String,
    pub cumulative_usage: AgyUsage,
    pub was_canceled: bool,
    pub stderr: String,
    #[serde(default)]
    pub has_blocked_actions: bool,
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

/// Defensively wraps sanitize_text so an unexpected panic cannot crash operational tasks.
pub fn safe_sanitize_text(text: &str) -> String {
    std::panic::catch_unwind(|| sanitize_text(text))
        .unwrap_or_else(|_| "[REDACTION_FALLBACK: sanitized due to internal error]".to_string())
}

/// Redacts sensitive credentials, tokens, and authorization headers from strings before persistent logging.
/// Supported patterns include:
/// - `Authorization: Bearer <token>` and `Bearer <token>`
/// - OpenAI-style `sk-[A-Za-z0-9_-]{15,}`
/// - Google-style `AIza[A-Za-z0-9_-]{20,}`
/// - JSON/quoted sensitive keys (`"api_key"`, `"token"`, `"authorization"`, etc.)
/// - Key-value assignments (`api_key=...`, `token=...`, `password=...`, etc.)
pub fn sanitize_text(text: &str) -> String {
    let step1 = redact_bearer_headers(text);
    let step2 = redact_quoted_key_values(&step1);
    let step3 = redact_assignments(&step2);
    redact_standalone_tokens(&step3)
}

fn redact_bearer_headers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let len = text.len();
    let mut prev_char: Option<char> = None;

    while i < len {
        let rem_bytes = &text.as_bytes()[i..];
        if rem_bytes.len() >= 7 && rem_bytes[..7].eq_ignore_ascii_case(b"bearer ") {
            let is_boundary = prev_char
                .map(|c| !c.is_alphanumeric() && c != '_')
                .unwrap_or(true);

            if is_boundary {
                out.push_str(&text[i..i + 7]);
                i += 7;
                while i < len && text.as_bytes()[i] == b' ' {
                    out.push(' ');
                    i += 1;
                }
                let token_start = i;
                let mut token_len = 0;
                for (rel_idx, ch) in text[token_start..].char_indices() {
                    if ch.is_ascii_whitespace()
                        || ch == '"'
                        || ch == '\''
                        || ch == ';'
                        || ch == ','
                        || ch == '}'
                        || ch == ']'
                    {
                        break;
                    }
                    token_len = rel_idx + ch.len_utf8();
                }
                let token_end = token_start + token_len;
                if token_len >= 8 {
                    out.push_str("[REDACTED]");
                } else {
                    out.push_str(&text[token_start..token_end]);
                }
                prev_char = text[token_start..token_end].chars().last();
                i = token_end;
                continue;
            }
        }
        let ch = text[i..].chars().next().unwrap();
        out.push(ch);
        prev_char = Some(ch);
        i += ch.len_utf8();
    }
    out
}

const SENSITIVE_KEYS: &[&str] = &[
    "api_key",
    "apikey",
    "access_token",
    "auth_token",
    "authorization",
    "secret",
    "password",
    "private_key",
    "token",
];

fn redact_quoted_key_values(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let len = text.len();

    while i < len {
        let ch = text[i..].chars().next().unwrap();
        if ch == '"' || ch == '\'' {
            let quote = ch;
            let key_start = i + 1;
            if let Some(rel_quote) = text[key_start..].find(quote) {
                let key_end = key_start + rel_quote;
                let potential_key = &text[key_start..key_end];
                let is_sensitive = SENSITIVE_KEYS
                    .iter()
                    .any(|k| potential_key.eq_ignore_ascii_case(k));

                if is_sensitive {
                    let after_key = key_end + 1;
                    let rest = &text[after_key..];
                    let trimmed = rest.trim_start();
                    if let Some(stripped) = trimmed.strip_prefix(':') {
                        let after_colon = stripped.trim_start();
                        if let Some(val_quote) = after_colon.chars().next() {
                            if val_quote == '"' || val_quote == '\'' {
                                let val_quote_idx = text.len() - after_colon.len();
                                let val_content_start = val_quote_idx + 1;
                                if let Some(val_end_rel) = text[val_content_start..].find(val_quote)
                                {
                                    let val_content_end = val_content_start + val_end_rel;
                                    let raw_val = &text[val_content_start..val_content_end];

                                    out.push_str(&text[i..val_content_start]);
                                    if raw_val.to_ascii_lowercase().starts_with("bearer ") {
                                        out.push_str("Bearer [REDACTED]");
                                    } else {
                                        out.push_str("[REDACTED]");
                                    }
                                    out.push(val_quote);
                                    i = val_content_end + 1;
                                    continue;
                                }
                            }
                        }
                    }
                }
            }
        }
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn redact_assignments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let len = text.len();
    let mut prev_char: Option<char> = None;

    while i < len {
        let is_boundary = prev_char
            .map(|c| !c.is_alphanumeric() && c != '_')
            .unwrap_or(true);

        if is_boundary {
            let mut matched_key_len = None;
            let rem_bytes = &text.as_bytes()[i..];
            for key in SENSITIVE_KEYS {
                let kbytes = key.as_bytes();
                if rem_bytes.len() >= kbytes.len()
                    && rem_bytes[..kbytes.len()].eq_ignore_ascii_case(kbytes)
                {
                    matched_key_len = Some(kbytes.len());
                    break;
                }
            }

            if let Some(klen) = matched_key_len {
                let after_key = i + klen;
                let rest = &text[after_key..];
                let trimmed = rest.trim_start();
                if let Some(stripped) = trimmed.strip_prefix('=') {
                    let after_eq = stripped.trim_start();
                    let prefix_len = (after_key - i)
                        + (rest.len() - trimmed.len())
                        + 1
                        + (stripped.len() - after_eq.len());
                    out.push_str(&text[i..i + prefix_len]);
                    let val_start = i + prefix_len;

                    if let Some(quote) = after_eq.chars().next() {
                        if quote == '"' || quote == '\'' {
                            let content_start = val_start + 1;
                            if let Some(rel_close) = text[content_start..].find(quote) {
                                out.push(quote);
                                out.push_str("[REDACTED]");
                                out.push(quote);
                                i = content_start + rel_close + 1;
                                prev_char = Some(quote);
                                continue;
                            }
                        }
                    }

                    let mut val_len = 0;
                    for (rel_idx, ch) in text[val_start..].char_indices() {
                        if ch.is_ascii_whitespace()
                            || ch == '&'
                            || ch == ';'
                            || ch == ','
                            || ch == '}'
                            || ch == ']'
                        {
                            break;
                        }
                        val_len = rel_idx + ch.len_utf8();
                    }
                    if val_len > 0 {
                        out.push_str("[REDACTED]");
                        i = val_start + val_len;
                        prev_char = text[val_start..i].chars().last();
                        continue;
                    }
                }
            }
        }

        let ch = text[i..].chars().next().unwrap();
        out.push(ch);
        prev_char = Some(ch);
        i += ch.len_utf8();
    }
    out
}

fn redact_standalone_tokens(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let len = text.len();
    let mut prev_char: Option<char> = None;

    while i < len {
        let is_boundary = prev_char
            .map(|c| !c.is_alphanumeric() && c != '_')
            .unwrap_or(true);

        if is_boundary {
            let rem_bytes = &text.as_bytes()[i..];

            // OpenAI sk-...
            if rem_bytes.len() >= 3 && &rem_bytes[..3] == b"sk-" {
                let mut token_len = 3;
                while token_len < rem_bytes.len()
                    && (rem_bytes[token_len].is_ascii_alphanumeric()
                        || rem_bytes[token_len] == b'_'
                        || rem_bytes[token_len] == b'-')
                {
                    token_len += 1;
                }
                if token_len >= 15 {
                    out.push_str("sk-[REDACTED]");
                    i += token_len;
                    prev_char = Some('-');
                    continue;
                }
            }

            // Google AIza...
            if rem_bytes.len() >= 4 && &rem_bytes[..4] == b"AIza" {
                let mut token_len = 4;
                while token_len < rem_bytes.len()
                    && (rem_bytes[token_len].is_ascii_alphanumeric()
                        || rem_bytes[token_len] == b'_'
                        || rem_bytes[token_len] == b'-')
                {
                    token_len += 1;
                }
                if token_len >= 20 {
                    out.push_str("AIza[REDACTED]");
                    i += token_len;
                    prev_char = Some('a');
                    continue;
                }
            }
        }

        let ch = text[i..].chars().next().unwrap();
        out.push(ch);
        prev_char = Some(ch);
        i += ch.len_utf8();
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
                .map(|d| safe_sanitize_text(truncate_utf8_safe(&d, 32 * 1024)));
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
                safe_sanitize_text(safe)
            });
            bounded_step.text_delta = bounded_delta.clone();

            if let Some(ref err) = bounded_step.error {
                let safe = truncate_utf8_safe(err, 16 * 1024);
                bounded_step.error = Some(safe_sanitize_text(safe));
            }

            let details = serde_json::to_string(&bounded_step)
                .ok()
                .map(|d| safe_sanitize_text(truncate_utf8_safe(&d, 32 * 1024)));

            let content = bounded_delta
                .or_else(|| bounded_step.error.clone())
                .or_else(|| bounded_step.tool_name.clone());

            (
                step_update
                    .step_type
                    .clone()
                    .unwrap_or_else(|| "STEP_UPDATE".to_string()),
                step_update.step_index,
                step_update.state.clone(),
                content,
                details,
            )
        }
        AgyEvent::Result { result } => {
            let mut bounded_result = result.clone();
            bounded_result.response = bounded_result.response.as_ref().map(|r| {
                let safe = truncate_utf8_safe(r, 16 * 1024);
                safe_sanitize_text(safe)
            });
            bounded_result.error = bounded_result.error.as_ref().map(|e| {
                let safe = truncate_utf8_safe(e, 16 * 1024);
                safe_sanitize_text(safe)
            });

            let details = serde_json::to_string(&bounded_result)
                .ok()
                .map(|d| safe_sanitize_text(truncate_utf8_safe(&d, 32 * 1024)));

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

pub struct ActiveExecutionGuard {
    registry: Arc<tokio::sync::Mutex<ActiveBuilderRegistry>>,
    project_id: String,
    session_id: String,
    active: bool,
}

impl ActiveExecutionGuard {
    pub fn new(
        registry: Arc<tokio::sync::Mutex<ActiveBuilderRegistry>>,
        project_id: &str,
        session_id: &str,
    ) -> Self {
        Self {
            registry,
            project_id: project_id.to_string(),
            session_id: session_id.to_string(),
            active: true,
        }
    }

    pub async fn unregister(&mut self) {
        if self.active {
            let mut reg = self.registry.lock().await;
            reg.unregister(&self.project_id, &self.session_id);
            self.active = false;
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }
}

impl Drop for ActiveExecutionGuard {
    fn drop(&mut self) {
        if self.active {
            if let Ok(mut reg) = self.registry.try_lock() {
                reg.unregister(&self.project_id, &self.session_id);
                self.active = false;
            } else {
                let reg_arc = self.registry.clone();
                let pid = self.project_id.clone();
                let sid = self.session_id.clone();
                tokio::spawn(async move {
                    let mut reg = reg_arc.lock().await;
                    reg.unregister(&pid, &sid);
                });
            }
        }
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

        let model_str = request.model.as_deref().unwrap_or("");
        let (effective_effort, pass_effort_flag) =
            resolve_effective_effort(model_str, request.effort.as_deref());

        if pass_effort_flag {
            args.push("--effort".to_string());
            args.push(effective_effort);
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
                provider_status: Some("CANCELLED".to_string()),
                text_response: final_text,
                cumulative_usage,
                was_canceled: true,
                stderr: stderr_buffer,
                has_blocked_actions: false,
            });
        }

        if proc_res.timed_out {
            return Ok(BuilderTurnResponse {
                conversation_id: active_conversation_id,
                status: STATUS_TIMEOUT.to_string(),
                provider_status: Some("TIMEOUT".to_string()),
                text_response: final_text,
                cumulative_usage,
                was_canceled: false,
                stderr: stderr_buffer,
                has_blocked_actions: false,
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

        let raw_provider_status = final_status.clone();
        let canonical_status = normalize_session_status(&raw_provider_status, proc_res.canceled);

        Ok(BuilderTurnResponse {
            conversation_id: active_conversation_id,
            status: canonical_status.to_string(),
            provider_status: Some(raw_provider_status),
            text_response: final_text,
            cumulative_usage,
            was_canceled: false,
            stderr: stderr_buffer,
            has_blocked_actions: false,
        })
    }
}

pub fn normalize_session_status(provider_status: &str, was_canceled: bool) -> &'static str {
    if was_canceled || provider_status == STATUS_CANCELLED || provider_status == "CANCELED" {
        STATUS_CANCELLED
    } else if provider_status == STATUS_TIMEOUT {
        STATUS_TIMEOUT
    } else if provider_status == "SUCCESS" || provider_status == "COMPLETED" {
        STATUS_SUCCESS
    } else {
        STATUS_FAILED
    }
}

pub fn resolve_effective_effort(model: &str, requested_effort: Option<&str>) -> (String, bool) {
    let lower = model.to_lowercase();
    if lower.ends_with("-high") || lower.ends_with("_high") {
        ("high".to_string(), false)
    } else if lower.ends_with("-medium") || lower.ends_with("_medium") {
        ("medium".to_string(), false)
    } else if lower.ends_with("-low") || lower.ends_with("_low") {
        ("low".to_string(), false)
    } else {
        let eff = requested_effort
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("medium")
            .to_string();
        (eff, true)
    }
}

pub fn detect_permission_refusal(
    stderr: &str,
    text_response: &str,
    events: &[AgyEvent],
) -> Vec<(String, String)> {
    let mut refusals: Vec<(String, String)> = Vec::new();

    let phrases = [
        "auto-denied",
        "cannot prompt",
        "tool required the \"command\" permission",
        "tool required the 'command' permission",
        "requires review but running headlessly",
        "permission denied",
        "tool execution denied",
        "confirmation rejected",
    ];

    let combined_text = format!(
        "{}\n{}",
        stderr.to_lowercase(),
        text_response.to_lowercase()
    );

    // 1. Structured event analysis for tool specific denials
    for ev in events {
        if let AgyEvent::StepUpdate { step_update } = ev {
            let is_err = step_update.state.as_deref() == Some("ERROR")
                || step_update.step_type.as_deref() == Some("tool_error")
                || step_update.error.is_some();

            let err_text = step_update
                .error
                .as_deref()
                .or(step_update.text_delta.as_deref())
                .unwrap_or("")
                .to_lowercase();

            let contains_refusal = phrases.iter().any(|p| err_text.contains(p));

            if is_err && contains_refusal {
                let tool = step_update
                    .tool_name
                    .clone()
                    .or_else(|| {
                        step_update
                            .extra
                            .get("tool")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                    })
                    .unwrap_or_else(|| "UNCLASSIFIED_EXTERNAL_ACTION".to_string());

                if !refusals.iter().any(|(t, _)| t == &tool) {
                    refusals.push((
                        tool,
                        format!(
                            "Tool execution refusal reported in Antigravity step update: {}",
                            err_text
                        ),
                    ));
                }
            }
        }
    }

    // 2. Generic refusal fallback (deduplicated: at most one generic refusal)
    if refusals.is_empty() {
        let has_generic = phrases.iter().any(|p| combined_text.contains(p));
        if has_generic {
            refusals.push((
                "UNCLASSIFIED_EXTERNAL_ACTION".to_string(),
                "Antigravity CLI reported a permission denial for an unclassified external action in headless mode. Review security requirements or authorize Icarus mode to grant autonomous tool execution.".to_string(),
            ));
        }
    }

    refusals
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
        Self::start_governed_turn_with_source(
            db_arc,
            registry_arc,
            event_sink,
            project_id,
            model,
            effort,
            custom_adapter,
            None,
            None,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn start_governed_turn_with_source(
        db_arc: Arc<tokio::sync::Mutex<crate::db::DbManager>>,
        registry_arc: Arc<tokio::sync::Mutex<ActiveBuilderRegistry>>,
        event_sink: Option<BuilderEventSink>,
        project_id: &str,
        model: Option<String>,
        effort: Option<String>,
        custom_adapter: Option<AntigravityCliAdapter>,
        instruction_source: Option<BuilderInstructionSource>,
        val_registry_arc: Option<
            Arc<tokio::sync::Mutex<crate::core::validation::ActiveValidationRegistry>>,
        >,
        val_event_sink: Option<crate::core::validation::ValidationEventSink>,
    ) -> Result<BuilderTurnResponse, BuilderError> {
        let effective_val_registry = val_registry_arc.clone().unwrap_or_else(|| {
            Arc::new(tokio::sync::Mutex::new(
                crate::core::validation::ActiveValidationRegistry::new(),
            ))
        });

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

        // 5. Invariant: Builder input comes authoritatively from frozen packet, review corrections, or validation diagnostics.
        let turn_prompt = match &instruction_source {
            Some(BuilderInstructionSource::ValidationDiagnostic { validation_run_id }) => {
                let db = db_arc.lock().await;
                struct RunCheck {
                    project_id: String,
                    architecture_version: String,
                    epoch_id: String,
                }
                let run_info: Option<RunCheck> = db
                    .connection()
                    .query_row(
                        "SELECT project_id, architecture_version, epoch_id FROM validation_runs WHERE run_id = ?1",
                        rusqlite::params![validation_run_id],
                        |r| Ok(RunCheck {
                            project_id: r.get(0)?,
                            architecture_version: r.get(1)?,
                            epoch_id: r.get(2)?,
                        }),
                    )
                    .optional()
                    .map_err(|e| BuilderError::Database(e.to_string()))?;

                let run = run_info.ok_or_else(|| {
                    BuilderError::ExecutionFailed(format!(
                        "Validation run {} not found",
                        validation_run_id
                    ))
                })?;

                if run.project_id != project_id {
                    return Err(BuilderError::ExecutionFailed(format!(
                        "Validation run {} does not belong to project {}",
                        validation_run_id, project_id
                    )));
                }

                if run.architecture_version != _arch_version {
                    return Err(BuilderError::ExecutionFailed(format!(
                        "Validation run {} belongs to architecture version {}, but current is {}",
                        validation_run_id, run.architecture_version, _arch_version
                    )));
                }

                if run.epoch_id != epoch_id {
                    return Err(BuilderError::ExecutionFailed(format!(
                        "Validation run {} belongs to Builder epoch {}, but current is {}",
                        validation_run_id, run.epoch_id, epoch_id
                    )));
                }

                let diagnostic =
                    crate::core::validation::ValidationService::generate_diagnostic_packet_for_run(
                        db.connection(),
                        validation_run_id,
                        50_000,
                    )
                    .map_err(|e| {
                        BuilderError::ExecutionFailed(format!(
                            "Failed to load validation diagnostic: {}",
                            e
                        ))
                    })?;
                let sanitized_diagnostic = safe_sanitize_text(&diagnostic);
                format!(
                    "{}\n\n=== VALIDATION DIAGNOSTIC REPORT ===\n{}",
                    builder_packet.prompt, sanitized_diagnostic
                )
            }
            Some(BuilderInstructionSource::ReviewCorrection { review_cycle_id }) => {
                let db = db_arc.lock().await;
                struct CycleCheck {
                    project_id: String,
                    architecture_version: String,
                    epoch_id: Option<String>,
                    status: String,
                    verdict: Option<String>,
                    completed_at: Option<String>,
                    corrections_packet: Option<String>,
                }
                let cycle_info: Option<CycleCheck> = db
                    .connection()
                    .query_row(
                        "SELECT project_id, architecture_version, epoch_id, status, verdict, completed_at, corrections_packet
                         FROM review_cycles WHERE cycle_id = ?1",
                        rusqlite::params![review_cycle_id],
                        |r| {
                            Ok(CycleCheck {
                                project_id: r.get(0)?,
                                architecture_version: r.get(1)?,
                                epoch_id: r.get(2)?,
                                status: r.get(3)?,
                                verdict: r.get(4)?,
                                completed_at: r.get(5)?,
                                corrections_packet: r.get(6)?,
                            })
                        },
                    )
                    .optional()
                    .map_err(|e| BuilderError::Database(e.to_string()))?;

                let cycle = cycle_info.ok_or_else(|| {
                    BuilderError::ExecutionFailed(format!(
                        "Review cycle {} not found",
                        review_cycle_id
                    ))
                })?;

                if cycle.project_id != project_id {
                    return Err(BuilderError::ExecutionFailed(format!(
                        "Review cycle {} does not belong to project {}",
                        review_cycle_id, project_id
                    )));
                }

                if cycle.status != "CORRECTIONS_REQUIRED"
                    && cycle.verdict.as_deref() != Some("CORRECTIONS_REQUIRED")
                {
                    return Err(BuilderError::ExecutionFailed(format!(
                        "Review cycle {} is not in CORRECTIONS_REQUIRED status (current: status={}, verdict={:?})",
                        review_cycle_id, cycle.status, cycle.verdict
                    )));
                }

                if cycle.completed_at.is_none() {
                    return Err(BuilderError::ExecutionFailed(format!(
                        "Review cycle {} has not been human-confirmed",
                        review_cycle_id
                    )));
                }

                if cycle.architecture_version != _arch_version {
                    return Err(BuilderError::ExecutionFailed(format!(
                        "Review cycle {} belongs to architecture version {}, but current project version is {}",
                        review_cycle_id, cycle.architecture_version, _arch_version
                    )));
                }

                if cycle.epoch_id.as_deref() != Some(epoch_id.as_str()) {
                    return Err(BuilderError::ExecutionFailed(format!(
                        "Review cycle {} belongs to Builder epoch {:?}, but current Builder epoch is {}",
                        review_cycle_id, cycle.epoch_id, epoch_id
                    )));
                }

                let text = cycle.corrections_packet.ok_or_else(|| {
                    BuilderError::ExecutionFailed(format!(
                        "Review cycle {} has no corrections packet. Failing closed.",
                        review_cycle_id
                    ))
                })?;

                if text.trim().is_empty() {
                    return Err(BuilderError::ExecutionFailed(format!(
                        "Review cycle {} corrections packet is empty. Failing closed.",
                        review_cycle_id
                    )));
                }

                let sanitized = safe_sanitize_text(&text);
                format!(
                    "{}\n\n=== REVIEW CORRECTION REQUEST ===\n{}",
                    builder_packet.prompt, sanitized
                )
            }
            _ => builder_packet.prompt.clone(),
        };

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
        let (effective_effort, pass_effort_flag) =
            resolve_effective_effort(&model_name, effort.as_deref());
        let effort_level = Some(effective_effort.clone());

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
        let mut exec_guard =
            ActiveExecutionGuard::new(registry_arc.clone(), project_id, &session_id);

        // 9. Insert initial session record
        let insert_res = {
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
            let res = db.insert_builder_session(&session_rec);
            if res.is_ok() {
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
            res
        };

        if let Err(e) = insert_res {
            exec_guard.unregister().await;
            return Err(BuilderError::Database(format!(
                "Failed to insert builder session: {}",
                e
            )));
        }

        // 10. Channel and event streaming: emit to frontend and persist to builder_events table
        let (tx, mut rx) = mpsc::channel::<AgyEvent>(200);

        let sink_clone = event_sink.clone();
        let session_id_clone = session_id.clone();
        let project_id_clone = project_id.to_string();
        let db_clone = db_arc.clone();
        let captured_events = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let captured_events_clone = captured_events.clone();

        let persist_handle = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                captured_events_clone.lock().await.push(event.clone());

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
                db.insert_builder_event(&rec)
                    .map_err(|e| format!("Failed to insert builder event: {}", e))?;
            }
            Ok::<(), String>(())
        });

        let request = BuilderTurnRequest {
            prompt: turn_prompt,
            conversation_id: existing_conv_id,
            model: Some(model_name.clone()),
            effort: if pass_effort_flag { effort_level } else { None },
            icarus_mode,
            working_dir: Some(repo_path.to_string_lossy().to_string()),
        };

        let turn_start = tokio::time::Instant::now();
        let execution_result = adapter
            .run_turn(request, session_cancel_flag.clone(), Some(tx))
            .await;
        let duration_ms = turn_start.elapsed().as_millis() as u64;
        let completed_at = chrono::Utc::now().to_rfc3339();

        // Await persistence drain before proceeding to ensure no trailing events are lost
        let persist_join_res = persist_handle.await;
        let persistence_failed = match &persist_join_res {
            Ok(Ok(())) => None,
            Ok(Err(e)) => Some(format!("Builder event persistence error: {}", e)),
            Err(e) => Some(format!(
                "Builder event persistence task panicked or failed to join: {}",
                e
            )),
        };

        if let Some(persist_err) = persistence_failed {
            // Terminate/finish the governed process safely if still running
            session_cancel_flag.store(true, std::sync::atomic::Ordering::SeqCst);
            // Unregister execution from registry
            exec_guard.unregister().await;

            let sanitized_err = safe_sanitize_text(&persist_err);
            {
                let db = db_arc.lock().await;
                let _ = db.update_builder_session_status(
                    &session_id,
                    STATUS_FAILED,
                    None,
                    Some(&sanitized_err),
                    Some(&completed_at),
                    duration_ms,
                    &AgyUsage::default(),
                    None,
                );
                let meta = serde_json::json!({
                    "session_id": session_id,
                    "error": sanitized_err,
                    "duration_ms": duration_ms,
                });
                let _ = crate::core::activity::ActivityManager::record_event(
                    db.connection(),
                    project_id,
                    "BUILDER_TURN_FAILED",
                    "BUILDER",
                    &format!("Builder turn failed: {}", sanitized_err),
                    Some(&meta),
                );
            }
            return Err(BuilderError::Database(persist_err));
        }

        // Unregister active execution from registry
        exec_guard.unregister().await;

        let session_events = captured_events.lock().await.clone();

        match execution_result {
            Ok(mut resp) => {
                let db = db_arc.lock().await;
                let final_status = normalize_session_status(&resp.status, resp.was_canceled);
                let sanitized_response = safe_sanitize_text(&resp.text_response);

                db.update_builder_session_status(
                    &session_id,
                    final_status,
                    Some(&sanitized_response),
                    None,
                    Some(&completed_at),
                    duration_ms,
                    &resp.cumulative_usage,
                    resp.conversation_id.as_deref(),
                )
                .map_err(|e| {
                    BuilderError::Database(format!(
                        "Failed to update builder session status: {}",
                        e
                    ))
                })?;

                let raw_provider = resp.provider_status.as_deref().unwrap_or(&resp.status);
                let meta = serde_json::json!({
                    "session_id": session_id,
                    "status": final_status,
                    "provider_status": raw_provider,
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

                // Permission refusal inspection & deduplicated history recording
                let refusals =
                    detect_permission_refusal(&resp.stderr, &resp.text_response, &session_events);
                resp.has_blocked_actions = !refusals.is_empty();

                for (tool_name, reason) in refusals {
                    let perm_rec = PermissionRecord {
                        id: 0,
                        project_id: project_id.to_string(),
                        session_id: Some(session_id.clone()),
                        tool_name,
                        target: None,
                        risk_level: "HIGH_RISK".to_string(),
                        decision: "BLOCKED".to_string(),
                        reason: Some(reason),
                        created_at: completed_at.clone(),
                    };
                    if let Err(audit_err) = db.record_permission_history(&perm_rec) {
                        let err_msg = format!(
                            "Governance audit persistence failure: could not persist permission refusal: {}",
                            audit_err
                        );
                        let sanitized_err = safe_sanitize_text(&err_msg);
                        let _ = db.update_builder_session_status(
                            &session_id,
                            STATUS_FAILED,
                            None,
                            Some(&sanitized_err),
                            Some(&completed_at),
                            duration_ms,
                            &resp.cumulative_usage,
                            resp.conversation_id.as_deref(),
                        );
                        let err_meta = serde_json::json!({
                            "session_id": session_id,
                            "error": sanitized_err,
                            "governance_audit_failure": true,
                        });
                        let _ = crate::core::activity::ActivityManager::record_event(
                            db.connection(),
                            project_id,
                            "BUILDER_AUDIT_PERSISTENCE_FAILED",
                            "GOVERNANCE",
                            &err_msg,
                            Some(&err_meta),
                        );
                        return Err(BuilderError::Database(err_msg));
                    }
                }

                if resp.text_response.contains("COALITION_REQUEST_VALIDATION") {
                    let config_res =
                        crate::core::validation::ValidationService::read_validation_config(
                            &repo_path,
                        );
                    let allow_builder = config_res
                        .as_ref()
                        .map(|c| c.is_builder_requested_run_allowed())
                        .unwrap_or(false);

                    if allow_builder {
                        let _ = crate::core::activity::ActivityManager::record_event(
                            db.connection(),
                            project_id,
                            "BUILDER_REQUESTED_VALIDATION",
                            "BUILDER",
                            "Builder requested validation execution via COALITION_REQUEST_VALIDATION",
                            None,
                        );

                        // Item 1: Real Builder-Requested Validation
                        // Authoritative trigger BUILDER_REQUESTED, ActiveValidationRegistry, human-configured validation
                        // Builder NEVER supplies shell command text, replacement validation commands, working dirs, etc.
                        drop(db);
                        let app_dir = repo_path.join(".coalition");
                        let val_res =
                            crate::core::validation::ValidationService::execute_validation_run(
                                db_arc.clone(),
                                effective_val_registry.clone(),
                                project_id,
                                &repo_path,
                                crate::core::validation::ValidationTriggerSource::BuilderRequested,
                                None,
                                val_event_sink.clone(),
                                &app_dir,
                            )
                            .await;
                        if let Err(e) = val_res {
                            let db = db_arc.lock().await;
                            let meta = serde_json::json!({
                                "session_id": session_id,
                                "error": e.to_string(),
                            });
                            let _ = crate::core::activity::ActivityManager::record_event(
                                db.connection(),
                                project_id,
                                "BUILDER_VALIDATION_FAILED",
                                "VALIDATION_SERVICE",
                                &format!("Builder-requested validation execution failed: {}", e),
                                Some(&meta),
                            );
                        }
                    } else {
                        let meta = serde_json::json!({
                            "reason": "Builder-requested validation runs are disabled in project configuration"
                        });
                        let _ = crate::core::activity::ActivityManager::record_event(
                            db.connection(),
                            project_id,
                            "BUILDER_VALIDATION_REFUSED",
                            "GOVERNANCE",
                            "Refused builder-requested validation: triggers.allow_builder_requested_runs is disabled",
                            Some(&meta),
                        );
                    }
                } else if resp.status == STATUS_SUCCESS && !resp.was_canceled {
                    // Item 2: Wire workflow-triggered post-build validation into production
                    // Successful builder completion drives BUILDING -> VALIDATING -> POST_BUILD validation -> WAITING_FOR_REVIEW
                    if let Some(ref val_reg) = val_registry_arc {
                        drop(db);
                        let app_dir = repo_path.join(".coalition");
                        let val_res =
                            crate::core::validation::ValidationService::run_post_build_validation(
                                db_arc.clone(),
                                val_reg.clone(),
                                project_id,
                                &repo_path,
                                val_event_sink.clone(),
                                &app_dir,
                            )
                            .await;
                        if let Err(e) = val_res {
                            let db = db_arc.lock().await;
                            let meta = serde_json::json!({
                                "session_id": session_id,
                                "error": e.to_string(),
                            });
                            let _ = crate::core::activity::ActivityManager::record_event(
                                db.connection(),
                                project_id,
                                "VALIDATION_ORCHESTRATION_FAILED",
                                "VALIDATION_SERVICE",
                                &format!("Post-build validation orchestration failed: {}", e),
                                Some(&meta),
                            );
                            return Err(BuilderError::ExecutionFailed(format!(
                                "Post-build validation orchestration failed: {}",
                                e
                            )));
                        }
                    }
                }

                Ok(resp)
            }
            Err(e) => {
                let err_str = e.to_string();
                let sanitized_err = safe_sanitize_text(&err_str);
                let db = db_arc.lock().await;

                // Also check if the failure was caused by a permission refusal
                let refusals = detect_permission_refusal(&err_str, "", &session_events);
                for (tool_name, reason) in refusals {
                    let perm_rec = PermissionRecord {
                        id: 0,
                        project_id: project_id.to_string(),
                        session_id: Some(session_id.clone()),
                        tool_name,
                        target: None,
                        risk_level: "HIGH_RISK".to_string(),
                        decision: "BLOCKED".to_string(),
                        reason: Some(reason),
                        created_at: completed_at.clone(),
                    };
                    if let Err(audit_err) = db.record_permission_history(&perm_rec) {
                        let err_msg = format!(
                            "Governance audit persistence failure: could not persist permission refusal: {}",
                            audit_err
                        );
                        let sanitized_err = safe_sanitize_text(&err_msg);
                        let _ = db.update_builder_session_status(
                            &session_id,
                            STATUS_FAILED,
                            None,
                            Some(&sanitized_err),
                            Some(&completed_at),
                            duration_ms,
                            &AgyUsage::default(),
                            None,
                        );
                        let err_meta = serde_json::json!({
                            "session_id": session_id,
                            "error": sanitized_err,
                            "governance_audit_failure": true,
                        });
                        let _ = crate::core::activity::ActivityManager::record_event(
                            db.connection(),
                            project_id,
                            "BUILDER_AUDIT_PERSISTENCE_FAILED",
                            "GOVERNANCE",
                            &err_msg,
                            Some(&err_meta),
                        );
                        return Err(BuilderError::Database(err_msg));
                    }
                }

                db.update_builder_session_status(
                    &session_id,
                    STATUS_FAILED,
                    None,
                    Some(&sanitized_err),
                    Some(&completed_at),
                    duration_ms,
                    &AgyUsage::default(),
                    None,
                )
                .map_err(|e| {
                    BuilderError::Database(format!(
                        "Failed to update builder session status on failure: {}",
                        e
                    ))
                })?;

                let meta = serde_json::json!({
                    "session_id": session_id,
                    "error": sanitized_err,
                    "duration_ms": duration_ms,
                });
                let _ = crate::core::activity::ActivityManager::record_event(
                    db.connection(),
                    project_id,
                    "BUILDER_TURN_FAILED",
                    "BUILDER",
                    &format!("Builder turn failed: {}", sanitized_err),
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
        // 1. Try cancelling active execution in memory first
        let active_cancel_result = {
            let registry = registry_arc.lock().await;
            if let Some(sid) = session_id {
                registry
                    .cancel_session(sid)
                    .map(|pid| (pid, sid.to_string()))
            } else if let Some(pid) = project_id {
                registry
                    .cancel_project(pid)
                    .map(|sid| (pid.to_string(), sid))
            } else {
                return Err(BuilderError::ExecutionFailed(
                    "Either project_id or session_id must be provided to cancel execution"
                        .to_string(),
                ));
            }
        };

        if let Ok((_pid, sid)) = active_cancel_result {
            // Process tree termination signaled to active worker
            return Ok(sid);
        }

        // 2. Active registry had no live execution. Reconcile stale RUNNING rows in SQLite.
        let db = db_arc.lock().await;
        let now = chrono::Utc::now().to_rfc3339();

        if let Some(sid) = session_id {
            let row: Result<(String, String), _> = db.connection().query_row(
                "SELECT session_id, project_id FROM builder_sessions WHERE session_id = ?1 AND status = 'RUNNING'",
                rusqlite::params![sid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            );

            match row {
                Ok((target_sid, proj_id)) => {
                    if let Some(pid) = project_id {
                        if proj_id != pid {
                            return Err(BuilderError::ExecutionFailed(format!(
                                "Session '{}' belongs to project '{}', not '{}'",
                                target_sid, proj_id, pid
                            )));
                        }
                    }

                    db.connection().execute(
                        "UPDATE builder_sessions SET status = ?1, completed_at = ?2 WHERE session_id = ?3",
                        rusqlite::params![STATUS_INTERRUPTED, now, target_sid],
                    ).map_err(|e| BuilderError::Database(e.to_string()))?;

                    let meta = serde_json::json!({
                        "session_id": target_sid,
                        "reconciled_status": STATUS_INTERRUPTED,
                    });
                    let _ = crate::core::activity::ActivityManager::record_event(
                        db.connection(),
                        &proj_id,
                        "BUILDER_TURN_INTERRUPTED",
                        "HUMAN",
                        &format!(
                            "Reconciled orphaned running Builder session '{}' to interrupted",
                            target_sid
                        ),
                        Some(&meta),
                    );

                    Ok(target_sid)
                }
                Err(_) => Err(BuilderError::SessionNotFound(format!(
                    "No running or active session found with ID '{}'",
                    sid
                ))),
            }
        } else if let Some(pid) = project_id {
            let mut stmt = db.connection().prepare(
                "SELECT session_id FROM builder_sessions WHERE project_id = ?1 AND status = 'RUNNING'"
            ).map_err(|e| BuilderError::Database(e.to_string()))?;

            let rows = stmt
                .query_map(rusqlite::params![pid], |r| r.get::<_, String>(0))
                .map_err(|e| BuilderError::Database(e.to_string()))?;

            let mut orphaned_ids = Vec::new();
            for r in rows {
                orphaned_ids.push(r.map_err(|e| BuilderError::Database(e.to_string()))?);
            }

            if orphaned_ids.is_empty() {
                return Err(BuilderError::SessionNotFound(format!(
                    "No active or stale running sessions found for project '{}'",
                    pid
                )));
            }

            for sid in &orphaned_ids {
                db.connection().execute(
                    "UPDATE builder_sessions SET status = ?1, completed_at = ?2 WHERE session_id = ?3",
                    rusqlite::params![STATUS_INTERRUPTED, now, sid],
                ).map_err(|e| BuilderError::Database(e.to_string()))?;

                let meta = serde_json::json!({
                    "session_id": sid,
                    "reconciled_status": STATUS_INTERRUPTED,
                });
                let _ = crate::core::activity::ActivityManager::record_event(
                    db.connection(),
                    pid,
                    "BUILDER_TURN_INTERRUPTED",
                    "HUMAN",
                    &format!(
                        "Reconciled orphaned running Builder session '{}' to interrupted",
                        sid
                    ),
                    Some(&meta),
                );
            }

            Ok(orphaned_ids.join(", "))
        } else {
            Err(BuilderError::ExecutionFailed(
                "Project ID required".to_string(),
            ))
        }
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

        assert!(response
            .conversation_id
            .as_deref()
            .unwrap_or("")
            .starts_with("fake-conv-uuid"));
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

    #[test]
    fn test_harden_secret_redaction() {
        let sample = r#"
Authorization: Bearer secret-token-12345
Bearer my-special-jwt-bearer-token
Here is an openai key: sk-abcdef1234567890abcdef and google key AIzaSyD1234567890abcdef1234567890
In json: {"api_key": "sk-abcdef1234567890", "token": "jwt-secret", "authorization": "Bearer my-auth-token", "other": "safe"}
CLI flag: --api_key=mysecretpass123 --token="quoted-secret" password=my-pass; next=ok
"#;

        let res = sanitize_text(sample);
        assert!(
            !res.contains("secret-token-12345"),
            "Bearer token must be redacted"
        );
        assert!(
            !res.contains("my-special-jwt-bearer-token"),
            "Standalone bearer token must be redacted"
        );
        assert!(
            !res.contains("sk-abcdef1234567890abcdef"),
            "OpenAI key must be redacted"
        );
        assert!(
            !res.contains("AIzaSyD1234567890abcdef1234567890"),
            "Google key must be redacted"
        );
        assert!(
            !res.contains("sk-abcdef1234567890"),
            "JSON api_key value must be redacted"
        );
        assert!(
            !res.contains("jwt-secret"),
            "JSON token value must be redacted"
        );
        assert!(
            !res.contains("my-auth-token"),
            "JSON authorization token value must be redacted"
        );
        assert!(
            !res.contains("mysecretpass123"),
            "CLI api_key flag value must be redacted"
        );
        assert!(
            !res.contains("quoted-secret"),
            "CLI quoted token value must be redacted"
        );
        assert!(
            !res.contains("my-pass"),
            "Assignment password value must be redacted"
        );
        assert!(
            res.contains(r#""other": "safe""#),
            "Non-sensitive JSON fields must be preserved"
        );
    }

    #[test]
    fn test_secret_never_survives_persisted_event() {
        let secret_key = "sk-live12345678901234567890";
        let secret_token = "my-bearer-secret-credential";
        let step_event = AgyEvent::StepUpdate {
            step_update: AgyStepUpdateData {
                conversation_id: Some("conv-1".to_string()),
                step_index: Some(1),
                step_type: Some("agent_response".to_string()),
                state: Some("DONE".to_string()),
                text_delta: Some(format!(
                    "Using key {} with Authorization: Bearer {}",
                    secret_key, secret_token
                )),
                duration_seconds: Some(0.5),
                usage: None,
                tool_name: None,
                error: None,
                extra: Default::default(),
            },
        };

        let rec = sanitize_and_bound_event_record(&step_event, "sess-1", "proj-1");

        // Verify content_str does not contain any secret
        let content = rec.content.expect("content must be present");
        assert!(
            !content.contains(secret_key),
            "Secret key must not survive in content_str"
        );
        assert!(
            !content.contains(secret_token),
            "Secret token must not survive in content_str"
        );
        assert!(
            content.contains("[REDACTED]"),
            "Content must contain redaction placeholder"
        );

        // Verify details_json does not contain any secret
        let details = rec.details_json.expect("details must be present");
        assert!(
            !details.contains(secret_key),
            "Secret key must not survive in details_json"
        );
        assert!(
            !details.contains(secret_token),
            "Secret token must not survive in details_json"
        );
        assert!(
            details.contains("[REDACTED]"),
            "Details JSON must contain redaction placeholder"
        );
    }

    #[test]
    fn test_secret_redacted_in_session_response_and_errors() {
        let raw_response = "Finished task. Used Authorization: Bearer secret-token-xyz and sk-abcdef1234567890abcdef";
        let raw_error = "Execution failed with key AIzaSyD1234567890abcdef1234567890 while executing --api_key=mysecret";

        let sanitized_resp = sanitize_text(raw_response);
        assert!(!sanitized_resp.contains("secret-token-xyz"));
        assert!(!sanitized_resp.contains("sk-abcdef1234567890abcdef"));
        assert!(sanitized_resp.contains("Bearer [REDACTED]"));
        assert!(sanitized_resp.contains("[REDACTED]"));

        let sanitized_err = sanitize_text(raw_error);
        assert!(!sanitized_err.contains("AIzaSyD1234567890abcdef1234567890"));
        assert!(!sanitized_err.contains("mysecret"));
        assert!(sanitized_err.contains("AIza[REDACTED]"));
        assert!(sanitized_err.contains("[REDACTED]"));
    }

    #[test]
    fn test_utf8_sanitizer_with_em_dash_completion_report() {
        let text_with_em_dash = "Architecture v1.0 — Completion Report";
        let sanitized = sanitize_text(text_with_em_dash);
        assert_eq!(sanitized, "Architecture v1.0 — Completion Report");

        // The exact panic message from the live incident
        let incident_panic = "thread 'tokio-rt-worker' panicked at src\\core\\builder\\mod.rs:332:32:\nend byte index 19 is not a char boundary; it is inside '—' (bytes 17..20) of `Architecture v1.0 — Completion Report`";
        let sanitized_panic = safe_sanitize_text(incident_panic);
        assert!(sanitized_panic.contains("Architecture v1.0 — Completion Report"));
    }

    #[test]
    fn test_utf8_sanitizer_multibyte_characters_and_adjacent_secrets() {
        let complex_unicode = r#"
Status: 🚀 Launching architecture v1.0 – preliminary review…
Quotes: “Bearer secret-token-inside-quotes” and ‘sk-12345678901234567’
CJK: 日本語のテスト and 中文测试 with api_key=“my-cjk-secret-pass”
Em-dash adjacent: —Bearer em-dash-secret-token—
Accented: café and español with password=secret-password-123; next=val
"#;

        let sanitized = sanitize_text(complex_unicode);
        assert!(!sanitized.contains("secret-token-inside-quotes"));
        assert!(!sanitized.contains("sk-12345678901234567"));
        assert!(!sanitized.contains("my-cjk-secret-pass"));
        assert!(!sanitized.contains("em-dash-secret-token"));
        assert!(!sanitized.contains("secret-password-123"));
        assert!(sanitized.contains("🚀"));
        assert!(sanitized.contains("–"));
        assert!(sanitized.contains("日本語のテスト"));
        assert!(sanitized.contains("café"));
    }

    #[test]
    fn test_effective_effort_resolution() {
        // Model with suffix overrides requested effort and omits CLI flag
        let (eff1, pass1) = resolve_effective_effort("gemini-3.8-flash-high", Some("medium"));
        assert_eq!(eff1, "high");
        assert!(!pass1, "Must omit --effort when model encodes suffix");

        let (eff2, pass2) = resolve_effective_effort("gemini-3.8-flash-low", None);
        assert_eq!(eff2, "low");
        assert!(!pass2);

        let (eff3, pass3) = resolve_effective_effort("gpt-oss-120b_medium", Some("high"));
        assert_eq!(eff3, "medium");
        assert!(!pass3);

        // Standard model without suffix honors requested effort and passes CLI flag
        let (eff4, pass4) = resolve_effective_effort("claude-sonnet-4-6", Some("high"));
        assert_eq!(eff4, "high");
        assert!(pass4, "Must pass --effort for standard models");

        let (eff5, pass5) = resolve_effective_effort("claude-sonnet-4-6", None);
        assert_eq!(eff5, "medium");
        assert!(pass5);
    }

    #[test]
    fn test_canonical_session_status_normalization() {
        assert_eq!(normalize_session_status("SUCCESS", false), STATUS_SUCCESS);
        assert_eq!(normalize_session_status("COMPLETED", false), STATUS_SUCCESS);
        assert_eq!(normalize_session_status("ERROR", false), STATUS_FAILED);
        assert_eq!(
            normalize_session_status("UNKNOWN_ERROR", false),
            STATUS_FAILED
        );
        assert_eq!(normalize_session_status("TIMEOUT", false), STATUS_TIMEOUT);
        assert_eq!(
            normalize_session_status("CANCELLED", false),
            STATUS_CANCELLED
        );
        assert_eq!(
            normalize_session_status("CANCELED", false),
            STATUS_CANCELLED
        );
        assert_eq!(normalize_session_status("SUCCESS", true), STATUS_CANCELLED);
    }

    #[test]
    fn test_permission_refusal_detection_and_deduplication() {
        let stderr = "Warning: tool auto-denied: requires review but running headlessly\n";
        let step = AgyEvent::StepUpdate {
            step_update: AgyStepUpdateData {
                conversation_id: Some("c-1".to_string()),
                step_index: Some(1),
                state: Some("ERROR".to_string()),
                step_type: Some("tool_error".to_string()),
                text_delta: None,
                duration_seconds: None,
                usage: None,
                tool_name: Some("run_command".to_string()),
                error: Some("tool required the \"command\" permission: auto-denied".to_string()),
                extra: std::collections::HashMap::new(),
            },
        };

        let refusals = detect_permission_refusal(stderr, "response text", &[step]);
        assert_eq!(
            refusals.len(),
            1,
            "Must deduplicate and record specific tool refusal"
        );
        assert_eq!(refusals[0].0, "run_command");

        // Generic fallback test when no structured tool is specified
        let generic_stderr = "Error: action was auto-denied by security governance";
        let generic_refusals = detect_permission_refusal(generic_stderr, "", &[]);
        assert_eq!(generic_refusals.len(), 1);
        assert_eq!(generic_refusals[0].0, "UNCLASSIFIED_EXTERNAL_ACTION");
    }

    #[tokio::test]
    async fn test_stale_cancellation_reconciles_multiple_orphaned_running_sessions() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let db_path = dir.path().join("test_orphan.db");
        let mut db = crate::db::DbManager::open(&db_path).expect("init db");
        db.run_migrations().expect("run migrations");
        let project_id = "proj-multi-orphan";

        db.connection()
            .execute(
                "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at) VALUES (?1, ?2, ?3, ?4, ?4, ?4)",
                rusqlite::params![project_id, "Orphan Project", "/fake/repo", "2026-09-23T00:00:00Z"],
            )
            .expect("insert project");

        let sess1 = BuilderSessionRecord {
            session_id: "sess-orphan-1".to_string(),
            project_id: project_id.to_string(),
            epoch_id: "epoch-1".to_string(),
            conversation_id: None,
            model: "gemini-3.8-flash-high".to_string(),
            effort: Some("high".to_string()),
            icarus_mode: false,
            status: STATUS_RUNNING.to_string(),
            prompt: "prompt 1".to_string(),
            response_text: None,
            error_message: None,
            started_at: "2026-09-23T10:00:00Z".to_string(),
            completed_at: None,
            duration_ms: 0,
            usage: AgyUsage::default(),
        };
        db.insert_builder_session(&sess1).expect("insert sess1");

        let sess2 = BuilderSessionRecord {
            session_id: "sess-orphan-2".to_string(),
            project_id: project_id.to_string(),
            epoch_id: "epoch-1".to_string(),
            conversation_id: None,
            model: "gemini-3.8-flash-high".to_string(),
            effort: Some("high".to_string()),
            icarus_mode: false,
            status: STATUS_RUNNING.to_string(),
            prompt: "prompt 2".to_string(),
            response_text: None,
            error_message: None,
            started_at: "2026-09-23T10:05:00Z".to_string(),
            completed_at: None,
            duration_ms: 0,
            usage: AgyUsage::default(),
        };
        db.insert_builder_session(&sess2).expect("insert sess2");

        let registry = Arc::new(tokio::sync::Mutex::new(ActiveBuilderRegistry::new()));
        let db_arc = Arc::new(tokio::sync::Mutex::new(db));

        // Reconcile project-scoped stale sessions
        let res =
            BuilderService::cancel_turn(registry.clone(), db_arc.clone(), Some(project_id), None)
                .await
                .expect("reconcile stale sessions");

        assert!(res.contains("sess-orphan-1"));
        assert!(res.contains("sess-orphan-2"));

        let db_guard = db_arc.lock().await;
        let s1 = db_guard
            .get_builder_session("sess-orphan-1")
            .expect("get s1")
            .unwrap();
        let s2 = db_guard
            .get_builder_session("sess-orphan-2")
            .expect("get s2")
            .unwrap();

        assert_eq!(s1.status, STATUS_INTERRUPTED);
        assert_eq!(s2.status, STATUS_INTERRUPTED);
        assert!(s1.completed_at.is_some());
        assert!(s2.completed_at.is_some());

        let reg_guard = registry.lock().await;
        assert!(!reg_guard.is_active(project_id));
    }

    #[tokio::test]
    async fn test_provider_raw_error_normalizes_to_failed() {
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
                    prompt: "trigger_raw_error".to_string(),
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
            .expect("should return turn response");

        assert_eq!(
            res.status, STATUS_FAILED,
            "Raw provider ERROR must normalize to canonical FAILED"
        );
        assert_eq!(
            res.provider_status.as_deref(),
            Some("ERROR"),
            "Raw provider status must preserve 'ERROR'"
        );
    }

    #[tokio::test]
    async fn test_live_incident_full_sequence_regression() {
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

        let dir = tempfile::tempdir().expect("create temp dir");
        let db_path = dir.path().join("test_incident.db");
        let mut db = crate::db::DbManager::open(&db_path).expect("init db");
        db.run_migrations().expect("run migrations");
        let project_id = "proj-incident-regress";

        db.connection()
            .execute(
                "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at) VALUES (?1, ?2, ?3, ?4, ?4, ?4)",
                rusqlite::params![project_id, "Incident Project", dir.path().to_string_lossy(), "2026-09-23T00:00:00Z"],
            )
            .expect("insert project");

        let adapter = AntigravityCliAdapter::with_path(fake_agy_path);
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, mut rx) = mpsc::channel::<AgyEvent>(100);

        // Turn 1: Least-privilege run with auto-denied command
        let res1 = adapter
            .run_turn(
                BuilderTurnRequest {
                    prompt: "trigger_least_privilege_blocked_command".to_string(),
                    conversation_id: None,
                    model: Some("gemini-3.8-flash-high".to_string()),
                    effort: Some("medium".to_string()), // Contradictory effort: should be ignored in favor of -high
                    icarus_mode: false,
                    working_dir: None,
                },
                cancel.clone(),
                Some(tx),
            )
            .await
            .expect("run turn 1");

        let mut events1 = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events1.push(ev);
        }

        let refusals = detect_permission_refusal(&res1.stderr, &res1.text_response, &events1);
        assert_eq!(refusals.len(), 1, "Must detect exactly one tool refusal");
        assert_eq!(
            refusals[0].0, "run_command",
            "Must capture structured tool name"
        );

        // Record to DB as start_governed_turn does
        for (tool_name, reason) in &refusals {
            let perm_rec = PermissionRecord {
                id: 0,
                project_id: project_id.to_string(),
                session_id: Some("sess-turn-1".to_string()),
                tool_name: tool_name.clone(),
                target: None,
                risk_level: "HIGH_RISK".to_string(),
                decision: "BLOCKED".to_string(),
                reason: Some(reason.clone()),
                created_at: "2026-09-23T11:00:00Z".to_string(),
            };
            db.record_permission_history(&perm_rec)
                .expect("record refusal");
        }

        let db_perms = db
            .list_permission_history(project_id, 10)
            .expect("list perms");
        assert_eq!(db_perms.len(), 1);
        assert_eq!(db_perms[0].decision, "BLOCKED");
        assert_eq!(db_perms[0].tool_name, "run_command");

        // Canonical session status remains SUCCESS even though action was blocked
        assert_eq!(res1.status, STATUS_SUCCESS);

        // Turn 2: User enables Icarus mode, response contains Unicode em-dash and completion report
        let (tx2, mut rx2) = mpsc::channel::<AgyEvent>(100);
        let res2 = adapter
            .run_turn(
                BuilderTurnRequest {
                    prompt: "trigger_unicode_completion_report".to_string(),
                    conversation_id: None,
                    model: Some("gemini-3.8-flash-high".to_string()),
                    effort: None,
                    icarus_mode: true,
                    working_dir: None,
                },
                cancel.clone(),
                Some(tx2),
            )
            .await
            .expect("run turn 2");

        let mut events2 = Vec::new();
        while let Ok(ev) = rx2.try_recv() {
            events2.push(ev);
        }

        // Test that event record sanitization does not panic on em dash
        for ev in &events2 {
            let rec = sanitize_and_bound_event_record(ev, "sess-turn-2", project_id);
            assert!(rec.content.is_some() || rec.details_json.is_some());
        }

        assert_eq!(res2.status, STATUS_SUCCESS);
        assert!(res2
            .text_response
            .contains("Architecture v1.0 — Completion Report"));
    }
}

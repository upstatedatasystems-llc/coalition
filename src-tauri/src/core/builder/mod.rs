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

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct AgyUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub thinking_tokens: u64,
    pub cache_read_tokens: u64,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatGptUsageSummary {
    pub rolling_5h_tokens: u64,
    pub rolling_7d_tokens: u64,
    pub total_tokens: u64,
    pub total_packets_sent: u64,
    pub total_imports_received: u64,
    pub last_calibrated_at: Option<String>,
    pub disclaimer: String,
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
                ("HIGH", "Arbitrary command line process execution")
            }
        }
        "write_to_file" => ("MEDIUM", "File modification or creation"),
        "replace_file_content" => ("MEDIUM", "In-place file mutation"),
        "view_file" => ("LOW", "Read-only file inspection"),
        "list_dir" => ("LOW", "Directory listing"),
        "grep_search" => ("LOW", "Read-only pattern search"),
        "find_by_name" => ("LOW", "File search"),
        _ => ("HIGH", "External tool invocation with unclassified risk"),
    }
}

pub struct AntigravityCliAdapter {
    bin_path: PathBuf,
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
        let output = std::process::Command::new(&self.bin_path)
            .arg("--version")
            .output()?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            Err(BuilderError::ExecutionFailed(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ))
        }
    }

    pub fn list_models(&self) -> Result<Vec<ModelInfo>, BuilderError> {
        let output = std::process::Command::new(&self.bin_path)
            .arg("models")
            .output()?;

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
                                    final_text.push_str(delta);
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
                                        final_text = resp.clone();
                                    }
                                }
                                if let Some(ref err) = result.error {
                                    if !err.is_empty() {
                                        stderr_buffer.push_str(err);
                                        stderr_buffer.push('\n');
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
                    stderr_buffer.push_str(&out_line.line);
                    stderr_buffer.push('\n');
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
                status: "CANCELED".to_string(),
                text_response: final_text,
                cumulative_usage,
                was_canceled: true,
                stderr: stderr_buffer,
            });
        }

        if proc_res.timed_out {
            return Ok(BuilderTurnResponse {
                conversation_id: active_conversation_id,
                status: "TIMEOUT".to_string(),
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
                if final_status == "UNKNOWN" || final_status == "SUCCESS" {
                    final_status = format!("EXIT_{}", code);
                }
            }
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

        assert_eq!(res.status, "ERROR");
        assert!(res.stderr.contains("permission denied"));
    }
}

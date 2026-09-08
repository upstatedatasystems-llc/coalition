use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
        let mut cmd = Command::new(&self.bin_path);
        cmd.arg("--input-format").arg("stream-json");
        cmd.arg("--output-format").arg("stream-json");

        if let Some(ref conv_id) = request.conversation_id {
            cmd.arg("--conversation").arg(conv_id);
        }

        if let Some(ref model) = request.model {
            cmd.arg("--model").arg(model);
        }

        if let Some(ref effort) = request.effort {
            cmd.arg("--effort").arg(effort);
        }

        if request.icarus_mode {
            cmd.arg("--dangerously-skip-permissions");
        }

        if let Some(ref cwd) = request.working_dir {
            cmd.current_dir(cwd);
        }

        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd.spawn()?;
        let mut stdin = child.stdin.take().ok_or_else(|| {
            BuilderError::ExecutionFailed("Failed to open child stdin".to_string())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            BuilderError::ExecutionFailed("Failed to open child stdout".to_string())
        })?;

        // Construct stream input message
        let input_msg = serde_json::json!({
            "event": "user",
            "message": {
                "content": request.prompt
            }
        });
        let mut input_line = serde_json::to_string(&input_msg)
            .map_err(|e| BuilderError::ParseError(e.to_string()))?;
        input_line.push('\n');

        stdin.write_all(input_line.as_bytes()).await?;
        stdin.flush().await?;
        drop(stdin); // Close stdin to signal end of stream turn

        let mut stdout_reader = BufReader::new(stdout).lines();

        let mut active_conversation_id = request.conversation_id.clone();
        let mut final_text = String::new();
        let mut final_status = "UNKNOWN".to_string();
        let mut cumulative_usage = AgyUsage::default();
        let mut was_canceled = false;

        loop {
            if cancel_flag.load(Ordering::Relaxed) {
                was_canceled = true;
                let _ = child.kill().await;
                break;
            }

            tokio::select! {
                line_res = stdout_reader.next_line() => {
                    match line_res {
                        Ok(Some(line)) => {
                            if let Ok(event) = serde_json::from_str::<AgyEvent>(&line) {
                                match &event {
                                    AgyEvent::Init { conversation_id, .. } => {
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
                                    }
                                    AgyEvent::Unknown => {}
                                }

                                if let Some(tx) = &event_sender {
                                    let _ = tx.send(event).await;
                                }
                            }
                        }
                        Ok(None) => break,
                        Err(e) => {
                            eprintln!("Error reading stdout from agy: {}", e);
                            break;
                        }
                    }
                }
                status_res = child.wait() => {
                    if let Ok(exit) = status_res {
                        if !exit.success() && final_status == "UNKNOWN" {
                            final_status = format!("EXIT_{}", exit.code().unwrap_or(1));
                        }
                    }
                    break;
                }
                _ = tokio::time::sleep(Duration::from_millis(50)) => {}
            }
        }

        Ok(BuilderTurnResponse {
            conversation_id: active_conversation_id,
            status: if was_canceled {
                "CANCELED".to_string()
            } else {
                final_status
            },
            text_response: final_text,
            cumulative_usage,
            was_canceled,
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
    }
}

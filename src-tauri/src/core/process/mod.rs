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

pub const MAX_ACCUMULATED_LINES: usize = 10_000;
pub const MAX_ACCUMULATED_BYTES: usize = 20 * 1024 * 1024; // 20 MB

#[derive(Error, Debug)]
pub enum ProcessError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Process timed out after {0:?}")]
    Timeout(Duration),
    #[error("Process was canceled")]
    Canceled,
    #[error("Execution error: {0}")]
    Execution(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProcessOutputKind {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessOutputLine {
    pub kind: ProcessOutputKind,
    pub line: String,
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessResult {
    pub exit_code: Option<i32>,
    pub output_lines: Vec<ProcessOutputLine>,
    pub canceled: bool,
    pub timed_out: bool,
    pub duration_ms: u64,
    pub is_truncated: bool,
}

#[cfg(target_os = "windows")]
pub async fn terminate_process_tree(pid: u32) {
    // Windows taskkill /F /T terminates process and all its children
    let _ = Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .output()
        .await;
}

#[cfg(not(target_os = "windows"))]
pub async fn terminate_process_tree(_pid: u32) {
    // Unix fallback
}

pub struct ProcessRunner;

impl ProcessRunner {
    pub async fn run_turn_stream(
        program: &Path,
        args: &[String],
        cwd: Option<&Path>,
        stdin_payload: Option<&str>,
        timeout_duration: Duration,
        cancel_flag: Arc<AtomicBool>,
        event_sender: Option<mpsc::Sender<ProcessOutputLine>>,
    ) -> Result<ProcessResult, ProcessError> {
        let start_time = tokio::time::Instant::now();

        let mut cmd = Command::new(program);
        cmd.args(args);
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        if stdin_payload.is_some() {
            cmd.stdin(Stdio::piped());
        } else {
            cmd.stdin(Stdio::null());
        }
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd.spawn()?;

        // If stdin payload was provided, write it and flush/close
        if let Some(payload) = stdin_payload {
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(payload.as_bytes()).await?;
                stdin.flush().await?;
                drop(stdin); // Signal EOF
            }
        }

        let stdout = child.stdout.take().ok_or_else(|| {
            ProcessError::Execution("Failed to open child stdout pipe".to_string())
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            ProcessError::Execution("Failed to open child stderr pipe".to_string())
        })?;

        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();

        let mut stdout_done = false;
        let mut stderr_done = false;

        let mut accumulated_lines: Vec<ProcessOutputLine> = Vec::new();
        let mut total_bytes: usize = 0;
        let mut is_truncated = false;

        let mut was_canceled = false;
        let mut was_timed_out = false;

        let poll_interval = Duration::from_millis(25);
        let mut exit_status = None;

        loop {
            // Check cancellation flag
            if cancel_flag.load(Ordering::Relaxed) {
                was_canceled = true;
                if let Some(pid) = child.id() {
                    terminate_process_tree(pid).await;
                }
                let _ = child.kill().await;
                break;
            }

            // Check overall timeout
            if start_time.elapsed() > timeout_duration {
                was_timed_out = true;
                if let Some(pid) = child.id() {
                    terminate_process_tree(pid).await;
                }
                let _ = child.kill().await;
                break;
            }

            tokio::select! {
                line_res = stdout_reader.next_line(), if !stdout_done => {
                    match line_res {
                        Ok(Some(line)) => {
                            let bytes = line.len();
                            let out = ProcessOutputLine {
                                kind: ProcessOutputKind::Stdout,
                                line,
                                timestamp_ms: chrono::Utc::now().timestamp_millis(),
                            };
                            if let Some(tx) = &event_sender {
                                let _ = tx.send(out.clone()).await;
                            }
                            if !is_truncated {
                                if accumulated_lines.len() >= MAX_ACCUMULATED_LINES || total_bytes + bytes >= MAX_ACCUMULATED_BYTES {
                                    is_truncated = true;
                                } else {
                                    total_bytes += bytes;
                                    accumulated_lines.push(out);
                                }
                            }
                        }
                        Ok(None) => {
                            stdout_done = true;
                        }
                        Err(e) => {
                            eprintln!("Error reading child stdout: {}", e);
                            stdout_done = true;
                        }
                    }
                }
                err_res = stderr_reader.next_line(), if !stderr_done => {
                    match err_res {
                        Ok(Some(line)) => {
                            let bytes = line.len();
                            let out = ProcessOutputLine {
                                kind: ProcessOutputKind::Stderr,
                                line,
                                timestamp_ms: chrono::Utc::now().timestamp_millis(),
                            };
                            if let Some(tx) = &event_sender {
                                let _ = tx.send(out.clone()).await;
                            }
                            if !is_truncated {
                                if accumulated_lines.len() >= MAX_ACCUMULATED_LINES || total_bytes + bytes >= MAX_ACCUMULATED_BYTES {
                                    is_truncated = true;
                                } else {
                                    total_bytes += bytes;
                                    accumulated_lines.push(out);
                                }
                            }
                        }
                        Ok(None) => {
                            stderr_done = true;
                        }
                        Err(e) => {
                            eprintln!("Error reading child stderr: {}", e);
                            stderr_done = true;
                        }
                    }
                }
                status_res = child.wait() => {
                    match status_res {
                        Ok(status) => {
                            exit_status = Some(status.code().unwrap_or(0));
                            // Drain remaining lines
                            while let Ok(Some(line)) = stdout_reader.next_line().await {
                                let bytes = line.len();
                                let out = ProcessOutputLine {
                                    kind: ProcessOutputKind::Stdout,
                                    line,
                                    timestamp_ms: chrono::Utc::now().timestamp_millis(),
                                };
                                if let Some(tx) = &event_sender {
                                    let _ = tx.send(out.clone()).await;
                                }
                                if !is_truncated {
                                    if accumulated_lines.len() >= MAX_ACCUMULATED_LINES || total_bytes + bytes >= MAX_ACCUMULATED_BYTES {
                                        is_truncated = true;
                                    } else {
                                        total_bytes += bytes;
                                        accumulated_lines.push(out);
                                    }
                                }
                            }
                            while let Ok(Some(line)) = stderr_reader.next_line().await {
                                let bytes = line.len();
                                let out = ProcessOutputLine {
                                    kind: ProcessOutputKind::Stderr,
                                    line,
                                    timestamp_ms: chrono::Utc::now().timestamp_millis(),
                                };
                                if let Some(tx) = &event_sender {
                                    let _ = tx.send(out.clone()).await;
                                }
                                if !is_truncated {
                                    if accumulated_lines.len() >= MAX_ACCUMULATED_LINES || total_bytes + bytes >= MAX_ACCUMULATED_BYTES {
                                        is_truncated = true;
                                    } else {
                                        total_bytes += bytes;
                                        accumulated_lines.push(out);
                                    }
                                }
                            }
                            break;
                        }
                        Err(e) => return Err(ProcessError::Io(e)),
                    }
                }
                _ = tokio::time::sleep(poll_interval) => {}
            }
        }

        let duration_ms = start_time.elapsed().as_millis() as u64;

        Ok(ProcessResult {
            exit_code: exit_status,
            output_lines: accumulated_lines,
            canceled: was_canceled,
            timed_out: was_timed_out,
            duration_ms,
            is_truncated,
        })
    }

    pub async fn run_bounded(
        program: &str,
        args: &[String],
        cwd: Option<PathBuf>,
        timeout_duration: Duration,
        cancel_flag: Arc<AtomicBool>,
        event_sender: Option<mpsc::Sender<ProcessOutputLine>>,
    ) -> Result<ProcessResult, ProcessError> {
        Self::run_turn_stream(
            Path::new(program),
            args,
            cwd.as_deref(),
            None,
            timeout_duration,
            cancel_flag,
            event_sender,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_process_runner_success() {
        let cancel = Arc::new(AtomicBool::new(false));
        #[cfg(target_os = "windows")]
        let (cmd, args) = (
            "cmd.exe",
            vec!["/C".to_string(), "echo hello-runner".to_string()],
        );
        #[cfg(not(target_os = "windows"))]
        let (cmd, args) = (
            "sh",
            vec!["-c".to_string(), "echo hello-runner".to_string()],
        );

        let result =
            ProcessRunner::run_bounded(cmd, &args, None, Duration::from_secs(5), cancel, None)
                .await
                .expect("run process");

        assert_eq!(result.exit_code, Some(0));
        assert!(!result.canceled);
        assert!(!result.timed_out);
        assert!(result
            .output_lines
            .iter()
            .any(|l| l.line.contains("hello-runner")));
    }

    #[tokio::test]
    async fn test_process_runner_cancellation() {
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel.clone();

        #[cfg(target_os = "windows")]
        let (cmd, args) = (
            "powershell.exe",
            vec![
                "-Command".to_string(),
                "Start-Sleep -Seconds 10".to_string(),
            ],
        );
        #[cfg(not(target_os = "windows"))]
        let (cmd, args) = ("sleep", vec!["10".to_string()]);

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            cancel_clone.store(true, Ordering::Relaxed);
        });

        let result =
            ProcessRunner::run_bounded(cmd, &args, None, Duration::from_secs(5), cancel, None)
                .await
                .expect("run process");

        assert!(result.canceled);
        assert!(result.duration_ms < 5000);
    }

    #[tokio::test]
    async fn test_process_runner_stderr_and_stdin_streaming() {
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, mut rx) = mpsc::channel(100);

        #[cfg(target_os = "windows")]
        let (cmd, args) = (
            "powershell.exe",
            vec![
                "-Command".to_string(),
                "$line = [Console]::In.ReadLine(); [Console]::Error.WriteLine(\"err: $line\"); Write-Output \"out: $line\"".to_string(),
            ],
        );
        #[cfg(not(target_os = "windows"))]
        let (cmd, args) = (
            "sh",
            vec![
                "-c".to_string(),
                "read line; echo \"err: $line\" >&2; echo \"out: $line\"".to_string(),
            ],
        );

        let result = ProcessRunner::run_turn_stream(
            Path::new(cmd),
            &args,
            None,
            Some("streaming-payload\n"),
            Duration::from_secs(5),
            cancel,
            Some(tx),
        )
        .await
        .expect("run stream process");

        assert_eq!(result.exit_code, Some(0));
        assert!(!result.canceled);
        assert!(!result.timed_out);

        let mut lines = Vec::new();
        while let Ok(line) = rx.try_recv() {
            lines.push(line);
        }

        assert!(lines
            .iter()
            .any(|l| l.kind == ProcessOutputKind::Stderr
                && l.line.contains("err: streaming-payload")));
        assert!(lines
            .iter()
            .any(|l| l.kind == ProcessOutputKind::Stdout
                && l.line.contains("out: streaming-payload")));
    }

    #[tokio::test]
    async fn test_process_runner_timeout() {
        let cancel = Arc::new(AtomicBool::new(false));
        #[cfg(target_os = "windows")]
        let (cmd, args) = (
            "powershell.exe",
            vec![
                "-Command".to_string(),
                "Start-Sleep -Seconds 10".to_string(),
            ],
        );
        #[cfg(not(target_os = "windows"))]
        let (cmd, args) = ("sleep", vec!["10".to_string()]);

        let result =
            ProcessRunner::run_bounded(cmd, &args, None, Duration::from_millis(300), cancel, None)
                .await
                .expect("run process");

        assert!(result.timed_out);
        assert!(!result.canceled);
        assert!(result.duration_ms >= 300);
    }
}

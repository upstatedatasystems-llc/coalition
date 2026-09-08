use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessOutputKind {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone)]
pub struct ProcessOutputLine {
    pub kind: ProcessOutputKind,
    pub line: String,
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone)]
pub struct ProcessResult {
    pub exit_code: Option<i32>,
    pub output_lines: Vec<ProcessOutputLine>,
    pub canceled: bool,
    pub timed_out: bool,
    pub duration_ms: u64,
}

pub struct ProcessRunner;

impl ProcessRunner {
    pub async fn run_bounded(
        program: &str,
        args: &[String],
        cwd: Option<PathBuf>,
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
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd.spawn()?;
        let stdout = child.stdout.take().ok_or_else(|| {
            ProcessError::Execution("Failed to open child stdout pipe".to_string())
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            ProcessError::Execution("Failed to open child stderr pipe".to_string())
        })?;

        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();

        let mut accumulated_lines: Vec<ProcessOutputLine> = Vec::new();
        let mut was_canceled = false;
        let mut was_timed_out = false;

        let poll_interval = Duration::from_millis(50);
        let mut exit_status = None;

        loop {
            // Check cancellation flag
            if cancel_flag.load(Ordering::Relaxed) {
                was_canceled = true;
                let _ = child.kill().await;
                break;
            }

            // Check overall timeout
            if start_time.elapsed() > timeout_duration {
                was_timed_out = true;
                let _ = child.kill().await;
                break;
            }

            tokio::select! {
                line_res = stdout_reader.next_line() => {
                    match line_res {
                        Ok(Some(line)) => {
                            let out = ProcessOutputLine {
                                kind: ProcessOutputKind::Stdout,
                                line,
                                timestamp_ms: chrono::Utc::now().timestamp_millis(),
                            };
                            if let Some(tx) = &event_sender {
                                let _ = tx.send(out.clone()).await;
                            }
                            accumulated_lines.push(out);
                        }
                        Ok(None) => {}
                        Err(e) => {
                            eprintln!("Error reading child stdout: {}", e);
                        }
                    }
                }
                err_res = stderr_reader.next_line() => {
                    match err_res {
                        Ok(Some(line)) => {
                            let out = ProcessOutputLine {
                                kind: ProcessOutputKind::Stderr,
                                line,
                                timestamp_ms: chrono::Utc::now().timestamp_millis(),
                            };
                            if let Some(tx) = &event_sender {
                                let _ = tx.send(out.clone()).await;
                            }
                            accumulated_lines.push(out);
                        }
                        Ok(None) => {}
                        Err(e) => {
                            eprintln!("Error reading child stderr: {}", e);
                        }
                    }
                }
                status_res = child.wait() => {
                    match status_res {
                        Ok(status) => {
                            exit_status = Some(status.code().unwrap_or(0));
                            // Drain remaining lines
                            while let Ok(Some(line)) = stdout_reader.next_line().await {
                                accumulated_lines.push(ProcessOutputLine {
                                    kind: ProcessOutputKind::Stdout,
                                    line,
                                    timestamp_ms: chrono::Utc::now().timestamp_millis(),
                                });
                            }
                            while let Ok(Some(line)) = stderr_reader.next_line().await {
                                accumulated_lines.push(ProcessOutputLine {
                                    kind: ProcessOutputKind::Stderr,
                                    line,
                                    timestamp_ms: chrono::Utc::now().timestamp_millis(),
                                });
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
        })
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
        assert!(result.duration_ms < 3000);
    }
}

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
pub const MAX_ACCUMULATED_BYTES: usize = 10 * 1024 * 1024; // 10 MB

/// Truncates `s` to at most `max_bytes` bytes, ensuring the cut occurs on a valid UTF-8 char boundary.
pub fn truncate_utf8_safe(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    match s
        .char_indices()
        .take_while(|(idx, ch)| *idx + ch.len_utf8() <= max_bytes)
        .last()
    {
        Some((idx, ch)) => &s[..idx + ch.len_utf8()],
        None => "",
    }
}

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

#[cfg(target_os = "windows")]
pub async fn request_process_graceful_stop(pid: u32) {
    // Windows taskkill without /F requests graceful termination of process and its tree
    let _ = Command::new("taskkill")
        .args(["/T", "/PID", &pid.to_string()])
        .output()
        .await;
}

#[cfg(not(target_os = "windows"))]
pub async fn terminate_process_tree(pid: u32) {
    unsafe {
        libc::kill(pid as i32, libc::SIGKILL);
    }
}

#[cfg(not(target_os = "windows"))]
pub async fn request_process_graceful_stop(pid: u32) {
    unsafe {
        libc::kill(pid as i32, libc::SIGTERM);
    }
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
        Self::run_turn_stream_advanced(
            program,
            args,
            cwd,
            stdin_payload,
            timeout_duration,
            cancel_flag,
            event_sender,
            None,
            Duration::from_millis(1500),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn run_turn_stream_advanced(
        program: &Path,
        args: &[String],
        cwd: Option<&Path>,
        stdin_payload: Option<&str>,
        timeout_duration: Duration,
        cancel_flag: Arc<AtomicBool>,
        event_sender: Option<mpsc::Sender<ProcessOutputLine>>,
        disk_log_path: Option<&Path>,
        grace_period: Duration,
    ) -> Result<ProcessResult, ProcessError> {
        let start_time = tokio::time::Instant::now();

        // If disk logging is requested, open file asynchronously
        let mut disk_file: Option<tokio::fs::File> = if let Some(path) = disk_log_path {
            if let Some(parent) = path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            tokio::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .append(true)
                .open(path)
                .await
                .ok()
        } else {
            None
        };

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
            // Check cancellation flag with two-phase graceful-first policy
            if cancel_flag.load(Ordering::Relaxed) {
                was_canceled = true;
                if let Some(pid) = child.id() {
                    request_process_graceful_stop(pid).await;
                    let wait_grace = tokio::time::timeout(grace_period, child.wait()).await;
                    if wait_grace.is_err() {
                        terminate_process_tree(pid).await;
                        let _ = child.kill().await;
                    }
                } else {
                    let _ = child.kill().await;
                }
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
                            if let Some(ref mut f) = disk_file {
                                let line_data = format!("[{}] [OUT] {}\n", out.timestamp_ms, out.line);
                                let _ = f.write_all(line_data.as_bytes()).await;
                                let _ = f.flush().await;
                            }
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
                            if let Some(ref mut f) = disk_file {
                                let line_data = format!("[{}] [ERR] {}\n", out.timestamp_ms, out.line);
                                let _ = f.write_all(line_data.as_bytes()).await;
                                let _ = f.flush().await;
                            }
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
                                if let Some(ref mut f) = disk_file {
                                    let line_data = format!("[{}] [OUT] {}\n", out.timestamp_ms, out.line);
                                    let _ = f.write_all(line_data.as_bytes()).await;
                                    let _ = f.flush().await;
                                }
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
                                if let Some(ref mut f) = disk_file {
                                    let line_data = format!("[{}] [ERR] {}\n", out.timestamp_ms, out.line);
                                    let _ = f.write_all(line_data.as_bytes()).await;
                                    let _ = f.flush().await;
                                }
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

        if let Some(ref mut f) = disk_file {
            let _ = f.flush().await;
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

    pub fn run_sync_bounded(
        program: &Path,
        args: &[&str],
        cwd: Option<&Path>,
        timeout_duration: Duration,
    ) -> Result<std::process::Output, ProcessError> {
        let mut cmd = std::process::Command::new(program);
        cmd.args(args);
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn()?;
        let pid = child.id();

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        // Incrementally read stdout with strict byte limit and drain remainder to avoid pipe deadlock
        let stdout_handle = std::thread::spawn(move || -> Vec<u8> {
            let mut buf = Vec::new();
            if let Some(mut stream) = stdout {
                use std::io::Read;
                let mut chunk = [0u8; 8192];
                loop {
                    match stream.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => {
                            if buf.len() < MAX_ACCUMULATED_BYTES {
                                let to_take = std::cmp::min(n, MAX_ACCUMULATED_BYTES - buf.len());
                                buf.extend_from_slice(&chunk[..to_take]);
                            }
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
            }
            buf
        });

        // Incrementally read stderr with strict byte limit and drain remainder to avoid pipe deadlock
        let stderr_handle = std::thread::spawn(move || -> Vec<u8> {
            let mut buf = Vec::new();
            if let Some(mut stream) = stderr {
                use std::io::Read;
                let mut chunk = [0u8; 8192];
                loop {
                    match stream.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => {
                            if buf.len() < MAX_ACCUMULATED_BYTES {
                                let to_take = std::cmp::min(n, MAX_ACCUMULATED_BYTES - buf.len());
                                buf.extend_from_slice(&chunk[..to_take]);
                            }
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
            }
            buf
        });

        // Wait on child process with timeout
        let (tx, rx) = std::sync::mpsc::channel();
        let wait_thread = std::thread::spawn(move || {
            let res = child.wait();
            let _ = tx.send(res);
        });

        let exit_status = match rx.recv_timeout(timeout_duration) {
            Ok(status_res) => {
                let _ = wait_thread.join();
                status_res?
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                #[cfg(target_os = "windows")]
                {
                    let _ = std::process::Command::new("taskkill")
                        .args(["/F", "/T", "/PID", &pid.to_string()])
                        .output();
                }
                #[cfg(not(target_os = "windows"))]
                {
                    unsafe {
                        libc::kill(pid as i32, libc::SIGKILL);
                    }
                }
                let _ = wait_thread.join();
                let _ = stdout_handle.join();
                let _ = stderr_handle.join();
                return Err(ProcessError::Timeout(timeout_duration));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                let _ = wait_thread.join();
                let _ = stdout_handle.join();
                let _ = stderr_handle.join();
                return Err(ProcessError::Execution(
                    "Process wait thread disconnected unexpectedly".to_string(),
                ));
            }
        };

        let stdout_data = stdout_handle.join().unwrap_or_default();
        let stderr_data = stderr_handle.join().unwrap_or_default();

        Ok(std::process::Output {
            status: exit_status,
            stdout: stdout_data,
            stderr: stderr_data,
        })
    }

    /// Safely opens an HTTP/HTTPS URL in the default desktop browser under the common process layer.
    pub fn open_url(url: &str) -> Result<(), ProcessError> {
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(ProcessError::Execution(
                "Only HTTP and HTTPS URLs are allowed".to_string(),
            ));
        }

        #[cfg(target_os = "windows")]
        {
            let mut cmd = std::process::Command::new("rundll32");
            cmd.args(["url.dll,FileProtocolHandler", url]);
            cmd.spawn()?;
        }
        #[cfg(target_os = "macos")]
        {
            let mut cmd = std::process::Command::new("open");
            cmd.arg(url);
            cmd.spawn()?;
        }
        #[cfg(target_os = "linux")]
        {
            let mut cmd = std::process::Command::new("xdg-open");
            cmd.arg(url);
            cmd.spawn()?;
        }

        Ok(())
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

    #[test]
    fn test_truncate_utf8_safe() {
        // Pure ASCII
        assert_eq!(truncate_utf8_safe("hello world", 5), "hello");
        assert_eq!(truncate_utf8_safe("hello", 10), "hello");
        assert_eq!(truncate_utf8_safe("hello", 0), "");

        // 2-byte Cyrillic chars (each letter is 2 bytes: 'д' = [0xD0, 0xB4])
        let cyrillic = "дада"; // 4 chars, 8 bytes
        assert_eq!(truncate_utf8_safe(cyrillic, 3), "д"); // 3 bytes cut falls inside second char -> 1 char returned (2 bytes)
        assert_eq!(truncate_utf8_safe(cyrillic, 4), "да"); // exactly 4 bytes -> 2 chars returned

        // 3-byte CJK chars (each char is 3 bytes: 'あ' = [0xE3, 0x81, 0x82])
        let japanese = "あいう"; // 3 chars, 9 bytes
        assert_eq!(truncate_utf8_safe(japanese, 5), "あ"); // 5 bytes cut falls inside second char -> 1 char returned (3 bytes)
        assert_eq!(truncate_utf8_safe(japanese, 6), "あい"); // exactly 6 bytes -> 2 chars returned

        // 4-byte emoji chars (each emoji is 4 bytes: '🚀' = [0xF0, 0x9F, 0x9a, 0x80])
        let emojis = "🚀🦀✨"; // 3 chars, 12 bytes
        assert_eq!(truncate_utf8_safe(emojis, 1), ""); // cut at byte 1 cannot fit 4-byte emoji
        assert_eq!(truncate_utf8_safe(emojis, 3), ""); // cut at byte 3 cannot fit 4-byte emoji
        assert_eq!(truncate_utf8_safe(emojis, 4), "🚀"); // exactly 4 bytes -> 1 emoji
        assert_eq!(truncate_utf8_safe(emojis, 7), "🚀"); // 7 bytes cut falls inside 2nd emoji -> 1 emoji
        assert_eq!(truncate_utf8_safe(emojis, 8), "🚀🦀"); // exactly 8 bytes -> 2 emojis
    }

    #[test]
    fn test_open_url_scheme_validation() {
        assert!(ProcessRunner::open_url("ftp://example.com").is_err());
        assert!(ProcessRunner::open_url("javascript:alert(1)").is_err());
        assert!(ProcessRunner::open_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn test_run_sync_bounded_execution() {
        #[cfg(target_os = "windows")]
        let (cmd, args) = ("cmd.exe", vec!["/C", "echo sync-bounded-ok"]);
        #[cfg(not(target_os = "windows"))]
        let (cmd, args) = ("sh", vec!["-c", "echo sync-bounded-ok"]);

        let out =
            ProcessRunner::run_sync_bounded(Path::new(cmd), &args, None, Duration::from_secs(5))
                .expect("run sync bounded");

        assert!(out.status.success());
        let stdout_str = String::from_utf8_lossy(&out.stdout);
        assert!(stdout_str.contains("sync-bounded-ok"));
        assert!(out.stdout.len() <= MAX_ACCUMULATED_BYTES);
    }

    #[tokio::test]
    async fn test_process_runner_disk_streaming() {
        let cancel = Arc::new(AtomicBool::new(false));
        let temp_dir = tempfile::tempdir().unwrap();
        let log_file = temp_dir.path().join("sub").join("output.log");

        #[cfg(target_os = "windows")]
        let (cmd, args) = (
            "cmd.exe",
            vec![
                "/C".to_string(),
                "echo disk-streaming-line-1& echo disk-streaming-line-2".to_string(),
            ],
        );
        #[cfg(not(target_os = "windows"))]
        let (cmd, args) = (
            "sh",
            vec![
                "-c".to_string(),
                "echo disk-streaming-line-1; echo disk-streaming-line-2".to_string(),
            ],
        );

        let result = ProcessRunner::run_turn_stream_advanced(
            Path::new(cmd),
            &args,
            None,
            None,
            Duration::from_secs(5),
            cancel,
            None,
            Some(&log_file),
            Duration::from_millis(500),
        )
        .await
        .expect("run turn stream advanced with disk log");

        assert_eq!(result.exit_code, Some(0));
        assert!(log_file.exists());
        let log_contents = std::fs::read_to_string(&log_file).expect("read disk log");
        assert!(log_contents.contains("disk-streaming-line-1"));
        assert!(log_contents.contains("disk-streaming-line-2"));
        assert!(log_contents.contains("[OUT]"));
    }

    #[tokio::test]
    async fn test_process_runner_graceful_cancellation() {
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
            tokio::time::sleep(Duration::from_millis(150)).await;
            cancel_clone.store(true, Ordering::Relaxed);
        });

        let result = ProcessRunner::run_turn_stream_advanced(
            Path::new(cmd),
            &args,
            None,
            None,
            Duration::from_secs(5),
            cancel,
            None,
            None,
            Duration::from_millis(300),
        )
        .await
        .expect("run process");

        assert!(result.canceled);
        assert!(result.duration_ms < 5000);
    }
}

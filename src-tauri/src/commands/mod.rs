use crate::core::builder::{
    AgyEvent, AntigravityCliAdapter, BuilderTurnRequest, BuilderTurnResponse, ModelInfo,
};
use crate::core::git::{GitAdapter, GitRepoInfo};
use crate::db::{DbManager, ProofResult};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::{mpsc, Mutex};

pub struct AppState {
    pub db: Mutex<DbManager>,
    pub git: Mutex<Option<GitAdapter>>,
    pub agy: Mutex<Option<AntigravityCliAdapter>>,
    pub cancel_flag: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemDiagnosticInfo {
    pub current_dir: String,
    pub git: Option<GitRepoInfo>,
    pub agy_detected: bool,
    pub agy_path: Option<String>,
    pub agy_version: Option<String>,
    pub agy_models: Vec<ModelInfo>,
    pub agy_error: Option<String>,
}

#[tauri::command]
pub async fn get_system_diagnostics(
    state: State<'_, AppState>,
) -> Result<SystemDiagnosticInfo, String> {
    let current_dir = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .to_string_lossy()
        .to_string();

    let mut git_lock = state.git.lock().await;
    if git_lock.is_none() {
        if let Ok(adapter) = GitAdapter::new() {
            *git_lock = Some(adapter);
        }
    }

    let git_info = if let Some(ref git) = *git_lock {
        git.inspect_repo(&current_dir).ok()
    } else {
        None
    };

    let mut agy_lock = state.agy.lock().await;
    if agy_lock.is_none() {
        if let Ok(adapter) = AntigravityCliAdapter::discover() {
            *agy_lock = Some(adapter);
        }
    }

    let mut agy_detected = false;
    let mut agy_path = None;
    let mut agy_version = None;
    let mut agy_models = Vec::new();
    let mut agy_error = None;

    if let Some(ref agy) = *agy_lock {
        agy_detected = true;
        agy_path = Some(agy.binary_path().to_string_lossy().to_string());
        match agy.get_version() {
            Ok(v) => agy_version = Some(v),
            Err(e) => agy_error = Some(e.to_string()),
        }
        match agy.list_models() {
            Ok(m) => agy_models = m,
            Err(e) => {
                if agy_error.is_none() {
                    agy_error = Some(e.to_string());
                }
            }
        }
    } else {
        agy_error = Some(
            "Antigravity CLI (agy) not found on PATH or standard local app directories".to_string(),
        );
    }

    Ok(SystemDiagnosticInfo {
        current_dir,
        git: git_info,
        agy_detected,
        agy_path,
        agy_version,
        agy_models,
        agy_error,
    })
}

#[tauri::command]
pub async fn run_sqlite_proof(state: State<'_, AppState>) -> Result<ProofResult, String> {
    let mut db = state.db.lock().await;
    db.run_proof().map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartBuilderTurnPayload {
    pub prompt: String,
    pub conversation_id: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub icarus_mode: bool,
    pub use_fake_agy: bool,
}

#[tauri::command]
pub async fn start_builder_turn(
    app: AppHandle,
    state: State<'_, AppState>,
    payload: StartBuilderTurnPayload,
) -> Result<BuilderTurnResponse, String> {
    state.cancel_flag.store(false, Ordering::Relaxed);

    let adapter = if payload.use_fake_agy {
        let fake_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("tests")
            .join("fake-commands")
            .join(if cfg!(windows) {
                "fake-agy.cmd"
            } else {
                "fake-agy"
            });

        if !fake_path.exists() {
            return Err(format!("fake-agy executable not found at {:?}", fake_path));
        }
        AntigravityCliAdapter::with_path(fake_path)
    } else {
        let lock = state.agy.lock().await;
        if let Some(ref a) = *lock {
            AntigravityCliAdapter::with_path(a.binary_path())
        } else {
            AntigravityCliAdapter::discover().map_err(|e| e.to_string())?
        }
    };

    let (tx, mut rx) = mpsc::channel::<AgyEvent>(100);

    let app_clone = app.clone();
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            let _ = app_clone.emit("agy-stream-event", &event);
        }
    });

    let current_dir = std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().to_string());

    let request = BuilderTurnRequest {
        prompt: payload.prompt,
        conversation_id: payload.conversation_id,
        model: payload.model,
        effort: payload.effort,
        icarus_mode: payload.icarus_mode,
        working_dir: current_dir,
    };

    let result = adapter
        .run_turn(request, state.cancel_flag.clone(), Some(tx))
        .await
        .map_err(|e| e.to_string())?;

    Ok(result)
}

#[tauri::command]
pub async fn cancel_builder_turn(state: State<'_, AppState>) -> Result<(), String> {
    state.cancel_flag.store(true, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
pub async fn desktop_clipboard_write(text: String) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    clipboard.set_text(text).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn desktop_clipboard_read() -> Result<String, String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    clipboard.get_text().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn desktop_open_url(url: String) -> Result<(), String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("Only HTTP and HTTPS URLs are allowed".to_string());
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", &url])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

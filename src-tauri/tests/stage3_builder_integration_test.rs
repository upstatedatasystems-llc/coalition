use coalition_lib::core::artifacts::CANONICAL_ARCHITECTURE_ARTIFACTS;
use coalition_lib::core::builder::{
    ActiveBuilderRegistry, AgyUsage, AntigravityCliAdapter, BuilderService, BuilderSessionRecord,
    BuilderTurnRequest,
};
use coalition_lib::core::freeze::FreezeService;
use coalition_lib::core::git::GitAdapter;
use coalition_lib::core::projects::ProjectService;
use coalition_lib::core::workflow::{apply_workflow_action, WorkflowAction};
use coalition_lib::db::DbManager;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tempfile::tempdir;

fn setup_frozen_test_project() -> (tempfile::TempDir, tempfile::TempDir, String, PathBuf) {
    let repo_temp = tempdir().unwrap();
    let repo_path = repo_temp.path();

    Command::new("git")
        .args(["init"])
        .current_dir(repo_path)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "Stage 3 Tester"])
        .current_dir(repo_path)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "stage3@test.local"])
        .current_dir(repo_path)
        .output()
        .unwrap();

    let fixture_file = repo_path.join("README.md");
    fs::write(&fixture_file, "# Test Project\n").unwrap();
    Command::new("git")
        .args(["add", "README.md"])
        .current_dir(repo_path)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "initial commit"])
        .current_dir(repo_path)
        .output()
        .unwrap();

    let db_temp = tempdir().unwrap();
    let db_path = db_temp.path().join("stage3_test.db");

    let git = GitAdapter::new().unwrap();
    let mut db = DbManager::open(&db_path).unwrap();
    db.run_migrations().unwrap();

    // Register project
    let details = ProjectService::register_or_open_project(
        db.connection_mut(),
        &git,
        repo_path.to_str().unwrap(),
    )
    .unwrap();
    let project_id = details.project.project_id;

    // Transition to ARCHITECTING
    apply_workflow_action(
        db.connection_mut(),
        &project_id,
        WorkflowAction::StartArchitecting,
        "HUMAN",
    )
    .unwrap();

    // Populate canonical architecture artifacts so project is Ready To Freeze
    let coalition_dir = repo_path.join(".coalition");
    for &art_path in CANONICAL_ARCHITECTURE_ARTIFACTS {
        let full_path = coalition_dir.join(art_path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        if art_path.ends_with(".yaml") || art_path.ends_with(".yml") {
            fs::write(
                &full_path,
                "version: 1\nreadiness: ready\nsummary: Test architecture\n",
            )
            .unwrap();
        } else {
            fs::write(
                &full_path,
                "# Architecture Spec\nSubstantive architecture content for Stage 3 validation testing.\n",
            )
            .unwrap();
        }
    }

    // Mark Ready to Freeze
    apply_workflow_action(
        db.connection_mut(),
        &project_id,
        WorkflowAction::MarkReadyToFreeze,
        "HUMAN",
    )
    .unwrap();

    // Freeze architecture contract
    let preview =
        FreezeService::prepare_freeze_preview(repo_path, &project_id, &git, db.connection())
            .unwrap();

    let freeze_res = FreezeService::confirm_freeze(
        repo_path,
        &project_id,
        &preview.preview_id,
        &git,
        db.connection_mut(),
    )
    .unwrap();

    assert_eq!(freeze_res.architecture_version, "1.0");

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

    (repo_temp, db_temp, project_id, fake_agy_path)
}

#[tokio::test]
async fn test_stage3_builder_end_to_end_journey() {
    let (repo_temp, db_temp, project_id, fake_agy_path) = setup_frozen_test_project();
    let repo_path = repo_temp.path();
    let db_path = db_temp.path().join("stage3_test.db");

    let db = DbManager::open(&db_path).unwrap();

    // 1. Verify frozen Builder packet exists and is authoritative contract input
    let packet = FreezeService::get_builder_packet(repo_path, None).unwrap();
    assert_eq!(packet.metadata.architecture_version, "1.0");
    let epoch_id = packet.metadata.builder_epoch_id.clone();
    assert!(!epoch_id.is_empty());
    assert!(!packet.prompt.is_empty());

    // 2. Drift Detection fails closed
    let test_art = repo_path
        .join(".coalition")
        .join("design")
        .join("architecture.md");
    let orig_content = fs::read_to_string(&test_art).unwrap();
    fs::write(&test_art, "unauthorized modification").unwrap();

    let drift = FreezeService::check_contract_drift(repo_path, &project_id).unwrap();
    assert!(drift.has_drift, "Contract drift must be detected");

    // Restore drift to resume lawful testing
    fs::write(&test_art, orig_content).unwrap();
    let drift_cleared = FreezeService::check_contract_drift(repo_path, &project_id).unwrap();
    assert!(!drift_cleared.has_drift, "Drift cleared after restoration");

    // 3. Antigravity CLI discovery & model enumeration
    let adapter = AntigravityCliAdapter::with_path(&fake_agy_path);
    let models = adapter.list_models().unwrap();
    assert!(!models.is_empty());
    assert!(models.iter().any(|m| m.id == "gemini-3.8-flash-high"));

    // 4. Initial Turn: run against frozen contract
    let cancel = Arc::new(AtomicBool::new(false));
    let turn1_req = BuilderTurnRequest {
        prompt: packet.prompt.clone(),
        conversation_id: None,
        model: Some("gemini-3.8-flash-high".to_string()),
        effort: Some("medium".to_string()),
        icarus_mode: false,
        working_dir: Some(repo_path.to_string_lossy().to_string()),
    };

    let session_id_1 = "session-turn-1".to_string();
    let session_rec_1 = BuilderSessionRecord {
        session_id: session_id_1.clone(),
        project_id: project_id.clone(),
        epoch_id: epoch_id.clone(),
        conversation_id: None,
        model: "gemini-3.8-flash-high".to_string(),
        effort: Some("medium".to_string()),
        icarus_mode: false,
        status: "RUNNING".to_string(),
        prompt: turn1_req.prompt.clone(),
        response_text: None,
        error_message: None,
        started_at: chrono::Utc::now().to_rfc3339(),
        completed_at: None,
        duration_ms: 0,
        usage: AgyUsage::default(),
    };
    db.insert_builder_session(&session_rec_1).unwrap();

    let resp1 = adapter
        .run_turn(turn1_req, cancel.clone(), None)
        .await
        .unwrap();
    assert_eq!(resp1.status, "SUCCESS");
    assert!(resp1.conversation_id.is_some());
    let conv_id = resp1.conversation_id.clone().unwrap();
    assert_eq!(resp1.cumulative_usage.total_tokens, 550);

    db.update_builder_session_status(
        &session_id_1,
        &resp1.status,
        Some(&resp1.text_response),
        None,
        Some(&chrono::Utc::now().to_rfc3339()),
        150,
        &resp1.cumulative_usage,
        Some(&conv_id),
    )
    .unwrap();

    let saved_session1 = db.get_builder_session(&session_id_1).unwrap().unwrap();
    assert_eq!(saved_session1.status, "SUCCESS");
    assert_eq!(saved_session1.usage.total_tokens, 550);
    assert_eq!(saved_session1.conversation_id, Some(conv_id.clone()));

    // 5. Conversation Resume & Model Switching mid-conversation
    let turn2_req = BuilderTurnRequest {
        prompt: "Second turn follow-up guidance".to_string(),
        conversation_id: Some(conv_id.clone()),
        model: Some("gemini-3.7-flash-medium".to_string()), // Safe model switch
        effort: Some("high".to_string()),
        icarus_mode: false,
        working_dir: Some(repo_path.to_string_lossy().to_string()),
    };

    let session_id_2 = "session-turn-2".to_string();
    let session_rec_2 = BuilderSessionRecord {
        session_id: session_id_2.clone(),
        project_id: project_id.clone(),
        epoch_id: epoch_id.clone(),
        conversation_id: Some(conv_id.clone()),
        model: "gemini-3.7-flash-medium".to_string(),
        effort: Some("high".to_string()),
        icarus_mode: false,
        status: "RUNNING".to_string(),
        prompt: turn2_req.prompt.clone(),
        response_text: None,
        error_message: None,
        started_at: chrono::Utc::now().to_rfc3339(),
        completed_at: None,
        duration_ms: 0,
        usage: AgyUsage::default(),
    };
    db.insert_builder_session(&session_rec_2).unwrap();

    let resp2 = adapter
        .run_turn(turn2_req, cancel.clone(), None)
        .await
        .unwrap();
    assert_eq!(resp2.status, "SUCCESS");
    assert_eq!(resp2.conversation_id, Some(conv_id.clone()));
    // Tokens incremented across turns
    assert_eq!(resp2.cumulative_usage.total_tokens, 1100);

    // 6. Unavailable Pinned Model Fails Visibly
    let unavail_req = BuilderTurnRequest {
        prompt: "test unavailable".to_string(),
        conversation_id: Some(conv_id.clone()),
        model: Some("unavailable-pinned-model".to_string()),
        effort: None,
        icarus_mode: false,
        working_dir: Some(repo_path.to_string_lossy().to_string()),
    };
    let unavail_res = adapter.run_turn(unavail_req, cancel.clone(), None).await;
    assert!(unavail_res.is_err(), "Unavailable pinned model must fail");
    let err_msg = unavail_res.unwrap_err().to_string();
    assert!(err_msg.contains("Model unavailable"));

    // 7. Restart Reconciliation
    let orphaned_session = BuilderSessionRecord {
        session_id: "orphaned-session-123".to_string(),
        project_id: project_id.clone(),
        epoch_id: epoch_id.clone(),
        conversation_id: Some(conv_id.clone()),
        model: "gemini-3.8-flash-high".to_string(),
        effort: None,
        icarus_mode: false,
        status: "RUNNING".to_string(),
        prompt: "interrupted prompt".to_string(),
        response_text: None,
        error_message: None,
        started_at: chrono::Utc::now().to_rfc3339(),
        completed_at: None,
        duration_ms: 0,
        usage: AgyUsage::default(),
    };
    db.insert_builder_session(&orphaned_session).unwrap();

    // Reconcile
    let reconciled_count = db.reconcile_orphaned_sessions().unwrap();
    assert!(reconciled_count >= 1);
    let reconciled_s = db
        .get_builder_session("orphaned-session-123")
        .unwrap()
        .unwrap();
    assert_eq!(reconciled_s.status, "INTERRUPTED");
    assert!(reconciled_s.completed_at.is_some());

    // 8. Icarus Mode Toggle & Persistence
    let icarus_initial = db.get_icarus_state(&project_id).unwrap();
    assert!(!icarus_initial.enabled);

    db.set_icarus_state(&project_id, true, Some("HUMAN"))
        .unwrap();
    let icarus_on = db.get_icarus_state(&project_id).unwrap();
    assert!(icarus_on.enabled);
    assert_eq!(icarus_on.enabled_by.as_deref(), Some("HUMAN"));

    // Run with Icarus mode enabled
    let icarus_turn_req = BuilderTurnRequest {
        prompt: "Run in Icarus mode".to_string(),
        conversation_id: Some(conv_id.clone()),
        model: Some("gemini-3.8-flash-high".to_string()),
        effort: None,
        icarus_mode: true,
        working_dir: Some(repo_path.to_string_lossy().to_string()),
    };
    let icarus_resp = adapter
        .run_turn(icarus_turn_req, cancel.clone(), None)
        .await
        .unwrap();
    assert_eq!(icarus_resp.status, "SUCCESS");

    // Disable Icarus mode
    db.set_icarus_state(&project_id, false, Some("HUMAN"))
        .unwrap();
    let icarus_off = db.get_icarus_state(&project_id).unwrap();
    assert!(!icarus_off.enabled);

    // 9. ChatGPT Usage Telemetry & Reset Controls
    db.record_chatgpt_usage(
        &project_id,
        Some("pkt-test-1"),
        "OUTBOUND_PACKET",
        8000,
        2000,
        1,
        4.0,
    )
    .unwrap();
    db.record_chatgpt_usage(
        &project_id,
        Some("pkt-test-1"),
        "INBOUND_IMPORT",
        4000,
        1000,
        1,
        4.0,
    )
    .unwrap();

    let usage_summary = db.get_chatgpt_usage_summary(&project_id).unwrap();
    assert_eq!(usage_summary.rolling_5h_tokens, 3000);
    assert_eq!(usage_summary.rolling_7d_tokens, 3000);
    assert_eq!(usage_summary.total_tokens, 3000);
    assert_eq!(usage_summary.total_packets_sent, 1);
    assert_eq!(usage_summary.total_imports_received, 1);
    assert!(usage_summary
        .disclaimer
        .contains("Estimated relay throughput"));

    db.reset_chatgpt_usage(&project_id).unwrap();
    let usage_reset = db.get_chatgpt_usage_summary(&project_id).unwrap();
    assert_eq!(usage_reset.rolling_5h_tokens, 0);
    assert_eq!(usage_reset.total_tokens, 0);
    assert!(usage_reset.last_calibrated_at.is_some());
}

#[tokio::test]
async fn test_stage3_drift_and_corruption_rejection_no_state_mutation() {
    let (repo_temp, db_temp, project_id, _fake_agy) = setup_frozen_test_project();
    let repo_path = repo_temp.path();
    let db_path = db_temp.path().join("stage3_test.db");
    let mut db = DbManager::open(&db_path).unwrap();

    // Verify initial workflow state is FROZEN, revision = 3
    let wf_before = db
        .connection()
        .query_row(
            "SELECT state, revision FROM workflow_state WHERE project_id = ?1",
            rusqlite::params![project_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
        )
        .unwrap();
    assert_eq!(wf_before.0, "FROZEN");

    // Case 1: Contract drift -> preflight must reject AND leave workflow state strictly FROZEN
    let test_art = repo_path
        .join(".coalition")
        .join("design")
        .join("architecture.md");
    let orig_content = fs::read_to_string(&test_art).unwrap();
    fs::write(&test_art, "drift modification").unwrap();

    let preflight_err = coalition_lib::commands::validate_builder_preflight(&mut db, &project_id);
    assert!(preflight_err.is_err());
    assert_eq!(
        preflight_err.unwrap_err().code,
        "FROZEN_CONTRACT_DRIFT_DETECTED"
    );

    // Verify workflow state was NOT mutated
    let wf_after_drift = db
        .connection()
        .query_row(
            "SELECT state, revision FROM workflow_state WHERE project_id = ?1",
            rusqlite::params![project_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
        )
        .unwrap();
    assert_eq!(wf_after_drift.0, "FROZEN");
    assert_eq!(wf_after_drift.1, wf_before.1);

    // Restore drift
    fs::write(&test_art, &orig_content).unwrap();

    // Case 2: Snapshot manifest corruption -> preflight must reject AND leave workflow state strictly FROZEN
    let manifest_path = repo_path
        .join(".coalition")
        .join("architecture-versions")
        .join("v1.0")
        .join("contract-manifest.yaml");
    fs::write(&manifest_path, "corrupt yaml: {[").unwrap();

    let corrupt_err = coalition_lib::commands::validate_builder_preflight(&mut db, &project_id);
    assert!(corrupt_err.is_err());
    assert_eq!(corrupt_err.unwrap_err().code, "FROZEN_SNAPSHOT_CORRUPT");

    // Verify workflow state was NOT mutated
    let wf_after_corrupt = db
        .connection()
        .query_row(
            "SELECT state, revision FROM workflow_state WHERE project_id = ?1",
            rusqlite::params![project_id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
        )
        .unwrap();
    assert_eq!(wf_after_corrupt.0, "FROZEN");
    assert_eq!(wf_after_corrupt.1, wf_before.1);
}

#[tokio::test]
async fn test_stage3_active_builder_registry_concurrency() {
    let mut registry = coalition_lib::core::builder::ActiveBuilderRegistry::new();

    // 1. Register session 1
    let cancel1 = registry
        .register(
            "proj-alpha",
            "session-1",
            "epoch-1",
            None,
            "gemini-3.8-flash-high",
            Some("medium".to_string()),
            false,
        )
        .expect("registration should succeed");
    assert!(!cancel1.load(std::sync::atomic::Ordering::SeqCst));
    assert!(registry.is_active("proj-alpha"));

    // 2. Registering concurrent session for same project must be rejected
    let err = registry
        .register(
            "proj-alpha",
            "session-2",
            "epoch-1",
            None,
            "gemini-3.8-flash-high",
            Some("medium".to_string()),
            false,
        )
        .unwrap_err();
    assert!(err.to_string().contains("Concurrent build forbidden"));

    // 3. Project cancel sets session cancel flag
    registry.cancel_project("proj-alpha").unwrap();
    assert!(cancel1.load(std::sync::atomic::Ordering::SeqCst));

    // 4. Unregister clears active status and allows new registration
    registry.unregister("proj-alpha", "session-1");
    assert!(!registry.is_active("proj-alpha"));

    let cancel2 = registry
        .register(
            "proj-alpha",
            "session-2",
            "epoch-1",
            None,
            "gemini-3.8-flash-high",
            Some("high".to_string()),
            true,
        )
        .expect("subsequent registration should succeed");
    assert!(!cancel2.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn test_stage3_chatgpt_usage_estimator_calibration_and_capacity() {
    let (_repo_temp, db_temp, project_id, _fake_agy) = setup_frozen_test_project();
    let db_path = db_temp.path().join("stage3_test.db");
    let db = DbManager::open(&db_path).unwrap();

    // Record some usage: 8,000 chars -> initially estimated as 2,000 tokens (4.0 chars/token)
    db.record_chatgpt_usage(
        &project_id,
        Some("pkt-1"),
        "OUTBOUND_PACKET",
        8000,
        2000,
        1,
        4.0,
    )
    .unwrap();

    let initial_summary = db.get_chatgpt_usage_summary(&project_id).unwrap();
    assert_eq!(initial_summary.chars_per_token, 4.0);
    assert_eq!(initial_summary.estimator_version, 1);
    assert_eq!(initial_summary.sample_count, 0);
    // Grounded capacity indicators: hardcoded 80k/500k removed, capacity pct is None
    assert!(initial_summary.estimated_5h_capacity_pct.is_none());
    assert!(initial_summary.estimated_weekly_capacity_pct.is_none());

    // Calibrate with observed sample: 1,000 tokens for 3,000 chars (observed: 3.0 chars/token)
    let calibrated = db
        .calibrate_chatgpt_estimator(&project_id, 1000, 3000)
        .unwrap();
    assert_eq!(calibrated.version, 2);
    assert_eq!(calibrated.sample_count, 1);
    assert_eq!(calibrated.chars_per_token, 3.0);

    let updated_summary = db.get_chatgpt_usage_summary(&project_id).unwrap();
    assert_eq!(updated_summary.chars_per_token, 3.0);
    assert_eq!(updated_summary.estimator_version, 2);
    assert_eq!(updated_summary.sample_count, 1);
    assert!(updated_summary.last_calibrated_at.is_some());
}

#[tokio::test]
async fn test_stage3_governed_builder_service_workflow() {
    let (_repo_temp, db_temp, project_id, fake_agy) = setup_frozen_test_project();
    let db_path = db_temp.path().join("stage3_test.db");
    let db = Arc::new(tokio::sync::Mutex::new(DbManager::open(&db_path).unwrap()));
    let registry = Arc::new(tokio::sync::Mutex::new(ActiveBuilderRegistry::new()));
    let adapter = AntigravityCliAdapter::with_path(fake_agy);

    // 1. Initial workflow state is FROZEN
    {
        let d = db.lock().await;
        let st: String = d
            .connection()
            .query_row(
                "SELECT state FROM workflow_state WHERE project_id = ?1",
                rusqlite::params![project_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(st, "FROZEN");
    }

    // 2. Start governed turn
    let resp = BuilderService::start_governed_turn(
        db.clone(),
        registry.clone(),
        None,
        &project_id,
        Some("gemini-3.8-flash-high".to_string()),
        Some("low".to_string()),
        Some(adapter.clone()),
    )
    .await
    .expect("governed turn should succeed");

    assert_eq!(resp.status, "SUCCESS");

    // 3. Workflow transitioned FROZEN -> BUILDING
    {
        let d = db.lock().await;
        let st: String = d
            .connection()
            .query_row(
                "SELECT state FROM workflow_state WHERE project_id = ?1",
                rusqlite::params![project_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(st, "BUILDING");

        // Session was persisted and status is SUCCESS
        let session = d.get_latest_builder_session(&project_id).unwrap().unwrap();
        assert_eq!(session.status, "SUCCESS");
        assert_eq!(session.model, "gemini-3.8-flash-high");
        assert!(!session.icarus_mode);
        assert!(session.response_text.is_some());

        // Event persistence drain: verify events were drained and inserted into SQLite
        let events = d.list_builder_events(&session.session_id, None).unwrap();
        assert!(!events.is_empty(), "Events should be persisted and drained");
    }

    // 4. Epoch conversation reuse: start a 2nd turn in the same epoch
    let resp2 = BuilderService::start_governed_turn(
        db.clone(),
        registry.clone(),
        None,
        &project_id,
        Some("gemini-3.8-flash-high".to_string()),
        Some("low".to_string()),
        Some(adapter.clone()),
    )
    .await
    .expect("second governed turn should succeed");

    assert_eq!(resp2.status, "SUCCESS");
    {
        let d = db.lock().await;
        let sessions = d.list_builder_sessions(&project_id, 10).unwrap();
        assert_eq!(sessions.len(), 2);
        // Epoch IDs match
        assert_eq!(sessions[0].epoch_id, sessions[1].epoch_id);
    }
}

#[tokio::test]
async fn test_stage3_governed_builder_service_drift_fail_closed() {
    let (repo_temp, db_temp, project_id, fake_agy) = setup_frozen_test_project();
    let db_path = db_temp.path().join("stage3_test.db");
    let db = Arc::new(tokio::sync::Mutex::new(DbManager::open(&db_path).unwrap()));
    let registry = Arc::new(tokio::sync::Mutex::new(ActiveBuilderRegistry::new()));
    let adapter = AntigravityCliAdapter::with_path(fake_agy);

    // Corrupt an artifact to introduce contract drift
    let vision_path = repo_temp
        .path()
        .join(".coalition")
        .join("design")
        .join("product-vision.md");
    fs::write(
        &vision_path,
        "# Corrupted Vision\nDrifted from frozen snapshot!\n",
    )
    .unwrap();

    // Governed turn must FAIL before mutating workflow state
    let err = BuilderService::start_governed_turn(
        db.clone(),
        registry.clone(),
        None,
        &project_id,
        Some("gemini-3.8-flash-high".to_string()),
        None,
        Some(adapter),
    )
    .await;

    assert!(err.is_err(), "Must fail on contract drift");
    let d = db.lock().await;
    let st: String = d
        .connection()
        .query_row(
            "SELECT state FROM workflow_state WHERE project_id = ?1",
            rusqlite::params![project_id],
            |r| r.get(0),
        )
        .unwrap();
    // Workflow MUST still be FROZEN (no mutation on preflight failure!)
    assert_eq!(st, "FROZEN");
}

#[tokio::test]
async fn test_stage3_governed_builder_service_single_active_run_concurrency() {
    let (_repo_temp, db_temp, project_id, fake_agy) = setup_frozen_test_project();
    let db_path = db_temp.path().join("stage3_test.db");
    let db = Arc::new(tokio::sync::Mutex::new(DbManager::open(&db_path).unwrap()));
    let registry = Arc::new(tokio::sync::Mutex::new(ActiveBuilderRegistry::new()));
    let adapter = AntigravityCliAdapter::with_path(fake_agy);

    // Manually register an active session in registry
    {
        let mut reg = registry.lock().await;
        reg.register(
            &project_id,
            "sess-active-1",
            "epoch-1",
            None,
            "gemini-3.8-flash-high",
            None,
            false,
        )
        .unwrap();
    }

    // Attempt to start a governed turn -> must reject with concurrency error
    let err = BuilderService::start_governed_turn(
        db.clone(),
        registry.clone(),
        None,
        &project_id,
        Some("gemini-3.8-flash-high".to_string()),
        None,
        Some(adapter),
    )
    .await;

    assert!(err.is_err());
    let err_msg = err.err().unwrap().to_string();
    assert!(err_msg.contains("Concurrent build forbidden"));
}

#[tokio::test]
async fn test_stage3_governed_builder_service_cancellation_isolation() {
    let (_repo_temp1, db_temp1, project_id1, _) = setup_frozen_test_project();
    let (_repo_temp2, _db_temp2, project_id2, _) = setup_frozen_test_project();
    let db_path = db_temp1.path().join("stage3_test.db");
    let _db = Arc::new(tokio::sync::Mutex::new(DbManager::open(&db_path).unwrap()));
    let registry = Arc::new(tokio::sync::Mutex::new(ActiveBuilderRegistry::new()));

    // Register active sessions for both projects
    let flag1 = {
        let mut reg = registry.lock().await;
        let f1 = reg
            .register(
                &project_id1,
                "sess-p1",
                "ep-1",
                None,
                "model-1",
                None,
                false,
            )
            .unwrap();
        let _f2 = reg
            .register(
                &project_id2,
                "sess-p2",
                "ep-2",
                None,
                "model-2",
                None,
                false,
            )
            .unwrap();
        f1
    };

    // Cancel project 1
    let canceled_sid = BuilderService::cancel_turn(registry.clone(), Some(&project_id1), None)
        .await
        .expect("cancellation should succeed");
    assert_eq!(canceled_sid, "sess-p1");

    // Project 1 flag is canceled
    assert!(flag1.load(std::sync::atomic::Ordering::SeqCst));

    // Project 2 is still active and uncanceled
    {
        let reg = registry.lock().await;
        let active2 = reg.get_active_execution(&project_id2);
        assert!(active2.is_some());
        assert!(!active2
            .unwrap()
            .cancel_flag
            .load(std::sync::atomic::Ordering::SeqCst));
    }
}

#[tokio::test]
async fn test_stage3_chatgpt_usage_calibrated_provenance_persisted() {
    let (_repo_temp, db_temp, project_id, _) = setup_frozen_test_project();
    let db_path = db_temp.path().join("stage3_test.db");
    let db = DbManager::open(&db_path).unwrap();

    // 1. Initial state has default version 1, 4.0
    let initial_summary = db.get_chatgpt_usage_summary(&project_id).unwrap();
    assert_eq!(initial_summary.estimator_version, 1);
    assert_eq!(initial_summary.chars_per_token, 4.0);

    // Record default usage under uncalibrated settings
    let char_count_default = 4000;
    let tokens_default =
        (char_count_default as f64 / initial_summary.chars_per_token).round() as usize;
    db.record_chatgpt_usage(
        &project_id,
        Some("pkt-default-1"),
        "OUTBOUND_PACKET",
        char_count_default,
        tokens_default,
        initial_summary.estimator_version,
        initial_summary.chars_per_token,
    )
    .unwrap();

    // 2. Calibrate estimator to version 2 with ratio 3.5 (1000 tokens, 3500 chars)
    let updated_estimator = db
        .calibrate_chatgpt_estimator(&project_id, 1000, 3500)
        .unwrap();
    assert_eq!(updated_estimator.version, 2);
    assert!((updated_estimator.chars_per_token - 3.5).abs() < 1e-4);

    // Also verify get_chatgpt_usage_summary reflects the new version and ratio
    let summary_after_cal = db.get_chatgpt_usage_summary(&project_id).unwrap();
    assert_eq!(summary_after_cal.estimator_version, 2);
    assert!((summary_after_cal.chars_per_token - 3.5).abs() < 1e-4);

    // 3. Record outbound and inbound usage simulating copy/import commands under calibrated settings
    let char_count_out = 7000;
    let tokens_out = (char_count_out as f64 / updated_estimator.chars_per_token).round() as usize;
    db.record_chatgpt_usage(
        &project_id,
        Some("pkt-cal-1"),
        "OUTBOUND_PACKET",
        char_count_out,
        tokens_out,
        updated_estimator.version,
        updated_estimator.chars_per_token,
    )
    .unwrap();

    let char_count_in = 3500;
    let tokens_in = (char_count_in as f64 / updated_estimator.chars_per_token).round() as usize;
    db.record_chatgpt_usage(
        &project_id,
        Some("pkt-cal-1"),
        "INBOUND_IMPORT",
        char_count_in,
        tokens_in,
        updated_estimator.version,
        updated_estimator.chars_per_token,
    )
    .unwrap();

    // 4. Assert records in chatgpt_usage_records have exact expected columns, version, and ratio
    let mut stmt = db
        .connection()
        .prepare(
            "SELECT project_id, packet_id, direction, char_count, estimated_tokens, estimator_version, chars_per_token 
             FROM chatgpt_usage_records WHERE project_id = ?1 ORDER BY id ASC",
        )
        .unwrap();
    let rows = stmt
        .query_map(rusqlite::params![project_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, u64>(3)?,
                row.get::<_, u64>(4)?,
                row.get::<_, u32>(5)?,
                row.get::<_, f64>(6)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(rows.len(), 3);

    // Row 0: default record
    assert_eq!(rows[0].0, project_id);
    assert_eq!(rows[0].1.as_deref(), Some("pkt-default-1"));
    assert_eq!(rows[0].2, "OUTBOUND_PACKET");
    assert_eq!(rows[0].3, 4000);
    assert_eq!(rows[0].4, 1000);
    assert_eq!(rows[0].5, 1, "Default record must have estimator version 1");
    assert!(
        (rows[0].6 - 4.0).abs() < 1e-4,
        "Default record must have ratio 4.0"
    );

    // Row 1: calibrated outbound record
    assert_eq!(rows[1].0, project_id);
    assert_eq!(rows[1].1.as_deref(), Some("pkt-cal-1"));
    assert_eq!(rows[1].2, "OUTBOUND_PACKET");
    assert_eq!(rows[1].3, 7000);
    assert_eq!(rows[1].4, 2000);
    assert_eq!(
        rows[1].5, 2,
        "Outbound record must persist estimator version 2"
    );
    assert!(
        (rows[1].6 - 3.5).abs() < 1e-4,
        "Outbound record must persist ratio 3.5"
    );

    // Row 2: calibrated inbound record
    assert_eq!(rows[2].0, project_id);
    assert_eq!(rows[2].1.as_deref(), Some("pkt-cal-1"));
    assert_eq!(rows[2].2, "INBOUND_IMPORT");
    assert_eq!(rows[2].3, 3500);
    assert_eq!(rows[2].4, 1000);
    assert_eq!(
        rows[2].5, 2,
        "Inbound record must persist estimator version 2"
    );
    assert!(
        (rows[2].6 - 3.5).abs() < 1e-4,
        "Inbound record must persist ratio 3.5"
    );
}

#[tokio::test]
async fn test_stage3_service_cancellation_with_hanging_fake_agy() {
    let (repo_temp, db_temp, project_id, fake_agy) = setup_frozen_test_project();
    let repo_path = repo_temp.path();
    let db_path = db_temp.path().join("stage3_test.db");
    let db = Arc::new(tokio::sync::Mutex::new(DbManager::open(&db_path).unwrap()));
    let registry = Arc::new(tokio::sync::Mutex::new(ActiveBuilderRegistry::new()));
    let adapter = AntigravityCliAdapter::with_path(fake_agy);

    // Inject trigger_hang into the frozen builder-packet.json so fake-agy hangs on execution
    let packet_path = repo_path
        .join(".coalition")
        .join("contract")
        .join("builder-packet.json");
    if packet_path.exists() {
        let packet_content = fs::read_to_string(&packet_path).unwrap();
        let modified = packet_content.replace("\"goals\": [", "\"goals\": [\"trigger_hang\", ");
        fs::write(&packet_path, modified).unwrap();
    }

    // Spawn the governed turn in a background task
    let db_for_turn = db.clone();
    let registry_for_turn = registry.clone();
    let proj_for_turn = project_id.clone();
    let adapter_for_turn = adapter.clone();

    let turn_handle = tokio::spawn(async move {
        BuilderService::start_governed_turn(
            db_for_turn,
            registry_for_turn,
            None,
            &proj_for_turn,
            Some("gemini-3.8-flash-high".to_string()),
            None,
            Some(adapter_for_turn),
        )
        .await
    });

    // Wait until the turn is registered as running in the registry
    let mut session_id = None;
    for _ in 0..50 {
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        let reg = registry.lock().await;
        if let Some(active) = reg.get_active_execution(&project_id) {
            session_id = Some(active.session_id.clone());
            break;
        }
    }
    let sid = session_id.expect("Turn must register in ActiveBuilderRegistry");

    // Request cancellation authoritative via BuilderService::cancel_turn
    let canceled_sid = BuilderService::cancel_turn(registry.clone(), Some(&project_id), None)
        .await
        .expect("Cancellation request must succeed");
    assert_eq!(canceled_sid, sid);

    // Await the turn handle
    let turn_result = turn_handle.await.expect("Turn task must join cleanly");
    let resp = turn_result.expect("Turn response must return Ok with canceled status");

    // Assert cancellation invariants
    assert!(resp.was_canceled, "Response was_canceled must be true");
    assert_eq!(resp.status, "CANCELLED");

    // Assert database session is updated to CANCELLED
    {
        let db_lock = db.lock().await;
        let session = db_lock
            .get_builder_session(&sid)
            .unwrap()
            .expect("Session must exist");
        assert_eq!(
            session.status, "CANCELLED",
            "Authoritative DB session must be CANCELLED"
        );
    }

    // Assert registry is cleaned up
    {
        let reg = registry.lock().await;
        assert!(
            reg.get_active_execution(&project_id).is_none(),
            "Registry must be cleared after turn finishes"
        );
    }

    // Assert another project can register without interference
    let (_repo_temp2, _db_temp2, project_id2, _) = setup_frozen_test_project();
    {
        let mut reg = registry.lock().await;
        let f2 = reg.register(
            &project_id2,
            "sess-p2",
            "ep-2",
            None,
            "model-2",
            None,
            false,
        );
        assert!(
            f2.is_ok(),
            "Second project registration must succeed without interference"
        );
    }
}

#[tokio::test]
async fn test_stage3_registry_cleanup_on_injected_persistence_and_session_failures() {
    let (_repo_temp, db_temp, project_id, fake_agy) = setup_frozen_test_project();
    let db_path = db_temp.path().join("stage3_test.db");
    let db = Arc::new(tokio::sync::Mutex::new(DbManager::open(&db_path).unwrap()));
    let registry = Arc::new(tokio::sync::Mutex::new(ActiveBuilderRegistry::new()));
    let adapter = AntigravityCliAdapter::with_path(fake_agy);

    // 1. Injected persistence failure: Create a trigger that fails inserts into builder_events
    {
        let db_lock = db.lock().await;
        db_lock
            .connection()
            .execute_batch(
                "CREATE TRIGGER fail_events_trigger BEFORE INSERT ON builder_events
                 BEGIN
                     SELECT RAISE(FAIL, 'injected persistence failure');
                 END;",
            )
            .unwrap();
    }

    let turn_res = BuilderService::start_governed_turn(
        db.clone(),
        registry.clone(),
        None,
        &project_id,
        Some("gemini-3.8-flash-high".to_string()),
        None,
        Some(adapter.clone()),
    )
    .await;

    // Assert that the turn failed with Database error
    assert!(turn_res.is_err());
    let err_str = turn_res.err().unwrap().to_string();
    assert!(
        err_str.contains("injected persistence failure")
            || err_str.contains("Builder event persistence"),
        "Error must reflect persistence failure: {}",
        err_str
    );

    // Assert registry is completely cleaned up (NOT stuck active)
    {
        let reg = registry.lock().await;
        assert!(
            reg.get_active_execution(&project_id).is_none(),
            "ActiveBuilderRegistry must be cleaned up on persistence failure"
        );
    }

    // Assert session status is FAILED (NOT left RUNNING)
    {
        let db_lock = db.lock().await;
        let session = db_lock
            .get_latest_builder_session(&project_id)
            .unwrap()
            .expect("Session must have been created");
        assert_eq!(
            session.status, "FAILED",
            "Operational session must be marked FAILED on persistence failure"
        );
        assert!(session
            .error_message
            .as_ref()
            .map(|e| e.contains("persistence"))
            .unwrap_or(false));
    }

    // 2. Drop the trigger, now inject session insert failure
    {
        let db_lock = db.lock().await;
        db_lock
            .connection()
            .execute_batch(
                "DROP TRIGGER fail_events_trigger;
                 CREATE TRIGGER fail_sessions_trigger BEFORE INSERT ON builder_sessions
                 BEGIN
                     SELECT RAISE(FAIL, 'injected session insert failure');
                 END;",
            )
            .unwrap();
    }

    let turn_res2 = BuilderService::start_governed_turn(
        db.clone(),
        registry.clone(),
        None,
        &project_id,
        Some("gemini-3.8-flash-high".to_string()),
        None,
        Some(adapter.clone()),
    )
    .await;

    // Assert that the turn failed with Database error
    assert!(turn_res2.is_err());
    let err_str2 = turn_res2.err().unwrap().to_string();
    assert!(
        err_str2.contains("injected session insert failure")
            || err_str2.contains("Failed to insert builder session"),
        "Error must reflect session insertion failure: {}",
        err_str2
    );

    // Assert registry is completely cleaned up again
    {
        let reg = registry.lock().await;
        assert!(
            reg.get_active_execution(&project_id).is_none(),
            "ActiveBuilderRegistry must be cleaned up on session insertion failure"
        );
    }

    // 3. Drop trigger and verify a subsequent turn can run without CONCURRENT_BUILD_FORBIDDEN
    {
        let db_lock = db.lock().await;
        db_lock
            .connection()
            .execute_batch("DROP TRIGGER fail_sessions_trigger;")
            .unwrap();
    }

    let turn_res3 = BuilderService::start_governed_turn(
        db.clone(),
        registry.clone(),
        None,
        &project_id,
        Some("gemini-3.8-flash-high".to_string()),
        None,
        Some(adapter),
    )
    .await;

    assert!(turn_res3.is_ok(), "Subsequent turn must succeed cleanly");
    {
        let reg = registry.lock().await;
        assert!(
            reg.get_active_execution(&project_id).is_none(),
            "ActiveBuilderRegistry must be cleaned up after successful turn"
        );
    }
}

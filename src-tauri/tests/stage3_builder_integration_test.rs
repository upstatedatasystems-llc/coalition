use coalition_lib::core::artifacts::CANONICAL_ARCHITECTURE_ARTIFACTS;
use coalition_lib::core::builder::{
    AgyUsage, AntigravityCliAdapter, BuilderSessionRecord, BuilderTurnRequest,
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
    )
    .unwrap();
    db.record_chatgpt_usage(
        &project_id,
        Some("pkt-test-1"),
        "INBOUND_IMPORT",
        4000,
        1000,
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
    assert!(err
        .to_string()
        .contains("Concurrent build forbidden"));

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
    db.record_chatgpt_usage(&project_id, Some("pkt-1"), "OUTBOUND_PACKET", 8000, 2000)
        .unwrap();

    let initial_summary = db.get_chatgpt_usage_summary(&project_id).unwrap();
    assert_eq!(initial_summary.chars_per_token, 4.0);
    assert_eq!(initial_summary.estimator_version, 1);
    assert_eq!(initial_summary.sample_count, 0);
    assert!(initial_summary.estimated_5h_capacity_pct.is_some());
    // 2000 / 80000 = 2.5%
    assert!((initial_summary.estimated_5h_capacity_pct.unwrap() - 2.5).abs() < 0.1);

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

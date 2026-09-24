use coalition_lib::core::activity::ActivityManager;
use coalition_lib::core::artifacts::CANONICAL_ARCHITECTURE_ARTIFACTS;
use coalition_lib::core::freeze::FreezeService;
use coalition_lib::core::git::GitAdapter;
use coalition_lib::core::projects::ProjectService;
use coalition_lib::core::review::{
    get_latest_review_cycle, list_review_cycles_for_project, ReviewError, ReviewService,
    ReviewVerdict, ReviewerType,
};
use coalition_lib::core::validation::{
    ActiveValidationRegistry, ValidationService, ValidationTriggerSource,
};
use coalition_lib::core::workflow::{apply_workflow_action, WorkflowAction, WorkflowState};
use coalition_lib::db::DbManager;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use tempfile::tempdir;
use tokio::sync::Mutex;

fn setup_frozen_test_project() -> (tempfile::TempDir, tempfile::TempDir, String, PathBuf) {
    let repo_temp = tempdir().unwrap();
    let repo_path = repo_temp.path();

    Command::new("git")
        .args(["init"])
        .current_dir(repo_path)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "Stage 4 Tester"])
        .current_dir(repo_path)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "stage4@test.local"])
        .current_dir(repo_path)
        .output()
        .unwrap();

    let fixture_file = repo_path.join("README.md");
    fs::write(&fixture_file, "# Stage 4 Governed Loop Project\n").unwrap();
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
    let db_path = db_temp.path().join("stage4_test.db");

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
                "# Architecture Spec\nSubstantive architecture content for Stage 4 governed loop.\n",
            )
            .unwrap();
        }
    }

    // Add validation config: validation.yaml
    let validation_path = coalition_dir.join("implementation").join("validation.yaml");
    fs::create_dir_all(validation_path.parent().unwrap()).unwrap();

    #[cfg(windows)]
    let val_yaml = r#"schema_version: 1
enabled: true
commands:
  - id: check-sentinel
    name: "Check Clean Sentinel"
    command: "if exist target\\clean.txt (exit 0) else (exit 1)"
    timeout_seconds: 30
    required: true
policy:
  gate_review_on_required_failure: true
  grace_period_seconds: 5
"#;

    #[cfg(not(windows))]
    let val_yaml = r#"schema_version: 1
enabled: true
commands:
  - id: check-sentinel
    name: "Check Clean Sentinel"
    command: "test -f target/clean.txt"
    timeout_seconds: 30
    required: true
policy:
  gate_review_on_required_failure: true
  grace_period_seconds: 5
"#;

    fs::write(&validation_path, val_yaml).unwrap();

    // Transition to READY_TO_FREEZE
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
    FreezeService::confirm_freeze(
        repo_path,
        &project_id,
        &preview.preview_id,
        &git,
        db.connection_mut(),
    )
    .unwrap();

    (repo_temp, db_temp, project_id, db_path)
}

#[tokio::test]
async fn test_stage4_end_to_end_governed_loop_acceptance() {
    let (repo_temp, _db_temp, project_id, db_path) = setup_frozen_test_project();
    let repo_path = repo_temp.path();
    let git = GitAdapter::new().unwrap();

    let db_manager = Arc::new(Mutex::new(DbManager::open(&db_path).unwrap()));
    let validation_reg = Arc::new(Mutex::new(ActiveValidationRegistry::new()));
    let app_data_temp = tempdir().unwrap();
    let app_data_dir = app_data_temp.path();

    // 1. Initial State: FROZEN. Start building turn.
    {
        let mut db = db_manager.lock().await;
        apply_workflow_action(
            db.connection_mut(),
            &project_id,
            WorkflowAction::StartBuild,
            "HUMAN",
        )
        .unwrap();
    }

    // 2. Introduce semantic defect in code: a new source file with an issue
    let src_dir = repo_path.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(
        src_dir.join("lib.rs"),
        "pub fn process_data(buf: &[u8]) { /* semantic defect: missing bounds check */ }\n",
    )
    .unwrap();

    // 3. Complete builder turn and transition to VALIDATING
    {
        let mut db = db_manager.lock().await;
        apply_workflow_action(
            db.connection_mut(),
            &project_id,
            WorkflowAction::StartValidation,
            "HUMAN",
        )
        .unwrap();
    }

    // 4. Validation Engine Run #1: Fails because target/clean.txt does not exist
    let run_1 = ValidationService::execute_validation_run(
        db_manager.clone(),
        validation_reg.clone(),
        &project_id,
        repo_path,
        ValidationTriggerSource::PostBuild,
        None,
        None,
        app_data_dir,
    )
    .await
    .unwrap();

    assert_eq!(
        run_1.status,
        coalition_lib::core::validation::ValidationRunStatus::Fail
    );
    assert!(!run_1.is_gate_passed);

    // 5. Verification: check_review_gate fails closed on failed required command
    {
        let db = db_manager.lock().await;
        let epoch_id = run_1.epoch_id.as_deref().unwrap_or("epoch-1");
        let gate_res = ValidationService::check_review_gate(
            db.connection(),
            repo_path,
            &project_id,
            "1.0",
            epoch_id,
        );
        assert!(
            gate_res.is_err(),
            "Gate must block submission when required command failed"
        );
    }

    // 6. Verification: Human Authoritative Gate Override
    {
        let mut db = db_manager.lock().await;
        let ovr = ValidationService::override_validation_gate(
            db.connection_mut(),
            repo_path,
            &project_id,
            &run_1.run_id,
            "Human diagnostic inspection: proceeding with review despite missing sentinel",
            "Human Operator",
        )
        .unwrap();
        assert_eq!(ovr.run_id, run_1.run_id);

        // Now gate passes due to human override
        let epoch_id = run_1.epoch_id.as_deref().unwrap_or("epoch-1");
        let gate_ok = ValidationService::check_review_gate(
            db.connection(),
            repo_path,
            &project_id,
            "1.0",
            epoch_id,
        )
        .unwrap()
        .expect("Passed validation run record expected");
        assert!(gate_ok.has_override);
        assert_eq!(gate_ok.run_id, run_1.run_id);
    }

    // 7. Workflow State reached WAITING_FOR_REVIEW (automatically advanced on gate override)
    {
        let db = db_manager.lock().await;
        let state = coalition_lib::core::workflow::get_workflow_state(db.connection(), &project_id)
            .unwrap();
        assert_eq!(state.state, WorkflowState::WaitingForReview);
    }

    // 8. Prepare Review Packet #1
    let (cycle_1, _packet_md) = {
        let db = db_manager.lock().await;
        ReviewService::prepare_review_packet(
            db.connection(),
            repo_path,
            &project_id,
            ReviewerType::ChatgptRelay,
        )
        .unwrap()
    };
    assert_eq!(cycle_1.cycle_number, 1);
    assert_eq!(
        cycle_1.status,
        coalition_lib::core::review::ReviewCycleStatus::Pending
    );

    // 9. Reviewer returns CORRECTIONS_REQUIRED with structured findings
    let reviewer_resp_1 = format!(
        r#"
We have reviewed the implementation diff and untracked files.

```verdict
schema_version: 1
response_type: REVIEW_VERDICT
project_id: "{}"
cycle_id: "{}"
review_packet_hash: "{}"
verdict: CORRECTIONS_REQUIRED
summary: "Memory safety concern: buffer bounds check is missing in lib.rs."
findings:
  - id: FND-1
    severity: CRITICAL
    title: "Missing buffer bounds check"
    file: "src/lib.rs"
    lines: "1-2"
    description: "The function process_data accepts buf without validating minimum length."
    suggested_fix: "Add `if buf.len() < 4 {{ return; }}` at the beginning of process_data."
```
"#,
        project_id, cycle_1.cycle_id, cycle_1.review_packet_hash
    );

    // Preview import
    let preview_1 = {
        let db = db_manager.lock().await;
        ReviewService::prepare_review_import(
            db.connection(),
            repo_path,
            &project_id,
            &cycle_1.cycle_id,
            &reviewer_resp_1,
        )
        .unwrap()
    };
    assert_eq!(preview_1.verdict, ReviewVerdict::CorrectionsRequired);
    assert_eq!(preview_1.findings.len(), 1);

    // Confirm import -> transitions to CORRECTIONS_REQUIRED
    let confirmed_1 = {
        let mut db = db_manager.lock().await;
        ReviewService::confirm_review_import(
            db.connection_mut(),
            repo_path,
            &project_id,
            &preview_1.preview_id,
            "HUMAN",
        )
        .unwrap()
    };
    assert_eq!(
        confirmed_1.status,
        coalition_lib::core::review::ReviewCycleStatus::CorrectionsRequired
    );
    assert!(confirmed_1.corrections_packet.is_some());

    {
        let db = db_manager.lock().await;
        let state = coalition_lib::core::workflow::get_workflow_state(db.connection(), &project_id)
            .unwrap();
        assert_eq!(state.state, WorkflowState::CorrectionsRequired);
    }

    // Verify durable disk artifacts for Cycle 1 exist under .coalition/reviews/cycle-1/
    let cycle_1_dir = repo_path.join(".coalition").join("reviews").join("cycle-1");
    assert!(cycle_1_dir.join("packet.md").exists());
    assert!(cycle_1_dir.join("response.md").exists());
    assert!(cycle_1_dir.join("verdict.yaml").exists());
    assert!(cycle_1_dir.join("corrections.md").exists());

    // 10. Start Builder Correction Turn
    {
        let mut db = db_manager.lock().await;
        apply_workflow_action(
            db.connection_mut(),
            &project_id,
            WorkflowAction::StartBuild,
            "HUMAN",
        )
        .unwrap();
    }

    // 11. Builder fixes the defect:
    // Update src/lib.rs with bounds check and create target/clean.txt so validation will pass
    fs::write(
        src_dir.join("lib.rs"),
        "pub fn process_data(buf: &[u8]) {\n    if buf.len() < 4 { return; }\n}\n",
    )
    .unwrap();
    let target_dir = repo_path.join("target");
    fs::create_dir_all(&target_dir).unwrap();
    fs::write(target_dir.join("clean.txt"), "clean sentinel\n").unwrap();

    // Complete builder turn -> VALIDATING
    {
        let mut db = db_manager.lock().await;
        apply_workflow_action(
            db.connection_mut(),
            &project_id,
            WorkflowAction::StartValidation,
            "HUMAN",
        )
        .unwrap();
    }

    // 12. Run Validation Engine Run #2: Passes!
    let run_2 = ValidationService::execute_validation_run(
        db_manager.clone(),
        validation_reg.clone(),
        &project_id,
        repo_path,
        ValidationTriggerSource::PostBuild,
        None,
        None,
        app_data_dir,
    )
    .await
    .unwrap();

    assert_eq!(
        run_2.status,
        coalition_lib::core::validation::ValidationRunStatus::Pass
    );
    assert!(run_2.is_gate_passed);

    // Review gate now passes without any override!
    {
        let db = db_manager.lock().await;
        let epoch_id = run_2.epoch_id.as_deref().unwrap_or("epoch-1");
        let gate_ok = ValidationService::check_review_gate(
            db.connection(),
            repo_path,
            &project_id,
            "1.0",
            epoch_id,
        )
        .unwrap()
        .expect("Passed validation run record expected");
        assert!(!gate_ok.has_override);
        assert_eq!(gate_ok.run_id, run_2.run_id);
    }

    // 13. Submit for Review (Round 2)
    {
        let mut db = db_manager.lock().await;
        apply_workflow_action(
            db.connection_mut(),
            &project_id,
            WorkflowAction::SubmitForReview,
            "HUMAN",
        )
        .unwrap();
    }

    // Prepare Review Packet #2
    let (cycle_2, _packet_md_2) = {
        let db = db_manager.lock().await;
        ReviewService::prepare_review_packet(
            db.connection(),
            repo_path,
            &project_id,
            ReviewerType::ChatgptRelay,
        )
        .unwrap()
    };
    assert_eq!(cycle_2.cycle_number, 2);

    // 14. Reviewer returns ACCEPT
    let reviewer_resp_2 = format!(
        r#"
```verdict
schema_version: 1
response_type: REVIEW_VERDICT
project_id: "{}"
cycle_id: "{}"
review_packet_hash: "{}"
verdict: ACCEPT
summary: "All findings from Cycle #1 have been cleanly resolved. Bounds check verified."
findings: []
```
"#,
        project_id, cycle_2.cycle_id, cycle_2.review_packet_hash
    );

    let preview_2 = {
        let db = db_manager.lock().await;
        ReviewService::prepare_review_import(
            db.connection(),
            repo_path,
            &project_id,
            &cycle_2.cycle_id,
            &reviewer_resp_2,
        )
        .unwrap()
    };
    assert_eq!(preview_2.verdict, ReviewVerdict::Accept);

    let confirmed_2 = {
        let mut db = db_manager.lock().await;
        ReviewService::confirm_review_import(
            db.connection_mut(),
            repo_path,
            &project_id,
            &preview_2.preview_id,
            "HUMAN",
        )
        .unwrap()
    };
    assert_eq!(
        confirmed_2.status,
        coalition_lib::core::review::ReviewCycleStatus::Accepted
    );

    // Workflow state authoritatively reached REVIEW_ACCEPTED!
    {
        let db = db_manager.lock().await;
        let state = coalition_lib::core::workflow::get_workflow_state(db.connection(), &project_id)
            .unwrap();
        assert_eq!(state.state, WorkflowState::ReviewAccepted);
    }

    // Verify durable disk artifacts for Cycle 2 exist
    let cycle_2_dir = repo_path.join(".coalition").join("reviews").join("cycle-2");
    assert!(cycle_2_dir.join("packet.md").exists());
    assert!(cycle_2_dir.join("response.md").exists());
    assert!(cycle_2_dir.join("verdict.yaml").exists());

    // 15. Invariant Test: Project truth survives SQLite deletion (rehydration)
    drop(db_manager);
    fs::remove_file(&db_path).unwrap();

    let mut fresh_db = DbManager::open(&db_path).unwrap();
    fresh_db.run_migrations().unwrap();

    // Reopen project from disk
    ProjectService::register_or_open_project(
        fresh_db.connection_mut(),
        &git,
        repo_path.to_str().unwrap(),
    )
    .unwrap();

    // Verify that register_or_open_project automatically rehydrated reviews from disk (Criterion 16)
    let all_rehydrated =
        list_review_cycles_for_project(fresh_db.connection(), &project_id, 20).unwrap();
    assert_eq!(
        all_rehydrated.len(),
        2,
        "Both cycle 1 and cycle 2 must automatically rehydrate on project reopen"
    );

    // Query latest review cycle from rehydrated SQLite
    let latest_rehydrated = get_latest_review_cycle(fresh_db.connection(), &project_id)
        .unwrap()
        .expect("Latest cycle must exist after rehydration");

    assert_eq!(latest_rehydrated.cycle_number, 2);
    assert_eq!(
        latest_rehydrated.status,
        coalition_lib::core::review::ReviewCycleStatus::Accepted
    );
    assert_eq!(latest_rehydrated.verdict, Some(ReviewVerdict::Accept));

    let all_cycles =
        list_review_cycles_for_project(fresh_db.connection(), &project_id, 20).unwrap();
    assert_eq!(all_cycles.len(), 2);
    assert_eq!(all_cycles[0].cycle_number, 2);
    assert_eq!(all_cycles[1].cycle_number, 1);
    assert_eq!(all_cycles[1].findings.len(), 1);
    assert_eq!(
        all_cycles[1].findings[0].title,
        "Missing buffer bounds check"
    );
}

#[tokio::test]
async fn test_stale_review_preview_guard() {
    let (repo_temp, _db_temp, project_id, db_path) = setup_frozen_test_project();
    let repo_path = repo_temp.path();

    let db_manager = Arc::new(Mutex::new(DbManager::open(&db_path).unwrap()));
    let validation_reg = Arc::new(Mutex::new(ActiveValidationRegistry::new()));
    let app_data_temp = tempdir().unwrap();
    let app_data_dir = app_data_temp.path();

    // Move to VALIDATING then WAITING_FOR_REVIEW
    {
        let mut db = db_manager.lock().await;
        apply_workflow_action(
            db.connection_mut(),
            &project_id,
            WorkflowAction::StartBuild,
            "HUMAN",
        )
        .unwrap();
        apply_workflow_action(
            db.connection_mut(),
            &project_id,
            WorkflowAction::StartValidation,
            "HUMAN",
        )
        .unwrap();
    }

    // Create sentinel so validation passes
    let target_dir = repo_path.join("target");
    fs::create_dir_all(&target_dir).unwrap();
    fs::write(target_dir.join("clean.txt"), "clean\n").unwrap();

    ValidationService::execute_validation_run(
        db_manager.clone(),
        validation_reg.clone(),
        &project_id,
        repo_path,
        ValidationTriggerSource::PostBuild,
        None,
        None,
        app_data_dir,
    )
    .await
    .unwrap();

    {
        let mut db = db_manager.lock().await;
        apply_workflow_action(
            db.connection_mut(),
            &project_id,
            WorkflowAction::SubmitForReview,
            "HUMAN",
        )
        .unwrap();
    }

    let (cycle, _packet_md) = {
        let db = db_manager.lock().await;
        ReviewService::prepare_review_packet(
            db.connection(),
            repo_path,
            &project_id,
            ReviewerType::ChatgptRelay,
        )
        .unwrap()
    };

    let reviewer_resp = format!(
        "```verdict\nschema_version: 1\nresponse_type: REVIEW_VERDICT\nproject_id: \"{}\"\ncycle_id: \"{}\"\nreview_packet_hash: \"{}\"\nverdict: ACCEPT\nsummary: Approved\nfindings: []\n```",
        project_id, cycle.cycle_id, cycle.review_packet_hash
    );
    let preview = {
        let db = db_manager.lock().await;
        ReviewService::prepare_review_import(
            db.connection(),
            repo_path,
            &project_id,
            &cycle.cycle_id,
            &reviewer_resp,
        )
        .unwrap()
    };

    // Tamper with repo after preview: change a source file
    fs::write(repo_path.join("README.md"), "# Modified while in review\n").unwrap();

    // Confirm import must fail closed with StalePreview
    let confirm_res = {
        let mut db = db_manager.lock().await;
        ReviewService::confirm_review_import(
            db.connection_mut(),
            repo_path,
            &project_id,
            &preview.preview_id,
            "HUMAN",
        )
    };

    assert!(confirm_res.is_err());
    match confirm_res.unwrap_err() {
        ReviewError::StalePreview(msg) => {
            assert!(msg.contains("Working tree fingerprint has changed"));
        }
        other => panic!("Expected StalePreview error, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_review_blocked_and_arch_concern_verdicts() {
    let (repo_temp, _db_temp, project_id, db_path) = setup_frozen_test_project();
    let repo_path = repo_temp.path();

    let db_manager = Arc::new(Mutex::new(DbManager::open(&db_path).unwrap()));
    let validation_reg = Arc::new(Mutex::new(ActiveValidationRegistry::new()));
    let app_data_temp = tempdir().unwrap();
    let app_data_dir = app_data_temp.path();

    // Start build and validation
    {
        let mut db = db_manager.lock().await;
        apply_workflow_action(
            db.connection_mut(),
            &project_id,
            WorkflowAction::StartBuild,
            "HUMAN",
        )
        .unwrap();
        apply_workflow_action(
            db.connection_mut(),
            &project_id,
            WorkflowAction::StartValidation,
            "HUMAN",
        )
        .unwrap();
    }

    let target_dir = repo_path.join("target");
    fs::create_dir_all(&target_dir).unwrap();
    fs::write(target_dir.join("clean.txt"), "clean\n").unwrap();

    ValidationService::execute_validation_run(
        db_manager.clone(),
        validation_reg.clone(),
        &project_id,
        repo_path,
        ValidationTriggerSource::PostBuild,
        None,
        None,
        app_data_dir,
    )
    .await
    .unwrap();

    {
        let mut db = db_manager.lock().await;
        apply_workflow_action(
            db.connection_mut(),
            &project_id,
            WorkflowAction::SubmitForReview,
            "HUMAN",
        )
        .unwrap();
    }

    let (cycle_1, _) = {
        let db = db_manager.lock().await;
        ReviewService::prepare_review_packet(
            db.connection(),
            repo_path,
            &project_id,
            ReviewerType::ChatgptRelay,
        )
        .unwrap()
    };

    // Test BLOCKED verdict
    let resp_blocked = format!(
        "```verdict\nschema_version: 1\nresponse_type: REVIEW_VERDICT\nproject_id: \"{}\"\ncycle_id: \"{}\"\nreview_packet_hash: \"{}\"\nverdict: BLOCKED\nsummary: \"Critical dependency vulnerability\"\nfindings: []\n```",
        project_id, cycle_1.cycle_id, cycle_1.review_packet_hash
    );

    let prev_blocked = {
        let db = db_manager.lock().await;
        ReviewService::prepare_review_import(
            db.connection(),
            repo_path,
            &project_id,
            &cycle_1.cycle_id,
            &resp_blocked,
        )
        .unwrap()
    };
    assert_eq!(prev_blocked.verdict, ReviewVerdict::Blocked);

    let confirmed_blocked = {
        let mut db = db_manager.lock().await;
        ReviewService::confirm_review_import(
            db.connection_mut(),
            repo_path,
            &project_id,
            &prev_blocked.preview_id,
            "HUMAN",
        )
        .unwrap()
    };
    assert_eq!(
        confirmed_blocked.status,
        coalition_lib::core::review::ReviewCycleStatus::Blocked
    );

    // Verify workflow state is Blocked
    {
        let db = db_manager.lock().await;
        let st = coalition_lib::core::workflow::get_workflow_state(db.connection(), &project_id)
            .unwrap();
        assert_eq!(st.state, WorkflowState::Blocked);
    }
}

#[tokio::test]
async fn test_review_architecture_concern_verdict() {
    let (repo_temp, _db_temp, project_id, db_path) = setup_frozen_test_project();
    let repo_path = repo_temp.path();

    let db_manager = Arc::new(Mutex::new(DbManager::open(&db_path).unwrap()));
    let validation_reg = Arc::new(Mutex::new(ActiveValidationRegistry::new()));
    let app_data_temp = tempdir().unwrap();
    let app_data_dir = app_data_temp.path();

    {
        let mut db = db_manager.lock().await;
        apply_workflow_action(
            db.connection_mut(),
            &project_id,
            WorkflowAction::StartBuild,
            "HUMAN",
        )
        .unwrap();
        apply_workflow_action(
            db.connection_mut(),
            &project_id,
            WorkflowAction::StartValidation,
            "HUMAN",
        )
        .unwrap();
    }

    let target_dir = repo_path.join("target");
    fs::create_dir_all(&target_dir).unwrap();
    fs::write(target_dir.join("clean.txt"), "clean\n").unwrap();

    ValidationService::execute_validation_run(
        db_manager.clone(),
        validation_reg.clone(),
        &project_id,
        repo_path,
        ValidationTriggerSource::PostBuild,
        None,
        None,
        app_data_dir,
    )
    .await
    .unwrap();

    {
        let mut db = db_manager.lock().await;
        apply_workflow_action(
            db.connection_mut(),
            &project_id,
            WorkflowAction::SubmitForReview,
            "HUMAN",
        )
        .unwrap();
    }

    let (cycle, _) = {
        let db = db_manager.lock().await;
        ReviewService::prepare_review_packet(
            db.connection(),
            repo_path,
            &project_id,
            ReviewerType::ChatgptRelay,
        )
        .unwrap()
    };

    let resp_concern = format!(
        "```verdict\nschema_version: 1\nresponse_type: REVIEW_VERDICT\nproject_id: \"{}\"\ncycle_id: \"{}\"\nreview_packet_hash: \"{}\"\nverdict: ARCHITECTURE_CONCERN\nsummary: \"Contract requirement contradiction\"\nfindings: []\n```",
        project_id, cycle.cycle_id, cycle.review_packet_hash
    );

    let prev_concern = {
        let db = db_manager.lock().await;
        ReviewService::prepare_review_import(
            db.connection(),
            repo_path,
            &project_id,
            &cycle.cycle_id,
            &resp_concern,
        )
        .unwrap()
    };
    assert_eq!(prev_concern.verdict, ReviewVerdict::ArchitectureConcern);

    let confirmed = {
        let mut db = db_manager.lock().await;
        ReviewService::confirm_review_import(
            db.connection_mut(),
            repo_path,
            &project_id,
            &prev_concern.preview_id,
            "HUMAN",
        )
        .unwrap()
    };
    assert_eq!(
        confirmed.status,
        coalition_lib::core::review::ReviewCycleStatus::ArchitectureConcern
    );

    {
        let db = db_manager.lock().await;
        let st = coalition_lib::core::workflow::get_workflow_state(db.connection(), &project_id)
            .unwrap();
        assert_eq!(st.state, WorkflowState::ArchitectureConcern);
    }
}

#[tokio::test]
async fn test_manual_validation_disabling() {
    let (repo_temp, _db_temp, project_id, db_path) = setup_frozen_test_project();
    let repo_path = repo_temp.path();

    // Disable manual runs in validation.yaml
    let val_path = repo_path
        .join(".coalition")
        .join("implementation")
        .join("validation.yaml");
    #[cfg(windows)]
    let val_yaml = r#"schema_version: 1
enabled: true
triggers:
  allow_manual_runs: false
  allow_builder_requested_runs: false
commands:
  - id: check-sentinel
    name: "Check Clean Sentinel"
    command: "cmd /c exit 0"
    timeout_seconds: 30
    required: true
policy:
  gate_review_on_required_failure: true
  grace_period_seconds: 5
"#;
    #[cfg(not(windows))]
    let val_yaml = r#"schema_version: 1
enabled: true
triggers:
  allow_manual_runs: false
  allow_builder_requested_runs: false
commands:
  - id: check-sentinel
    name: "Check Clean Sentinel"
    command: "exit 0"
    timeout_seconds: 30
    required: true
policy:
  gate_review_on_required_failure: true
  grace_period_seconds: 5
"#;
    fs::write(&val_path, val_yaml).unwrap();

    let db_manager = Arc::new(Mutex::new(DbManager::open(&db_path).unwrap()));
    let validation_reg = Arc::new(Mutex::new(ActiveValidationRegistry::new()));
    let app_data_temp = tempdir().unwrap();
    let app_data_dir = app_data_temp.path();

    let res = ValidationService::execute_validation_run(
        db_manager,
        validation_reg,
        &project_id,
        repo_path,
        ValidationTriggerSource::Manual,
        None,
        None,
        app_data_dir,
    )
    .await;

    assert!(res.is_err());
    let err_str = res.unwrap_err().to_string();
    assert!(err_str.contains("Manual validation runs are disabled"));
}

#[tokio::test]
async fn test_partial_validation_gate_rejection() {
    let (repo_temp, _db_temp, project_id, db_path) = setup_frozen_test_project();
    let repo_path = repo_temp.path();

    // Config with 2 required commands
    let val_path = repo_path
        .join(".coalition")
        .join("implementation")
        .join("validation.yaml");
    #[cfg(windows)]
    let val_yaml = r#"schema_version: 1
enabled: true
commands:
  - id: cmd-1
    name: "Command 1"
    command: "cmd /c exit 0"
    timeout_seconds: 30
    required: true
  - id: cmd-2
    name: "Command 2"
    command: "cmd /c exit 0"
    timeout_seconds: 30
    required: true
policy:
  gate_review_on_required_failure: true
  grace_period_seconds: 5
"#;
    #[cfg(not(windows))]
    let val_yaml = r#"schema_version: 1
enabled: true
commands:
  - id: cmd-1
    name: "Command 1"
    command: "exit 0"
    timeout_seconds: 30
    required: true
  - id: cmd-2
    name: "Command 2"
    command: "exit 0"
    timeout_seconds: 30
    required: true
policy:
  gate_review_on_required_failure: true
  grace_period_seconds: 5
"#;
    fs::write(&val_path, val_yaml).unwrap();

    let db_manager = Arc::new(Mutex::new(DbManager::open(&db_path).unwrap()));
    let validation_reg = Arc::new(Mutex::new(ActiveValidationRegistry::new()));
    let app_data_temp = tempdir().unwrap();
    let app_data_dir = app_data_temp.path();

    // Run only cmd-1
    let run = ValidationService::execute_validation_run(
        db_manager,
        validation_reg,
        &project_id,
        repo_path,
        ValidationTriggerSource::PostBuild,
        Some(vec!["cmd-1".to_string()]),
        None,
        app_data_dir,
    )
    .await
    .unwrap();

    // Gate must NOT pass because cmd-2 was not run!
    assert!(
        !run.is_gate_passed,
        "Gate must not pass when required commands were omitted"
    );
}

#[tokio::test]
async fn test_strict_envelope_rejections() {
    let (repo_temp, _db_temp, project_id, db_path) = setup_frozen_test_project();
    let repo_path = repo_temp.path();

    let db_manager = Arc::new(Mutex::new(DbManager::open(&db_path).unwrap()));
    let validation_reg = Arc::new(Mutex::new(ActiveValidationRegistry::new()));
    let app_data_temp = tempdir().unwrap();
    let app_data_dir = app_data_temp.path();

    // Advance to WAITING_FOR_REVIEW
    {
        let mut db = db_manager.lock().await;
        apply_workflow_action(
            db.connection_mut(),
            &project_id,
            WorkflowAction::StartBuild,
            "HUMAN",
        )
        .unwrap();
        apply_workflow_action(
            db.connection_mut(),
            &project_id,
            WorkflowAction::StartValidation,
            "HUMAN",
        )
        .unwrap();
    }

    let target_dir = repo_path.join("target");
    fs::create_dir_all(&target_dir).unwrap();
    fs::write(target_dir.join("clean.txt"), "clean\n").unwrap();

    ValidationService::execute_validation_run(
        db_manager.clone(),
        validation_reg.clone(),
        &project_id,
        repo_path,
        ValidationTriggerSource::PostBuild,
        None,
        None,
        app_data_dir,
    )
    .await
    .unwrap();

    {
        let mut db = db_manager.lock().await;
        apply_workflow_action(
            db.connection_mut(),
            &project_id,
            WorkflowAction::SubmitForReview,
            "HUMAN",
        )
        .unwrap();
    }

    let (cycle, _) = {
        let db = db_manager.lock().await;
        ReviewService::prepare_review_packet(
            db.connection(),
            repo_path,
            &project_id,
            ReviewerType::ChatgptRelay,
        )
        .unwrap()
    };

    // 1. Missing verdict field
    let missing_verdict = format!(
        "```verdict\nschema_version: 1\nresponse_type: REVIEW_VERDICT\nproject_id: \"{}\"\ncycle_id: \"{}\"\nreview_packet_hash: \"{}\"\nsummary: Test missing verdict\nfindings: []\n```",
        project_id, cycle.cycle_id, cycle.review_packet_hash
    );
    {
        let db = db_manager.lock().await;
        let err = ReviewService::prepare_review_import(
            db.connection(),
            repo_path,
            &project_id,
            &cycle.cycle_id,
            &missing_verdict,
        );
        assert!(
            err.is_err(),
            "Must reject verdict envelope missing verdict field"
        );
    }

    // 2. Missing review_packet_hash field
    let missing_hash = format!(
        "```verdict\nschema_version: 1\nresponse_type: REVIEW_VERDICT\nproject_id: \"{}\"\ncycle_id: \"{}\"\nverdict: ACCEPT\nsummary: Test missing hash\nfindings: []\n```",
        project_id, cycle.cycle_id
    );
    {
        let db = db_manager.lock().await;
        let err = ReviewService::prepare_review_import(
            db.connection(),
            repo_path,
            &project_id,
            &cycle.cycle_id,
            &missing_hash,
        );
        assert!(
            err.is_err(),
            "Must reject verdict envelope missing review_packet_hash"
        );
    }

    // 3. Unknown field injected (deny_unknown_fields)
    let unknown_field = format!(
        "```verdict\nschema_version: 1\nresponse_type: REVIEW_VERDICT\nproject_id: \"{}\"\ncycle_id: \"{}\"\nreview_packet_hash: \"{}\"\nverdict: ACCEPT\nsummary: Test unknown field\nfindings: []\nextra_unknown_prop: \"injected\"\n```",
        project_id, cycle.cycle_id, cycle.review_packet_hash
    );
    {
        let db = db_manager.lock().await;
        let err = ReviewService::prepare_review_import(
            db.connection(),
            repo_path,
            &project_id,
            &cycle.cycle_id,
            &unknown_field,
        );
        assert!(
            err.is_err(),
            "Must reject verdict envelope with unknown fields"
        );
    }
}

#[tokio::test]
async fn test_override_validation_gate_fails_closed_on_mismatched_epoch() {
    let (repo_temp, _db_temp, project_id, db_path) = setup_frozen_test_project();
    let repo_path = repo_temp.path();

    let mut db = DbManager::open(&db_path).unwrap();

    let coalition_dir = repo_path.join(".coalition");
    let py = coalition_lib::core::artifacts::ArtifactManager::read_project_yaml(
        coalition_dir.join("project.yaml"),
    )
    .unwrap();
    let arch_version = py
        .current_architecture_version
        .unwrap_or_else(|| "1.0.0".to_string());

    // Insert an active epoch into builder_epochs (column is `status`)
    db.connection_mut().execute(
        "INSERT INTO builder_epochs (epoch_id, project_id, architecture_version, created_at, status) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params!["epoch-active-current", &project_id, &arch_version, "2026-09-24T00:00:00Z", "BUILDING"],
    ).unwrap();

    // Create a failed validation run with a different epoch
    let run_rec = coalition_lib::core::validation::ValidationRunRecord {
        run_id: "val-run-old-epoch".to_string(),
        project_id: project_id.clone(),
        architecture_version: arch_version.clone(),
        epoch_id: Some("epoch-stale-old".to_string()),
        trigger_source: ValidationTriggerSource::PostBuild,
        status: coalition_lib::core::validation::ValidationRunStatus::Fail,
        is_gate_passed: false,
        has_override: false,
        git_head: Some("abc1234".to_string()),
        git_dirty_fingerprint: Some("none".to_string()),
        config_fingerprint: Some(
            ValidationService::read_validation_config(repo_path)
                .unwrap()
                .fingerprint(),
        ),
        log_path: None,
        started_at: "2026-09-24T00:00:00Z".to_string(),
        completed_at: Some("2026-09-24T00:00:05Z".to_string()),
        duration_ms: 5000,
        commands: vec![],
    };
    coalition_lib::core::validation::insert_validation_run(db.connection_mut(), &run_rec).unwrap();

    // Attempt override - must fail closed because epoch doesn't match current epoch
    let result = ValidationService::override_validation_gate(
        db.connection_mut(),
        repo_path,
        &project_id,
        "val-run-old-epoch",
        "Override attempt with stale epoch",
        "HUMAN",
    );

    assert!(
        result.is_err(),
        "Override must fail closed when run epoch does not match current builder epoch"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("does not match current Builder epoch"));
}

#[tokio::test]
async fn test_crash_safe_journal_recovery() {
    let (repo_temp, _db_temp, project_id, db_path) = setup_frozen_test_project();
    let repo_path = repo_temp.path();
    let reviews_dir = repo_path.join(".coalition").join("reviews");
    fs::create_dir_all(&reviews_dir).unwrap();

    let mut db = DbManager::open(&db_path).unwrap();

    // Seam 1: STAGING phase - staging dir exists, target dir does not exist yet.
    // Uncommitted SQLite transaction: Reconcile must remove the orphaned staging dir and clean up journal row.
    let staging_1 = reviews_dir.join(".staging-cycle-99");
    fs::create_dir_all(&staging_1).unwrap();
    fs::write(staging_1.join("temp.txt"), "in progress").unwrap();

    db.connection_mut().execute(
        "INSERT INTO review_acceptance_journal (cycle_id, project_id, preview_id, phase, staging_dir, target_dir, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            "cycle-staging-test",
            &project_id,
            "prev-1",
            "STAGING",
            staging_1.to_str().unwrap(),
            reviews_dir.join("cycle-99").to_str().unwrap(),
            "2026-09-24T00:00:00Z",
            "2026-09-24T00:00:00Z",
        ],
    ).unwrap();

    let reconciled = ReviewService::reconcile_review_acceptance_journal(
        db.connection_mut(),
        repo_path,
        &project_id,
    )
    .unwrap();
    assert_eq!(reconciled, 1);
    assert!(
        !staging_1.exists(),
        "Orphaned staging dir must be deleted on recovery"
    );

    let count: i64 = db
        .connection_mut()
        .query_row(
            "SELECT COUNT(*) FROM review_acceptance_journal WHERE cycle_id = 'cycle-staging-test'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0, "Journal entry must be deleted after recovery");

    // Seam 2: COMMITTED phase where staging dir exists and target dir does not exist yet.
    // SQLite transaction committed: Reconcile must promote staging dir to target dir and remove journal row.
    let staging_2 = reviews_dir.join(".staging-cycle-100");
    let target_2 = reviews_dir.join("cycle-100");
    fs::create_dir_all(&staging_2).unwrap();
    fs::write(staging_2.join("verdict.yaml"), "schema_version: 1\n").unwrap();

    db.connection_mut().execute(
        "INSERT INTO review_acceptance_journal (cycle_id, project_id, preview_id, phase, staging_dir, target_dir, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            "cycle-committing-test",
            &project_id,
            "prev-2",
            "COMMITTED",
            staging_2.to_str().unwrap(),
            target_2.to_str().unwrap(),
            "2026-09-24T00:00:00Z",
            "2026-09-24T00:00:00Z",
        ],
    ).unwrap();

    let reconciled_2 = ReviewService::reconcile_review_acceptance_journal(
        db.connection_mut(),
        repo_path,
        &project_id,
    )
    .unwrap();
    assert_eq!(reconciled_2, 1);
    assert!(!staging_2.exists(), "Staging dir should no longer exist");
    assert!(
        target_2.exists(),
        "Target dir must be promoted from staging dir"
    );

    let count_2: i64 = db.connection_mut().query_row(
        "SELECT COUNT(*) FROM review_acceptance_journal WHERE cycle_id = 'cycle-committing-test'",
        [],
        |r| r.get(0),
    ).unwrap();
    assert_eq!(count_2, 0);

    // Seam 3: COMMITTED phase where target dir already exists.
    // Reconcile must remove journal entry cleanly.
    db.connection_mut().execute(
        "INSERT INTO review_acceptance_journal (cycle_id, project_id, preview_id, phase, staging_dir, target_dir, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            "cycle-committed-test",
            &project_id,
            "prev-3",
            "COMMITTED",
            staging_2.to_str().unwrap(),
            target_2.to_str().unwrap(),
            "2026-09-24T00:00:00Z",
            "2026-09-24T00:00:00Z",
        ],
    ).unwrap();

    let reconciled_3 = ReviewService::reconcile_review_acceptance_journal(
        db.connection_mut(),
        repo_path,
        &project_id,
    )
    .unwrap();
    assert_eq!(reconciled_3, 1);

    let count_3: i64 = db.connection_mut().query_row(
        "SELECT COUNT(*) FROM review_acceptance_journal WHERE cycle_id = 'cycle-committed-test'",
        [],
        |r| r.get(0),
    ).unwrap();
    assert_eq!(count_3, 0);
}

#[tokio::test]
async fn test_review_packet_hash_identity() {
    use sha2::{Digest, Sha256};

    let (repo_temp, _db_temp, project_id, db_path) = setup_frozen_test_project();
    let repo_path = repo_temp.path();

    let db_manager = Arc::new(Mutex::new(DbManager::open(&db_path).unwrap()));
    let validation_reg = Arc::new(Mutex::new(ActiveValidationRegistry::new()));
    let app_data_temp = tempdir().unwrap();
    let app_data_dir = app_data_temp.path();

    // Advance to WAITING_FOR_REVIEW
    {
        let mut db = db_manager.lock().await;
        apply_workflow_action(
            db.connection_mut(),
            &project_id,
            WorkflowAction::StartBuild,
            "HUMAN",
        )
        .unwrap();
        apply_workflow_action(
            db.connection_mut(),
            &project_id,
            WorkflowAction::StartValidation,
            "HUMAN",
        )
        .unwrap();
    }

    let target_dir = repo_path.join("target");
    fs::create_dir_all(&target_dir).unwrap();
    fs::write(target_dir.join("clean.txt"), "clean\n").unwrap();

    ValidationService::execute_validation_run(
        db_manager.clone(),
        validation_reg.clone(),
        &project_id,
        repo_path,
        ValidationTriggerSource::PostBuild,
        None,
        None,
        app_data_dir,
    )
    .await
    .unwrap();

    {
        let mut db = db_manager.lock().await;
        apply_workflow_action(
            db.connection_mut(),
            &project_id,
            WorkflowAction::SubmitForReview,
            "HUMAN",
        )
        .unwrap();
    }

    let (cycle, packet_text) = {
        let db = db_manager.lock().await;
        ReviewService::prepare_review_packet(
            db.connection(),
            repo_path,
            &project_id,
            ReviewerType::ChatgptRelay,
        )
        .unwrap()
    };

    // Verify disk packet file exists
    let packet_file = repo_path
        .join(".coalition")
        .join("reviews")
        .join("cycle-1")
        .join("packet.md");
    assert!(packet_file.exists());
    let disk_content = fs::read_to_string(&packet_file).unwrap();
    assert_eq!(disk_content, packet_text);

    // Verify hash calculation is over the body below delimiter
    let body_marker = "=== COALITION REVIEW PACKET BODY ===\n\n";
    let body_start = disk_content
        .find(body_marker)
        .expect("Body marker must be present");
    let packet_body = &disk_content[body_start + body_marker.len()..];

    let computed_hash = format!("{:x}", Sha256::digest(packet_body.as_bytes()));
    assert_eq!(
        cycle.review_packet_hash, computed_hash,
        "Packet hash must strictly equal SHA-256 of the packet body below delimiter"
    );

    // Verify header references the non-self-referential hash
    assert!(
        disk_content.contains(&format!("Review-Packet-Hash: {}", computed_hash)),
        "Header must cleanly state the review packet hash"
    );
}

#[tokio::test]
async fn test_stage4_validation_disabled_continues_to_review() {
    let (repo_temp, _db_temp, project_id, db_path) = setup_frozen_test_project();
    let repo_path = repo_temp.path();
    let app_data_temp = tempdir().unwrap();
    let app_data_dir = app_data_temp.path();

    let db_manager = Arc::new(Mutex::new(DbManager::open(&db_path).unwrap()));
    let val_registry = Arc::new(Mutex::new(ActiveValidationRegistry::new()));

    // 1. Transition FROZEN -> BUILDING
    {
        let mut db = db_manager.lock().await;
        apply_workflow_action(
            db.connection_mut(),
            &project_id,
            WorkflowAction::StartBuild,
            "HUMAN",
        )
        .unwrap();
    }

    // Ensure validation.yaml is explicitly disabled (or absent)
    let val_yaml_path = repo_path
        .join(".coalition")
        .join("implementation")
        .join("validation.yaml");
    fs::create_dir_all(val_yaml_path.parent().unwrap()).unwrap();
    fs::write(
        &val_yaml_path,
        "schema_version: 1\nenabled: false\ncommands: []\npolicy:\n  gate_review_on_required_failure: true\n  grace_period_seconds: 10\n",
    )
    .unwrap();

    // 2. Run post-build validation orchestration
    let result = ValidationService::run_post_build_validation(
        db_manager.clone(),
        val_registry.clone(),
        &project_id,
        repo_path,
        None,
        app_data_dir,
    )
    .await
    .expect("post-build validation should succeed when validation is disabled");

    assert!(
        result.is_none(),
        "No validation run record is created when validation is disabled"
    );

    // 3. Verify workflow state legally transitioned BUILDING -> VALIDATING -> WAITING_FOR_REVIEW
    {
        let db = db_manager.lock().await;
        let wf = coalition_lib::core::workflow::get_workflow_state(db.connection(), &project_id)
            .unwrap();
        assert_eq!(
            wf.state,
            WorkflowState::WaitingForReview,
            "Project must reach WAITING_FOR_REVIEW when validation is disabled"
        );

        // Verify VALIDATION_SKIPPED audit event was logged
        let events =
            ActivityManager::get_project_activity(db.connection(), &project_id, Some(20)).unwrap();
        let skip_event = events
            .iter()
            .find(|e| e.event_type == "VALIDATION_SKIPPED")
            .expect("VALIDATION_SKIPPED event must be recorded in activity history");
        assert!(skip_event.summary.contains("disabled or unconfigured"));

        // Verify review packet can now be prepared directly
        let (cycle, packet) = ReviewService::prepare_review_packet(
            db.connection(),
            repo_path,
            &project_id,
            ReviewerType::ChatgptRelay,
        )
        .expect("Must be able to prepare review packet from WAITING_FOR_REVIEW");
        assert_eq!(cycle.cycle_number, 1);
        assert!(!packet.is_empty());
    }
}

#[tokio::test]
async fn test_stage4_diagnostic_only_mode_allows_review_progression() {
    let (repo_temp, _db_temp, project_id, db_path) = setup_frozen_test_project();
    let repo_path = repo_temp.path();
    let app_data_temp = tempdir().unwrap();
    let app_data_dir = app_data_temp.path();

    let db_manager = Arc::new(Mutex::new(DbManager::open(&db_path).unwrap()));
    let val_registry = Arc::new(Mutex::new(ActiveValidationRegistry::new()));

    // 1. Transition FROZEN -> BUILDING
    {
        let mut db = db_manager.lock().await;
        apply_workflow_action(
            db.connection_mut(),
            &project_id,
            WorkflowAction::StartBuild,
            "HUMAN",
        )
        .unwrap();
    }

    // Configure validation with diagnostic-only policy (gate_review_on_required_failure: false)
    // and a command that purposefully fails
    let val_yaml_path = repo_path
        .join(".coalition")
        .join("implementation")
        .join("validation.yaml");
    fs::create_dir_all(val_yaml_path.parent().unwrap()).unwrap();
    #[cfg(windows)]
    let failing_cmd = "cmd.exe /C exit 1";
    #[cfg(not(windows))]
    let failing_cmd = "false";

    fs::write(
        &val_yaml_path,
        format!(
            "schema_version: 1\nenabled: true\ncommands:\n  - id: cmd-fail\n    name: Failing Check\n    command: '{}'\n    required: true\npolicy:\n  gate_review_on_required_failure: false\n  grace_period_seconds: 10\n",
            failing_cmd
        ),
    )
    .unwrap();

    // 2. Run post-build validation
    let result = ValidationService::run_post_build_validation(
        db_manager.clone(),
        val_registry.clone(),
        &project_id,
        repo_path,
        None,
        app_data_dir,
    )
    .await
    .expect("post-build validation should succeed");

    let record = result.expect("Must return validation run record");
    assert_eq!(
        record.status,
        coalition_lib::core::validation::ValidationRunStatus::Fail
    );
    assert!(
        record.is_gate_passed,
        "In diagnostic-only mode, is_gate_passed must be true despite command failure"
    );

    // 3. Workflow must progress to WAITING_FOR_REVIEW
    {
        let db = db_manager.lock().await;
        let wf = coalition_lib::core::workflow::get_workflow_state(db.connection(), &project_id)
            .unwrap();
        assert_eq!(
            wf.state,
            WorkflowState::WaitingForReview,
            "Diagnostic-only failure must not block progression to WAITING_FOR_REVIEW"
        );
    }
}

#[tokio::test]
async fn test_stage4_reject_review_import_governed_operation() {
    let (repo_temp, _db_temp, project_id, db_path) = setup_frozen_test_project();
    let repo_path = repo_temp.path();
    let app_data_temp = tempdir().unwrap();
    let app_data_dir = app_data_temp.path();

    let db_manager = Arc::new(Mutex::new(DbManager::open(&db_path).unwrap()));
    let val_registry = Arc::new(Mutex::new(ActiveValidationRegistry::new()));

    // 1. Progress to WAITING_FOR_REVIEW
    {
        let mut db = db_manager.lock().await;
        apply_workflow_action(
            db.connection_mut(),
            &project_id,
            WorkflowAction::StartBuild,
            "HUMAN",
        )
        .unwrap();
    }

    // Create sentinel so validation passes
    let target_dir = repo_path.join("target");
    fs::create_dir_all(&target_dir).unwrap();
    fs::write(target_dir.join("clean.txt"), "clean\n").unwrap();

    ValidationService::run_post_build_validation(
        db_manager.clone(),
        val_registry.clone(),
        &project_id,
        repo_path,
        None,
        app_data_dir,
    )
    .await
    .unwrap();

    // 2. Prepare review packet
    let (cycle, _) = {
        let db = db_manager.lock().await;
        ReviewService::prepare_review_packet(
            db.connection(),
            repo_path,
            &project_id,
            ReviewerType::ChatgptRelay,
        )
        .unwrap()
    };

    // 3. Prepare review import preview
    let raw_review_response = format!(
        "```verdict\nschema_version: 1\nresponse_type: REVIEW_VERDICT\nproject_id: \"{}\"\ncycle_id: \"{}\"\nreview_packet_hash: \"{}\"\nverdict: CORRECTIONS_REQUIRED\nsummary: Security flaw found\nfindings:\n  - id: \"1\"\n    severity: MAJOR\n    title: Broken auth\n    description: Missing signature check in auth module\n    problem_statement: Missing signature\n    required_change: Validate Ed25519\n    required_test: Run auth test\n```",
        project_id, cycle.cycle_id, cycle.review_packet_hash
    );
    let preview = {
        let db = db_manager.lock().await;
        ReviewService::prepare_review_import(
            db.connection(),
            repo_path,
            &project_id,
            &cycle.cycle_id,
            &raw_review_response,
        )
        .unwrap()
    };

    // Verify preview row exists in review_import_previews
    {
        let db = db_manager.lock().await;
        let count: i64 = db
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM review_import_previews WHERE preview_id = ?1",
                rusqlite::params![preview.preview_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "Preview row must exist before rejection");
    }

    // 4. Reject the import preview via governed ReviewService::reject_review_import
    {
        let mut db = db_manager.lock().await;
        let rejected_id = ReviewService::reject_review_import(
            db.connection_mut(),
            &project_id,
            &preview.preview_id,
            "HUMAN",
        )
        .expect("Rejecting review import should succeed");
        assert_eq!(rejected_id, preview.preview_id);

        // Verify preview row is deleted
        let count: i64 = db
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM review_import_previews WHERE preview_id = ?1",
                rusqlite::params![preview.preview_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "Preview row must be deleted after rejection");

        // Verify review cycle remains PENDING
        let loaded_cycle =
            coalition_lib::core::review::get_review_cycle(db.connection(), &cycle.cycle_id)
                .unwrap()
                .unwrap();
        assert_eq!(
            loaded_cycle.status,
            coalition_lib::core::review::ReviewCycleStatus::Pending,
            "Review cycle must remain PENDING after import rejection"
        );

        // Verify workflow state remains WAITING_FOR_REVIEW
        let wf = coalition_lib::core::workflow::get_workflow_state(db.connection(), &project_id)
            .unwrap();
        assert_eq!(
            wf.state,
            WorkflowState::WaitingForReview,
            "Workflow must remain in WAITING_FOR_REVIEW after import rejection"
        );

        // Verify REVIEW_IMPORT_REJECTED audit event was recorded
        let events =
            ActivityManager::get_project_activity(db.connection(), &project_id, Some(20)).unwrap();
        let reject_event = events
            .iter()
            .find(|e| e.event_type == "REVIEW_IMPORT_REJECTED")
            .expect("REVIEW_IMPORT_REJECTED activity event must be recorded");
        assert_eq!(reject_event.actor, "HUMAN");
    }

    // 5. Verify confirming the rejected preview ID fails with NotFound
    {
        let mut db = db_manager.lock().await;
        let confirm_res = ReviewService::confirm_review_import(
            db.connection_mut(),
            repo_path,
            &project_id,
            &preview.preview_id,
            "HUMAN",
        );
        match confirm_res {
            Err(ReviewError::NotFound(_)) => {}
            other => panic!("Expected NotFound for rejected preview ID, got {:?}", other),
        }
    }

    // 6. Verify subsequent import preview can be prepared and confirmed
    let accept_response = format!(
        "```verdict\nschema_version: 1\nresponse_type: REVIEW_VERDICT\nproject_id: \"{}\"\ncycle_id: \"{}\"\nreview_packet_hash: \"{}\"\nverdict: ACCEPT\nsummary: Architecture fully compliant\nfindings: []\n```",
        project_id, cycle.cycle_id, cycle.review_packet_hash
    );
    let new_preview = {
        let db = db_manager.lock().await;
        ReviewService::prepare_review_import(
            db.connection(),
            repo_path,
            &project_id,
            &cycle.cycle_id,
            &accept_response,
        )
        .unwrap()
    };

    {
        let mut db = db_manager.lock().await;
        let confirmed_cycle = ReviewService::confirm_review_import(
            db.connection_mut(),
            repo_path,
            &project_id,
            &new_preview.preview_id,
            "HUMAN",
        )
        .expect("Subsequent import confirmation must succeed");
        assert_eq!(
            confirmed_cycle.status,
            coalition_lib::core::review::ReviewCycleStatus::Accepted
        );
        assert_eq!(confirmed_cycle.verdict, Some(ReviewVerdict::Accept));

        let wf = coalition_lib::core::workflow::get_workflow_state(db.connection(), &project_id)
            .unwrap();
        assert_eq!(wf.state, WorkflowState::ReviewAccepted);
    }
}

#[tokio::test]
async fn test_stage4_validation_diagnostic_routing_to_builder() {
    let (repo_temp, _db_temp, project_id, db_path) = setup_frozen_test_project();
    let repo_path = repo_temp.path();
    let app_data_temp = tempdir().unwrap();
    let app_data_dir = app_data_temp.path();

    let db_manager = Arc::new(Mutex::new(DbManager::open(&db_path).unwrap()));
    let val_registry = Arc::new(Mutex::new(ActiveValidationRegistry::new()));
    let builder_registry = Arc::new(Mutex::new(
        coalition_lib::core::builder::ActiveBuilderRegistry::new(),
    ));
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
    let adapter = coalition_lib::core::builder::AntigravityCliAdapter::with_path(fake_agy_path);

    // 1. Move to BUILDING
    {
        let mut db = db_manager.lock().await;
        apply_workflow_action(
            db.connection_mut(),
            &project_id,
            WorkflowAction::StartBuild,
            "HUMAN",
        )
        .unwrap();
    }

    // 2. Run post-build validation: fails because target/clean.txt does not exist, leaving workflow in VALIDATING
    let val_run = ValidationService::run_post_build_validation(
        db_manager.clone(),
        val_registry.clone(),
        &project_id,
        repo_path,
        None,
        app_data_dir,
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(
        val_run.status,
        coalition_lib::core::validation::ValidationRunStatus::Fail
    );
    assert!(!val_run.is_gate_passed);

    {
        let db = db_manager.lock().await;
        let wf = coalition_lib::core::workflow::get_workflow_state(db.connection(), &project_id)
            .unwrap();
        assert_eq!(wf.state, WorkflowState::Validating);
    }

    // 3. Route diagnostics: legal transition VALIDATING -> CORRECTIONS_REQUIRED via RequestCorrections
    {
        let mut db = db_manager.lock().await;
        apply_workflow_action(
            db.connection_mut(),
            &project_id,
            WorkflowAction::RequestCorrections,
            "HUMAN",
        )
        .unwrap();

        let wf = coalition_lib::core::workflow::get_workflow_state(db.connection(), &project_id)
            .unwrap();
        assert_eq!(wf.state, WorkflowState::CorrectionsRequired);
    }

    // 4. Start Builder turn with BuilderInstructionSource::ValidationDiagnostic
    let turn_resp = coalition_lib::core::builder::BuilderService::start_governed_turn_with_source(
        db_manager.clone(),
        builder_registry.clone(),
        None,
        &project_id,
        Some("gemini-3.8-flash-high".to_string()),
        Some("low".to_string()),
        Some(adapter),
        Some(
            coalition_lib::core::builder::BuilderInstructionSource::ValidationDiagnostic {
                validation_run_id: val_run.run_id.clone(),
            },
        ),
        None, // None so post-build doesn't auto-run inside this unit check
        None,
    )
    .await
    .expect("Builder diagnostic turn should start and complete successfully");

    assert_eq!(turn_resp.status, "SUCCESS");

    // Verify workflow state transitioned CORRECTIONS_REQUIRED -> BUILDING
    {
        let db = db_manager.lock().await;
        let wf = coalition_lib::core::workflow::get_workflow_state(db.connection(), &project_id)
            .unwrap();
        assert_eq!(
            wf.state,
            WorkflowState::Building,
            "Builder turn must transition project back to BUILDING"
        );

        // Verify the prompt recorded in the session includes the validation diagnostic report
        let latest_session = db.get_latest_builder_session(&project_id).unwrap().unwrap();
        assert!(
            latest_session
                .prompt
                .contains("=== VALIDATION DIAGNOSTIC REPORT ==="),
            "Builder prompt must contain sanitized validation diagnostic report"
        );
        assert!(
            latest_session.prompt.contains(&val_run.run_id),
            "Builder prompt must reference the validation run ID"
        );
    }
}

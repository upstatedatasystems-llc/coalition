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

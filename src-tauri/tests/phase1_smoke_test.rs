use coalition_lib::core::activity::ActivityManager;
use coalition_lib::core::artifacts::{ArchitectureState, ArtifactManager};
use coalition_lib::core::git::GitAdapter;
use coalition_lib::core::projects::ProjectService;
use coalition_lib::core::workflow::{apply_workflow_action, WorkflowAction, WorkflowState};
use coalition_lib::db::DbManager;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_phase1_complete_desktop_lifecycle_smoke() {
    // 1. Start from a clean fixture git repository
    let repo_temp = tempdir().unwrap();
    let repo_path = repo_temp.path();

    Command::new("git")
        .args(["init"])
        .current_dir(repo_path)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "Smoke Tester"])
        .current_dir(repo_path)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "smoke@test.local"])
        .current_dir(repo_path)
        .output()
        .unwrap();

    let fixture_file = repo_path.join("fixture.txt");
    fs::write(&fixture_file, "initial content\n").unwrap();
    Command::new("git")
        .args(["add", "fixture.txt"])
        .current_dir(repo_path)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "initial commit"])
        .current_dir(repo_path)
        .output()
        .unwrap();

    // Dedicated database file on disk to simulate app restart & SQLite deletion
    let db_temp = tempdir().unwrap();
    let db_path = db_temp.path().join("coalition_smoke.db");

    let git = GitAdapter::new().unwrap();
    let durable_project_id: String;

    // --- STEP 2 & 3: Register repository & verify layout validation and project.yaml ---
    {
        let mut db = DbManager::open(&db_path).unwrap();
        db.run_migrations().unwrap();

        // Verify initially empty
        let initial_projects = ProjectService::list_projects(db.connection(), Some(&git)).unwrap();
        assert_eq!(initial_projects.len(), 0, "No projects on brand new launch");

        // 2. Register the repository in Coalition
        let details =
            ProjectService::register_or_open_project(db.connection_mut(), &git, repo_path).unwrap();
        durable_project_id = details.project.project_id.clone();

        // 3. Verify .coalition layout validation passes and project.yaml is created
        assert!(repo_path.join(".coalition").is_dir());
        assert!(repo_path.join(".coalition").join("project.yaml").is_file());
        ArtifactManager::validate_coalition_layout(repo_path, &repo_path.join(".coalition"))
            .expect("Layout validation must pass");

        let art = details.artifact.expect("Durable artifact must be present");
        assert_eq!(art.schema_version, 1);
        assert_eq!(art.architecture_state, ArchitectureState::Draft);
        assert_eq!(details.workflow_state.state, WorkflowState::Draft);
        assert!(details.is_available);

        // Commit .coalition to establish clean baseline
        Command::new("git")
            .args(["add", ".coalition"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "commit coalition artifacts"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        // 4. Inspect live git status (clean repo fixture)
        let clean_details =
            ProjectService::get_project_details(db.connection(), Some(&git), &durable_project_id)
                .unwrap();
        let clean_git = clean_details.git.expect("git state present");
        assert!(
            clean_git.status.is_clean,
            "Repository should be clean after committing .coalition"
        );
        assert_eq!(clean_git.status.untracked, 0);
        assert_eq!(clean_git.status.unstaged, 0);

        // 5. Modify a file in the repository and verify live git status updates to dirty without polling loops
        fs::write(&fixture_file, "modified content for dirty state test\n").unwrap();
        let dirty_details =
            ProjectService::get_project_details(db.connection(), Some(&git), &durable_project_id)
                .unwrap();
        let dirty_git = dirty_details.git.expect("dirty git state present");
        assert!(
            !dirty_git.status.is_clean,
            "Working tree must reflect dirty state immediately"
        );
        assert_eq!(dirty_git.status.unstaged, 1);

        // 6. Initiate transition DRAFT -> ARCHITECTING and verify operational state, activity event, SQLite persistence
        let wf_rec = apply_workflow_action(
            db.connection_mut(),
            &durable_project_id,
            WorkflowAction::StartArchitecting,
            "HUMAN",
        )
        .unwrap();
        assert_eq!(wf_rec.state, WorkflowState::Architecting);

        let events =
            ActivityManager::get_project_activity(db.connection(), &durable_project_id, Some(10))
                .unwrap();
        assert!(
            events.iter().any(|e| e.event_type == "WORKFLOW_TRANSITION"),
            "Activity event WORKFLOW_TRANSITION must be recorded"
        );

        // Commit dirty change to clean repo for restart checks
        Command::new("git")
            .args(["add", "fixture.txt"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "commit fixture modification"])
            .current_dir(repo_path)
            .output()
            .unwrap();
    } // --- CLOSE COALITION (Simulated app shutdown) ---

    // --- STEP 7: Reconnect to SQLite store and verify persistence & live git ---
    {
        let mut db = DbManager::open(&db_path).unwrap();
        db.run_migrations().unwrap();

        // Verify project list recovered
        let projects = ProjectService::list_projects(db.connection(), Some(&git)).unwrap();
        assert_eq!(projects.len(), 1, "Project must persist across restart");
        assert_eq!(projects[0].project_id, durable_project_id);
        assert!(projects[0].is_available);
        assert_eq!(
            projects[0].workflow_state,
            Some(WorkflowState::Architecting)
        );

        // Verify last-opened project tracked
        let last_opened_id =
            ProjectService::get_app_setting(db.connection(), "last_opened_project_id")
                .unwrap()
                .expect("last opened project preserved");
        assert_eq!(last_opened_id, durable_project_id);

        let details =
            ProjectService::get_project_details(db.connection(), Some(&git), &last_opened_id)
                .unwrap();
        assert_eq!(details.project.project_id, durable_project_id);
        assert!(details.git.as_ref().unwrap().status.is_clean);
    }

    // --- STEP 8: Simulate repository path unavailable on disk ---
    {
        let mut db = DbManager::open(&db_path).unwrap();
        db.run_migrations().unwrap();

        // Create a temporary project that we will delete to make unavailable
        let unavail_temp = tempdir().unwrap();
        let unavail_path = unavail_temp.path();
        Command::new("git")
            .args(["init"])
            .current_dir(unavail_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Tester"])
            .current_dir(unavail_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@test.local"])
            .current_dir(unavail_path)
            .output()
            .unwrap();
        fs::write(unavail_path.join("init.txt"), "hello").unwrap();
        Command::new("git")
            .args(["add", "init.txt"])
            .current_dir(unavail_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(unavail_path)
            .output()
            .unwrap();

        let unavail_details =
            ProjectService::register_or_open_project(db.connection_mut(), &git, unavail_path)
                .unwrap();
        let unavail_pid = unavail_details.project.project_id;

        // Delete the repository from disk
        drop(unavail_temp);

        // Verify project list marks it unavailable without deletion
        let projects = ProjectService::list_projects(db.connection(), Some(&git)).unwrap();
        let found = projects
            .iter()
            .find(|p| p.project_id == unavail_pid)
            .expect("Must retain project");
        assert!(!found.is_available, "Must be marked unavailable");

        // Detail view must NOT report fake Draft contract
        let details =
            ProjectService::get_project_details(db.connection(), Some(&git), &unavail_pid).unwrap();
        assert!(!details.is_available);
        assert!(
            details.artifact.is_none(),
            "Durable artifact must be None when repository is unavailable (never fabricate Draft)"
        );
    }

    // --- STEP 9: Delete SQLite database file entirely, restart, reopen repository path ---
    {
        // Drop database file completely
        let _ = fs::remove_file(&db_path);
        assert!(!db_path.exists());

        // Reconnect with fresh DB
        let mut fresh_db = DbManager::open(&db_path).unwrap();
        fresh_db.run_migrations().unwrap();

        // Initially 0 projects
        let list = ProjectService::list_projects(fresh_db.connection(), Some(&git)).unwrap();
        assert_eq!(list.len(), 0);

        // Reopen repository
        let rehydrated =
            ProjectService::register_or_open_project(fresh_db.connection_mut(), &git, repo_path)
                .unwrap();
        assert_eq!(rehydrated.project.project_id, durable_project_id);

        // Verify activity log has PROJECT_REHYDRATED
        let activity = ActivityManager::get_project_activity(
            fresh_db.connection(),
            &durable_project_id,
            Some(10),
        )
        .unwrap();
        assert!(
            activity
                .iter()
                .any(|e| e.event_type == "PROJECT_REHYDRATED"),
            "Rehydration must be logged as PROJECT_REHYDRATED"
        );
    }

    // --- STEP 10: Safe project.yaml replacement with frozen architecture state and version ---
    {
        let mut db = DbManager::open(&db_path).unwrap();
        db.run_migrations().unwrap();

        let project_yaml_path = repo_path.join(".coalition").join("project.yaml");
        let mut yaml = ArtifactManager::read_project_yaml(&project_yaml_path).unwrap();
        yaml.architecture_state = ArchitectureState::Frozen;
        yaml.current_architecture_version = Some("1.0.0".to_string());

        // Safe atomic write
        ArtifactManager::write_project_yaml_atomic(&project_yaml_path, &yaml).unwrap();

        // Read back and validate
        let read_back = ArtifactManager::read_project_yaml(&project_yaml_path).unwrap();
        assert_eq!(read_back.architecture_state, ArchitectureState::Frozen);
        assert_eq!(
            read_back.current_architecture_version,
            Some("1.0.0".to_string())
        );

        // Delete DB once more and verify rehydration preserves frozen state
        drop(db);
        let _ = fs::remove_file(&db_path);

        let mut fresh_db2 = DbManager::open(&db_path).unwrap();
        fresh_db2.run_migrations().unwrap();

        let frozen_rehydrated =
            ProjectService::register_or_open_project(fresh_db2.connection_mut(), &git, repo_path)
                .unwrap();
        assert_eq!(
            frozen_rehydrated.workflow_state.state,
            WorkflowState::Frozen,
            "Rehydration must preserve frozen durable state"
        );
        assert_eq!(
            frozen_rehydrated
                .artifact
                .unwrap()
                .current_architecture_version,
            Some("1.0.0".to_string())
        );
    }
}

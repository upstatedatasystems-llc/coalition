use coalition_lib::core::git::GitAdapter;
use coalition_lib::core::projects::ProjectService;
use coalition_lib::core::workflow::WorkflowState;
use coalition_lib::db::DbManager;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_phase1_complete_desktop_lifecycle_smoke() {
    // 1. Disposable test git repository
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

    // Dedicated database file on disk to simulate app restart
    let db_temp = tempdir().unwrap();
    let db_path = db_temp.path().join("coalition_smoke.db");

    let git = GitAdapter::new().unwrap();
    let durable_project_id: String;

    // --- FIRST APP RUN (Launch Coalition) ---
    {
        let mut db = DbManager::open(&db_path).unwrap();
        db.run_migrations().unwrap();

        // Verify initially empty
        let initial_projects = ProjectService::list_projects(db.connection(), Some(&git)).unwrap();
        assert_eq!(initial_projects.len(), 0, "No projects on brand new launch");

        // Open repository
        let details =
            ProjectService::register_or_open_project(db.connection_mut(), &git, repo_path).unwrap();
        durable_project_id = details.project.project_id.clone();

        // Assert .coalition created
        assert!(repo_path.join(".coalition").is_dir());
        assert!(repo_path.join(".coalition").join("project.yaml").is_file());

        // Assert contract values
        assert_eq!(details.artifact.schema_version, 1);
        assert_eq!(details.workflow_state.state, WorkflowState::Draft);
        assert!(details.is_available);

        // Assert git state accurately detects the newly created .coalition directory as untracked
        let git_state = details.git.expect("git state present");
        assert!(git_state.is_repo);
        assert_eq!(
            git_state.status.untracked, 1,
            ".coalition should be untracked"
        );

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

        let clean_git = git.inspect_repo(repo_path).unwrap();
        assert!(
            clean_git.status.is_clean,
            "Repository should be clean after committing .coalition"
        );

        // Assert project appears in list
        let projects = ProjectService::list_projects(db.connection(), Some(&git)).unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].project_id, durable_project_id);

        // Assert last_opened_project_id tracked
        let last_opened =
            ProjectService::get_app_setting(db.connection(), "last_opened_project_id").unwrap();
        assert_eq!(last_opened, Some(durable_project_id.clone()));
    } // --- CLOSE COALITION (DB connection closed) ---

    // --- SECOND APP RUN (Relaunch Coalition) ---
    {
        let mut db = DbManager::open(&db_path).unwrap();
        db.run_migrations().unwrap();

        // Assert project list recovered from SQLite
        let projects = ProjectService::list_projects(db.connection(), Some(&git)).unwrap();
        assert_eq!(projects.len(), 1, "Project must persist across restart");
        assert_eq!(projects[0].project_id, durable_project_id);
        assert!(projects[0].is_available);
        assert_eq!(projects[0].workflow_state, Some(WorkflowState::Draft));

        // Restore last opened project
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

        // Modify fixture file in the repo
        fs::write(&fixture_file, "modified content for dirty state test\n").unwrap();

        // Refresh git state
        let refreshed =
            ProjectService::get_project_details(db.connection(), Some(&git), &last_opened_id)
                .unwrap();
        let refreshed_git = refreshed.git.expect("refreshed git state");
        assert!(
            !refreshed_git.status.is_clean,
            "Working tree must now be dirty"
        );
        assert_eq!(refreshed_git.status.unstaged, 1);
    }
}

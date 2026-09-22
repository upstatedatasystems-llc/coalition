use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GitError {
    #[error("Git executable not found: {0}")]
    NotFound(String),
    #[error("Path is not a Git repository: {0}")]
    NotAGitRepository(String),
    #[error("Git command failed: {0}")]
    ExecutionFailed(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitStatusCounts {
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
    pub is_clean: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitDetailedDirtyState {
    pub is_clean: bool,
    pub staged_count: usize,
    pub unstaged_count: usize,
    pub untracked_count: usize,
    pub raw_porcelain: String,
    pub unstaged_diff_hash: String,
    pub staged_diff_hash: String,
    pub untracked_fingerprint: String,
    pub composite_fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitRepoInfo {
    pub git_version: String,
    pub is_repo: bool,
    pub root_dir: Option<String>,
    pub current_branch: Option<String>,
    pub is_detached: bool,
    pub head_commit: Option<String>,
    pub status: GitStatusCounts,
    pub diff_summary: String,
}

pub struct GitAdapter {
    git_bin: PathBuf,
}

impl GitAdapter {
    pub fn new() -> Result<Self, GitError> {
        let output = Command::new("git")
            .arg("--version")
            .output()
            .map_err(|e| GitError::NotFound(format!("Failed to find git binary on PATH: {}", e)))?;

        if !output.status.success() {
            return Err(GitError::NotFound("Git command check failed".to_string()));
        }

        Ok(Self {
            git_bin: PathBuf::from("git"),
        })
    }

    pub fn get_version(&self) -> Result<String, GitError> {
        let output = Command::new(&self.git_bin).arg("--version").output()?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            Err(GitError::ExecutionFailed(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ))
        }
    }

    /// Resolves the canonical repository root for a given directory.
    /// Returns GitError::NotAGitRepository if the directory is not inside a git repository.
    pub fn resolve_repo_root<P: AsRef<Path>>(&self, working_dir: P) -> Result<PathBuf, GitError> {
        let dir = working_dir.as_ref();
        if !dir.exists() {
            return Err(GitError::NotAGitRepository(format!(
                "Directory does not exist: {:?}",
                dir
            )));
        }

        let output = Command::new(&self.git_bin)
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(dir)
            .output()?;

        if !output.status.success() {
            return Err(GitError::NotAGitRepository(format!(
                "Directory {:?} is not a Git repository",
                dir
            )));
        }

        let raw_root = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let canonical_root = PathBuf::from(raw_root).canonicalize()?;
        Ok(canonical_root)
    }

    pub fn inspect_repo<P: AsRef<Path>>(&self, working_dir: P) -> Result<GitRepoInfo, GitError> {
        let version = self.get_version()?;
        let dir = working_dir.as_ref();

        let is_repo_res = Command::new(&self.git_bin)
            .args(["rev-parse", "--is-inside-work-tree"])
            .current_dir(dir)
            .output();

        let is_repo = match is_repo_res {
            Ok(out) => {
                out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "true"
            }
            Err(_) => false,
        };

        if !is_repo {
            return Ok(GitRepoInfo {
                git_version: version,
                is_repo: false,
                root_dir: None,
                current_branch: None,
                is_detached: false,
                head_commit: None,
                status: GitStatusCounts {
                    staged: 0,
                    unstaged: 0,
                    untracked: 0,
                    is_clean: true,
                },
                diff_summary: String::new(),
            });
        }

        let root_dir = Command::new(&self.git_bin)
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(dir)
            .output()
            .ok()
            .and_then(|out| {
                if out.status.success() {
                    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    PathBuf::from(s)
                        .canonicalize()
                        .ok()
                        .map(|p| p.to_string_lossy().to_string())
                } else {
                    None
                }
            });

        let branch_output = Command::new(&self.git_bin)
            .args(["branch", "--show-current"])
            .current_dir(dir)
            .output()
            .ok();

        let current_branch = branch_output.and_then(|out| {
            if out.status.success() {
                let b = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if b.is_empty() {
                    None
                } else {
                    Some(b)
                }
            } else {
                None
            }
        });

        // Safely inspect HEAD: in an empty repository with 0 commits, rev-parse HEAD exits non-zero
        let head_commit = Command::new(&self.git_bin)
            .args(["rev-parse", "HEAD"])
            .current_dir(dir)
            .output()
            .ok()
            .and_then(|out| {
                if out.status.success() {
                    let commit = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if commit.is_empty() {
                        None
                    } else {
                        Some(commit)
                    }
                } else {
                    None
                }
            });

        let is_detached = current_branch.is_none() && head_commit.is_some();

        let status_output = Command::new(&self.git_bin)
            .args(["status", "--porcelain"])
            .current_dir(dir)
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).to_string())
            .unwrap_or_default();

        let status_counts = Self::parse_porcelain_status(&status_output);

        let diff_summary = Command::new(&self.git_bin)
            .args(["diff", "--stat"])
            .current_dir(dir)
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
            .unwrap_or_default();

        Ok(GitRepoInfo {
            git_version: version,
            is_repo: true,
            root_dir,
            current_branch,
            is_detached,
            head_commit,
            status: status_counts,
            diff_summary,
        })
    }

    pub fn parse_porcelain_status(status_str: &str) -> GitStatusCounts {
        let mut staged = 0;
        let mut unstaged = 0;
        let mut untracked = 0;

        for line in status_str.lines() {
            if line.len() < 2 {
                continue;
            }
            let index_char = line.chars().next().unwrap_or(' ');
            let worktree_char = line.chars().nth(1).unwrap_or(' ');

            if index_char == '?' && worktree_char == '?' {
                untracked += 1;
            } else {
                if index_char != ' ' && index_char != '?' {
                    staged += 1;
                }
                if worktree_char != ' ' && worktree_char != '?' {
                    unstaged += 1;
                }
            }
        }

        let is_clean = staged == 0 && unstaged == 0 && untracked == 0;

        GitStatusCounts {
            staged,
            unstaged,
            untracked,
            is_clean,
        }
    }

    /// Captures an authoritative, multi-factor, deterministic working-tree fingerprint.
    /// Incorporates:
    /// - Full raw porcelain status with path details (-uall).
    /// - Unstaged diff content identity (SHA-256).
    /// - Staged / cached diff content identity (SHA-256).
    /// - Untracked file path + content identities (SHA-256).
    pub fn compute_detailed_dirty_state<P: AsRef<Path>>(
        &self,
        working_dir: P,
    ) -> Result<GitDetailedDirtyState, GitError> {
        let dir = working_dir.as_ref();

        // 1. Porcelain status with all untracked individual files listed (-uall)
        let status_output = Command::new(&self.git_bin)
            .args(["status", "--porcelain=v1", "-uall"])
            .current_dir(dir)
            .output()?;

        let raw_porcelain = String::from_utf8_lossy(&status_output.stdout).to_string();
        let counts = Self::parse_porcelain_status(&raw_porcelain);

        // 2. Unstaged diff hash
        let unstaged_diff = Command::new(&self.git_bin)
            .args(["diff"])
            .current_dir(dir)
            .output()?;
        let mut hasher = Sha256::new();
        hasher.update(&unstaged_diff.stdout);
        let unstaged_diff_hash = format!("{:x}", hasher.finalize());

        // 3. Staged / cached diff hash
        let staged_diff = Command::new(&self.git_bin)
            .args(["diff", "--cached"])
            .current_dir(dir)
            .output()?;
        let mut hasher = Sha256::new();
        hasher.update(&staged_diff.stdout);
        let staged_diff_hash = format!("{:x}", hasher.finalize());

        // 4. Untracked files path + content fingerprints
        let mut untracked_entries = Vec::new();
        for line in raw_porcelain.lines() {
            if let Some(rel_path_raw) = line.strip_prefix("?? ") {
                let rel_path = rel_path_raw.trim().trim_matches('"');
                let full_path = dir.join(rel_path);
                if full_path.is_file() {
                    let content_bytes = std::fs::read(&full_path).unwrap_or_default();
                    let mut file_hasher = Sha256::new();
                    file_hasher.update(&content_bytes);
                    let file_hash = format!("{:x}", file_hasher.finalize());
                    untracked_entries.push(format!("{}:{}", rel_path, file_hash));
                } else {
                    untracked_entries.push(format!("{}:exists", rel_path));
                }
            }
        }
        untracked_entries.sort();
        let untracked_joined = untracked_entries.join("\n");
        let mut hasher = Sha256::new();
        hasher.update(untracked_joined.as_bytes());
        let untracked_fingerprint = format!("{:x}", hasher.finalize());

        // 5. Composite dirty fingerprint
        let composite_fingerprint = if counts.is_clean {
            "CLEAN".to_string()
        } else {
            let composite_input = format!(
                "porcelain:\n{}\nunstaged:{}\nstaged:{}\nuntracked:{}",
                raw_porcelain.trim(),
                unstaged_diff_hash,
                staged_diff_hash,
                untracked_fingerprint
            );
            let mut hasher = Sha256::new();
            hasher.update(composite_input.as_bytes());
            format!("{:x}", hasher.finalize())
        };

        Ok(GitDetailedDirtyState {
            is_clean: counts.is_clean,
            staged_count: counts.staged,
            unstaged_count: counts.unstaged,
            untracked_count: counts.untracked,
            raw_porcelain,
            unstaged_diff_hash,
            staged_diff_hash,
            untracked_fingerprint,
            composite_fingerprint,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_parse_porcelain_status() {
        let porcelain = "M  src/main.rs\n M Cargo.toml\n?? new_file.txt\nA  added.rs\n";
        let counts = GitAdapter::parse_porcelain_status(porcelain);
        assert_eq!(counts.staged, 2); // 'M ' and 'A '
        assert_eq!(counts.unstaged, 1); // ' M'
        assert_eq!(counts.untracked, 1); // '??'
        assert!(!counts.is_clean);

        let clean_counts = GitAdapter::parse_porcelain_status("");
        assert!(clean_counts.is_clean);
    }

    #[test]
    fn test_git_adapter_detection() {
        let adapter = GitAdapter::new();
        assert!(
            adapter.is_ok(),
            "System git must be detected in environment"
        );
        let version = adapter.unwrap().get_version().unwrap();
        assert!(version.contains("git version"));
    }

    #[test]
    fn test_git_adapter_repo_operations() {
        let adapter = GitAdapter::new().unwrap();
        let dir = tempdir().unwrap();
        let repo_path = dir.path();

        // 1. Non-git directory
        let err = adapter.resolve_repo_root(repo_path).unwrap_err();
        assert!(matches!(err, GitError::NotAGitRepository(_)));

        // Initialize git repository with local config (no global dependency)
        Command::new("git")
            .args(["init"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        // 2. Empty repository (0 commits)
        let resolved = adapter.resolve_repo_root(repo_path).unwrap();
        assert_eq!(resolved, repo_path.canonicalize().unwrap());

        let info = adapter.inspect_repo(repo_path).unwrap();
        assert!(info.is_repo);
        assert!(
            info.head_commit.is_none(),
            "Empty repo should have no HEAD commit"
        );
        assert!(info.status.is_clean);

        // 3. Untracked file
        fs::write(repo_path.join("file1.txt"), "hello").unwrap();
        let info = adapter.inspect_repo(repo_path).unwrap();
        assert_eq!(info.status.untracked, 1);
        assert!(!info.status.is_clean);

        // 4. Staged and committed file
        Command::new("git")
            .args(["add", "file1.txt"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        let info = adapter.inspect_repo(repo_path).unwrap();
        assert_eq!(info.status.staged, 1);

        Command::new("git")
            .args(["commit", "-m", "initial commit"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        let info = adapter.inspect_repo(repo_path).unwrap();
        assert!(info.head_commit.is_some());
        assert!(info.status.is_clean);
        assert!(
            info.current_branch == Some("main".to_string())
                || info.current_branch == Some("master".to_string())
        );

        // 5. Modified tracked file
        fs::write(repo_path.join("file1.txt"), "hello modified").unwrap();
        let info = adapter.inspect_repo(repo_path).unwrap();
        assert_eq!(info.status.unstaged, 1);
        assert!(!info.status.is_clean);

        // 6. Path with spaces
        let space_dir = tempdir().unwrap();
        let space_repo = space_dir.path().join("path with spaces");
        fs::create_dir_all(&space_repo).unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(&space_repo)
            .output()
            .unwrap();
        let space_resolved = adapter.resolve_repo_root(&space_repo).unwrap();
        assert_eq!(space_resolved, space_repo.canonicalize().unwrap());
    }

    #[test]
    fn test_compute_detailed_dirty_state_untracked_and_diff_changes() {
        let adapter = GitAdapter::new().unwrap();
        let dir = tempdir().unwrap();
        let repo_path = dir.path();

        Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        // 1. Initial commit
        fs::write(repo_path.join("committed.txt"), "v1").unwrap();
        Command::new("git")
            .args(["add", "committed.txt"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(repo_path)
            .output()
            .unwrap();

        let clean_state = adapter.compute_detailed_dirty_state(repo_path).unwrap();
        assert!(clean_state.is_clean);
        assert_eq!(clean_state.composite_fingerprint, "CLEAN");

        // 2. Untracked file added
        let untracked_path = repo_path.join("untracked.txt");
        fs::write(&untracked_path, "initial untracked content").unwrap();

        let state_untracked1 = adapter.compute_detailed_dirty_state(repo_path).unwrap();
        assert!(!state_untracked1.is_clean);
        assert_eq!(state_untracked1.untracked_count, 1);
        assert_ne!(state_untracked1.composite_fingerprint, "CLEAN");

        // 3. Untracked file content changed (same filename, different content)
        fs::write(&untracked_path, "modified untracked content").unwrap();
        let state_untracked2 = adapter.compute_detailed_dirty_state(repo_path).unwrap();
        assert_ne!(
            state_untracked1.composite_fingerprint, state_untracked2.composite_fingerprint,
            "Changing content of an untracked file must produce a different composite fingerprint"
        );

        // 4. Tracked file unstaged edit
        fs::write(repo_path.join("committed.txt"), "v2 unstaged").unwrap();
        let state_unstaged = adapter.compute_detailed_dirty_state(repo_path).unwrap();
        assert_eq!(state_unstaged.unstaged_count, 1);
        assert_ne!(
            state_unstaged.composite_fingerprint,
            state_untracked2.composite_fingerprint
        );

        // 5. Staged file edit
        Command::new("git")
            .args(["add", "committed.txt"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        let state_staged = adapter.compute_detailed_dirty_state(repo_path).unwrap();
        assert_eq!(state_staged.staged_count, 1);
        assert_ne!(
            state_staged.composite_fingerprint,
            state_unstaged.composite_fingerprint
        );
    }
}

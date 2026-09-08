use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GitError {
    #[error("Git executable not found: {0}")]
    NotFound(String),
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitRepoInfo {
    pub git_version: String,
    pub is_repo: bool,
    pub root_dir: Option<String>,
    pub current_branch: Option<String>,
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
                head_commit: None,
                status: GitStatusCounts {
                    staged: 0,
                    unstaged: 0,
                    untracked: 0,
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
                    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
                } else {
                    None
                }
            });

        let current_branch = Command::new(&self.git_bin)
            .args(["branch", "--show-current"])
            .current_dir(dir)
            .output()
            .ok()
            .and_then(|out| {
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

        let head_commit = Command::new(&self.git_bin)
            .args(["rev-parse", "HEAD"])
            .current_dir(dir)
            .output()
            .ok()
            .and_then(|out| {
                if out.status.success() {
                    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
                } else {
                    None
                }
            });

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

        GitStatusCounts {
            staged,
            unstaged,
            untracked,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_porcelain_status() {
        let porcelain = "M  src/main.rs\n M Cargo.toml\n?? new_file.txt\nA  added.rs\n";
        let counts = GitAdapter::parse_porcelain_status(porcelain);
        assert_eq!(counts.staged, 2); // 'M ' and 'A '
        assert_eq!(counts.unstaged, 1); // ' M'
        assert_eq!(counts.untracked, 1); // '??'
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
}

use crate::core::artifacts::ArtifactManager;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const REQUIRED_ARCHITECTURE_ARTIFACTS: &[(&str, &str)] = &[
    ("design/product-vision.md", "Product Vision"),
    ("design/requirements.md", "Requirements"),
    ("design/architecture.md", "Architecture"),
    ("design/constraints.md", "Constraints"),
    ("design/interfaces.md", "Interfaces"),
    ("design/security.md", "Security"),
    (
        "implementation/implementation-plan.md",
        "Implementation Plan",
    ),
    (
        "implementation/acceptance-criteria.yaml",
        "Acceptance Criteria",
    ),
    ("implementation/test-plan.md", "Test Plan"),
];

pub const READINESS_CHARACTER_THRESHOLD: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ArtifactReadinessStatus {
    Missing,
    Incomplete,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OverallReadiness {
    Incomplete,
    ReadyToFreeze,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactReadinessItem {
    pub path: String,
    pub title: String,
    pub status: ArtifactReadinessStatus,
    pub character_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadinessReport {
    pub overall_readiness: OverallReadiness,
    pub ready_count: usize,
    pub total_required: usize,
    pub artifacts: Vec<ArtifactReadinessItem>,
    pub has_open_questions: bool,
}

pub struct ReadinessEvaluator;

impl ReadinessEvaluator {
    /// Evaluates the current durable artifacts in `.coalition/` and computes readiness.
    pub fn evaluate<P: AsRef<Path>>(repo_root: P) -> ReadinessReport {
        let root = repo_root.as_ref();
        let mut ready_count = 0;
        let mut items = Vec::new();

        for &(path, title) in REQUIRED_ARCHITECTURE_ARTIFACTS {
            let content = ArtifactManager::read_artifact(root, path).ok().flatten();

            let (status, char_count) = match content {
                Some(text) => {
                    let trimmed_len = text.trim().len();
                    if trimmed_len >= READINESS_CHARACTER_THRESHOLD {
                        ready_count += 1;
                        (ArtifactReadinessStatus::Ready, trimmed_len)
                    } else {
                        (ArtifactReadinessStatus::Incomplete, trimmed_len)
                    }
                }
                None => (ArtifactReadinessStatus::Missing, 0),
            };

            items.push(ArtifactReadinessItem {
                path: path.to_string(),
                title: title.to_string(),
                status,
                character_count: char_count,
            });
        }

        let has_open_questions = ArtifactManager::read_artifact(root, "design/open-questions.md")
            .ok()
            .flatten()
            .map(|t| !t.trim().is_empty())
            .unwrap_or(false);

        let overall_readiness = if ready_count == REQUIRED_ARCHITECTURE_ARTIFACTS.len() {
            OverallReadiness::ReadyToFreeze
        } else {
            OverallReadiness::Incomplete
        };

        ReadinessReport {
            overall_readiness,
            ready_count,
            total_required: REQUIRED_ARCHITECTURE_ARTIFACTS.len(),
            artifacts: items,
            has_open_questions,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_readiness_initially_missing() {
        let dir = tempfile::tempdir().unwrap();
        ArtifactManager::initialize_new_project(dir.path(), "test-readiness").unwrap();

        let report = ReadinessEvaluator::evaluate(dir.path());
        assert_eq!(report.overall_readiness, OverallReadiness::Incomplete);
        assert_eq!(report.ready_count, 0);
        assert_eq!(report.total_required, 9);
        assert_eq!(report.artifacts.len(), 9);
        for item in &report.artifacts {
            assert_eq!(item.status, ArtifactReadinessStatus::Missing);
        }
    }

    #[test]
    fn test_readiness_incomplete_when_trivial_content() {
        let dir = tempfile::tempdir().unwrap();
        ArtifactManager::initialize_new_project(dir.path(), "test-readiness-trivial").unwrap();

        ArtifactManager::write_artifact_atomic(
            dir.path(),
            "design/product-vision.md",
            "# Too short",
        )
        .unwrap();

        let report = ReadinessEvaluator::evaluate(dir.path());
        assert_eq!(report.overall_readiness, OverallReadiness::Incomplete);
        assert_eq!(report.ready_count, 0);

        let pv = report
            .artifacts
            .iter()
            .find(|a| a.path == "design/product-vision.md")
            .unwrap();
        assert_eq!(pv.status, ArtifactReadinessStatus::Incomplete);
    }

    #[test]
    fn test_readiness_reaches_ready_to_freeze_when_all_populated() {
        let dir = tempfile::tempdir().unwrap();
        ArtifactManager::initialize_new_project(dir.path(), "test-readiness-complete").unwrap();

        let substantive_content = "# Section\nThis is a substantive section of the architecture contract that exceeds fifty characters.";

        for &(path, _) in REQUIRED_ARCHITECTURE_ARTIFACTS {
            ArtifactManager::write_artifact_atomic(dir.path(), path, substantive_content).unwrap();
        }

        let report = ReadinessEvaluator::evaluate(dir.path());
        assert_eq!(report.overall_readiness, OverallReadiness::ReadyToFreeze);
        assert_eq!(report.ready_count, 9);
        for item in &report.artifacts {
            assert_eq!(item.status, ArtifactReadinessStatus::Ready);
        }
    }
}

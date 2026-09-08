pub use crate::core::artifacts::ArtifactApplicability;
use crate::core::artifacts::ArtifactManager;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const CURRENT_READINESS_POLICY_VERSION: u32 = 1;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ArtifactKind {
    Markdown,
    StructuredYaml,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactReadinessRule {
    pub path: String,
    pub title: String,
    pub kind: ArtifactKind,
    pub applicability: ArtifactApplicability,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadinessPolicy {
    pub policy_version: u32,
    pub rules: Vec<ArtifactReadinessRule>,
}

pub fn default_readiness_policy() -> ReadinessPolicy {
    ReadinessPolicy {
        policy_version: CURRENT_READINESS_POLICY_VERSION,
        rules: vec![
            ArtifactReadinessRule {
                path: "design/product-vision.md".to_string(),
                title: "Product Vision".to_string(),
                kind: ArtifactKind::Markdown,
                applicability: ArtifactApplicability::Required,
            },
            ArtifactReadinessRule {
                path: "design/requirements.md".to_string(),
                title: "Requirements".to_string(),
                kind: ArtifactKind::Markdown,
                applicability: ArtifactApplicability::Required,
            },
            ArtifactReadinessRule {
                path: "design/architecture.md".to_string(),
                title: "Architecture".to_string(),
                kind: ArtifactKind::Markdown,
                applicability: ArtifactApplicability::Required,
            },
            ArtifactReadinessRule {
                path: "design/constraints.md".to_string(),
                title: "Constraints".to_string(),
                kind: ArtifactKind::Markdown,
                applicability: ArtifactApplicability::Required,
            },
            ArtifactReadinessRule {
                path: "design/interfaces.md".to_string(),
                title: "Interfaces".to_string(),
                kind: ArtifactKind::Markdown,
                applicability: ArtifactApplicability::Required,
            },
            ArtifactReadinessRule {
                path: "design/security.md".to_string(),
                title: "Security".to_string(),
                kind: ArtifactKind::Markdown,
                applicability: ArtifactApplicability::Required,
            },
            ArtifactReadinessRule {
                path: "implementation/implementation-plan.md".to_string(),
                title: "Implementation Plan".to_string(),
                kind: ArtifactKind::Markdown,
                applicability: ArtifactApplicability::Required,
            },
            ArtifactReadinessRule {
                path: "implementation/acceptance-criteria.yaml".to_string(),
                title: "Acceptance Criteria".to_string(),
                kind: ArtifactKind::StructuredYaml,
                applicability: ArtifactApplicability::Required,
            },
            ArtifactReadinessRule {
                path: "implementation/test-plan.md".to_string(),
                title: "Test Plan".to_string(),
                kind: ArtifactKind::Markdown,
                applicability: ArtifactApplicability::Required,
            },
            ArtifactReadinessRule {
                path: "design/open-questions.md".to_string(),
                title: "Open Questions".to_string(),
                kind: ArtifactKind::Markdown,
                applicability: ArtifactApplicability::Optional,
            },
        ],
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactReadinessItem {
    pub path: String,
    pub title: String,
    pub applicability: ArtifactApplicability,
    pub status: ArtifactReadinessStatus,
    pub character_count: usize,
    pub details: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadinessReport {
    pub policy_version: u32,
    pub overall_readiness: OverallReadiness,
    pub ready_required_count: usize,
    pub total_required_count: usize,
    pub total_artifacts_count: usize,
    pub artifacts: Vec<ArtifactReadinessItem>,
    pub has_open_questions: bool,
    pub unresolved_open_questions_count: usize,
}

pub struct ReadinessEvaluator;

impl ReadinessEvaluator {
    /// Evaluates the current durable artifacts in `.coalition/` against the project's readiness policy.
    pub fn evaluate<P: AsRef<Path>>(repo_root: P) -> ReadinessReport {
        let root = repo_root.as_ref();
        let mut policy = default_readiness_policy();

        let project_yaml_path = root.join(".coalition").join("project.yaml");
        if let Ok(project) = ArtifactManager::read_project_yaml(&project_yaml_path) {
            if let Some(ref cfg) = project.readiness {
                policy.policy_version = cfg.policy_version;
                for (path, app) in &cfg.applicability {
                    if let Some(rule) = policy.rules.iter_mut().find(|r| r.path == *path) {
                        rule.applicability = *app;
                    }
                }
            }
        }

        Self::evaluate_with_policy(root, &policy)
    }

    /// Sets the durable readiness applicability for an artifact in the project's project.yaml.
    pub fn set_artifact_applicability<P: AsRef<Path>>(
        repo_root: P,
        artifact_path: &str,
        applicability: ArtifactApplicability,
    ) -> Result<ReadinessReport, crate::core::artifacts::ArtifactError> {
        let root = repo_root.as_ref();
        ArtifactManager::update_project_readiness_applicability(
            root,
            artifact_path,
            applicability,
        )?;
        Ok(Self::evaluate(root))
    }

    pub fn evaluate_with_policy<P: AsRef<Path>>(
        repo_root: P,
        policy: &ReadinessPolicy,
    ) -> ReadinessReport {
        let root = repo_root.as_ref();
        let mut ready_required_count = 0;
        let mut total_required_count = 0;
        let mut items = Vec::new();

        for rule in &policy.rules {
            if rule.applicability == ArtifactApplicability::Required {
                total_required_count += 1;
            }

            let content = ArtifactManager::read_artifact(root, &rule.path)
                .ok()
                .flatten();

            let (status, details) = match content.as_deref() {
                None => (
                    ArtifactReadinessStatus::Missing,
                    if rule.applicability == ArtifactApplicability::NotApplicable {
                        Some("Marked NOT_APPLICABLE for this project".to_string())
                    } else {
                        Some("File is absent from .coalition/".to_string())
                    },
                ),
                Some(text) => match rule.kind {
                    ArtifactKind::Markdown => Self::evaluate_markdown(text),
                    ArtifactKind::StructuredYaml => Self::evaluate_structured_yaml(text),
                },
            };

            let char_count = content.as_deref().map(|s| s.trim().len()).unwrap_or(0);

            if rule.applicability == ArtifactApplicability::Required
                && status == ArtifactReadinessStatus::Ready
            {
                ready_required_count += 1;
            }

            items.push(ArtifactReadinessItem {
                path: rule.path.to_string(),
                title: rule.title.to_string(),
                applicability: rule.applicability,
                status,
                character_count: char_count,
                details,
            });
        }

        let (has_open_questions, unresolved_open_questions_count) =
            Self::check_open_questions(root);

        let overall_readiness =
            if total_required_count > 0 && ready_required_count == total_required_count {
                OverallReadiness::ReadyToFreeze
            } else {
                OverallReadiness::Incomplete
            };

        ReadinessReport {
            policy_version: policy.policy_version,
            overall_readiness,
            ready_required_count,
            total_required_count,
            total_artifacts_count: policy.rules.len(),
            artifacts: items,
            has_open_questions,
            unresolved_open_questions_count,
        }
    }

    /// Evaluates markdown substantive completeness.
    /// Requires:
    /// - non-empty
    /// - not solely headings/template boilerplate
    /// - not solely TODO/TBD/placeholder tokens
    fn evaluate_markdown(text: &str) -> (ArtifactReadinessStatus, Option<String>) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return (
                ArtifactReadinessStatus::Incomplete,
                Some("File is empty".to_string()),
            );
        }

        // Strip headings and comments
        let mut substantive_lines = Vec::new();
        for line in trimmed.lines() {
            let l = line.trim();
            if l.is_empty() {
                continue;
            }
            // Markdown heading
            if l.starts_with('#') {
                continue;
            }
            // Markdown horizontal rule
            if l == "---" || l == "***" || l == "___" {
                continue;
            }
            // HTML comment
            if l.starts_with("<!--") && l.ends_with("-->") {
                continue;
            }
            substantive_lines.push(l);
        }

        if substantive_lines.is_empty() {
            return (
                ArtifactReadinessStatus::Incomplete,
                Some("File contains only template headings or separators".to_string()),
            );
        }

        let combined_body = substantive_lines.join(" ");
        let normalized = combined_body
            .to_lowercase()
            .replace(|c: char| !c.is_alphanumeric() && !c.is_whitespace(), " ");

        let words: Vec<&str> = normalized.split_whitespace().collect();
        if words.is_empty() {
            return (
                ArtifactReadinessStatus::Incomplete,
                Some("File has no substantive words".to_string()),
            );
        }

        // Check if words are solely placeholders
        let placeholder_tokens = [
            "todo",
            "tbd",
            "placeholder",
            "tba",
            "wip",
            "draft",
            "none",
            "na",
            "coming",
            "soon",
            "pending",
            "lorem",
            "ipsum",
        ];

        let substantive_words: Vec<&&str> = words
            .iter()
            .filter(|w| !placeholder_tokens.contains(w))
            .collect();

        let substantive_chars: usize = substantive_words.iter().map(|w| w.len()).sum();
        if substantive_chars < 20 {
            return (
                ArtifactReadinessStatus::Incomplete,
                Some("File contains only placeholders or trivial content".to_string()),
            );
        }

        (ArtifactReadinessStatus::Ready, None)
    }

    /// Evaluates structured YAML artifacts.
    /// Requires:
    /// - Parses valid YAML
    /// - Must be a Mapping or Sequence (not a scalar)
    /// - Must contain substantive data, not solely placeholders
    fn evaluate_structured_yaml(text: &str) -> (ArtifactReadinessStatus, Option<String>) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return (
                ArtifactReadinessStatus::Incomplete,
                Some("File is empty".to_string()),
            );
        }

        let parsed: Result<serde_yaml::Value, _> = serde_yaml::from_str(trimmed);
        let val = match parsed {
            Ok(v) => v,
            Err(e) => {
                return (
                    ArtifactReadinessStatus::Incomplete,
                    Some(format!("Malformed YAML syntax: {}", e)),
                );
            }
        };

        match val {
            serde_yaml::Value::Mapping(map) => {
                if map.is_empty() {
                    return (
                        ArtifactReadinessStatus::Incomplete,
                        Some("YAML mapping is empty".to_string()),
                    );
                }
                // Check if all values are null or placeholders
                let mut has_substance = false;
                for (_k, v) in map.iter() {
                    match v {
                        serde_yaml::Value::Null => {}
                        serde_yaml::Value::String(s) => {
                            let s_lower = s.trim().to_lowercase();
                            if s_lower != "todo" && s_lower != "tbd" && !s_lower.is_empty() {
                                has_substance = true;
                                break;
                            }
                        }
                        serde_yaml::Value::Sequence(seq) => {
                            if !seq.is_empty() {
                                has_substance = true;
                                break;
                            }
                        }
                        serde_yaml::Value::Mapping(sub) => {
                            if !sub.is_empty() {
                                has_substance = true;
                                break;
                            }
                        }
                        _ => {
                            has_substance = true;
                            break;
                        }
                    }
                }
                if has_substance {
                    (ArtifactReadinessStatus::Ready, None)
                } else {
                    (
                        ArtifactReadinessStatus::Incomplete,
                        Some("YAML mapping contains only empty or placeholder values".to_string()),
                    )
                }
            }
            serde_yaml::Value::Sequence(seq) => {
                if seq.is_empty() {
                    (
                        ArtifactReadinessStatus::Incomplete,
                        Some("YAML list is empty".to_string()),
                    )
                } else {
                    (ArtifactReadinessStatus::Ready, None)
                }
            }
            _ => (
                ArtifactReadinessStatus::Incomplete,
                Some("YAML root must be a mapping or list".to_string()),
            ),
        }
    }

    /// Evaluates open questions artifact and counts unresolved items.
    fn check_open_questions<P: AsRef<Path>>(root: P) -> (bool, usize) {
        let content = ArtifactManager::read_artifact(root, "design/open-questions.md")
            .ok()
            .flatten();

        match content {
            None => (false, 0),
            Some(text) => {
                let mut unresolved_count = 0;
                for line in text.lines() {
                    let l = line.trim();
                    if l.contains("[OPEN]") || l.contains("- [ ]") || l.contains("status: OPEN") {
                        unresolved_count += 1;
                    }
                }
                let has_any = !text.trim().is_empty();
                (has_any, unresolved_count)
            }
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
        assert_eq!(report.ready_required_count, 0);
        assert_eq!(report.total_required_count, 9);
        assert_eq!(report.total_artifacts_count, 10);
        for item in &report.artifacts {
            assert_eq!(item.status, ArtifactReadinessStatus::Missing);
        }
    }

    #[test]
    fn test_placeholder_only_markdown_is_incomplete() {
        let dir = tempfile::tempdir().unwrap();
        ArtifactManager::initialize_new_project(dir.path(), "test-placeholder").unwrap();

        // 1. Only headings
        ArtifactManager::write_artifact_atomic(
            dir.path(),
            "design/product-vision.md",
            "# Product Vision\n## Goals\n### Strategy",
        )
        .unwrap();

        let report = ReadinessEvaluator::evaluate(dir.path());
        let item = report
            .artifacts
            .iter()
            .find(|a| a.path == "design/product-vision.md")
            .unwrap();
        assert_eq!(item.status, ArtifactReadinessStatus::Incomplete);

        // 2. Headings + TODO / placeholder tokens
        ArtifactManager::write_artifact_atomic(
            dir.path(),
            "design/product-vision.md",
            "# Product Vision\nTODO: write vision here\nTBD\n<!-- draft -->",
        )
        .unwrap();

        let report2 = ReadinessEvaluator::evaluate(dir.path());
        let item2 = report2
            .artifacts
            .iter()
            .find(|a| a.path == "design/product-vision.md")
            .unwrap();
        assert_eq!(item2.status, ArtifactReadinessStatus::Incomplete);
    }

    #[test]
    fn test_required_ready_with_substantive_markdown() {
        let dir = tempfile::tempdir().unwrap();
        ArtifactManager::initialize_new_project(dir.path(), "test-substantive").unwrap();

        ArtifactManager::write_artifact_atomic(
            dir.path(),
            "design/product-vision.md",
            "# Product Vision\nCoalition provides local-first governed development control planes.",
        )
        .unwrap();

        let report = ReadinessEvaluator::evaluate(dir.path());
        let item = report
            .artifacts
            .iter()
            .find(|a| a.path == "design/product-vision.md")
            .unwrap();
        assert_eq!(item.status, ArtifactReadinessStatus::Ready);
        assert_eq!(report.ready_required_count, 1);
    }

    #[test]
    fn test_malformed_yaml_is_incomplete() {
        let dir = tempfile::tempdir().unwrap();
        ArtifactManager::initialize_new_project(dir.path(), "test-malformed-yaml").unwrap();

        ArtifactManager::write_artifact_atomic(
            dir.path(),
            "implementation/acceptance-criteria.yaml",
            ": invalid : yaml :",
        )
        .unwrap();

        let report = ReadinessEvaluator::evaluate(dir.path());
        let item = report
            .artifacts
            .iter()
            .find(|a| a.path == "implementation/acceptance-criteria.yaml")
            .unwrap();
        assert_eq!(item.status, ArtifactReadinessStatus::Incomplete);
        assert!(item.details.as_ref().unwrap().contains("Malformed YAML"));
    }

    #[test]
    fn test_valid_structured_yaml_is_ready() {
        let dir = tempfile::tempdir().unwrap();
        ArtifactManager::initialize_new_project(dir.path(), "test-valid-yaml").unwrap();

        ArtifactManager::write_artifact_atomic(
            dir.path(),
            "implementation/acceptance-criteria.yaml",
            "criteria:\n  - id: AC-01\n    description: Must parse valid YAML\n",
        )
        .unwrap();

        let report = ReadinessEvaluator::evaluate(dir.path());
        let item = report
            .artifacts
            .iter()
            .find(|a| a.path == "implementation/acceptance-criteria.yaml")
            .unwrap();
        assert_eq!(item.status, ArtifactReadinessStatus::Ready);
    }

    #[test]
    fn test_optional_open_questions_does_not_gate_readiness() {
        let dir = tempfile::tempdir().unwrap();
        ArtifactManager::initialize_new_project(dir.path(), "test-optional").unwrap();

        // Populate all 9 required artifacts substantively
        let required_docs = [
            (
                "design/product-vision.md",
                "# Vision\nThis is a substantive vision document for the system.",
            ),
            (
                "design/requirements.md",
                "# Requirements\nREQ-01: System must enforce invariants reliably.",
            ),
            (
                "design/architecture.md",
                "# Architecture\nLayered modular architecture with Rust backend.",
            ),
            (
                "design/constraints.md",
                "# Constraints\nOffline-first operation and bounded local resource usage.",
            ),
            (
                "design/interfaces.md",
                "# Interfaces\nTyped IPC contracts across frontend and machine side.",
            ),
            (
                "design/security.md",
                "# Security\nLeast privilege process management and local safe paths.",
            ),
            (
                "implementation/implementation-plan.md",
                "# Plan\nPhased delivery with deterministic test milestones.",
            ),
            (
                "implementation/acceptance-criteria.yaml",
                "criteria:\n  - id: AC-1\n    name: Verified passes\n",
            ),
            (
                "implementation/test-plan.md",
                "# Test Plan\nDeterministic automated test suite and regression checks.",
            ),
        ];

        for (p, c) in required_docs {
            ArtifactManager::write_artifact_atomic(dir.path(), p, c).unwrap();
        }

        // open-questions.md is MISSING
        let report = ReadinessEvaluator::evaluate(dir.path());
        assert_eq!(report.overall_readiness, OverallReadiness::ReadyToFreeze);
        assert_eq!(report.ready_required_count, 9);
        assert_eq!(report.total_required_count, 9);

        let oq = report
            .artifacts
            .iter()
            .find(|a| a.path == "design/open-questions.md")
            .unwrap();
        assert_eq!(oq.applicability, ArtifactApplicability::Optional);
        assert_eq!(oq.status, ArtifactReadinessStatus::Missing);

        // Populate open-questions.md with an unresolved question
        ArtifactManager::write_artifact_atomic(
            dir.path(),
            "design/open-questions.md",
            "# Open Questions\n- [ ] Q-01: Should we support SQLite WAL mode? [OPEN]\n",
        )
        .unwrap();

        let report2 = ReadinessEvaluator::evaluate(dir.path());
        assert_eq!(report2.overall_readiness, OverallReadiness::ReadyToFreeze);
        assert!(report2.has_open_questions);
        assert_eq!(report2.unresolved_open_questions_count, 1);
    }

    #[test]
    fn test_readiness_regression_after_editing() {
        let dir = tempfile::tempdir().unwrap();
        ArtifactManager::initialize_new_project(dir.path(), "test-regression").unwrap();

        // Initially complete
        let required_docs = [
            (
                "design/product-vision.md",
                "# Vision\nThis is a substantive vision document for the system.",
            ),
            (
                "design/requirements.md",
                "# Requirements\nREQ-01: System must enforce invariants reliably.",
            ),
            (
                "design/architecture.md",
                "# Architecture\nLayered modular architecture with Rust backend.",
            ),
            (
                "design/constraints.md",
                "# Constraints\nOffline-first operation and bounded local resource usage.",
            ),
            (
                "design/interfaces.md",
                "# Interfaces\nTyped IPC contracts across frontend and machine side.",
            ),
            (
                "design/security.md",
                "# Security\nLeast privilege process management and local safe paths.",
            ),
            (
                "implementation/implementation-plan.md",
                "# Plan\nPhased delivery with deterministic test milestones.",
            ),
            (
                "implementation/acceptance-criteria.yaml",
                "criteria:\n  - id: AC-1\n    name: Verified passes\n",
            ),
            (
                "implementation/test-plan.md",
                "# Test Plan\nDeterministic automated test suite and regression checks.",
            ),
        ];

        for (p, c) in required_docs {
            ArtifactManager::write_artifact_atomic(dir.path(), p, c).unwrap();
        }

        let report1 = ReadinessEvaluator::evaluate(dir.path());
        assert_eq!(report1.overall_readiness, OverallReadiness::ReadyToFreeze);

        // Edit one artifact to be a placeholder
        ArtifactManager::write_artifact_atomic(
            dir.path(),
            "design/requirements.md",
            "# Requirements\nTODO: need to rewrite",
        )
        .unwrap();

        let report2 = ReadinessEvaluator::evaluate(dir.path());
        assert_eq!(report2.overall_readiness, OverallReadiness::Incomplete);
        assert_eq!(report2.ready_required_count, 8);
    }

    #[test]
    fn test_project_specific_applicability_override() {
        let dir = tempfile::tempdir().unwrap();
        ArtifactManager::initialize_new_project(dir.path(), "test-applicability").unwrap();

        // Initially total_required_count is 9
        let report0 = ReadinessEvaluator::evaluate(dir.path());
        assert_eq!(report0.total_required_count, 9);

        // Mark test-plan.md as NOT_APPLICABLE and constraints.md as OPTIONAL
        let report1 = ReadinessEvaluator::set_artifact_applicability(
            dir.path(),
            "implementation/test-plan.md",
            ArtifactApplicability::NotApplicable,
        )
        .unwrap();
        assert_eq!(report1.total_required_count, 8);

        let report2 = ReadinessEvaluator::set_artifact_applicability(
            dir.path(),
            "design/constraints.md",
            ArtifactApplicability::Optional,
        )
        .unwrap();
        assert_eq!(report2.total_required_count, 7);

        // Provide content for the 7 remaining required artifacts
        let remaining_required = [
            (
                "design/product-vision.md",
                "# Vision\nThis is a substantive vision document for the system.",
            ),
            (
                "design/requirements.md",
                "# Requirements\nREQ-01: System must enforce invariants reliably.",
            ),
            (
                "design/architecture.md",
                "# Architecture\nLayered modular architecture with Rust backend.",
            ),
            (
                "design/interfaces.md",
                "# Interfaces\nTyped IPC contracts across frontend and machine side.",
            ),
            (
                "design/security.md",
                "# Security\nLeast privilege process management and local safe paths.",
            ),
            (
                "implementation/implementation-plan.md",
                "# Plan\nPhased delivery with deterministic test milestones.",
            ),
            (
                "implementation/acceptance-criteria.yaml",
                "criteria:\n  - id: AC-1\n    name: Verified passes\n",
            ),
        ];

        for (p, c) in remaining_required {
            ArtifactManager::write_artifact_atomic(dir.path(), p, c).unwrap();
        }

        let report3 = ReadinessEvaluator::evaluate(dir.path());
        assert_eq!(report3.ready_required_count, 7);
        assert_eq!(report3.total_required_count, 7);
        assert_eq!(report3.overall_readiness, OverallReadiness::ReadyToFreeze);

        // Verify NOT_APPLICABLE item has correct item status
        let na_item = report3
            .artifacts
            .iter()
            .find(|a| a.path == "implementation/test-plan.md")
            .unwrap();
        assert_eq!(na_item.applicability, ArtifactApplicability::NotApplicable);
        assert_eq!(
            na_item.details,
            Some("Marked NOT_APPLICABLE for this project".to_string())
        );
    }
}

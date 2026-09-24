use crate::core::activity::ActivityManager;
use crate::core::builder::safe_sanitize_text;
use crate::core::git::GitAdapter;
use crate::core::projects::ProjectService;
use crate::core::validation::ValidationService;
use crate::core::workflow::{self, WorkflowAction, WorkflowState};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::path::Path;
use thiserror::Error;
use uuid::Uuid;

#[derive(Error, Debug)]
pub enum ReviewError {
    #[error("Review cycle not found: {0}")]
    NotFound(String),
    #[error("Invalid workflow state {0}: expected WAITING_FOR_REVIEW")]
    InvalidWorkflowState(String),
    #[error("Validation gate blocked: {0}")]
    ValidationGateBlocked(String),
    #[error("Stale review preview: {0}")]
    StalePreview(String),
    #[error("Review verdict parse error: {0}")]
    ParseError(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Database error: {0}")]
    Database(String),
    #[error("Git error: {0}")]
    Git(String),
    #[error("YAML parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewVerdict {
    Accept,
    CorrectionsRequired,
    Blocked,
    ArchitectureConcern,
}

impl fmt::Display for ReviewVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Accept => write!(f, "ACCEPT"),
            Self::CorrectionsRequired => write!(f, "CORRECTIONS_REQUIRED"),
            Self::Blocked => write!(f, "BLOCKED"),
            Self::ArchitectureConcern => write!(f, "ARCHITECTURE_CONCERN"),
        }
    }
}

impl std::str::FromStr for ReviewVerdict {
    type Err = ReviewError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "ACCEPT" => Ok(Self::Accept),
            "CORRECTIONS_REQUIRED" => Ok(Self::CorrectionsRequired),
            "BLOCKED" => Ok(Self::Blocked),
            "ARCHITECTURE_CONCERN" => Ok(Self::ArchitectureConcern),
            _ => Err(ReviewError::ParseError(format!(
                "Invalid verdict string: '{}'. Expected ACCEPT, CORRECTIONS_REQUIRED, BLOCKED, or ARCHITECTURE_CONCERN.",
                s
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewCycleStatus {
    Pending,
    Accepted,
    CorrectionsRequired,
    Blocked,
    ArchitectureConcern,
    Superseded,
}

impl fmt::Display for ReviewCycleStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "PENDING"),
            Self::Accepted => write!(f, "ACCEPTED"),
            Self::CorrectionsRequired => write!(f, "CORRECTIONS_REQUIRED"),
            Self::Blocked => write!(f, "BLOCKED"),
            Self::ArchitectureConcern => write!(f, "ARCHITECTURE_CONCERN"),
            Self::Superseded => write!(f, "SUPERSEDED"),
        }
    }
}

impl std::str::FromStr for ReviewCycleStatus {
    type Err = ReviewError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "PENDING" => Ok(Self::Pending),
            "ACCEPTED" => Ok(Self::Accepted),
            "CORRECTIONS_REQUIRED" => Ok(Self::CorrectionsRequired),
            "BLOCKED" => Ok(Self::Blocked),
            "ARCHITECTURE_CONCERN" => Ok(Self::ArchitectureConcern),
            "SUPERSEDED" => Ok(Self::Superseded),
            _ => Err(ReviewError::ParseError(format!(
                "Unknown cycle status: {}",
                s
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewFindingSeverity {
    Critical,
    Major,
    Minor,
    Info,
}

impl fmt::Display for ReviewFindingSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Critical => write!(f, "CRITICAL"),
            Self::Major => write!(f, "MAJOR"),
            Self::Minor => write!(f, "MINOR"),
            Self::Info => write!(f, "INFO"),
        }
    }
}

impl std::str::FromStr for ReviewFindingSeverity {
    type Err = ReviewError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_uppercase().as_str() {
            "CRITICAL" => Ok(Self::Critical),
            "MAJOR" => Ok(Self::Major),
            "MINOR" => Ok(Self::Minor),
            "INFO" => Ok(Self::Info),
            _ => Ok(Self::Major),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewFindingStatus {
    Open,
    Resolved,
    Superseded,
    Blocked,
}

impl fmt::Display for ReviewFindingStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open => write!(f, "OPEN"),
            Self::Resolved => write!(f, "RESOLVED"),
            Self::Superseded => write!(f, "SUPERSEDED"),
            Self::Blocked => write!(f, "BLOCKED"),
        }
    }
}

impl std::str::FromStr for ReviewFindingStatus {
    type Err = ReviewError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_uppercase().as_str() {
            "OPEN" => Ok(Self::Open),
            "RESOLVED" => Ok(Self::Resolved),
            "SUPERSEDED" => Ok(Self::Superseded),
            "BLOCKED" => Ok(Self::Blocked),
            _ => Ok(Self::Open),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReviewerType {
    ChatgptRelay,
    DiagnosticFake,
}

impl fmt::Display for ReviewerType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ChatgptRelay => write!(f, "CHATGPT_RELAY"),
            Self::DiagnosticFake => write!(f, "DIAGNOSTIC_FAKE"),
        }
    }
}

impl std::str::FromStr for ReviewerType {
    type Err = ReviewError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_uppercase().as_str() {
            "CHATGPT_RELAY" => Ok(Self::ChatgptRelay),
            "DIAGNOSTIC_FAKE" => Ok(Self::DiagnosticFake),
            _ => Ok(Self::ChatgptRelay),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewFindingRecord {
    pub finding_id: String,
    pub project_id: String,
    pub first_cycle_id: String,
    pub last_cycle_id: String,
    pub fingerprint: String,
    pub severity: ReviewFindingSeverity,
    pub status: ReviewFindingStatus,
    pub file_path: Option<String>,
    pub line_range: Option<String>,
    pub title: String,
    pub description: String,
    pub suggested_fix: Option<String>,
    pub resolution_cycle_id: Option<String>,
    pub is_repeat: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewCycleRecord {
    pub cycle_id: String,
    pub project_id: String,
    pub cycle_number: i64,
    pub architecture_version: String,
    pub epoch_id: Option<String>,
    pub validation_run_id: Option<String>,
    pub status: ReviewCycleStatus,
    pub verdict: Option<ReviewVerdict>,
    pub reviewer_type: ReviewerType,
    pub git_head: Option<String>,
    pub git_dirty_fingerprint: Option<String>,
    pub review_packet_hash: String,
    pub corrections_packet: Option<String>,
    pub summary: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub created_at: String,
    pub findings: Vec<ReviewFindingRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedReviewFinding {
    pub id: Option<String>,
    pub severity: Option<String>,
    pub title: String,
    pub file: Option<String>,
    pub lines: Option<String>,
    pub description: String,
    pub suggested_fix: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawVerdictEnvelope {
    #[serde(default)]
    pub schema_version: Option<u32>,
    #[serde(default)]
    pub response_type: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub cycle_id: Option<String>,
    #[serde(default)]
    pub review_packet_hash: Option<String>,
    pub verdict: String,
    pub summary: Option<String>,
    #[serde(default)]
    pub findings: Vec<ParsedReviewFinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewImportPreview {
    pub preview_id: String,
    pub project_id: String,
    pub cycle_id: String,
    pub verdict: ReviewVerdict,
    pub summary: String,
    pub findings: Vec<ParsedReviewFinding>,
    pub raw_response_hash: String,
    pub git_head: Option<String>,
    pub git_dirty_fingerprint: Option<String>,
    pub corrections_preview: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableReviewVerdictFile {
    pub schema_version: u32,
    pub cycle_id: String,
    pub cycle_number: i64,
    pub project_id: String,
    pub architecture_version: String,
    pub epoch_id: Option<String>,
    pub validation_run_id: Option<String>,
    pub verdict: String,
    pub summary: String,
    pub git_head: Option<String>,
    pub git_dirty_fingerprint: Option<String>,
    pub review_packet_hash: String,
    pub raw_response_hash: String,
    pub completed_at: String,
    pub findings: Vec<DurableReviewFinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DurableReviewFinding {
    pub finding_id: String,
    pub fingerprint: String,
    pub severity: String,
    pub title: String,
    pub file_path: Option<String>,
    pub line_range: Option<String>,
    pub description: String,
    pub suggested_fix: Option<String>,
    pub is_repeat: bool,
}

pub struct ReviewService;

impl ReviewService {
    /// Prepares a comprehensive review packet including contract, validation evidence, git diff, and untracked files.
    pub fn prepare_review_packet(
        conn: &Connection,
        repo_path: &Path,
        project_id: &str,
        reviewer_type: ReviewerType,
    ) -> Result<(ReviewCycleRecord, String), ReviewError> {
        // 1. Verify workflow state is WAITING_FOR_REVIEW
        let current_state = workflow::get_workflow_state(conn, project_id)
            .map_err(|e| ReviewError::Database(e.to_string()))?
            .state;

        if current_state != WorkflowState::WaitingForReview {
            return Err(ReviewError::InvalidWorkflowState(current_state.to_string()));
        }

        // 2. Fetch project details and architecture version
        let project_details = ProjectService::get_project_details(conn, None, project_id)
            .map_err(|e| ReviewError::Database(e.to_string()))?;
        let arch_version = project_details
            .artifact
            .as_ref()
            .and_then(|a| a.current_architecture_version.clone())
            .unwrap_or_else(|| "1.0".to_string());

        let epoch_id: Option<String> = conn
            .query_row(
                "SELECT epoch_id FROM builder_epochs WHERE project_id = ?1 AND architecture_version = ?2 ORDER BY created_at DESC LIMIT 1",
                params![project_id, &arch_version],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| ReviewError::Database(e.to_string()))?;

        // 3. Directive 4: Validation review gate check
        let validation_run = ValidationService::check_review_gate(
            conn,
            repo_path,
            project_id,
            &arch_version,
            epoch_id.as_deref().unwrap_or(""),
        )
        .map_err(|e| ReviewError::ValidationGateBlocked(e.to_string()))?;

        let validation_run_id = validation_run.as_ref().map(|r| r.run_id.clone());

        // 4. Git inspection: HEAD, dirty fingerprint, tracked diff
        let git = GitAdapter::new().map_err(|e| ReviewError::Git(e.to_string()))?;
        let git_info = git
            .inspect_repo(repo_path)
            .map_err(|e| ReviewError::Git(e.to_string()))?;
        let git_head = git_info.head_commit;
        let dirty_fingerprint = git
            .compute_implementation_fingerprint(repo_path)
            .map_err(|e| ReviewError::Git(e.to_string()))?;

        let tracked_diff = git
            .get_implementation_diff(repo_path)
            .unwrap_or_else(|_| "No tracked changes.".to_string());

        // 5. Directive 11: Collect and embed untracked source files bounded via GitAdapter
        let untracked_files_section = Self::collect_untracked_source_files(repo_path)?;

        // 6. Calculate cycle number and globally unique cycle ID
        let cycle_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM review_cycles WHERE project_id = ?1",
                params![project_id],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let cycle_number = cycle_count + 1;
        let cycle_id = format!("rcy-{}-{}", project_id, cycle_number);

        // 7. Format validation section
        let validation_section = if let Some(ref run) = validation_run {
            format!(
                "### Validation Evidence\n- **Run ID**: `{}`\n- **Overall Status**: `{}`\n- **Gate Passed**: `{}` (Override: `{}`)\n- **Executed Commands**: {}\n\n",
                run.run_id,
                run.status,
                run.is_gate_passed,
                run.has_override,
                run.commands.len()
            )
        } else {
            "### Validation Evidence\nValidation is not required or unconfigured for this project.\n\n".to_string()
        };

        // 8. Assemble review packet prompt with frozen architecture contracts and prior open findings
        let mut packet = String::new();
        packet.push_str(&format!(
            "# Coalition Review Packet: {} (Cycle {})\n\n",
            project_details.project.name, cycle_number
        ));
        packet.push_str("## Project Context\n");
        packet.push_str(&format!("- **Project ID**: `{}`\n", project_id));
        packet.push_str(&format!("- **Review Cycle ID**: `{}`\n", cycle_id));
        packet.push_str(&format!("- **Architecture Version**: `{}`\n", arch_version));
        if let Some(ref ep) = epoch_id {
            packet.push_str(&format!("- **Builder Epoch**: `{}`\n", ep));
        }
        if let Some(ref h) = git_head {
            packet.push_str(&format!("- **Git HEAD**: `{}`\n", h));
        }
        packet.push_str(&format!(
            "- **Implementation Fingerprint**: `{}`\n\n",
            dirty_fingerprint
        ));

        // Governed Architecture Contracts (Item 4)
        if let Ok(builder_packet) =
            crate::core::freeze::FreezeService::get_builder_packet(repo_path, Some(&arch_version))
        {
            packet.push_str(&format!(
                "## Governed Architecture Contracts (Version {})\n\n",
                arch_version
            ));
            for art in &builder_packet.artifacts {
                packet.push_str(&format!(
                    "### Contract Artifact: `{}`\n```markdown\n{}\n```\n\n",
                    art.path, art.content
                ));
            }
        }

        // Prior Open Review Findings
        let open_findings =
            list_findings_for_project(conn, project_id, Some(ReviewFindingStatus::Open))?;
        if !open_findings.is_empty() {
            packet.push_str("## Prior Open Review Findings\n\n");
            for f in &open_findings {
                packet.push_str(&format!(
                    "- **Finding `{}`** (Severity: {}): {}\n  - File: `{}` (Lines: `{}`)\n  - Defect: {}\n  - Suggested Fix: {}\n\n",
                    f.finding_id,
                    f.severity,
                    f.title,
                    f.file_path.as_deref().unwrap_or("n/a"),
                    f.line_range.as_deref().unwrap_or("n/a"),
                    f.description,
                    f.suggested_fix.as_deref().unwrap_or("n/a"),
                ));
            }
        }

        packet.push_str("## Governance Instructions for Independent Reviewer\n");
        packet.push_str(&format!(
            "You are an authoritative independent code reviewer governing this implementation.\n\
             Evaluate the code changes against the architecture contracts, acceptance criteria, and validation evidence.\n\
             You MUST respond with a structured verdict block in either ```verdict or ```yaml fence:\n\n\
             ```verdict\n\
             schema_version: 1\n\
             response_type: REVIEW_VERDICT\n\
             project_id: {}\n\
             cycle_id: {}\n\
             review_packet_hash: <REVIEW_PACKET_HASH>\n\
             verdict: ACCEPT | CORRECTIONS_REQUIRED | BLOCKED | ARCHITECTURE_CONCERN\n\
             summary: \"Comprehensive summary of your review evaluation\"\n\
             findings:\n\
               - id: FND-1\n\
                 severity: CRITICAL | MAJOR | MINOR | INFO\n\
                 title: \"Short concise issue title\"\n\
                 file: \"relative/path/to/file.ext\"\n\
                 lines: \"10-25\"\n\
                 description: \"Detailed description of the defect or contract violation\"\n\
                 suggested_fix: \"Specific instructions on how to correct this defect\"\n\
             ```\n\n\
             Rules:\n\
             - If all implementation contracts and acceptance criteria are satisfied and tests pass, verdict is ACCEPT.\n\
             - If there are actionable defects that the Builder must fix in code, verdict is CORRECTIONS_REQUIRED.\n\
             - If the implementation is fundamentally flawed or irrecoverable without human intervention, verdict is BLOCKED.\n\
             - If the code cannot satisfy requirements due to a contradictory, missing, or impossible architecture contract, verdict is ARCHITECTURE_CONCERN.\n\n",
            project_id, cycle_id
        ));

        packet.push_str("## Validation State\n");
        packet.push_str(&validation_section);

        packet.push_str("## Code Modifications (Tracked Git Diff)\n");
        packet.push_str("```diff\n");
        packet.push_str(&tracked_diff);
        if !tracked_diff.ends_with('\n') {
            packet.push('\n');
        }
        packet.push_str("```\n\n");

        if !untracked_files_section.is_empty() {
            packet.push_str("## Untracked Source Files\n");
            packet.push_str(&untracked_files_section);
            packet.push_str("\n\n");
        }

        // Compute packet hash and embed in instruction placeholder
        let placeholder = "<REVIEW_PACKET_HASH>";
        let mut hasher = Sha256::new();
        hasher.update(packet.as_bytes());
        let review_packet_hash = format!("{:x}", hasher.finalize());
        let final_packet = packet.replace(placeholder, &review_packet_hash);
        let sanitized_packet = safe_sanitize_text(&final_packet);

        let now = chrono::Utc::now().to_rfc3339();

        let cycle_rec = ReviewCycleRecord {
            cycle_id: cycle_id.clone(),
            project_id: project_id.to_string(),
            cycle_number,
            architecture_version: arch_version,
            epoch_id,
            validation_run_id,
            status: ReviewCycleStatus::Pending,
            verdict: None,
            reviewer_type,
            git_head,
            git_dirty_fingerprint: Some(dirty_fingerprint),
            review_packet_hash: review_packet_hash.clone(),
            corrections_packet: None,
            summary: None,
            started_at: now.clone(),
            completed_at: None,
            created_at: now,
            findings: vec![],
        };

        // Write packet to disk under .coalition/reviews/cycle-<n>/packet.md
        let cycle_dir = repo_path
            .join(".coalition")
            .join("reviews")
            .join(format!("cycle-{}", cycle_number));
        std::fs::create_dir_all(&cycle_dir)?;
        std::fs::write(cycle_dir.join("packet.md"), &sanitized_packet)?;

        // Insert cycle record into SQLite
        insert_review_cycle(conn, &cycle_rec)?;

        Ok((cycle_rec, sanitized_packet))
    }

    /// Scans untracked source files bounded to avoid memory exhaustion or huge binaries.
    fn collect_untracked_source_files(repo_path: &Path) -> Result<String, ReviewError> {
        let git = GitAdapter::new().map_err(|e| ReviewError::Git(e.to_string()))?;
        git.collect_untracked_source_files_bounded(repo_path, 500 * 1024, 100 * 1024)
            .map_err(|e| ReviewError::Git(e.to_string()))
    }

    /// Strict parser for reviewer verdict response.
    pub fn parse_reviewer_response(
        text: &str,
    ) -> Result<(ReviewVerdict, String, Vec<ParsedReviewFinding>), ReviewError> {
        let parsed = Self::parse_reviewer_envelope(text)?;
        let verdict: ReviewVerdict = parsed.verdict.parse()?;
        let summary = parsed
            .summary
            .unwrap_or_else(|| format!("Review completed with verdict {}", verdict));

        Ok((verdict, summary, parsed.findings))
    }

    /// Strict parser for structured reviewer envelope with schema and binding metadata.
    pub fn parse_reviewer_envelope(text: &str) -> Result<RawVerdictEnvelope, ReviewError> {
        let verdict_block = extract_verdict_block(text).ok_or_else(|| {
            ReviewError::ParseError(
                "No ```verdict, ```yaml, or ```json code block containing a structured review verdict was found."
                    .to_string(),
            )
        })?;

        let parsed: RawVerdictEnvelope = serde_yaml::from_str(&verdict_block)
            .or_else(|_| serde_json::from_str(&verdict_block))
            .map_err(|e| {
                ReviewError::ParseError(format!(
                    "Failed to parse structured review verdict YAML/JSON: {}",
                    e
                ))
            })?;

        // Strict verdict validation
        let _verdict: ReviewVerdict = parsed.verdict.parse()?;

        Ok(parsed)
    }

    /// Prepares a preview of importing a review response, validates envelope bounds,
    /// and persists an opaque server-owned preview record.
    pub fn prepare_review_import(
        conn: &Connection,
        repo_path: &Path,
        project_id: &str,
        cycle_id: &str,
        raw_response: &str,
    ) -> Result<ReviewImportPreview, ReviewError> {
        let cycle = get_review_cycle(conn, cycle_id)?
            .ok_or_else(|| ReviewError::NotFound(cycle_id.to_string()))?;

        if cycle.project_id != project_id {
            return Err(ReviewError::NotFound(format!(
                "Cycle {} does not belong to project {}",
                cycle_id, project_id
            )));
        }

        if cycle.status != ReviewCycleStatus::Pending {
            return Err(ReviewError::ParseError(format!(
                "Review cycle {} is not PENDING (current status: {})",
                cycle_id, cycle.status
            )));
        }

        // Parse reviewer envelope
        let envelope = Self::parse_reviewer_envelope(raw_response)?;
        let verdict: ReviewVerdict = envelope.verdict.parse()?;
        let summary = envelope
            .summary
            .clone()
            .unwrap_or_else(|| format!("Review completed with verdict {}", verdict));
        let findings = envelope.findings;

        // Strict Reviewer Envelope Validation (Item 2)
        if let Some(sv) = envelope.schema_version {
            if sv != 1 {
                return Err(ReviewError::ParseError(format!(
                    "Unsupported schema_version: {}. Expected 1.",
                    sv
                )));
            }
        }
        if let Some(ref rt) = envelope.response_type {
            if rt.trim().to_uppercase() != "REVIEW_VERDICT" {
                return Err(ReviewError::ParseError(format!(
                    "Invalid response_type: '{}'. Expected 'REVIEW_VERDICT'.",
                    rt
                )));
            }
        }
        if let Some(ref pid) = envelope.project_id {
            if pid != project_id {
                return Err(ReviewError::ParseError(format!(
                    "Reviewer response project_id '{}' does not match active project '{}'",
                    pid, project_id
                )));
            }
        }
        if let Some(ref cid) = envelope.cycle_id {
            if cid != cycle_id {
                return Err(ReviewError::ParseError(format!(
                    "Reviewer response cycle_id '{}' does not match active cycle '{}'",
                    cid, cycle_id
                )));
            }
        }
        if let Some(ref rph) = envelope.review_packet_hash {
            if rph != &cycle.review_packet_hash {
                return Err(ReviewError::ParseError(format!(
                    "Reviewer response review_packet_hash '{}' does not match active cycle packet hash '{}'",
                    rph, cycle.review_packet_hash
                )));
            }
        }

        // Verify git state has not drifted since packet was prepared
        let git = GitAdapter::new().map_err(|e| ReviewError::Git(e.to_string()))?;
        let current_info = git
            .inspect_repo(repo_path)
            .map_err(|e| ReviewError::Git(e.to_string()))?;
        let current_dirty = git
            .compute_implementation_fingerprint(repo_path)
            .map_err(|e| ReviewError::Git(e.to_string()))?;

        if cycle.git_head != current_info.head_commit {
            return Err(ReviewError::StalePreview(format!(
                "Git HEAD has moved from {:?} to {:?} since review packet was prepared",
                cycle.git_head, current_info.head_commit
            )));
        }

        if cycle.git_dirty_fingerprint.as_deref() != Some(&current_dirty) {
            return Err(ReviewError::StalePreview(format!(
                "Working tree fingerprint has changed from {:?} to {} since review packet was prepared",
                cycle.git_dirty_fingerprint, current_dirty
            )));
        }

        let mut hasher = Sha256::new();
        hasher.update(raw_response.as_bytes());
        let raw_response_hash = format!("{:x}", hasher.finalize());

        let corrections_preview = if verdict == ReviewVerdict::CorrectionsRequired {
            Some(Self::generate_corrections_packet(
                cycle.cycle_number,
                cycle_id,
                &summary,
                &findings,
            ))
        } else {
            None
        };

        let preview_id = format!("prev-rev-{}", Uuid::new_v4());
        let now = chrono::Utc::now().to_rfc3339();

        let preview = ReviewImportPreview {
            preview_id: preview_id.clone(),
            project_id: project_id.to_string(),
            cycle_id: cycle_id.to_string(),
            verdict,
            summary: summary.clone(),
            findings: findings.clone(),
            raw_response_hash: raw_response_hash.clone(),
            git_head: cycle.git_head.clone(),
            git_dirty_fingerprint: Some(current_dirty.clone()),
            corrections_preview: corrections_preview.clone(),
            created_at: now.clone(),
        };

        // Server-Owned Preview Persistence (Item 3, Forward Migration 012)
        let findings_json = serde_json::to_string(&findings).unwrap_or_else(|_| "[]".to_string());
        conn.execute(
            "INSERT INTO review_import_previews (
                preview_id, project_id, cycle_id, verdict, summary, raw_response,
                raw_response_hash, git_head, git_dirty_fingerprint, corrections_preview,
                findings_json, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                preview_id,
                project_id,
                cycle_id,
                verdict.to_string(),
                summary,
                raw_response,
                raw_response_hash,
                cycle.git_head,
                current_dirty,
                corrections_preview,
                findings_json,
                now,
            ],
        )
        .map_err(|e| ReviewError::Database(e.to_string()))?;

        Ok(preview)
    }

    /// Authoritatively confirms review import from a server-owned preview record.
    /// Writes durable review artifacts, updates SQLite atomically, and transitions workflow.
    pub fn confirm_review_import(
        conn: &mut Connection,
        repo_path: &Path,
        project_id: &str,
        preview_id: &str,
        actor: &str,
    ) -> Result<ReviewCycleRecord, ReviewError> {
        // 1. Fetch server-owned preview record from review_import_previews (Item 3)
        struct StoredPreview {
            cycle_id: String,
            verdict: ReviewVerdict,
            summary: String,
            raw_response: String,
            raw_response_hash: String,
            git_head: Option<String>,
            git_dirty_fingerprint: Option<String>,
            findings: Vec<ParsedReviewFinding>,
        }

        let stored: StoredPreview = conn
            .query_row(
                "SELECT cycle_id, verdict, summary, raw_response, raw_response_hash,
                        git_head, git_dirty_fingerprint, findings_json
                 FROM review_import_previews
                 WHERE preview_id = ?1 AND project_id = ?2",
                params![preview_id, project_id],
                |r| {
                    let v_str: String = r.get(1)?;
                    let v: ReviewVerdict = v_str.parse().map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            1,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?;
                    let f_json: String = r.get(7)?;
                    let findings: Vec<ParsedReviewFinding> =
                        serde_json::from_str(&f_json).unwrap_or_default();
                    Ok(StoredPreview {
                        cycle_id: r.get(0)?,
                        verdict: v,
                        summary: r.get(2)?,
                        raw_response: r.get(3)?,
                        raw_response_hash: r.get(4)?,
                        git_head: r.get(5)?,
                        git_dirty_fingerprint: r.get(6)?,
                        findings,
                    })
                },
            )
            .optional()
            .map_err(|e| ReviewError::Database(e.to_string()))?
            .ok_or_else(|| {
                ReviewError::NotFound(format!(
                    "Review import preview '{}' not found for project '{}'",
                    preview_id, project_id
                ))
            })?;

        let cycle = get_review_cycle(conn, &stored.cycle_id)?
            .ok_or_else(|| ReviewError::NotFound(stored.cycle_id.clone()))?;

        if cycle.status != ReviewCycleStatus::Pending {
            return Err(ReviewError::ParseError(format!(
                "Review cycle {} is not PENDING (current status: {})",
                cycle.cycle_id, cycle.status
            )));
        }

        // 2. Verify working tree has not drifted since preview was created
        let git = GitAdapter::new().map_err(|e| ReviewError::Git(e.to_string()))?;
        let current_info = git
            .inspect_repo(repo_path)
            .map_err(|e| ReviewError::Git(e.to_string()))?;
        let current_dirty = git
            .compute_implementation_fingerprint(repo_path)
            .map_err(|e| ReviewError::Git(e.to_string()))?;

        if stored.git_head != current_info.head_commit {
            return Err(ReviewError::StalePreview(format!(
                "Git HEAD has moved from {:?} to {:?} since preview was created",
                stored.git_head, current_info.head_commit
            )));
        }

        if stored.git_dirty_fingerprint.as_deref() != Some(&current_dirty) {
            return Err(ReviewError::StalePreview(format!(
                "Working tree fingerprint has changed from {:?} to {} since preview was created",
                stored.git_dirty_fingerprint, current_dirty
            )));
        }

        // 3. Directive 4: Ensure validation review gate remains satisfied at confirmation point
        let _ = ValidationService::check_review_gate(
            conn,
            repo_path,
            project_id,
            &cycle.architecture_version,
            cycle.epoch_id.as_deref().unwrap_or(""),
        )
        .map_err(|e| ReviewError::ValidationGateBlocked(e.to_string()))?;

        let now = chrono::Utc::now().to_rfc3339();
        let cycle_dir = repo_path
            .join(".coalition")
            .join("reviews")
            .join(format!("cycle-{}", cycle.cycle_number));
        std::fs::create_dir_all(&cycle_dir)?;

        // 4. Persist response.md
        let sanitized_resp = safe_sanitize_text(&stored.raw_response);
        std::fs::write(cycle_dir.join("response.md"), &sanitized_resp)?;

        // 5. Process findings & compute fingerprints (excluding severity per Item 14)
        let mut finding_records = Vec::new();
        let mut durable_findings = Vec::new();

        let mut open_findings_map = std::collections::HashMap::new();
        {
            let mut stmt = conn
                .prepare(
                    "SELECT finding_id, fingerprint, first_cycle_id, title FROM review_findings
                     WHERE project_id = ?1 AND status = 'OPEN'",
                )
                .map_err(|e| ReviewError::Database(e.to_string()))?;
            let rows = stmt
                .query_map(params![project_id], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })
                .map_err(|e| ReviewError::Database(e.to_string()))?;
            for item in rows.flatten() {
                open_findings_map.insert(item.1, (item.0, item.2));
            }
        }

        for pf in &stored.findings {
            let severity: ReviewFindingSeverity = pf
                .severity
                .as_deref()
                .unwrap_or("MAJOR")
                .parse()
                .unwrap_or(ReviewFindingSeverity::Major);

            // Compute stable finding fingerprint EXCLUDING severity (Item 14)
            let norm_file = pf
                .file
                .as_deref()
                .unwrap_or("")
                .replace('\\', "/")
                .to_lowercase();
            let norm_title = pf.title.trim().to_lowercase();
            let mut hasher = Sha256::new();
            hasher.update(norm_file.as_bytes());
            hasher.update(b":");
            hasher.update(norm_title.as_bytes());
            let fp = format!("{:x}", hasher.finalize());

            let (finding_id, first_cycle_id, is_repeat) =
                if let Some((existing_id, orig_cycle)) = open_findings_map.get(&fp) {
                    (existing_id.clone(), orig_cycle.clone(), true)
                } else {
                    let fid = format!("fnd-{}-{}", project_id, Uuid::new_v4());
                    (fid, cycle.cycle_id.clone(), false)
                };

            let f_rec = ReviewFindingRecord {
                finding_id: finding_id.clone(),
                project_id: project_id.to_string(),
                first_cycle_id,
                last_cycle_id: cycle.cycle_id.clone(),
                fingerprint: fp.clone(),
                severity,
                status: ReviewFindingStatus::Open,
                file_path: pf.file.clone(),
                line_range: pf.lines.clone(),
                title: pf.title.clone(),
                description: pf.description.clone(),
                suggested_fix: pf.suggested_fix.clone(),
                resolution_cycle_id: None,
                is_repeat,
                created_at: now.clone(),
                updated_at: now.clone(),
            };

            durable_findings.push(DurableReviewFinding {
                finding_id: finding_id.clone(),
                fingerprint: fp,
                severity: severity.to_string(),
                title: pf.title.clone(),
                file_path: pf.file.clone(),
                line_range: pf.lines.clone(),
                description: pf.description.clone(),
                suggested_fix: pf.suggested_fix.clone(),
                is_repeat,
            });

            finding_records.push(f_rec);
        }

        // 6. Generate corrections packet if corrections required
        let corrections_text = if stored.verdict == ReviewVerdict::CorrectionsRequired {
            let cp = Self::generate_corrections_packet(
                cycle.cycle_number,
                &cycle.cycle_id,
                &stored.summary,
                &stored.findings,
            );
            std::fs::write(cycle_dir.join("corrections.md"), &cp)?;
            Some(cp)
        } else {
            None
        };

        // 7. Write verdict.yaml atomically
        let verdict_file = DurableReviewVerdictFile {
            schema_version: 1,
            cycle_id: cycle.cycle_id.clone(),
            cycle_number: cycle.cycle_number,
            project_id: project_id.to_string(),
            architecture_version: cycle.architecture_version.clone(),
            epoch_id: cycle.epoch_id.clone(),
            validation_run_id: cycle.validation_run_id.clone(),
            verdict: stored.verdict.to_string(),
            summary: stored.summary.clone(),
            git_head: stored.git_head.clone(),
            git_dirty_fingerprint: stored.git_dirty_fingerprint.clone(),
            review_packet_hash: cycle.review_packet_hash.clone(),
            raw_response_hash: stored.raw_response_hash.clone(),
            completed_at: now.clone(),
            findings: durable_findings,
        };

        let verdict_yaml = serde_yaml::to_string(&verdict_file)?;
        let final_path = cycle_dir.join("verdict.yaml");
        let temp_path = cycle_dir.join(format!(".verdict-{}.tmp", Uuid::new_v4()));
        std::fs::write(&temp_path, &verdict_yaml)?;
        std::fs::rename(&temp_path, &final_path)?;

        // 8. Atomic SQLite persistence transaction (Item 15)
        let new_cycle_status = match stored.verdict {
            ReviewVerdict::Accept => ReviewCycleStatus::Accepted,
            ReviewVerdict::CorrectionsRequired => ReviewCycleStatus::CorrectionsRequired,
            ReviewVerdict::Blocked => ReviewCycleStatus::Blocked,
            ReviewVerdict::ArchitectureConcern => ReviewCycleStatus::ArchitectureConcern,
        };

        let tx = conn
            .transaction()
            .map_err(|e| ReviewError::Database(e.to_string()))?;

        tx.execute(
            "UPDATE review_cycles
             SET status = ?1, verdict = ?2, summary = ?3, corrections_packet = ?4, completed_at = ?5
             WHERE cycle_id = ?6",
            params![
                new_cycle_status.to_string(),
                stored.verdict.to_string(),
                stored.summary,
                corrections_text,
                now,
                cycle.cycle_id
            ],
        )
        .map_err(|e| ReviewError::Database(e.to_string()))?;

        for f in &finding_records {
            tx.execute(
                "INSERT INTO review_findings (
                    finding_id, project_id, first_cycle_id, last_cycle_id, fingerprint,
                    severity, status, file_path, line_range, title, description,
                    suggested_fix, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                ON CONFLICT(finding_id) DO UPDATE SET
                    last_cycle_id = excluded.last_cycle_id,
                    updated_at = excluded.updated_at",
                params![
                    f.finding_id,
                    f.project_id,
                    f.first_cycle_id,
                    f.last_cycle_id,
                    f.fingerprint,
                    f.severity.to_string(),
                    f.status.to_string(),
                    f.file_path,
                    f.line_range,
                    f.title,
                    f.description,
                    f.suggested_fix,
                    f.created_at,
                    f.updated_at
                ],
            )
            .map_err(|e| ReviewError::Database(e.to_string()))?;

            tx.execute(
                "INSERT OR REPLACE INTO review_cycle_findings (cycle_id, finding_id, is_repeat)
                 VALUES (?1, ?2, ?3)",
                params![
                    cycle.cycle_id,
                    f.finding_id,
                    if f.is_repeat { 1 } else { 0 }
                ],
            )
            .map_err(|e| ReviewError::Database(e.to_string()))?;
        }

        // On ACCEPT verdict: all open findings for this project are authoritatively resolved (Item 14)
        if stored.verdict == ReviewVerdict::Accept {
            tx.execute(
                "UPDATE review_findings
                 SET status = 'RESOLVED', resolution_cycle_id = ?1, updated_at = ?2
                 WHERE project_id = ?3 AND status = 'OPEN'",
                params![cycle.cycle_id, now, project_id],
            )
            .map_err(|e| ReviewError::Database(e.to_string()))?;
        }

        // Clean up consumed preview record
        tx.execute(
            "DELETE FROM review_import_previews WHERE preview_id = ?1",
            params![preview_id],
        )
        .map_err(|e| ReviewError::Database(e.to_string()))?;

        // 9. Authoritative workflow state transition
        let action = match stored.verdict {
            ReviewVerdict::Accept => WorkflowAction::AcceptReview,
            ReviewVerdict::CorrectionsRequired => WorkflowAction::RequestCorrections,
            ReviewVerdict::Blocked => WorkflowAction::Block,
            ReviewVerdict::ArchitectureConcern => WorkflowAction::RaiseArchitectureConcern,
        };

        workflow::apply_workflow_action_conn(&tx, project_id, action, actor)
            .map_err(|e| ReviewError::Database(e.to_string()))?;

        // 10. Record audit activity event
        let meta = serde_json::json!({
            "cycle_id": cycle.cycle_id,
            "verdict": stored.verdict.to_string(),
            "findings_count": finding_records.len(),
            "summary": stored.summary
        });
        let _ = ActivityManager::record_event(
            &tx,
            project_id,
            "REVIEW_VERDICT_CONFIRMED",
            actor,
            &format!(
                "Review cycle {} concluded with verdict {}: {}",
                cycle.cycle_id, stored.verdict, stored.summary
            ),
            Some(&meta),
        );

        tx.commit()
            .map_err(|e| ReviewError::Database(e.to_string()))?;

        let mut completed_cycle = cycle;
        completed_cycle.status = new_cycle_status;
        completed_cycle.verdict = Some(stored.verdict);
        completed_cycle.summary = Some(stored.summary);
        completed_cycle.corrections_packet = corrections_text;
        completed_cycle.completed_at = Some(now);
        completed_cycle.findings = finding_records;

        Ok(completed_cycle)
    }

    /// Formats a normalized correction prompt for the Builder.
    pub fn generate_corrections_packet(
        cycle_number: i64,
        cycle_id: &str,
        summary: &str,
        findings: &[ParsedReviewFinding],
    ) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "# Coalition Review Correction Request (Cycle {})\n\n",
            cycle_number
        ));
        out.push_str(&format!("- **Review Cycle ID**: `{}`\n", cycle_id));
        out.push_str("- **Source**: Governed Reviewer\n");
        out.push_str("- **Directive**: Independent review requires corrections before this change can be accepted.\n\n");
        out.push_str("## Overall Review Evaluation\n");
        out.push_str(summary);
        out.push_str("\n\n## Actionable Findings to Correct\n");

        if findings.is_empty() {
            out.push_str("No specific findings reported. Review summary indicates general corrections required.\n");
        } else {
            for (idx, f) in findings.iter().enumerate() {
                let sev = f.severity.as_deref().unwrap_or("MAJOR");
                out.push_str(&format!(
                    "### [{}-{:02}] ({}) {}\n",
                    cycle_number,
                    idx + 1,
                    sev,
                    f.title
                ));
                if let Some(ref path) = f.file {
                    out.push_str(&format!("- **File**: `{}`", path));
                    if let Some(ref lines) = f.lines {
                        out.push_str(&format!(" (lines: {})", lines));
                    }
                    out.push('\n');
                }
                out.push_str(&format!("- **Defect Description**: {}\n", f.description));
                if let Some(ref fix) = f.suggested_fix {
                    out.push_str(&format!("- **Required Correction**: {}\n", fix));
                }
                out.push('\n');
            }
        }

        safe_sanitize_text(&out)
    }

    /// Rehydrates review cycles from durable disk artifacts if SQLite state was deleted.
    pub fn rehydrate_reviews_from_disk(
        conn: &mut Connection,
        repo_path: &Path,
        project_id: &str,
    ) -> Result<usize, ReviewError> {
        let reviews_dir = repo_path.join(".coalition").join("reviews");
        if !reviews_dir.exists() || !reviews_dir.is_dir() {
            return Ok(0);
        }

        let mut count = 0;
        let entries = std::fs::read_dir(&reviews_dir)?;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let verdict_path = path.join("verdict.yaml");
            if !verdict_path.exists() {
                continue;
            }

            if let Ok(content) = std::fs::read_to_string(&verdict_path) {
                if let Ok(vf) = serde_yaml::from_str::<DurableReviewVerdictFile>(&content) {
                    if vf.project_id != project_id {
                        continue;
                    }

                    // Check if cycle already exists
                    let exists: bool = conn
                        .query_row(
                            "SELECT 1 FROM review_cycles WHERE cycle_id = ?1",
                            params![vf.cycle_id],
                            |_| Ok(true),
                        )
                        .unwrap_or(false);

                    if !exists {
                        let verdict: Option<ReviewVerdict> = vf.verdict.parse().ok();
                        let status = match verdict {
                            Some(ReviewVerdict::Accept) => ReviewCycleStatus::Accepted,
                            Some(ReviewVerdict::CorrectionsRequired) => {
                                ReviewCycleStatus::CorrectionsRequired
                            }
                            Some(ReviewVerdict::Blocked) => ReviewCycleStatus::Blocked,
                            Some(ReviewVerdict::ArchitectureConcern) => {
                                ReviewCycleStatus::ArchitectureConcern
                            }
                            None => ReviewCycleStatus::Pending,
                        };

                        let corrections_content =
                            std::fs::read_to_string(path.join("corrections.md")).ok();

                        conn.execute(
                            "INSERT INTO review_cycles (
                                cycle_id, project_id, cycle_number, architecture_version, epoch_id,
                                validation_run_id, status, verdict, reviewer_type, git_head,
                                git_dirty_fingerprint, review_packet_hash, corrections_packet, summary,
                                started_at, completed_at, created_at
                            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'CHATGPT_RELAY', ?9, ?10, ?11, ?12, ?13, ?14, ?14, ?14)",
                            params![
                                vf.cycle_id,
                                project_id,
                                vf.cycle_number,
                                vf.architecture_version,
                                vf.epoch_id,
                                vf.validation_run_id,
                                status.to_string(),
                                vf.verdict,
                                vf.git_head,
                                vf.git_dirty_fingerprint,
                                vf.review_packet_hash,
                                corrections_content,
                                vf.summary,
                                vf.completed_at
                            ],
                        ).map_err(|e| ReviewError::Database(e.to_string()))?;

                        for f in &vf.findings {
                            conn.execute(
                                "INSERT OR IGNORE INTO review_findings (
                                    finding_id, project_id, first_cycle_id, last_cycle_id, fingerprint,
                                    severity, status, file_path, line_range, title, description,
                                    suggested_fix, created_at, updated_at
                                ) VALUES (?1, ?2, ?3, ?3, ?4, ?5, 'OPEN', ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
                                params![
                                    f.finding_id,
                                    project_id,
                                    vf.cycle_id,
                                    f.fingerprint,
                                    f.severity,
                                    f.file_path,
                                    f.line_range,
                                    f.title,
                                    f.description,
                                    f.suggested_fix,
                                    vf.completed_at
                                ],
                            ).map_err(|e| ReviewError::Database(e.to_string()))?;

                            conn.execute(
                                "INSERT OR REPLACE INTO review_cycle_findings (cycle_id, finding_id, is_repeat)
                                 VALUES (?1, ?2, ?3)",
                                params![vf.cycle_id, f.finding_id, if f.is_repeat { 1 } else { 0 }],
                            ).map_err(|e| ReviewError::Database(e.to_string()))?;
                        }

                        // If verdict was Accept, mark findings resolved
                        if verdict == Some(ReviewVerdict::Accept) {
                            let _ = conn.execute(
                                "UPDATE review_findings SET status = 'RESOLVED', resolution_cycle_id = ?1, updated_at = ?2 WHERE project_id = ?3 AND status = 'OPEN'",
                                params![vf.cycle_id, vf.completed_at, project_id],
                            );
                        }

                        count += 1;
                    }
                }
            }
        }

        Ok(count)
    }
}

// Database helper functions

pub fn insert_review_cycle(
    conn: &Connection,
    record: &ReviewCycleRecord,
) -> Result<(), ReviewError> {
    conn.execute(
        "INSERT INTO review_cycles (
            cycle_id, project_id, cycle_number, architecture_version, epoch_id,
            validation_run_id, status, verdict, reviewer_type, git_head,
            git_dirty_fingerprint, review_packet_hash, corrections_packet, summary,
            started_at, completed_at, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        params![
            record.cycle_id,
            record.project_id,
            record.cycle_number,
            record.architecture_version,
            record.epoch_id,
            record.validation_run_id,
            record.status.to_string(),
            record.verdict.map(|v| v.to_string()),
            record.reviewer_type.to_string(),
            record.git_head,
            record.git_dirty_fingerprint,
            record.review_packet_hash,
            record.corrections_packet,
            record.summary,
            record.started_at,
            record.completed_at,
            record.created_at,
        ],
    )
    .map_err(|e| ReviewError::Database(e.to_string()))?;

    Ok(())
}

pub fn get_review_cycle(
    conn: &Connection,
    cycle_id: &str,
) -> Result<Option<ReviewCycleRecord>, ReviewError> {
    let mut stmt = conn
        .prepare(
            "SELECT cycle_id, project_id, cycle_number, architecture_version, epoch_id,
                    validation_run_id, status, verdict, reviewer_type, git_head,
                    git_dirty_fingerprint, review_packet_hash, corrections_packet, summary,
                    started_at, completed_at, created_at
             FROM review_cycles WHERE cycle_id = ?1",
        )
        .map_err(|e| ReviewError::Database(e.to_string()))?;

    let cycle_opt = stmt
        .query_row(params![cycle_id], |row| {
            let status_str: String = row.get(6)?;
            let verdict_str: Option<String> = row.get(7)?;
            let reviewer_str: String = row.get(8)?;

            Ok(ReviewCycleRecord {
                cycle_id: row.get(0)?,
                project_id: row.get(1)?,
                cycle_number: row.get(2)?,
                architecture_version: row.get(3)?,
                epoch_id: row.get(4)?,
                validation_run_id: row.get(5)?,
                status: status_str.parse().unwrap_or(ReviewCycleStatus::Pending),
                verdict: verdict_str.and_then(|v| v.parse().ok()),
                reviewer_type: reviewer_str.parse().unwrap_or(ReviewerType::ChatgptRelay),
                git_head: row.get(9)?,
                git_dirty_fingerprint: row.get(10)?,
                review_packet_hash: row.get(11)?,
                corrections_packet: row.get(12)?,
                summary: row.get(13)?,
                started_at: row.get(14)?,
                completed_at: row.get(15)?,
                created_at: row.get(16)?,
                findings: Vec::new(),
            })
        })
        .optional()
        .map_err(|e| ReviewError::Database(e.to_string()))?;

    if let Some(mut cycle) = cycle_opt {
        cycle.findings = list_findings_for_cycle(conn, &cycle.cycle_id)?;
        Ok(Some(cycle))
    } else {
        Ok(None)
    }
}

pub fn get_latest_review_cycle(
    conn: &Connection,
    project_id: &str,
) -> Result<Option<ReviewCycleRecord>, ReviewError> {
    let latest_id: Option<String> = conn
        .query_row(
            "SELECT cycle_id FROM review_cycles WHERE project_id = ?1 ORDER BY cycle_number DESC LIMIT 1",
            params![project_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| ReviewError::Database(e.to_string()))?;

    if let Some(id) = latest_id {
        get_review_cycle(conn, &id)
    } else {
        Ok(None)
    }
}

pub fn list_review_cycles_for_project(
    conn: &Connection,
    project_id: &str,
    limit: usize,
) -> Result<Vec<ReviewCycleRecord>, ReviewError> {
    let mut stmt = conn
        .prepare(
            "SELECT cycle_id, project_id, cycle_number, architecture_version, epoch_id,
                    validation_run_id, status, verdict, reviewer_type, git_head,
                    git_dirty_fingerprint, review_packet_hash, corrections_packet, summary,
                    started_at, completed_at, created_at
             FROM review_cycles WHERE project_id = ?1
             ORDER BY cycle_number DESC LIMIT ?2",
        )
        .map_err(|e| ReviewError::Database(e.to_string()))?;

    let rows = stmt
        .query_map(params![project_id, limit as i64], |row| {
            let status_str: String = row.get(6)?;
            let verdict_str: Option<String> = row.get(7)?;
            let reviewer_str: String = row.get(8)?;

            Ok(ReviewCycleRecord {
                cycle_id: row.get(0)?,
                project_id: row.get(1)?,
                cycle_number: row.get(2)?,
                architecture_version: row.get(3)?,
                epoch_id: row.get(4)?,
                validation_run_id: row.get(5)?,
                status: status_str.parse().unwrap_or(ReviewCycleStatus::Pending),
                verdict: verdict_str.and_then(|v| v.parse().ok()),
                reviewer_type: reviewer_str.parse().unwrap_or(ReviewerType::ChatgptRelay),
                git_head: row.get(9)?,
                git_dirty_fingerprint: row.get(10)?,
                review_packet_hash: row.get(11)?,
                corrections_packet: row.get(12)?,
                summary: row.get(13)?,
                started_at: row.get(14)?,
                completed_at: row.get(15)?,
                created_at: row.get(16)?,
                findings: Vec::new(),
            })
        })
        .map_err(|e| ReviewError::Database(e.to_string()))?;

    let mut result = Vec::new();
    for mut item in rows.flatten() {
        item.findings = list_findings_for_cycle(conn, &item.cycle_id)?;
        result.push(item);
    }
    Ok(result)
}

pub fn list_findings_for_cycle(
    conn: &Connection,
    cycle_id: &str,
) -> Result<Vec<ReviewFindingRecord>, ReviewError> {
    let mut stmt = conn
        .prepare(
            "SELECT f.finding_id, f.project_id, f.first_cycle_id, f.last_cycle_id, f.fingerprint,
                    f.severity, f.status, f.file_path, f.line_range, f.title, f.description,
                    f.suggested_fix, f.resolution_cycle_id, cf.is_repeat, f.created_at, f.updated_at
             FROM review_findings f
             JOIN review_cycle_findings cf ON f.finding_id = cf.finding_id
             WHERE cf.cycle_id = ?1
             ORDER BY f.finding_id ASC",
        )
        .map_err(|e| ReviewError::Database(e.to_string()))?;

    let rows = stmt
        .query_map(params![cycle_id], |row| {
            let sev_str: String = row.get(5)?;
            let st_str: String = row.get(6)?;
            let repeat_int: i64 = row.get(13)?;

            Ok(ReviewFindingRecord {
                finding_id: row.get(0)?,
                project_id: row.get(1)?,
                first_cycle_id: row.get(2)?,
                last_cycle_id: row.get(3)?,
                fingerprint: row.get(4)?,
                severity: sev_str.parse().unwrap_or(ReviewFindingSeverity::Major),
                status: st_str.parse().unwrap_or(ReviewFindingStatus::Open),
                file_path: row.get(7)?,
                line_range: row.get(8)?,
                title: row.get(9)?,
                description: row.get(10)?,
                suggested_fix: row.get(11)?,
                resolution_cycle_id: row.get(12)?,
                is_repeat: repeat_int != 0,
                created_at: row.get(14)?,
                updated_at: row.get(15)?,
            })
        })
        .map_err(|e| ReviewError::Database(e.to_string()))?;

    let mut list = Vec::new();
    for item in rows.flatten() {
        list.push(item);
    }
    Ok(list)
}

pub fn list_findings_for_project(
    conn: &Connection,
    project_id: &str,
    status_filter: Option<ReviewFindingStatus>,
) -> Result<Vec<ReviewFindingRecord>, ReviewError> {
    let sql = match status_filter {
        Some(st) => format!(
            "SELECT finding_id, project_id, first_cycle_id, last_cycle_id, fingerprint,
                    severity, status, file_path, line_range, title, description,
                    suggested_fix, resolution_cycle_id, 0, created_at, updated_at
             FROM review_findings
             WHERE project_id = ?1 AND status = '{}'
             ORDER BY finding_id ASC",
            st
        ),
        None => "SELECT finding_id, project_id, first_cycle_id, last_cycle_id, fingerprint,
                        severity, status, file_path, line_range, title, description,
                        suggested_fix, resolution_cycle_id, 0, created_at, updated_at
                 FROM review_findings
                 WHERE project_id = ?1
                 ORDER BY finding_id ASC"
            .to_string(),
    };

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| ReviewError::Database(e.to_string()))?;

    let rows = stmt
        .query_map(params![project_id], |row| {
            let sev_str: String = row.get(5)?;
            let st_str: String = row.get(6)?;
            let repeat_int: i64 = row.get(13)?;

            Ok(ReviewFindingRecord {
                finding_id: row.get(0)?,
                project_id: row.get(1)?,
                first_cycle_id: row.get(2)?,
                last_cycle_id: row.get(3)?,
                fingerprint: row.get(4)?,
                severity: sev_str.parse().unwrap_or(ReviewFindingSeverity::Major),
                status: st_str.parse().unwrap_or(ReviewFindingStatus::Open),
                file_path: row.get(7)?,
                line_range: row.get(8)?,
                title: row.get(9)?,
                description: row.get(10)?,
                suggested_fix: row.get(11)?,
                resolution_cycle_id: row.get(12)?,
                is_repeat: repeat_int != 0,
                created_at: row.get(14)?,
                updated_at: row.get(15)?,
            })
        })
        .map_err(|e| ReviewError::Database(e.to_string()))?;

    let mut list = Vec::new();
    for item in rows.flatten() {
        list.push(item);
    }
    Ok(list)
}

/// Helper function to extract structured verdict code fence.
fn extract_verdict_block(text: &str) -> Option<String> {
    // Look for ```verdict, ```yaml, or ```json
    for tag in &["```verdict", "```yaml", "```json", "```"] {
        if let Some(start_idx) = text.find(tag) {
            let content_start = start_idx + tag.len();
            if let Some(end_idx) = text[content_start..].find("```") {
                let candidate = text[content_start..content_start + end_idx].trim();
                if candidate.contains("verdict:") || candidate.contains("\"verdict\":") {
                    return Some(candidate.to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_parse_reviewer_response_accept() {
        let resp = r#"
Here is my review of the changes.

```verdict
verdict: ACCEPT
summary: "All implementation details adhere strictly to the frozen contract. Tests pass."
findings: []
```
"#;
        let (verdict, summary, findings) = ReviewService::parse_reviewer_response(resp).unwrap();
        assert_eq!(verdict, ReviewVerdict::Accept);
        assert!(summary.contains("adhere strictly"));
        assert!(findings.is_empty());
    }

    #[test]
    fn test_parse_reviewer_response_corrections_required() {
        let resp = r#"
```verdict
verdict: CORRECTIONS_REQUIRED
summary: "Found 2 issues that violate error handling requirements."
findings:
  - id: FND-1
    severity: CRITICAL
    title: "Missing input bounds check"
    file: "src/calc.rs"
    lines: "45-52"
    description: "Integer overflow possible when input exceeds MAX_VAL."
    suggested_fix: "Use checked_add or saturating_add."
  - id: FND-2
    severity: MINOR
    title: "Unused parameter in helper"
    file: "src/utils.rs"
    description: "Parameter `debug` is not used in log_event."
```
"#;
        let (verdict, _summary, findings) = ReviewService::parse_reviewer_response(resp).unwrap();
        assert_eq!(verdict, ReviewVerdict::CorrectionsRequired);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].title, "Missing input bounds check");
        assert_eq!(findings[0].severity.as_deref(), Some("CRITICAL"));
        assert_eq!(findings[1].severity.as_deref(), Some("MINOR"));
    }

    #[test]
    fn test_generate_corrections_packet() {
        let findings = vec![ParsedReviewFinding {
            id: Some("FND-1".to_string()),
            severity: Some("CRITICAL".to_string()),
            title: "Memory leak in cache".to_string(),
            file: Some("src/cache.rs".to_string()),
            lines: Some("12-15".to_string()),
            description: "Entries are not pruned on TTL expiry.".to_string(),
            suggested_fix: Some("Add cleanup_expired() call.".to_string()),
        }];
        let packet = ReviewService::generate_corrections_packet(
            1,
            "rcy-test-1",
            "Fix critical issues",
            &findings,
        );
        assert!(packet.contains("# Coalition Review Correction Request (Cycle 1)"));
        assert!(packet.contains("rcy-test-1"));
        assert!(packet.contains("Memory leak in cache"));
        assert!(packet.contains("src/cache.rs"));
        assert!(packet.contains("cleanup_expired()"));
    }

    #[test]
    fn test_rehydrate_reviews_from_disk() {
        let dir = tempdir().unwrap();
        let repo_path = dir.path();

        let cycle_dir = repo_path.join(".coalition").join("reviews").join("cycle-1");
        std::fs::create_dir_all(&cycle_dir).unwrap();

        let verdict_file = DurableReviewVerdictFile {
            schema_version: 1,
            cycle_id: "cycle-1".to_string(),
            cycle_number: 1,
            project_id: "proj-rehydrate".to_string(),
            architecture_version: "1.0".to_string(),
            epoch_id: Some("ep-1".to_string()),
            validation_run_id: None,
            verdict: "ACCEPT".to_string(),
            summary: "Accepted on rehydration".to_string(),
            git_head: Some("head-abc".to_string()),
            git_dirty_fingerprint: Some("dirty-xyz".to_string()),
            review_packet_hash: "hash-123".to_string(),
            raw_response_hash: "hash-456".to_string(),
            completed_at: chrono::Utc::now().to_rfc3339(),
            findings: vec![],
        };
        std::fs::write(
            cycle_dir.join("verdict.yaml"),
            serde_yaml::to_string(&verdict_file).unwrap(),
        )
        .unwrap();

        let mut db = crate::db::DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();

        db.connection()
            .execute(
                "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    "proj-rehydrate",
                    "Rehydration Test",
                    repo_path.to_str().unwrap(),
                    chrono::Utc::now().to_rfc3339(),
                    chrono::Utc::now().to_rfc3339(),
                    chrono::Utc::now().to_rfc3339()
                ],
            )
            .unwrap();

        let count = ReviewService::rehydrate_reviews_from_disk(
            db.connection_mut(),
            repo_path,
            "proj-rehydrate",
        )
        .unwrap();
        assert_eq!(count, 1);

        let cycle = get_review_cycle(db.connection(), "cycle-1")
            .unwrap()
            .unwrap();
        assert_eq!(cycle.status, ReviewCycleStatus::Accepted);
        assert_eq!(cycle.verdict, Some(ReviewVerdict::Accept));
        assert_eq!(cycle.summary.as_deref(), Some("Accepted on rehydration"));
    }
}

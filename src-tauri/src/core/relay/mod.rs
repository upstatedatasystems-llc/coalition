pub mod readiness;

use crate::core::activity::ActivityManager;
use crate::core::artifacts::{ArtifactError, ArtifactManager};
use crate::core::relay::readiness::{OverallReadiness, ReadinessEvaluator, ReadinessReport};
use crate::core::workflow::{self, WorkflowAction, WorkflowState};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RelayRole {
    Architect,
    Reviewer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RelayPacketType {
    ArchitectInitial,
    ArchitectUpdate,
    ArchitectureRevision,
    ReviewRequest,
    ReviewVerdict,
    ArchitectureConcernAnalysis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RelayExpectedResponse {
    ArchitectUpdate,
    ReviewVerdict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RelayPacketStatus {
    Pending,
    DecisionPending,
    Imported,
    Superseded,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayPacketMetadata {
    pub schema: u32,
    pub project_id: String,
    pub packet_id: String,
    pub role: RelayRole,
    pub packet_type: RelayPacketType,
    pub architecture_version: String,
    pub expected_response: RelayExpectedResponse,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayPacket {
    pub metadata: RelayPacketMetadata,
    pub prompt: String,
    pub human_instructions: String,
    pub project_context_summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ArtifactAction {
    Create,
    Modify,
    Delete,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProposedArtifactChange {
    pub path: String,
    pub action: ArtifactAction,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OpenQuestionStatus {
    Open,
    Resolved,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProposedOpenQuestion {
    pub id: String,
    pub question: String,
    pub status: OpenQuestionStatus,
    pub resolution: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchitectResponsePayload {
    pub schema: u32,
    pub packet_id: String,
    pub project_id: String,
    pub response_type: String,
    pub summary: String,
    pub artifacts: Vec<ProposedArtifactChange>,
    #[serde(default)]
    pub open_questions: Vec<ProposedOpenQuestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchitectResponseEnvelope {
    pub coalition_response: ArchitectResponsePayload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ArtifactDiffStatus {
    New,
    Modified,
    Unchanged,
    Deleted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactPreviewItem {
    pub path: String,
    pub title: String,
    pub action: ArtifactAction,
    pub status: ArtifactDiffStatus,
    pub current_content: Option<String>,
    pub proposed_content: String,
    pub baseline_fingerprint: Option<String>,
    pub baseline_exists: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportPreview {
    pub import_id: String,
    pub packet_id: String,
    pub project_id: String,
    pub summary: String,
    pub artifacts: Vec<ArtifactPreviewItem>,
    pub open_questions: Vec<ProposedOpenQuestion>,
    pub raw_response: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelayHistoryItem {
    pub id: String,
    pub packet_id: Option<String>,
    pub project_id: String,
    pub item_type: String,
    pub summary: String,
    pub status: String,
    pub created_at: String,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceState {
    pub pending_packet: Option<RelayPacket>,
    pub pending_preview: Option<ImportPreview>,
    pub readiness: ReadinessReport,
    pub history: Vec<RelayHistoryItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactContentDetails {
    pub path: String,
    pub content: String,
    pub fingerprint: Option<String>,
    pub exists: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BatchOperation {
    pub path: String,
    pub action: ArtifactAction,
    pub content: String,
    pub baseline_exists: bool,
    pub baseline_fingerprint: Option<String>,
    pub staged_temp_path: Option<String>,
    pub backup_path: Option<String>,
}

#[derive(Error, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelayError {
    #[error("Project not found: {0}")]
    ProjectNotFound(String),
    #[error("Database error: {0}")]
    Database(String),
    #[error("Artifact error: {0}")]
    Artifact(String),
    #[error("Workflow error: {0}")]
    Workflow(String),
    #[error("Cannot perform operation '{operation}' while project is in workflow state {current}")]
    IllegalWorkflowState { current: String, operation: String },
    #[error("No pending relay packet exists for project {0}. An Architect prompt must be prepared before importing.")]
    NoPendingPacket(String),
    #[error("Packet {0} has already been imported and completed")]
    PacketAlreadyImported(String),
    #[error("Packet {0} has been superseded by a newer packet")]
    PacketSuperseded(String),
    #[error("An import is already pending review: import_id={0}")]
    ImportAlreadyPending(String),
    #[error("Import {0} has already been accepted")]
    ImportAlreadyAccepted(String),
    #[error("Duplicate import response: exact response has already been imported")]
    DuplicateImport(String),
    #[error("Unrelated clipboard content: {0}")]
    UnrelatedContent(String),
    #[error("Parse error in response block: {0}")]
    ParseError(String),
    #[error("Schema error: {0}")]
    SchemaError(String),
    #[error("Project ID mismatch: expected {expected}, got {actual}")]
    ProjectIdMismatch { expected: String, actual: String },
    #[error("Packet ID mismatch: expected {expected}, got {actual}")]
    PacketIdMismatch { expected: String, actual: String },
    #[error("Disallowed artifact path: {0}")]
    DisallowedArtifactPath(String),
    #[error("Stale import preview for '{path}': {details}")]
    StaleImportPreview { path: String, details: String },
    #[error("Stale artifact content for '{path}': {details}")]
    StaleArtifactContent { path: String, details: String },
    #[error("Action semantic conflict for '{path}': {details}")]
    ActionSemanticConflict { path: String, details: String },
    #[error("Invalid YAML syntax in '{path}': {error}")]
    InvalidYamlContent { path: String, error: String },
    #[error("Batch recovery required: {0}")]
    BatchRecoveryRequired(String),
    #[error("Parse failure requiring manual recovery: import_id={import_id}, message={message}")]
    ParseFailure {
        import_id: String,
        raw_content: String,
        message: String,
    },
    #[error("Import record not found: {0}")]
    ImportNotFound(String),
    #[error("Import already decided: status is {0}")]
    AlreadyDecided(String),
    #[cfg(test)]
    #[error("Injected test failure: {0}")]
    InjectedFailure(String),
}

impl From<rusqlite::Error> for RelayError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Database(e.to_string())
    }
}

impl From<ArtifactError> for RelayError {
    fn from(e: ArtifactError) -> Self {
        Self::Artifact(e.to_string())
    }
}

pub struct RelayPromptBuilder;

impl RelayPromptBuilder {
    pub fn build_prompt<P: AsRef<Path>>(
        repo_root: P,
        project_id: &str,
        project_name: &str,
        packet_id: &str,
        packet_type: RelayPacketType,
        custom_notes: Option<&str>,
    ) -> (String, String, String) {
        let root = repo_root.as_ref();
        let readiness = ReadinessEvaluator::evaluate(root);

        let mut context_summary = format!(
            "Project Name: {}\nProject ID: {}\nCurrent Architecture State: draft\nReadiness: {}/{} required artifacts ready\n",
            project_name, project_id, readiness.ready_required_count, readiness.total_required_count
        );

        let mut existing_artifacts_text = String::new();
        for item in &readiness.artifacts {
            if let Ok(Some(content)) = ArtifactManager::read_artifact(root, &item.path) {
                let preview: String = content.chars().take(300).collect();
                existing_artifacts_text.push_str(&format!(
                    "\n--- {} ({}) ---\n{}...\n",
                    item.title,
                    item.path,
                    preview.trim()
                ));
            }
        }

        if !existing_artifacts_text.is_empty() {
            context_summary.push_str("\nExisting Project Artifacts Summary:");
            context_summary.push_str(&existing_artifacts_text);
        }

        if let Ok(Some(oq)) = ArtifactManager::read_artifact(root, "design/open-questions.md") {
            context_summary.push_str("\nCurrent Open Questions:\n");
            context_summary.push_str(&oq);
        }

        let human_instructions = format!(
            "You are the ChatGPT Architect for Coalition project '{}'.\n\
            Human Authority Invariants:\n\
            1. The human user owns all product decisions and requirements.\n\
            2. You assist in creating and evolving design specifications.\n\
            3. No AI may freeze architecture; architecture freeze is an explicit human action.\n\
            4. Preserved decisions must not be silently removed or contradicted.\n\
            5. Unresolved questions and architectural tradeoffs must be surfaced clearly.\n\
            6. Return all proposed artifact updates in the structured YAML code block requested below.",
            project_name
        );

        let type_str = match packet_type {
            RelayPacketType::ArchitectInitial => "ARCHITECT_INITIAL",
            RelayPacketType::ArchitectUpdate => "ARCHITECT_UPDATE",
            _ => "ARCHITECT_UPDATE",
        };

        let mut prompt = format!(
            "# COALITION ARCHITECT RELAY PACKET\n\
            ```yaml\n\
            coalition_packet:\n  schema: 1\n  project_id: \"{}\"\n  packet_id: \"{}\"\n  role: ARCHITECT\n  packet_type: {}\n  architecture_version: draft\n  expected_response: ARCHITECT_UPDATE\n\
            ```\n\n\
            {}\n\n\
            ## Current Project Context\n\
            {}\n",
            project_id, packet_id, type_str, human_instructions, context_summary
        );

        if let Some(notes) = custom_notes {
            if !notes.trim().is_empty() {
                prompt.push_str(&format!(
                    "\n## Human User Guidance / Instructions\n{}\n",
                    notes.trim()
                ));
            }
        }

        prompt.push_str(
            "\n## Response Instructions\n\
            Discuss your analysis freely in prose. When proposing artifact additions or updates, include exactly one structured YAML code block as follows:\n\n\
            ```yaml\n\
            coalition_response:\n\
              schema: 1\n\
              packet_id: \"<packet_id>\"\n\
              project_id: \"<project_id>\"\n\
              response_type: \"ARCHITECT_UPDATE\"\n\
              summary: \"<Brief summary of proposed changes>\"\n\
              artifacts:\n\
                - path: \"design/product-vision.md\"\n\
                  action: \"CREATE\" # or MODIFY or DELETE\n\
                  content: |\n\
                    # Product Vision\n\
                    ...\n\
                - path: \"design/requirements.md\"\n\
                  action: \"CREATE\"\n\
                  content: |\n\
                    # Requirements\n\
                    ...\n\
              open_questions:\n\
                - id: \"OQ-1\"\n\
                  question: \"...\"\n\
                  status: \"OPEN\" # or RESOLVED\n\
                  resolution: null\n\
            ```\n\
            Only propose relative paths within the canonical architecture package: \n\
            design/product-vision.md, design/requirements.md, design/architecture.md, design/constraints.md, design/interfaces.md, design/security.md, implementation/implementation-plan.md, implementation/acceptance-criteria.yaml, implementation/test-plan.md, design/open-questions.md, decisions/ADR-*.md.\n"
        );

        (prompt, human_instructions, context_summary)
    }
}

pub struct RelayParser;

impl RelayParser {
    /// Extracts and parses the structured Coalition response from raw clipboard text.
    /// Strictly requires the `coalition_response:` envelope, schema: 1, and response_type: "ARCHITECT_UPDATE".
    pub fn parse_response(
        raw_text: &str,
        expected_project_id: &str,
        expected_packet_id: Option<&str>,
    ) -> Result<ArchitectResponsePayload, RelayError> {
        let block = Self::extract_structured_block(raw_text)?;

        let envelope: ArchitectResponseEnvelope = serde_yaml::from_str(&block).map_err(|e| {
            RelayError::ParseError(format!(
                "Failed to parse response YAML into coalition_response envelope: {}",
                e
            ))
        })?;

        let payload = envelope.coalition_response;

        if payload.schema != 1 {
            return Err(RelayError::SchemaError(format!(
                "Unsupported response schema version {}. Expected 1",
                payload.schema
            )));
        }

        if payload.response_type != "ARCHITECT_UPDATE" {
            return Err(RelayError::SchemaError(format!(
                "Invalid response_type '{}'. Stage 2A Architect responses must have response_type 'ARCHITECT_UPDATE'",
                payload.response_type
            )));
        }

        if payload.project_id != expected_project_id {
            return Err(RelayError::ProjectIdMismatch {
                expected: expected_project_id.to_string(),
                actual: payload.project_id,
            });
        }

        if let Some(expected_pkt) = expected_packet_id {
            if payload.packet_id != expected_pkt {
                return Err(RelayError::PacketIdMismatch {
                    expected: expected_pkt.to_string(),
                    actual: payload.packet_id,
                });
            }
        }

        for art in &payload.artifacts {
            if !ArtifactManager::is_valid_architecture_artifact_path(&art.path) {
                return Err(RelayError::DisallowedArtifactPath(art.path.clone()));
            }
        }

        Ok(payload)
    }

    fn extract_structured_block(text: &str) -> Result<String, RelayError> {
        // 1. Check for markdown code fences (```yaml ... ``` or ```json ... ``` or plain ``` ... ```)
        let lines: Vec<&str> = text.lines().collect();
        let mut in_fence = false;
        let mut fence_lines = Vec::new();
        let mut candidates = Vec::new();

        for line in &lines {
            let trimmed = line.trim();
            if trimmed.starts_with("```") {
                if in_fence {
                    // Close fence
                    let block = fence_lines.join("\n");
                    candidates.push(block);
                    fence_lines.clear();
                    in_fence = false;
                } else {
                    // Open fence
                    in_fence = true;
                }
            } else if in_fence {
                fence_lines.push(*line);
            }
        }

        for candidate in candidates {
            if candidate.contains("coalition_response:") {
                return Ok(candidate);
            }
        }

        // 2. Check if raw text contains coalition_response:
        if let Some(idx) = text.find("coalition_response:") {
            let sub = &text[idx..];
            return Ok(sub.to_string());
        }

        Err(RelayError::UnrelatedContent(
            "No structured 'coalition_response:' block found in clipboard text. Ensure you copied ChatGPT's response containing the YAML code block.".to_string(),
        ))
    }
}

pub struct RelayService;

impl RelayService {
    /// Prepares a new Architect relay packet, persists it in SQLite as PENDING,
    /// advances workflow from DRAFT to ARCHITECTING if applicable, and logs the event.
    pub fn prepare_architect_packet<P: AsRef<Path>>(
        conn: &mut Connection,
        repo_root: P,
        project_id: &str,
        custom_notes: Option<&str>,
    ) -> Result<RelayPacket, RelayError> {
        let root = repo_root.as_ref();

        // 1. Verify project exists
        let name: String = conn
            .query_row(
                "SELECT name FROM projects WHERE project_id = ?1",
                params![project_id],
                |r| r.get(0),
            )
            .map_err(|_| RelayError::ProjectNotFound(project_id.to_string()))?;

        // 2. Check workflow state
        let current_state_str: String = conn
            .query_row(
                "SELECT state FROM workflow_state WHERE project_id = ?1",
                params![project_id],
                |r| r.get(0),
            )
            .map_err(|e| RelayError::Workflow(e.to_string()))?;

        let current_state: WorkflowState = current_state_str
            .parse()
            .map_err(|e: workflow::WorkflowError| RelayError::Workflow(e.to_string()))?;

        if current_state != WorkflowState::Draft
            && current_state != WorkflowState::Architecting
            && current_state != WorkflowState::ReadyToFreeze
        {
            return Err(RelayError::IllegalWorkflowState {
                current: current_state.to_string(),
                operation: "prepare_architect_relay_packet".to_string(),
            });
        }

        // Advance DRAFT -> ARCHITECTING
        if current_state == WorkflowState::Draft {
            let tx = conn.transaction()?;
            let (next_state, next_resume) = workflow::compute_transition(
                current_state,
                None,
                WorkflowAction::StartArchitecting,
            )
            .map_err(|e| RelayError::Workflow(e.to_string()))?;

            let now = chrono::Utc::now().to_rfc3339();
            tx.execute(
                "UPDATE workflow_state SET state = ?1, resume_state = ?2, revision = revision + 1, updated_at = ?3 WHERE project_id = ?4",
                params![next_state.to_string(), next_resume.map(|s| s.to_string()), now, project_id],
            )?;

            ActivityManager::record_event(
                &tx,
                project_id,
                "WORKFLOW_STATE_CHANGED",
                "Human",
                &format!(
                    "Project transitioned from {} to {}",
                    current_state, next_state
                ),
                Some(&serde_json::json!({
                    "from_state": current_state.to_string(),
                    "to_state": next_state.to_string(),
                    "action": "START_ARCHITECTING"
                })),
            )
            .map_err(|e| RelayError::Database(e.to_string()))?;

            tx.commit()?;
        }

        // 3. Determine packet type: ARCHITECT_INITIAL if no prior packets, else ARCHITECT_UPDATE
        let prior_packets_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM relay_packets WHERE project_id = ?1",
            params![project_id],
            |r| r.get(0),
        )?;

        let packet_type = if prior_packets_count == 0 {
            RelayPacketType::ArchitectInitial
        } else {
            RelayPacketType::ArchitectUpdate
        };

        // 4. Supersede any previous PENDING or DECISION_PENDING packets
        let old_packets: Vec<String> = {
            let mut stmt = conn.prepare(
                "SELECT packet_id FROM relay_packets WHERE project_id = ?1 AND status IN ('PENDING', 'DECISION_PENDING')",
            )?;
            let rows = stmt.query_map(params![project_id], |r| r.get(0))?;
            rows.filter_map(|r| r.ok()).collect()
        };

        for old_pkt in &old_packets {
            conn.execute(
                "UPDATE relay_packets SET status = 'SUPERSEDED' WHERE packet_id = ?1",
                params![old_pkt],
            )?;
            Self::record_relay_history(
                conn,
                project_id,
                Some(old_pkt),
                None,
                "PACKET_SUPERSEDED",
                "Relay packet superseded by newly generated packet",
                None,
            )?;
        }

        // Supersede any pending import
        conn.execute(
            "UPDATE relay_imports SET decision = 'SUPERSEDED' WHERE project_id = ?1 AND decision = 'PENDING'",
            params![project_id],
        )?;

        // 5. Build packet
        let packet_id = Uuid::new_v4().to_string();
        let created_at = chrono::Utc::now().to_rfc3339();
        let (prompt, human_instructions, context_summary) = RelayPromptBuilder::build_prompt(
            root,
            project_id,
            &name,
            &packet_id,
            packet_type,
            custom_notes,
        );

        let type_str = match packet_type {
            RelayPacketType::ArchitectInitial => "ARCHITECT_INITIAL",
            RelayPacketType::ArchitectUpdate => "ARCHITECT_UPDATE",
            _ => "ARCHITECT_UPDATE",
        };

        let metadata = RelayPacketMetadata {
            schema: 1,
            project_id: project_id.to_string(),
            packet_id: packet_id.clone(),
            role: RelayRole::Architect,
            packet_type,
            architecture_version: "draft".to_string(),
            expected_response: RelayExpectedResponse::ArchitectUpdate,
            created_at: created_at.clone(),
        };

        // 6. Persist packet in SQLite
        conn.execute(
            "INSERT INTO relay_packets (packet_id, project_id, role, packet_type, architecture_version, expected_response, prompt, created_at, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'PENDING')",
            params![
                packet_id,
                project_id,
                "ARCHITECT",
                type_str,
                "draft",
                "ARCHITECT_UPDATE",
                prompt,
                created_at
            ],
        )?;

        Self::record_relay_history(
            conn,
            project_id,
            Some(&packet_id),
            None,
            "PACKET_GENERATED",
            &format!("Generated Architect relay prompt packet ({})", type_str),
            Some(&serde_json::json!({
                "packet_type": type_str,
                "packet_id": packet_id,
            })),
        )?;

        let packet = RelayPacket {
            metadata,
            prompt,
            human_instructions,
            project_context_summary: context_summary,
        };

        Ok(packet)
    }

    /// Retrieves current active pending packet for project if one exists.
    pub fn get_pending_packet(
        conn: &Connection,
        project_id: &str,
    ) -> Result<Option<RelayPacket>, RelayError> {
        let res = conn.query_row(
            "SELECT packet_id, project_id, role, packet_type, architecture_version, expected_response, prompt, created_at
             FROM relay_packets WHERE project_id = ?1 AND status IN ('PENDING', 'DECISION_PENDING') ORDER BY created_at DESC LIMIT 1",
            params![project_id],
            |row| {
                let packet_id: String = row.get(0)?;
                let proj_id: String = row.get(1)?;
                let role_str: String = row.get(2)?;
                let type_str: String = row.get(3)?;
                let arch_ver: String = row.get(4)?;
                let expected_str: String = row.get(5)?;
                let prompt: String = row.get(6)?;
                let created_at: String = row.get(7)?;

                let role = match role_str.as_str() {
                    "ARCHITECT" => RelayRole::Architect,
                    "REVIEWER" => RelayRole::Reviewer,
                    _ => RelayRole::Architect,
                };

                let packet_type = match type_str.as_str() {
                    "ARCHITECT_INITIAL" => RelayPacketType::ArchitectInitial,
                    "ARCHITECT_UPDATE" => RelayPacketType::ArchitectUpdate,
                    "ARCHITECTURE_REVISION" => RelayPacketType::ArchitectureRevision,
                    "REVIEW_REQUEST" => RelayPacketType::ReviewRequest,
                    "REVIEW_VERDICT" => RelayPacketType::ReviewVerdict,
                    _ => RelayPacketType::ArchitectUpdate,
                };

                let expected_response = match expected_str.as_str() {
                    "ARCHITECT_UPDATE" => RelayExpectedResponse::ArchitectUpdate,
                    "REVIEW_VERDICT" => RelayExpectedResponse::ReviewVerdict,
                    _ => RelayExpectedResponse::ArchitectUpdate,
                };

                Ok(RelayPacket {
                    metadata: RelayPacketMetadata {
                        schema: 1,
                        project_id: proj_id,
                        packet_id,
                        role,
                        packet_type,
                        architecture_version: arch_ver,
                        expected_response,
                        created_at,
                    },
                    prompt,
                    human_instructions: String::new(),
                    project_context_summary: String::new(),
                })
            },
        ).optional()?;

        Ok(res)
    }

    /// Processes imported clipboard text, validates, parses, computes diff preview,
    /// captures optimistic concurrency baselines, validates action preconditions,
    /// and persists import record in SQLite without mutating project artifacts.
    pub fn process_import<P: AsRef<Path>>(
        conn: &Connection,
        repo_root: P,
        project_id: &str,
        raw_clipboard_text: &str,
    ) -> Result<ImportPreview, RelayError> {
        let root = repo_root.as_ref();

        // 1. Verify workflow state
        let state_str: String = conn
            .query_row(
                "SELECT state FROM workflow_state WHERE project_id = ?1",
                params![project_id],
                |r| r.get(0),
            )
            .map_err(|_| RelayError::ProjectNotFound(project_id.to_string()))?;

        let current_state: WorkflowState = state_str
            .parse()
            .map_err(|e: workflow::WorkflowError| RelayError::Workflow(e.to_string()))?;

        if current_state != WorkflowState::Draft
            && current_state != WorkflowState::Architecting
            && current_state != WorkflowState::ReadyToFreeze
        {
            return Err(RelayError::IllegalWorkflowState {
                current: current_state.to_string(),
                operation: "import_from_clipboard".to_string(),
            });
        }

        // 2. Strict relay lifecycle: MUST have an active pending packet
        let pending = Self::get_pending_packet(conn, project_id)?
            .ok_or_else(|| RelayError::NoPendingPacket(project_id.to_string()))?;

        // 3. Prevent multiple pending imports
        let existing_pending_import: Option<String> = conn
            .query_row(
                "SELECT import_id FROM relay_imports WHERE project_id = ?1 AND decision = 'PENDING' AND parse_status = 'SUCCESS'",
                params![project_id],
                |r| r.get(0),
            )
            .optional()?;

        if let Some(existing_id) = existing_pending_import {
            return Err(RelayError::ImportAlreadyPending(existing_id));
        }

        let import_id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        // 4. Parse response with exact envelope and matching IDs
        let parsed = match RelayParser::parse_response(
            raw_clipboard_text,
            project_id,
            Some(&pending.metadata.packet_id),
        ) {
            Ok(p) => p,
            Err(e) => {
                let error_msg = e.to_string();
                conn.execute(
                    "INSERT INTO relay_imports (import_id, packet_id, project_id, raw_content, parsed_payload_json, parse_status, error_message, decision, imported_at, decided_at)
                     VALUES (?1, ?2, ?3, ?4, NULL, 'PARSE_ERROR', ?5, 'PENDING', ?6, NULL)",
                    params![import_id, pending.metadata.packet_id, project_id, raw_clipboard_text, error_msg, now],
                )?;

                Self::record_relay_history(
                    conn,
                    project_id,
                    Some(&pending.metadata.packet_id),
                    Some(&import_id),
                    "IMPORT_PARSE_FAILURE",
                    &format!("Failed to parse import response: {}", error_msg),
                    Some(&serde_json::json!({ "error": error_msg })),
                )?;

                return Err(RelayError::ParseFailure {
                    import_id,
                    raw_content: raw_clipboard_text.to_string(),
                    message: error_msg,
                });
            }
        };

        // 5. Check duplicate import (prevent identical raw content re-import)
        let dup_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM relay_imports WHERE project_id = ?1 AND raw_content = ?2 AND decision = 'ACCEPTED'",
            params![project_id, raw_clipboard_text],
            |r| r.get(0),
        )?;
        if dup_count > 0 {
            return Err(RelayError::DuplicateImport(
                "Identical architect response has already been accepted".to_string(),
            ));
        }

        // 6. Compute diff, baseline fingerprints, and validate action preconditions
        let mut preview_items = Vec::new();
        let policy = readiness::default_readiness_policy();

        for art in &parsed.artifacts {
            let current = ArtifactManager::read_artifact(root, &art.path)
                .ok()
                .flatten();
            let current_exists = current.is_some();
            let baseline_fp = ArtifactManager::compute_file_fingerprint(root, &art.path)?;

            // Enforce ArtifactAction semantic preconditions
            match art.action {
                ArtifactAction::Create => {
                    if current_exists {
                        return Err(RelayError::ActionSemanticConflict {
                            path: art.path.clone(),
                            details: format!(
                                "Action CREATE requires target to not exist, but {} already exists on disk",
                                art.path
                            ),
                        });
                    }
                }
                ArtifactAction::Modify => {
                    if !current_exists {
                        return Err(RelayError::ActionSemanticConflict {
                            path: art.path.clone(),
                            details: format!(
                                "Action MODIFY requires target to exist, but {} is absent from disk",
                                art.path
                            ),
                        });
                    }
                }
                ArtifactAction::Delete => {
                    if !current_exists {
                        return Err(RelayError::ActionSemanticConflict {
                            path: art.path.clone(),
                            details: format!(
                                "Action DELETE requires target to exist, but {} is absent from disk",
                                art.path
                            ),
                        });
                    }
                }
            }

            let title = policy
                .rules
                .iter()
                .find(|r| r.path == art.path)
                .map(|r| r.title.as_str())
                .unwrap_or("Architecture Artifact");

            let status = match (&art.action, &current) {
                (ArtifactAction::Delete, _) => ArtifactDiffStatus::Deleted,
                (ArtifactAction::Create, _) => ArtifactDiffStatus::New,
                (ArtifactAction::Modify, Some(existing)) => {
                    if existing.trim() == art.content.trim() {
                        ArtifactDiffStatus::Unchanged
                    } else {
                        ArtifactDiffStatus::Modified
                    }
                }
                (ArtifactAction::Modify, None) => ArtifactDiffStatus::Modified,
            };

            preview_items.push(ArtifactPreviewItem {
                path: art.path.clone(),
                title: title.to_string(),
                action: art.action,
                status,
                current_content: current,
                proposed_content: art.content.clone(),
                baseline_fingerprint: baseline_fp,
                baseline_exists: current_exists,
            });
        }

        // 7. Incorporate open-questions.md into the batch if questions are present
        if !parsed.open_questions.is_empty() {
            let oq_path = "design/open-questions.md";
            let mut oq_md = String::from("# Architectural Open Questions\n\n");
            for q in &parsed.open_questions {
                let status_str = match q.status {
                    OpenQuestionStatus::Open => "OPEN",
                    OpenQuestionStatus::Resolved => "RESOLVED",
                };
                oq_md.push_str(&format!("## [{}] {}\n", status_str, q.id));
                oq_md.push_str(&format!("**Question:** {}\n\n", q.question));
                if let Some(ref res) = q.resolution {
                    oq_md.push_str(&format!("**Resolution:** {}\n\n", res));
                }
            }

            let current_oq = ArtifactManager::read_artifact(root, oq_path).ok().flatten();
            let oq_exists = current_oq.is_some();
            let oq_fp = ArtifactManager::compute_file_fingerprint(root, oq_path)?;
            let oq_action = if oq_exists {
                ArtifactAction::Modify
            } else {
                ArtifactAction::Create
            };

            let oq_status = if !oq_exists {
                ArtifactDiffStatus::New
            } else if current_oq.as_deref().unwrap_or("").trim() == oq_md.trim() {
                ArtifactDiffStatus::Unchanged
            } else {
                ArtifactDiffStatus::Modified
            };

            preview_items.push(ArtifactPreviewItem {
                path: oq_path.to_string(),
                title: "Open Questions".to_string(),
                action: oq_action,
                status: oq_status,
                current_content: current_oq,
                proposed_content: oq_md,
                baseline_fingerprint: oq_fp,
                baseline_exists: oq_exists,
            });
        }

        let preview = ImportPreview {
            import_id: import_id.clone(),
            packet_id: parsed.packet_id.clone(),
            project_id: project_id.to_string(),
            summary: parsed.summary,
            artifacts: preview_items,
            open_questions: parsed.open_questions,
            raw_response: raw_clipboard_text.to_string(),
        };

        let preview_json =
            serde_json::to_string(&preview).map_err(|e| RelayError::ParseError(e.to_string()))?;

        // 8. Persist import record and update packet status to DECISION_PENDING
        conn.execute(
            "INSERT INTO relay_imports (import_id, packet_id, project_id, raw_content, parsed_payload_json, parse_status, error_message, decision, imported_at, decided_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'SUCCESS', NULL, 'PENDING', ?6, NULL)",
            params![import_id, parsed.packet_id, project_id, raw_clipboard_text, preview_json, now],
        )?;

        conn.execute(
            "UPDATE relay_packets SET status = 'DECISION_PENDING' WHERE packet_id = ?1",
            params![parsed.packet_id],
        )?;

        Self::record_relay_history(
            conn,
            project_id,
            Some(&parsed.packet_id),
            Some(&import_id),
            "IMPORT_PARSED",
            &format!("Parsed response: {}", preview.summary),
            Some(&serde_json::json!({
                "artifacts_count": preview.artifacts.len(),
                "summary": preview.summary
            })),
        )?;

        Ok(preview)
    }

    /// Retries parsing with edited text for a failed import record.
    pub fn retry_parse_import<P: AsRef<Path>>(
        conn: &Connection,
        repo_root: P,
        project_id: &str,
        import_id: &str,
        edited_raw_text: &str,
    ) -> Result<ImportPreview, RelayError> {
        let root = repo_root.as_ref();
        let pending = Self::get_pending_packet(conn, project_id)?
            .ok_or_else(|| RelayError::NoPendingPacket(project_id.to_string()))?;

        // Verify import record is unresolved
        let decision: String = conn
            .query_row(
                "SELECT decision FROM relay_imports WHERE import_id = ?1 AND project_id = ?2",
                params![import_id, project_id],
                |r| r.get(0),
            )
            .map_err(|_| RelayError::ImportNotFound(import_id.to_string()))?;

        if decision != "PENDING" {
            return Err(RelayError::AlreadyDecided(decision));
        }

        let parsed = RelayParser::parse_response(
            edited_raw_text,
            project_id,
            Some(&pending.metadata.packet_id),
        )?;

        let policy = readiness::default_readiness_policy();
        let mut preview_items = Vec::new();

        for art in &parsed.artifacts {
            let current = ArtifactManager::read_artifact(root, &art.path)
                .ok()
                .flatten();
            let current_exists = current.is_some();
            let baseline_fp = ArtifactManager::compute_file_fingerprint(root, &art.path)?;

            let title = policy
                .rules
                .iter()
                .find(|r| r.path == art.path)
                .map(|r| r.title.as_str())
                .unwrap_or("Architecture Artifact");

            let status = match (&art.action, &current) {
                (ArtifactAction::Delete, _) => ArtifactDiffStatus::Deleted,
                (ArtifactAction::Create, _) => ArtifactDiffStatus::New,
                (ArtifactAction::Modify, Some(existing)) => {
                    if existing.trim() == art.content.trim() {
                        ArtifactDiffStatus::Unchanged
                    } else {
                        ArtifactDiffStatus::Modified
                    }
                }
                (ArtifactAction::Modify, None) => ArtifactDiffStatus::Modified,
            };

            preview_items.push(ArtifactPreviewItem {
                path: art.path.clone(),
                title: title.to_string(),
                action: art.action,
                status,
                current_content: current,
                proposed_content: art.content.clone(),
                baseline_fingerprint: baseline_fp,
                baseline_exists: current_exists,
            });
        }

        if !parsed.open_questions.is_empty() {
            let oq_path = "design/open-questions.md";
            let mut oq_md = String::from("# Architectural Open Questions\n\n");
            for q in &parsed.open_questions {
                let status_str = match q.status {
                    OpenQuestionStatus::Open => "OPEN",
                    OpenQuestionStatus::Resolved => "RESOLVED",
                };
                oq_md.push_str(&format!("## [{}] {}\n", status_str, q.id));
                oq_md.push_str(&format!("**Question:** {}\n\n", q.question));
                if let Some(ref res) = q.resolution {
                    oq_md.push_str(&format!("**Resolution:** {}\n\n", res));
                }
            }

            let current_oq = ArtifactManager::read_artifact(root, oq_path).ok().flatten();
            let oq_exists = current_oq.is_some();
            let oq_fp = ArtifactManager::compute_file_fingerprint(root, oq_path)?;
            let oq_action = if oq_exists {
                ArtifactAction::Modify
            } else {
                ArtifactAction::Create
            };

            let oq_status = if !oq_exists {
                ArtifactDiffStatus::New
            } else if current_oq.as_deref().unwrap_or("").trim() == oq_md.trim() {
                ArtifactDiffStatus::Unchanged
            } else {
                ArtifactDiffStatus::Modified
            };

            preview_items.push(ArtifactPreviewItem {
                path: oq_path.to_string(),
                title: "Open Questions".to_string(),
                action: oq_action,
                status: oq_status,
                current_content: current_oq,
                proposed_content: oq_md,
                baseline_fingerprint: oq_fp,
                baseline_exists: oq_exists,
            });
        }

        let preview = ImportPreview {
            import_id: import_id.to_string(),
            packet_id: parsed.packet_id.clone(),
            project_id: project_id.to_string(),
            summary: parsed.summary,
            artifacts: preview_items,
            open_questions: parsed.open_questions,
            raw_response: edited_raw_text.to_string(),
        };

        let preview_json =
            serde_json::to_string(&preview).map_err(|e| RelayError::ParseError(e.to_string()))?;

        conn.execute(
            "UPDATE relay_imports SET raw_content = ?1, parsed_payload_json = ?2, parse_status = 'SUCCESS', error_message = NULL
             WHERE import_id = ?3 AND project_id = ?4",
            params![edited_raw_text, preview_json, import_id, project_id],
        )?;

        conn.execute(
            "UPDATE relay_packets SET status = 'DECISION_PENDING' WHERE packet_id = ?1",
            params![parsed.packet_id],
        )?;

        Self::record_relay_history(
            conn,
            project_id,
            Some(&parsed.packet_id),
            Some(import_id),
            "IMPORT_PARSED",
            &format!("Successfully recovered import: {}", preview.summary),
            None,
        )?;

        Ok(preview)
    }

    /// Explicitly accepts an import using the crash-safe batch transaction protocol.
    /// 1. Verifies legal workflow state.
    /// 2. Re-validates optimistic concurrency against disk (aborts if any target changed).
    /// 3. Re-validates semantic preconditions (CREATE/MODIFY/DELETE).
    /// 4. Writes batch recovery journal in SQLite (`STAGING`).
    /// 5. Stages all replacement files (`STAGED`).
    /// 6. Replaces files in deterministic alphabetical order with backups (`COMMITTING`).
    /// 7. Updates journal (`COMMITTED`).
    /// 8. Finalizes SQLite records, cleans up backups, reconciles workflow state to READY_TO_FREEZE if ready.
    /// 9. Deletes batch journal entry.
    pub fn accept_import<P: AsRef<Path>>(
        conn: &Connection,
        repo_root: P,
        project_id: &str,
        import_id: &str,
    ) -> Result<ReadinessReport, RelayError> {
        let root = repo_root.as_ref();

        // 1. Verify workflow state
        let state_str: String = conn
            .query_row(
                "SELECT state FROM workflow_state WHERE project_id = ?1",
                params![project_id],
                |r| r.get(0),
            )
            .map_err(|_| RelayError::ProjectNotFound(project_id.to_string()))?;

        let current_state: WorkflowState = state_str
            .parse()
            .map_err(|e: workflow::WorkflowError| RelayError::Workflow(e.to_string()))?;

        if current_state != WorkflowState::Architecting
            && current_state != WorkflowState::ReadyToFreeze
        {
            return Err(RelayError::IllegalWorkflowState {
                current: current_state.to_string(),
                operation: "accept_relay_import".to_string(),
            });
        }

        // 2. Fetch import record
        let (parsed_json_opt, decision, packet_id): (Option<String>, String, Option<String>) = conn
            .query_row(
                "SELECT parsed_payload_json, decision, packet_id FROM relay_imports WHERE import_id = ?1 AND project_id = ?2",
                params![import_id, project_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|_| RelayError::ImportNotFound(import_id.to_string()))?;

        if decision == "ACCEPTED" {
            return Err(RelayError::ImportAlreadyAccepted(import_id.to_string()));
        }
        if decision != "PENDING" {
            return Err(RelayError::AlreadyDecided(decision));
        }

        let parsed_json = parsed_json_opt.ok_or_else(|| {
            RelayError::ParseError("Import record has no parsed payload".to_string())
        })?;

        let preview: ImportPreview = serde_json::from_str(&parsed_json)
            .map_err(|e| RelayError::ParseError(e.to_string()))?;

        // 3. Optimistic concurrency & semantic precondition revalidation across ALL items
        for item in &preview.artifacts {
            let current_exists = ArtifactManager::read_artifact(root, &item.path)?.is_some();
            let current_fp = ArtifactManager::compute_file_fingerprint(root, &item.path)?;

            // Baseline conflict check
            if current_exists != item.baseline_exists || current_fp != item.baseline_fingerprint {
                return Err(RelayError::StaleImportPreview {
                    path: item.path.clone(),
                    details: format!(
                        "Target file '{}' was modified on disk since preview was generated. Expected exists={}, fp={:?}, but found exists={}, fp={:?}",
                        item.path, item.baseline_exists, item.baseline_fingerprint, current_exists, current_fp
                    ),
                });
            }

            // Semantic action precondition check
            match item.action {
                ArtifactAction::Create => {
                    if current_exists {
                        return Err(RelayError::ActionSemanticConflict {
                            path: item.path.clone(),
                            details: format!(
                                "Target '{}' already exists for action CREATE",
                                item.path
                            ),
                        });
                    }
                }
                ArtifactAction::Modify => {
                    if !current_exists {
                        return Err(RelayError::ActionSemanticConflict {
                            path: item.path.clone(),
                            details: format!(
                                "Target '{}' does not exist for action MODIFY",
                                item.path
                            ),
                        });
                    }
                }
                ArtifactAction::Delete => {
                    if !current_exists {
                        return Err(RelayError::ActionSemanticConflict {
                            path: item.path.clone(),
                            details: format!(
                                "Target '{}' does not exist for action DELETE",
                                item.path
                            ),
                        });
                    }
                }
            }
        }

        // 4. Build deterministic batch operations (sorted alphabetically by path)
        let mut operations: Vec<BatchOperation> = preview
            .artifacts
            .iter()
            .map(|item| BatchOperation {
                path: item.path.clone(),
                action: item.action,
                content: item.proposed_content.clone(),
                baseline_exists: item.baseline_exists,
                baseline_fingerprint: item.baseline_fingerprint.clone(),
                staged_temp_path: None,
                backup_path: None,
            })
            .collect();

        operations.sort_by(|a, b| a.path.cmp(&b.path));

        let now = chrono::Utc::now().to_rfc3339();
        let ops_json = serde_json::to_string(&operations)
            .map_err(|e| RelayError::ParseError(e.to_string()))?;

        // 5. Insert batch journal in STAGING phase
        conn.execute(
            "INSERT OR REPLACE INTO relay_import_batch_journal (import_id, project_id, packet_id, phase, operations_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'STAGING', ?4, ?5, ?6)",
            params![import_id, project_id, preview.packet_id, ops_json, now, now],
        )?;

        // 6. Stage all replacement content
        let coalition_dir = ArtifactManager::resolve_coalition_dir(root)?;
        for op in &mut operations {
            if op.action == ArtifactAction::Create || op.action == ArtifactAction::Modify {
                let target_path = coalition_dir.join(&op.path);
                let parent = target_path.parent().ok_or_else(|| {
                    RelayError::Artifact("No parent directory for target path".to_string())
                })?;
                if !parent.exists() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        RelayError::Artifact(format!("Failed to create parent directory: {}", e))
                    })?;
                }

                let temp_name = format!(
                    "staged.{}.tmp.{}",
                    op.path.replace('/', "_"),
                    Uuid::new_v4()
                );
                let temp_path = parent.join(&temp_name);

                {
                    use std::io::Write;
                    let mut f = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&temp_path)
                        .map_err(|e| {
                            RelayError::Artifact(format!("Failed to create temp file: {}", e))
                        })?;
                    f.write_all(op.content.as_bytes()).map_err(|e| {
                        RelayError::Artifact(format!("Failed to write temp file: {}", e))
                    })?;
                    f.sync_all().map_err(|e| {
                        RelayError::Artifact(format!("Failed to sync temp file: {}", e))
                    })?;
                }

                op.staged_temp_path = Some(temp_path.to_string_lossy().to_string());
            }
        }

        let staged_ops_json = serde_json::to_string(&operations)
            .map_err(|e| RelayError::ParseError(e.to_string()))?;
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE relay_import_batch_journal SET phase = 'STAGED', operations_json = ?1, updated_at = ?2 WHERE import_id = ?3",
            params![staged_ops_json, now, import_id],
        )?;

        #[cfg(test)]
        {
            if INJECTED_BATCH_FAILURE.with(|f| f.get())
                == InjectedBatchFailure::BeforeFirstReplacement
            {
                return Err(RelayError::InjectedFailure(
                    "BeforeFirstReplacement".to_string(),
                ));
            }
        }

        // 7. Commit phase: apply changes with backups
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE relay_import_batch_journal SET phase = 'COMMITTING', updated_at = ?1 WHERE import_id = ?2",
            params![now, import_id],
        )?;

        #[allow(unused_variables)]
        let total_ops = operations.len();
        for i in 0..total_ops {
            let (action, path, staged_temp_path) = {
                let op = &operations[i];
                (op.action, op.path.clone(), op.staged_temp_path.clone())
            };
            let target_path = coalition_dir.join(&path);

            #[cfg(test)]
            {
                if i == 1
                    && INJECTED_BATCH_FAILURE.with(|f| f.get())
                        == InjectedBatchFailure::AfterFirstReplacement
                {
                    return Err(RelayError::InjectedFailure(
                        "AfterFirstReplacement".to_string(),
                    ));
                }
                if i == total_ops / 2
                    && total_ops > 1
                    && INJECTED_BATCH_FAILURE.with(|f| f.get()) == InjectedBatchFailure::MidBatch
                {
                    return Err(RelayError::InjectedFailure("MidBatch".to_string()));
                }
                if action == ArtifactAction::Delete
                    && INJECTED_BATCH_FAILURE.with(|f| f.get())
                        == InjectedBatchFailure::DuringDeleteOperation
                {
                    return Err(RelayError::InjectedFailure(
                        "DuringDeleteOperation".to_string(),
                    ));
                }
            }

            match action {
                ArtifactAction::Create => {
                    let temp_path = Path::new(staged_temp_path.as_ref().unwrap());
                    ArtifactManager::replace_file_atomically(temp_path, &target_path)?;
                }
                ArtifactAction::Modify => {
                    let temp_path = Path::new(staged_temp_path.as_ref().unwrap());
                    let backup_name = format!(
                        "{}.bak.{}",
                        target_path.file_name().unwrap().to_string_lossy(),
                        Uuid::new_v4()
                    );
                    let backup_path = target_path.parent().unwrap().join(&backup_name);

                    // Create explicit backup before replacing
                    std::fs::copy(&target_path, &backup_path).map_err(|e| {
                        RelayError::Artifact(format!("Failed to create batch backup: {}", e))
                    })?;
                    operations[i].backup_path = Some(backup_path.to_string_lossy().to_string());

                    let interim_ops_json = serde_json::to_string(&operations)
                        .map_err(|e| RelayError::ParseError(e.to_string()))?;
                    conn.execute(
                        "UPDATE relay_import_batch_journal SET operations_json = ?1 WHERE import_id = ?2",
                        params![interim_ops_json, import_id],
                    )?;

                    ArtifactManager::replace_file_atomically(temp_path, &target_path)?;
                }
                ArtifactAction::Delete => {
                    let backup_name = format!(
                        "{}.bak.{}",
                        target_path.file_name().unwrap().to_string_lossy(),
                        Uuid::new_v4()
                    );
                    let backup_path = target_path.parent().unwrap().join(&backup_name);
                    std::fs::rename(&target_path, &backup_path).map_err(|e| {
                        RelayError::Artifact(format!(
                            "Failed to rename target to backup for DELETE: {}",
                            e
                        ))
                    })?;
                    operations[i].backup_path = Some(backup_path.to_string_lossy().to_string());

                    let interim_ops_json = serde_json::to_string(&operations)
                        .map_err(|e| RelayError::ParseError(e.to_string()))?;
                    conn.execute(
                        "UPDATE relay_import_batch_journal SET operations_json = ?1 WHERE import_id = ?2",
                        params![interim_ops_json, import_id],
                    )?;
                }
            }
        }

        #[cfg(test)]
        {
            if INJECTED_BATCH_FAILURE.with(|f| f.get())
                == InjectedBatchFailure::AfterFinalReplacement
            {
                return Err(RelayError::InjectedFailure(
                    "AfterFinalReplacement".to_string(),
                ));
            }
        }

        // 8. Mark journal COMMITTED
        let committed_ops_json = serde_json::to_string(&operations)
            .map_err(|e| RelayError::ParseError(e.to_string()))?;
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE relay_import_batch_journal SET phase = 'COMMITTED', operations_json = ?1, updated_at = ?2 WHERE import_id = ?3",
            params![committed_ops_json, now, import_id],
        )?;

        #[cfg(test)]
        {
            if INJECTED_BATCH_FAILURE.with(|f| f.get())
                == InjectedBatchFailure::BeforeSqliteDecisionUpdate
            {
                return Err(RelayError::InjectedFailure(
                    "BeforeSqliteDecisionUpdate".to_string(),
                ));
            }
        }

        // 9. Update SQLite bookkeeping
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE relay_imports SET decision = 'ACCEPTED', decided_at = ?1 WHERE import_id = ?2",
            params![now, import_id],
        )?;

        if let Some(ref pkt_id) = packet_id {
            conn.execute(
                "UPDATE relay_packets SET status = 'IMPORTED' WHERE packet_id = ?1",
                params![pkt_id],
            )?;
        }

        let affected_paths: Vec<String> =
            preview.artifacts.iter().map(|a| a.path.clone()).collect();
        ActivityManager::record_event(
            conn,
            project_id,
            "ARCHITECT_IMPORT_ACCEPTED",
            "Human",
            &format!("Accepted architect changes: {}", preview.summary),
            Some(&serde_json::json!({
                "import_id": import_id,
                "summary": preview.summary,
                "affected_artifacts": affected_paths,
            })),
        )
        .map_err(|e| RelayError::Database(e.to_string()))?;

        Self::record_relay_history(
            conn,
            project_id,
            packet_id.as_deref(),
            Some(import_id),
            "IMPORT_ACCEPTED",
            &format!("Accepted changes: {}", preview.summary),
            Some(&serde_json::json!({ "summary": preview.summary })),
        )?;

        // 10. Re-evaluate readiness and reconcile workflow state
        let readiness = ReadinessEvaluator::evaluate(root);
        Self::reconcile_workflow_readiness_state(conn, project_id, &readiness, "Human")?;

        #[cfg(test)]
        {
            if INJECTED_BATCH_FAILURE.with(|f| f.get())
                == InjectedBatchFailure::AfterSqliteDecisionUpdate
            {
                return Err(RelayError::InjectedFailure(
                    "AfterSqliteDecisionUpdate".to_string(),
                ));
            }
        }

        // 11. Clean up backups and delete batch journal
        for op in &operations {
            if let Some(ref bak) = op.backup_path {
                let p = Path::new(bak);
                if p.exists() {
                    let _ = std::fs::remove_file(p);
                }
            }
        }

        conn.execute(
            "DELETE FROM relay_import_batch_journal WHERE import_id = ?1",
            params![import_id],
        )?;

        Ok(readiness)
    }

    /// Explicitly rejects an import. Project files on disk remain untouched.
    pub fn reject_import(
        conn: &Connection,
        project_id: &str,
        import_id: &str,
    ) -> Result<(), RelayError> {
        let (decision, packet_id): (String, Option<String>) = conn
            .query_row(
                "SELECT decision, packet_id FROM relay_imports WHERE import_id = ?1 AND project_id = ?2",
                params![import_id, project_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| RelayError::ImportNotFound(import_id.to_string()))?;

        if decision != "PENDING" {
            return Err(RelayError::AlreadyDecided(decision));
        }

        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE relay_imports SET decision = 'REJECTED', decided_at = ?1 WHERE import_id = ?2",
            params![now, import_id],
        )?;

        // Reset packet status to PENDING so user can try again or wait for another prompt
        if let Some(ref pkt_id) = packet_id {
            conn.execute(
                "UPDATE relay_packets SET status = 'PENDING' WHERE packet_id = ?1",
                params![pkt_id],
            )?;
        }

        ActivityManager::record_event(
            conn,
            project_id,
            "ARCHITECT_IMPORT_REJECTED",
            "Human",
            "Rejected architect import proposal without changing project artifacts",
            Some(&serde_json::json!({
                "import_id": import_id
            })),
        )
        .map_err(|e| RelayError::Database(e.to_string()))?;

        Self::record_relay_history(
            conn,
            project_id,
            packet_id.as_deref(),
            Some(import_id),
            "IMPORT_REJECTED",
            "Rejected architect import proposal",
            None,
        )?;

        Ok(())
    }

    /// Reconciles any interrupted batch import transactions found in `relay_import_batch_journal`.
    /// Restores internally consistent state (either fully rolled back to pre-import state or fully committed).
    pub fn reconcile_interrupted_batches<P: AsRef<Path>>(
        conn: &Connection,
        repo_root: P,
        project_id: &str,
    ) -> Result<bool, RelayError> {
        let root = repo_root.as_ref();
        let row: Option<(String, String, String, String)> = conn
            .query_row(
                "SELECT import_id, packet_id, phase, operations_json FROM relay_import_batch_journal WHERE project_id = ?1",
                params![project_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;

        let (import_id, packet_id, phase, operations_json) = match row {
            Some(r) => r,
            None => return Ok(false),
        };

        #[cfg(test)]
        {
            if INJECTED_BATCH_FAILURE.with(|f| f.get())
                == InjectedBatchFailure::DuringRecoveryReconciliation
            {
                return Err(RelayError::InjectedFailure(
                    "DuringRecoveryReconciliation".to_string(),
                ));
            }
        }

        let ops: Vec<BatchOperation> = serde_json::from_str(&operations_json)
            .map_err(|e| RelayError::ParseError(e.to_string()))?;

        let coalition_dir = ArtifactManager::resolve_coalition_dir(root)?;

        match phase.as_str() {
            "STAGING" | "STAGED" => {
                // Disk files were never mutated. Clean up staged temp files.
                for op in &ops {
                    if let Some(ref tmp) = op.staged_temp_path {
                        let p = Path::new(tmp);
                        if p.exists() {
                            let _ = std::fs::remove_file(p);
                        }
                    }
                }
                conn.execute(
                    "UPDATE relay_imports SET decision = 'RECOVERY_ABORTED' WHERE import_id = ?1",
                    params![import_id],
                )?;
                conn.execute(
                    "DELETE FROM relay_import_batch_journal WHERE import_id = ?1",
                    params![import_id],
                )?;
                Self::record_relay_history(
                    conn,
                    project_id,
                    Some(&packet_id),
                    Some(&import_id),
                    "RECOVERY_ROLLED_BACK",
                    "Cleaned up interrupted uncommitted batch import",
                    None,
                )?;
            }
            "COMMITTING" => {
                // Roll back to pre-import state using backup files.
                for op in &ops {
                    let target_path = coalition_dir.join(&op.path);
                    let mut found_bak = op.backup_path.as_ref().map(std::path::PathBuf::from);
                    if found_bak.as_ref().map(|p| !p.exists()).unwrap_or(true) {
                        if let Some(parent) = target_path.parent() {
                            if let Ok(entries) = std::fs::read_dir(parent) {
                                let prefix = format!(
                                    "{}.bak.",
                                    target_path
                                        .file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                );
                                for entry in entries.flatten() {
                                    if entry.file_name().to_string_lossy().starts_with(&prefix) {
                                        found_bak = Some(entry.path());
                                        break;
                                    }
                                }
                            }
                        }
                    }

                    if let Some(ref bak_path) = found_bak {
                        if bak_path.exists() {
                            if target_path.exists() {
                                let _ = std::fs::remove_file(&target_path);
                            }
                            std::fs::rename(bak_path, &target_path).map_err(|e| {
                                RelayError::Artifact(format!(
                                    "Failed to restore backup {:?} to {:?}: {}",
                                    bak_path, target_path, e
                                ))
                            })?;
                        }
                    } else if op.action == ArtifactAction::Create && target_path.exists() {
                        let _ = std::fs::remove_file(&target_path);
                    }

                    if let Some(ref tmp) = op.staged_temp_path {
                        let p = Path::new(tmp);
                        if p.exists() {
                            let _ = std::fs::remove_file(p);
                        }
                    }
                }
                conn.execute(
                    "UPDATE relay_imports SET decision = 'RECOVERY_ROLLED_BACK' WHERE import_id = ?1",
                    params![import_id],
                )?;
                conn.execute(
                    "DELETE FROM relay_import_batch_journal WHERE import_id = ?1",
                    params![import_id],
                )?;
                Self::record_relay_history(
                    conn,
                    project_id,
                    Some(&packet_id),
                    Some(&import_id),
                    "RECOVERY_ROLLED_BACK",
                    "Rolled back interrupted batch import to pre-import state",
                    None,
                )?;
            }
            "COMMITTED" => {
                // All file replacements succeeded. Clean up backups and finalize DB.
                for op in &ops {
                    if let Some(ref bak) = op.backup_path {
                        let bak_path = Path::new(bak);
                        if bak_path.exists() {
                            let _ = std::fs::remove_file(bak_path);
                        }
                    }
                }
                let now = chrono::Utc::now().to_rfc3339();
                conn.execute(
                    "UPDATE relay_imports SET decision = 'ACCEPTED', decided_at = ?1 WHERE import_id = ?2",
                    params![now, import_id],
                )?;
                conn.execute(
                    "UPDATE relay_packets SET status = 'IMPORTED' WHERE packet_id = ?1",
                    params![packet_id],
                )?;
                let readiness = ReadinessEvaluator::evaluate(root);
                Self::reconcile_workflow_readiness_state(conn, project_id, &readiness, "System")?;
                conn.execute(
                    "DELETE FROM relay_import_batch_journal WHERE import_id = ?1",
                    params![import_id],
                )?;
                Self::record_relay_history(
                    conn,
                    project_id,
                    Some(&packet_id),
                    Some(&import_id),
                    "RECOVERY_COMPLETED",
                    "Finalized interrupted committed batch import",
                    None,
                )?;
            }
            _ => {}
        }

        Ok(true)
    }

    /// Saves governed manual edits to an architecture artifact.
    /// Validates path allowlist, optimistic concurrency fingerprint, YAML syntax,
    /// replaces file atomically, re-evaluates readiness, and reconciles workflow state.
    pub fn save_artifact_content<P: AsRef<Path>>(
        conn: &Connection,
        repo_root: P,
        project_id: &str,
        relative_path: &str,
        content: &str,
        expected_fingerprint: Option<&str>,
    ) -> Result<ReadinessReport, RelayError> {
        let root = repo_root.as_ref();

        // 1. Verify workflow state is legal (ARCHITECTING or READY_TO_FREEZE)
        let state_str: String = conn
            .query_row(
                "SELECT state FROM workflow_state WHERE project_id = ?1",
                params![project_id],
                |r| r.get(0),
            )
            .map_err(|_| RelayError::ProjectNotFound(project_id.to_string()))?;

        let current_state: WorkflowState = state_str
            .parse()
            .map_err(|e: workflow::WorkflowError| RelayError::Workflow(e.to_string()))?;

        if current_state != WorkflowState::Architecting
            && current_state != WorkflowState::ReadyToFreeze
        {
            return Err(RelayError::IllegalWorkflowState {
                current: current_state.to_string(),
                operation: "save_artifact_content".to_string(),
            });
        }

        // 2. Validate path in allowlist
        if !ArtifactManager::is_valid_architecture_artifact_path(relative_path) {
            return Err(RelayError::DisallowedArtifactPath(
                relative_path.to_string(),
            ));
        }

        // 3. Optimistic concurrency check
        let current_fp = ArtifactManager::compute_file_fingerprint(root, relative_path)?;
        if current_fp.as_deref() != expected_fingerprint {
            return Err(RelayError::StaleArtifactContent {
                path: relative_path.to_string(),
                details: format!(
                    "Artifact was modified. Expected fingerprint {:?}, but current is {:?}",
                    expected_fingerprint, current_fp
                ),
            });
        }

        // 4. If structured YAML, validate syntax
        if relative_path.ends_with(".yaml") || relative_path.ends_with(".yml") {
            let parsed: Result<serde_yaml::Value, _> = serde_yaml::from_str(content);
            if let Err(e) = parsed {
                return Err(RelayError::InvalidYamlContent {
                    path: relative_path.to_string(),
                    error: e.to_string(),
                });
            }
        }

        // 5. Atomically write replacement
        ArtifactManager::write_artifact_atomic(root, relative_path, content)?;

        // 6. Log activity event
        ActivityManager::record_event(
            conn,
            project_id,
            "ARTIFACT_MANUALLY_EDITED",
            "Human",
            &format!("Saved governed manual edits to {}", relative_path),
            Some(&serde_json::json!({
                "path": relative_path,
                "character_count": content.len(),
            })),
        )
        .map_err(|e| RelayError::Database(e.to_string()))?;

        // 7. Re-evaluate readiness
        let readiness = ReadinessEvaluator::evaluate(root);

        // 8. Reconcile workflow state
        Self::reconcile_workflow_readiness_state(conn, project_id, &readiness, "Human")?;

        Ok(readiness)
    }

    /// Reads an architecture artifact with content, existence, and fingerprint.
    pub fn get_artifact_content<P: AsRef<Path>>(
        repo_root: P,
        relative_path: &str,
    ) -> Result<ArtifactContentDetails, RelayError> {
        let root = repo_root.as_ref();
        if !ArtifactManager::is_valid_architecture_artifact_path(relative_path) {
            return Err(RelayError::DisallowedArtifactPath(
                relative_path.to_string(),
            ));
        }

        let content = ArtifactManager::read_artifact(root, relative_path)?;
        let fp = ArtifactManager::compute_file_fingerprint(root, relative_path)?;
        let exists = content.is_some();

        Ok(ArtifactContentDetails {
            path: relative_path.to_string(),
            content: content.unwrap_or_default(),
            fingerprint: fp,
            exists,
        })
    }

    /// Helper to reconcile workflow state when readiness changes.
    pub fn reconcile_workflow_readiness_state(
        conn: &Connection,
        project_id: &str,
        readiness: &ReadinessReport,
        actor: &str,
    ) -> Result<(), RelayError> {
        let current_state_str: String = conn
            .query_row(
                "SELECT state FROM workflow_state WHERE project_id = ?1",
                params![project_id],
                |r| r.get(0),
            )
            .map_err(|e| RelayError::Database(e.to_string()))?;

        let current_state: WorkflowState = current_state_str
            .parse()
            .map_err(|e: workflow::WorkflowError| RelayError::Workflow(e.to_string()))?;

        match (current_state, readiness.overall_readiness) {
            (WorkflowState::Architecting, OverallReadiness::ReadyToFreeze) => {
                workflow::apply_workflow_action_conn(
                    conn,
                    project_id,
                    WorkflowAction::MarkReadyToFreeze,
                    actor,
                )
                .map_err(|e| RelayError::Workflow(e.to_string()))?;
            }
            (WorkflowState::ReadyToFreeze, OverallReadiness::Incomplete) => {
                workflow::apply_workflow_action_conn(
                    conn,
                    project_id,
                    WorkflowAction::ReturnToArchitecting,
                    actor,
                )
                .map_err(|e| RelayError::Workflow(e.to_string()))?;
            }
            _ => {}
        }

        Ok(())
    }

    /// Fetches relay history for a project from `relay_history`.
    pub fn get_relay_history(
        conn: &Connection,
        project_id: &str,
        limit: usize,
    ) -> Result<Vec<RelayHistoryItem>, RelayError> {
        let mut stmt = conn.prepare(
            "SELECT id, packet_id, project_id, event_type, summary, created_at, details_json
             FROM relay_history WHERE project_id = ?1 ORDER BY id DESC LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![project_id, limit as i64], |row| {
            let id_num: i64 = row.get(0)?;
            let packet_id: Option<String> = row.get(1)?;
            let proj_id: String = row.get(2)?;
            let event_type: String = row.get(3)?;
            let summary: String = row.get(4)?;
            let created_at: String = row.get(5)?;
            let details_json: Option<String> = row.get(6)?;

            Ok(RelayHistoryItem {
                id: id_num.to_string(),
                packet_id,
                project_id: proj_id,
                item_type: event_type.clone(),
                summary,
                status: event_type,
                created_at,
                error_message: details_json,
            })
        })?;

        let mut items = Vec::new();
        for r in rows {
            items.push(r?);
        }
        Ok(items)
    }

    /// Assembles the complete architecture workspace state for UI rendering.
    pub fn get_workspace_state<P: AsRef<Path>>(
        conn: &Connection,
        repo_root: P,
        project_id: &str,
    ) -> Result<WorkspaceState, RelayError> {
        let root = repo_root.as_ref();
        let pending_packet = Self::get_pending_packet(conn, project_id)?;
        let readiness = ReadinessEvaluator::evaluate(root);
        let history = Self::get_relay_history(conn, project_id, 20)?;

        // Check if there is a pending import preview
        let pending_preview: Option<ImportPreview> = conn
            .query_row(
                "SELECT parsed_payload_json FROM relay_imports
                 WHERE project_id = ?1 AND decision = 'PENDING' AND parse_status = 'SUCCESS'
                 ORDER BY imported_at DESC LIMIT 1",
                params![project_id],
                |row| row.get(0),
            )
            .optional()?
            .and_then(|json_str: Option<String>| {
                json_str.and_then(|s| serde_json::from_str(&s).ok())
            });

        Ok(WorkspaceState {
            pending_packet,
            pending_preview,
            readiness,
            history,
        })
    }

    fn record_relay_history(
        conn: &Connection,
        project_id: &str,
        packet_id: Option<&str>,
        import_id: Option<&str>,
        event_type: &str,
        summary: &str,
        details: Option<&serde_json::Value>,
    ) -> Result<(), RelayError> {
        let now = chrono::Utc::now().to_rfc3339();
        let details_str = details.map(|d| d.to_string());
        conn.execute(
            "INSERT INTO relay_history (project_id, packet_id, import_id, event_type, summary, details_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![project_id, packet_id, import_id, event_type, summary, details_str, now],
        )?;
        Ok(())
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InjectedBatchFailure {
    None,
    BeforeFirstReplacement,
    AfterFirstReplacement,
    MidBatch,
    AfterFinalReplacement,
    DuringDeleteOperation,
    BeforeSqliteDecisionUpdate,
    AfterSqliteDecisionUpdate,
    DuringRecoveryReconciliation,
}

#[cfg(test)]
thread_local! {
    pub static INJECTED_BATCH_FAILURE: std::cell::Cell<InjectedBatchFailure> = const { std::cell::Cell::new(InjectedBatchFailure::None) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbManager;

    fn setup_test_project() -> (tempfile::TempDir, DbManager, String) {
        let dir = tempfile::tempdir().unwrap();
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();

        let proj = ArtifactManager::initialize_new_project(dir.path(), "Test Proj").unwrap();
        let now = chrono::Utc::now().to_rfc3339();

        db.connection()
            .execute(
                "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![proj.project_id, proj.name, dir.path().to_str().unwrap(), now, now, now],
            )
            .unwrap();

        db.connection()
            .execute(
                "INSERT INTO workflow_state (project_id, state, resume_state, revision, updated_at)
                 VALUES (?1, 'DRAFT', NULL, 1, ?2)",
                params![proj.project_id, now],
            )
            .unwrap();

        let pid = proj.project_id.clone();
        (dir, db, pid)
    }

    #[test]
    fn test_architect_initial_first_packet_and_update_subsequent_packet() {
        let (dir, mut db, pid) = setup_test_project();

        // First packet on DRAFT project transitions to ARCHITECTING and produces ARCHITECT_INITIAL
        let pkt1 =
            RelayService::prepare_architect_packet(db.connection_mut(), dir.path(), &pid, None)
                .unwrap();
        assert_eq!(pkt1.metadata.packet_type, RelayPacketType::ArchitectInitial);

        let state_str: String = db
            .connection()
            .query_row(
                "SELECT state FROM workflow_state WHERE project_id = ?1",
                params![pid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state_str, "ARCHITECTING");

        // Second packet produces ARCHITECT_UPDATE and supersedes first packet
        let pkt2 =
            RelayService::prepare_architect_packet(db.connection_mut(), dir.path(), &pid, None)
                .unwrap();
        assert_eq!(pkt2.metadata.packet_type, RelayPacketType::ArchitectUpdate);

        let old_status: String = db
            .connection()
            .query_row(
                "SELECT status FROM relay_packets WHERE packet_id = ?1",
                params![pkt1.metadata.packet_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(old_status, "SUPERSEDED");

        // Reconstruct from get_pending_packet
        let pending = RelayService::get_pending_packet(db.connection(), &pid)
            .unwrap()
            .unwrap();
        assert_eq!(pending.metadata.packet_id, pkt2.metadata.packet_id);
        assert_eq!(
            pending.metadata.packet_type,
            RelayPacketType::ArchitectUpdate
        );
        assert_eq!(pending.prompt, pkt2.prompt);
    }

    #[test]
    fn test_no_pending_packet_import_fails() {
        let (dir, db, pid) = setup_test_project();
        let raw = "```yaml\ncoalition_response:\n  schema: 1\n  packet_id: \"p1\"\n  project_id: \"pid\"\n  response_type: \"ARCHITECT_UPDATE\"\n  summary: \"test\"\n  artifacts: []\n```";
        let res = RelayService::process_import(db.connection(), dir.path(), &pid, raw);
        assert!(matches!(res, Err(RelayError::NoPendingPacket(_))));
    }

    #[test]
    fn test_wrapped_envelope_only_and_wrong_response_type_rejected() {
        let (dir, mut db, pid) = setup_test_project();
        let pkt =
            RelayService::prepare_architect_packet(db.connection_mut(), dir.path(), &pid, None)
                .unwrap();

        // 1. Direct un-wrapped payload rejected
        let unwrapped = format!(
            "```yaml\nschema: 1\npacket_id: \"{}\"\nproject_id: \"{}\"\nresponse_type: \"ARCHITECT_UPDATE\"\nsummary: \"test\"\nartifacts: []\n```",
            pkt.metadata.packet_id, pid
        );
        let res1 = RelayService::process_import(db.connection(), dir.path(), &pid, &unwrapped);
        assert!(res1.is_err());

        // 2. Wrong response_type rejected
        let wrong_type = format!(
            "```yaml\ncoalition_response:\n  schema: 1\n  packet_id: \"{}\"\n  project_id: \"{}\"\n  response_type: \"REVIEW_VERDICT\"\n  summary: \"test\"\n  artifacts: []\n```",
            pkt.metadata.packet_id, pid
        );
        let res2 = RelayService::process_import(db.connection(), dir.path(), &pid, &wrong_type);
        assert!(matches!(res2, Err(RelayError::ParseFailure { .. })));
    }

    #[test]
    fn test_create_modify_delete_semantic_conflicts() {
        let (dir, mut db, pid) = setup_test_project();
        let pkt =
            RelayService::prepare_architect_packet(db.connection_mut(), dir.path(), &pid, None)
                .unwrap();

        // Target already exists on disk
        ArtifactManager::write_artifact_atomic(
            dir.path(),
            "design/product-vision.md",
            "# Vision\nExisting",
        )
        .unwrap();

        // Proposal claims action: CREATE on existing file
        let conflict_create = format!(
            "```yaml\ncoalition_response:\n  schema: 1\n  packet_id: \"{}\"\n  project_id: \"{}\"\n  response_type: \"ARCHITECT_UPDATE\"\n  summary: \"test\"\n  artifacts:\n    - path: \"design/product-vision.md\"\n      action: \"CREATE\"\n      content: \"# Vision\\nNew\"\n```",
            pkt.metadata.packet_id, pid
        );

        let res = RelayService::process_import(db.connection(), dir.path(), &pid, &conflict_create);
        assert!(matches!(
            res,
            Err(RelayError::ActionSemanticConflict { .. })
        ));

        // Proposal claims action: MODIFY on absent file
        let conflict_modify = format!(
            "```yaml\ncoalition_response:\n  schema: 1\n  packet_id: \"{}\"\n  project_id: \"{}\"\n  response_type: \"ARCHITECT_UPDATE\"\n  summary: \"test\"\n  artifacts:\n    - path: \"design/requirements.md\"\n      action: \"MODIFY\"\n      content: \"# Reqs\\nModified\"\n```",
            pkt.metadata.packet_id, pid
        );
        let res2 =
            RelayService::process_import(db.connection(), dir.path(), &pid, &conflict_modify);
        assert!(matches!(
            res2,
            Err(RelayError::ActionSemanticConflict { .. })
        ));
    }

    #[test]
    fn test_preview_hash_conflict_and_external_edit_blocks_accept() {
        let (dir, mut db, pid) = setup_test_project();
        let pkt =
            RelayService::prepare_architect_packet(db.connection_mut(), dir.path(), &pid, None)
                .unwrap();

        let raw = format!(
            "```yaml\ncoalition_response:\n  schema: 1\n  packet_id: \"{}\"\n  project_id: \"{}\"\n  response_type: \"ARCHITECT_UPDATE\"\n  summary: \"test\"\n  artifacts:\n    - path: \"design/product-vision.md\"\n      action: \"CREATE\"\n      content: \"# Vision\\nSubstantive vision document.\"\n```",
            pkt.metadata.packet_id, pid
        );

        let preview =
            RelayService::process_import(db.connection(), dir.path(), &pid, &raw).unwrap();

        // External actor creates the file before Accept
        ArtifactManager::write_artifact_atomic(
            dir.path(),
            "design/product-vision.md",
            "# Vision\nExternal change",
        )
        .unwrap();

        let accept_res =
            RelayService::accept_import(db.connection(), dir.path(), &pid, &preview.import_id);
        assert!(matches!(
            accept_res,
            Err(RelayError::StaleImportPreview { .. })
        ));
    }

    #[test]
    fn test_duplicate_accept_and_multiple_pending_imports() {
        let (dir, mut db, pid) = setup_test_project();
        let pkt =
            RelayService::prepare_architect_packet(db.connection_mut(), dir.path(), &pid, None)
                .unwrap();

        let raw = format!(
            "```yaml\ncoalition_response:\n  schema: 1\n  packet_id: \"{}\"\n  project_id: \"{}\"\n  response_type: \"ARCHITECT_UPDATE\"\n  summary: \"test\"\n  artifacts:\n    - path: \"design/product-vision.md\"\n      action: \"CREATE\"\n      content: \"# Vision\\nSubstantive vision document.\"\n```",
            pkt.metadata.packet_id, pid
        );

        let preview =
            RelayService::process_import(db.connection(), dir.path(), &pid, &raw).unwrap();

        // Attempt second import while first is pending
        let second_import = RelayService::process_import(db.connection(), dir.path(), &pid, &raw);
        assert!(matches!(
            second_import,
            Err(RelayError::ImportAlreadyPending(_))
        ));

        // Accept first import
        RelayService::accept_import(db.connection(), dir.path(), &pid, &preview.import_id).unwrap();

        // Duplicate Accept on same import
        let dup_accept =
            RelayService::accept_import(db.connection(), dir.path(), &pid, &preview.import_id);
        assert!(matches!(
            dup_accept,
            Err(RelayError::ImportAlreadyAccepted(_))
        ));
    }

    #[test]
    fn test_illegal_workflow_state_mutation_rejected() {
        let (dir, mut db, pid) = setup_test_project();
        RelayService::prepare_architect_packet(db.connection_mut(), dir.path(), &pid, None)
            .unwrap();

        // Forcibly set workflow state to FROZEN
        db.connection()
            .execute(
                "UPDATE workflow_state SET state = 'FROZEN' WHERE project_id = ?1",
                params![pid],
            )
            .unwrap();

        let raw = "```yaml\ncoalition_response:\n  schema: 1\n  packet_id: \"pkt\"\n  project_id: \"pid\"\n  response_type: \"ARCHITECT_UPDATE\"\n  summary: \"test\"\n  artifacts: []\n```";
        let res = RelayService::process_import(db.connection(), dir.path(), &pid, raw);
        assert!(matches!(res, Err(RelayError::IllegalWorkflowState { .. })));

        let edit_res = RelayService::save_artifact_content(
            db.connection(),
            dir.path(),
            &pid,
            "design/product-vision.md",
            "# New vision",
            None,
        );
        assert!(matches!(
            edit_res,
            Err(RelayError::IllegalWorkflowState { .. })
        ));
    }

    #[test]
    fn test_authoritative_transition_to_ready_to_freeze_and_return_to_architecting() {
        let (dir, mut db, pid) = setup_test_project();
        let pkt =
            RelayService::prepare_architect_packet(db.connection_mut(), dir.path(), &pid, None)
                .unwrap();

        // Prepare full package of all 9 required artifacts
        let complete_response = format!(
            "```yaml\ncoalition_response:\n  schema: 1\n  packet_id: \"{}\"\n  project_id: \"{}\"\n  response_type: \"ARCHITECT_UPDATE\"\n  summary: \"Full package\"\n  artifacts:\n    - path: \"design/product-vision.md\"\n      action: \"CREATE\"\n      content: \"# Vision\\nSubstantive vision document.\"\n    - path: \"design/requirements.md\"\n      action: \"CREATE\"\n      content: \"# Requirements\\nREQ-01: System must enforce invariants reliably.\"\n    - path: \"design/architecture.md\"\n      action: \"CREATE\"\n      content: \"# Architecture\\nLayered modular architecture with Rust backend.\"\n    - path: \"design/constraints.md\"\n      action: \"CREATE\"\n      content: \"# Constraints\\nOffline-first operation and bounded local resource usage.\"\n    - path: \"design/interfaces.md\"\n      action: \"CREATE\"\n      content: \"# Interfaces\\nTyped IPC contracts across frontend and machine side.\"\n    - path: \"design/security.md\"\n      action: \"CREATE\"\n      content: \"# Security\\nLeast privilege process management and local safe paths.\"\n    - path: \"implementation/implementation-plan.md\"\n      action: \"CREATE\"\n      content: \"# Plan\\nPhased delivery with deterministic test milestones.\"\n    - path: \"implementation/acceptance-criteria.yaml\"\n      action: \"CREATE\"\n      content: \"criteria:\\n  - id: AC-1\\n    name: Verified passes\\n\"\n    - path: \"implementation/test-plan.md\"\n      action: \"CREATE\"\n      content: \"# Test Plan\\nDeterministic automated test suite and regression checks.\"\n```",
            pkt.metadata.packet_id, pid
        );

        let preview =
            RelayService::process_import(db.connection(), dir.path(), &pid, &complete_response)
                .unwrap();
        let report =
            RelayService::accept_import(db.connection(), dir.path(), &pid, &preview.import_id)
                .unwrap();

        assert_eq!(report.overall_readiness, OverallReadiness::ReadyToFreeze);

        // Authoritative workflow state MUST have transitioned to READY_TO_FREEZE
        let state_str: String = db
            .connection()
            .query_row(
                "SELECT state FROM workflow_state WHERE project_id = ?1",
                params![pid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state_str, "READY_TO_FREEZE");

        // Now governed manual edit makes requirements incomplete -> returns to ARCHITECTING
        let fp = ArtifactManager::compute_file_fingerprint(dir.path(), "design/requirements.md")
            .unwrap();
        let report2 = RelayService::save_artifact_content(
            db.connection(),
            dir.path(),
            &pid,
            "design/requirements.md",
            "# Requirements\nTODO",
            fp.as_deref(),
        )
        .unwrap();

        assert_eq!(report2.overall_readiness, OverallReadiness::Incomplete);
        let state_str2: String = db
            .connection()
            .query_row(
                "SELECT state FROM workflow_state WHERE project_id = ?1",
                params![pid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state_str2, "ARCHITECTING");
    }

    #[test]
    fn test_batch_import_crash_mid_commit_and_recovery_reconciliation() {
        let (dir, mut db, pid) = setup_test_project();
        let pkt =
            RelayService::prepare_architect_packet(db.connection_mut(), dir.path(), &pid, None)
                .unwrap();

        // Write existing file
        ArtifactManager::write_artifact_atomic(
            dir.path(),
            "design/product-vision.md",
            "# Vision\nOriginal content",
        )
        .unwrap();

        let response = format!(
            "```yaml\ncoalition_response:\n  schema: 1\n  packet_id: \"{}\"\n  project_id: \"{}\"\n  response_type: \"ARCHITECT_UPDATE\"\n  summary: \"Two files\"\n  artifacts:\n    - path: \"design/product-vision.md\"\n      action: \"MODIFY\"\n      content: \"# Vision\\nModified substantive content.\"\n    - path: \"design/requirements.md\"\n      action: \"CREATE\"\n      content: \"# Requirements\\nNew substantive content.\"\n```",
            pkt.metadata.packet_id, pid
        );

        let preview =
            RelayService::process_import(db.connection(), dir.path(), &pid, &response).unwrap();

        // Inject failure mid-commit (AfterFirstReplacement)
        INJECTED_BATCH_FAILURE.with(|f| f.set(InjectedBatchFailure::AfterFirstReplacement));

        let res =
            RelayService::accept_import(db.connection(), dir.path(), &pid, &preview.import_id);
        assert!(res.is_err());

        // Reset injection
        INJECTED_BATCH_FAILURE.with(|f| f.set(InjectedBatchFailure::None));

        // Reconcile interrupted batch
        let recovered =
            RelayService::reconcile_interrupted_batches(db.connection(), dir.path(), &pid).unwrap();
        assert!(recovered);

        // Pre-import state must be authoritatively restored!
        let pv = ArtifactManager::read_artifact(dir.path(), "design/product-vision.md")
            .unwrap()
            .unwrap();
        assert_eq!(pv, "# Vision\nOriginal content");

        let req = ArtifactManager::read_artifact(dir.path(), "design/requirements.md").unwrap();
        assert!(req.is_none());
    }

    #[test]
    fn test_governed_manual_artifact_edit_and_stale_conflict() {
        let (dir, mut db, pid) = setup_test_project();
        RelayService::prepare_architect_packet(db.connection_mut(), dir.path(), &pid, None)
            .unwrap();

        ArtifactManager::write_artifact_atomic(
            dir.path(),
            "design/product-vision.md",
            "# Vision\nInitial",
        )
        .unwrap();
        let fp1 = ArtifactManager::compute_file_fingerprint(dir.path(), "design/product-vision.md")
            .unwrap();

        // Successful save
        let report = RelayService::save_artifact_content(
            db.connection(),
            dir.path(),
            &pid,
            "design/product-vision.md",
            "# Vision\nSubstantive updated vision content.",
            fp1.as_deref(),
        )
        .unwrap();
        assert_eq!(report.ready_required_count, 1);

        // Stale fingerprint rejected
        let stale_res = RelayService::save_artifact_content(
            db.connection(),
            dir.path(),
            &pid,
            "design/product-vision.md",
            "# Vision\nOverwritten",
            fp1.as_deref(), // using stale fingerprint fp1 instead of new
        );
        assert!(matches!(
            stale_res,
            Err(RelayError::StaleArtifactContent { .. })
        ));

        // Invalid YAML save rejected
        let invalid_yaml_res = RelayService::save_artifact_content(
            db.connection(),
            dir.path(),
            &pid,
            "implementation/acceptance-criteria.yaml",
            ": invalid yaml :",
            None,
        );
        assert!(matches!(
            invalid_yaml_res,
            Err(RelayError::InvalidYamlContent { .. })
        ));
    }
}

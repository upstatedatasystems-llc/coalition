pub mod readiness;

use crate::core::activity::ActivityManager;
use crate::core::artifacts::{ArtifactError, ArtifactManager};
use crate::core::relay::readiness::{ReadinessEvaluator, ReadinessReport};
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
            "Project Name: {}\nProject ID: {}\nCurrent Architecture State: draft\nReadiness: {}/{} required artifacts populated\n",
            project_name, project_id, readiness.ready_count, readiness.total_required
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
            design/product-vision.md, design/requirements.md, design/architecture.md, design/constraints.md, design/interfaces.md, design/security.md, implementation/implementation-plan.md, implementation/acceptance-criteria.yaml, implementation/test-plan.md, decisions/ADR-*.md.\n"
        );

        (prompt, human_instructions, context_summary)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ParsedEnvelope {
    Wrapped(ArchitectResponseEnvelope),
    Direct(ArchitectResponsePayload),
}

pub struct RelayParser;

impl RelayParser {
    /// Extracts and parses the structured Coalition response from raw clipboard text.
    /// Tolerates explanatory prose before and after the structured block.
    pub fn parse_response(
        raw_text: &str,
        expected_project_id: &str,
        expected_packet_id: Option<&str>,
    ) -> Result<ArchitectResponsePayload, RelayError> {
        let block = Self::extract_structured_block(raw_text)?;

        let parsed_envelope: ParsedEnvelope = serde_yaml::from_str(&block)
            .map_err(|e| RelayError::ParseError(format!("Failed to parse response YAML: {}", e)))?;

        let payload = match parsed_envelope {
            ParsedEnvelope::Wrapped(env) => env.coalition_response,
            ParsedEnvelope::Direct(p) => p,
        };

        if payload.schema != 1 {
            return Err(RelayError::SchemaError(format!(
                "Unsupported response schema version {}. Expected 1",
                payload.schema
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

        // If in DRAFT, transition to ARCHITECTING
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

        // 3. Mark any previous PENDING packet as SUPERSEDED
        conn.execute(
            "UPDATE relay_packets SET status = 'SUPERSEDED' WHERE project_id = ?1 AND status = 'PENDING'",
            params![project_id],
        )?;

        // 4. Build packet
        let packet_id = Uuid::new_v4().to_string();
        let created_at = chrono::Utc::now().to_rfc3339();
        let (prompt, human_instructions, context_summary) = RelayPromptBuilder::build_prompt(
            root,
            project_id,
            &name,
            &packet_id,
            RelayPacketType::ArchitectInitial,
            custom_notes,
        );

        let metadata = RelayPacketMetadata {
            schema: 1,
            project_id: project_id.to_string(),
            packet_id: packet_id.clone(),
            role: RelayRole::Architect,
            packet_type: RelayPacketType::ArchitectInitial,
            architecture_version: "draft".to_string(),
            expected_response: RelayExpectedResponse::ArchitectUpdate,
            created_at: created_at.clone(),
        };

        // 5. Persist packet in SQLite
        conn.execute(
            "INSERT INTO relay_packets (packet_id, project_id, role, packet_type, architecture_version, expected_response, prompt, created_at, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'PENDING')",
            params![
                packet_id,
                project_id,
                "ARCHITECT",
                "ARCHITECT_INITIAL",
                "draft",
                "ARCHITECT_UPDATE",
                prompt,
                created_at
            ],
        )?;

        let packet = RelayPacket {
            metadata,
            prompt,
            human_instructions,
            project_context_summary: context_summary,
        };

        Ok(packet)
    }

    /// Retrieves current pending packet for project if one exists.
    pub fn get_pending_packet(
        conn: &Connection,
        project_id: &str,
    ) -> Result<Option<RelayPacket>, RelayError> {
        let res = conn.query_row(
            "SELECT packet_id, project_id, role, packet_type, architecture_version, expected_response, prompt, created_at
             FROM relay_packets WHERE project_id = ?1 AND status = 'PENDING' ORDER BY created_at DESC LIMIT 1",
            params![project_id],
            |row| {
                let packet_id: String = row.get(0)?;
                let proj_id: String = row.get(1)?;
                let _role_str: String = row.get(2)?;
                let _type_str: String = row.get(3)?;
                let arch_ver: String = row.get(4)?;
                let _expected_str: String = row.get(5)?;
                let prompt: String = row.get(6)?;
                let created_at: String = row.get(7)?;

                Ok(RelayPacket {
                    metadata: RelayPacketMetadata {
                        schema: 1,
                        project_id: proj_id,
                        packet_id,
                        role: RelayRole::Architect,
                        packet_type: RelayPacketType::ArchitectInitial,
                        architecture_version: arch_ver,
                        expected_response: RelayExpectedResponse::ArchitectUpdate,
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
    /// and persists import record in SQLite without mutating project artifacts.
    pub fn process_import<P: AsRef<Path>>(
        conn: &Connection,
        repo_root: P,
        project_id: &str,
        raw_clipboard_text: &str,
    ) -> Result<ImportPreview, RelayError> {
        let root = repo_root.as_ref();
        let pending = Self::get_pending_packet(conn, project_id)?;
        let expected_packet_id = pending.as_ref().map(|p| p.metadata.packet_id.as_str());

        let import_id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        let parsed = match RelayParser::parse_response(
            raw_clipboard_text,
            project_id,
            expected_packet_id,
        ) {
            Ok(p) => p,
            Err(e) => {
                // Record failed parse in SQLite for audit and manual recovery
                let error_msg = e.to_string();
                conn.execute(
                    "INSERT INTO relay_imports (import_id, packet_id, project_id, raw_content, parsed_payload_json, parse_status, error_message, decision, imported_at, decided_at)
                     VALUES (?1, ?2, ?3, ?4, NULL, 'PARSE_ERROR', ?5, 'PENDING', ?6, NULL)",
                    params![import_id, expected_packet_id, project_id, raw_clipboard_text, error_msg, now],
                )?;

                return Err(RelayError::ParseFailure {
                    import_id,
                    raw_content: raw_clipboard_text.to_string(),
                    message: error_msg,
                });
            }
        };

        // Compute diff against existing files on disk
        let mut preview_items = Vec::new();
        for art in &parsed.artifacts {
            let current = ArtifactManager::read_artifact(root, &art.path)
                .ok()
                .flatten();
            let title = readiness::REQUIRED_ARCHITECTURE_ARTIFACTS
                .iter()
                .find(|(p, _)| *p == art.path)
                .map(|(_, t)| *t)
                .unwrap_or("Architecture Artifact");

            let status = match (&art.action, &current) {
                (ArtifactAction::Delete, None) => ArtifactDiffStatus::Unchanged,
                (ArtifactAction::Delete, Some(_)) => ArtifactDiffStatus::Deleted,
                (_, None) => ArtifactDiffStatus::New,
                (_, Some(existing)) => {
                    if existing.trim() == art.content.trim() {
                        ArtifactDiffStatus::Unchanged
                    } else {
                        ArtifactDiffStatus::Modified
                    }
                }
            };

            preview_items.push(ArtifactPreviewItem {
                path: art.path.clone(),
                title: title.to_string(),
                action: art.action,
                status,
                current_content: current,
                proposed_content: art.content.clone(),
            });
        }

        let parsed_json =
            serde_json::to_string(&parsed).map_err(|e| RelayError::ParseError(e.to_string()))?;

        conn.execute(
            "INSERT INTO relay_imports (import_id, packet_id, project_id, raw_content, parsed_payload_json, parse_status, error_message, decision, imported_at, decided_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'SUCCESS', NULL, 'PENDING', ?6, NULL)",
            params![import_id, parsed.packet_id, project_id, raw_clipboard_text, parsed_json, now],
        )?;

        Ok(ImportPreview {
            import_id,
            packet_id: parsed.packet_id,
            project_id: project_id.to_string(),
            summary: parsed.summary,
            artifacts: preview_items,
            open_questions: parsed.open_questions,
            raw_response: raw_clipboard_text.to_string(),
        })
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
        let pending = Self::get_pending_packet(conn, project_id)?;
        let expected_packet_id = pending.as_ref().map(|p| p.metadata.packet_id.as_str());

        let parsed = RelayParser::parse_response(edited_raw_text, project_id, expected_packet_id)?;

        let mut preview_items = Vec::new();
        for art in &parsed.artifacts {
            let current = ArtifactManager::read_artifact(root, &art.path)
                .ok()
                .flatten();
            let title = readiness::REQUIRED_ARCHITECTURE_ARTIFACTS
                .iter()
                .find(|(p, _)| *p == art.path)
                .map(|(_, t)| *t)
                .unwrap_or("Architecture Artifact");

            let status = match (&art.action, &current) {
                (ArtifactAction::Delete, None) => ArtifactDiffStatus::Unchanged,
                (ArtifactAction::Delete, Some(_)) => ArtifactDiffStatus::Deleted,
                (_, None) => ArtifactDiffStatus::New,
                (_, Some(existing)) => {
                    if existing.trim() == art.content.trim() {
                        ArtifactDiffStatus::Unchanged
                    } else {
                        ArtifactDiffStatus::Modified
                    }
                }
            };

            preview_items.push(ArtifactPreviewItem {
                path: art.path.clone(),
                title: title.to_string(),
                action: art.action,
                status,
                current_content: current,
                proposed_content: art.content.clone(),
            });
        }

        let parsed_json =
            serde_json::to_string(&parsed).map_err(|e| RelayError::ParseError(e.to_string()))?;

        conn.execute(
            "UPDATE relay_imports SET raw_content = ?1, parsed_payload_json = ?2, parse_status = 'SUCCESS', error_message = NULL
             WHERE import_id = ?3 AND project_id = ?4",
            params![edited_raw_text, parsed_json, import_id, project_id],
        )?;

        Ok(ImportPreview {
            import_id: import_id.to_string(),
            packet_id: parsed.packet_id,
            project_id: project_id.to_string(),
            summary: parsed.summary,
            artifacts: preview_items,
            open_questions: parsed.open_questions,
            raw_response: edited_raw_text.to_string(),
        })
    }

    /// Explicitly accepts an import. Atomically writes proposed artifacts to `.coalition/`,
    /// updates SQLite records, logs activity event, and returns updated readiness report.
    pub fn accept_import<P: AsRef<Path>>(
        conn: &Connection,
        repo_root: P,
        project_id: &str,
        import_id: &str,
    ) -> Result<ReadinessReport, RelayError> {
        let root = repo_root.as_ref();

        // 1. Fetch import record
        let (parsed_json_opt, decision, packet_id): (Option<String>, String, Option<String>) = conn
            .query_row(
                "SELECT parsed_payload_json, decision, packet_id FROM relay_imports WHERE import_id = ?1 AND project_id = ?2",
                params![import_id, project_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|_| RelayError::ImportNotFound(import_id.to_string()))?;

        if decision != "PENDING" {
            return Err(RelayError::AlreadyDecided(decision));
        }

        let parsed_json = parsed_json_opt.ok_or_else(|| {
            RelayError::ParseError("Import record has no parsed payload".to_string())
        })?;

        let payload: ArchitectResponsePayload = serde_json::from_str(&parsed_json)
            .map_err(|e| RelayError::ParseError(e.to_string()))?;

        // 2. Validate all paths before modifying disk
        for art in &payload.artifacts {
            if !ArtifactManager::is_valid_architecture_artifact_path(&art.path) {
                return Err(RelayError::DisallowedArtifactPath(art.path.clone()));
            }
        }

        // 3. Atomically write artifacts
        for art in &payload.artifacts {
            match art.action {
                ArtifactAction::Create | ArtifactAction::Modify => {
                    ArtifactManager::write_artifact_atomic(root, &art.path, &art.content)?;
                }
                ArtifactAction::Delete => {
                    let _ = ArtifactManager::delete_artifact(root, &art.path)?;
                }
            }
        }

        // 4. Update open questions if present
        if !payload.open_questions.is_empty() {
            let mut oq_md = String::from("# Architectural Open Questions\n\n");
            for q in &payload.open_questions {
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
            ArtifactManager::write_artifact_atomic(root, "design/open-questions.md", &oq_md)?;
        }

        // 5. Update SQLite import record
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE relay_imports SET decision = 'ACCEPTED', decided_at = ?1 WHERE import_id = ?2",
            params![now, import_id],
        )?;

        // 6. Update pending packet status to IMPORTED
        if let Some(pkt_id) = packet_id {
            conn.execute(
                "UPDATE relay_packets SET status = 'IMPORTED' WHERE packet_id = ?1",
                params![pkt_id],
            )?;
        }

        // 7. Log activity event
        let affected_paths: Vec<String> =
            payload.artifacts.iter().map(|a| a.path.clone()).collect();
        ActivityManager::record_event(
            conn,
            project_id,
            "ARCHITECT_IMPORT_ACCEPTED",
            "Human",
            &format!("Accepted architect changes: {}", payload.summary),
            Some(&serde_json::json!({
                "import_id": import_id,
                "summary": payload.summary,
                "affected_artifacts": affected_paths,
            })),
        )
        .map_err(|e| RelayError::Database(e.to_string()))?;

        // 8. Compute readiness
        let readiness = ReadinessEvaluator::evaluate(root);
        Ok(readiness)
    }

    /// Explicitly rejects an import. Project files on disk remain untouched.
    pub fn reject_import(
        conn: &Connection,
        project_id: &str,
        import_id: &str,
    ) -> Result<(), RelayError> {
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

        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE relay_imports SET decision = 'REJECTED', decided_at = ?1 WHERE import_id = ?2",
            params![now, import_id],
        )?;

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

        Ok(())
    }

    /// Fetches relay history for a project.
    pub fn get_relay_history(
        conn: &Connection,
        project_id: &str,
        limit: usize,
    ) -> Result<Vec<RelayHistoryItem>, RelayError> {
        let mut stmt = conn.prepare(
            "SELECT import_id, packet_id, project_id, parse_status, decision, imported_at, error_message, raw_content
             FROM relay_imports WHERE project_id = ?1 ORDER BY imported_at DESC LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![project_id, limit as i64], |row| {
            let id: String = row.get(0)?;
            let packet_id: Option<String> = row.get(1)?;
            let project_id: String = row.get(2)?;
            let parse_status: String = row.get(3)?;
            let decision: Option<String> = row.get(4)?;
            let imported_at: String = row.get(5)?;
            let error_message: Option<String> = row.get(6)?;

            let status = match decision.as_deref() {
                Some("ACCEPTED") => "ACCEPTED".to_string(),
                Some("REJECTED") => "REJECTED".to_string(),
                _ if parse_status == "PARSE_ERROR" => "PARSE_ERROR".to_string(),
                _ => "PENDING".to_string(),
            };

            let summary = format!("Relay Import ({})", status);

            Ok(RelayHistoryItem {
                id,
                packet_id,
                project_id,
                item_type: "IMPORT".to_string(),
                summary,
                status,
                created_at: imported_at,
                error_message,
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
        let pending_preview: Option<ImportPreview> = conn.query_row(
            "SELECT import_id, packet_id, project_id, raw_content, parsed_payload_json
             FROM relay_imports WHERE project_id = ?1 AND decision = 'PENDING' AND parse_status = 'SUCCESS'
             ORDER BY imported_at DESC LIMIT 1",
            params![project_id],
            |row| {
                let import_id: String = row.get(0)?;
                let packet_id: Option<String> = row.get(1)?;
                let proj_id: String = row.get(2)?;
                let raw_response: String = row.get(3)?;
                let parsed_json: Option<String> = row.get(4)?;
                Ok((import_id, packet_id, proj_id, raw_response, parsed_json))
            },
        ).optional()?.and_then(|(import_id, packet_id, proj_id, raw_response, parsed_json)| {
            let payload: ArchitectResponsePayload = serde_json::from_str(&parsed_json?).ok()?;
            let mut preview_items = Vec::new();
            for art in &payload.artifacts {
                let current = ArtifactManager::read_artifact(root, &art.path).ok().flatten();
                let title = readiness::REQUIRED_ARCHITECTURE_ARTIFACTS
                    .iter()
                    .find(|(p, _)| *p == art.path)
                    .map(|(_, t)| *t)
                    .unwrap_or("Architecture Artifact");

                let status = match (&art.action, &current) {
                    (ArtifactAction::Delete, None) => ArtifactDiffStatus::Unchanged,
                    (ArtifactAction::Delete, Some(_)) => ArtifactDiffStatus::Deleted,
                    (_, None) => ArtifactDiffStatus::New,
                    (_, Some(existing)) => {
                        if existing.trim() == art.content.trim() {
                            ArtifactDiffStatus::Unchanged
                        } else {
                            ArtifactDiffStatus::Modified
                        }
                    }
                };

                preview_items.push(ArtifactPreviewItem {
                    path: art.path.clone(),
                    title: title.to_string(),
                    action: art.action,
                    status,
                    current_content: current,
                    proposed_content: art.content.clone(),
                });
            }

            Some(ImportPreview {
                import_id,
                packet_id: packet_id.unwrap_or_default(),
                project_id: proj_id,
                summary: payload.summary,
                artifacts: preview_items,
                open_questions: payload.open_questions,
                raw_response,
            })
        });

        Ok(WorkspaceState {
            pending_packet,
            pending_preview,
            readiness,
            history,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbManager;

    fn setup_test_project() -> (tempfile::TempDir, DbManager, String) {
        let dir = tempfile::tempdir().unwrap();
        let mut db = DbManager::new_in_memory().unwrap();
        db.run_migrations().unwrap();

        let project_id = Uuid::new_v4().to_string();
        ArtifactManager::initialize_new_project(dir.path(), "Relay Test Project").unwrap();

        let now = chrono::Utc::now().to_rfc3339();
        db.connection()
            .execute(
                "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at)
                 VALUES (?1, 'Relay Test Project', ?2, ?3, ?4, ?5)",
                params![project_id, dir.path().to_string_lossy().to_string(), now, now, now],
            )
            .unwrap();

        db.connection()
            .execute(
                "INSERT INTO workflow_state (project_id, state, resume_state, revision, updated_at)
                 VALUES (?1, 'DRAFT', NULL, 1, ?2)",
                params![project_id, now],
            )
            .unwrap();

        (dir, db, project_id)
    }

    #[test]
    fn test_prepare_architect_packet_transitions_draft_and_persists() {
        let (dir, mut db, project_id) = setup_test_project();

        let packet = RelayService::prepare_architect_packet(
            db.connection_mut(),
            dir.path(),
            &project_id,
            Some("Focus on security first"),
        )
        .unwrap();

        assert_eq!(packet.metadata.project_id, project_id);
        assert_eq!(packet.metadata.schema, 1);
        assert_eq!(packet.metadata.role, RelayRole::Architect);
        assert_eq!(
            packet.metadata.packet_type,
            RelayPacketType::ArchitectInitial
        );
        assert!(packet.prompt.contains("COALITION ARCHITECT RELAY PACKET"));
        assert!(packet.prompt.contains("Focus on security first"));

        // Workflow state should now be ARCHITECTING
        let state: String = db
            .connection()
            .query_row(
                "SELECT state FROM workflow_state WHERE project_id = ?1",
                params![project_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, "ARCHITECTING");

        // Pending packet should be retrievable
        let pending = RelayService::get_pending_packet(db.connection(), &project_id).unwrap();
        assert!(pending.is_some());
        assert_eq!(
            pending.unwrap().metadata.packet_id,
            packet.metadata.packet_id
        );
    }

    #[test]
    fn test_response_parser_tolerates_surrounding_prose() {
        let project_id = Uuid::new_v4().to_string();
        let packet_id = Uuid::new_v4().to_string();

        let chatgpt_output = format!(
            r##"Hello human! I have drafted your requested specifications.

```yaml
coalition_response:
  schema: 1
  packet_id: "{}"
  project_id: "{}"
  response_type: "ARCHITECT_UPDATE"
  summary: "Initial architecture draft"
  artifacts:
    - path: "design/product-vision.md"
      action: "CREATE"
      content: |
        # Vision
        Substantive product vision content with more than fifty characters here!
    - path: "design/requirements.md"
      action: "CREATE"
      content: |
        # Requirements
        Detailed requirements content with more than fifty characters here!
  open_questions:
    - id: "OQ-1"
      question: "Should we support Linux in V1?"
      status: "OPEN"
      resolution: null
```

Let me know if you would like to refine any interfaces!"##,
            packet_id, project_id
        );

        let parsed =
            RelayParser::parse_response(&chatgpt_output, &project_id, Some(&packet_id)).unwrap();
        assert_eq!(parsed.project_id, project_id);
        assert_eq!(parsed.packet_id, packet_id);
        assert_eq!(parsed.artifacts.len(), 2);
        assert_eq!(parsed.open_questions.len(), 1);
        assert_eq!(parsed.open_questions[0].id, "OQ-1");
    }

    #[test]
    fn test_response_parser_rejects_unrelated_clipboard() {
        let project_id = Uuid::new_v4().to_string();
        let random_clipboard =
            "Hey check out this recipe for chocolate chip cookies: 2 cups flour, 1 cup butter...";

        let err = RelayParser::parse_response(random_clipboard, &project_id, None).unwrap_err();
        match err {
            RelayError::UnrelatedContent(_) => {}
            other => panic!("Expected UnrelatedContent error, got {:?}", other),
        }
    }

    #[test]
    fn test_response_parser_rejects_disallowed_paths() {
        let project_id = Uuid::new_v4().to_string();
        let packet_id = Uuid::new_v4().to_string();

        let evil_output = format!(
            r##"```yaml
coalition_response:
  schema: 1
  packet_id: "{}"
  project_id: "{}"
  response_type: "ARCHITECT_UPDATE"
  summary: "Evil path escape attempt"
  artifacts:
    - path: "../secret.txt"
      action: "CREATE"
      content: "malicious"
```"##,
            packet_id, project_id
        );

        let err =
            RelayParser::parse_response(&evil_output, &project_id, Some(&packet_id)).unwrap_err();
        match err {
            RelayError::DisallowedArtifactPath(p) => assert_eq!(p, "../secret.txt"),
            other => panic!("Expected DisallowedArtifactPath, got {:?}", other),
        }
    }

    #[test]
    fn test_process_import_and_accept_workflow() {
        let (dir, mut db, project_id) = setup_test_project();

        let packet = RelayService::prepare_architect_packet(
            db.connection_mut(),
            dir.path(),
            &project_id,
            None,
        )
        .unwrap();

        let response_text = format!(
            r##"```yaml
coalition_response:
  schema: 1
  packet_id: "{}"
  project_id: "{}"
  response_type: "ARCHITECT_UPDATE"
  summary: "Add vision and requirements"
  artifacts:
    - path: "design/product-vision.md"
      action: "CREATE"
      content: |
        # Vision
        Substantive product vision content with more than fifty characters here!
    - path: "design/requirements.md"
      action: "CREATE"
      content: |
        # Requirements
        Detailed requirements content with more than fifty characters here!
  open_questions:
    - id: "OQ-1"
      question: "First open question"
      status: "OPEN"
      resolution: null
```"##,
            packet.metadata.packet_id, project_id
        );

        // 1. Process import (generates preview, does not mutate files)
        let preview =
            RelayService::process_import(db.connection(), dir.path(), &project_id, &response_text)
                .unwrap();

        assert_eq!(preview.artifacts.len(), 2);
        assert_eq!(preview.artifacts[0].status, ArtifactDiffStatus::New);

        // Verify files on disk do NOT exist yet
        assert!(
            ArtifactManager::read_artifact(dir.path(), "design/product-vision.md")
                .unwrap()
                .is_none()
        );

        // 2. Accept import
        let readiness = RelayService::accept_import(
            db.connection(),
            dir.path(),
            &project_id,
            &preview.import_id,
        )
        .unwrap();

        assert_eq!(readiness.ready_count, 2);

        // Verify files on disk DO exist now
        let pv = ArtifactManager::read_artifact(dir.path(), "design/product-vision.md").unwrap();
        assert!(pv.is_some());
        assert!(pv.unwrap().contains("# Vision"));

        let req = ArtifactManager::read_artifact(dir.path(), "design/requirements.md").unwrap();
        assert!(req.is_some());

        let oq = ArtifactManager::read_artifact(dir.path(), "design/open-questions.md").unwrap();
        assert!(oq.is_some());

        // Verify packet is no longer PENDING
        let pending = RelayService::get_pending_packet(db.connection(), &project_id).unwrap();
        assert!(pending.is_none());
    }

    #[test]
    fn test_process_import_and_reject_leaves_disk_unchanged() {
        let (dir, mut db, project_id) = setup_test_project();

        let packet = RelayService::prepare_architect_packet(
            db.connection_mut(),
            dir.path(),
            &project_id,
            None,
        )
        .unwrap();

        let response_text = format!(
            r##"```yaml
coalition_response:
  schema: 1
  packet_id: "{}"
  project_id: "{}"
  response_type: "ARCHITECT_UPDATE"
  summary: "Draft to be rejected"
  artifacts:
    - path: "design/product-vision.md"
      action: "CREATE"
      content: |
        # Vision
        Content to reject
```"##,
            packet.metadata.packet_id, project_id
        );

        let preview =
            RelayService::process_import(db.connection(), dir.path(), &project_id, &response_text)
                .unwrap();

        RelayService::reject_import(db.connection(), &project_id, &preview.import_id).unwrap();

        // Verify file is NOT created on disk
        assert!(
            ArtifactManager::read_artifact(dir.path(), "design/product-vision.md")
                .unwrap()
                .is_none()
        );

        // History shows rejected
        let history = RelayService::get_relay_history(db.connection(), &project_id, 10).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].status, "REJECTED");
    }

    #[test]
    fn test_malformed_import_failure_and_retry_recovery() {
        let (dir, mut db, project_id) = setup_test_project();

        let packet = RelayService::prepare_architect_packet(
            db.connection_mut(),
            dir.path(),
            &project_id,
            None,
        )
        .unwrap();

        // Malformed YAML (unclosed quote / bad indentation)
        let malformed = format!(
            r##"```yaml
coalition_response:
  schema: 1
  packet_id: "{}"
  project_id: "{}"
  response_type: "ARCHITECT_UPDATE"
  summary: "broken yaml
  artifacts: [
```"##,
            packet.metadata.packet_id, project_id
        );

        let err =
            RelayService::process_import(db.connection(), dir.path(), &project_id, &malformed)
                .unwrap_err();
        let (import_id, raw) = match err {
            RelayError::ParseFailure {
                import_id,
                raw_content,
                ..
            } => (import_id, raw_content),
            other => panic!("Expected ParseFailure, got {:?}", other),
        };

        assert_eq!(raw, malformed);

        // Fixed YAML
        let fixed = format!(
            r##"```yaml
coalition_response:
  schema: 1
  packet_id: "{}"
  project_id: "{}"
  response_type: "ARCHITECT_UPDATE"
  summary: "fixed yaml"
  artifacts:
    - path: "design/product-vision.md"
      action: "CREATE"
      content: |
        # Vision
        Substantive content exceeding fifty characters threshold for test!
```"##,
            packet.metadata.packet_id, project_id
        );

        let recovered = RelayService::retry_parse_import(
            db.connection(),
            dir.path(),
            &project_id,
            &import_id,
            &fixed,
        )
        .unwrap();

        assert_eq!(recovered.summary, "fixed yaml");
        assert_eq!(recovered.artifacts.len(), 1);
    }

    #[test]
    fn test_relay_persistence_across_restart() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_restart.db");

        let project_id = Uuid::new_v4().to_string();
        ArtifactManager::initialize_new_project(dir.path(), "Restart Project").unwrap();

        let packet_id;
        // Session 1: prepare packet and process an import
        {
            let mut db = DbManager::open(&db_path).unwrap();
            db.run_migrations().unwrap();

            let now = chrono::Utc::now().to_rfc3339();
            db.connection()
                .execute(
                    "INSERT INTO projects (project_id, name, repository_path, created_at, updated_at, last_opened_at)
                     VALUES (?1, 'Restart Project', ?2, ?3, ?4, ?5)",
                    params![project_id, dir.path().to_string_lossy().to_string(), now, now, now],
                )
                .unwrap();

            db.connection()
                .execute(
                    "INSERT INTO workflow_state (project_id, state, resume_state, revision, updated_at)
                     VALUES (?1, 'DRAFT', NULL, 1, ?2)",
                    params![project_id, now],
                )
                .unwrap();

            let packet = RelayService::prepare_architect_packet(
                db.connection_mut(),
                dir.path(),
                &project_id,
                None,
            )
            .unwrap();
            packet_id = packet.metadata.packet_id;
        }

        // Session 2: open fresh DbManager (simulating app restart)
        {
            let db = DbManager::open(&db_path).unwrap();

            // 1. Pending packet preserved
            let pending = RelayService::get_pending_packet(db.connection(), &project_id).unwrap();
            assert!(pending.is_some());
            assert_eq!(pending.unwrap().metadata.packet_id, packet_id);

            // 2. Workflow state preserved as ARCHITECTING
            let state: String = db
                .connection()
                .query_row(
                    "SELECT state FROM workflow_state WHERE project_id = ?1",
                    params![project_id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(state, "ARCHITECTING");

            // 3. Workspace state assembled cleanly
            let ws = RelayService::get_workspace_state(db.connection(), dir.path(), &project_id)
                .unwrap();
            assert!(ws.pending_packet.is_some());
            assert_eq!(ws.readiness.total_required, 9);
        }
    }

    #[test]
    fn test_security_untrusted_payload_cannot_mutate_workflow_or_freeze() {
        let (dir, mut db, project_id) = setup_test_project();

        let packet = RelayService::prepare_architect_packet(
            db.connection_mut(),
            dir.path(),
            &project_id,
            None,
        )
        .unwrap();

        // An attacker attempts to inject workflow fields or commands in YAML
        let malicious_output = format!(
            r##"```yaml
coalition_response:
  schema: 1
  packet_id: "{}"
  project_id: "{}"
  response_type: "ARCHITECT_UPDATE"
  summary: "Attack payload"
  workflow_state: "FROZEN"
  execute_command: "rm -rf /"
  permissions: "ICARUS"
  artifacts:
    - path: "design/product-vision.md"
      action: "CREATE"
      content: |
        # Injected Vision
        Attempting to freeze workflow or run shell commands. Exceeds fifty characters!
```"##,
            packet.metadata.packet_id, project_id
        );

        let preview = RelayService::process_import(
            db.connection(),
            dir.path(),
            &project_id,
            &malicious_output,
        )
        .unwrap();

        // Accept the import
        let _ = RelayService::accept_import(
            db.connection(),
            dir.path(),
            &project_id,
            &preview.import_id,
        )
        .unwrap();

        // 1. Workflow state MUST NOT be FROZEN
        let state: String = db
            .connection()
            .query_row(
                "SELECT state FROM workflow_state WHERE project_id = ?1",
                params![project_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            state, "ARCHITECTING",
            "Untrusted AI payload cannot mutate workflow to FROZEN!"
        );

        // 2. Durable project.yaml must still be draft
        let project_yaml =
            ArtifactManager::read_project_yaml(dir.path().join(".coalition/project.yaml")).unwrap();
        assert_eq!(
            project_yaml.architecture_state,
            crate::core::artifacts::ArchitectureState::Draft
        );
        assert!(project_yaml.current_architecture_version.is_none());
    }
}

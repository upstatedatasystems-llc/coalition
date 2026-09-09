# ChatGPT Relay Protocol

## Concept

Coalition V1 integrates with ChatGPT Plus through an **explicit human relay**. This avoids fragile browser DOM scraping, OCR hacks, or unauthorized access to consumer web sessions. The human serves as the trusted bridge, copying governed prompt packets into ChatGPT and copying structured responses back into Coalition.

## Relay Flow

```text
Coalition Desktop App
       │
       ▼ (User clicks "Prepare Prompt" -> "Copy for ChatGPT" or Ctrl+Shift+R)
Operating System Clipboard
       │
       ▼ (User pastes prompt into chatgpt.com via "Open ChatGPT ↗")
ChatGPT (Architect role)
       │
       ▼ (User copies response block from ChatGPT)
Operating System Clipboard
       │
       ▼ (User clicks "Import from Clipboard" or Ctrl+Shift+I)
Coalition Desktop App
       │
       ▼ (Validates strict envelope, schema 1, safe allowlist, baseline fingerprints)
Diff & Change Preview
       │
   ┌───┴───┐
   ▼       ▼
[Accept] [Reject]
   │       │
   │       └─ Zero disk mutation (import discarded)
   │
   ▼
Multi-Artifact Crash-Safe Batch Transaction Protocol:
1. Record batch journal in SQLite (phase: STAGING)
2. Stage all replacement files in same directories (.tmp.<uuid>)
3. Update journal (phase: STAGED)
4. Update journal (phase: COMMITTING)
5. Deterministically replace files in alphabetical order with same-directory backups (.bak.<uuid>)
6. Commit journal in SQLite (phase: COMMITTED)
7. Mark import ACCEPTED and cleanup backup files
8. Evaluate readiness and authoritatively transition workflow state (e.g. to READY_TO_FREEZE)
```

## Security & Safety Rules

1. **No Passive Clipboard Snooping**: The clipboard is inspected only upon explicit user action (clicking "Import from Clipboard" or pressing `Ctrl+Shift+I`). There are no background polling timers or OS hook listeners monitoring clipboard changes.
2. **Strict Wire Schema & Path Containment**:
   - Imported responses must contain a strict YAML or JSON block wrapped in `coalition_response:`, `schema: 1`, and `response_type: "ARCHITECT_UPDATE"`. Untagged direct-payload acceptance is strictly rejected.
   - Proposed artifact paths are checked against the canonical allowlist:
     - `design/product-vision.md`
     - `design/requirements.md`
     - `design/architecture.md`
     - `design/constraints.md`
     - `design/interfaces.md`
     - `design/security.md`
     - `implementation/implementation-plan.md`
     - `implementation/acceptance-criteria.yaml`
     - `implementation/test-plan.md`
     - `design/open-questions.md` (must be submitted via the dedicated top-level `open_questions:` block, never directly in `artifacts:`)
     - `decisions/ADR-*.md`
   - Relative path traversal (`..`), absolute paths (`/` or `C:\`), duplicate paths in a single response, and paths targeting anything outside `.coalition/` are strictly rejected.
3. **Optimistic Concurrency & Semantic Preconditions**:
   - When previewing an import, baseline file existence and SHA-256 content hashes are captured.
   - Action preconditions: `CREATE` requires target to not exist; `MODIFY` and `DELETE` require target to exist.
   - When accepting an import, current file fingerprints are compared against baseline fingerprints; any intervening external disk modification rejects the batch with `STALE_IMPORT_PREVIEW`.
4. **Crash-Safe Multi-File Batch Transaction Protocol & Fail-Closed Reads**:
   - Non-transactional OS filesystems lack multi-file atomic primitives. Coalition achieves multi-file crash safety via a durable write-ahead journal (`relay_import_batch_journal`), temporary staging files, deterministic alphabetical commit ordering, and same-directory `.bak.<uuid>` backups.
   - Pre-mutation backup paths are recorded durably into SQLite before any filesystem replacement begins.
   - `ensure_clean_batch_state` runs before packet preparation, clipboard imports, manual edits, and workspace reads (`get_workspace_state`). If an interrupted batch is found:
     - Batches in `COMMITTING` are rolled back to pre-import state using backup files.
     - Batches in `COMMITTED` are rolled forward to `ACCEPTED` with DB updates.
     - If recovery reconciliation fails or cannot resolve the active journal, all read and mutation paths fail closed with `BATCH_RECOVERY_REQUIRED`, preventing reads of mixed files or state mutations from inconsistent disks.
5. **Strict Relay Lifecycle & Idempotency**:
   - Only 1 pending packet allowed per project; preparing a new packet supersedes any existing pending packet.
   - Only 1 pending import preview allowed; multiple pending imports are rejected.
   - Duplicate imports with identical content are rejected.
   - Only an active pending packet matching the response's `packet_id` may be imported.
6. **Workflow State Gating & Authoritative Transitions**:
   - Relay packet preparation, clipboard imports, and artifact manual editing are permitted only in `DRAFT`, `ARCHITECTING`, or `READY_TO_FREEZE`.
   - Preparing the first packet transitions `DRAFT` to `ARCHITECTING`.
   - Accepting an import that completes all required readiness rules authoritatively transitions `ARCHITECTING` to `READY_TO_FREEZE`. If an edit regresses readiness, the state returns to `ARCHITECTING`.
   - Per-project readiness applicability overrides (`REQUIRED`, `OPTIONAL`, `NOT_APPLICABLE`) in `project.yaml` are honored.
7. **Governed Manual Artifact Editing & Applicability**:
   - Users may view and manually edit architecture artifacts directly in Coalition.
   - Saves verify baseline SHA-256 fingerprints to prevent overwriting concurrent external changes (`STALE_ARTIFACT_CONTENT`).
   - Structured YAML artifacts (`acceptance-criteria.yaml`) are validated for valid YAML syntax (`INVALID_YAML_CONTENT`).
   - Applicability selection (`REQUIRED`, `OPTIONAL`, `NOT_APPLICABLE`) is gated through Rust workflow state and restricted to `ARCHITECTING` and `READY_TO_FREEZE`.
8. **Context Budget & Prompt Guidance**:
   - Architect prompts operate within a 60,000 character context budget.
   - Priority artifacts are included in full; lower-priority artifacts receive explicit truncation markers (`[... truncated for context budget ...]`) when budget is constrained.
   - Prompts include explicit disk inspection guidance for each artifact: `EXISTS ON DISK -> action: MODIFY` or `ABSENT FROM DISK -> action: CREATE`.
9. **Native Global Shortcuts & Narrow ChatGPT Opener**:
   - Registered OS shortcuts `CommandOrControl+Shift+R` (prepare/copy packet) and `CommandOrControl+Shift+I` (import from clipboard) via `tauri-plugin-global-shortcut` and top-level keyboard listeners.
   - The ChatGPT opener is strictly locked to `https://chatgpt.com` via the dedicated `open_chatgpt` command.

## Relay Packet Schema (v1)

Packets prepared by Coalition include typed metadata and context:

```yaml
schema: 1
project_id: "3e1b7c89-2df4-46b7-a021-995f3b7d1e84"
packet_id: "pkt-018f3a9e-..."
role: "ARCHITECT"
packet_type: "ARCHITECT_INITIAL"
architecture_version: "draft"
expected_response: "ARCHITECT_UPDATE"
created_at: "2026-09-08T12:00:00Z"
```

The packet prompt instructs ChatGPT on:
- Human authority invariants (human decides, ChatGPT proposes).
- Required schema and syntax for the response block (YAML `coalition_response:` or JSON `"coalition_response": { ... }`).
- Current repository state, available canonical artifacts, and required next steps.

## Response Payload Schema

Coalition tolerates surrounding conversational prose from ChatGPT, provided a valid `coalition_response` block is present in YAML or JSON format:

```yaml
coalition_response:
  schema: 1
  project_id: "3e1b7c89-2df4-46b7-a021-995f3b7d1e84"
  packet_id: "pkt-018f3a9e-..."
  role: "ARCHITECT"
  response_type: "ARCHITECT_UPDATE"
  summary: "Initial draft of product vision, requirements, and system architecture"
  artifacts:
    - path: "design/product-vision.md"
      action: "CREATE"
      content: |
        # Product Vision
        ...
  open_questions:
    - id: "Q-01"
      question: "Should SQLite be the sole operational store?"
      status: "OPEN"
```

JSON responses are equally supported:

```json
{
  "coalition_response": {
    "schema": 1,
    "project_id": "3e1b7c89-2df4-46b7-a021-995f3b7d1e84",
    "packet_id": "pkt-018f3a9e-...",
    "role": "ARCHITECT",
    "response_type": "ARCHITECT_UPDATE",
    "summary": "Initial draft in JSON format",
    "artifacts": [
      {
        "path": "design/product-vision.md",
        "action": "CREATE",
        "content": "# Product Vision\n..."
      }
    ],
    "open_questions": []
  }
}
```

## Parsing Tolerance & Manual Recovery

The parser strictly enforces the response contract:
1. **Fenced Code Blocks**: Extracts YAML or JSON from ```` ```yaml ... ``` ````, ```` ```json ... ``` ````, ```` ```coalition ... ``` ````, or generic ```` ``` ... ``` ```` code fences.
2. **Raw Fallback**: If fences are omitted, parses the raw string directly for `coalition_response:` or `"coalition_response":`.
3. **Strict Envelope**: Requires the root key `coalition_response` with `schema: 1` and `response_type: "ARCHITECT_UPDATE"`.
4. **Manual Recovery UI**: If malformed syntax cannot be parsed, Coalition renders an inline error recovery editor displaying the exact raw clipboard text. The user can adjust indentation or syntax directly in the UI and click "Retry Parsing" without re-copying.

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
       ▼ (User pastes prompt into chatgpt.com)
ChatGPT (Architect role)
       │
       ▼ (User copies response block from ChatGPT)
Operating System Clipboard
       │
       ▼ (User clicks "Import from Clipboard" or Ctrl+Shift+I)
Coalition Desktop App
       │
       ▼ (Validates schema, path traversal, safe allowlist)
Diff & Change Preview
       │
   ┌───┴───┐
   ▼       ▼
[Accept] [Reject]
   │       │
   │       └─ Zero disk mutation (import discarded)
   │
   ▼
Atomic File Replacement (.coalition/design/...)
Readiness Evaluation (Updates to ReadyToFreeze when 9/9 substantive)
```

## Security & Safety Rules

1. **No Passive Clipboard Snooping**: The clipboard is inspected only upon explicit user action (clicking "Import from Clipboard" or pressing `Ctrl+Shift+I`). There are no background polling timers or OS hook listeners monitoring clipboard changes.
2. **Schema Validation & Path Containment**:
   - Imported responses are parsed for structured YAML blocks wrapped in `coalition_response:`.
   - Proposed artifact paths are checked against the canonical allowlist (`design/product-vision.md`, `design/requirements.md`, `design/architecture.md`, `design/constraints.md`, `design/interfaces.md`, `design/security.md`, `implementation/validation.yaml`, `implementation/acceptance-criteria.yaml`, `implementation/test-plan.md`).
   - Relative path traversal (`..`), absolute paths (`/` or `C:\`), and paths targeting anything outside `.coalition/` are strictly rejected.
3. **No Automatic Execution**: Text, scripts, or instructions contained inside imported responses are never executed as code or shell commands.
4. **Crash-Safe Atomic Writes**: Artifacts are staged in temporary files and committed using native atomic replacement primitives (`ReplaceFileW` on Windows with backup reconciliation, `rename` on POSIX).
5. **Human Authority**: No changes touch the filesystem until the human explicitly clicks "Accept & Apply Changes". Rejecting an import leaves the disk 100% untouched.

## Relay Packet Schema (v1)

Packets prepared by Coalition include typed metadata and context:

```yaml
schema: 1
project_id: "3e1b7c89-2df4-46b7-a021-995f3b7d1e84"
packet_id: "pkt-018f3a9e-..."
role: "ARCHITECT"
packet_type: "ARCHITECT_INITIAL"
architecture_version: "0.1"
expected_response: "ARCHITECT_UPDATE"
created_at: "2026-09-08T12:00:00Z"
```

The packet prompt instructs ChatGPT on:
- Human authority invariants (human decides, ChatGPT proposes).
- Required schema and syntax for the response block (````yaml coalition_response: ... ````).
- Current repository state, available canonical artifacts, and required next steps.

## Response Payload Schema

Coalition tolerates surrounding conversational prose from ChatGPT, provided a valid `coalition_response` block is present:

```yaml
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
```

## Parsing Tolerance & Manual Recovery

The parser incorporates multiple recovery levels:
1. **Fenced Code Blocks**: Extracts YAML from ```` ```yaml ... ``` ````, ```` ```coalition ... ``` ````, or generic ```` ``` ... ``` ```` code fences.
2. **Raw YAML Fallback**: If fences are omitted, parses the raw string directly for `coalition_response:`.
3. **Direct vs Wrapped Envelope**: Tolerates both `coalition_response: { ... }` envelopes and unwrapped payload dictionaries.
4. **Manual Recovery UI**: If malformed syntax cannot be parsed, Coalition renders an inline error recovery editor displaying the exact raw clipboard text. The user can adjust indentation or syntax directly in the UI and click "Retry Parsing" without re-copying.

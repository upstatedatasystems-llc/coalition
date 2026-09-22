# Permissions and Icarus Autonomy

## Core Philosophy

Coalition enforces human authority over tool actions and machine side-effects.

In governed AI-assisted software development, routine and safe inspections proceed automatically, while high-risk or mutating tool operations require explicit human governance.

## Headless CLI Architecture & Permission Reality

In official Google Antigravity CLI (`agy` v1.1.27), running in headless streaming mode (`--input-format stream-json --output-format stream-json`) does not provide a bidirectional stdin protocol for runtime Allow Once / Always Allow prompts. If a tool action requires approval and `--dangerously-skip-permissions` is omitted, the CLI immediately rejects the action and terminates.

### Coalition Governance Principle
Coalition never fakes interactive stdin prompts or silently mutates global Antigravity configuration files. Instead, autonomy is explicitly governed at the launch boundary:
1. **Least-Privilege Mode (Default)**:
   - Antigravity runs without `--dangerously-skip-permissions`.
   - Actions requiring external execution or high-risk mutations are blocked by the engine policy.
   - Blocked actions are captured, classified by risk level, and logged in SQLite permission history with helpful retry guidance for the user.
2. **Icarus Mode (Full Autonomous Execution)**:
   - Explicitly enabled by the human for the active project.
   - Passes `--dangerously-skip-permissions` to allow the Builder to autonomously invoke tools, edit files, and run commands.
   - Persistent, glowing high-visibility banner displayed in the UI whenever active.
   - **Active-Run Immutability**: Toggling project-level Icarus mode while a session is actively executing does not retroactively mutate the running process. The active turn executes strictly under the permission flag established at launch.
   - Can be toggled on or off for subsequent turns at any time.

### Honest Permission Evaluation vs. Generic Runtime Errors

In headless streaming mode, when official `agy` encounters a tool block or permission denial without `--dangerously-skip-permissions`, it emits an execution refusal in stderr without emitting a structured tool-call request or awaiting interactive stdin. 

Coalition inspects this output honestly:
- Because the CLI's generic headless refusal payload does not expose structured tool call parameters, Coalition records the refusal as `tool_name: "UNCLASSIFIED_EXTERNAL_ACTION"` with `risk_level: "HIGH_RISK"` and decision `"BLOCKED"`.
- Coalition does *not* invent fictitious specific tool invocations or guess unprovided targets. `evaluate_tool_risk()` is reserved for structured tool events where explicit tool names and targets are provided by the engine.
- Coalition strictly avoids misclassifying ordinary build, compilation, test, or runtime script failures as permission refusals; only genuine execution refusal patterns trigger permission history entries.

## Risk Classification

The system defines the following canonical risk tiers:
- **`READ_ONLY`**: Non-mutating inspections and queries (`view_file`, `list_dir`, `grep_search`). Safe under all policies.
- **`MUTATING`**: Workspace file creation and modifications (`write_to_file`, `replace_file_content`).
- **`HIGH_RISK`**: Arbitrary command execution (`run_command`, subagent orchestration, external network actions).
- **`CRITICAL`**: Sensitive system modifications or credential operations.
- **`UNCLASSIFIED_EXTERNAL_ACTION`**: External tool invocations where specific tool metadata cannot be deterministically inferred from the engine's refusal payload in headless mode.

## Active-Run Visual Indicators

The UI displays an **Active Run: Icarus Mode** warning banner strictly when the currently executing session was launched with `active_run_icarus == true`. It does not rely on transient project-level settings, preventing false warnings during least-privilege runs.

## Persistent Credential and Secret Redaction

To prevent sensitive credentials from leaking into operational databases or activity history, all persisted text derived from external engine execution is processed through a deterministic, bounded multi-pattern redactor before database insertion:
- **Persistent Text Scope**: Applied to `builder_events` (content and details JSON), `builder_sessions.response_text`, `builder_sessions.error_message`, and `activity_events` metadata and failure messages.
- **Live Terminal Feeds**: Live streaming terminal output in the desktop UI is ephemeral process stdout/stderr held in-memory and capped at the most recent 1,000 lines to prevent DOM bloat; persistent records in SQLite are strictly sanitized.
- **Authorization & Bearer Tokens**: Replaces `Authorization: Bearer <token>` and standalone `Bearer <token>` with `Bearer [REDACTED]`.
- **Known API Keys**: Detects and redacts OpenAI keys (`sk-...`) and Google API keys (`AIza...`).
- **Structured JSON & Configs**: Sanitizes quoted key-value pairs matching sensitive keys (`api_key`, `token`, `password`, `secret`, `access_token`, `refresh_token`, `private_key`) to `"[REDACTED]"`.
- **Command-Line & Script Assignments**: Sanitizes command assignments (`--api_key=...`, `token="quoted-secret"`, `password=xyz`) to `[REDACTED]`.
- **Payload Size Bounds**: Content deltas are capped at 16 KB and detail JSON structures at 32 KB using UTF-8 safe boundary truncation.

## Authoritative Cancellation & Lifecycle State

Cancellation of an active Builder run adheres strictly to single-ownership lifecycle governance:
1. **Cancellation Request**: When the human clicks **Cancel Turn**, the Tauri command `cancel_builder_turn` signals cancellation exclusively through `ActiveBuilderRegistry`.
2. **Process Tree Termination**: On Windows, the registry invokes `taskkill /F /T /PID <pid>` to cleanly terminate the running `agy` process and any spawned child processes without leaving orphaned subprocesses.
3. **No Dual Ownership**: `cancel_builder_turn` does *not* mutate the database session status directly. `BuilderService::start_governed_turn` retains sole authoritative ownership over updating session status to `CANCELLED` once process termination completes.
4. **UI State Preservation**: The frontend transitions the Cancel button to a disabled `⏳ Cancellation requested / terminating…` state while termination is pending, keeping running flags intact until the main execution loop finishes and clears state.

## Scoping & Storage Invariants

- Icarus state and permission history reside exclusively in transient operational storage (`project_icarus_state` and `builder_permission_history` tables in SQLite).
- Permission state is never committed to `.coalition/` durable project files.
- Permission decisions do not leak across projects or to other developers cloning the repository.


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
When `agy` encounters a tool block or permission denial in headless execution without Icarus, it emits an execution refusal in stderr. Coalition inspects this output and classifies the target action (`READ_ONLY`, `MUTATING`, `HIGH_RISK`, `CRITICAL`, or `UNCLASSIFIED_EXTERNAL_ACTION`) using `evaluate_tool_risk`. Coalition strictly avoids misclassifying generic runtime compilation, build, or script errors as permission denials.

## Risk Classification

When tools are evaluated during execution:
- **`READ_ONLY`**: Non-mutating inspections and queries (`view_file`, `list_dir`, `grep_search`). Safe under all policies.
- **`MUTATING`**: Workspace file creation and modifications (`write_to_file`, `replace_file_content`).
- **`HIGH_RISK`**: Arbitrary command execution (`run_command`, subagent orchestration, external network actions).
- **`CRITICAL`**: Sensitive system modifications or credential operations.
- **`UNCLASSIFIED_EXTERNAL_ACTION`**: External tool invocations where the specific sub-command cannot be deterministically inferred from the engine's refusal payload.

## Active-Run Visual Indicators

The UI displays an **Active Run: Icarus Mode** warning banner strictly when the currently executing session was launched with `active_run_icarus == true`. It does not rely on transient project-level settings, preventing false warnings during least-privilege runs.

## Persistent Credential and Secret Redaction

To prevent sensitive credentials from leaking into operational databases, terminal feeds, or activity logs, all persisted event content and details are processed through a deterministic, bounded multi-pattern redactor before database insertion:
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


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
   - Can be toggled on or off at any time.

## Risk Classification

When tools are evaluated during execution:
- **`READ_ONLY`**: Non-mutating queries (`view_file`, `list_dir`, `grep_search`). Safe under all policies.
- **`MUTATING`**: Workspace file modifications (`write_to_file`, `replace_file_content`).
- **`HIGH_RISK`**: Arbitrary command execution (`run_command`, subagent orchestration, external network actions).

## Scoping & Storage Invariants

- Icarus state and permission history reside exclusively in transient operational storage (`project_icarus_state` and `builder_permission_history` tables in SQLite).
- Permission state is never committed to `.coalition/` durable project files.
- Permission decisions do not leak across projects or to other developers cloning the repository.

# Antigravity Builder Integration

## Role & Scope

Google Antigravity serves as the external machine **Builder** in Coalition.

Coalition invokes the official Antigravity CLI binary (`agy`) in structured headless mode:
```text
agy --input-format stream-json --output-format stream-json
```

All communication occurs via standard NDJSON messages over standard input and standard output. Coalition does not use undocumented APIs, synthetic clipboard snooping, or Antigravity GUI automation.

## Bounded Process Execution

All external Builder processes run under the common `ProcessRunner` layer:
- **Child Process Lifecycle**: Tracks process IDs, child trees, cancellation tokens, and timeouts.
- **Windows Process Tree Termination**: On Windows, process cancellation executes `taskkill /F /T /PID <pid>` to cleanly terminate the entire child process tree (including any subprocesses spawned by the Builder).
- **Asynchronous Stdin Streaming**: Prompt lines are written asynchronously to stdin, which is then explicitly closed (EOF) to allow headless CLIs to proceed without hanging.
- **Deadlock & Busy-Spin Prevention**: Output streaming uses guarded select loops with EOF detection, preventing busy-spins when stdout/stderr streams close.
- **Memory Bounding**: Retains up to 10,000 lines or 10 MB of process output in memory, preventing out-of-memory exhaustion during runaway generation while setting the `is_truncated` flag.

## Builder Adapter & Session Management

The `AntigravityCliAdapter` in Rust provides typed, bounded execution for Builder turns:
- **Binary Discovery**: Resolves `agy` from system `PATH`, with fallback using platform environment variables (`%LOCALAPPDATA%\agy\bin\agy.exe` on Windows).
- **Live Model Discovery**: Queries available models dynamically via `agy models`, mapping model names and descriptions.
- **Reasoning Effort**: Passes `--effort <low|medium|high>` when configured.
- **Conversation Resumption & Safe Model Switching**: Captures `conversation_id` from the initial turn and supplies `--conversation <id>` for subsequent turns. Supports mid-conversation model switching by passing the new `--model <name>` alongside `--conversation`.
- **Authoritative Contract Input**: The Builder prompt is strictly derived from the Stage 2 frozen `builder-packet.json`.
- **Autonomy Flag**: When Icarus mode is authorized, passes `--dangerously-skip-permissions` to bypass interactive CLI prompts in headless mode.

## Startup Reconciliation

Because Coalition is a local desktop application that may be closed or crashed mid-turn, the database and application startup lifecycle automatically reconciles transient state:
- Any session marked `RUNNING` in SQLite when the application initializes is reconciled to `INTERRUPTED` with a current timestamp and logged activity event.
- Durable project truth in `.coalition/` remains unaffected.

## Zero-Quota Automated Testing

To ensure automated CI pipelines and local unit tests consume zero Google AI quota:
- Coalition provides a deterministic `fake-agy` test harness (`tests/fake-commands/fake-agy.cjs`).
- Simulates `version`, `models`, multi-turn `--conversation` persistence, reasoning effort, permission rejection, and streaming NDJSON events.
- All automated unit and integration tests run against this test double.

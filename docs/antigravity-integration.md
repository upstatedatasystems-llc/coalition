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
- **Incremental Streaming Memory Bounding**: Consumes stdout and stderr pipes incrementally. Synchronous execution caps in-memory retention at 10 MB per stream, draining excess bytes to null to prevent process pipe backpressure deadlocks. Asynchronous turns enforce a 5 MB buffer ceiling with explicit `is_truncated` tracking.
- **UTF-8 Safe Truncation**: All output truncation respects UTF-8 character boundaries via `truncate_utf8_safe`, eliminating corrupt multi-byte UTF-8 sequences.

## Builder Adapter & Session Management

The `AntigravityCliAdapter` in Rust provides typed, bounded execution for Builder turns:
- **Production Trust Boundary**: Production commands (`start_builder_turn`) strictly resolve the official `agy` binary and never accept fake-adapter overrides.
- **Preflight Validation Before Mutation**: Prior to mutating workflow state, `start_builder_turn` validates repository path, restoration journal cleanliness, frozen snapshot integrity, and contract drift. If any validation fails, execution aborts immediately without mutating workflow state.
- **ActiveBuilderRegistry Concurrency Control**: Tracks executing sessions per project and epoch. Enforces strictly 1 active Builder execution per project (`CONCURRENT_BUILD_FORBIDDEN`) and manages thread-safe, single-ownership cancellation tokens.
- **Decoupled BuilderService**: Core turn execution is encapsulated in `BuilderService`, accepting an optional `BuilderEventSink` (`Arc<dyn Fn(&str, &Value)>`) rather than coupling to GUI windowing handles.
- **Deterministic Event Persistence**: Streamed NDJSON events are sanitized (stripping sensitive paths, auth tokens, and session secrets) and persisted to SQLite with explicit field limits (32 KB `details_json`, 16 KB `text_delta`). The turn handler awaits persistence task completion before returning the turn response.
- **Binary Discovery**: Resolves `agy` from system `PATH`, with fallback using platform environment variables (`%LOCALAPPDATA%\agy\bin\agy.exe` on Windows).
- **Live Model Discovery**: Queries available models dynamically via `agy models`, mapping model names and descriptions without hardcoded fallbacks; discovery errors surface cleanly in the UI.
- **Reasoning Effort**: Passes `--effort <low|medium|high>` when configured.
- **Conversation Resumption & Safe Model Switching**: Captures `conversation_id` from the initial turn and supplies `--conversation <id>` for subsequent turns. Supports mid-conversation model switching by passing the new `--model <name>` alongside `--conversation`.
- **Authoritative Contract Input**: The Builder prompt is strictly derived from the Stage 2 frozen `builder-packet.json`.
- **Autonomy Flag**: When Icarus mode is authorized, passes `--dangerously-skip-permissions` to bypass interactive CLI prompts in headless mode.

## Startup Reconciliation

Because Coalition is a local desktop application that may be closed or crashed mid-turn, the database and application startup lifecycle automatically reconciles transient state:
- Any session marked `RUNNING` in SQLite when the application initializes is reconciled to `INTERRUPTED` with a current timestamp and logged activity event.
- Durable project truth in `.coalition/` remains unaffected.

## Zero-Quota Automated Testing & Diagnostic Isolation

To ensure automated CI pipelines and local unit tests consume zero Google AI quota:
- Coalition provides a deterministic `fake-agy` test harness (`tests/fake-commands/fake-agy.cjs`).
- Simulates `version`, `models`, multi-turn `--conversation` persistence, reasoning effort, permission rejection, and streaming NDJSON events.
- All automated unit and integration tests run against this test double.
- **Diagnostic Proof Isolation**: The desktop diagnostics view provides a dedicated `run_diagnostic_fake_agy_turn` command to verify NDJSON parsing and IPC streaming against the test double without touching project workflows, SQLite state, or epochs.


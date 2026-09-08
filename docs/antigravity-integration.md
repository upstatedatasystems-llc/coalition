# Antigravity Builder Integration

## Role & Scope

Google Antigravity serves as the external **Builder** in Coalition.

Coalition invokes the official Antigravity CLI binary (`agy`) in structured headless mode:
```text
agy --input-format stream-json --output-format stream-json
```

All communication occurs via standard NDJSON messages over standard input and standard output.

## Adapter Architecture

The `AntigravityCliAdapter` in Rust isolates all CLI details from the rest of the application:
- **Executable Discovery**: Resolves `agy` from system `PATH`, with dynamic fallback using platform environment variables (`%LOCALAPPDATA%` on Windows). No developer-specific paths are embedded.
- **Model Discovery**: Queries available models via `agy models`.
- **Session Streaming**: Streams NDJSON events (`init`, `step_update`, `result`).
- **Conversation Tracking**: Captures `conversation_id` from `init` and passes `--conversation <id>` for subsequent turns.
- **Process Cancellation**: Safely terminates the child process and escalates termination if the process fails to exit within a bounded timeout.
- **Autonomy Control**: Maps Icarus mode to `--dangerously-skip-permissions`.

## Zero-Quota Automated Testing

To ensure automated CI pipelines and local unit tests consume zero Google AI quota, Coalition provides a `fake-agy` test double (`tests/fake-commands/fake-agy`). All automated tests run against this harness. Real CLI runs remain strictly opt-in and manual.

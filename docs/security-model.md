# Security Model & Operational Invariants

## Core Principles

1. **Local-First Boundary**:
   - Coalition runs entirely locally. No telemetry, user tracking, or third-party cloud communication occurs.
2. **Narrow Tauri Capabilities**:
   - Frontend JavaScript does not have direct access to arbitrary shell commands or arbitrary filesystem access.
   - All machine-side operations are exposed through typed, validated Rust commands.
3. **Process Safety**:
   - Child processes are invoked directly using explicit arguments without passing concatenated command strings to `cmd.exe /c` or `sh -c`.
   - Process lifecycles are tracked to prevent orphan background processes.
4. **Credential Protection**:
   - Coalition does not store, request, or handle AI provider passwords or API keys.
   - Logs are scrubbed of secrets before storage or relay.
5. **Untrusted AI Text**:
   - Content imported from ChatGPT or output from Antigravity is treated as untrusted text. It is never executed directly as shell commands.

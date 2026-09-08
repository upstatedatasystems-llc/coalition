# Spike B: Project-Scoped Permissions & Autonomy

**Status:** Completed  
**Subject:** Google Antigravity CLI (`agy` v1.1.27) permission modes, interactive approvals, and Icarus mapping  
**Date:** September 8, 2026

## Objective

Determine the supported mechanism for applying Coalition's permission decisions (`deny`, `allow once`, `always allow`, `always deny`, project-scoped rules, and Icarus mode) to Antigravity in headless execution mode.

## Evidence & Observed CLI Behavior

### 1. Default Permission Mode
When invoked in headless stream mode (`--input-format stream-json --output-format stream-json`), `agy` reports:
```json
{
  "event": "init",
  "init": {
    "permission_mode": "request-review",
    "tools": ["..."]
  }
}
```

### 2. Icarus Mapping
When invoked with `--dangerously-skip-permissions`, `agy` reports:
```json
{
  "event": "init",
  "init": {
    "permission_mode": "always-proceed",
    "tools": ["..."]
  }
}
```
This is a direct, verified 1-to-1 mapping for Coalition's **Icarus Mode**.

### 3. Headless Dynamic Permission Responses
Binary inspection and runtime testing of `agy` 1.1.27 stream-json input shows:
- The stream input handler only processes `event: "user"` messages containing user prompt content.
- There is currently no supported NDJSON event over stdin (such as `permission_response` or `allow_tool`) to approve individual tool actions interactively while `agy` is running headlessly.
- In interactive terminal mode, `agy` prompts via readline/terminal UI, which cannot be automated cleanly over standard headless NDJSON pipes.

## Interface Classification

| Capability | Classification | Notes |
| :--- | :--- | :--- |
| Icarus Autonomy Mapping | **PROVEN** | `--dangerously-skip-permissions` maps directly to `always-proceed`. |
| Permission Mode Reporting | **PROVEN** | Emitted in `init` event (`request-review` vs `always-proceed`). |
| Interactive Allow Once via Stream Input | **NOT SUPPORTED** | Stdin stream parser only accepts `user` prompt messages; no dynamic permission response event exists in 1.1.27. |
| Global Unsafe Bypass | **PROVEN** | Supported via `--dangerously-skip-permissions`, but must be scoped and bounded strictly by Coalition. |
| Dynamic Per-Tool Interactive Prompting in Headless Mode | **ARCHITECTURE CONCERN** | In headless non-Icarus mode, actions requiring review cannot be approved interactively via stdin in v1.1.27. |

## Architecture Concern & Mitigation
> [!WARNING]
> **ARCHITECTURE_CONCERN**: In `agy` 1.1.27, headless execution does not provide a bidirectional stdin protocol for runtime tool-approval prompts. 
> 
> **Proposed Direction for Phase 5**:
> 1. Coalition will control autonomy at the process launch boundary (using `--dangerously-skip-permissions` only when Icarus is explicitly enabled by the human).
> 2. When Icarus is disabled, unapproved actions will fail or pause at the turn boundary, where Coalition can inspect git diffs, prompt the user, and restart/resume the conversation with explicit user approvals.
> 3. Coalition must never mutate global machine configuration files to simulate local permissions.

# Spike A: Antigravity Usage & Quota

**Status:** Completed  
**Subject:** Google Antigravity CLI (`agy` v1.1.27) usage, capacity, and quota telemetry  
**Date:** September 8, 2026

## Objective

Determine the most reliable, documented method for Coalition to obtain model, context-window, quota, reset, and token telemetry from the official Antigravity CLI without scraping the GUI or fabricating unavailable data.

## Evidence & Observed CLI Behavior

### 1. Token Metrics
When executing in `--output-format stream-json`, `agy` emits structured token usage in both `step_update` and `result` events:
```json
{
  "event": "step_update",
  "step_update": {
    "conversation_id": "...",
    "step_index": 1,
    "state": "DONE",
    "step_type": "agent_response",
    "text_delta": "...",
    "duration_seconds": 0.92,
    "usage": {
      "input_tokens": 7169,
      "output_tokens": 22,
      "thinking_tokens": 21,
      "cache_read_tokens": 8130,
      "total_tokens": 7191
    }
  }
}
```
And final cumulative metrics on `result`:
```json
{
  "event": "result",
  "result": {
    "status": "SUCCESS",
    "duration_seconds": 1.28,
    "num_turns": 1,
    "usage": {
      "input_tokens": 7181,
      "output_tokens": 79,
      "thinking_tokens": 76,
      "cache_read_tokens": 8130,
      "total_tokens": 7260
    }
  }
}
```

### 2. Context Window & Quota Limits
Inspection of `agy.exe --help`, available subcommands, and NDJSON outputs confirms that `agy` 1.1.27 does **not** expose:
- Maximum context window length.
- Context window percentage remaining.
- Account-level quota buckets, percentage quota remaining, or quota reset countdowns.

## Interface Classification

| Capability | Classification | Notes |
| :--- | :--- | :--- |
| Active Model Reporting | **PROVEN** | Emitted in `init` event `init.model` and configured via `--model`. |
| Incremental & Total Token Counts | **PROVEN** | `input_tokens`, `output_tokens`, `thinking_tokens`, `cache_read_tokens`, `total_tokens` provided in `step_update` and `result`. |
| Context Window Capacity / Remaining % | **NOT SUPPORTED** | Not emitted by `agy` stream-json output in 1.1.27. Coalition must not fabricate this value. |
| Account Quota Buckets / Reset Timers | **NOT SUPPORTED** | No CLI command or JSON field exposes subscription quota balance or reset timers. |
| User Tier / Plan Info | **SUPPORTED BUT NOT FULLY TESTED** | Referenced internally in auth provider logic, but no public command or structured field outputs it. |

## Architectural Recommendations for Coalition
1. In Phase 0 and beyond, present Antigravity token usage strictly from provider-reported `usage` objects.
2. Gracefully handle the absence of quota percentage / reset timers. Do not crash or display mock numbers as provider-reported truth.
3. If context utilization is required in future phases, compute it heuristically against known model context limits or file an ARCHITECTURE_CONCERN before presenting it to users.

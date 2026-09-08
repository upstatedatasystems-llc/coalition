# Usage and Capacity Telemetry

## Overview

Coalition provides visibility into AI capacity without calculating dollar costs:
- **Antigravity Telemetry**: Captures provider-reported token metrics (`input_tokens`, `output_tokens`, `thinking_tokens`, `cache_read_tokens`, `total_tokens`) directly from NDJSON `step_update` and `result` events.
- **ChatGPT Usage Estimate (Phase 6)**: An approximate, heuristic calculation based on relayed prompt and response character counts. It will always be clearly labeled as an estimate with a descriptive disclaimer.

## Telemetry Integrity

- Antigravity numbers are presented as provider-reported facts.
- Any missing or optional fields are tolerated gracefully without crashing the UI.
- Coalition never fakes provider quotas when unavailable.

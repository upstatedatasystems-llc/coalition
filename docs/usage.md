# Usage and Capacity Telemetry

## Overview

Coalition provides comprehensive visibility into model resource consumption without estimating volatile dollar costs or querying paid billing APIs:
- **Antigravity Provider-Reported Tokens**: Exact, authoritative token metrics (`input_tokens`, `output_tokens`, `thinking_tokens`, `cache_read_tokens`, `total_tokens`) captured directly from Antigravity NDJSON streams.
- **ChatGPT Relay Estimated Tokens**: Heuristic token estimations (~4 characters per token) calculated across outbound relay packets and inbound imported clipboard responses.

## Telemetry Integrity & Differentiation

### 1. Antigravity Metrics (Authoritative)
- Metrics originate directly from the model provider engine via `step_update` and `result` event payloads.
- Recorded per session in the `builder_sessions` operational database table.
- Displayed with the **Provider-Reported** badge in the Builder Control Plane.

### 2. ChatGPT Architecture Relay Metrics (Estimated)
- Because ChatGPT interaction is an explicit human relay without direct API ties, tokens cannot be fetched from OpenAI.
- Estimated conservatively using the standard rule of thumb: `estimated_tokens = round(char_count / 4.0)`.
- Recorded for both `OUTBOUND_PACKET` (packet copied to clipboard) and `INBOUND_IMPORT` (architecture preview imported from clipboard).
- Aggregated across two key operational windows:
  - **Rolling 5-Hour Window**: Tracks active burst throughput.
  - **Weekly Rolling Window**: Tracks cumulative sprint volume.
- Includes a dedicated "Reset 5h/7d Usage Window" control allowing the human to recalibrate their tracking period.
- Always accompanied by the prominent disclaimer:
  *"Estimated relay throughput (~4 chars/token heuristic). Does not reflect official OpenAI billing or subscription usage."*

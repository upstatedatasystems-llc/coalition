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

### 2. ChatGPT Architecture Relay Metrics (Estimated & Calibrated)
- Because ChatGPT interaction is an explicit human relay without direct API ties, tokens cannot be fetched from OpenAI.
- Estimated conservatively using a calibrated estimator (`ChatGptUsageEstimator`, default: `4.0 chars/token`).
- Recorded for both `OUTBOUND_PACKET` (when copied to clipboard via `copy_relay_packet_to_clipboard`) and `INBOUND_IMPORT` (when architecture preview is imported from clipboard). Prompt drafting alone does not increment usage until actually copied.
- Aggregated across two key operational windows:
  - **Rolling 5-Hour Window**: Tracks active burst throughput with an estimated capacity percentage against typical tier limits.
  - **Weekly Rolling Window**: Tracks cumulative sprint volume with weekly capacity percentage.
- **Human Calibration**: Includes an interactive estimator calibration modal and `calibrate_chatgpt_usage` command. The user can submit a sample character count alongside an observed OpenAI token count to tune `chars_per_token` and increment the estimator version.
- Includes a dedicated "Reset Window" control allowing the human to restart their tracking window.
- Always accompanied by the prominent disclaimer:
  *"Estimated relay throughput (~4 chars/token heuristic). Does not reflect official OpenAI billing or subscription usage."*

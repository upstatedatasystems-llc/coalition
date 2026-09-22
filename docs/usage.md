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
- **Estimator Provenance**: Every usage event recorded in `chatgpt_usage_history` persists the specific `estimator_version` and `chars_per_token` in effect at record time.
- **Copy-First Invariant**: For `OUTBOUND_PACKET` events, usage is recorded only *after* the OS clipboard write has succeeded. Draft generation or clipboard write failures do not record usage.
- Aggregated across two key operational windows:
  - **Rolling 5-Hour Window**: Tracks active burst throughput.
  - **Weekly Rolling Window**: Tracks cumulative sprint volume.
- **Honest Capacity Grounding**: Capacity percentages remain unset (`null`) unless explicitly configured with authoritative tier quotas, preventing misleading or arbitrary utilization percentages.
- **Human Calibration**: Includes an interactive estimator calibration modal and `calibrate_chatgpt_usage` command. The user can submit a sample character count alongside an observed OpenAI token count to tune `chars_per_token` and increment the estimator version.
- Includes a dedicated "Reset Window" control allowing the human to restart their tracking window.
- Always accompanied by the prominent disclaimer:
  *"Estimated relay throughput (~4 chars/token heuristic). Does not reflect official OpenAI billing or subscription usage."*


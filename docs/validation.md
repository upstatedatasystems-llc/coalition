# Validation Subsystem Design

## Purpose

The Validation subsystem executes deterministic checks (builds, tests, linters, type checks) to generate evidence for:
- The Builder (Antigravity) as diagnostic feedback.
- The Reviewer (ChatGPT) as proof of correctness.
- The Human as transparency and gating controls.

## Optional & Configurable

Validation is never mandatory across all projects. A project's `.coalition/implementation/validation.yaml` declares whether validation is enabled, whether checks are sequential, and whether failed required checks gate review transitions.

### Post-Build Orchestration & Skip Path

When a Builder turn completes successfully:
1. **Disabled / Unconfigured Validation**: If `.coalition/implementation/validation.yaml` is absent or `enabled: false`, no external processes are executed. The workflow legally transitions `BUILDING -> VALIDATING` (`StartValidation`), records a `VALIDATION_SKIPPED` activity audit event with rationale metadata, evaluates `check_review_gate` (which evaluates as satisfied), and transitions `VALIDATING -> WAITING_FOR_REVIEW` (`SubmitForReview`).
2. **Diagnostic-Only Mode**: When `policy.gate_review_on_required_failure: false`, configured validation commands are executed and dual disk-streamed, but command failures do not block gate approval (`is_gate_passed: true`). The project automatically transitions to `WAITING_FOR_REVIEW`.
3. **Required Validation Gate**: When `policy.gate_review_on_required_failure: true`, any failing required command marks the run status as `FAIL` and `is_gate_passed: false`. The workflow remains paused in `VALIDATING` until human override or automated correction.
4. **Fail-Closed Orchestration**: Any unexpected failure during post-build validation orchestration records a `VALIDATION_ORCHESTRATION_FAILED` activity event and fails closed. Builder-requested validation (`COALITION_REQUEST_VALIDATION`) errors record `BUILDER_VALIDATION_FAILED` audit events.

## Routable Validation Diagnostics to Builder

When a validation run produces failures or timeouts while in the `VALIDATING` state:
- The operator can dispatch diagnostics to the Builder via `start_builder_diagnostic_turn`.
- **Preflights**: The run is verified to belong to the project, match the active frozen architecture version, and match the current builder epoch.
- **Workflow State**: The workflow transitions `VALIDATING -> CORRECTIONS_REQUIRED` via `RequestCorrections`, logs `VALIDATION_DIAGNOSTICS_ROUTED_TO_BUILDER`, and initiates a Builder turn with `BuilderInstructionSource::ValidationDiagnostic`.
- **Sanitized Packet**: Antigravity receives a bounded, secret-sanitized diagnostic tail report appended to the canonical frozen prompt, legally transitioning `CORRECTIONS_REQUIRED -> BUILDING`.

## Review Import Governance & Rejection

Independent reviewer responses are parsed into server-owned previews (`review_import_previews`):
- **Confirmation (`confirm_review_import`)**: Validates working tree freshness, reconciles journal, commits findings, removes preview, and advances workflow.
- **Rejection (`reject_review_import`)**: Dedicated governed operation that deletes/invalidates the pending server preview, keeps the review cycle `PENDING`, leaves the workflow in `WAITING_FOR_REVIEW`, logs `REVIEW_IMPORT_REJECTED`, and enables clean subsequent imports without leaving orphaned previews.

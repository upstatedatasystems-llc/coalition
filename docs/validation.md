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

When a validation run produces failures or timeouts, Coalition supports two governed diagnostic-routing paths depending on the workflow state and trigger:

1. **Post-Build Validation Failures (`VALIDATING` / `CORRECTIONS_REQUIRED`)**:
   - For failed or timed-out `POST_BUILD` validation runs while the workflow is in `VALIDATING`.
   - The workflow legally transitions `VALIDATING -> CORRECTIONS_REQUIRED` via `RequestCorrections`, logs `VALIDATION_DIAGNOSTICS_ROUTED_TO_BUILDER`, and initiates a Builder turn with `BuilderInstructionSource::ValidationDiagnostic`.
   - The Builder turn transitions `CORRECTIONS_REQUIRED -> BUILDING`.

2. **Builder-Requested Validation Failures (`BUILDING`)**:
   - When a Builder turn outputs `COALITION_REQUEST_VALIDATION`, an intermediate validation run executes with trigger `BUILDER_REQUESTED` while the project remains in `BUILDING`.
   - If that validation run fails or times out, the operator can dispatch diagnostics directly back to the Builder via `start_builder_diagnostic_turn` without routing through `CORRECTIONS_REQUIRED`.
   - This provides intermediate inner-loop development feedback rather than a Reviewer correction cycle; the project remains in `BUILDING`.
   - Once the Builder resolves the defect and completes normally, ordinary `POST_BUILD` validation orchestration executes automatically and advances the project to `WAITING_FOR_REVIEW`.

3. **Authoritative Invariants & Eligibility Rules**:
   - The validation run must belong to the active project.
   - The validation run status must be `FAIL` or `TIMEOUT`.
   - The run architecture version must match the active frozen architecture version.
   - The run epoch must match the active Builder epoch.
   - In `BUILDING` state: only `BUILDER_REQUESTED` runs qualify (preventing `MANUAL` or `POST_BUILD` runs from bypassing state checks).
   - In `VALIDATING` or `CORRECTIONS_REQUIRED` states: only `POST_BUILD` runs qualify.
   - Frontend cannot supply arbitrary prompt text or executable commands; diagnostic contents are generated authoritatively by Coalition from stored execution records and remain bounded and secret-sanitized.
   - The UI only presents and enables "Send Diagnostics to Builder" when the selected run matches the required trigger and state eligibility criteria.

## Review Import Governance & Rejection

Independent reviewer responses are parsed into server-owned previews (`review_import_previews`):
- **Confirmation (`confirm_review_import`)**: Validates working tree freshness, reconciles journal, commits findings, removes preview, and advances workflow.
- **Rejection (`reject_review_import`)**: Dedicated governed operation that deletes/invalidates the pending server preview, keeps the review cycle `PENDING`, leaves the workflow in `WAITING_FOR_REVIEW`, logs `REVIEW_IMPORT_REJECTED`, and enables clean subsequent imports without leaving orphaned previews.

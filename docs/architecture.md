# Coalition Architecture

## 1. System Overview

Coalition is a local-first desktop control plane that coordinates:
- A human product owner.
- ChatGPT Plus conversations acting as Architect and Reviewer through explicit human relay.
- Google Antigravity acting as the Builder through its official CLI (`agy`).
- Local Git repositories, local project artifacts, and project-defined validation.

Coalition is not a coding agent, IDE, or AI API gateway. Its primary responsibility is governance: ensuring that implementation never drifts away from the design the human approved.

## 2. Core Authority Hierarchy

```text
Human
  ↓
Frozen Architecture Contract
  ↓
Deterministic Evidence (Git diff, tests, builds)
  ↓
Builder (Antigravity) + Independent Reviewer (ChatGPT)
  ↓
Human Final Acceptance
```

Only the human may:
- Freeze an architecture version.
- Authorize architecture revisions.
- Grant sensitive permissions or activate Icarus mode.
- Accept the final implementation.

## 3. Technology Stack

- **Application Form**: Native cross-platform desktop application using Tauri 2.x.
- **Frontend**: React 19, TypeScript, Vite. Modular project dashboard and diagnostics shell.
- **Backend**: Rust MSVC on Windows (secondary target macOS, future Linux).
- **Persistence Boundary**:
  - `.coalition/` filesystem hierarchy for durable project meaning (survives SQLite loss). Portable, Git-versioned, contains no local absolute machine paths.
  - SQLite database (`coalition.db`) for local operational state: project registrations, authoritative workflow state and monotonic revisions, structured activity events, and app settings.
- **IPC Error Contract**: Structured, typed `CommandError` (`code`, `message`, `details`) preventing string-matching control flow in frontend.

## 4. Role Separation

- **Architect (ChatGPT Plus Relay)**: Collaborates with the human to explore intent, analyze trade-offs, and draft architecture contracts. *(Deferred to Phase 2)*.
- **Builder (Google Antigravity CLI)**: Executes bounded implementation milestones strictly against the frozen architecture contract. *(Core adapter proven in Phase 0, UI integration deferred to Phase 4)*.
- **Reviewer (ChatGPT Plus Relay)**: Independently critiques Git diffs and validation evidence in an isolated conversation to verify conformance with the contract. *(Deferred to Phase 6)*.

## 5. Authoritative Workflow State Machine

Authored and enforced authoritatively in Rust across 19 explicit states:
- Mainline progression:
  `DRAFT → ARCHITECTING → READY_TO_FREEZE → FROZEN → BUILDING → VALIDATING → WAITING_FOR_REVIEW → REVIEW_ACCEPTED → FINAL_VALIDATION → READY_FOR_HUMAN_REVIEW → HUMAN_ACCEPTED` (Terminal).
- Reversible preparation:
  `READY_TO_FREEZE ↔ ARCHITECTING`
  `READY_TO_REFREEZE ↔ ARCHITECTING_REVISION`
- Correction loop:
  `WAITING_FOR_REVIEW → CORRECTIONS_REQUIRED → BUILDING`
- Architecture revision loop:
  Permitted from post-freeze states (`FROZEN`, `BUILDING`, `VALIDATING`, `WAITING_FOR_REVIEW`, `CORRECTIONS_REQUIRED`, `BLOCKED`, `ARCHITECTURE_CONCERN`, `REVIEW_ACCEPTED`, `FINAL_VALIDATION`, `READY_FOR_HUMAN_REVIEW`) as well as `PAUSED` and `INTERRUPTED` states when their recorded `resume_state` is a post-freeze development state, via `RequestArchitectureChange → ARCHITECTURE_CHANGE → ARCHITECTING_REVISION → READY_TO_REFREEZE → FROZEN`. Requesting architecture change from paused/interrupted clears the prior resume state.
- Operational suspension & recovery:
  `Pause` / `Resume` (preserving `resume_state`), `Interrupt` / `Resume`, `Block` / `Unblock`, and `RaiseArchitectureConcern` / `ResolveArchitectureConcern`.
- Invariants:
  - React cannot mutate workflow state arbitrarily.
  - Every valid transition atomically increments the workflow revision and writes a `WORKFLOW_TRANSITION` activity event.
  - Human acceptance is valid only from `READY_FOR_HUMAN_REVIEW` and is strictly terminal.
  - Corrupted or unrecognized workflow states in SQLite are treated as errors (`CORRUPTED_STATE`) rather than silently converted to `None`.

## 6. Project Rehydration & Persistence

## 6. Project Rehydration & Persistence

- **Project Registration**: Canonicalizes Git repository root, queries SQLite canonical path before generating durable identity, inspects or recovers `.coalition/project.yaml`, registers or updates the operational record in SQLite, ensures workflow state exists, and records `PROJECT_REGISTERED`, `PROJECT_OPENED`, or `PROJECT_REHYDRATED`.
- **Operational-First Path Check (Case D)**: If a repository path is registered in SQLite but its durable `project.yaml` is missing on disk, registration immediately fails with typed `DURABLE_CONTRACT_MISSING` and performs zero durable identity mutation on disk.
- **Transactional Operational Writes**: Operational state creation/updates across `projects`, `workflow_state`, `activity_events`, and `app_settings` run inside an explicit SQLite transaction. Any error rolls back all operational changes, preventing orphaned partial records.
- **Identity Conflict & Move Reconciliation**: Deterministically handles collisions between durable `project_id`, canonical filesystem path, and existing SQLite rows:
  - If a project folder was moved/renamed and the old path no longer exists on disk, Coalition updates the registered repository path.
  - If distinct projects collide on ID or path, Coalition rejects registration with a typed `PROJECT_IDENTITY_CONFLICT` error.
- **SQLite Loss Recovery**: If SQLite is deleted, reopening a governed repository reconstructs operational records from the durable `.coalition/project.yaml` metadata, maintaining durable project identity and restoring workflow state according to durable architecture state (`Frozen` or `Draft`).
- **Unavailable Repository Handling**: If a registered repository directory is moved or deleted, SQLite retains the registration, marks it `is_available: false`, and sets `artifact: None`. Coalition never assumes or fabricates a synthetic "Draft" contract for inaccessible repositories.
- **Selective Startup Restoration**: Startup check inspects repository availability before auto-selecting the last-opened project. If the repository is offline, the UI remains on the project list with the repository marked unavailable and no modal error dialog.

## 7. Safe Contract Replacement & Filesystem Integrity

- **Crash-Safe Platform-Native Atomic Replacement**: Modifying `project.yaml` validates invariants (strictly requiring RFC 4122 UUID v4 and exact draft/frozen version rules), writes and flushes replacement data before invoking the native replacement primitive:
  - On Windows: Coalition uses `ReplaceFileW` with `dwReplaceFlags = 0` (Microsoft documents `REPLACEFILE_WRITE_THROUGH` as unsupported for `ReplaceFileW`), a same-directory backup (`project.yaml.bak.<uuid>`), post-call destination validation before unlinking the backup, and reconciliation of documented failure states. If the destination does not exist initially, it falls back to `MoveFileExW` with write-through.
  - On POSIX: `rename` followed by directory sync.
  - Coalition flushes replacement data before invoking the native replacement primitive. On Windows it uses `ReplaceFileW` with a same-directory backup, reconciles documented failure states, and never authorizes recovery candidate promotion until project identity has been validated.
- **Non-Mutating Inspection & Identity-Before-Promotion**: On project inspection (`inspect_project_artifacts`), valid canonical `project.yaml` takes precedence and stale temp files are cleaned. If canonical `project.yaml` is missing, recovery candidates (`project.yaml.bak.*`) are inspected in a strictly non-mutating manner and are never promoted until `ProjectService` validates project identity against SQLite registration history. Conflicting identities, ambiguous/multiple backups, or orphaned temp files return typed errors (`PROJECT_IDENTITY_CONFLICT`, `ARTIFACT_RECOVERY_REQUIRED`) with zero durable mutations.
- **Hierarchy & Layout Validation**: All standard `.coalition/` subdirectories (`design`, `implementation`, `decisions`, `architecture-versions`, `changes`, `reviews`, `evidence`) and `project.yaml` are validated against path traversal (`..`) and Windows junction / symlink reparse points escaping the repository root.

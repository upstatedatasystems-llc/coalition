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
  Permitted from post-freeze states (`FROZEN`, `BUILDING`, `VALIDATING`, `WAITING_FOR_REVIEW`, `CORRECTIONS_REQUIRED`, `BLOCKED`, `ARCHITECTURE_CONCERN`, `REVIEW_ACCEPTED`, `FINAL_VALIDATION`, `READY_FOR_HUMAN_REVIEW`) via `RequestArchitectureChange → ARCHITECTURE_CHANGE → ARCHITECTING_REVISION → READY_TO_REFREEZE → FROZEN`.
- Operational suspension & recovery:
  `Pause` / `Resume` (preserving `resume_state`), `Interrupt` / `Resume`, `Block` / `Unblock`, and `RaiseArchitectureConcern` / `ResolveArchitectureConcern`.
- Invariants:
  - React cannot mutate workflow state arbitrarily.
  - Every valid transition atomically increments the workflow revision and writes a `WORKFLOW_TRANSITION` activity event.
  - Human acceptance is valid only from `READY_FOR_HUMAN_REVIEW` and is strictly terminal.

## 6. Project Rehydration & Persistence

- **Project Registration**: Canonicalizes Git repository root, creates or loads `.coalition/project.yaml`, registers or updates the operational record in SQLite, ensures workflow state exists, and records `PROJECT_REGISTERED`, `PROJECT_OPENED`, or `PROJECT_REHYDRATED`.
- **SQLite Loss Recovery**: If SQLite is deleted, reopening a governed repository reconstructs operational records from the durable `.coalition/project.yaml` metadata, maintaining durable project identity and restoring workflow state according to durable architecture state.
- **Unavailable Repository Handling**: If a registered repository directory is moved or deleted, SQLite retains the registration, marks it `is_available: false`, and surfaces this clearly in the UI without silent data loss.

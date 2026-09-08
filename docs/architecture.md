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
- **Frontend**: React 19, TypeScript, Vite. (Strictly minimal CSS in Phase 0).
- **Backend**: Rust MSVC on Windows (secondary target macOS, future Linux).
- **Persistence**:
  - `.coalition/` filesystem hierarchy for durable project meaning (survives SQLite loss).
  - SQLite database for local operational state, process tracking, and workflow sessions.

## 4. Role Separation

- **Architect (ChatGPT Plus Relay)**: Collaborates with the human to explore intent, analyze trade-offs, and draft architecture contracts.
- **Builder (Google Antigravity CLI)**: Executes bounded implementation milestones strictly against the frozen architecture contract.
- **Reviewer (ChatGPT Plus Relay)**: Independently critiques Git diffs and validation evidence in an isolated conversation to verify conformance with the contract.

## 5. Workflow State Machine

Authored and enforced authoritatively in Rust:
```text
DRAFT → ARCHITECTING → READY_TO_FREEZE → FROZEN → BUILDING 
  → VALIDATING → WAITING_FOR_REVIEW → (CORRECTIONS_REQUIRED ↔ BUILDING)
  → REVIEW_ACCEPTED → FINAL_VALIDATION → READY_FOR_HUMAN_REVIEW → HUMAN_ACCEPTED
```
At any point during development, the human may invoke **Change Architecture**, safely pausing the Builder and returning the project to `ARCHITECTURE_CHANGE` for revision and re-freezing.

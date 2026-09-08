# Coalition Invariants & Agent Guidelines

Coalition is a local-first desktop control plane for governed AI-assisted software development.

All AI agents (including Google Antigravity) working on this codebase must adhere strictly to these non-negotiable invariants derived from the Master Implementation Plan:

## 1. Human Authority Hierarchy
```text
Human
  ↓
Frozen Architecture Contract
  ↓
Configured Validation / Evidence
  ↓
Builder + Independent Reviewer
  ↓
Human Final Acceptance
```
- Only the human may freeze, revise, or re-freeze architecture contracts.
- Only the human may authorize architecture changes.
- Only the human may grant high-risk permissions or enable Icarus mode.
- Only the human may accept the final product.

## 2. Architecture Is the Contract
- Once frozen, architecture artifacts are immutable contracts.
- AI agents may not silently change architecture requirements or constraints.
- If a requirement is contradictory, impractical, or impossible with supported tools, agents must pause and produce an `ARCHITECTURE_CONCERN` rather than inventing undocumented workarounds.

## 3. Separation of Concerns & State
- `.coalition/` stores durable project truth (what the project *is*).
- SQLite stores transient operational state (what Coalition is *doing*).
- Project truth must survive SQLite deletion.
- Rust owns authoritative machine-side state transitions.
- React frontend must never bypass Rust state checks.

## 4. Third-Party Integrations
- No ChatGPT DOM, web, or UI scraping; no OCR; no synthetic clipboard snooping; no undocumented API endpoints.
- ChatGPT interaction is explicit human relay only.
- No AI API dependencies in V1.
- No Antigravity GUI automation.
- All external tool protocols (Git, `agy`) must remain adapter-isolated behind typed Rust interfaces.

## 5. Process & Permission Safety
- All external command execution must use a common, bounded process-management layer.
- Timeouts, cancellation, and process trees must be tracked.
- Output streams must be bounded in memory.
- Dynamic permission decisions must default to least-privilege.
- Icarus mode (full auto-approval) must always be visibly indicated when active.
- No secrets, private credentials, or developer-specific local paths in code, logs, or commits.

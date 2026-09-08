# Changelog

All notable changes to Coalition will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [Unreleased]

### Added - Phase 1 (Coalition Core and Project Persistence)
- Authoritative Rust workflow state machine implementing all 19 V1 states with strict transition validation, pause/resume state preservation, atomic revision increments, and human architecture change requests from `PAUSED` and `INTERRUPTED` states when paused from post-freeze development.
- Durable `.coalition/` project contract hierarchy (`project.yaml`, `design/`, `implementation/`, `decisions/`, `architecture-versions/`, `changes/`, `reviews/`, `evidence/`) with complete layout validation against path traversal and symlink/junction reparse points.
- Portable, schema-versioned `project.yaml` with stable UUID v4 project identity, RFC3339 timestamp validation, non-empty project names, draft vs. frozen version invariants, and Windows-safe atomic replacement via staging and backup rollback.
- Forward SQLite migration 2 (`002_phase1_core_persistence`) creating `projects`, `workflow_state`, `activity_events`, and `app_settings` tables with enforced foreign key integrity.
- Transactional project registration and operational persistence wrapping `projects`, `workflow_state`, `activity_events`, and `app_settings` writes in an explicit SQLite transaction with rollback on failure.
- Deterministic project identity conflict and moved-repository reconciliation detecting collisions on `project_id` and canonical path.
- Non-fabricated durable contract representation (`Option<ProjectYaml>`) ensuring unavailable repositories display offline state without assuming or fabricating Draft contracts.
- Strict authoritative state parsing in `list_projects` surfacing corrupted state strings as typed `CORRUPTED_STATE` errors rather than swallowing them.
- Rehydration service restoring operational state from durable `.coalition/project.yaml` when local SQLite database is deleted or recreated.
- Git repository adapter extensions: repository root canonicalization, clean/dirty detection (staged, unstaged, untracked counts), 0-commit and detached HEAD handling without crashing.
- Structured Tauri IPC error boundary (`CommandError`) with typed error codes, human-readable messages, and safe structured details, eliminating string-matching control flow.
- Project dashboard frontend: empty state, project list with workflow and Git badges, project detail view with live status and refresh, recent activity timeline, open repository modal, and selective startup restoration avoiding broken auto-navigation for offline repositories.
- Complete 10-step end-to-end desktop lifecycle smoke test (`phase1_smoke_test.rs`).
- Preserved Phase 0 diagnostics view with React StrictMode singular stream listener fix.
- Comprehensive automated test suite across Rust unit/integration tests and frontend vitest tests.

### Added - Phase 0 (Bootstrap & Technical Proofs)
- Initial public repository baseline for `upstatedatasystems-llc/coalition`.
- Core invariants and agent instructions in `AGENTS.md`.
- Staged development skills in `.agents/skills/`.
- Tauri 2.x + React + TypeScript + Vite + Rust MSVC desktop scaffolding.
- Minimal Phase 0 technical diagnostic UI.
- Isolated Git adapter querying branch, status, diff, and commit info.
- Isolated Antigravity CLI adapter (`agy`) supporting detection, versioning, model enumeration, NDJSON stream parsing, and cancellation.
- Bounded process runner with streaming stdout/stderr and graceful/forced cancellation.
- SQLite versioned migration proof and operational schema foundation.
- Zero-quota `fake-agy` test double harness for deterministic CI/CD and unit testing.
- Initial CI workflow `.github/workflows/ci.yml`.
- Technical spike documentation for usage metrics, permissions, model switching, and process ownership.

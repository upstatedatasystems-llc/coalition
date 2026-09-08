# Changelog

All notable changes to Coalition will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [Unreleased]

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

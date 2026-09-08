# Coalition

Coalition is a local-first desktop control plane for governed AI-assisted software development.

It coordinates:
- Human product authority
- ChatGPT Plus subscription conversations (via explicit human relay for Architect and Reviewer roles)
- Google Antigravity (as the external Builder via official CLI)
- Local Git repositories and project artifacts
- Deterministic project-defined validation

Coalition ensures that AI coding work remains strictly accountable to an explicit, human-approved architecture, and that the human can inspect, pause, revise, and resume the implementation at any point.

## Status: Phase 1 (Coalition Core and Project Persistence)

This repository is currently at **Phase 1 — Coalition Core and Project Persistence**.

Phase 1 establishes:
- Local project registration and re-opening with Git root canonicalization.
- Durable `.coalition/` project contract hierarchy and portable `project.yaml` metadata (schema v1, stable UUID v4, typed architecture state).
- Operational SQLite database persistence with forward migrations for projects, workflow state, activity events, and app settings.
- Authoritative Rust workflow state machine enforcing all 19 V1 states, explicit transition validation, pause/resume state tracking, and atomic revision increments.
- SQLite-loss recovery and rehydration reconstructing operational state from durable `.coalition/project.yaml`.
- Git repository inspection surfacing branch, HEAD commit, clean vs. dirty status (staged, unstaged, untracked counts), and diff stats, handling empty repositories and detached HEAD gracefully.
- Structured Tauri IPC error boundary (`CommandError`) with typed error codes and safe context.
- Modular React frontend featuring the primary project dashboard (empty state, project cards, detail view, refresh, activity timeline, open repository modal) and dedicated diagnostics.
- Unavailable repository detection preserving registrations when folders are moved or deleted.
- Automatic restoration of last-opened project on startup.
- Full preservation of Phase 0 technical proofs and fake-`agy` test harness.

## Technology Stack

- **Desktop Framework**: Tauri 2.x
- **Frontend**: React 19, TypeScript, Vite
- **Backend & Core Engine**: Rust (MSVC toolchain on Windows)
- **Local Persistence**: SQLite (operational state) & `.coalition/` (durable project truth)

## Development Setup

See [docs/development.md](docs/development.md) for full prerequisites and environment setup.

### Quick Start

```powershell
# Install frontend dependencies
npm install

# Run frontend typecheck
npm run typecheck

# Run frontend tests
npm test -- --run

# Run frontend build
npm run build

# Run Rust formatting check
cargo fmt --manifest-path src-tauri/Cargo.toml --check

# Run Rust linter
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings

# Run Rust unit tests
cargo test --manifest-path src-tauri/Cargo.toml
```

## Documentation

- [Architecture Overview](docs/architecture.md)
- [Development Guide](docs/development.md)
- [Antigravity Integration](docs/antigravity-integration.md)
- [Security Model](docs/security-model.md)
- [Permissions & Autonomy](docs/permissions.md)
- [Validation Subsystem Design](docs/validation.md)
- [Usage & Capacity Design](docs/usage.md)
- [ChatGPT Relay Protocol](docs/chatgpt-relay.md)
- [Durable Project Artifacts](docs/project-artifacts.md)
- [Technical Spikes](docs/spikes/)

## License & Copyright

Copyright © 2026 Upstate Data Systems LLC.  
Licensing terms have not yet been selected. This repository is publicly visible for evaluation and collaboration, but no open-source license is granted at this time.

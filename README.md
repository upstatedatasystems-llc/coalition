# Coalition

Coalition is a local-first desktop control plane for governed AI-assisted software development.

It coordinates:
- Human product authority
- ChatGPT Plus subscription conversations (via explicit human relay for Architect and Reviewer roles)
- Google Antigravity (as the external Builder via official CLI)
- Local Git repositories and project artifacts
- Deterministic project-defined validation

Coalition ensures that AI coding work remains strictly accountable to an explicit, human-approved architecture, and that the human can inspect, pause, revise, and resume the implementation at any point.

## Status: Phase 0 (Bootstrap & Technical Proofs)

This repository is currently at **Phase 0 — Repository Bootstrap and Technical Proofs**.

Phase 0 establishes:
- Tauri 2.x + React + TypeScript + Vite + Rust desktop runtime.
- System Git detection and isolated Git adapter.
- Isolated Antigravity CLI adapter (`agy`) supporting headless stream I/O, conversation tracking, model discovery, and process cancellation.
- Versioned SQLite local operational state proof.
- Bounded process runner with streaming logs and cancellation.
- Test harness (`fake-agy`) for deterministic zero-quota automated testing.
- Initial technical spikes for usage metrics, permissions, model switching, and process ownership.

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

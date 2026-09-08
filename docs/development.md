# Development Guide

This guide details environment setup, build steps, and verification procedures for Coalition.

## Prerequisites

- **OS**: Windows 10/11 (MSVC environment), macOS, or Linux.
- **Node.js**: v20+ (v22 recommended) with npm.
- **Rust**: 1.85+ with MSVC toolchain on Windows (`rustup default stable-x86_64-pc-windows-msvc`).
- **C++ Build Tools**: Visual Studio C++ Build Tools with Windows SDK.
- **Git**: System Git installed and on `PATH`.
- **Antigravity CLI (`agy`)**: Official Antigravity CLI binary installed and authenticated.

## Project Structure

```text
coalition/
├── AGENTS.md                  # Durable project invariants for agents
├── README.md                  # Project overview and status
├── CONTRIBUTING.md            # Guidelines for changes
├── SECURITY.md                # Security and reporting policy
├── CODE_OF_CONDUCT.md         # Community standards
├── CHANGELOG.md               # Version history
├── package.json               # Node workspace configuration
├── tsconfig.json              # TypeScript root configuration
├── vite.config.ts             # Vite build configuration
├── .agents/skills/            # Agent development skills
├── .coalition/                # Project self-validation & artifacts
├── docs/                      # Architecture and integration docs
│   └── spikes/                # Phase 0 technical spike findings
├── src/                       # Frontend source (React 19 + TS + Vite)
│   ├── app/                   # Root shell layout
│   ├── features/              # Modular features
│   │   ├── activity/          # Activity timeline
│   │   ├── diagnostics/       # Phase 0 diagnostics panel
│   │   └── projects/          # Project list, detail, empty state, open modal
│   └── types/                 # Domain & IPC TypeScript definitions
├── src-tauri/                 # Rust core backend & Tauri configuration
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── src/
│   │   ├── commands/          # Narrow Tauri IPC command handlers & CommandError
│   │   ├── core/
│   │   │   ├── activity/      # Structured activity events & queries
│   │   │   ├── artifacts/     # Durable .coalition structure & project.yaml
│   │   │   ├── builder/       # Antigravity CLI adapter & process runner
│   │   │   ├── git/           # Git adapter & repository inspection
│   │   │   ├── process/       # Bounded async process runner
│   │   │   ├── projects/      # Project registration, rehydration, availability
│   │   │   └── workflow/      # 19-state authoritative workflow state machine
│   │   └── db/                # SQLite connection, migrations (001 & 002)
│   └── tests/
│       └── phase1_smoke_test.rs # Desktop lifecycle smoke tests
└── tests/
    └── fake-commands/         # Zero-quota fake-agy test double
```

## Running Verification Locally

All code contributions must pass the complete verification suite with zero errors or warnings:

```powershell
# 1. TypeScript typecheck
npm run typecheck

# 2. Frontend Vitest suite (headless, non-interactive)
npm test -- --run

# 3. Frontend production build
npm run build

# 4. Rust formatting check
cargo fmt --manifest-path src-tauri/Cargo.toml --check

# 5. Rust Clippy with strict warnings
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings

# 6. Rust unit & integration test suite
cargo test --manifest-path src-tauri/Cargo.toml

# 7. Rust compilation check
cargo build --manifest-path src-tauri/Cargo.toml

# 8. Dedicated Phase 1 End-to-End Desktop Lifecycle Smoke Test
cargo test --manifest-path src-tauri/Cargo.toml --test phase1_smoke_test
```

### Smoke Test Verification Criteria

`tests/phase1_smoke_test.rs` validates the complete desktop lifecycle across 10 deterministic steps:
1. Starts from a clean fixture git repository.
2. Registers the repository in Coalition.
3. Verifies `.coalition/` layout validation passes and `project.yaml` is created.
4. Inspects live Git status (clean repo fixture).
5. Modifies a file in the repository and verifies live Git status updates to dirty without polling loops.
6. Initiates transition `DRAFT -> ARCHITECTING` and verifies operational state, activity event, and SQLite persistence.
7. Closes and reopens the application (simulating app restart against SQLite store), verifying project list recovery, last-opened tracking, and live Git state.
8. Simulates repository path unavailable on disk, verifying project list marks it unavailable without deletion, and detail view does not report a fake Draft contract.
9. Deletes the SQLite database file entirely, restarts, and reopens the repository path, verifying operational state successfully rehydrates from `.coalition/project.yaml` with an activity log entry `PROJECT_REHYDRATED`.
10. Tests safe `project.yaml` replacement by updating `project.yaml` to frozen architecture state with a version and verifying both validation and rehydration preserve frozen state.

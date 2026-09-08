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
├── src/                       # Frontend source (React + TS + Vite)
├── src-tauri/                 # Rust core backend & Tauri configuration
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── src/
│   │   ├── commands/          # Tauri IPC command handlers
│   │   ├── core/              # Adapters (Git, Antigravity, Process)
│   │   └── db/                # SQLite connection and migrations
└── tests/
    └── fake-commands/         # Zero-quota fake-agy test double
```

## Running Verification Locally

```powershell
# Typecheck
npm run typecheck

# Frontend tests
npm test -- --run

# Frontend build
npm run build

# Rust formatting
cargo fmt --manifest-path src-tauri/Cargo.toml --check

# Rust Clippy
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings

# Rust unit tests
cargo test --manifest-path src-tauri/Cargo.toml
```

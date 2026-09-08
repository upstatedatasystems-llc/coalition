# Contributing to Coalition

Thank you for your interest in Coalition.

## Project Principles & Rules

1. **Follow the Master Implementation Plan**: Architecture and implementation follow the staged roadmap in sequential phases. Do not implement later-phase abstractions prematurely.
2. **Strict Invariants**: All contributors and AI agents must follow the non-negotiable invariants codified in `AGENTS.md`.
3. **No Unapproved Architecture Changes**: Only human project owners may authorize architectural changes. If a technical obstacle arises, document it as an Architecture Concern.
4. **Deterministic Evidence**: All pull requests and changes must include automated tests and pass the full verification suite without skipping or suppressing checks.
5. **No Private Secrets**: Never commit API keys, personal credentials, local absolute user paths, or private diagnostics.
6. **No AI Quota Consumption in CI**: Standard automated tests must run against test doubles (such as `fake-agy`) rather than real billable or quota-limited AI endpoints.

## Verification Checklist

Before submitting changes, run:
```powershell
npm run typecheck
npm test -- --run
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

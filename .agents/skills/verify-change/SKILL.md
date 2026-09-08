---
name: verify-change
description: Guide for running deterministic checks and reporting truthful results in Coalition.
---

# Verify Change

Use this skill whenever verifying code changes, running test suites, or confirming phase exit criteria.

## Principles
1. **Execute Actual Checks**: Always run the concrete automated commands (e.g., `npm run typecheck`, `npm test -- --run`, `npm run build`, `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`).
2. **Truthful Reporting**: Report exact commands executed, exit codes, and stdout/stderr snippets.
3. **Never Fake Results**: Never claim unexecuted checks passed. Never suppress or hide failures to present an artificial green status.
4. **No AI Quota in CI**: Standard automated tests must use test doubles (such as `fake-agy`) and never consume real AI subscription quota.

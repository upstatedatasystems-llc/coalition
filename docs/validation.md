# Validation Subsystem Design

## Purpose

The Validation subsystem executes deterministic checks (builds, tests, linters, type checks) to generate evidence for:
- The Builder (Antigravity) as diagnostic feedback.
- The Reviewer (ChatGPT) as proof of correctness.
- The Human as transparency and gating controls.

## Optional & Configurable

Validation is never mandatory across all projects. A project's `.coalition/implementation/validation.yaml` declares whether validation is enabled, whether checks are sequential, and whether failed required checks gate review transitions.

## Phase 0 Scope

Phase 0 provides the foundational process-runner infrastructure:
- Spawning child processes with explicit working directories.
- Streaming stdout and stderr in real-time.
- Enforcing bounded execution timeouts.
- Clean cancellation and child process tree cleanup.
- Self-validation configuration for Coalition development.

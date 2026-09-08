# Permissions and Icarus Autonomy

## Core Philosophy

Coalition enforces human authority over tool actions and machine side-effects.

Routine, safe operations within the project repository should proceed automatically, while high-risk, destructive, or out-of-scope operations pause for explicit human approval.

## Decision Categories (Target Model for Phase 5)

- **Deny**: Disallow the requested action.
- **Allow Once**: Permit the single occurrence; remove approval immediately after execution.
- **Always Allow This**: Remember approval for this specific pattern within this project.
- **Always Deny This**: Automatically reject matching actions in the future.
- **Icarus Mode**: Temporarily auto-approve all Builder actions for the active project session.

## Scoping Rules

- Approvals default to project-local user operational state (SQLite), never machine-global settings and never committed repository artifacts.
- Approvals granted by one developer do not transfer to other developers cloning the repository.

## Icarus Safeguards

- Never enabled by default; requires explicit human activation.
- Displays a persistent, high-visibility visual warning whenever active.
- Can be revoked with a single click at any time.
- Maps to the CLI flag `--dangerously-skip-permissions`.

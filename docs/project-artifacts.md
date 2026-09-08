# Durable Project Artifacts

## Purpose & Scope

Durable project meaning lives inside `.coalition/` within the managed project root.

Unlike SQLite (which tracks transient operational activity and may be deleted or regenerated), `.coalition/` contains the authoritative, human-approved contracts that define what the project *is*.

## Directory Layout (Target Model)

```text
<project-root>/
└── .coalition/
    ├── project.yaml                 # Portable project descriptor & schema version
    ├── design/                      # Human-approved architecture specifications
    │   ├── product-vision.md
    │   ├── requirements.md
    │   ├── architecture.md
    │   ├── constraints.md
    │   ├── interfaces.md
    │   └── security.md
    ├── implementation/              # Implementation guidelines and validation
    │   ├── validation.yaml          # Project validation command declarations
    │   ├── acceptance-criteria.yaml
    │   └── test-plan.md
    ├── decisions/                   # Architectural Decision Records (ADRs)
    │   └── ADR-*.md
    ├── architecture-versions/       # Frozen immutable historical contract snapshots
    │   └── vX.Y/
    ├── changes/                     # Architecture Change Requests (ACRs)
    │   └── ACR-*.md
    ├── reviews/                     # Stored Reviewer verdicts and findings
    │   └── cycle-*/
    └── evidence/                    # Bounded validation summaries and test outputs
        └── validation-*/
```

## Phase 1 Implementation

In Phase 1, Coalition initializes the full standard directory hierarchy and manages `project.yaml` lifecycle.

### `project.yaml` Schema v1

`project.yaml` is the portable root descriptor of the project contract. It must never contain machine-specific absolute filesystem paths.

```yaml
schema_version: 1
project_id: "3e1b7c89-2df4-46b7-a021-995f3b7d1e84"
name: "my-project"
current_architecture_version: null
architecture_state: draft
created_at: "2026-09-08T12:00:00Z"
```

- `schema_version`: Explicit unsigned integer (1). Deserialization and validation strictly reject versions != 1.
- `project_id`: Stable UUID v4 identifier generated on initial registration. Deserialization and validation strictly require RFC 4122 version 4 (Random), rejecting Nil UUIDs, v1, v3, v5, and malformed identifiers. Survives local SQLite deletion.
- `name`: Human-readable project name. Cannot be empty or whitespace-only.
- `current_architecture_version`: Immutable version string (e.g. `"1.0"`) once frozen, or omitted/`null` while in draft. Schema rules strictly mandate:
  - If `architecture_state` is `draft`, `current_architecture_version` must be `None` / omitted (any string, including empty `""` or whitespace `"   "`, is rejected with `INVALID_ARCHITECTURE_STATE`).
  - If `architecture_state` is `frozen`, `current_architecture_version` must be `Some(v)` with a non-empty trimmed version string.
- `architecture_state`: Typed domain state (`draft` | `frozen`).
- `created_at`: ISO-8601 UTC RFC3339 timestamp validated via `chrono::DateTime::parse_from_rfc3339`.

### Hierarchy & Layout Validation

All standard `.coalition/` subdirectories (`design`, `implementation`, `decisions`, `architecture-versions`, `changes`, `reviews`, `evidence`) and `project.yaml` are validated via `ArtifactManager::validate_coalition_layout`:
- Ensures each subdirectory exists or is created safely.
- Resolves each path and verifies it remains strictly inside the canonical repository root.
- Rejects path traversal escapes (`..`) with `ArtifactError::PathTraversal`.
- Rejects Windows directory junctions, symlinks, or reparse points that point outside the repository with `ArtifactError::UnsafeReparsePoint`.

### Crash-Safe Platform-Native Atomic Replacement

Updating `project.yaml` via `ArtifactManager::write_project_yaml_atomic` provides genuine crash-safe replacement without ever leaving the system in a window where no canonical file exists:
1. Validates the `ProjectYaml` descriptor in memory against all schema v1 invariants before touching the disk.
2. Writes serialized YAML to a unique temporary file (`project.yaml.tmp.<uuid>`) in the `.coalition/` folder.
3. Flushes and syncs the file descriptor (`sync_all()`) to ensure physical disk commitment.
4. Atomically replaces destination using native platform APIs:
   - **Windows**: `ReplaceFileW` with `REPLACEFILE_WRITE_THROUGH` (falling back to `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH` if destination does not exist). The existing file is never renamed to a backup on normal writes, eliminating any crash window between an unlinking and renaming step.
   - **POSIX**: `std::fs::rename` plus directory `sync_all()`.
5. If replacement fails, the temporary file is deleted and the existing `project.yaml` is left completely intact.

### Stale Backup & Temp File Recovery Policy

If an unexpected crash or interrupted operation occurred, `ArtifactManager::inspect_or_recover_project` resolves state with strict precedence:
1. **Canonical `project.yaml` exists and is valid**: Project truth is intact. Any stale `project.yaml.tmp.*` files are safely pruned.
2. **Canonical `project.yaml` is missing**:
   - The `.coalition` directory is scanned for legacy or recovery backups (`project.yaml.bak.*`).
   - If **exactly 1** valid backup exists: It is safely and atomically promoted to `project.yaml` via `replace_file_atomically`, stale temp files are cleaned, and the project is loaded.
   - If **multiple** backups exist, or if a single backup is corrupted/invalid: The system refuses to guess, avoids arbitrary selection, and returns structured error `ARTIFACT_RECOVERY_REQUIRED` (`ArtifactError::RecoveryRequired`).
   - If no backups exist: Returns `None`.

### Operational-First Registration Ordering & Missing Contract Semantics

When opening or registering a project repository:
1. SQLite is queried by canonical path *first*, before any durable files are created or identity is generated.
2. If the canonical path is already registered in SQLite, but `project.yaml` is missing on disk (Case D):
   - The operation fails immediately with structured error `DURABLE_CONTRACT_MISSING` (`ProjectError::DurableContractMissing`).
   - **Zero durable identity mutation occurs on disk** (no new `project.yaml` is created).
3. If the repository is completely new to both SQLite and disk (Case A), a new UUID v4 `project.yaml` is initialized.
4. When a project's repository is offline or deleted from disk:
   - Coalition returns `artifact: None` in `ProjectDetails`.
   - The system never fabricates a synthetic or assumed "Draft" contract.
   - The UI visibly alerts the user that durable architecture state exists only in `.coalition/project.yaml` and is currently offline.

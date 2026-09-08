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

> Coalition flushes replacement data before invoking the native replacement primitive. On Windows it uses `ReplaceFileW` with a same-directory backup, reconciles documented failure states, and never authorizes recovery candidate promotion until project identity has been validated.

Updating `project.yaml` via `ArtifactManager::write_project_yaml_atomic` guarantees durability and handles all documented failure states:
1. Validates the `ProjectYaml` descriptor in memory against all schema v1 invariants before touching the disk.
2. Writes serialized YAML to a unique temporary file (`project.yaml.tmp.<uuid>`) in the `.coalition/` folder.
3. Flushes and syncs the file descriptor (`sync_all()`) to ensure physical disk commitment before replacement.
4. Atomically replaces destination using native platform APIs:
   - **Windows**: `ReplaceFileW` with `dwReplaceFlags = 0` (Microsoft documents `REPLACEFILE_WRITE_THROUGH` as unsupported for `ReplaceFileW`) and an explicit same-directory backup path (`project.yaml.bak.<uuid>`). If destination was genuinely absent at entry, uses `MoveFileExW` creation path. If destination existed at entry, failure never falls through into file creation.
   - **POSIX**: `std::fs::rename` plus directory `sync_all()`.
5. **Post-Call Reconciliation & Recovery Artifact Preservation**:
   - **On Success**: Validates and reads canonical destination `project.yaml`. Only after successful validation is the generated backup safely deleted.
   - **On Failure**: Explicitly inspects destination, temporary file, and generated backup. If destination was unlinked/renamed to backup by the OS prior to failure, destination is restored from backup; if destination is intact and valid, the temp file is removed; if state is ambiguous or returns `RecoveryRequired`, all recognized temp, backup, and canonical artifacts are preserved intact for recovery inspection.

### Non-Mutating Artifact Inspection & Identity-Before-Promotion Policy

1. **Non-Mutating Inspection**:
   - `ArtifactManager::inspect_project_artifacts` inspects `.coalition/` and reports `ProjectArtifactInspection` (`canonical`, `valid_backups`, `temp_files_present`, `ambiguous_or_invalid_recovery_state`) without mutating, promoting, or removing any files.
2. **Identity-Before-Promotion Reconciliation**:
   - `ProjectService::register_or_open_project` queries SQLite by canonical path *before* evaluating backups.
   - **Matching Candidate**: If path is known and single valid backup shares the registered `project_id`, promotion to `project.yaml` is authorized and executed.
   - **Conflicting Candidate**: If path is known but candidate backup has a different `project_id`, returns `PROJECT_IDENTITY_CONFLICT` with zero promotion (filesystem remains completely untouched).
   - **Moved Repository / Rehydration**: If path is unknown, candidate ID is checked against SQLite to detect checkout collisions or moved paths before promotion.
   - **Ambiguous State**: If multiple backups or corrupted backups exist, returns `ARTIFACT_RECOVERY_REQUIRED` with zero mutation.
   - **Incomplete Writes (Temp-only)**: If canonical is missing and only temp files exist, returns `ARTIFACT_RECOVERY_REQUIRED` with zero mutation.
   - **Case D (Missing Contract)**: If SQLite path is known, but no canonical contract or valid backup exists, returns `DURABLE_CONTRACT_MISSING` with zero durable identity mutation.
3. **Complete Layout Validation on Every Project Open**:
   - Every successful open validates the complete `.coalition/` hierarchy, ensuring that all standard subdirectories (`design/`, `implementation/`, `decisions/`, `architecture-versions/`, `changes/`, `reviews/`, `evidence/`) exist and resolve safely inside the repository root.
   - Missing standard directories are recreated safely.
   - Symlink or junction escapes outside the repository root are strictly rejected before Coalition writes through that path.
4. When a project's repository is offline or deleted from disk:
   - Coalition returns `artifact: None` in `ProjectDetails`.
   - The system never fabricates a synthetic or assumed "Draft" contract.
   - The UI visibly alerts the user that durable architecture state exists only in `.coalition/project.yaml` and is currently offline.

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
- `project_id`: Stable UUID v4 identifier generated on initial registration. Deserialization validates proper UUID syntax. Survives local SQLite deletion.
- `name`: Human-readable project name. Cannot be empty or whitespace-only.
- `current_architecture_version`: Immutable version string (e.g. `"1.0"`) once frozen, or `null` while in draft. Schema rules strictly mandate:
  - If `architecture_state` is `draft`, `current_architecture_version` must be `None` / `null`.
  - If `architecture_state` is `frozen`, `current_architecture_version` must be `Some(v)` with a non-empty trimmed version string.
- `architecture_state`: Typed domain state (`draft` | `frozen`).
- `created_at`: ISO-8601 UTC RFC3339 timestamp validated via `chrono::DateTime::parse_from_rfc3339`.

### Hierarchy & Layout Validation

All standard `.coalition/` subdirectories (`design`, `implementation`, `decisions`, `architecture-versions`, `changes`, `reviews`, `evidence`) and `project.yaml` are validated via `ArtifactManager::validate_coalition_layout`:
- Ensures each subdirectory exists or is created safely.
- Resolves each path and verifies it remains strictly inside the canonical repository root.
- Rejects path traversal escapes (`..`) with `ArtifactError::PathTraversal`.
- Rejects Windows directory junctions, symlinks, or reparse points that point outside the repository with `ArtifactError::UnsafeReparsePoint`.

### Windows-Safe Atomic Replacement Algorithm

Updating `project.yaml` via `ArtifactManager::write_project_yaml_atomic` guarantees durable contract preservation even across crashes or filesystem errors:
1. Validates the `ProjectYaml` descriptor in memory against all schema v1 invariants before touching the disk.
2. Writes the serialized YAML to a unique temporary file (`project.yaml.tmp.<uuid>`) in the `.coalition/` folder.
3. Flushes and syncs the file descriptor (`sync_all()`) to ensure physical disk commitment.
4. If a target `project.yaml` already exists:
   - Renames `project.yaml` to `project.yaml.bak.<uuid>`.
   - Renames `project.yaml.tmp.<uuid>` to `project.yaml`.
   - If renaming the replacement into place fails, immediately restores the original file from `.bak`.
   - Cleans up `.bak` upon confirmed replacement.
5. If no target file existed initially, renames the temp file directly into place.
6. The valid original file is never deleted prior to committing the replacement.

### Unavailable Repository Contract Semantics

When a project's repository is moved or unavailable on disk:
- Coalition returns `artifact: None` in `ProjectDetails`.
- The system never fabricates a synthetic or assumed "Draft" contract.
- The UI visibly alerts the user that durable architecture state exists only in `.coalition/project.yaml` and is currently offline.

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

- `schema_version`: Explicit unsigned integer (1). Deserialization strictly rejects versions > 1.
- `project_id`: Stable UUID v4 identifier generated on initial registration. Survives local SQLite deletion.
- `name`: Human-readable project name, defaults to repository folder name.
- `current_architecture_version`: Immutable version string (e.g. `"1.0"`) once frozen, or `null` while in draft.
- `architecture_state`: Typed domain state (`draft` | `frozen`).
- `created_at`: ISO-8601 UTC RFC3339 timestamp.

### Filesystem Safety & Atomic Writes

All `.coalition/` operations obey strict filesystem safety constraints:
1. **Repository Root Canonicalization**: Every governed path is resolved through Git (`git rev-parse --show-toplevel`) and canonicalized.
2. **Path Traversal Escape Prevention**: Any path resolving or escaping outside the canonical root via `..` is rejected.
3. **Symlink and Reparse Point Safety**: Windows directory junctions, symlinks, or reparse points that redirect `.coalition` writes outside the repository root are strictly rejected with typed `ArtifactError::UnsafeReparsePoint`.
4. **Atomic Updates**: Metadata writes use a temporary-file write, flush, and atomic replace sequence (`project.yaml.tmp.<uuid> -> project.yaml`) suitable for Windows and cross-platform filesystems to prevent partial corruption.

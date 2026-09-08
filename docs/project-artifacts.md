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

## Phase 0 Boundary

In Phase 0, only `.coalition/implementation/validation.yaml` is utilized for Coalition's own self-validation commands. Full project artifact lifecycle management is scheduled for Phase 1.

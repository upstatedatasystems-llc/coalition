# Coalition — Project Draft

**Status:** Early Product / Architecture Draft
**Working Name:** Coalition
**Version:** 0.1
**Purpose:** Define the initial product vision, principles, workflow, scope, and technical direction for Coalition before implementation planning begins.

---

# 1. Executive Summary

Coalition is a local-first AI software development environment designed to turn a human idea into a deliberately designed, implemented, tested, and independently reviewed software product.

The fundamental premise is that no single AI model should be trusted to define, implement, and approve the same software system.

Instead, Coalition assigns distinct responsibilities to different AI roles:

* **Architect** — collaborates with the human to define what should be built.
* **Builder** — implements the frozen architecture.
* **Reviewer** — independently evaluates whether the implementation actually conforms to that architecture.
* **Arbiter** — optionally resolves substantive disagreement between Builder and Reviewer.

These roles may be powered by different models from different model families.

The authority hierarchy is:

**Human → Architecture Contract → Deterministic Evidence → Coalition Agents**

No AI agent may autonomously redefine an approved architecture simply because implementation becomes difficult.

At the same time, software development is inherently iterative. A human may discover during development that the approved architecture itself needs to change.

Coalition therefore includes an explicit **human-only Architecture Change cycle** that pauses implementation, reopens design, creates a new architecture version, and then resumes development against the revised contract.

Coalition V1 will use **OpenRouter exclusively** for model access, providing one-key access to a wide catalog of models while preserving an internal provider abstraction that allows direct providers, AWS Bedrock, Azure/Microsoft Foundry, Vertex AI, local models, and other model gateways to be added later.

Coalition V1 will be a **local-first browser application backed by a local Coalition engine**, with a minimal CLI exposing the same engine.

Its goal is not to become another AI autocomplete tool or coding chatbot.

Its goal is to provide a governed environment in which multiple AI systems cooperate—and challenge one another—until they demonstrably converge on the product a human actually defined.

---

# 2. Core Product Thesis

Existing AI coding tools are increasingly capable of autonomously writing software.

The more important unsolved problem is:

> How do we know that the software an AI created is actually the software we intended to create?

Coalition addresses this by separating:

**Intent**

from

**Implementation**

from

**Verification**

The desired lifecycle is:

```text
Human Idea
    │
    ▼
Architectural Exploration
    │
    ▼
Architecture Contract
    │
    ▼
Human Freezes Architecture
    │
    ▼
Builder
    │
    ▼
Tests / Build / Static Validation
    │
    ▼
Independent Reviewer
    │
    ├── Corrections Required ──────┐
    │                              │
    │                              ▼
    │                           Builder
    │                              │
    └──────────────────────────────┘
    │
    ▼
Final Architectural Validation
    │
    ▼
Ready for Human Review
```

At any point during implementation, the human may invoke:

```text
OH SHIT
Architecture needs to change.
```

This enters a controlled Architecture Change cycle rather than allowing implementation to drift away from the original design.

---

# 3. Core Design Principles

## 3.1 Human Authority

The human owns the product.

AI agents assist with reasoning, implementation, and verification but do not possess authority to redefine the intended product.

Only a human may:

* freeze an architecture;
* approve a substantive architecture change;
* abandon an architecture;
* accept the final product;
* authorize a new architecture version.

---

## 3.2 Architecture Is the Contract

Once frozen, the architecture becomes the authoritative definition of the product.

The Builder does not decide whether a deviation is acceptable.

The Reviewer does not redefine requirements to make an implementation acceptable.

If implementation reveals that the architecture itself is wrong, the architecture must be explicitly changed.

---

## 3.3 Deliberate Separation of Roles

Coalition should avoid having the same model simultaneously serve as designer, implementer, and ultimate reviewer whenever practical.

Model diversity is desirable because different model families may possess different strengths, biases, and failure modes.

Example:

```text
Architect
Claude-family model

Builder
Gemini-family model

Reviewer
GPT-family model

Arbiter
DeepSeek-family model
```

The exact models are configurable.

Roles are permanent concepts.

Models are replaceable resources.

---

## 3.4 Simplicity

Simplicity is a governing product principle.

Coalition should prefer:

* explicit workflows over hidden automation;
* a few understandable states over dozens of agent modes;
* readable Markdown/YAML artifacts over proprietary formats;
* Git over custom source-control mechanisms;
* local execution over unnecessary cloud infrastructure;
* one OpenRouter credential over many provider accounts;
* observable actions over mysterious autonomous behavior.

Complexity must justify itself.

Coalition should not become complicated merely because multi-agent systems can be complicated.

---

## 3.5 Evidence Over Agent Confidence

An agent saying something is complete is not evidence of completion.

Coalition should prioritize:

1. Build success.
2. Compiler/type-checking results.
3. Automated tests.
4. Integration tests.
5. Static/security analysis where applicable.
6. Explicit acceptance criteria.
7. Independent architectural review.
8. Human review.

---

## 3.6 Model Neutrality

Coalition must not structurally depend on a specific model vendor.

V1 uses OpenRouter exclusively as the access layer, but internal abstractions must treat models independently from their providers.

Future model-access methods may include:

* direct OpenAI;
* direct Anthropic;
* direct Google;
* xAI;
* DeepSeek;
* Mistral;
* AWS Bedrock;
* Microsoft Foundry;
* Google Vertex;
* local inference;
* other OpenAI-compatible endpoints.

These are future integrations, not V1 scope.

---

## 3.7 Local First

Coalition V1 runs on the user's computer.

The source repository remains local.

Coalition does not require:

* a Coalition account;
* Coalition-hosted source-code storage;
* a cloud database;
* organization administration;
* subscription infrastructure;
* remote development environments.

Model context is necessarily sent to OpenRouter/model providers when required, but Coalition itself does not need a hosted backend.

---

# 4. Primary User Experience

Coalition should feel like an application for building software, not like a configuration interface for AI models.

The user's first question should not be:

> Which model do you want?

It should be:

> What do you want to build?

---

# 5. First-Run Experience

V1 setup should be intentionally short.

## Step 1 — Welcome

Explain briefly:

> Coalition uses independent AI models to design, build, and review software against a shared architectural goal.

## Step 2 — OpenRouter

Request:

* OpenRouter API key.

Coalition validates the key and obtains available models.

The key should be stored using appropriate OS credential storage rather than plaintext project files.

## Step 3 — Local Development Environment

Coalition detects relevant local tools where possible:

* Git;
* Python;
* Node.js;
* Docker;
* other common development runtimes.

This should be informative rather than overwhelming.

Setup completes.

Additional settings may be changed later.

---

# 6. Project Creation

The user selects:

**Create Project**

The first screen asks:

> What do you want to build?

The user can proceed in two primary ways.

---

# 7. Project Entry Mode A — Start From an Idea

Example:

> I want to build a local tool that monitors Palo Alto SCM configuration changes and performs pre/post maintenance validation.

Coalition starts an Architect conversation.

The Architect helps progressively define:

* product purpose;
* user needs;
* functional requirements;
* non-functional requirements;
* constraints;
* architecture;
* major components;
* data model;
* interfaces;
* security considerations;
* implementation strategy;
* testing strategy;
* acceptance criteria.

The conversation is exploratory.

The resulting artifacts are structured.

---

# 8. Project Entry Mode B — Import Existing Work

A user may already have extensively developed the idea elsewhere.

Examples include:

* a ChatGPT conversation;
* Markdown planning documents;
* requirements documents;
* architecture documents;
* meeting notes;
* an existing specification;
* exported AI conversations;
* a collection of project notes;
* an existing repository;
* combinations of these.

Coalition should support an **Import Project Context** flow.

The user may paste or add relevant material.

Coalition then analyzes the supplied material and constructs a proposed project package.

It should identify:

* established decisions;
* requirements;
* unresolved questions;
* contradictions;
* assumptions;
* architecture;
* acceptance criteria;
* implementation phases.

The Architect then says, conceptually:

> Here is my understanding of the project based on what you provided. These decisions appear settled. These items remain unclear.

The user can refine the result before anything is frozen.

This allows a project such as Coalition itself—already extensively brainstormed in ChatGPT—to begin without repeating the entire discovery process.

Import does not bypass the Architect phase.

It **bootstraps** it.

---

# 9. Architect Workspace

The Architect workspace should combine conversation with visible structured artifacts.

Conceptual layout:

```text
┌─────────────────────────────┬─────────────────────────┐
│                             │ PROJECT DESIGN          │
│ ARCHITECT CONVERSATION      │                         │
│                             │ Product Vision       ✓  │
│ Human                       │ Requirements         ✓  │
│ Architect                   │ Architecture         ◐  │
│                             │ Data Model           ○  │
│                             │ Interfaces           ○  │
│                             │ Security             ○  │
│                             │ Test Plan            ○  │
│                             │ Acceptance Criteria  ○  │
│                             │                         │
│ [ message ]                 │ [ View Artifacts ]     │
└─────────────────────────────┴─────────────────────────┘
```

Artifacts evolve as the discussion progresses.

The user should be able to:

* inspect artifacts;
* edit them;
* ask the Architect to revise them;
* compare versions;
* revisit decisions.

---

# 10. Project Artifact Package

A Coalition project should produce human-readable artifacts stored with the project.

Proposed structure:

```text
.coalition/
│
├── project.yaml
│
├── design/
│   ├── product-vision.md
│   ├── requirements.md
│   ├── architecture.md
│   ├── data-model.md
│   ├── interfaces.md
│   ├── security.md
│   └── constraints.md
│
├── implementation/
│   ├── implementation-plan.md
│   ├── milestones.yaml
│   ├── acceptance-criteria.yaml
│   └── test-plan.md
│
├── decisions/
│   ├── ADR-001.md
│   ├── ADR-002.md
│   └── ...
│
├── reviews/
│   └── ...
│
└── state/
    └── ...
```

The exact structure may change during detailed design.

The principles should remain:

* readable without Coalition;
* versionable in Git;
* portable;
* non-proprietary;
* suitable for AI consumption;
* suitable for human inspection.

---

# 11. Architecture Freeze

When the Architect believes the project is sufficiently defined, Coalition presents:

```text
ARCHITECTURE READY

✓ Product Vision
✓ Requirements
✓ Architecture
✓ Interfaces
✓ Security / Constraints
✓ Implementation Plan
✓ Test Plan
✓ Acceptance Criteria

[ Continue Designing ]

[ Freeze Architecture & Build ]
```

Only the human may select:

**Freeze Architecture & Build**

This creates:

```text
Architecture v1.0 — FROZEN
```

The frozen architecture becomes the implementation contract.

---

# 12. Frozen Architecture Rules

After architecture freeze:

The Architect cannot silently alter the contract.

The Builder cannot alter the contract.

The Reviewer cannot alter the contract.

The Arbiter cannot alter the contract.

Agents may:

* identify a problem;
* identify ambiguity;
* recommend an architecture change;
* explain why a requirement may be impossible or undesirable.

They may not execute that change themselves.

Only a human may enter Architecture Change mode.

---

# 13. Human-Only Architecture Change Cycle

This is a foundational Coalition capability.

During implementation the human may realize:

> The original design made sense, but after seeing/testing the product I want something different.

Coalition must make this easy and safe.

The interface should provide an always-available action such as:

**Change Architecture**

Internally this may be referred to informally as the **“Oh Shit” button**, but production wording should remain professional.

Invoking it immediately pauses autonomous development.

State:

```text
BUILDING
   │
   │ Human selects Change Architecture
   ▼
PAUSED — ARCHITECTURE CHANGE
```

Coalition preserves:

* current repository state;
* current commit;
* outstanding review findings;
* test results;
* cost history;
* Builder state;
* current architecture version.

The human then returns to an Architect conversation.

---

# 14. Architecture Change Workflow

Example:

```text
Architecture v1.0
       │
       ▼
Implementation
       │
       ▼
Human discovers design problem
       │
       ▼
CHANGE ARCHITECTURE
       │
       ▼
Implementation PAUSED
       │
       ▼
Architect + Human
       │
       ▼
Proposed Architecture v1.1
       │
       ▼
Impact Analysis
       │
       ▼
Human Approval
       │
       ▼
Architecture v1.1 FROZEN
       │
       ▼
Revised Implementation Plan
       │
       ▼
Builder resumes
```

---

# 15. Architecture Change Request

Coalition should create a durable record of architectural changes.

Example:

```text
ACR-003

Architecture:
v1.0 → v1.1

Initiated by:
Human

Reason:

After testing the application, the user determined
that real-time updates are necessary rather than
30-second polling.

Affected areas:

- backend event model;
- frontend state synchronization;
- API design;
- test strategy.

Superseded decisions:

ADR-005

New decisions:

ADR-011
ADR-012
```

This gives the project architectural provenance rather than allowing the implementation history to become confusing.

---

# 16. Architecture Impact Analysis

Before freezing the revised architecture, the Architect should examine the existing implementation and explain the consequences.

For example:

```text
Architecture Change Impact

Existing work reusable             ~70%

Components requiring modification
- API layer
- state synchronization
- frontend event handling

Components unaffected
- authentication
- persistence layer
- settings

Tests requiring revision
- 14

New implementation milestones
- 2

Estimated additional AI cost
$2.50–$5.00
```

The human then decides whether to proceed.

---

# 17. Builder

After architecture freeze, Coalition assigns the Builder.

The **Builder** is a role.

The underlying model is replaceable.

The Builder receives:

* frozen architecture;
* acceptance criteria;
* implementation plan;
* repository context;
* relevant decisions;
* test requirements;
* existing code.

Its responsibility is:

> Implement the defined product.

Not:

> Decide what product should exist.

---

# 18. Coalition-Native Builder Runtime

Long-term, Coalition should provide its own execution environment rather than fundamentally depending on Antigravity, Claude Code, or another coding product.

The Builder model should interact with standardized tools such as:

```text
read_file
search_code
list_files
write_file
patch_file
run_command
run_tests
git_diff
git_status
git_commit
inspect_logs
report_progress
report_completion
```

This keeps:

**Model**

separate from:

**Execution runtime**

Future external Builder adapters may still be supported.

Examples:

* Antigravity;
* Claude Code;
* OpenHands;
* other coding agents.

But Coalition Native should ultimately be sufficient on its own.

---

# 19. Builder Milestones

The implementation plan should be divided into logical milestones.

Example:

```text
1. Project Foundation
2. Core Domain Model
3. API
4. Frontend
5. Integration
6. Final Validation
```

The Builder should preferably work in bounded units rather than attempting an entire major application as one giant autonomous action.

This improves:

* observability;
* error recovery;
* review quality;
* cost control;
* Git history.

---

# 20. Git as Source of Truth

Coalition should rely on Git rather than inventing a proprietary source-history system.

A Coalition run may use a dedicated branch:

```text
main
 │
 └── coalition/project-feature
        │
        ├── implementation commit
        ├── reviewer correction
        ├── correction
        └── accepted implementation
```

Coalition should never silently overwrite known-good source state.

Important development events should be recoverable.

---

# 21. Deterministic Validation

Before an AI Reviewer evaluates implementation quality, Coalition should gather objective evidence.

Depending on project type:

* build;
* compiler;
* type checker;
* unit tests;
* integration tests;
* lint;
* formatting validation;
* static analysis;
* security analysis;
* application-specific checks.

Where appropriate, failure of required deterministic validation should prevent architectural acceptance.

---

# 22. Reviewer

The Reviewer is independent from the Builder.

Its job is to determine:

> Does the implementation conform to the frozen architecture and acceptance criteria?

The Reviewer receives evidence including:

* architecture;
* requirements;
* acceptance criteria;
* implementation diff;
* relevant source;
* tests;
* test results;
* Builder completion report;
* previous review findings.

The Reviewer must not trust Builder assertions without evidence.

---

# 23. Structured Review Verdicts

Reviews should return structured findings.

Example:

```yaml
verdict: CORRECTIONS_REQUIRED

findings:

  - id: REV-007
    severity: high
    requirement: ARCH-014
    file: src/api/auth.py

    problem:
      New endpoint bypasses required authorization middleware.

    required_change:
      Route must execute through the standard authorization path.

    required_test:
      Add unauthorized-access regression coverage.
```

Possible review outcomes:

```text
ACCEPT
CORRECTIONS_REQUIRED
BLOCKED
ARCHITECTURE_CONCERN
```

`ARCHITECTURE_CONCERN` does not authorize an architecture change.

It surfaces the issue to the human.

---

# 24. Correction Loop

Normal correction cycle:

```text
Builder
   │
   ▼
CI
   │
   ▼
Reviewer
   │
   ├── ACCEPT
   │
   └── CORRECTIONS_REQUIRED
              │
              ▼
           Builder
              │
              ▼
             CI
              │
              ▼
           Reviewer
```

Correction cycles should generally review the **delta**, not repeatedly send the entire repository.

This reduces cost and noise.

Final acceptance should include a broader validation pass.

---

# 25. Arbiter

The Arbiter should not run routinely.

It exists for substantive disagreement.

Example:

```text
Reviewer:
ARCH-021 is not satisfied.

Builder:
The implementation does satisfy ARCH-021 because...

Reviewer:
The architecture requires different behavior.

             ↓

           Arbiter
```

The Arbiter receives:

* relevant architecture;
* Builder reasoning;
* Reviewer reasoning;
* implementation evidence.

The Arbiter may determine:

```text
BUILDER_CORRECT
REVIEWER_CORRECT
AMBIGUOUS_ARCHITECTURE
```

If architecture is genuinely ambiguous, the project should surface the issue to the human.

The Arbiter cannot alter the architecture.

---

# 26. Coalition Consensus

The product reaches automated completion only when all required gates succeed.

Example:

```text
Acceptance Criteria           PASS
Build                         PASS
Tests                         PASS
Static Analysis               PASS
Primary Reviewer              ACCEPT
Required Secondary Reviews    ACCEPT
Architecture Conformance      PASS
Blocking Findings             0
```

Only then does Coalition enter:

**READY FOR HUMAN REVIEW**

Coalition must never automatically declare the actual product accepted on behalf of the human.

---

# 27. Human Final Review

The completed project view should summarize:

```text
READY FOR HUMAN REVIEW

Architecture                 v1.2

Acceptance Criteria          42 / 42
Tests                        186 / 186
Build                        PASS
Architecture Review          ACCEPT
Blocking Findings            0

Architect                    Model A
Builder                      Model B
Reviewer                     Model C
Arbiter                      Not Required

Review Cycles                3
Architecture Revisions       2
AI Cost                      $6.42
Commits                      11
```

Actions might include:

* Review Changes;
* View Architecture;
* Run Application;
* View Tests;
* View Review History;
* Export Report;
* Approve;
* Change Architecture.

Even at this stage the human may discover something requiring another architecture revision.

The Architecture Change mechanism remains available.

---

# 28. Model Roles

Coalition V1 recognizes four conceptual roles:

## Architect

Optimized for:

* requirements discovery;
* reasoning;
* architecture;
* tradeoff analysis;
* long-term consistency.

## Builder

Optimized for:

* repository interaction;
* implementation;
* debugging;
* testing;
* code modification.

## Reviewer

Optimized for:

* critical analysis;
* code review;
* architectural conformance;
* detecting omissions and regressions.

## Arbiter

Optimized for:

* resolving disagreements;
* interpreting requirements;
* comparing competing technical claims.

The same underlying model may technically fill multiple roles, but Coalition should favor role independence where practical.

---

# 29. OpenRouter-Only V1

V1 uses OpenRouter exclusively.

Benefits:

* one API key;
* one billing relationship;
* wide model selection;
* unified model access;
* dynamic model catalog;
* simplified onboarding;
* easier experimentation.

Coalition should still contain an internal provider abstraction.

Conceptually:

```text
ModelGateway

    V1
     └── OpenRouter

    Future
     ├── OpenAI Direct
     ├── Anthropic Direct
     ├── Google Direct
     ├── AWS Bedrock
     ├── Microsoft Foundry
     ├── Vertex AI
     ├── Local
     └── Custom
```

Nothing above the gateway layer should depend on OpenRouter-specific assumptions unnecessarily.

---

# 30. Model Catalog

Coalition should dynamically discover available OpenRouter models.

For each model, Coalition maintains metadata such as:

* provider/model;
* context length;
* tool support;
* structured-output support;
* reasoning capabilities;
* pricing;
* availability;
* known limitations.

This catalog should not require an application update every time a new model appears.

---

# 31. Published Model Performance Data

Coalition should supplement the raw model catalog with publicly available performance evidence.

Potential categories include:

* repository-level coding;
* terminal-agent performance;
* code editing;
* reasoning;
* long-context performance;
* software engineering;
* review performance where available.

Potential sources may include benchmarks such as:

* SWE-bench;
* Terminal-Bench;
* Aider Polyglot;
* LiveCodeBench;
* other credible benchmark suites.

Coalition must preserve provenance.

---

# 32. Benchmark Provenance

Every external statistic should track:

```text
Model
Model version
Benchmark
Benchmark version
Score
Metric
Date
Agent harness
Tool configuration
Source
Source type
Verification status
```

Source classifications might include:

```text
INDEPENDENT
BENCHMARK MAINTAINER
MODEL VENDOR
COALITION
```

Vendor-published results must not be presented as equivalent to independently reproduced results.

---

# 33. Avoid a Misleading Universal Intelligence Score

Coalition should not collapse model capability into a single opaque score.

Instead it may produce role-specific suitability ratings.

For example:

```text
Model X

Architect Fit       92
Builder Fit         88
Reviewer Fit        94
```

Selecting a score should reveal the evidence behind it.

---

# 34. Coalition-Generated Performance Data

Coalition records its own operational statistics.

Examples:

* projects attempted;
* projects completed;
* role used;
* language/framework;
* first-pass review acceptance;
* number of correction cycles;
* regression findings;
* architecture violations found;
* build success;
* cost;
* token use;
* completion time;
* human architecture-change frequency.

Metrics should be collected separately by role.

A model excellent at coding may not be an excellent Architect.

---

# 35. User-Specific Model Performance

Coalition should eventually be able to say:

```text
Published evidence suggests Model A is strongest.

On your projects, Model B has performed better.
```

User-specific history may eventually become more useful than public benchmarks.

V1 may collect the underlying data even if sophisticated recommendation logic comes later.

---

# 36. Model Strategy

Users should not have to manually select every model unless they want to.

Possible strategies:

```text
Recommended
Maximum Quality
Balanced
Economy
Maximum Diversity
Custom
```

V1 may begin with:

* Recommended;
* Custom.

Other strategies can be added later.

---

# 37. Recommended Coalition

Before implementation begins, Coalition may propose:

```text
Architect
Model A

Builder
Model B

Reviewer
Model C

Arbiter
Model D — only if needed

Estimated AI cost
$4.00–$8.00
```

The user may override selections.

---

# 38. Cost Management

Cost visibility should be a first-class feature.

Project view should show:

* current spend;
* spend by model;
* spend by role;
* spend by cycle;
* token usage;
* estimated remaining cost.

Example:

```text
Architecture       $0.52
Builder Cycle 1    $1.21
Review 1           $0.44
Builder Cycle 2    $0.67
Review 2           $0.19

Total              $3.03
```

---

# 39. Cost Controls

Coalition should support:

```text
Soft project budget
Hard project budget
Maximum review cycles
Per-cycle warnings
```

Example:

```yaml
project_budget:
  warning_usd: 10
  hard_stop_usd: 20

review:
  max_cycles: 6
```

If a limit is reached, Coalition pauses and asks the human what to do.

Agents must never autonomously spend without bound.

---

# 40. Prompt/Context Efficiency

Coalition should deliberately minimize unnecessary model context.

Strategies include:

* send relevant files rather than whole repositories;
* review diffs during correction cycles;
* maintain durable architecture artifacts;
* reuse stable prompt prefixes where caching is available;
* perform full-context review only when warranted.

Cost efficiency is an architectural requirement, not an afterthought.

---

# 41. Local Application Architecture

Recommended V1 shape:

```text
Browser UI
    │
    ▼
Local Coalition API
    │
    ▼
Coalition Engine
    │
    ├── Project Manager
    ├── Architect
    ├── Builder
    ├── Reviewer
    ├── Arbiter
    ├── Workflow Engine
    ├── Benchmark Registry
    ├── Model Catalog
    ├── Cost Tracker
    └── Git / Test Runtime
          │
          ├── Local Repository
          └── OpenRouter
```

---

# 42. Proposed V1 Technology Direction

This is preliminary rather than frozen.

## Frontend

React
TypeScript
Vite

## Local API / Application Engine

Python
FastAPI

## Persistent Application State

SQLite

## Project Artifacts

Markdown
YAML
JSON

## Source Control

Git

## Model Gateway

OpenRouter

## CLI

Thin Python CLI invoking the same Coalition engine.

## Secret Storage

OS-specific secure credential storage.

The detailed technical architecture should be evaluated before implementation rather than assumed solely from this draft.

---

# 43. Browser UI + CLI

The browser application is the primary user experience.

The CLI is a secondary control surface.

Both must operate against the same Coalition engine.

Conceptually:

```text
              Coalition Engine
                ▲           ▲
                │           │
               GUI         CLI
```

The CLI must not become a separate implementation of Coalition logic.

Initial CLI might contain only:

```text
coalition
coalition start
coalition stop
coalition status
```

Future commands may include:

```text
coalition build
coalition review
coalition resume
coalition models
coalition cost
coalition run --headless
```

---

# 44. Project Dashboard

Home screen:

```text
COALITION

Projects

[ + New Project ]

Herald
READY FOR HUMAN REVIEW

LiTerra3D
REVIEW CYCLE 2

New Network Tool
ARCHITECTURE DRAFT
```

Users should see project state immediately.

---

# 45. Project State Machine

Initial proposed states:

```text
DRAFT
 │
 ▼
ARCHITECTING
 │
 ▼
READY_TO_FREEZE
 │
 ▼
FROZEN
 │
 ▼
BUILDING
 │
 ▼
VALIDATING
 │
 ▼
REVIEWING
 │
 ├── CORRECTIONS_REQUIRED ──→ BUILDING
 │
 ├── BLOCKED
 │
 ├── ARCHITECTURE_CONCERN
 │
 └── ACCEPTED
          │
          ▼
FINAL_VALIDATION
          │
          ▼
READY_FOR_HUMAN_REVIEW
          │
          ▼
HUMAN_ACCEPTED
```

Human architecture change adds:

```text
ANY DEVELOPMENT STATE
        │
        │ human only
        ▼
ARCHITECTURE_CHANGE
        │
        ▼
ARCHITECTING_REVISION
        │
        ▼
READY_TO_REFREEZE
        │
        ▼
FROZEN vNext
        │
        ▼
BUILDING
```

---

# 46. Pause / Resume / Recovery

Coalition must assume that:

* the app closes;
* a model API fails;
* a machine reboots;
* tests hang;
* a process crashes;
* a user intentionally pauses work.

Project state must therefore be persistent.

On restart:

```text
Coalition found an interrupted project.

LiTerra3D
Review Cycle 2

Last stable state:
Builder completed correction REV-004.

[ Resume ]
[ Inspect ]
```

Autonomous state should never exist only in an LLM conversation context.

---

# 47. Activity Transparency

Users should be able to see what Coalition is doing without being forced to read raw logs.

Default:

```text
Builder
Implementing API validation...

Tests
104 / 104 passing

Files changed
7
```

Advanced:

```text
22:18:04 Reading src/api/routes.py
22:18:10 Reading tests/test_routes.py
22:18:43 Modified routes.py
22:19:02 Running pytest...
22:19:18 18 passed
```

Transparency should be layered.

---

# 48. Security Principles

V1 Builder execution involves substantial local authority.

Coalition should therefore treat execution safety seriously.

Principles:

* project-scoped filesystem access;
* no silent access to unrelated files;
* secrets excluded from model context wherever possible;
* no automatic production deployment;
* no automatic force pushes;
* no automatic destructive Git actions;
* sensitive operations require explicit approval;
* terminal commands are logged;
* local execution is recoverable.

Detailed sandboxing requires dedicated architecture work.

---

# 49. V1 Scope

Coalition V1 should prove one complete workflow well.

## Included

* local application;
* browser UI;
* minimal CLI;
* OpenRouter setup;
* dynamic model discovery;
* project creation;
* existing-context import;
* Architect conversation;
* structured project artifact generation;
* architecture freeze;
* human-only architecture revision;
* Coalition-native Builder;
* local Git integration;
* test/build execution;
* Reviewer loop;
* optional Arbiter;
* cost tracking;
* basic published benchmark catalog;
* internal performance telemetry;
* ready-for-human-review state;
* persistent/recoverable project state.

---

# 50. Explicitly Not V1

To maintain simplicity, V1 should not attempt:

* Coalition cloud accounts;
* hosted source repositories;
* multi-user collaboration;
* organizations;
* permissions/RBAC;
* SaaS billing;
* direct-provider APIs;
* AWS Bedrock;
* Microsoft Foundry;
* Vertex AI;
* mobile apps;
* remote Builder fleets;
* automatic production deployment;
* marketplace/plugin ecosystem;
* complex team management;
* aggregate telemetry across different Coalition users;
* enterprise governance;
* IDE extensions;
* GitHub App integration beyond basic Git/local workflows unless clearly required.

These may be revisited later.

---

# 51. Extensibility Boundaries

Even though V1 remains small, several interfaces should deliberately support future expansion.

## Model Gateway

```text
OpenRouter today
Other gateways tomorrow
```

## Execution Agent

```text
Coalition Native today

Future:
Antigravity
Claude Code
OpenHands
other agents
```

## Validation

```text
Local tests today

Future:
CI systems
security scanners
deployment validation
```

## Repository

```text
Local Git today

Future:
GitHub
GitLab
Bitbucket
enterprise SCM
```

V1 should create the seams without implementing every adapter.

---

# 52. Product Differentiation

Coalition should not position itself merely as:

**A multi-agent coding tool.**

That category already exists.

The stronger positioning is:

> Coalition is an architecture-governed AI development environment where independent AI models design, implement, challenge, and verify software against an explicit human-approved contract.

Key differentiation:

1. Human-guided architecture creation.
2. Explicit architecture freeze.
3. Human-only architecture revisions.
4. Builder/reviewer independence.
5. Cross-model coalition.
6. Deterministic evidence.
7. Machine-verifiable acceptance criteria.
8. Architecture-version provenance.
9. Published model performance data.
10. Coalition-observed model performance.
11. Cost-aware role selection.
12. Local-first operation.
13. Vendor-neutral architecture.
14. Human remains final authority.

---

# 53. Example Complete Journey

A user launches Coalition for the first time.

They enter one OpenRouter API key.

They select:

**New Project**

They paste several pages of an idea previously developed in ChatGPT.

Coalition analyzes it.

The Architect responds:

> I found 24 established requirements, six architectural decisions, and four unresolved issues.

The user and Architect discuss those issues.

Coalition generates:

```text
Product Vision
Requirements
Architecture
Interfaces
ADRs
Test Plan
Acceptance Criteria
Implementation Plan
```

The user selects:

**Freeze Architecture & Build**

Architecture v1.0 becomes immutable.

Coalition recommends:

```text
Architect: Model A
Builder: Model B
Reviewer: Model C
```

The Builder begins implementation.

Tests run continuously.

Reviewer finds three architectural defects.

Builder corrects them.

The user runs the partially completed application and realizes:

> Oh shit. This really needs offline operation.

The user selects:

**Change Architecture**

Coalition immediately pauses development.

The Architect evaluates the new requirement against the existing system.

Architecture v1.1 is proposed.

Coalition shows the implementation impact.

The human approves v1.1.

The Builder receives the new architecture and revised plan.

Implementation continues.

Two review cycles later:

```text
Architecture                 v1.1
Acceptance Criteria          38 / 38
Tests                        214 / 214
Build                        PASS
Reviewer                     ACCEPT
Architecture Conformance     PASS
Blocking Findings            0
Cost                         $7.13
```

Coalition reports:

**READY FOR HUMAN REVIEW**

The human—not an AI—decides whether the product is actually finished.

---

# 54. V1 Success Criteria

V1 succeeds if a user can:

1. Install and launch Coalition locally.
2. Configure one OpenRouter API key.
3. Start from either a raw idea or existing planning material.
4. Collaboratively develop a structured architecture.
5. Freeze that architecture.
6. Select/recommend independent models for key roles.
7. Have a Builder implement the product locally.
8. Automatically run project validation.
9. Have an independent Reviewer find implementation defects.
10. Automatically return actionable findings to the Builder.
11. Repeat until acceptance.
12. Pause development and revise architecture at any time through an explicitly human-triggered workflow.
13. Preserve architecture versions and their rationale.
14. Recover after application interruption.
15. Track model usage and cost.
16. Display meaningful published model statistics.
17. Record local model-performance statistics.
18. End in Ready for Human Review rather than autonomous acceptance.

If Coalition can execute this workflow reliably, it has proven its core thesis.

---

# 55. Questions to Resolve During Detailed Architecture

Before development begins, the following need deeper design work:

### Builder Runtime

* How should local command execution be sandboxed?
* Should V1 use containers, OS process isolation, or project-scoped permissions?
* How should long-running processes work?
* How does Builder inspect running applications?

### Context Management

* How does Coalition determine which source files to send a model?
* How are large repositories indexed?
* How do we avoid excessive token usage?
* What information is retained between model calls?

### Architect

* How structured should the architecture-generation process be?
* Which documents are always required versus project-dependent?
* How should manual artifact edits interact with the Architect?

### Acceptance Criteria

* What schema should machine-verifiable acceptance criteria use?
* How are subjective criteria represented?
* What counts as evidence that a criterion has been met?

### Reviewer

* How much repository context does the Reviewer receive?
* When is a full review required?
* When is delta review sufficient?

### Architecture Change

* How should partially completed work be handled after a revision?
* Should architecture-change impact analysis include automatic Git/code analysis?
* How should superseded acceptance criteria be tracked?

### Model Performance

* Which benchmarks are sufficiently trustworthy?
* How frequently should benchmark data update?
* Should V1 retrieve benchmark data dynamically or ship a curated registry?

### Cost Estimation

* How should Coalition forecast cost before work begins?
* How should estimates improve using observed project history?

### Git

* Does Coalition always create its own working branch?
* What working-tree conditions should prevent a build from starting?
* How should user-created commits during autonomous work be treated?

### Technology

* Confirm Python/FastAPI versus alternatives.
* Confirm browser architecture.
* Determine packaging strategy for Windows/macOS/Linux.

---

# 56. Proposed Development Philosophy

Coalition itself should be built according to the principles it promotes.

Development should proceed in small, explicit phases.

Every major behavior should have:

* a defined requirement;
* a reason for existing;
* clear acceptance criteria;
* tests where feasible;
* no unnecessary abstraction.

Avoid building future enterprise functionality prematurely.

Prefer a narrow but excellent vertical slice over a broad incomplete platform.

The first objective is not:

> Support every model and every development workflow.

It is:

> Make one human → Architect → frozen architecture → Builder → Reviewer → architecture revision → completion workflow work exceptionally well.

---

# 57. Long-Term Possibilities

These are intentionally outside V1 but compatible with the design.

Potential future capabilities include:

* direct provider APIs;
* AWS Bedrock;
* Azure/Microsoft Foundry;
* Vertex AI;
* local models;
* enterprise model gateways;
* external Builder agents;
* GitHub/GitLab integration;
* CI/CD integration;
* remote Builders;
* team collaboration;
* organization policies;
* centralized architecture governance;
* benchmark aggregation;
* anonymous global Coalition performance statistics;
* automatic role selection learned from historical outcomes;
* framework/language-specific model recommendations;
* multi-reviewer panels;
* security-specialist reviewers;
* performance-specialist reviewers;
* UX reviewers;
* architecture templates;
* reusable organizational standards;
* IDE integration;
* fully headless Coalition pipelines.

These should remain possibilities rather than commitments.

---

# 58. Concise Product Definition

**Coalition is a local-first, model-neutral AI software development environment that turns human ideas into explicit architecture contracts, assigns independent AI models to Architect, Builder, Reviewer, and Arbiter roles, and iteratively builds and verifies software until the defined product satisfies its human-approved design.**

Coalition supports deliberate human-only architectural revision whenever real-world development reveals that the original design needs to change.

The human defines success.

The architecture records success.

The Builder attempts it.

The Coalition verifies it.

The human decides when it is finished.

---

# 59. North Star

Coalition should make sophisticated AI-assisted software development feel less like managing several powerful chatbots and more like directing a disciplined engineering team.

A user should be able to begin with:

> I have an idea.

and eventually reach:

> This implementation has been built, tested, independently challenged, compared against the architecture I approved, revised where I chose to revise it, and is now ready for me to evaluate.

Everything Coalition adds should serve that journey.

If a feature does not materially improve that journey, it probably does not belong in V1.

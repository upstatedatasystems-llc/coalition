# Coalition — Master Implementation Plan

**Status:** Implementation-ready master plan  
**Product:** Coalition  
**Plan Version:** 1.1  
**Design Baseline:** Coalition Product & Technical Design Draft v0.2  
**Date:** September 8, 2026  
**Target Repository:** `upstatedatasystems-llc/coalition`  
**Repository Visibility:** Public  
**Primary Implementation Environment:** Windows 10/11  
**Post-V1 Target:** macOS (V1.1)  
**Future Target:** Linux
**Roadmap Model:** Five V1 product stages with bounded internal implementation checkpoints  

---

# 1. Purpose and Authority

This document is the master implementation plan for Coalition V1.

It is intended to be handed directly to Google Antigravity as the authoritative implementation package for the initial public repository.

**Roadmap revision in Plan v1.1:** the original Phase 0–10 implementation sequence has been consolidated into five human-facing V1 product stages. The original technical sequencing is preserved as bounded A/B/C checkpoints inside those stages so Antigravity can continue to work in small, testable, recoverable increments. macOS is no longer counted as part of the Windows V1 completion path; it is a post-V1 V1.1 platform-expansion stage.

The Coalition v0.2 design remains the product and architecture reference. Where this master plan adds detail or changes decisions made after v0.2, **this master plan takes precedence**.

The most important post-v0.2 decisions incorporated here are:

1. Coalition remains a native Tauri desktop application using React/TypeScript and Rust.
2. ChatGPT Plus remains an explicit human-relay Architect and Reviewer.
3. Antigravity remains the external Builder through Google's official CLI.
4. Coalition must expose Antigravity model selection and reasoning effort where supported by the CLI.
5. Coalition must display Antigravity usage/quota/context information where supported.
6. Coalition must provide an approximate ChatGPT usage indicator, clearly labeled as an estimate rather than provider-reported truth.
7. Builder permissions must support:
   - deny;
   - allow once;
   - allow and remember;
   - Icarus mode, which auto-approves everything for the active project/run.
8. Coalition-owned validation is configurable and may be disabled entirely.
9. When enabled, Coalition validation must be independently runnable from Builder activity, visible in real time, stoppable, restartable, and fully logged.
10. Validation results are primarily diagnostic evidence for the Builder, Architect, Reviewer, and human. A project may configure which checks, if any, are required gates.
11. The repository will be public under Upstate Data Systems LLC.
12. Licensing is intentionally not decided by this plan. Do not add a permissive open-source license without explicit human approval.

Do not reinterpret these decisions during implementation merely because an alternative is easier.

If implementation reveals a genuine contradiction or requirement that cannot be fulfilled with supported third-party interfaces, stop that affected work and report an **Architecture Concern** rather than silently changing product behavior.

---

# 2. Product Definition

Coalition is a local-first desktop control plane for governed AI-assisted software development.

It coordinates:

- a human;
- ChatGPT subscription conversations;
- Google Antigravity;
- local Git repositories;
- local project artifacts;
- project-defined validation;
- architecture versioning;
- independent review;
- usage awareness;
- permissions;
- interruption/recovery.

Coalition is **not** a coding agent in V1.

Coalition is **not** an IDE in V1.

Coalition is **not** an AI API gateway in V1.

Coalition is **not** a hosted SaaS product in V1.

Its differentiated responsibility is to ensure that AI coding work remains accountable to an explicit, human-approved architecture and that the human can see, stop, inspect, revise, and resume the process.

The core authority hierarchy is:

```text
Human
  ↓
Frozen Architecture Contract
  ↓
Configured Validation / Evidence
  ↓
Builder + Independent Reviewer
  ↓
Human Final Acceptance
```

Only the human may:

- freeze an architecture;
- re-freeze a revised architecture;
- authorize architecture change;
- decide sensitive Builder permissions;
- enable Icarus mode;
- accept the finished product.

---

# 3. Core V1 User Journey

The V1 implementation must support this complete path:

```text
Install Coalition
      ↓
Open/Create Local Project
      ↓
Prepare Architect packet
      ↓
Human sends packet to ChatGPT
      ↓
Human copies ChatGPT response
      ↓
Coalition imports structured result
      ↓
Architecture artifacts evolve
      ↓
Human freezes Architecture v1.0
      ↓
Coalition starts Antigravity Builder
      ↓
Builder works against frozen contract
      ↓
Coalition shows progress / model / quota / permissions
      ↓
Optional Coalition validation runs
      ↓
Coalition generates Reviewer packet
      ↓
Human sends packet to separate ChatGPT Reviewer
      ↓
Human copies Reviewer result
      ↓
Coalition imports findings
      ↓
Corrections automatically routed to Antigravity
      ↓
Repeat
      ↓
Human may invoke Change Architecture at any time
      ↓
Architecture revised and re-frozen
      ↓
Eventually READY FOR HUMAN REVIEW
      ↓
Human accepts or changes architecture again
```

No critical project state may exist only inside an LLM conversation.

---

# 4. Fixed Technology Stack

## 4.1 Desktop Application

Use:

- Tauri 2.x
- React
- TypeScript
- Vite
- Rust

Do not introduce Electron.

Do not introduce Python/FastAPI for application runtime.

Do not require a localhost server.

Do not require end users to install Python, Node, or Rust to run packaged Coalition binaries.

Development dependencies are allowed.

## 4.2 Persistence

Use:

- repository-local `.coalition/` artifacts for durable project meaning;
- SQLite for local operational/application state.

Principle:

```text
.coalition/
= what the project IS

SQLite
= what Coalition is DOING
```

## 4.3 Source Control

Use the system Git CLI.

Do not build a custom source-control implementation.

## 4.4 Builder

Use the official Antigravity CLI (`agy`).

Preferred Builder transport:

```text
--input-format stream-json
--output-format stream-json
```

Use supported conversation IDs/resume behavior.

## 4.5 ChatGPT

ChatGPT interaction is explicit human relay only.

Coalition may:

- prepare prompts;
- copy user-requested prompts to clipboard;
- open/focus the user's browser/app surface;
- import clipboard contents only after explicit user action;
- parse the copied response.

Coalition must not:

- scrape ChatGPT DOM;
- inspect accessibility output to harvest responses;
- OCR ChatGPT;
- robot-click Copy;
- call undocumented consumer endpoints;
- reuse hidden authentication.

## 4.6 App Identifier

Use this default unless a platform conflict requires adjustment:

```text
com.upstatedatasystems.coalition
```

Product display name:

```text
Coalition
```

---

# 5. Public Repository Baseline

Target repository:

```text
GitHub organization:
upstatedatasystems-llc

repository:
coalition

visibility:
public
```

The repository should be suitable for outside inspection from the first pushed baseline.

Do not commit:

- tokens;
- cookies;
- OAuth material;
- Antigravity credentials;
- personal email addresses in fixtures;
- local filesystem paths that identify a developer;
- test repositories containing private code;
- captured ChatGPT conversations containing private content.

## 5.1 Initial Root Files

Create:

```text
README.md
CONTRIBUTING.md
SECURITY.md
CODE_OF_CONDUCT.md
CHANGELOG.md
AGENTS.md
.gitignore
.editorconfig
package.json
tsconfig.json
vite.config.ts
src/
src-tauri/
docs/
tests/
.github/
```

A `LICENSE` file must **not** be added until the owner explicitly chooses a license.

The README may state:

```text
Copyright © 2026 Upstate Data Systems LLC.
Licensing terms have not yet been selected.
```

Do not describe the repository as "open source" until a license is deliberately selected.

## 5.2 AGENTS.md

Because Antigravity will implement and maintain Coalition, root `AGENTS.md` must state the project's non-negotiable architecture rules, including:

- no ChatGPT scraping;
- no AI API dependency in V1;
- no silent architecture changes;
- `.coalition/` is durable project truth;
- SQLite is operational only;
- Rust owns authoritative state transitions;
- React must not bypass Rust state checks;
- all external commands must go through a common process-management layer;
- all high-risk operations must be permission-aware;
- Icarus must always be visibly indicated;
- no secrets in logs;
- external-tool schemas must be adapter-isolated.

---

# 6. Proposed Repository Structure

Use a structure close to:

```text
coalition/
├── README.md
├── CONTRIBUTING.md
├── SECURITY.md
├── CODE_OF_CONDUCT.md
├── CHANGELOG.md
├── AGENTS.md
├── package.json
├── tsconfig.json
├── vite.config.ts
├── src/
│   ├── app/
│   ├── components/
│   ├── features/
│   │   ├── projects/
│   │   ├── architecture/
│   │   ├── relay/
│   │   ├── builder/
│   │   ├── permissions/
│   │   ├── usage/
│   │   ├── validation/
│   │   ├── reviews/
│   │   ├── activity/
│   │   └── settings/
│   ├── hooks/
│   ├── lib/
│   ├── routes/
│   ├── types/
│   └── test/
├── src-tauri/
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── capabilities/
│   ├── src/
│   │   ├── lib.rs
│   │   ├── commands/
│   │   ├── core/
│   │   │   ├── workflow/
│   │   │   ├── projects/
│   │   │   ├── artifacts/
│   │   │   ├── relay/
│   │   │   ├── builder/
│   │   │   ├── permissions/
│   │   │   ├── usage/
│   │   │   ├── validation/
│   │   │   ├── git/
│   │   │   ├── process/
│   │   │   ├── logging/
│   │   │   └── recovery/
│   │   └── db/
│   └── migrations/
├── docs/
│   ├── architecture.md
│   ├── development.md
│   ├── project-artifacts.md
│   ├── chatgpt-relay.md
│   ├── antigravity-integration.md
│   ├── permissions.md
│   ├── validation.md
│   ├── usage.md
│   └── security-model.md
├── tests/
│   ├── fixtures/
│   ├── fake-commands/
│   └── integration/
└── .github/
    ├── workflows/
    │   └── ci.yml
    └── ISSUE_TEMPLATE/
```

Minor changes are allowed if they improve idiomatic Tauri/Rust/React organization without changing boundaries.

---

# 7. Durable Project Artifact Model

Each governed project contains:

```text
project-root/
└── .coalition/
    ├── project.yaml
    ├── design/
    │   ├── product-vision.md
    │   ├── requirements.md
    │   ├── architecture.md
    │   ├── constraints.md
    │   ├── interfaces.md
    │   └── security.md
    ├── implementation/
    │   ├── implementation-plan.md
    │   ├── acceptance-criteria.yaml
    │   ├── validation.yaml
    │   └── test-plan.md
    ├── decisions/
    │   └── ADR-*.md
    ├── architecture-versions/
    │   └── vX.Y/
    ├── changes/
    │   └── ACR-*.md
    ├── reviews/
    │   └── cycle-*/
    └── evidence/
        └── validation-*/
```

## 7.1 project.yaml

Keep this small and portable.

Example:

```yaml
schema_version: 1
project_id: coalition
name: Coalition
current_architecture_version: "1.0"
architecture_state: frozen
created_at: "..."
```

Do not place local absolute paths in Git-versioned project artifacts unless unavoidable.

## 7.2 validation.yaml

Example:

```yaml
schema_version: 1

enabled: true

policy:
  gate_review_on_required_failure: true
  allow_manual_runs: true
  allow_builder_requested_runs: true

commands:
  - id: frontend-tests
    name: Frontend unit tests
    command: npm test -- --run
    required: true
    timeout_seconds: 600

  - id: rust-tests
    name: Rust unit tests
    command: cargo test
    working_directory: src-tauri
    required: true
    timeout_seconds: 900

  - id: frontend-build
    name: Frontend build
    command: npm run build
    required: true
    timeout_seconds: 600
```

Validation can also be completely disabled:

```yaml
enabled: false
```

No workflow should imply that Coalition validation is mandatory for every project.

---

# 8. SQLite Operational State

SQLite stores local runtime state such as:

```text
projects
active_project_sessions
workflow_state
builder_epochs
builder_sessions
builder_events
permission_rules_local
permission_decisions
validation_runs
validation_processes
relay_packets
relay_imports
review_cycles
usage_samples
chatgpt_usage_estimates
activity_events
recovery_checkpoints
ui_preferences
```

Requirements:

- migrations must be versioned;
- DB writes affecting workflow state must be transactional;
- project artifacts must remain usable if SQLite is deleted;
- recovery should reconstruct as much as possible from `.coalition/` and Git;
- SQLite may cache artifact metadata but must not become the only copy of architecture or reviews.

---

# 9. Workflow State Machine

Implement authoritative transitions in Rust.

Minimum states:

```text
DRAFT
ARCHITECTING
READY_TO_FREEZE
FROZEN
BUILDING
VALIDATING
WAITING_FOR_REVIEW
CORRECTIONS_REQUIRED
BLOCKED
ARCHITECTURE_CONCERN
REVIEW_ACCEPTED
FINAL_VALIDATION
READY_FOR_HUMAN_REVIEW
HUMAN_ACCEPTED
ARCHITECTURE_CHANGE
ARCHITECTING_REVISION
READY_TO_REFREEZE
PAUSED
INTERRUPTED
```

Requirements:

- UI cannot directly assign arbitrary states.
- Every transition is validated.
- Every accepted transition is logged.
- Illegal transitions return typed errors.
- app restart does not lose authoritative state.
- active processes are reconciled on startup.
- architecture change can be invoked from any development state.
- human acceptance can only occur from `READY_FOR_HUMAN_REVIEW`.

Add unit tests for legal and illegal transitions.

---

# 10. ChatGPT Relay Subsystem

## 10.1 Outbound

Coalition creates a versioned relay packet and offers:

```text
Copy for ChatGPT
Open ChatGPT
```

Clipboard write occurs only after explicit user action.

## 10.2 Inbound

The user copies a ChatGPT response and presses:

```text
Import from Clipboard
```

or a configured global hotkey.

Coalition reads clipboard only at that moment.

No passive clipboard monitoring by default.

## 10.3 Relay Types

V1 requires at least:

```text
ARCHITECT_INITIAL
ARCHITECT_UPDATE
ARCHITECTURE_REVISION
REVIEW_REQUEST
REVIEW_VERDICT
ARCHITECTURE_CONCERN_ANALYSIS
```

Every packet has:

- schema version;
- project ID;
- packet ID;
- role;
- architecture version;
- related milestone/review cycle;
- expected response type.

## 10.4 Parsing

The prompt should request a structured YAML or JSON result block.

The parser must:

- prefer the structured block;
- tolerate surrounding prose;
- reject obviously unrelated clipboard data;
- provide a manual fallback when parsing fails;
- show a preview before substantive project-state changes;
- never execute commands merely because imported text contains them.

## 10.5 Reviewer Isolation

Encourage separate ChatGPT conversations:

```text
Project Architect
Implementation Reviewer
Architecture Revision
```

Coalition cannot enforce model independence at the subscription layer, but it can enforce different prompts and different evidence packages.

---

# 11. Antigravity Builder Adapter

Implement an isolated `AntigravityCliAdapter`.

Do not spread `agy` schema assumptions across the app.

Adapter responsibilities:

- detect CLI installation/version;
- validate authentication;
- enumerate available models;
- launch Builder process;
- send prompts over stdin;
- read NDJSON events;
- extract conversation ID;
- stream progress;
- capture usage metadata;
- capture stderr;
- detect permission denials;
- cancel/interrupt safely;
- resume supported conversations;
- switch model at safe boundaries;
- expose typed errors.

Preferred process mode:

```text
agy
  --input-format stream-json
  --output-format stream-json
```

Each architecture freeze/re-freeze creates a new **Builder epoch**.

A Builder epoch may contain multiple turns.

Persist:

- conversation ID;
- model;
- effort;
- epoch architecture version;
- start/end timestamps;
- terminal status;
- cumulative usage metadata.

---

# 12. Antigravity Model Selection

Coalition must expose Builder model selection.

Requirements:

1. Discover available models through the supported CLI (`agy models` or equivalent documented mechanism).
2. Store the user's selected model locally.
3. Allow reasoning effort selection when supported:
   - low;
   - medium;
   - high.
4. Show the active model in the project header/Builder card.
5. Do not silently fall back to another model if a pinned model becomes unavailable.
6. Surface model-unavailable errors clearly.
7. Allow model changes between Builder turns.
8. If model change requires process restart:
   - safely finish/stop the current turn;
   - retain conversation ID;
   - start a new CLI process with the new model;
   - resume the supported conversation;
   - log the model transition.
9. Never mutate the frozen architecture merely because the model changes.

UI concept:

```text
Builder

Model
Gemini ...                 [ Change ]

Reasoning
High                       [ Change ]
```

Do not build a Coalition-maintained model benchmark database in V1.

The CLI is the authoritative model-availability source.

---

# 13. Subscription Usage and Session Capacity

Coalition must provide a compact, always-accessible usage view.

This replaces v0.2's earlier "No AI Cost Accounting" concept.

Coalition still does **not** calculate dollar cost.

## 13.1 Antigravity Usage

Where supported by documented CLI metadata, display:

- active model;
- context-window used percentage;
- context-window remaining percentage;
- quota buckets;
- quota remaining fraction;
- reset time/countdown;
- AI credits if provided;
- cumulative token usage for the active Builder session.

Treat Antigravity-reported values as provider-reported.

The adapter must tolerate fields being absent or changing.

Do not make one optional quota field a crash dependency.

## 13.2 ChatGPT Usage Estimate

There is no assumption of an official machine-readable Plus quota interface.

Coalition therefore provides an **estimate**.

Display must use language such as:

```text
ChatGPT Usage — Estimated
~42% used
```

An information icon must explain:

> ChatGPT does not provide Coalition with authoritative subscription usage data. This value is estimated from the ChatGPT work relayed through Coalition and may differ significantly from the usage shown by ChatGPT.

Never label an estimated value as provider-reported.

## 13.3 Estimator Inputs

Track locally:

- outbound relay timestamp;
- inbound relay timestamp;
- Architect vs Reviewer role;
- approximate outbound text tokens;
- approximate inbound text tokens;
- turn count;
- user-selected ChatGPT model label if configured;
- rolling 5-hour window;
- rolling weekly window.

Use a versioned heuristic implementation behind an interface:

```text
ChatGptUsageEstimator
```

Initial approximate token calculation may use a documented characters-to-token heuristic.

The exact weighting is not a product contract and may be tuned later.

## 13.4 Calibration

Provide:

```text
Reset Estimate
Calibrate Estimate
```

so a user can realign Coalition after ChatGPT visibly resets or if the estimate drifts.

Store heuristic version with samples so future changes are explainable.

---

# 14. Permission System

Coalition owns the human-facing permission workflow while respecting Antigravity's supported permission engine.

Routine known-safe project activity should be able to proceed automatically.

Unknown, out-of-scope, or dangerous activity should pause for a human decision.

## 14.1 Permission Decisions

Present:

```text
Deny
Allow Once
Always Allow This
Enable Icarus
```

For appropriate events also permit:

```text
Always Deny This
```

## 14.2 Scope

Remembered approvals default to **project-local user state**, not global machine state and not Git-versioned project artifacts.

One developer's approvals must not silently become another developer's approvals merely because the repo was cloned.

## 14.3 Rule Matching

Represent actions with narrow patterns.

Examples:

```text
command(git status)
command(git diff*)
command(npm test*)
command(npm run build)
```

Avoid overly broad approvals such as:

```text
command(npm *)
command(git *)
```

unless the human explicitly chooses that breadth.

## 14.4 Risk Defaults

Likely routine:

- reads/writes inside project workspace;
- Git status/diff/log;
- configured project validation;
- ordinary build/test commands already declared by project policy.

Prompt by default:

- new package installation;
- new network access;
- commands outside repository;
- unconfigured shell commands;
- access to credentials;
- file access outside project;
- deployment;
- push to remote;
- destructive Git operations;
- system configuration.

Strong deny candidates:

- attempts to read SSH/private-key stores;
- password/credential stores;
- destructive commands outside project;
- commands matching explicit user deny rules.

## 14.5 Headless Antigravity Integration Spike

Stage 1A — Technical Proofs must determine the cleanest supported mechanism for applying Coalition decisions to project-scoped Antigravity permissions in headless mode.

Do not make final architecture depend on unsafe global configuration mutation.

Acceptable final implementation must:

- preserve project scoping;
- avoid leaking approvals between projects;
- log every policy mutation;
- remove temporary "allow once" rules after their intended scope;
- survive interrupted processes without leaving accidental broad permissions behind.

If the CLI cannot support dynamic one-time approval cleanly, report an Architecture Concern before designing an undocumented workaround.

---

# 15. Icarus Mode

**Icarus** is Coalition's explicit maximum-autonomy mode.

Semantics:

```text
Icarus ON
= automatically approve all Builder tool actions
  for the active project/run
```

Implementation may map to Antigravity's documented always-proceed / skip-permission mechanism where appropriate.

Requirements:

- never enabled by default;
- human must explicitly enable it;
- display a high-visibility persistent banner whenever active;
- indicate project/run scope;
- log activation and deactivation;
- do not automatically carry it to unrelated projects;
- do not silently restore it after application restart without an obvious visible state;
- offer one-click disable;
- final UI wording must clearly state that commands and file writes will be auto-approved.

Suggested banner:

```text
ICARUS MODE ACTIVE
All Builder actions for this project are being automatically approved.
[ Disable Icarus ]
```

The internal name and user-facing name are both **Icarus**.

---

# 16. Validation Subsystem

Validation is a first-class subsystem but is **optional and configurable**.

Its purposes are:

- give the Builder independent diagnostics;
- give the Architect evidence during architecture changes;
- give the Reviewer evidence;
- give the human visibility;
- optionally act as a workflow gate when the project chooses.

Coalition must not assume every project needs automated tests.

## 16.1 Two Test Layers

### Builder-run testing

Antigravity may run tests during development.

This is development feedback.

### Coalition-run validation

Coalition independently executes configured commands.

This is Coalition-owned diagnostic evidence.

These must be distinguishable in logs and UI.

## 16.2 Configurability

Project configuration supports:

```text
Validation entirely off

Validation on, diagnostic only

Validation on, required commands gate review

Validation on, mix of required and optional commands
```

## 16.3 Trigger Sources

Validation may be started by:

- workflow automation after Builder completion;
- human manually;
- Builder request routed through Coalition;
- pre-review gate;
- final-validation gate.

Each run records its trigger source.

## 16.4 Runtime Monitoring

While validation is running, the UI must show:

- run ID;
- overall state;
- current command;
- elapsed time;
- stdout/stderr stream or tail;
- completed commands;
- pass/fail;
- queued commands;
- progress when determinable.

Example:

```text
Validation

RUNNING                        02:14

1 / 4 complete

✓ Frontend tests       38 sec
▶ Rust tests           01:12
○ Typecheck
○ Build

[ View Log ] [ Stop ]
```

## 16.5 Stop Behavior

User can stop:

- current command;
- entire validation run.

Stopping must:

- send graceful termination first;
- escalate termination after timeout;
- record `CANCELED`;
- preserve partial logs;
- never report a canceled check as PASS.

## 16.6 Logging

Record for every command:

- run ID;
- command ID/name;
- exact command;
- working directory;
- start/end;
- duration;
- exit code;
- terminal state;
- stdout/stderr file location;
- trigger source;
- architecture version;
- Git commit/working-tree fingerprint;
- whether required/optional.

Logs containing potential secrets must be redacted before inclusion in ChatGPT relay packets.

## 16.7 Diagnostic Routing

On failure, Coalition should be able to generate a bounded diagnostic packet for Antigravity containing:

- failed command;
- exit code;
- relevant output tail;
- associated requirement/milestone;
- current Git state.

Architect packets during architecture-change analysis may include summarized validation history.

Reviewer packets may include validation summaries and relevant failure logs.

Raw megabyte-scale logs should not be blindly copied to ChatGPT.

---

# 17. Validation Command Safety

Configured validation commands are not exempt from process safety.

Requirements:

- command execution uses the common Coalition process layer;
- timeouts are enforced;
- process trees are tracked;
- cancellation works;
- output size is bounded in memory and streamed to disk;
- UI reads/tails logs rather than retaining unlimited output;
- commands run from explicit working directories;
- environment overrides are explicit;
- secrets are not written to `.coalition/` evidence by default.

Do not execute arbitrary ChatGPT-imported commands as validation merely because they appear in a response.

Validation configuration must be human-approved project state.

---

# 18. Git Integration

Use Git CLI through a typed adapter.

V1 requirements:

- repository detection;
- branch detection;
- dirty-tree detection;
- status;
- diff;
- log;
- commit hash;
- create Coalition branch;
- architecture freeze boundary;
- milestone boundary support;
- review correction boundary support.

Recommended working model:

```text
baseline branch
      ↓
coalition/<feature-or-project>
      ↓
architecture freeze
      ↓
milestone commits
      ↓
correction commits
```

Do not automatically:

- force push;
- hard reset;
- clean untracked files;
- delete branches;
- rewrite user history.

These require explicit human action/permission.

Before autonomous build begins, ambiguous dirty-state conditions must be surfaced.

---

# 19. Architecture Freeze

Freeze is a human-only action.

Freeze must:

1. validate required project artifacts exist;
2. snapshot the current contract under `architecture-versions/vX.Y/`;
3. record the architecture version;
4. record Git state/commit;
5. protect frozen artifacts logically;
6. create a Builder epoch;
7. generate the bounded Builder packet.

Coalition should detect external modifications to frozen contract files.

V1 behavior:

- do not rely on OS read-only flags;
- detect drift;
- block affected workflow progression;
- offer:
  - restore frozen artifact;
  - inspect diff;
  - enter Architecture Change.

---

# 20. Human-Only Architecture Change

Always expose:

```text
Change Architecture
```

from development states.

Invoking it:

- stops new Builder prompts;
- requests safe Builder pause/cancel if active;
- captures Git state;
- captures current Builder epoch/conversation ID;
- captures validation state;
- preserves open findings;
- records the initiating human event;
- transitions to `ARCHITECTURE_CHANGE`.

Coalition then prepares an Architect revision packet.

A new architecture version is not active until the human re-freezes.

Re-freeze creates a new Builder epoch so stale assumptions do not silently carry over.

---

# 21. Review and Correction Loop

Reviewer packet includes bounded evidence:

- relevant frozen architecture;
- relevant requirements;
- acceptance criteria;
- milestone;
- Git diff;
- selected source excerpts when necessary;
- validation summary;
- Builder completion report;
- prior open findings.

Reviewer verdicts:

```text
ACCEPT
CORRECTIONS_REQUIRED
BLOCKED
ARCHITECTURE_CONCERN
```

Imported findings are previewed before acceptance.

Accepted corrections are not passed as raw ChatGPT prose directly to Antigravity.

Coalition generates a normalized correction packet.

Normal loop:

```text
Builder
  ↓
optional validation
  ↓
Reviewer relay
  ↓
import findings
  ↓
Builder correction
  ↓
optional validation
  ↓
Reviewer
```

If validation is disabled, the loop still works.

If required validation is enabled and configured as a review gate, failures prevent automatic transition to `WAITING_FOR_REVIEW` until the user overrides or the checks pass.

---

# 22. Activity and Audit Logging

Coalition should produce understandable activity history without requiring raw terminal reading.

Event categories:

- workflow transition;
- architecture freeze/change;
- Builder session start/stop;
- model change;
- usage sample;
- permission request;
- permission decision;
- Icarus toggle;
- Git operation;
- validation start/stop/result;
- relay packet generation/import;
- review verdict;
- recovery event;
- error.

Every event should have:

- timestamp;
- project;
- event type;
- actor/source;
- summary;
- relevant IDs;
- safe structured metadata.

Logs must avoid secrets.

---

# 23. Recovery

Treat interruption as normal.

On startup:

1. inspect persisted active workflow state;
2. check for stale process records;
3. inspect repository state;
4. reconcile pending validation;
5. inspect pending Builder epoch/session;
6. report last durable event.

Example:

```text
Interrupted project detected

Coalition
Architecture v1.1
Builder Epoch 3

Last durable event:
Builder completed correction REV-014.

Validation:
Canceled during Rust tests.

[ Resume ]
[ Inspect ]
```

Do not pretend a dead process is still running.

---

# 24. Security Requirements

V1 must implement:

- project-scoped filesystem behavior;
- explicit non-project access permission;
- no stored AI passwords/tokens;
- no hidden OAuth handling;
- command argument sanitization;
- process lifecycle ownership;
- no automatic deployment;
- no automatic remote push without policy;
- imported AI text treated as untrusted;
- path traversal protections;
- relay schema validation;
- SQL migrations;
- log redaction helpers;
- architecture drift detection;
- Icarus visibility.

Tauri capabilities should be narrowly scoped.

Do not expose broad shell/filesystem capability directly to arbitrary frontend JavaScript when Rust commands can provide a narrower API.

---

# 25. UI Baseline

Coalition should look like a project-control application, not an IDE.

Primary areas:

```text
Projects
Project Overview
Architecture
Builder
Usage
Validation
Review
Activity
Settings
```

## 25.1 Project Dashboard

At a glance:

```text
COALITION

Herald
BUILDING
Architecture v1.2
Builder: Gemini ...
Antigravity quota: 71% remaining
ChatGPT: ~34% estimated 5h usage
Validation: running

LiTerra3D
WAITING FOR REVIEW
Architecture v2.0
```

## 25.2 Project View

Show:

- state;
- architecture version;
- Builder model/effort;
- Builder activity;
- usage;
- validation;
- open review findings;
- pause;
- Change Architecture.

## 25.3 Global Human-Decision Queue

A user should be able to immediately see when Coalition needs them.

Examples:

```text
Permission required
Review relay ready
Architecture concern
Validation failed
Architecture ready to freeze
Final human review ready
```

---

# 26. Testing Strategy for Coalition Itself

Coalition must have its own automated test suite.

## 26.1 Rust Unit Tests

Required areas:

- workflow transition rules;
- artifact versioning;
- relay parser;
- permission rule matching;
- Icarus state;
- Git adapter result parsing;
- Antigravity NDJSON parsing;
- process termination;
- validation orchestration;
- usage/quota parsing;
- ChatGPT estimator windows;
- recovery reconciliation;
- log redaction.

## 26.2 Frontend Tests

Use Vitest + React Testing Library or equivalent.

Cover:

- key project-state views;
- permission modal;
- Icarus warning;
- usage estimate disclaimer;
- validation monitor;
- review import preview;
- architecture freeze confirmation;
- interrupted-project recovery view.

## 26.3 Integration Tests

Use fixtures/fake executables rather than consuming AI quota in CI.

Create fake command binaries/scripts for:

```text
agy
git
validation command
```

The fake `agy` must be able to emit:

- init event;
- tool events;
- text deltas;
- result;
- usage metadata;
- permission denial on stderr;
- malformed JSON;
- process hang;
- canceled status;
- unavailable model.

Real Antigravity smoke tests are opt-in/manual and must not run in normal public CI.

## 26.4 CI

Initial GitHub Actions should run at least:

```text
npm install / lockfile install
frontend typecheck
frontend tests
frontend build
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

Add Tauri packaging smoke build when practical on Windows.

macOS CI is added during the post-V1 V1.1 macOS support stage.

---

# 27. Coding and Quality Rules

- TypeScript strict mode.
- Rust errors should use typed error categories rather than string-only flow control.
- No unchecked `unwrap()` in production paths where recoverable errors are possible.
- External process protocols must be adapter-isolated.
- State-changing Tauri commands return structured results/errors.
- UI should never parse raw Antigravity NDJSON itself.
- UI should never invoke arbitrary shell commands directly.
- Prefer narrow components and modules over broad "manager" classes that mix concerns.
- All persistent schemas have explicit versions.
- Backward-compatible migrations are required after public releases begin.
- No generated binaries committed to Git.
- No private diagnostic captures committed.

---

---

# 28. Revised V1 Development Roadmap

Coalition V1 is organized into **five product stages**.

The stages are the human-facing roadmap.

Within each stage, Antigravity must still execute the smaller checkpoints in order, verify each checkpoint, update documentation, and commit a recoverable boundary before continuing.

The consolidation is:

```text
Stage 1 — Foundation
  ├── Stage 1A — Repository Bootstrap and Technical Proofs
  │              (former Phase 0)
  └── Stage 1B — Coalition Core and Project Persistence
                 (former Phase 1)

Stage 2 — Architecture Contract
  ├── Stage 2A — ChatGPT Relay and Architecture Workspace
  │              (former Phase 2)
  └── Stage 2B — Architecture Freeze and Git Boundaries
                 (former Phase 3)

Stage 3 — Builder Control Plane
  ├── Stage 3A — Antigravity Builder Integration
  │              (former Phase 4)
  ├── Stage 3B — Permissions and Icarus
  │              (former Phase 5)
  └── Stage 3C — Usage and Capacity
                 (former Phase 6)

Stage 4 — Governed Development Loop
  ├── Stage 4A — Validation Engine
  │              (former Phase 7)
  └── Stage 4B — Reviewer and Correction Loop
                 (former Phase 8)

Stage 5 — V1 Completion and Windows Release
  ├── Stage 5A — Human-Only Architecture Change
  │              (former Phase 9)
  └── Stage 5B — Recovery, Product UX, and Windows Packaging
                 (former Phase 10)

Post-V1 / V1.1
  └── macOS Platform Support
      (former Phase 11)
```

The consolidation changes roadmap presentation, not product requirements.

A stage is complete only when all of its internal checkpoints and stage-level acceptance criteria have been verified.

## 28.1 Current Implementation Status at Plan v1.1

At the time of this roadmap revision:

```text
Stage 1 — Foundation                         COMPLETE
Stage 2 — Architecture Contract              NEXT
Stage 3 — Builder Control Plane              NOT STARTED
Stage 4 — Governed Development Loop          NOT STARTED
Stage 5 — V1 Completion and Windows Release  NOT STARTED
V1.1 — macOS Platform Support                POST-V1
```

Stage 1 has established the authoritative project foundation, including the Phase 0 technical proofs and Phase 1 project/persistence work.

Future implementation sessions should begin from Stage 2A, not repeat Stage 1 technical spikes unless a regression or external-tool change requires targeted revalidation.

---

# 29. Stage 1 — Foundation

**Status at Plan v1.1:** COMPLETE

**Product goal:** Establish the trustworthy local control-plane foundation on which every governed Coalition workflow depends.

Stage 1 combines the original technical prototype and project-persistence phases.

Its product-level result is:

```text
Launch Coalition
      ↓
Open/register a local Git repository
      ↓
Establish durable .coalition project identity
      ↓
Persist operational state in SQLite
      ↓
Expose authoritative Rust workflow state
      ↓
Inspect current Git state
      ↓
Close/reopen Coalition
      ↓
Recover project identity and operational state
```

## 29.1 Stage 1A — Repository Bootstrap and Technical Proofs

**Formerly:** Phase 0

**Goal:** Prove external and desktop assumptions before product workflow depends on them.

Implement/prove:

- public repository skeleton;
- Tauri + React + TypeScript + Rust boot;
- SQLite connection/migration proof;
- system Git detection;
- Antigravity CLI detection/version;
- launch `agy` in structured headless mode;
- read NDJSON incrementally;
- capture conversation ID;
- cancel process;
- enumerate models;
- select a model and effort;
- investigate quota/status integration;
- clipboard explicit read/write;
- global hotkey;
- native opener;
- process logging;
- fake-`agy` harness for quota-free CI/testing.

Technical proofs:

### Antigravity usage

Determine the most stable documented method for Coalition to obtain, where exposed:

- model;
- context-window remaining;
- quota remaining/reset data;
- plan tier.

Do not scrape the Antigravity GUI.

### Project-scoped permissions

Determine the supported mechanism for:

- project-scoped allow/deny/ask rules;
- temporary allow-once behavior;
- persistent project-local user approval;
- Icarus mapping;
- cleanup after temporary approval.

Do not ship a solution that mutates broad global permissions as an undocumented shortcut.

### Model switching with conversation continuity

Verify:

- discover models;
- start conversation with model A;
- persist conversation ID;
- restart process with model B;
- resume conversation safely.

### Stage 1A acceptance

- clean build;
- Tauri window launches;
- external-interface findings documented in `docs/`;
- real `agy` smoke test succeeds;
- fake-`agy` harness exists;
- structured Builder events are parsed by Rust;
- cancellation works;
- Phase 0 diagnostics do not depend on consuming AI quota in CI.

## 29.2 Stage 1B — Coalition Core and Project Persistence

**Formerly:** Phase 1

**Goal:** Establish authoritative local project state and durable project identity.

Implement:

- SQLite forward migrations;
- project registration;
- open existing repository;
- `.coalition/` durable hierarchy;
- strict `project.yaml` schema;
- artifact filesystem service;
- Rust workflow state machine;
- activity log;
- project dashboard shell;
- Git repository adapter;
- dirty-state detection;
- basic local application settings;
- restart/reopen behavior;
- SQLite-loss rehydration;
- project identity reconciliation;
- safe durable artifact replacement and recovery.

The Stage 1 durable/operational authority boundary is:

```text
.coalition/
= what the project IS

SQLite
= what Coalition is DOING
```

Stage 1B must ensure:

- `project.yaml` is portable and contains no local machine path;
- project IDs are stable UUID v4 values;
- workflow state is Rust-owned and typed;
- React cannot assign arbitrary states;
- state mutation and associated activity events are transactional;
- durable project identity can rehydrate operational state after SQLite loss;
- an unavailable repository remains registered without fabricated durable state;
- project identity conflicts are surfaced rather than silently merged;
- recognized recovery artifacts never cause Coalition to invent a new identity;
- successful project open validates the full `.coalition/` hierarchy before later stages write into it;
- Phase 0 technical diagnostics remain available without dominating the product UI.

### Stage 1 exit criteria

Stage 1 is complete only when:

- project registration/reopen is idempotent;
- project list survives restart;
- `.coalition/` survives SQLite deletion and can rehydrate operational identity;
- all required V1 workflow states and legal/illegal transition tests pass;
- Git state is visible and distinguishes clean/dirty/unavailable conditions;
- durable descriptor replacement/recovery is validated on Windows;
- filesystem escape through the managed `.coalition/` hierarchy is rejected;
- Phase 0 regression tests remain green;
- full local validation passes;
- GitHub Actions is green for the Stage 1 closure commit.

Once Stage 1 is accepted, do not continue adding speculative foundation abstractions. Move to Stage 2 unless a real regression is discovered.

---

# 30. Stage 2 — Architecture Contract

**Product goal:** Let a human use ChatGPT as Architect through an explicit relay, evolve real project architecture inside Coalition, and deliberately freeze a versioned implementation contract.

This stage combines the former ChatGPT Relay/Architecture Workspace and Architecture Freeze/Git Boundaries phases.

The stage-level journey is:

```text
Open Coalition project
      ↓
Prepare Architect relay packet
      ↓
Human sends it through ChatGPT
      ↓
Human copies ChatGPT response
      ↓
Coalition imports and previews structured changes
      ↓
Project architecture artifacts evolve
      ↓
Coalition determines architecture readiness
      ↓
Human chooses Freeze Architecture
      ↓
Architecture v1.0 snapshot created
      ↓
Git/frozen-contract boundary recorded
      ↓
Builder implementation packet generated
```

Stage 2 must not start Antigravity implementation automatically merely because the architecture becomes ready.

The human freeze action remains the authority boundary.

## 30.1 Stage 2A — ChatGPT Relay and Architecture Workspace

**Formerly:** Phase 2

**Goal:** Support real Architect work without API access or ChatGPT scraping.

Implement:

- versioned relay packet builder;
- Architect relay roles/types;
- clipboard copy after explicit user action;
- browser opener;
- explicit clipboard import;
- structured relay envelope;
- Architect update parser;
- structured YAML/JSON result extraction;
- manual parse/edit fallback;
- unrelated-clipboard rejection;
- imported change preview;
- relay history persistence;
- architecture artifact workspace;
- required-artifact/readiness evaluation;
- global relay/import hotkeys;
- safe bounded relay logging.

At minimum support relay types already defined by this plan:

```text
ARCHITECT_INITIAL
ARCHITECT_UPDATE
ARCHITECTURE_REVISION
REVIEW_REQUEST
REVIEW_VERDICT
ARCHITECTURE_CONCERN_ANALYSIS
```

Stage 2A should initially exercise the Architect types; later relay types may exist in schema/code without prematurely building Stage 4/5 workflows.

### Architect artifact workspace

The workspace should support the durable design package already defined under `.coalition/`, including as applicable:

```text
design/product-vision.md
design/requirements.md
design/architecture.md
design/constraints.md
design/interfaces.md
design/security.md

implementation/implementation-plan.md
implementation/acceptance-criteria.yaml
implementation/test-plan.md

decisions/ADR-*.md
```

Do not require every file for every possible project simply because the directory exists.

Readiness rules must be explicit and versioned.

Imported ChatGPT content is untrusted until parsed and confirmed.

It must never execute local commands.

### Stage 2A acceptance

- user can take a real project through at least one Architect relay round trip;
- user explicitly initiates clipboard export/import;
- no passive clipboard surveillance;
- unrelated clipboard content is rejected safely;
- malformed structured output has a manual recovery path;
- imported changes are previewed before durable project mutation;
- relay history persists across app restart;
- architecture artifacts are inspectable and editable through Coalition's governed workflow;
- imported AI text cannot directly invoke shell/process behavior;
- readiness state is derived by Rust/core rules rather than arbitrary UI state.

**Checkpoint rule:** When Stage 2A acceptance passes, update docs, run full validation, and commit a recoverable boundary before beginning Stage 2B.

## 30.2 Stage 2B — Architecture Freeze and Git Boundaries

**Formerly:** Phase 3

**Goal:** Turn the completed architecture workspace into a reliable human-approved implementation contract.

Implement:

- explicit readiness checks;
- freeze confirmation UX;
- human-only freeze action;
- architecture version numbering;
- snapshots under `architecture-versions/vX.Y/`;
- recorded architecture version in `project.yaml`;
- Git boundary/commit fingerprint recording;
- frozen active-contract protection;
- frozen artifact drift detection;
- restore/inspect/change-architecture choices when drift is detected;
- bounded Builder implementation packet generation.

Freeze must:

1. validate required architecture artifacts;
2. ensure the project is in a legal workflow state;
3. require explicit human confirmation;
4. snapshot the contract;
5. update durable architecture state/version safely;
6. record the Git boundary;
7. transition authoritative workflow state through Rust;
8. generate the Builder packet for the next stage.

Do not automatically mutate architecture because Git or source implementation differs.

Do not silently overwrite prior architecture versions.

### Stage 2B acceptance

- only explicit human action can freeze;
- architecture readiness alone does not freeze;
- architecture version snapshot is immutable historical evidence;
- `project.yaml` records the active frozen version consistently;
- frozen snapshot and Git boundary are recoverable after restart;
- external modification of active frozen contract files is detected;
- workflow cannot silently continue on frozen-contract drift;
- the user can inspect or restore drift, or deliberately enter Architecture Change when that later workflow is available;
- a bounded Builder packet can be generated from the frozen architecture without sending the entire repository blindly.

### Stage 2 exit criteria

Stage 2 is complete when a user can demonstrably perform:

```text
project
→ Architect relay
→ imported structured design
→ architecture workspace
→ readiness
→ explicit human freeze
→ Architecture v1.0
→ frozen Git/contract boundary
→ Builder packet
```

Run the full Coalition validation suite and perform a manual Stage 2 desktop smoke test before declaring this stage complete.

Do not begin Stage 3 until Stage 2A and Stage 2B are separately verified and the complete Stage 2 journey is human-reviewed.

---

# 31. Stage 3 — Builder Control Plane

**Product goal:** Turn the Stage 1 Antigravity technical proof into a safe, observable, usable Builder controlled by Coalition.

This stage combines Builder execution, human-facing permissions, Icarus mode, and usage/capacity awareness.

## 31.1 Stage 3A — Antigravity Builder Integration

**Formerly:** Phase 4

**Goal:** Run one bounded Builder milestone against a frozen architecture end to end.

Implement:

- Builder epoch;
- one active Antigravity conversation per build epoch;
- persistent stream-json process where validated;
- bounded Builder packets from frozen architecture;
- meaningful progress event mapping;
- selected model;
- reasoning effort;
- cancellation;
- retry/recovery;
- supported conversation resume;
- model switching at safe boundaries;
- Builder completion report;
- persisted Builder session metadata;
- fake-`agy` integration tests.

Acceptance:

- Antigravity implements a fixture milestone against a frozen test contract;
- UI shows meaningful activity rather than requiring raw NDJSON;
- process can be safely stopped;
- conversation ID persists;
- model and effort are visible;
- unavailable pinned model fails visibly rather than silently falling back;
- restart does not falsely report a dead Builder as running;
- each architecture re-freeze creates a new Builder epoch.

**Checkpoint rule:** Verify and commit Stage 3A before Stage 3B.

## 31.2 Stage 3B — Permissions and Icarus

**Formerly:** Phase 5

**Goal:** Make autonomous Builder operation safe and usable.

Implement based on the documented Stage 1A permission findings:

- permission policy evaluator;
- routine project-safe auto-allow patterns;
- human permission decision UI;
- Deny;
- Allow Once;
- Always Allow This;
- Always Deny This;
- project-local remembered rules;
- permission history;
- temporary-rule cleanup;
- Icarus enable/disable;
- persistent high-visibility Icarus warning;
- restart reconciliation.

Acceptance:

- unknown or out-of-policy command prompts;
- allow-once does not persist beyond intended scope;
- remembered project rule works on later matching actions;
- unrelated project does not inherit the rule;
- higher-risk actions remain visible/escalated;
- Icarus bypasses prompts within its explicit project/run scope;
- Icarus is never enabled by default;
- Icarus active state is impossible to miss;
- all permission decisions are logged;
- no undocumented global Antigravity permission mutation is used.

**Checkpoint rule:** Verify and commit Stage 3B before Stage 3C.

## 31.3 Stage 3C — Usage and Capacity

**Formerly:** Phase 6

**Goal:** Give the user practical "how much juice is left" visibility.

Implement:

- Antigravity provider-reported usage samples where supported;
- active model display;
- context remaining;
- quota buckets;
- reset timers;
- missing-field tolerance;
- ChatGPT relay telemetry;
- approximate outbound/inbound token estimation;
- rolling 5-hour ChatGPT estimate;
- rolling weekly ChatGPT estimate;
- estimate information/disclaimer;
- calibration/reset controls.

Acceptance:

- Antigravity data identifies itself as provider-reported;
- absent provider fields do not crash the app;
- ChatGPT data identifies itself as estimated;
- ChatGPT estimate explanatory text clearly states limitations;
- no fictitious dollar-cost accounting is shown;
- usage history survives restart;
- model selector uses the live supported Antigravity model list rather than a hardcoded catalog.

### Stage 3 exit criteria

Stage 3 is complete when Coalition can:

```text
receive a frozen Builder packet
      ↓
start/resume Antigravity
      ↓
show model / effort / progress
      ↓
govern permissions
      ↓
allow explicit Icarus mode
      ↓
show available session/quota capacity
      ↓
stop safely
```

Run full automated validation plus a manual Antigravity smoke test.

Normal public CI must continue using fake `agy` and must not consume subscription quota.

---

# 32. Stage 4 — Governed Development Loop

**Product goal:** Prove Coalition's central thesis end to end: implementation is checked by deterministic evidence and independently challenged by ChatGPT Reviewer, with accepted corrections routed back to Antigravity.

## 32.1 Stage 4A — Validation Engine

**Formerly:** Phase 7

**Goal:** Add optional Coalition-owned diagnostics independent of Builder claims.

Implement:

- `validation.yaml` parser/schema;
- validation enabled/disabled;
- diagnostic-only mode;
- required-gate mode;
- required/optional commands;
- validation queue;
- common process runner;
- sequential execution by default;
- timeouts;
- live stdout/stderr;
- disk-backed logs;
- bounded UI tails;
- stop current;
- stop all;
- CANCELED state;
- manual runs;
- workflow-triggered runs;
- Builder-requested runs through Coalition;
- Builder diagnostic packet;
- Reviewer validation summary.

Acceptance:

- a project with validation disabled can continue through the workflow;
- enabled project executes configured commands;
- user can monitor current command and run status;
- user can cancel safely;
- canceled checks never report PASS;
- partial logs remain available;
- required failure blocks review only when project policy says it should;
- optional failure remains diagnostic;
- bounded diagnostics can be routed to Builder;
- arbitrary ChatGPT-imported commands do not become validation commands.

**Checkpoint rule:** Verify and commit Stage 4A before Stage 4B.

## 32.2 Stage 4B — Reviewer and Correction Loop

**Formerly:** Phase 8

**Goal:** Complete the primary Coalition governed-development loop.

Implement:

- Reviewer packet generation;
- relevant architecture/requirements selection;
- bounded Git diff/evidence inclusion;
- validation summary inclusion;
- review import parser;
- structured verdicts;
- finding IDs;
- severity;
- requirement linkage;
- finding preview;
- accept/reject import;
- normalized correction packet;
- route accepted corrections to active Builder;
- open/resolved finding tracking;
- repeated correction cycles;
- Reviewer ACCEPT transition.

Supported V1 verdicts remain:

```text
ACCEPT
CORRECTIONS_REQUIRED
BLOCKED
ARCHITECTURE_CONCERN
```

Reviewer output must never be passed raw to Antigravity as executable instruction.

Coalition must normalize accepted findings into its own correction protocol.

### Stage 4B acceptance

At minimum prove an intentional-defect scenario:

```text
Frozen architecture
      ↓
Builder implementation with known defect
      ↓
Coalition validation/evidence
      ↓
Reviewer relay
      ↓
CORRECTIONS_REQUIRED
      ↓
Human confirms imported findings
      ↓
Coalition correction packet
      ↓
Antigravity fixes defect
      ↓
Validation reruns according to policy
      ↓
Reviewer ACCEPT
```

Acceptance requires:

- finding history remains durable and inspectable;
- imported review text cannot execute local commands;
- corrections are tied to the relevant architecture version and review cycle;
- Reviewer ACCEPT cannot bypass configured required validation gates;
- Architecture Concern remains a human escalation, not an AI-authorized architecture mutation.

### Stage 4 exit criteria

Stage 4 is the first complete proof of Coalition's central product thesis.

It is complete when the intentional-defect correction journey works reliably, repeatably, and recoverably.

Do not declare V1 finished at Stage 4; human-only Architecture Change and Windows product hardening remain.

---

# 33. Stage 5 — V1 Completion and Windows Release

**Product goal:** Add the human-only architecture revision cycle, harden interruption/recovery and UX, and produce a usable Windows V1 release candidate.

## 33.1 Stage 5A — Human-Only Architecture Change

**Formerly:** Phase 9

**Goal:** Implement the foundational human-only "Oh Shit" cycle.

Implement:

- always-available Change Architecture action from applicable development states;
- safe Builder pause/cancel;
- validation pause/cancel handling;
- durable checkpoint;
- current Git state capture;
- current Builder epoch/session capture;
- open review findings preservation;
- ACR creation;
- Architect revision relay;
- impact analysis packet/summary;
- `ARCHITECTING_REVISION`;
- ready-to-refreeze;
- vNext snapshot;
- explicit human re-freeze;
- new Builder epoch;
- old Builder epoch retained historically.

Acceptance:

- human can interrupt active implementation;
- no AI agent can self-authorize architecture revision;
- open implementation/review state is preserved;
- prior architecture remains inspectable;
- ACR records provenance;
- revised contract is not active until human re-freezes;
- re-freeze creates a new Builder epoch;
- stale prior Builder process cannot continue writing unnoticed.

**Checkpoint rule:** Verify and commit Stage 5A before Stage 5B.

## 33.2 Stage 5B — Recovery, Product UX, and Windows Packaging

**Formerly:** Phase 10

**Goal:** Make Coalition usable as a Windows application outside the development environment.

Implement:

- startup reconciliation;
- interrupted Builder truth reconciliation;
- interrupted validation truth reconciliation;
- stale process handling;
- interrupted-state UX;
- human-decision queue;
- notifications;
- configurable hotkeys;
- polished project dashboard;
- activity/history views;
- validation monitor;
- usage card;
- review/finding UX;
- architecture-change UX polish;
- tray behavior if valuable;
- NSIS or MSI packaging;
- Windows code-signing plan/documentation;
- release artifact workflow;
- installation/update documentation;
- Windows packaging smoke test in CI when practical.

Acceptance:

- fresh Windows machine can install packaged Coalition without developer runtimes;
- interruption/restart is represented truthfully;
- no dead process is shown as running;
- no critical routine decision requires raw terminal usage;
- Architecture Change remains accessible;
- final human acceptance remains explicit;
- GitHub CI is green;
- release candidate installs and uninstalls cleanly;
- release artifacts contain no private development data or credentials.

Code signing may remain a release prerequisite if certificate procurement is incomplete.

Do not fake signing.

### Stage 5 / V1 exit criteria

Coalition Windows V1 is complete only when the complete Definition of Done in Section 35 passes.

The final stage-level journey must support:

```text
Idea / existing planning
      ↓
Architect
      ↓
Human freeze
      ↓
Builder
      ↓
Permissions / usage visibility
      ↓
Validation
      ↓
Independent Reviewer
      ↓
Correction loop
      ↓
Human Architecture Change when needed
      ↓
Re-freeze / new Builder epoch
      ↓
Ready for Human Review
      ↓
Explicit Human Acceptance
```

and the product must be installable/recoverable as a normal Windows desktop application.

---

# 34. Post-V1 — V1.1 macOS Platform Support

macOS is no longer part of the Windows V1 stage count.

Begin V1.1 macOS support only after the Windows V1 vertical slice is stable.

Validate:

- macOS application packaging;
- DMG/app distribution;
- code signing;
- notarization requirements;
- global hotkeys;
- clipboard behavior;
- browser opener;
- process signaling;
- Antigravity CLI discovery;
- Git discovery;
- permission scoping;
- filesystem/path handling;
- durable artifact replacement semantics;
- validation process-tree cancellation;
- installer/update behavior.

Do not fork Coalition into a separate macOS architecture.

Use platform adapters only where OS behavior genuinely differs.

Linux remains a future target after Windows and macOS behavior are stable.

---

# 35. Definition of Done for Windows V1

Coalition V1 is complete when a user can:

1. Install Coalition as a normal Windows desktop application.
2. Create/open a local Git project.
3. Use ChatGPT Plus as Architect through explicit human relay.
4. Import structured design changes.
5. Inspect and evolve durable architecture artifacts.
6. Explicitly freeze an architecture contract.
7. Preserve immutable architecture-version history.
8. Start Antigravity Builder without AI API keys.
9. Select an available Antigravity model and reasoning effort.
10. See meaningful Builder activity.
11. Stop Builder activity safely.
12. See Antigravity context/quota data when available.
13. See a clearly labeled rough ChatGPT usage estimate.
14. Respond to out-of-policy Builder activity using deny/allow-once/remembered rules.
15. Enable/disable Icarus mode explicitly.
16. Configure Coalition validation or disable it entirely.
17. Watch validation progress and logs.
18. Stop validation safely.
19. Use validation failures as diagnostics for Builder/Architect/Reviewer.
20. Generate a bounded Reviewer packet.
21. Use ChatGPT Plus as Reviewer.
22. Import structured findings safely.
23. Route accepted corrections to Antigravity through Coalition's normalized protocol.
24. Repeat build/validate/review cycles until Reviewer acceptance.
25. Pause development.
26. Invoke human-only Architecture Change.
27. Preserve ACRs and prior architecture versions.
28. Re-freeze a revised contract.
29. Start a fresh Builder epoch against the revised architecture.
30. Recover truthfully after application/machine/process interruption.
31. Detect frozen-contract drift.
32. Reach Ready for Human Review.
33. Require explicit human final acceptance.
34. Install/uninstall a Windows release candidate without developer runtimes.
35. Do all of the above without OpenRouter, OpenAI API, Gemini API keys, Coalition cloud accounts, or Coalition AI billing.

macOS support is not required for Windows V1 completion.

---

# 36. Explicit V1 Non-Goals

Do not implement unless this plan is formally revised:

- OpenRouter;
- OpenAI API;
- Gemini API;
- direct AI provider billing;
- Coalition model marketplace;
- public benchmark aggregation;
- Coalition-native coding agent;
- embedded IDE;
- ChatGPT scraping;
- Antigravity GUI automation;
- automatic production deployment;
- cloud Coalition backend;
- Coalition accounts;
- multi-user collaboration;
- organizations/RBAC;
- SaaS billing;
- remote Builder fleet;
- mobile application;
- GitHub App;
- plugin marketplace;
- enterprise policy server.

macOS is not a V1 non-goal; it is the planned V1.1 platform-expansion target after Windows V1 is complete.

---

# 37. Implementation Sequencing Rules for Antigravity

When implementing this plan:

1. Work **stage by stage** and **checkpoint by checkpoint**.
2. Do not execute an entire multi-checkpoint stage as one unbounded autonomous run.
3. Do not implement later-stage abstractions prematurely.
4. At each A/B/C checkpoint:
   - inspect the current baseline;
   - verify a clean or deliberately understood working tree;
   - add/adjust tests;
   - implement only the checkpoint scope;
   - update relevant documentation;
   - run affected validation;
   - run the full validation suite before checkpoint closure;
   - produce a concise completion report;
   - commit a recoverable boundary.
5. A checkpoint is not complete because code exists; verify its acceptance criteria.
6. A product stage is not complete until every internal checkpoint and the stage-level journey have been verified.
7. Human review should occur at the end of each product stage and may also occur between checkpoints where risk warrants it.
8. Do not silently remove requirements that prove difficult.
9. Do not introduce a new framework/runtime unless required by the authoritative design.
10. Prefer supported third-party interfaces over UI automation.
11. When external documentation contradicts this plan, stop the affected integration and report an Architecture Concern.
12. Preserve cross-platform seams even while developing Windows first.
13. Keep the public repository free of private data and credentials.
14. Real Antigravity smoke tests remain opt-in/manual; public CI must use fake fixtures and consume no subscription quota.
15. Do not begin the next product stage until the prior stage's validation and human review are complete.

---

# 38. Coalition Self-Validation Baseline

Coalition's own project validation should remain approximately:

```yaml
schema_version: 1

enabled: true

policy:
  gate_review_on_required_failure: true

commands:
  - id: frontend-typecheck
    name: Frontend TypeScript check
    command: npm run typecheck
    required: true
    timeout_seconds: 600

  - id: frontend-tests
    name: Frontend tests
    command: npm test -- --run
    required: true
    timeout_seconds: 900

  - id: frontend-build
    name: Frontend build
    command: npm run build
    required: true
    timeout_seconds: 600

  - id: rust-format
    name: Rust formatting
    command: cargo fmt --check
    working_directory: src-tauri
    required: true
    timeout_seconds: 300

  - id: rust-clippy
    name: Rust Clippy
    command: cargo clippy -- -D warnings
    working_directory: src-tauri
    required: true
    timeout_seconds: 900

  - id: rust-tests
    name: Rust tests
    command: cargo test
    working_directory: src-tauri
    required: true
    timeout_seconds: 1200
```

Packaging builds may remain a separate/manual release check until Stage 5B if they materially slow every development cycle.

Add a Windows Tauri package smoke build in CI when packaging work begins.

V1.1 adds macOS CI once macOS implementation begins.

---

# 39. Next Antigravity Assignment — Stage 2A

Stage 1 is complete.

The next bounded implementation session should execute **Stage 2A — ChatGPT Relay and Architecture Workspace only**.

Do not ask Antigravity to implement all of Stage 2 in one autonomous run.

Initial Stage 2A assignment should be grounded in the requirements of Section 30.1 and should:

- inspect the current Stage 1 baseline;
- preserve the Stage 1 durable/operational authority model;
- implement versioned Architect relay packets;
- implement explicit outbound clipboard copy;
- implement explicit inbound clipboard import;
- implement ChatGPT opener behavior;
- parse structured Architect results with safe fallback;
- persist relay history;
- preview imported changes before project mutation;
- establish the architecture artifact workspace;
- implement required-artifact/readiness evaluation;
- add relay/import hotkeys;
- maintain no-passive-clipboard-surveillance and no-ChatGPT-scraping boundaries;
- add focused Rust/frontend/integration tests;
- update documentation;
- run the full Coalition validation suite;
- commit a recoverable Stage 2A boundary.

The assignment must end before Stage 2B.

After Stage 2A is human-reviewed and accepted, issue a separate Stage 2B implementation assignment for Architecture Freeze and Git Boundaries.

---

# 40. Master Product Principle

Coalition is successful when it becomes difficult for an AI implementation to drift silently away from what the human actually approved.

The human defines the product.

The architecture records the product.

Antigravity attempts the implementation.

Coalition observes and governs the work.

Configured validation supplies diagnostics.

ChatGPT independently challenges the implementation.

Coalition records the evidence and decisions.

Only the human decides when the product is finished.

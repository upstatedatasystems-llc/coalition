# Changelog

All notable changes to Coalition will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [Unreleased]

### Added - Stage 3 (Builder Control Plane)
- Common bounded process execution layer (`ProcessRunner`): eliminated stream busy-spinning with guarded `select!` loop, implemented Windows process-tree termination via `taskkill /F /T /PID` to prevent orphaned child processes, and added asynchronous stdin streaming with clean EOF closure.
- Memory-bounded output capture: buffers up to 10,000 lines or 10 MB per turn with explicit `is_truncated` tracking, preventing runaway memory exhaustion during excessive generation.
- Production `AntigravityCliAdapter` execution: routes headless stream-json execution through `ProcessRunner`, dynamically parsing `init`, `step_update`, and `result` NDJSON events.
- Frozen Builder Packet authority: Builder turn prompt strictly derived from the Stage 2 frozen `builder-packet.json` contract, epoch ID, and rules with zero frontend arbitrary prompt override.
- Strict workflow & contract drift gating: blocks Builder turns if workflow state is not `FROZEN`, `BUILDING`, or `CORRECTIONS_REQUIRED`, automatically transitions `FROZEN` to `BUILDING`, and fails closed if frozen contract drift is detected.
- Dynamic Antigravity model discovery & safe switching: queries available models via `agy models`, exposes reasoning effort (`--effort low|medium|high`), and supports safe model switching mid-conversation by providing a new `--model` alongside existing `--conversation`.
- Startup session reconciliation: automatically reconciles orphaned `RUNNING` sessions from previous app runs or unexpected crashes to `INTERRUPTED` on startup, logging activity events without touching durable project truth.
- Permission architecture & Icarus autonomy: governed autonomy at the launch boundary via explicit Icarus Mode (`--dangerously-skip-permissions`), tool risk classification (`READ_ONLY`, `MUTATING`, `HIGH_RISK`), audit history in `builder_permission_history`, and actionable guidance for blocked actions.
- Capacity & usage telemetry: authoritative provider-reported token tracking for Antigravity, heuristic ChatGPT token estimation (~4 chars/token heuristic) across 5-hour rolling and weekly windows, calibration reset controls, and prominent disclaimers.
- Desktop Builder Control Plane UI: `BuilderControlPlaneView` tab in `ProjectDetailView`, featuring persistent glowing amber/red Icarus warning banner, configuration controls, live streaming terminal with auto-scroll, telemetry cards, permission history, and packet inspector modal.
- Zero-quota test double (`fake-agy.cjs`): deterministic CLI simulation for CI and automated testing, covering models, effort, multi-turn conversation persistence, permission denial, and stderr warnings.

### Added - Stage 2 (Architecture Freeze and Git Boundaries)
- Explicit human-only architecture freeze: readiness-gated transition (`READY_TO_FREEZE` -> `FROZEN`) requiring all required contract artifacts to be substantively complete with zero machine bypass.
- Single-use server-owned `preview_id`: bound freeze confirmation to an authoritative database-backed preview ID, preventing client parameter tampering, stale previews, or duplicate confirmations.
- Non-circular Builder Packet and Manifest identity: deterministic `contract_fingerprint` derived from contract metadata and artifact hashes shared by both manifest and Builder packet; packet hash stored in manifest; `FreezeResult.builder_packet` matches durable `get_builder_packet()` byte-for-byte.
- Authoritative `FreezePreview` preflight with exact SHA-256 baseline fingerprints for all active contract artifacts, Git HEAD boundary, dirty working tree status, and Builder packet summary.
- Stale preview rejection: revalidates disk state and Git status upon human confirmation and rejects stale state with typed `STALE_FREEZE_PREVIEW`.
- Multi-resource crash-safe freeze transaction: isolated staging in `.coalition/architecture-versions/.staging-v1.0-<uuid>/`, explicit file handles with `sync_all()`, atomic directory promotion to `architecture-versions/v1.0/`, durable commit point updating `project.yaml` (`architecture_state: frozen`, `current_architecture_version: "1.0"`, `active_manifest_fingerprint: <sha256>`), and operational synchronization of `workflow_state`, `frozen_boundaries`, and `builder_epochs`.
- Strict snapshot integrity verification: checks manifest fingerprint, file sizes, individual SHA-256 hashes, repository containment, and actively rejects symlinks, junctions, reparse points, and unmanifested or unexpected files in `contract/`, returning typed `FROZEN_SNAPSHOT_CORRUPT`.
- Durable disk-backed drift restoration journal: authoritative lifecycle (`STAGED` -> `COMMITTING` -> verified & removed) using `.coalition/`-relative paths, directory synchronization (`durable_directory_sync`), and strict fail-closed recovery (`DRIFT_RESTORATION_RECOVERY_REQUIRED`).
- Untrusted journal input validation: re-validates all `RestorationOp` paths (`path`, `staged_file_rel`, `quarantine_dest_rel`) during recovery, strictly rejecting `..`, absolute paths, drive letters, UNC prefixes, control/NUL characters, unmanaged paths, and directory traversal outside `.coalition/recovery/.staging-restore-<id>/` or `.coalition/recovery/quarantine-*`.
- Real filesystem containment for recovery paths: enforces that parent chains and staged/quarantine targets cannot escape through symlinks, directory junctions, or Windows reparse points, and canonical paths resolve strictly within expected recovery directories.
- Active restoration staging preservation: defers staging cleanup until after journal read, parse, and validation, preserving active staging directories (`.staging-restore-<active_id>`) for rollback or replay while keeping forensic evidence intact when a journal is corrupt.
- Pre-mutation snapshot integrity verification: validates frozen snapshot against `active_manifest_fingerprint` prior to reading snapshot contents or performing any recovery mutations, failing closed with `FROZEN_SNAPSHOT_CORRUPT` without modifying active architecture files.
- Disk-journal project ownership validation: requires `journal.project_id == project_id` even when SQLite was deleted and disk journal is the sole authority, returning `DRIFT_RESTORATION_RECOVERY_REQUIRED` on mismatch.
- Rejection of unsupported COMMITTED phase: fails closed with `DRIFT_RESTORATION_RECOVERY_REQUIRED` on encountering unpersisted or untrusted `COMMITTED` phase in disk or SQLite journals.
- Full active artifact baseline revalidation at freeze commit: recomputes complete `compute_active_contract_baselines` right before the durable commit point, detecting modified, deleted, newly added canonical artifacts, and added/removed ADRs with `STALE_FREEZE_PREVIEW`.
- Atomic restoration journal updates: writes temporary journals (`drift-restoration-journal.tmp.<uuid>`), calls `sync_all`, replaces atomically via `replace_file_atomically`, and syncs the parent directory for both `STAGED` and `COMMITTING` phase updates.
- Strict dual-journal consensus: compares disk and SQLite restoration journals for `journal_id`, `project_id`, `architecture_version`, and operations, failing closed with `DRIFT_RESTORATION_RECOVERY_REQUIRED` on mismatch or unknown SQLite phases.
- Final commit preview freshness revalidation: revalidates Git HEAD, working-tree dirty status, and artifact baselines immediately preceding durable freeze commit point in `confirm_freeze`.
- Fail-closed `StartBuild` workflow guard: blocks transition when project record is missing (`PROJECT_NOT_FOUND`), repository is unavailable (`REPOSITORY_UNAVAILABLE`), restoration reconciliation fails (`DRIFT_RESTORATION_RECOVERY_REQUIRED`), snapshot is corrupt (`FROZEN_SNAPSHOT_CORRUPT`), or active contract has drifted (`FROZEN_CONTRACT_DRIFT_DETECTED`).
- Fail-aware Windows `durable_directory_sync`: opens directory handles with `FILE_FLAG_BACKUP_SEMANTICS` and write attribute access, tolerating only documented unsupported driver codes (`ERROR_INVALID_FUNCTION`, `ERROR_ACCESS_DENIED`, `ERROR_NOT_SUPPORTED`) while propagating all real I/O errors.
- Architecture version validation in read helpers: enforces version format and directory containment in `read_contract_manifest`, `get_drift_diff`, and artifact restoration entry points.
- Orphan staging directory cleanup: explicit discovery and cleanup of stale `.staging-restore-*` directories via `ArtifactManager::clean_stale_restore_staging`.
- Crash-safe batch restoration recovery: deterministic batch completion from disk journal alone if SQLite is deleted during `COMMITTING`, safe rollback if interrupted in `STAGED`, and zero swallowed errors on quarantine, copy, atomic replacement, or journal deletion.
- Contract drift verification before journal cleanup: runs authoritative drift check to ensure active contract matches frozen baseline before journal deletion.
- 6 Failure injection seams for batch restoration (`BeforeFirstRestoreMutation`, `AfterFirstRestoreMutation`, `MidRestoreBatch`, `DuringAddedArtifactQuarantine`, `AfterAllMutationsBeforeVerification`, `AfterVerificationBeforeJournalCleanup`) with verified convergence across all seams.
- Freeze failure injection seam and rollback verification: `MidDbTransaction` failure rolls back SQLite transaction (preview remains `PENDING`, workflow remains `READY_TO_FREEZE`), and `reconcile_freeze_state` idempotently completes operational reconciliation and records `ARCHITECTURE_FROZEN` activity event.
- Strict architecture version validation: `validate_architecture_version` enforces 1-32 chars of ASCII alphanumeric, dots, and hyphens, strictly rejecting path traversals (`..`), slashes, whitespace, and control chars.
- Authoritative drift path lockdown: rejects directory traversal (`..`), drive letters, paths outside the architecture allowlist, and un-drifted artifact paths.
- Builder workflow gate: `StartBuild` transition and `get_contract_drift` strictly reconcile any interrupted drift journals and block execution when contract drift is detected (`FROZEN_CONTRACT_DRIFT_DETECTED`).
- Hardened Git dirty state capture: exit status checking, unreadable file error propagation, and safe untracked link detection without following outside repository.
- Bounded Builder implementation packet: derived strictly from frozen snapshot with 200 KB prompt context budget, UTF-8 safe char boundary truncation (`floor_char_boundary`), rules, and truncation flag (`is_truncated: true`).
- Desktop UI controls: Freeze Architecture button gated by readiness, Freeze Confirmation Modal with server-owned preview ID authorization, Frozen status banner, Builder Packet viewer modal, and Drift Alert panel with Diff Inspector and single/batch Restore actions.
- SQLite wipe rehydration & crash recovery: independent operational rehydration, instant return of post-reconciliation `Frozen` state in `register_or_open_project`, and full recovery verified across all 6 freeze injection seams and restoration seams.
- Master Plan archiving and roadmap synchronization: archived `Coalition_Master_Implementation_Plan_v1.2.md` to `docs/archive/` and updated `Coalition_Master_Implementation_Plan_v1.3.md` marking Stage 2 COMPLETE and Stage 3 NEXT.

### Added - Stage 1 Final Durability and Identity Closure
- Authoritative backup preservation on failed promotion: removed source deletion from low-level failed `MoveFileExW` destination-absent path so that authoritative recovery backups (`project.yaml.bak.*`) remain byte-for-byte intact if promotion fails.
- Deferred stale-temp cleanup: artifact inspection remains strictly non-mutating through SQLite identity reconciliation, deferring stale temporary file cleanup until project open and layout validation are fully authorized and preventing identity conflicts from purging recovery evidence.
- Genuine crash-safe platform-native atomic replacement for `project.yaml` via Windows `ReplaceFileW` with `dwReplaceFlags = 0` (Microsoft documents `REPLACEFILE_WRITE_THROUGH` as unsupported for `ReplaceFileW`), an explicit same-directory backup (`project.yaml.bak.<uuid>`), post-call destination validation, and reconciliation of documented failure states.
- Preservation of recovery artifacts when replacement returns `RecoveryRequired`, making cleanup ownership explicit so callers never purge staged temporary files or backups during ambiguous failure states.
- Prevention of failed existing-file `ReplaceFileW` calls from falling through into `MoveFileExW` creation path, ensuring disappearance or replacement failure is not silently reinterpreted as file creation.
- Complete `.coalition/` layout validation restored on every project open, recreating missing standard subdirectories (`design/`, `implementation/`, `decisions/`, `architecture-versions/`, `changes/`, `reviews/`, `evidence/`) and rejecting symlink/junction reparse points escaping repository root before writes can occur.
- Hardened `ArtifactManager::initialize_or_load_project` to reject initialization with `RecoveryRequired` when backup or temp recovery evidence exists.
- Separation of artifact inspection from recovery mutation via `inspect_project_artifacts()`, enforcing strict identity-before-promotion in `ProjectService` so that recovery candidates are never promoted until identity is validated against SQLite registration history.
- Coalition flushes replacement data before invoking the native replacement primitive. On Windows it uses `ReplaceFileW` with a same-directory backup, reconciles documented failure states, and never authorizes recovery candidate promotion until project identity has been validated.
- Stale temp file cleanup and single valid backup recovery in `ArtifactManager`, with ambiguous/corrupted backups returning typed `ARTIFACT_RECOVERY_REQUIRED` (`ArtifactError::RecoveryRequired`).
- Strict UUID v4 enforcement in `ArtifactManager::validate_project_yaml` requiring RFC 4122 version 4 (`Version::Random`), rejecting Nil UUIDs, v1, v3, v5, and malformed strings.
- Exact Draft architecture version invariant: `architecture_state: draft` strictly requires `current_architecture_version` to be `None` / omitted (rejecting empty string `""` and whitespace `"   "` with `INVALID_ARCHITECTURE_STATE`).
- Operational-first project registration ordering querying SQLite canonical path before touching durable files; missing contract for registered path (Case D) returns typed `DURABLE_CONTRACT_MISSING` with zero durable identity mutation.
- Replacement failure injection test seam and comprehensive tests covering Windows native `ReplaceFileW` replacement, UUID v4 validation, draft invariants, backup recovery, stale temp cleanup, and Case D/E identity reconciliation.

### Added - Phase 1 (Coalition Core and Project Persistence)
- Authoritative Rust workflow state machine implementing all 19 V1 states with strict transition validation, pause/resume state preservation, atomic revision increments, and human architecture change requests from `PAUSED` and `INTERRUPTED` states when paused from post-freeze development.
- Durable `.coalition/` project contract hierarchy (`project.yaml`, `design/`, `implementation/`, `decisions/`, `architecture-versions/`, `changes/`, `reviews/`, `evidence/`) with complete layout validation against path traversal and symlink/junction reparse points.
- Portable, schema-versioned `project.yaml` with stable UUID v4 project identity, RFC3339 timestamp validation, non-empty project names, draft vs. frozen version invariants, and Windows-safe atomic replacement via staging and backup rollback.
- Forward SQLite migration 2 (`002_phase1_core_persistence`) creating `projects`, `workflow_state`, `activity_events`, and `app_settings` tables with enforced foreign key integrity.
- Transactional project registration and operational persistence wrapping `projects`, `workflow_state`, `activity_events`, and `app_settings` writes in an explicit SQLite transaction with rollback on failure.
- Deterministic project identity conflict and moved-repository reconciliation detecting collisions on `project_id` and canonical path.
- Non-fabricated durable contract representation (`Option<ProjectYaml>`) ensuring unavailable repositories display offline state without assuming or fabricating Draft contracts.
- Strict authoritative state parsing in `list_projects` surfacing corrupted state strings as typed `CORRUPTED_STATE` errors rather than swallowing them.
- Rehydration service restoring operational state from durable `.coalition/project.yaml` when local SQLite database is deleted or recreated.
- Git repository adapter extensions: repository root canonicalization, clean/dirty detection (staged, unstaged, untracked counts), 0-commit and detached HEAD handling without crashing.
- Structured Tauri IPC error boundary (`CommandError`) with typed error codes, human-readable messages, and safe structured details, eliminating string-matching control flow.
- Project dashboard frontend: empty state, project list with workflow and Git badges, project detail view with live status and refresh, recent activity timeline, open repository modal, and selective startup restoration avoiding broken auto-navigation for offline repositories.
- Complete 10-step end-to-end desktop lifecycle smoke test (`phase1_smoke_test.rs`).
- Preserved Phase 0 diagnostics view with React StrictMode singular stream listener fix.
- Comprehensive automated test suite across Rust unit/integration tests and frontend vitest tests.

### Added - Phase 0 (Bootstrap & Technical Proofs)
- Initial public repository baseline for `upstatedatasystems-llc/coalition`.
- Core invariants and agent instructions in `AGENTS.md`.
- Staged development skills in `.agents/skills/`.
- Tauri 2.x + React + TypeScript + Vite + Rust MSVC desktop scaffolding.
- Minimal Phase 0 technical diagnostic UI.
- Isolated Git adapter querying branch, status, diff, and commit info.
- Isolated Antigravity CLI adapter (`agy`) supporting detection, versioning, model enumeration, NDJSON stream parsing, and cancellation.
- Bounded process runner with streaming stdout/stderr and graceful/forced cancellation.
- SQLite versioned migration proof and operational schema foundation.
- Zero-quota `fake-agy` test double harness for deterministic CI/CD and unit testing.
- Initial CI workflow `.github/workflows/ci.yml`.
- Technical spike documentation for usage metrics, permissions, model switching, and process ownership.

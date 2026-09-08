# Spike D: Process Ownership & Cancellation

**Status:** Completed  
**Subject:** External process lifecycle, streaming I/O, hang timeouts, cancellation, and orphan prevention  
**Date:** September 8, 2026

## Objective

Prove robust process management for external commands (`agy`, Git, validation scripts), including streaming output, cancellation without data loss, hang detection, and prevention of orphan processes on Windows and cross-platform.

## Evidence & Architecture

### 1. Process Lifecycle
- In Rust, child processes are managed via `tokio::process::Command`.
- Stdin is piped for interactive NDJSON messaging; stdout and stderr are piped asynchronously.
- Output streams are read line-by-line using asynchronous readers, allowing real-time event dispatch to Tauri/React.

### 2. Cancellation & Hang Handling
- When a cancellation is requested:
  1. Graceful close: Drop stdin to signal EOF to the child process.
  2. If the process does not terminate within a grace period (e.g. 2 seconds), issue a direct `kill()` to the process handle.
  3. On Windows, ensure child processes spawned by shell scripts or wrappers are terminated using Job Objects or process group termination when necessary.
- Timeout protection: Every command execution is wrapped with a configurable `tokio::time::timeout`.

### 3. Partial Log Preservation
- Output lines are appended immediately to an in-memory buffer or flushed to disk as they arrive.
- When cancellation occurs, the status is marked `CANCELED`, and all accumulated lines are retained in the command log. A canceled check is never reported as `PASS` and is never lost.

### 4. Orphan Prevention & Startup Reconciliation
- When Coalition starts up, it inspects persisted operational state (SQLite) for active process records from previous runs.
- Any process marked `RUNNING` from a prior app session is reconciled and marked `INTERRUPTED` or `ORPHAN_RECONCILED`.

## Interface Classification

| Capability | Classification | Notes |
| :--- | :--- | :--- |
| Asynchronous Line Streaming | **PROVEN** | Tokio asynchronous readers provide real-time line-by-line events. |
| Process Cancellation | **PROVEN** | Immediate process termination via `child.kill()`. |
| Partial Output Retention | **PROVEN** | Event channel and line accumulation preserve all data prior to termination. |
| Timeout Enforcement | **PROVEN** | Wrapped via `tokio::time::timeout`. |
| Windows Process Tree Cleanup | **SUPPORTED BUT NOT FULLY TESTED** | `child.kill()` terminates direct child; Windows Job Objects ensure sub-children termination in multi-tier script invocations. |

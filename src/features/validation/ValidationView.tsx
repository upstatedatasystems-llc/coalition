import React, { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  ValidationConfig,
  ValidationRunRecord,
} from '../../types';

interface ValidationViewProps {
  projectId: string;
  projectName: string;
  workflowState: string;
  onRefreshProject: () => Promise<void>;
  onNavigateToBuilder?: () => void;
}

export const ValidationView: React.FC<ValidationViewProps> = ({
  projectId,
  workflowState,
  onRefreshProject,
  onNavigateToBuilder,
}) => {
  const [config, setConfig] = useState<ValidationConfig | null>(null);
  const [activeRun, setActiveRun] = useState<ValidationRunRecord | null>(null);
  const [selectedRun, setSelectedRun] = useState<ValidationRunRecord | null>(null);
  const [history, setHistory] = useState<ValidationRunRecord[]>([]);
  const [liveLogs, setLiveLogs] = useState<string[]>([]);
  const [selectedCommandLog, setSelectedCommandLog] = useState<string | null>(null);

  const [isLoading, setIsLoading] = useState<boolean>(true);
  const [isStarting, setIsStarting] = useState<boolean>(false);
  const [isStoppingCurrent, setIsStoppingCurrent] = useState<boolean>(false);
  const [isStoppingAll, setIsStoppingAll] = useState<boolean>(false);
  const [isSubmittingReview, setIsSubmittingReview] = useState<boolean>(false);

  // Override dialog
  const [overrideModalOpen, setOverrideModalOpen] = useState<boolean>(false);
  const [overrideRunId, setOverrideRunId] = useState<string | null>(null);
  const [overrideReason, setOverrideReason] = useState<string>('');
  const [isSubmittingOverride, setIsSubmittingOverride] = useState<boolean>(false);

  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [successMessage, setSuccessMessage] = useState<string | null>(null);

  const logsEndRef = useRef<HTMLDivElement>(null);

  const loadData = useCallback(async () => {
    try {
      setErrorMessage(null);
      const [cfg, active, hist] = await Promise.all([
        invoke<ValidationConfig>('get_validation_config', { projectId }),
        invoke<ValidationRunRecord | null>('get_active_validation_run', { projectId }),
        invoke<ValidationRunRecord[]>('get_validation_history', { projectId, limit: 20 }),
      ]);
      setConfig(cfg);
      setActiveRun(active);
      setHistory(hist);
      if (!selectedRun && hist.length > 0) {
        setSelectedRun(hist[0]);
      }
    } catch (e: unknown) {
      const err = e as { message?: string };
      setErrorMessage(err.message || String(e));
    } finally {
      setIsLoading(false);
    }
  }, [projectId, selectedRun]);

  useEffect(() => {
    loadData();
  }, [loadData]);

  // Listen to live validation events & terminal output
  useEffect(() => {
    let unlisteners: (() => void)[] = [];

    const setupListeners = async () => {
      const u1 = await listen<any>('validation://status', (evt) => {
        const payload = evt.payload;
        if (payload?.project_id !== projectId) return;
        setActiveRun((prev) =>
          prev
            ? {
                ...prev,
                status: payload.status,
                commands: payload.commands ?? prev.commands,
              }
            : payload.status === 'RUNNING' || payload.status === 'QUEUED'
            ? {
                run_id: payload.run_id,
                project_id: payload.project_id,
                architecture_version: payload.architecture_version ?? '',
                epoch_id: payload.epoch_id ?? null,
                trigger_source: payload.trigger_source ?? 'MANUAL',
                status: payload.status,
                is_gate_passed: false,
                has_override: false,
                git_head: payload.git_head ?? null,
                git_dirty_fingerprint: null,
                config_fingerprint: null,
                log_path: null,
                started_at: new Date().toISOString(),
                completed_at: null,
                duration_ms: 0,
                commands: payload.commands ?? [],
              }
            : null
        );
      });

      const u2 = await listen<any>('validation://command-start', (evt) => {
        const payload = evt.payload;
        if (payload?.project_id !== projectId) return;
        setLiveLogs((prev) => [
          ...prev.slice(-1000),
          `\n>>> [${new Date().toLocaleTimeString()}] Running: ${payload.name || payload.command_id}...\n`,
        ]);
      });

      const u3 = await listen<any>('validation://output', (evt) => {
        const payload = evt.payload;
        if (payload?.project_id !== projectId) return;
        if (payload?.line !== undefined) {
          setLiveLogs((prev) => [...prev.slice(-1500), payload.line]);
        }
      });

      const u4 = await listen<any>('validation://command-finish', (evt) => {
        const payload = evt.payload;
        if (payload?.project_id !== projectId) return;
        setLiveLogs((prev) => [
          ...prev.slice(-1000),
          `<<< [${new Date().toLocaleTimeString()}] Command ${payload.command_id} finished with status: ${payload.status} (exit: ${payload.exit_code ?? 'n/a'}) in ${payload.duration_ms}ms\n`,
        ]);
      });

      const u5 = await listen<any>('validation://finish', (evt) => {
        const payload = evt.payload;
        if (payload?.project_id !== projectId) return;
        setActiveRun(null);
        setLiveLogs((prev) => [
          ...prev.slice(-1000),
          `\n=== [${new Date().toLocaleTimeString()}] Validation Run ${payload.run_id} finished: ${payload.status} (Gate: ${payload.is_gate_passed ? 'PASSED' : 'FAILED'}) in ${payload.duration_ms}ms ===\n`,
        ]);
        loadData();
        onRefreshProject();
      });

      // Backward-compatible listeners
      const u6 = await listen<any>('coalition:validation-event', (evt) => {
        const payload = evt.payload;
        if (payload?.project_id !== projectId) return;
        if (payload.event_type === 'RUN_COMPLETED') {
          loadData();
          onRefreshProject();
        }
      });

      const u7 = await listen<any>('coalition:validation-line', (evt) => {
        const payload = evt.payload;
        if (payload?.project_id !== projectId) return;
        if (payload?.line) {
          setLiveLogs((prev) => [...prev.slice(-1500), payload.line]);
        }
      });

      unlisteners = [u1, u2, u3, u4, u5, u6, u7];
    };

    setupListeners();

    return () => {
      unlisteners.forEach((u) => u());
    };
  }, [projectId, loadData, onRefreshProject]);

  // Auto-scroll terminal logs
  useEffect(() => {
    logsEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [liveLogs]);

  const handleStartValidation = async () => {
    setIsStarting(true);
    setErrorMessage(null);
    setSuccessMessage(null);
    setLiveLogs([]);
    try {
      const run = await invoke<ValidationRunRecord>('start_validation_run', {
        payload: {
          projectId,
          trigger: 'MANUAL',
        },
      });
      setActiveRun(run);
      setSuccessMessage(`Validation run ${run.run_id} started.`);
      await onRefreshProject();
    } catch (e: unknown) {
      const err = e as { message?: string };
      setErrorMessage(err.message || String(e));
    } finally {
      setIsStarting(false);
      loadData();
    }
  };

  const handleStopCurrent = async () => {
    if (!activeRun) return;
    setIsStoppingCurrent(true);
    setErrorMessage(null);
    try {
      await invoke('stop_validation_command', {
        runId: activeRun.run_id,
      });
      setSuccessMessage('Stop requested for currently executing command. Continuing remaining suite...');
    } catch (e: unknown) {
      const err = e as { message?: string };
      setErrorMessage(err.message || String(e));
    } finally {
      setIsStoppingCurrent(false);
      loadData();
    }
  };

  const handleStopAll = async () => {
    if (!activeRun) return;
    setIsStoppingAll(true);
    setErrorMessage(null);
    try {
      await invoke('stop_validation_run', {
        runId: activeRun.run_id,
      });
      setSuccessMessage('Stop all requested. Canceling run and skipping remaining commands.');
    } catch (e: unknown) {
      const err = e as { message?: string };
      setErrorMessage(err.message || String(e));
    } finally {
      setIsStoppingAll(false);
      loadData();
    }
  };

  const handleOpenOverride = (runId: string) => {
    setOverrideRunId(runId);
    setOverrideReason('');
    setOverrideModalOpen(true);
  };

  const handleConfirmOverride = async () => {
    if (!overrideRunId || !overrideReason.trim()) return;
    setIsSubmittingOverride(true);
    setErrorMessage(null);
    try {
      await invoke('override_validation_gate', {
        payload: {
          projectId,
          runId: overrideRunId,
          reason: overrideReason.trim(),
        },
      });
      setSuccessMessage(`Validation gate override granted for run ${overrideRunId}.`);
      setOverrideModalOpen(false);
      await loadData();
      await onRefreshProject();
    } catch (e: unknown) {
      const err = e as { message?: string };
      setErrorMessage(err.message || String(e));
    } finally {
      setIsSubmittingOverride(false);
    }
  };

  const handleSubmitForReview = async () => {
    setIsSubmittingReview(true);
    setErrorMessage(null);
    setSuccessMessage(null);
    try {
      await invoke('submit_for_review', { projectId });
      setSuccessMessage('Validation gate passed! Project successfully submitted for review.');
      await onRefreshProject();
      await loadData();
    } catch (e: unknown) {
      const err = e as { message?: string; code?: string };
      if (err.code === 'VALIDATION_GATE_BLOCKED') {
        setErrorMessage(`Submission Blocked by Validation Gate: ${err.message}`);
      } else {
        setErrorMessage(err.message || String(e));
      }
    } finally {
      setIsSubmittingReview(false);
    }
  };

  const [isStartingDiagnosticTurn, setIsStartingDiagnosticTurn] = useState<boolean>(false);

  const handleStartDiagnosticTurn = async (runId: string) => {
    setIsStartingDiagnosticTurn(true);
    setErrorMessage(null);
    setSuccessMessage(null);
    try {
      await invoke('start_builder_diagnostic_turn', {
        payload: {
          projectId,
          validationRunId: runId,
        },
      });
      setSuccessMessage('Builder diagnostic turn started successfully!');
      await onRefreshProject();
      await loadData();
      if (onNavigateToBuilder) {
        onNavigateToBuilder();
      }
    } catch (e: unknown) {
      const err = e as { message?: string };
      setErrorMessage(err.message || String(e));
    } finally {
      setIsStartingDiagnosticTurn(false);
    }
  };

  if (isLoading) {
    return <div className="loading-state">Loading validation configuration and runs...</div>;
  }

  const isConfigured = config && config.enabled && config.commands.length > 0;
  const isRunning = !!activeRun;

  return (
    <div className="validation-control-container" data-testid="validation-view">
      {errorMessage && (
        <div className="banner error-banner" role="alert" data-testid="validation-error-banner">
          <strong>Error:</strong> {errorMessage}
          <button className="banner-dismiss" onClick={() => setErrorMessage(null)}>
            ×
          </button>
        </div>
      )}

      {successMessage && (
        <div className="banner success-banner" role="status" data-testid="validation-success-banner">
          {successMessage}
          <button className="banner-dismiss" onClick={() => setSuccessMessage(null)}>
            ×
          </button>
        </div>
      )}

      {/* Top Governance Bar */}
      <div className="governance-card">
        <div className="governance-card-header">
          <div>
            <h2>Validation Engine</h2>
            <p className="section-subtitle">
              Hermetic multi-command validation gate and regression detection
            </p>
          </div>
          <div className="action-buttons-row">
            <button
              className="primary-btn"
              onClick={handleStartValidation}
              disabled={isRunning || isStarting || !isConfigured}
              data-testid="start-validation-btn"
            >
              {isStarting ? 'Starting...' : isRunning ? 'Validating...' : 'Run Validation'}
            </button>

            {isRunning && (
              <>
                <button
                  className="secondary-btn warning-btn"
                  onClick={handleStopCurrent}
                  disabled={isStoppingCurrent}
                  data-testid="stop-current-btn"
                >
                  {isStoppingCurrent ? 'Stopping...' : 'Stop Current'}
                </button>
                <button
                  className="danger-btn"
                  onClick={handleStopAll}
                  disabled={isStoppingAll}
                  data-testid="stop-all-btn"
                >
                  {isStoppingAll ? 'Stopping All...' : 'Stop All'}
                </button>
              </>
            )}

            <button
              className="success-btn"
              onClick={handleSubmitForReview}
              disabled={isRunning || isSubmittingReview || workflowState === 'WAITING_FOR_REVIEW'}
              data-testid="submit-review-btn"
            >
              {isSubmittingReview ? 'Submitting...' : 'Submit for Review →'}
            </button>
          </div>
        </div>

        {/* Configuration Summary */}
        <div className="validation-config-summary">
          {!isConfigured ? (
            <div className="info-box warning-box" data-testid="validation-unconfigured-note">
              <strong>Validation Not Configured:</strong> No active commands found in{' '}
              <code>.coalition/implementation/validation.yaml</code>. Validation checks are currently
              non-blocking.
            </div>
          ) : (
            <div className="config-grid">
              <div className="config-item">
                <span className="config-label">Policy:</span>
                <span className="config-value">
                  {config.policy.gate_review_on_required_failure
                    ? 'Required Gate (Blocks Review on Failure)'
                    : 'Diagnostic Only'}
                </span>
              </div>
              <div className="config-item">
                <span className="config-label">Configured Suite:</span>
                <span className="config-value">
                  {config.commands.length} command{config.commands.length === 1 ? '' : 's'} (
                  {config.commands.filter((c) => c.required !== false).length} required)
                </span>
              </div>
              <div className="config-item">
                <span className="config-label">Grace Period:</span>
                <span className="config-value">{config.policy.grace_period_seconds}s</span>
              </div>
            </div>
          )}
        </div>
      </div>

      {/* Active Validation Execution / Terminal */}
      {isRunning && (
        <div className="active-run-card" data-testid="active-validation-run">
          <div className="active-run-header">
            <div>
              <h3>
                Active Run: <code>{activeRun.run_id}</code>
              </h3>
              <span className={`badge badge-${activeRun.status.toLowerCase()}`}>
                {activeRun.status}
              </span>
            </div>
          </div>

          {/* Live Command Stream Checklist */}
          <div className="command-checklist">
            {activeRun.commands.map((cmd) => (
              <div key={cmd.command_id} className={`command-status-row status-${cmd.status.toLowerCase()}`}>
                <span className="cmd-badge">{cmd.status}</span>
                <span className="cmd-name">{cmd.name}</span>
                <code className="cmd-line">{cmd.command}</code>
                {cmd.duration_ms > 0 && <span className="cmd-duration">{cmd.duration_ms}ms</span>}
              </div>
            ))}
          </div>

          {/* Live Streaming Terminal */}
          <div className="terminal-container" data-testid="validation-terminal">
            <div className="terminal-header">
              <span>Execution Output</span>
              <span className="terminal-hint">Dual disk-streamed & bounded</span>
            </div>
            <pre className="terminal-body">
              {liveLogs.length === 0 ? 'Waiting for command output...' : liveLogs.join('\n')}
              <div ref={logsEndRef} />
            </pre>
          </div>
        </div>
      )}

      {/* Latest Run & History Section */}
      <div className="validation-history-section">
        <h3>Validation History</h3>
        {history.length === 0 ? (
          <div className="empty-history-note" data-testid="no-runs-message">
            No validation runs recorded yet. Click "Run Validation" to execute configured checks.
          </div>
        ) : (
          <div className="history-grid">
            <div className="history-list">
              {history.map((run) => (
                <div
                  key={run.run_id}
                  className={`history-item ${selectedRun?.run_id === run.run_id ? 'selected' : ''}`}
                  onClick={() => {
                    setSelectedRun(run);
                    setSelectedCommandLog(null);
                  }}
                  data-testid={`run-item-${run.run_id}`}
                >
                  <div className="history-item-top">
                    <span className={`badge badge-${run.status.toLowerCase()}`}>
                      {run.status}
                    </span>
                    <span className="run-trigger">{run.trigger_source}</span>
                    <span className="run-time">
                      {new Date(run.started_at).toLocaleTimeString()}
                    </span>
                  </div>
                  <div className="history-item-sub">
                    <code>{run.run_id}</code>
                    {run.has_override && <span className="override-tag">OVERRIDDEN</span>}
                    <span>{run.duration_ms}ms</span>
                  </div>
                </div>
              ))}
            </div>

            {/* Run Detail Panel */}
            {selectedRun && (
              <div className="run-detail-panel" data-testid="run-detail-panel">
                <div className="run-detail-header">
                  <div>
                    <h4>Run {selectedRun.run_id}</h4>
                    <p className="detail-meta">
                      Arch: {selectedRun.architecture_version} | Trigger: {selectedRun.trigger_source}
                    </p>
                  </div>
                  <div className="detail-header-actions">
                    {(selectedRun.status === 'FAIL' || selectedRun.status === 'TIMEOUT') && (
                      <button
                        className="primary-btn diagnostic-turn-btn"
                        onClick={() => handleStartDiagnosticTurn(selectedRun.run_id)}
                        disabled={isStartingDiagnosticTurn}
                        data-testid="send-diagnostics-builder-btn"
                      >
                        {isStartingDiagnosticTurn ? 'Starting Builder Turn...' : 'Send Diagnostics to Builder'}
                      </button>
                    )}
                    {selectedRun.status === 'FAIL' && !selectedRun.has_override && (
                      <button
                        className="warning-btn"
                        onClick={() => handleOpenOverride(selectedRun.run_id)}
                        data-testid="override-gate-btn"
                      >
                        Override Gate
                      </button>
                    )}
                    {selectedRun.has_override && (
                      <span className="badge badge-warning" data-testid="override-indicator">
                        Gate Overridden by Human
                      </span>
                    )}
                  </div>
                </div>

                {/* Fingerprint Binding Verification Info */}
                <div className="fingerprint-bindings-box">
                  <div>
                    <strong>Git HEAD:</strong> <code>{selectedRun.git_head?.slice(0, 10) || 'n/a'}</code>
                  </div>
                  <div>
                    <strong>Implementation Fingerprint:</strong>{' '}
                    <code>{selectedRun.git_dirty_fingerprint?.slice(0, 16) || 'clean'}...</code>
                  </div>
                  <div>
                    <strong>Gate Decision:</strong>{' '}
                    <span className={selectedRun.is_gate_passed ? 'text-success' : 'text-danger'}>
                      {selectedRun.is_gate_passed ? 'PASSED' : 'BLOCKED'}
                    </span>
                  </div>
                </div>

                {/* Command Breakdown */}
                <div className="command-breakdown">
                  <h5>Command Results</h5>
                  <div className="command-table">
                    {selectedRun.commands.map((cmd) => (
                      <div
                        key={cmd.command_id}
                        className={`cmd-result-row ${selectedCommandLog === cmd.command_id ? 'active' : ''}`}
                        onClick={() =>
                          setSelectedCommandLog(
                            selectedCommandLog === cmd.command_id ? null : cmd.command_id
                          )
                        }
                      >
                        <span className={`badge badge-${cmd.status.toLowerCase()}`}>
                          {cmd.status}
                        </span>
                        <div className="cmd-meta">
                          <strong>{cmd.name}</strong>
                          <code>{cmd.command}</code>
                        </div>
                        <span className="cmd-duration">{cmd.duration_ms}ms</span>
                        <span className="cmd-expand-icon">
                          {selectedCommandLog === cmd.command_id ? '▲' : '▼'}
                        </span>
                      </div>
                    ))}
                  </div>

                  {/* Expanded Command Output Preview */}
                  {selectedCommandLog && (
                    <div className="command-log-preview">
                      {(() => {
                        const cmd = selectedRun.commands.find((c) => c.command_id === selectedCommandLog);
                        if (!cmd) return null;
                        const output = cmd.stdout_preview || cmd.stderr_preview || 'No output recorded.';
                        return (
                          <pre className="terminal-body log-preview">
                            {output}
                          </pre>
                        );
                      })()}
                    </div>
                  )}
                </div>
              </div>
            )}
          </div>
        )}
      </div>

      {/* Human Gate Override Modal */}
      {overrideModalOpen && (
        <div className="modal-backdrop" data-testid="override-modal">
          <div className="modal-content override-modal">
            <h3>Authoritative Gate Override</h3>
            <p className="warning-text">
              <strong>Human Authority Directive:</strong> You are authorizing an explicit override of
              a failed validation gate for run <code>{overrideRunId}</code>. This decision is permanently
              recorded in the immutable project audit log.
            </p>

            <div className="form-group">
              <label htmlFor="override-reason">
                Required Rationale / Justification <span className="required-star">*</span>
              </label>
              <textarea
                id="override-reason"
                className="input-textarea"
                rows={4}
                placeholder="Explain why this validation failure is safe to bypass for review..."
                value={overrideReason}
                onChange={(e) => setOverrideReason(e.target.value)}
                data-testid="override-rationale-input"
              />
            </div>

            <div className="modal-actions">
              <button
                className="secondary-btn"
                onClick={() => setOverrideModalOpen(false)}
                disabled={isSubmittingOverride}
              >
                Cancel
              </button>
              <button
                className="danger-btn"
                onClick={handleConfirmOverride}
                disabled={!overrideReason.trim() || isSubmittingOverride}
                data-testid="confirm-override-btn"
              >
                {isSubmittingOverride ? 'Authorizing...' : 'Authorize Override'}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};

import React, { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  ModelInfo,
  BuilderPacket,
  DriftReport,
  IcarusState,
  UsageTelemetryReport,
  PermissionRecord,
  BuilderSessionRecord,
  BuilderTurnResponse,
  BuilderEventRecord,
  ChatGptUsageSummary,
} from '../../types';

interface BuilderControlPlaneViewProps {
  projectId: string;
  projectName: string;
  workflowState: string;
  onRefreshProject: () => Promise<void>;
}

interface StreamEventPayload {
  session_id: string;
  project_id: string;
  event: Record<string, unknown>;
}

export const BuilderControlPlaneView: React.FC<BuilderControlPlaneViewProps> = ({
  projectId,
  projectName,
  workflowState,
  onRefreshProject,
}) => {
  // Model & Execution State
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [selectedModel, setSelectedModel] = useState<string>('');
  const [modelError, setModelError] = useState<string | null>(null);
  const [effort, setEffort] = useState<'low' | 'medium' | 'high'>('medium');

  // Status & Telemetry
  const [builderPacket, setBuilderPacket] = useState<BuilderPacket | null>(null);
  const [driftReport, setDriftReport] = useState<DriftReport | null>(null);
  const [icarusState, setIcarusState] = useState<IcarusState | null>(null);
  const [telemetry, setTelemetry] = useState<UsageTelemetryReport | null>(null);
  const [permissionHistory, setPermissionHistory] = useState<PermissionRecord[]>([]);
  const [sessions, setSessions] = useState<BuilderSessionRecord[]>([]);

  // Execution & Live Output
  const [isRunning, setIsRunning] = useState<boolean>(false);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [activeRunIcarus, setActiveRunIcarus] = useState<boolean | null>(null);
  const [terminalLogs, setTerminalLogs] = useState<string[]>([]);
  const [lastResponse, setLastResponse] = useState<BuilderTurnResponse | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  // Modals & Controls
  const [showIcarusModal, setShowIcarusModal] = useState<boolean>(false);
  const [showPacketModal, setShowPacketModal] = useState<boolean>(false);
  const [showCalibrationModal, setShowCalibrationModal] = useState<boolean>(false);
  const [calTokens, setCalTokens] = useState<number>(1000);
  const [calChars, setCalChars] = useState<number>(4000);
  const [isLoading, setIsLoading] = useState<boolean>(false);

  const terminalEndRef = useRef<HTMLDivElement>(null);

  const isFrozen =
    workflowState === 'FROZEN' ||
    workflowState === 'BUILDING' ||
    workflowState === 'VALIDATING' ||
    workflowState === 'WAITING_FOR_REVIEW' ||
    workflowState === 'CORRECTIONS_REQUIRED' ||
    workflowState === 'REVIEW_ACCEPTED' ||
    workflowState === 'FINAL_VALIDATION' ||
    workflowState === 'READY_FOR_HUMAN_REVIEW' ||
    workflowState === 'HUMAN_ACCEPTED';

  // Load initial data
  useEffect(() => {
    loadAllData();
  }, [projectId]);

  // Terminal auto-scroll
  useEffect(() => {
    if (typeof terminalEndRef.current?.scrollIntoView === 'function') {
      terminalEndRef.current.scrollIntoView({ behavior: 'smooth' });
    }
  }, [terminalLogs]);

  // Listen to builder streaming events
  useEffect(() => {
    let canceled = false;
    let unlisten: (() => void) | null = null;

    listen<StreamEventPayload>('coalition:builder-event', (event) => {
      if (event.payload.project_id === projectId) {
        const raw = event.payload.event;
        formatAndAppendEvent(raw);
      }
    })
      .then((fn) => {
        if (canceled) {
          fn();
        } else {
          unlisten = fn;
        }
      })
      .catch((err) => {
        console.error('Failed to bind coalition:builder-event listener:', err);
      });

    return () => {
      canceled = true;
      if (unlisten) unlisten();
    };
  }, [projectId]);

  const loadAllData = async () => {
    setIsLoading(true);
    try {
      await Promise.allSettled([
        loadModels(),
        loadBuilderPacket(),
        loadDriftReport(),
        loadIcarusState(),
        loadTelemetry(),
        loadPermissionHistory(),
        loadSessions(),
      ]);
    } finally {
      setIsLoading(false);
    }
  };

  const loadModels = async () => {
    try {
      setModelError(null);
      const availableModels = await invoke<ModelInfo[]>('list_builder_models');
      setModels(availableModels);
      if (availableModels.length > 0) {
        if (!availableModels.some((m) => m.id === selectedModel)) {
          setSelectedModel(availableModels[0].id);
        }
      } else {
        setSelectedModel('');
        setModelError("No Antigravity models discovered. Run 'agy models' in terminal to verify CLI setup.");
      }
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : JSON.stringify(err);
      console.warn('Could not load builder models:', err);
      setModelError(`Live model discovery failed: ${msg}`);
      setModels([]);
      setSelectedModel('');
    }
  };

  const loadBuilderPacket = async () => {
    try {
      const packet = await invoke<BuilderPacket>('get_builder_packet', {
        projectId,
        version: null,
      });
      setBuilderPacket(packet);
    } catch (_) {
      setBuilderPacket(null);
    }
  };

  const loadDriftReport = async () => {
    try {
      const report = await invoke<DriftReport>('get_contract_drift', { projectId });
      setDriftReport(report);
    } catch (_) {
      setDriftReport(null);
    }
  };

  const loadIcarusState = async () => {
    try {
      const state = await invoke<IcarusState>('get_icarus_state', { projectId });
      setIcarusState(state);
    } catch (err) {
      console.error('Could not load Icarus state:', err);
    }
  };

  const loadTelemetry = async () => {
    try {
      const report = await invoke<UsageTelemetryReport>('get_usage_telemetry', { projectId });
      setTelemetry(report);
    } catch (err) {
      console.error('Could not load usage telemetry:', err);
    }
  };

  const loadPermissionHistory = async () => {
    try {
      const history = await invoke<PermissionRecord[]>('get_permission_history', {
        projectId,
        limit: 50,
      });
      setPermissionHistory(history);
    } catch (err) {
      console.error('Could not load permission history:', err);
    }
  };

  const loadSessions = async () => {
    try {
      const sess = await invoke<BuilderSessionRecord[]>('list_builder_sessions', {
        projectId,
        limit: 10,
      });
      setSessions(sess);

      // Check running session
      const runningSession = sess.find((s) => s.status === 'RUNNING');
      if (runningSession) {
        setIsRunning(true);
        setActiveSessionId(runningSession.session_id);
        setActiveRunIcarus(runningSession.icarus_mode);
      } else {
        setIsRunning(false);
        setActiveSessionId(null);
        setActiveRunIcarus(null);
      }

      // Restore past terminal events from latest session if terminal is empty
      if (sess.length > 0 && terminalLogs.length === 0) {
        const latest = sess[0];
        try {
          const pastEvents = await invoke<BuilderEventRecord[]>('get_builder_events', {
            sessionId: latest.session_id,
          });
          if (pastEvents.length > 0) {
            const restoredLines = pastEvents.map((e) => {
              const time = new Date(e.timestamp).toLocaleTimeString();
              return `[${time}] ${e.event_type}: ${e.content || e.details_json || e.state || ''}`;
            });
            setTerminalLogs(restoredLines.slice(-1000));
          }
        } catch (_) {
          // Non-blocking history restore
        }
      }
    } catch (err) {
      console.error('Could not load builder sessions:', err);
    }
  };

  const formatAndAppendEvent = (evt: Record<string, unknown>) => {
    const eventType = String(evt.event || 'event');
    let line = `[${new Date().toLocaleTimeString()}] `;

    if (eventType === 'init') {
      const initData = evt.init as Record<string, unknown> | undefined;
      line += `🚀 Initialized Antigravity session (Model: ${initData?.model || 'default'}, Effort: ${initData?.effort || 'default'})`;
    } else if (eventType === 'step_update') {
      const step = evt.step_update as Record<string, unknown> | undefined;
      if (step?.step_type === 'agent_response' && step?.text_delta) {
        line += `🤖 Agent: ${step.text_delta}`;
      } else if (step?.step_type === 'user_input') {
        line += `👤 Human prompt submitted (Step #${step?.step_index ?? 0})`;
      } else {
        line += `⚙️ Step ${step?.step_index ?? ''}: ${step?.step_type || 'update'} (${step?.state || ''})`;
      }
    } else if (eventType === 'tool_execution') {
      const tool = evt.tool_execution as Record<string, unknown> | undefined;
      line += `🔧 Tool: ${tool?.tool_name || 'external'} → ${tool?.status || 'executing'}`;
    } else if (eventType === 'result') {
      const result = evt.result as Record<string, unknown> | undefined;
      line += `🏁 Turn Finished [${result?.status || 'DONE'}] (Turns: ${result?.num_turns ?? 1}, Duration: ${result?.duration_seconds ?? 0}s)`;
    } else {
      line += `${eventType}: ${JSON.stringify(evt)}`;
    }

    setTerminalLogs((prev) => {
      const next = [...prev, line];
      return next.length > 1000 ? next.slice(next.length - 1000) : next;
    });
  };

  const handleRunTurn = async () => {
    if (!selectedModel) {
      setErrorMessage('Please select a valid Antigravity model before running.');
      return;
    }

    setErrorMessage(null);
    setIsRunning(true);
    setLastResponse(null);
    const runIcarus = Boolean(icarusState?.enabled);
    setActiveRunIcarus(runIcarus);

    setTerminalLogs((prev) => {
      const next = [
        ...prev,
        `--- Starting Governed Builder Turn [${new Date().toLocaleTimeString()}] Model: ${selectedModel} (Effort: ${effort})${runIcarus ? ' [ICARUS MODE]' : ''} ---`,
      ];
      return next.length > 1000 ? next.slice(next.length - 1000) : next;
    });

    try {
      const resp = await invoke<BuilderTurnResponse>('start_builder_turn', {
        payload: {
          projectId,
          model: selectedModel,
          effort,
        },
      });

      setLastResponse(resp);
      await loadAllData();
      await onRefreshProject();

      if (resp.status === 'FAILED' || resp.status === 'ERROR') {
        setErrorMessage(resp.text_response || resp.stderr || 'Builder turn reported an error.');
      }
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : JSON.stringify(err);
      setErrorMessage(msg);
      setTerminalLogs((prev) => {
        const next = [...prev, `❌ Turn Execution Error: ${msg}`];
        return next.length > 1000 ? next.slice(next.length - 1000) : next;
      });
    } finally {
      setIsRunning(false);
      setActiveSessionId(null);
      setActiveRunIcarus(null);
    }
  };

  const handleCancelTurn = async () => {
    try {
      setTerminalLogs((prev) => {
        const next = [...prev, `🛑 Cancellation requested by user...`];
        return next.length > 1000 ? next.slice(next.length - 1000) : next;
      });
      await invoke('cancel_builder_turn', {
        projectId,
        sessionId: activeSessionId,
      });
      setIsRunning(false);
      setActiveSessionId(null);
      setActiveRunIcarus(null);
      await loadSessions();
    } catch (err: unknown) {
      console.error('Failed to cancel turn:', err);
    }
  };

  const handleToggleIcarus = async (enable: boolean) => {
    try {
      const updated = await invoke<IcarusState>('set_icarus_mode', {
        projectId,
        enabled: enable,
      });
      setIcarusState(updated);
      setShowIcarusModal(false);
    } catch (err: unknown) {
      setErrorMessage(`Failed to change Icarus mode: ${err instanceof Error ? err.message : JSON.stringify(err)}`);
    }
  };

  const handleResetChatGpt = async () => {
    try {
      await invoke('reset_chatgpt_usage', { projectId });
      await loadTelemetry();
    } catch (err: unknown) {
      console.error('Failed to reset ChatGPT usage:', err);
    }
  };

  const handleCalibrateChatGpt = async (e: React.FormEvent) => {
    e.preventDefault();
    if (calTokens <= 0 || calChars <= 0) return;
    try {
      const summary = await invoke<ChatGptUsageSummary>('calibrate_chatgpt_usage', {
        projectId,
        sampleTokens: calTokens,
        sampleChars: calChars,
      });
      if (telemetry) {
        setTelemetry({ ...telemetry, chatgpt_estimated_usage: summary });
      }
      setShowCalibrationModal(false);
    } catch (err: unknown) {
      console.error('Failed to calibrate ChatGPT estimator:', err);
    }
  };

  const latestSession = sessions.length > 0 ? sessions[0] : null;

  return (
    <div className="builder-control-plane" data-testid="builder-control-plane">
      {/* 1. Active-Run Icarus Persistent Warning Banner */}
      {(activeRunIcarus || (isRunning && icarusState?.enabled)) && (
        <div className="icarus-warning-banner active-run-icarus-banner" role="alert" data-testid="active-run-icarus-banner">
          <div className="icarus-banner-content">
            <span className="icarus-icon">⚡</span>
            <div className="icarus-text">
              <strong>ACTIVE BUILDER RUN IN ICARUS MODE (Full Auto-Approval)</strong>
              <p>
                This active run was launched with autonomous permissions (<code>--dangerously-skip-permissions</code>).
                All tool calls and file mutations proceed automatically until turn completion or cancellation.
              </p>
            </div>
          </div>
          {isRunning && (
            <button
              className="danger-btn cancel-turn-btn"
              onClick={handleCancelTurn}
              data-testid="cancel-active-run-btn"
            >
              Cancel Active Run
            </button>
          )}
        </div>
      )}

      {/* Standard Project-Level Icarus Warning Banner */}
      {!activeRunIcarus && icarusState?.enabled && (
        <div className="icarus-warning-banner" role="alert" data-testid="icarus-banner">
          <div className="icarus-banner-content">
            <span className="icarus-icon">⚠️</span>
            <div className="icarus-text">
              <strong>ICARUS MODE ACTIVE: Full Autonomous Execution Preference</strong>
              <p>
                Antigravity will run external tools and file modifications without interactive
                prompts (<code>--dangerously-skip-permissions</code>).
              </p>
            </div>
          </div>
          <button
            className="secondary-btn icarus-disable-btn"
            onClick={() => handleToggleIcarus(false)}
            data-testid="disable-icarus-btn"
          >
            Disable Icarus Mode
          </button>
        </div>
      )}

      {/* 2. Contract Status Bar */}
      <div className="builder-status-bar">
        <div className="status-item">
          <span className="status-label">Project:</span>
          <span className="status-val">{projectName}</span>
        </div>

        <div className="status-item">
          <span className="status-label">Contract Status:</span>
          <span className={`workflow-badge badge-${workflowState.toLowerCase()}`}>
            {workflowState}
          </span>
        </div>

        {builderPacket && (
          <>
            <div className="status-item">
              <span className="status-label">Epoch:</span>
              <code className="status-val">{builderPacket.metadata.builder_epoch_id}</code>
            </div>
            <div className="status-item">
              <span className="status-label">Contract Version:</span>
              <span className="status-val">{builderPacket.metadata.architecture_version}</span>
            </div>
          </>
        )}

        {activeSessionId && (
          <div className="status-item">
            <span className="status-label">Active Session:</span>
            <code className="status-val">{activeSessionId.substring(0, 8)}...</code>
          </div>
        )}

        {sessions.length > 0 && (
          <div className="status-item">
            <span className="status-label">Recorded Sessions:</span>
            <span className="status-val">{sessions.length}</span>
          </div>
        )}

        {isLoading && <span className="status-label subdued-text">Refreshing...</span>}

        <div className="status-item icarus-indicator">
          <span className="status-label">Icarus Autonomy:</span>
          {icarusState?.enabled ? (
            <span className="icarus-badge active" data-testid="icarus-active-badge">ENABLED</span>
          ) : (
            <span className="icarus-badge inactive">LEAST-PRIVILEGE</span>
          )}
          <button
            className="link-btn"
            onClick={() => setShowIcarusModal(true)}
            data-testid="toggle-icarus-modal-btn"
          >
            {icarusState?.enabled ? 'Configure' : 'Enable Icarus Mode'}
          </button>
        </div>

        {builderPacket && (
          <button
            className="secondary-btn inspect-packet-btn"
            onClick={() => setShowPacketModal(true)}
            data-testid="inspect-packet-btn"
          >
            Inspect Frozen Builder Packet
          </button>
        )}
      </div>

      {/* Contract Invalidation / Non-Frozen Guard */}
      {!isFrozen && (
        <div className="builder-gated-warning" role="alert">
          <h3>🔒 Architecture Contract Not Frozen</h3>
          <p>
            The Builder Control Plane executes strictly against the immutable architecture contract.
            Please complete and freeze your architecture in the <strong>Architecture Workspace</strong> before
            running builder turns.
          </p>
        </div>
      )}

      {/* Contract Drift Warning */}
      {driftReport?.has_drift && (
        <div className="builder-drift-warning" role="alert">
          <h3>⚠️ Frozen Contract Drift Detected</h3>
          <p>
            {driftReport.drifted_artifacts.length} architecture artifact(s) have been modified or deleted
            since the contract was frozen. You must reconcile drift in the Architecture Workspace before
            running the Builder.
          </p>
        </div>
      )}

      {errorMessage && (
        <div className="builder-error-alert" role="alert">
          <strong>Execution Error:</strong> {errorMessage}
        </div>
      )}

      {/* 3. Main Builder Control Grid */}
      <div className="builder-grid">
        {/* Left Column: Controls & Execution */}
        <div className="builder-main-col">
          <section className="builder-card control-panel-card">
            <h3>Governed Builder Execution</h3>
            <p className="builder-source-notice">
              Builder prompt and instructions are 100% governed by the frozen Stage 2 Builder Packet.
            </p>

            <div className="config-form-row">
              <div className="form-group model-group">
                <label htmlFor="model-select">Antigravity Model (Live Discovery):</label>
                <select
                  id="model-select"
                  className="select-input"
                  value={selectedModel}
                  onChange={(e) => setSelectedModel(e.target.value)}
                  disabled={isRunning || !isFrozen || models.length === 0}
                  data-testid="model-select"
                >
                  {models.length > 0 ? (
                    models.map((m) => (
                      <option key={m.id} value={m.id}>
                        {m.name} ({m.id})
                      </option>
                    ))
                  ) : (
                    <option value="">No models available</option>
                  )}
                </select>
                {modelError && (
                  <span className="field-error-text" role="alert" data-testid="model-error-banner">{modelError}</span>
                )}
              </div>

              <div className="form-group effort-group">
                <label htmlFor="effort-select">Reasoning Effort:</label>
                <select
                  id="effort-select"
                  className="select-input"
                  value={effort}
                  onChange={(e) => setEffort(e.target.value as 'low' | 'medium' | 'high')}
                  disabled={isRunning || !isFrozen}
                  data-testid="effort-select"
                >
                  <option value="low">Low</option>
                  <option value="medium">Medium</option>
                  <option value="high">High</option>
                </select>
              </div>
            </div>

            <div className="builder-action-bar">
              <button
                className="primary-btn run-turn-btn"
                onClick={handleRunTurn}
                disabled={isRunning || !isFrozen || (driftReport?.has_drift ?? false) || !selectedModel}
                data-testid="run-turn-btn"
              >
                {isRunning ? 'Turn In Progress...' : '▶ Run Builder Turn'}
              </button>

              {isRunning && (
                <button
                  className="danger-btn cancel-turn-btn"
                  onClick={handleCancelTurn}
                  data-testid="cancel-turn-btn"
                >
                  ⏹ Cancel Turn
                </button>
              )}

              {(lastResponse?.status === 'ERROR' || lastResponse?.status === 'FAILED') && !isRunning && (
                <button
                  className="secondary-btn retry-btn"
                  onClick={handleRunTurn}
                  disabled={!isFrozen || !selectedModel}
                  data-testid="retry-turn-btn"
                >
                  🔄 Retry Turn
                </button>
              )}

              <button
                className="secondary-btn clear-term-btn"
                onClick={() => setTerminalLogs([])}
              >
                Clear Terminal
              </button>
            </div>
          </section>

          {/* Completion / Status Report Card */}
          {(latestSession || lastResponse) && (
            <section className="builder-card completion-report-card" data-testid="completion-report">
              <div className="report-header">
                <h3>Latest Builder Session Report</h3>
                <span className={`status-badge status-${(latestSession?.status || lastResponse?.status || '').toLowerCase()}`}>
                  {latestSession?.status || lastResponse?.status}
                </span>
              </div>
              <div className="report-metrics-grid">
                <div className="metric-item">
                  <span className="metric-label">Session ID:</span>
                  <code>{(latestSession?.session_id || lastResponse?.conversation_id || 'unknown').substring(0, 8)}...</code>
                </div>
                <div className="metric-item">
                  <span className="metric-label">Model:</span>
                  <span>{latestSession?.model || selectedModel}</span>
                </div>
                <div className="metric-item">
                  <span className="metric-label">Duration:</span>
                  <span>{latestSession ? `${(latestSession.duration_ms / 1000).toFixed(1)}s` : 'Completed'}</span>
                </div>
                <div className="metric-item">
                  <span className="metric-label">Total Tokens:</span>
                  <span>{(latestSession?.usage.total_tokens ?? lastResponse?.cumulative_usage.total_tokens ?? 0).toLocaleString()}</span>
                </div>
                <div className="metric-item">
                  <span className="metric-label">Icarus Mode:</span>
                  <span>{(latestSession?.icarus_mode ?? icarusState?.enabled) ? 'Yes (Auto-approval)' : 'No (Least-privilege)'}</span>
                </div>
                <div className="metric-item">
                  <span className="metric-label">Completed:</span>
                  <span>{latestSession?.completed_at ? new Date(latestSession.completed_at).toLocaleTimeString() : 'Just now'}</span>
                </div>
              </div>
              {lastResponse?.text_response && (
                <div className="report-text-response">
                  <strong>Response:</strong> <p>{lastResponse.text_response}</p>
                </div>
              )}
              {(latestSession?.error_message || lastResponse?.stderr) && (
                <div className="report-error-box">
                  <strong>Error / Stderr:</strong> {latestSession?.error_message || lastResponse?.stderr}
                </div>
              )}
            </section>
          )}

          {/* Live Streaming Terminal */}
          <section className="builder-card terminal-card">
            <div className="terminal-header">
              <div className="terminal-title">
                <span className="term-dot green"></span>
                <span className="term-dot yellow"></span>
                <span className="term-dot red"></span>
                <span className="term-label">
                  Antigravity Live Stream {isRunning ? '(Streaming Active)' : '(Idle)'}
                </span>
              </div>
              <span className="term-count">{terminalLogs.length} events (max 1,000)</span>
            </div>
            <div className="terminal-body" data-testid="terminal-stream">
              {terminalLogs.length === 0 ? (
                <div className="terminal-placeholder">
                  Ready to execute builder turns. Execution events, tool invocations, and agent output
                  will stream here in real time.
                </div>
              ) : (
                terminalLogs.map((log, idx) => (
                  <div key={idx} className="terminal-line">
                    {log}
                  </div>
                ))
              )}
              <div ref={terminalEndRef} />
            </div>
          </section>
        </div>

        {/* Right Column: Telemetry & Permissions */}
        <div className="builder-side-col">
          {/* Capacity & Usage Telemetry */}
          <section className="builder-card telemetry-card">
            <h3>Resource Usage &amp; Capacity</h3>

            <div className="telemetry-box provider-box">
              <div className="box-header">
                <span className="box-title">Antigravity Tokens</span>
                <span className="authoritative-tag">Provider-Reported</span>
              </div>
              <div className="token-stat-grid">
                <div className="stat-pill">
                  <span className="stat-label">Total:</span>
                  <span className="stat-number">
                    {telemetry?.provider_antigravity_usage.total_tokens.toLocaleString() ?? 0}
                  </span>
                </div>
                <div className="stat-pill">
                  <span className="stat-label">Input:</span>
                  <span className="stat-number">
                    {telemetry?.provider_antigravity_usage.input_tokens.toLocaleString() ?? 0}
                  </span>
                </div>
                <div className="stat-pill">
                  <span className="stat-label">Output:</span>
                  <span className="stat-number">
                    {telemetry?.provider_antigravity_usage.output_tokens.toLocaleString() ?? 0}
                  </span>
                </div>
                <div className="stat-pill">
                  <span className="stat-label">Thinking:</span>
                  <span className="stat-number">
                    {telemetry?.provider_antigravity_usage.thinking_tokens.toLocaleString() ?? 0}
                  </span>
                </div>
              </div>
            </div>

            <div className="telemetry-box chatgpt-box">
              <div className="box-header">
                <span className="box-title">ChatGPT Architecture Relay</span>
                <span className="estimated-tag">
                  v{telemetry?.chatgpt_estimated_usage?.estimator_version ?? 1} Calibrated
                </span>
              </div>
              <div className="token-stat-grid">
                <div className="stat-pill">
                  <span className="stat-label">5-Hour Rolling:</span>
                  <span className="stat-number">
                    {telemetry?.chatgpt_estimated_usage?.rolling_5h_tokens?.toLocaleString() ?? 0}
                    {telemetry?.chatgpt_estimated_usage?.estimated_5h_capacity_pct != null && (
                      <span className="capacity-pct-badge">
                        ({telemetry.chatgpt_estimated_usage.estimated_5h_capacity_pct.toFixed(1)}%)
                      </span>
                    )}
                  </span>
                </div>
                <div className="stat-pill">
                  <span className="stat-label">Weekly Rolling:</span>
                  <span className="stat-number">
                    {telemetry?.chatgpt_estimated_usage?.rolling_7d_tokens?.toLocaleString() ?? 0}
                    {telemetry?.chatgpt_estimated_usage?.estimated_weekly_capacity_pct != null && (
                      <span className="capacity-pct-badge">
                        ({telemetry.chatgpt_estimated_usage.estimated_weekly_capacity_pct.toFixed(1)}%)
                      </span>
                    )}
                  </span>
                </div>
                <div className="stat-pill">
                  <span className="stat-label">Total Cumulative:</span>
                  <span className="stat-number">
                    {telemetry?.chatgpt_estimated_usage?.total_tokens?.toLocaleString() ?? 0}
                  </span>
                </div>
                <div className="stat-pill">
                  <span className="stat-label">Ratio:</span>
                  <span className="stat-number">
                    ~{(telemetry?.chatgpt_estimated_usage?.chars_per_token ?? 4.0).toFixed(2)} chars/tok
                  </span>
                </div>
              </div>
              <p className="disclaimer-text">
                {telemetry?.chatgpt_estimated_usage?.disclaimer ||
                  'Estimated relay throughput based on character heuristics. Does not reflect official OpenAI billing.'}
              </p>
              <div className="chatgpt-actions-row">
                <button
                  className="secondary-btn reset-window-btn"
                  onClick={handleResetChatGpt}
                  data-testid="reset-chatgpt-btn"
                >
                  Reset Window
                </button>
                <button
                  className="secondary-btn calibrate-btn"
                  onClick={() => setShowCalibrationModal(true)}
                  data-testid="calibrate-chatgpt-btn"
                >
                  Calibrate Estimator
                </button>
              </div>
            </div>
          </section>

          {/* Permission History & Blocked Guidance */}
          <section className="builder-card permissions-card">
            <h3>Tool Permissions &amp; History</h3>
            <div className="permission-history-list" data-testid="permission-history">
              {permissionHistory.length === 0 ? (
                <p className="no-records-text">No permission events recorded yet.</p>
              ) : (
                permissionHistory.slice(0, 10).map((p) => (
                  <div key={p.id} className={`permission-item risk-${p.risk_level.toLowerCase()}`}>
                    <div className="perm-header">
                      <span className="perm-tool"><code>{p.tool_name}</code></span>
                      <span className={`decision-badge decision-${p.decision.toLowerCase()}`}>
                        {p.decision}
                      </span>
                    </div>
                    {p.target && <div className="perm-target">Target: <code>{p.target}</code></div>}
                    {p.reason && <div className="perm-reason">{p.reason}</div>}
                  </div>
                ))
              )}
            </div>

            {permissionHistory.some((p) => p.decision === 'DENIED' || p.decision === 'BLOCKED') && (
              <div className="blocked-guidance-box" role="note">
                <strong>Why was a tool blocked?</strong>
                <p>
                  Antigravity runs headlessly and cannot prompt for interactive CLI approval.
                  To allow mutating commands, either review requirements or authorize{' '}
                  <button className="link-btn" onClick={() => setShowIcarusModal(true)}>
                    Icarus Mode
                  </button>{' '}
                  for full autonomous tool execution.
                </p>
              </div>
            )}
          </section>
        </div>
      </div>

      {/* Icarus Mode Confirmation Modal */}
      {showIcarusModal && (
        <div className="modal-backdrop" role="dialog" aria-modal="true" data-testid="icarus-modal">
          <div className="modal-dialog icarus-dialog">
            <div className="modal-header">
              <h2>⚠️ Icarus Mode Authorization</h2>
              <button className="close-btn" onClick={() => setShowIcarusModal(false)}>✕</button>
            </div>
            <div className="modal-body">
              <p>
                <strong>Icarus Mode</strong> enables full autonomous tool execution for the Builder
                by passing <code>--dangerously-skip-permissions</code> to the Antigravity engine.
              </p>
              <div className="icarus-modal-notice">
                <ul>
                  <li>Mutating tools (command execution, file writes) will proceed without interactive confirmation.</li>
                  <li>All actions remain bounded by Coalition process managers and are logged in audit history.</li>
                  <li>You maintain human supervisory authority and may cancel execution at any time.</li>
                </ul>
              </div>
              <p className="status-question">
                Current status:{' '}
                <strong>{icarusState?.enabled ? 'Active (Auto-approval enabled)' : 'Inactive (Least-privilege)'}</strong>
              </p>
            </div>
            <div className="modal-footer">
              <button className="secondary-btn" onClick={() => setShowIcarusModal(false)}>
                Cancel
              </button>
              {icarusState?.enabled ? (
                <button
                  className="secondary-btn"
                  onClick={() => handleToggleIcarus(false)}
                  data-testid="confirm-disable-icarus"
                >
                  Disable Icarus Mode
                </button>
              ) : (
                <button
                  className="danger-btn"
                  onClick={() => handleToggleIcarus(true)}
                  data-testid="confirm-enable-icarus"
                >
                  Authorize &amp; Enable Icarus Mode
                </button>
              )}
            </div>
          </div>
        </div>
      )}

      {/* Estimator Calibration Modal */}
      {showCalibrationModal && (
        <div className="modal-backdrop" role="dialog" aria-modal="true" data-testid="calibration-modal">
          <div className="modal-dialog calibration-dialog">
            <div className="modal-header">
              <h2>📐 Calibrate ChatGPT Usage Estimator</h2>
              <button className="close-btn" onClick={() => setShowCalibrationModal(false)}>✕</button>
            </div>
            <form onSubmit={handleCalibrateChatGpt}>
              <div className="modal-body">
                <p>
                  Provide an observed prompt or response sample from ChatGPT Plus to calibrate the characters-per-token ratio.
                </p>
                <div className="form-group">
                  <label htmlFor="cal-chars">Sample Characters:</label>
                  <input
                    id="cal-chars"
                    type="number"
                    min="1"
                    className="text-input"
                    value={calChars}
                    onChange={(e) => setCalChars(parseInt(e.target.value, 10) || 0)}
                    required
                  />
                </div>
                <div className="form-group">
                  <label htmlFor="cal-tokens">Observed / Reported Tokens:</label>
                  <input
                    id="cal-tokens"
                    type="number"
                    min="1"
                    className="text-input"
                    value={calTokens}
                    onChange={(e) => setCalTokens(parseInt(e.target.value, 10) || 0)}
                    required
                  />
                </div>
                <p className="subtext">
                  Observed ratio: {calTokens > 0 ? (calChars / calTokens).toFixed(2) : '0'} chars/token.
                </p>
              </div>
              <div className="modal-footer">
                <button type="button" className="secondary-btn" onClick={() => setShowCalibrationModal(false)}>
                  Cancel
                </button>
                <button type="submit" className="primary-btn" data-testid="submit-calibration-btn">
                  Apply Calibration
                </button>
              </div>
            </form>
          </div>
        </div>
      )}

      {/* Packet Inspector Modal */}
      {showPacketModal && builderPacket && (
        <div className="modal-backdrop" role="dialog" aria-modal="true" data-testid="packet-modal">
          <div className="modal-dialog packet-dialog">
            <div className="modal-header">
              <h2>Authoritative Frozen Builder Packet</h2>
              <button className="close-btn" onClick={() => setShowPacketModal(false)}>✕</button>
            </div>
            <div className="modal-body">
              <div className="packet-meta-row">
                <span><strong>Epoch:</strong> {builderPacket.metadata.builder_epoch_id}</span>
                <span><strong>Version:</strong> {builderPacket.metadata.architecture_version}</span>
                <span><strong>Fingerprint:</strong> <code>{builderPacket.metadata.contract_fingerprint.substring(0, 12)}...</code></span>
              </div>

              <h4>Immutable Builder Prompt</h4>
              <pre className="packet-prompt-box">{builderPacket.prompt}</pre>

              <h4>Frozen Artifacts ({builderPacket.artifacts.length})</h4>
              <div className="packet-artifact-list">
                {builderPacket.artifacts.map((a) => (
                  <div key={a.path} className="packet-art-item">
                    <code>{a.path}</code> ({a.title})
                  </div>
                ))}
              </div>
            </div>
            <div className="modal-footer">
              <button className="primary-btn" onClick={() => setShowPacketModal(false)}>
                Close Inspector
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};

import React, { useEffect, useState, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  WorkspaceState,
  RelayPacket,
  ImportPreview,
  ReadinessReport,
  ArtifactReadinessItem,
  ArtifactContentDetails,
  ArtifactApplicability,
  CommandError,
  FreezePreview,
  FreezeResult,
  DriftReport,
  DriftDiff,
  BuilderPacket,
} from '../../types';

interface ArchitectureWorkspaceViewProps {
  projectId: string;
  projectName: string;
  workflowState: string;
  onRefreshProject: () => Promise<void>;
}

export const ArchitectureWorkspaceView: React.FC<ArchitectureWorkspaceViewProps> = ({
  projectId,
  projectName: _projectName,
  workflowState,
  onRefreshProject,
}) => {
  const [workspace, setWorkspace] = useState<WorkspaceState | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [copyFeedback, setCopyFeedback] = useState<string | null>(null);
  const [isPromptExpanded, setIsPromptExpanded] = useState(false);

  // Manual recovery / parse failure state
  const [parseErrorData, setParseErrorData] = useState<{
    importId: string;
    rawContent: string;
    message: string;
  } | null>(null);
  const [editedRecoveryText, setEditedRecoveryText] = useState('');

  // Selected artifact content viewer and editor
  const [viewingArtifact, setViewingArtifact] = useState<{
    path: string;
    title: string;
    content: string;
    fingerprint?: string | null;
    isEditing: boolean;
    editedContent: string;
    saveError: string | null;
  } | null>(null);

  // Stage 2B: Freeze and Drift State
  const [freezePreview, setFreezePreview] = useState<FreezePreview | null>(null);
  const [isFreezeModalOpen, setIsFreezeModalOpen] = useState(false);
  const [isFreezeSuccess, setIsFreezeSuccess] = useState<FreezeResult | null>(null);
  const [driftReport, setDriftReport] = useState<DriftReport | null>(null);
  const [viewingDiff, setViewingDiff] = useState<DriftDiff | null>(null);
  const [builderPacket, setBuilderPacket] = useState<BuilderPacket | null>(null);
  const [isPacketModalOpen, setIsPacketModalOpen] = useState(false);

  const loadWorkspaceState = useCallback(async () => {
    setIsLoading(true);
    setErrorMessage(null);
    try {
      const state = await invoke<WorkspaceState>('get_architecture_workspace_state', {
        projectId,
      });
      setWorkspace(state);
    } catch (err: unknown) {
      setErrorMessage(formatError(err));
    } finally {
      setIsLoading(false);
    }
  }, [projectId]);

  useEffect(() => {
    loadWorkspaceState();
  }, [loadWorkspaceState]);

  const handlePreparePacket = async () => {
    setIsLoading(true);
    setErrorMessage(null);
    try {
      await invoke<RelayPacket>('prepare_architect_relay_packet', {
        projectId,
        customNotes: null,
      });
      await loadWorkspaceState();
      await onRefreshProject();
    } catch (err: unknown) {
      setErrorMessage(formatError(err));
    } finally {
      setIsLoading(false);
    }
  };

  const handleCopyPacket = useCallback(async () => {
    if (!workspace?.pending_packet) return;
    setErrorMessage(null);
    try {
      await invoke('copy_relay_packet_to_clipboard', {
        packetId: workspace.pending_packet.metadata.packet_id,
      });
      setCopyFeedback('Prompt copied to clipboard!');
      setTimeout(() => setCopyFeedback(null), 3000);
    } catch (err: unknown) {
      setErrorMessage(formatError(err));
    }
  }, [workspace?.pending_packet]);

  const handleOpenChatGPT = async () => {
    try {
      await invoke('open_chatgpt');
    } catch (err: unknown) {
      setErrorMessage(formatError(err));
    }
  };

  const handleImportClipboard = useCallback(async () => {
    setIsLoading(true);
    setErrorMessage(null);
    setParseErrorData(null);
    try {
      const preview = await invoke<ImportPreview>('import_from_clipboard', {
        projectId,
      });
      if (workspace) {
        setWorkspace({ ...workspace, pending_preview: preview });
      } else {
        await loadWorkspaceState();
      }
    } catch (err: unknown) {
      const cmdErr = err as CommandError;
      if (
        cmdErr &&
        cmdErr.code === 'RELAY_PARSE_FAILURE' &&
        cmdErr.details &&
        typeof cmdErr.details.import_id === 'string'
      ) {
        const importId = cmdErr.details.import_id as string;
        const rawContent = (cmdErr.details.raw_content as string) || '';
        const message = (cmdErr.details.message as string) || cmdErr.message;
        setParseErrorData({ importId, rawContent, message });
        setEditedRecoveryText(rawContent);
      } else {
        setErrorMessage(formatError(err));
      }
    } finally {
      setIsLoading(false);
    }
  }, [projectId, workspace, loadWorkspaceState]);

  const handleRetryParse = async () => {
    if (!parseErrorData) return;
    setIsLoading(true);
    setErrorMessage(null);
    try {
      const preview = await invoke<ImportPreview>('retry_parse_import', {
        projectId,
        importId: parseErrorData.importId,
        editedRawText: editedRecoveryText,
      });
      setParseErrorData(null);
      if (workspace) {
        setWorkspace({ ...workspace, pending_preview: preview });
      } else {
        await loadWorkspaceState();
      }
    } catch (err: unknown) {
      setErrorMessage(formatError(err));
    } finally {
      setIsLoading(false);
    }
  };

  const handleAcceptImport = async () => {
    if (!workspace?.pending_preview) return;
    setIsLoading(true);
    setErrorMessage(null);
    try {
      await invoke<ReadinessReport>('accept_relay_import', {
        projectId,
        importId: workspace.pending_preview.import_id,
      });
      await loadWorkspaceState();
      await onRefreshProject();
    } catch (err: unknown) {
      setErrorMessage(formatError(err));
    } finally {
      setIsLoading(false);
    }
  };

  const handleRejectImport = async () => {
    if (!workspace?.pending_preview) return;
    setIsLoading(true);
    setErrorMessage(null);
    try {
      await invoke('reject_relay_import', {
        projectId,
        importId: workspace.pending_preview.import_id,
      });
      await loadWorkspaceState();
      await onRefreshProject();
    } catch (err: unknown) {
      setErrorMessage(formatError(err));
    } finally {
      setIsLoading(false);
    }
  };

  const handleViewArtifactContent = async (item: ArtifactReadinessItem) => {
    try {
      const result = await invoke<ArtifactContentDetails | string | null>('get_artifact_content', {
        projectId,
        artifactPath: item.path,
      });
      const content = typeof result === 'string' ? result : (result?.content || '');
      const fingerprint = typeof result === 'object' && result !== null ? result.fingerprint : null;
      setViewingArtifact({
        path: item.path,
        title: item.title,
        content: content || '(File is currently empty)',
        fingerprint,
        isEditing: false,
        editedContent: content,
        saveError: null,
      });
    } catch (err: unknown) {
      setErrorMessage(formatError(err));
    }
  };

  const handleSaveArtifactContent = async () => {
    if (!viewingArtifact) return;
    setIsLoading(true);
    setViewingArtifact((prev) => (prev ? { ...prev, saveError: null } : null));
    try {
      await invoke<ReadinessReport>('save_artifact_content', {
        projectId,
        path: viewingArtifact.path,
        content: viewingArtifact.editedContent,
        expectedFingerprint: viewingArtifact.fingerprint,
      });
      setViewingArtifact((prev) =>
        prev
          ? {
              ...prev,
              content: prev.editedContent,
              isEditing: false,
              saveError: null,
            }
          : null
      );
      await loadWorkspaceState();
      await onRefreshProject();
    } catch (err: unknown) {
      const formatted = formatError(err);
      setViewingArtifact((prev) => (prev ? { ...prev, saveError: formatted } : null));
    } finally {
      setIsLoading(false);
    }
  };

  const handleApplicabilityChange = async (
    artifactPath: string,
    newApplicability: ArtifactApplicability,
    e: React.SyntheticEvent
  ) => {
    e.stopPropagation();
    setIsLoading(true);
    setErrorMessage(null);
    try {
      const updated = await invoke<WorkspaceState>('set_project_artifact_applicability', {
        projectId,
        artifactPath,
        applicability: newApplicability,
      });
      setWorkspace(updated);
      await onRefreshProject();
    } catch (err: unknown) {
      setErrorMessage(formatError(err));
    } finally {
      setIsLoading(false);
    }
  };

  const loadDriftReport = useCallback(async () => {
    if (workflowState !== 'FROZEN') {
      setDriftReport(null);
      return;
    }
    try {
      const drift = await invoke<DriftReport>('get_contract_drift', { projectId });
      setDriftReport(drift);
    } catch (err: unknown) {
      // Non-fatal background drift check failure
      console.error('Failed to check contract drift:', err);
    }
  }, [projectId, workflowState]);

  useEffect(() => {
    loadDriftReport();
  }, [loadDriftReport]);

  const handleOpenFreezeModal = async () => {
    setIsLoading(true);
    setErrorMessage(null);
    try {
      const preview = await invoke<FreezePreview>('prepare_architecture_freeze', {
        projectId,
      });
      setFreezePreview(preview);
      setIsFreezeModalOpen(true);
    } catch (err: unknown) {
      setErrorMessage(formatError(err));
    } finally {
      setIsLoading(false);
    }
  };

  const handleConfirmFreeze = async () => {
    if (!freezePreview) return;
    setIsLoading(true);
    setErrorMessage(null);
    try {
      const result = await invoke<FreezeResult>('confirm_architecture_freeze', {
        projectId,
        previewId: freezePreview.preview_id,
      });
      setIsFreezeSuccess(result);
      setIsFreezeModalOpen(false);
      setFreezePreview(null);
      await loadWorkspaceState();
      await loadDriftReport();
      await onRefreshProject();
    } catch (err: unknown) {
      setErrorMessage(formatError(err));
    } finally {
      setIsLoading(false);
    }
  };

  const handleInspectDriftDiff = async (artifactPath: string) => {
    setIsLoading(true);
    setErrorMessage(null);
    try {
      const diff = await invoke<DriftDiff>('get_drift_diff', {
        projectId,
        artifactPath,
      });
      setViewingDiff(diff);
    } catch (err: unknown) {
      setErrorMessage(formatError(err));
    } finally {
      setIsLoading(false);
    }
  };

  const handleRestoreDriftedArtifact = async (artifactPath: string) => {
    setIsLoading(true);
    setErrorMessage(null);
    try {
      const report = await invoke<DriftReport>('restore_drifted_artifact', {
        projectId,
        artifactPath,
      });
      setDriftReport(report);
      await loadWorkspaceState();
      await onRefreshProject();
      if (viewingDiff && viewingDiff.path === artifactPath) {
        setViewingDiff(null);
      }
    } catch (err: unknown) {
      setErrorMessage(formatError(err));
    } finally {
      setIsLoading(false);
    }
  };

  const handleRestoreAllDrift = async () => {
    setIsLoading(true);
    setErrorMessage(null);
    try {
      const report = await invoke<DriftReport>('restore_all_drifted_artifacts', {
        projectId,
      });
      setDriftReport(report);
      await loadWorkspaceState();
      await onRefreshProject();
      setViewingDiff(null);
    } catch (err: unknown) {
      setErrorMessage(formatError(err));
    } finally {
      setIsLoading(false);
    }
  };

  const handleViewBuilderPacket = async () => {
    setIsLoading(true);
    setErrorMessage(null);
    try {
      const packet = await invoke<BuilderPacket>('get_builder_packet', {
        projectId,
        version: null,
      });
      setBuilderPacket(packet);
      setIsPacketModalOpen(true);
    } catch (err: unknown) {
      setErrorMessage(formatError(err));
    } finally {
      setIsLoading(false);
    }
  };

  // Listen to in-window keyboard shortcuts and relay events dispatched from top-level shortcuts
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.shiftKey) {
        if (e.key === 'R' || e.key === 'r') {
          e.preventDefault();
          handleCopyPacket();
        } else if (e.key === 'I' || e.key === 'i') {
          e.preventDefault();
          handleImportClipboard();
        }
      }
    };
    window.addEventListener('keydown', handleKeyDown);

    const handleCopied = () => {
      setCopyFeedback('Prompt copied to clipboard!');
      setTimeout(() => setCopyFeedback(null), 3000);
    };
    const handleImported = (e: Event) => {
      const customEvent = e as CustomEvent<ImportPreview>;
      if (customEvent.detail) {
        setWorkspace((prev) => (prev ? { ...prev, pending_preview: customEvent.detail } : prev));
      } else {
        loadWorkspaceState();
      }
    };
    window.addEventListener('coalition:relay-packet-copied', handleCopied);
    window.addEventListener('coalition:relay-imported', handleImported);
    return () => {
      window.removeEventListener('keydown', handleKeyDown);
      window.removeEventListener('coalition:relay-packet-copied', handleCopied);
      window.removeEventListener('coalition:relay-imported', handleImported);
    };
  }, [handleCopyPacket, handleImportClipboard, loadWorkspaceState]);

  const formatError = (err: unknown): string => {
    if (typeof err === 'object' && err !== null && 'message' in err) {
      const cmdErr = err as CommandError;
      return `[${cmdErr.code || 'ERROR'}] ${cmdErr.message}`;
    }
    if (err instanceof Error) return err.message;
    return String(err);
  };

  return (
    <div className="architecture-workspace">
      {errorMessage && (
        <div className="workspace-error-banner" role="alert">
          <span>{errorMessage}</span>
          <button onClick={() => setErrorMessage(null)} aria-label="Dismiss error">✕</button>
        </div>
      )}

      {copyFeedback && (
        <div className="workspace-success-banner" role="status">
          <span>✓ {copyFeedback}</span>
        </div>
      )}

      {/* Frozen Architecture Status Banner */}
      {workflowState === 'FROZEN' && (
        <div className="frozen-architecture-banner" role="status">
          <div className="frozen-banner-left">
            <span className="frozen-lock-icon">🔒</span>
            <div className="frozen-info-text">
              <strong>Architecture Contract Frozen (v1.0)</strong>
              <p>
                Architecture specifications are immutable contracts. Active files in <code>.coalition/</code> are continuously verified against the frozen snapshot.
              </p>
            </div>
          </div>
          <div className="frozen-banner-actions">
            <button
              className="secondary-btn view-packet-btn"
              onClick={handleViewBuilderPacket}
              disabled={isLoading}
            >
              View Builder Packet
            </button>
          </div>
        </div>
      )}

      {/* Contract Drift Detection Panel */}
      {driftReport && driftReport.has_drift && (
        <div className="drift-alert-panel" role="alert">
          <div className="drift-panel-header">
            <div className="drift-panel-title">
              <span className="drift-warning-icon">⚠️</span>
              <strong>Architecture Contract Drift Detected ({driftReport.drifted_artifacts.length} file{driftReport.drifted_artifacts.length === 1 ? '' : 's'})</strong>
            </div>
            <div className="drift-panel-actions">
              <button
                className="secondary-btn restore-all-btn"
                onClick={handleRestoreAllDrift}
                disabled={isLoading}
              >
                Restore All to Frozen
              </button>
              <span className="arch-change-notice" title="Changes cannot be incorporated into the contract without a formal human architecture change">
                Architecture Change Required
              </span>
            </div>
          </div>
          <p className="drift-panel-desc">
            Active architecture files deviate from the frozen v1.0 snapshot. Builder execution is blocked until contract integrity is restored.
          </p>
          <div className="drift-items-list">
            {driftReport.drifted_artifacts.map((drifted) => (
              <div key={drifted.path} className={`drift-item drift-${drifted.drift_type.toLowerCase()}`}>
                <div className="drift-item-info">
                  <span className={`diff-badge badge-${drifted.drift_type.toLowerCase()}`}>
                    {drifted.drift_type}
                  </span>
                  <code className="drift-path">{drifted.path}</code>
                </div>
                <div className="drift-item-actions">
                  <button
                    className="secondary-btn inspect-diff-btn"
                    onClick={() => handleInspectDriftDiff(drifted.path)}
                    disabled={isLoading}
                  >
                    Inspect Diff
                  </button>
                  <button
                    className="primary-btn restore-file-btn"
                    onClick={() => handleRestoreDriftedArtifact(drifted.path)}
                    disabled={isLoading}
                  >
                    {drifted.drift_type === 'ADDED' ? 'Quarantine' : 'Restore'}
                  </button>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      <div className="workspace-columns">
        {/* Left Column: Architect Relay */}
        <section className="workspace-column relay-column">
          <div className="column-header">
            <h2>Architect Relay</h2>
            <span className={`relay-state-tag state-${workflowState.toLowerCase()}`}>
              {workflowState}
            </span>
          </div>

          <p className="governance-note">
            Human-mediated ChatGPT relay. No automated scraping; explicit human clipboard transport only.
          </p>

          <div className="relay-actions-panel">
            <button
              className="primary-btn prepare-btn"
              onClick={handlePreparePacket}
              disabled={isLoading}
            >
              {workspace?.pending_packet ? 'Regenerate Prompt' : 'Prepare Architect Prompt'}
            </button>

            <div className="relay-action-row">
              <button
                className="secondary-btn copy-prompt-btn"
                onClick={handleCopyPacket}
                disabled={isLoading || !workspace?.pending_packet}
                title="Ctrl+Shift+R"
              >
                Copy for ChatGPT (Ctrl+Shift+R)
              </button>

              <button
                className="secondary-btn open-chatgpt-btn"
                onClick={handleOpenChatGPT}
                disabled={isLoading}
              >
                Open ChatGPT ↗
              </button>
            </div>

            <button
              className="primary-btn import-btn"
              onClick={handleImportClipboard}
              disabled={isLoading}
              title="Ctrl+Shift+I"
            >
              Import from Clipboard (Ctrl+Shift+I)
            </button>
          </div>

          {/* Active Prompt Preview / Inspector */}
          {workspace?.pending_packet && (
            <div className="pending-packet-card">
              <div className="card-header-toggle" onClick={() => setIsPromptExpanded(!isPromptExpanded)}>
                <strong>Active Packet:</strong>
                <code>{workspace.pending_packet.metadata.packet_id.substring(0, 8)}...</code>
                <span className="toggle-indicator">{isPromptExpanded ? '▲ Hide Prompt' : '▼ Inspect Prompt'}</span>
              </div>
              {isPromptExpanded && (
                <div className="prompt-preview-content">
                  <pre className="prompt-text-box">{workspace.pending_packet.prompt}</pre>
                </div>
              )}
            </div>
          )}

          {/* Import Preview Card (When an import is pending human review) */}
          {workspace?.pending_preview && (
            <div className="import-preview-card" role="region" aria-label="Proposed Changes Preview">
              <div className="preview-header">
                <h3>Proposed Architecture Changes</h3>
                <span className="preview-summary-text">{workspace.pending_preview.summary}</span>
              </div>

              <div className="preview-artifacts-list">
                {workspace.pending_preview.artifacts.map((art) => (
                  <div key={art.path} className={`preview-artifact-item status-${art.status.toLowerCase()}`}>
                    <div className="preview-item-top">
                      <span className={`diff-badge badge-${art.status.toLowerCase()}`}>{art.status}</span>
                      <span className="art-title">{art.title}</span>
                      <code className="art-path">{art.path}</code>
                    </div>
                  </div>
                ))}
              </div>

              {workspace.pending_preview.open_questions.length > 0 && (
                <div className="preview-questions">
                  <h4>Open Questions ({workspace.pending_preview.open_questions.length})</h4>
                  <ul>
                    {workspace.pending_preview.open_questions.map((q) => (
                      <li key={q.id}>
                        <strong>[{q.status}] {q.id}:</strong> {q.question}
                      </li>
                    ))}
                  </ul>
                </div>
              )}

              <div className="preview-actions">
                <button
                  className="primary-btn accept-import-btn"
                  onClick={handleAcceptImport}
                  disabled={isLoading}
                >
                  ✓ Accept & Apply Changes
                </button>
                <button
                  className="secondary-btn reject-import-btn"
                  onClick={handleRejectImport}
                  disabled={isLoading}
                >
                  ✕ Reject
                </button>
              </div>
            </div>
          )}

          {/* Parse Failure Manual Recovery Section */}
          {parseErrorData && (
            <div className="recovery-panel" role="alert">
              <h3>Import Parse Error (Manual Recovery)</h3>
              <p className="error-hint">{parseErrorData.message}</p>
              <label htmlFor="recovery-textarea">
                Edit the imported response text to correct syntax or formatting:
              </label>
              <textarea
                id="recovery-textarea"
                className="recovery-editor"
                rows={8}
                value={editedRecoveryText}
                onChange={(e) => setEditedRecoveryText(e.target.value)}
              />
              <div className="recovery-actions">
                <button
                  className="primary-btn retry-parse-btn"
                  onClick={handleRetryParse}
                  disabled={isLoading}
                >
                  Retry Parsing
                </button>
                <button
                  className="secondary-btn discard-recovery-btn"
                  onClick={() => setParseErrorData(null)}
                >
                  Discard
                </button>
              </div>
            </div>
          )}

          {/* Relay History Log */}
          {workspace?.history && workspace.history.length > 0 && (
            <div className="relay-history-section">
              <h3>Relay History</h3>
              <ul className="history-list">
                {workspace.history.map((h) => (
                  <li key={h.id} className={`history-item history-${h.status.toLowerCase()}`}>
                    <span className="history-status">{h.status}</span>
                    <span className="history-summary">{h.summary}</span>
                    <span className="history-time">{new Date(h.created_at).toLocaleTimeString()}</span>
                  </li>
                ))}
              </ul>
            </div>
          )}
        </section>

        {/* Right Column: Project Design & Readiness */}
        <section className="workspace-column design-column">
          <div className="column-header">
            <h2>Project Design & Readiness</h2>
            <div className="column-header-actions">
              {workspace?.readiness && (
                <span
                  className={`readiness-badge readiness-${workspace.readiness.overall_readiness.toLowerCase()}`}
                >
                  {workspace.readiness.overall_readiness === 'READY_TO_FREEZE'
                    ? `READY TO FREEZE (${workspace.readiness.ready_required_count ?? workspace.readiness.ready_count ?? 0}/${workspace.readiness.total_required_count ?? workspace.readiness.total_required ?? 0})`
                    : `INCOMPLETE (${workspace.readiness.ready_required_count ?? workspace.readiness.ready_count ?? 0}/${workspace.readiness.total_required_count ?? workspace.readiness.total_required ?? 0})`}
                </span>
              )}
              {workflowState !== 'FROZEN' && (
                <button
                  className="primary-btn freeze-btn"
                  onClick={handleOpenFreezeModal}
                  disabled={isLoading || workspace?.readiness?.overall_readiness !== 'READY_TO_FREEZE'}
                  title={
                    workspace?.readiness?.overall_readiness === 'READY_TO_FREEZE'
                      ? 'Freeze Architecture v1.0 (Requires explicit human authorization)'
                      : 'All required architecture artifacts must reach substantive readiness before freezing'
                  }
                >
                  Freeze Architecture
                </button>
              )}
            </div>
          </div>

          <p className="readiness-guidance">
            Architecture contracts are drafted and reviewed in this workspace. All required architecture
            artifacts ({workspace?.readiness?.ready_required_count ?? workspace?.readiness?.ready_count ?? 0}/{workspace?.readiness?.total_required_count ?? workspace?.readiness?.total_required ?? 0})
            must reach substantive readiness (Readiness Policy v{workspace?.readiness?.policy_version ?? 1}) before the human can authorize freeze.
          </p>

          {Boolean(workspace?.readiness?.unresolved_open_questions_count && workspace.readiness.unresolved_open_questions_count > 0) && (
            <div className="open-questions-warning-banner" role="alert">
              <strong>⚠️ Open Questions ({workspace?.readiness?.unresolved_open_questions_count} unresolved):</strong> Unresolved architectural questions remain in <code>design/open-questions.md</code>.
            </div>
          )}

          <div className="artifacts-readiness-list">
            {workspace?.readiness?.artifacts?.map((art) => {
              const isReady = art.status === 'READY';
              const isIncomplete = art.status === 'INCOMPLETE';
              const isMissing = art.status === 'MISSING';

              return (
                <div
                  key={art.path}
                  className={`artifact-readiness-card card-${art.status.toLowerCase()}`}
                  onClick={() => handleViewArtifactContent(art)}
                  role="button"
                  tabIndex={0}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' || e.key === ' ') {
                      handleViewArtifactContent(art);
                    }
                  }}
                  title="Click to view file content"
                >
                  <div className="card-left">
                    <span className="status-indicator">
                      {isReady && '✓'}
                      {isIncomplete && '◐'}
                      {isMissing && '○'}
                    </span>
                    <div className="card-titles">
                      <span className="artifact-title">{art.title}</span>
                      <div className="applicability-row">
                        <select
                          className={`applicability-select badge-${(art.applicability || 'REQUIRED').toLowerCase()}`}
                          value={art.applicability || 'REQUIRED'}
                          onClick={(e) => e.stopPropagation()}
                          onChange={(e) =>
                            handleApplicabilityChange(
                              art.path,
                              e.target.value as ArtifactApplicability,
                              e
                            )
                          }
                          aria-label={`Applicability for ${art.title}`}
                          title="Change artifact applicability"
                        >
                          <option value="REQUIRED">REQUIRED</option>
                          <option value="OPTIONAL">OPTIONAL</option>
                          <option value="NOT_APPLICABLE">NOT APPLICABLE</option>
                        </select>
                      </div>
                      <code className="artifact-path">{art.path}</code>
                      {art.details && <span className="artifact-details-hint">{art.details}</span>}
                    </div>
                  </div>
                  <div className="card-right">
                    <span className="char-count">
                      {isMissing ? 'Missing' : `${art.character_count} chars`}
                    </span>
                    <span className="view-link">View / Edit ↗</span>
                  </div>
                </div>
              );
            })}
          </div>
        </section>
      </div>

      {/* Artifact Content Viewer / Editor Modal */}
      {viewingArtifact && (
        <div className="modal-overlay" onClick={() => setViewingArtifact(null)}>
          <div
            className="modal-container artifact-modal"
            onClick={(e) => e.stopPropagation()}
            role="dialog"
            aria-labelledby="modal-artifact-title"
          >
            <div className="modal-header">
              <h2 id="modal-artifact-title">{viewingArtifact.title}</h2>
              <div className="modal-header-actions">
                {!viewingArtifact.isEditing ? (
                  <button
                    className="secondary-btn edit-artifact-btn"
                    onClick={() =>
                      setViewingArtifact({
                        ...viewingArtifact,
                        isEditing: true,
                        editedContent: viewingArtifact.content === '(File is currently empty)' ? '' : viewingArtifact.content,
                        saveError: null,
                      })
                    }
                  >
                    Edit
                  </button>
                ) : (
                  <>
                    <button
                      className="primary-btn save-artifact-btn"
                      onClick={handleSaveArtifactContent}
                      disabled={isLoading}
                    >
                      Save
                    </button>
                    <button
                      className="secondary-btn cancel-edit-btn"
                      onClick={() =>
                        setViewingArtifact({
                          ...viewingArtifact,
                          isEditing: false,
                          editedContent: viewingArtifact.content,
                          saveError: null,
                        })
                      }
                      disabled={isLoading}
                    >
                      Cancel
                    </button>
                  </>
                )}
                <button
                  className="modal-close-btn"
                  onClick={() => setViewingArtifact(null)}
                  aria-label="Close"
                >
                  ✕
                </button>
              </div>
            </div>

            {viewingArtifact.saveError && (
              <div className="artifact-edit-error-banner" role="alert">
                {viewingArtifact.saveError}
              </div>
            )}

            <div className="modal-body">
              <p className="artifact-modal-path"><code>{viewingArtifact.path}</code></p>
              {viewingArtifact.isEditing ? (
                <textarea
                  className="artifact-content-editor"
                  aria-label="Artifact Content Editor"
                  rows={18}
                  value={viewingArtifact.editedContent}
                  onChange={(e) =>
                    setViewingArtifact({
                      ...viewingArtifact,
                      editedContent: e.target.value,
                    })
                  }
                />
              ) : (
                <pre className="artifact-modal-content">{viewingArtifact.content}</pre>
              )}
            </div>
          </div>
        </div>
      )}

      {/* Freeze Confirmation Modal */}
      {isFreezeModalOpen && freezePreview && (
        <div className="modal-overlay" onClick={() => setIsFreezeModalOpen(false)}>
          <div
            className="modal-container freeze-modal"
            onClick={(e) => e.stopPropagation()}
            role="dialog"
            aria-labelledby="modal-freeze-title"
          >
            <div className="modal-header">
              <h2 id="modal-freeze-title">Confirm Architecture Freeze (v{freezePreview.target_version})</h2>
              <button
                className="modal-close-btn"
                onClick={() => setIsFreezeModalOpen(false)}
                aria-label="Close"
              >
                ✕
              </button>
            </div>

            <div className="modal-body">
              <div className="freeze-authority-notice">
                <strong>⚠️ Human Authority Authorization:</strong>
                <p>
                  Freezing the architecture creates an immutable contract baseline. AI agents and Builder workflows cannot alter these specifications once frozen.
                </p>
              </div>

              <div className="freeze-summary-cards">
                <div className="freeze-card">
                  <h4>Contract Readiness</h4>
                  <p>
                    {freezePreview.ready_required_count} of {freezePreview.total_required_count} required artifacts ready (Policy v{freezePreview.readiness_policy_version})
                  </p>
                  <p className="subdued-text">
                    {freezePreview.unresolved_open_questions_count} unresolved open questions
                  </p>
                </div>

                <div className="freeze-card">
                  <h4>Git Boundary</h4>
                  <p>
                    HEAD: <code>{freezePreview.git_boundary.head_commit.substring(0, 8)}</code>
                    {freezePreview.git_boundary.branch ? ` (${freezePreview.git_boundary.branch})` : ' (Detached)'}
                  </p>
                  <p className="subdued-text">
                    Status: {freezePreview.git_boundary.is_clean ? 'Clean working tree' : `${freezePreview.git_boundary.staged_count + freezePreview.git_boundary.unstaged_count + freezePreview.git_boundary.untracked_count} uncommitted changes`}
                  </p>
                </div>

                <div className="freeze-card">
                  <h4>Builder Packet</h4>
                  <p>
                    {freezePreview.builder_packet_summary.total_artifacts} artifacts (~{Math.round(freezePreview.builder_packet_summary.total_characters / 1024)} KB)
                  </p>
                  <p className="subdued-text">
                    ~{freezePreview.builder_packet_summary.estimated_tokens.toLocaleString()} estimated tokens {freezePreview.builder_packet_summary.is_truncated && '(Truncated to budget)'}
                  </p>
                </div>
              </div>

              <div className="freeze-artifacts-table">
                <h4>Contract Artifacts Baseline</h4>
                <table>
                  <thead>
                    <tr>
                      <th>Artifact Path</th>
                      <th>SHA-256 Fingerprint</th>
                    </tr>
                  </thead>
                  <tbody>
                    {Object.entries(freezePreview.artifact_baselines).map(([path, hash]) => (
                      <tr key={path}>
                        <td><code>{path}</code></td>
                        <td><code className="hash-code">{hash.substring(0, 16)}...</code></td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>

            <div className="modal-footer">
              <button
                className="secondary-btn cancel-freeze-btn"
                onClick={() => setIsFreezeModalOpen(false)}
                disabled={isLoading}
              >
                Cancel
              </button>
              <button
                className="primary-btn confirm-freeze-btn"
                onClick={handleConfirmFreeze}
                disabled={isLoading}
              >
                {isLoading ? 'Freezing...' : 'Authorize Architecture Freeze'}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Freeze Success Modal */}
      {isFreezeSuccess && (
        <div className="modal-overlay" onClick={() => setIsFreezeSuccess(null)}>
          <div
            className="modal-container freeze-success-modal"
            onClick={(e) => e.stopPropagation()}
            role="dialog"
            aria-labelledby="modal-freeze-success-title"
          >
            <div className="modal-header">
              <h2 id="modal-freeze-success-title">✓ Architecture Frozen Successfully</h2>
              <button
                className="modal-close-btn"
                onClick={() => setIsFreezeSuccess(null)}
                aria-label="Close"
              >
                ✕
              </button>
            </div>

            <div className="modal-body">
              <p>
                Architecture contract version <strong>v{isFreezeSuccess.architecture_version}</strong> has been frozen and committed to <code>.coalition/architecture-versions/v{isFreezeSuccess.architecture_version}/</code>.
              </p>
              <div className="freeze-success-details">
                <div>
                  <span className="label">Git Boundary Commit:</span>
                  <code>{isFreezeSuccess.git_boundary.head_commit.substring(0, 8)}</code>
                </div>
                <div>
                  <span className="label">Active Manifest Fingerprint:</span>
                  <code>{isFreezeSuccess.manifest_fingerprint.substring(0, 16)}...</code>
                </div>
                <div>
                  <span className="label">Builder Epoch ID:</span>
                  <code>{isFreezeSuccess.epoch_id.substring(0, 8)}...</code>
                </div>
              </div>
            </div>

            <div className="modal-footer">
              <button
                className="primary-btn close-success-btn"
                onClick={() => setIsFreezeSuccess(null)}
              >
                Done
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Drift Diff Inspector Modal */}
      {viewingDiff && (
        <div className="modal-overlay" onClick={() => setViewingDiff(null)}>
          <div
            className="modal-container diff-modal"
            onClick={(e) => e.stopPropagation()}
            role="dialog"
            aria-labelledby="modal-diff-title"
          >
            <div className="modal-header">
              <div className="diff-modal-title-row">
                <h2 id="modal-diff-title">Inspect Drift: <code>{viewingDiff.path}</code></h2>
                <span className={`diff-badge badge-${viewingDiff.drift_type.toLowerCase()}`}>
                  {viewingDiff.drift_type}
                </span>
              </div>
              <div className="modal-header-actions">
                <button
                  className="primary-btn restore-from-diff-btn"
                  onClick={() => handleRestoreDriftedArtifact(viewingDiff.path)}
                  disabled={isLoading}
                >
                  {viewingDiff.drift_type === 'ADDED' ? 'Quarantine Artifact' : 'Restore to Frozen'}
                </button>
                <button
                  className="modal-close-btn"
                  onClick={() => setViewingDiff(null)}
                  aria-label="Close"
                >
                  ✕
                </button>
              </div>
            </div>

            <div className="modal-body diff-modal-body">
              <div className="diff-columns">
                <div className="diff-column">
                  <h4>Frozen Contract (v1.0 Baseline)</h4>
                  <pre className="diff-code-box">
                    {viewingDiff.frozen_content ?? '(File does not exist in frozen snapshot)'}
                  </pre>
                </div>
                <div className="diff-column">
                  <h4>Active Working File</h4>
                  <pre className="diff-code-box">
                    {viewingDiff.active_content ?? '(File was deleted from active workspace)'}
                  </pre>
                </div>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Builder Packet Inspector Modal */}
      {isPacketModalOpen && builderPacket && (
        <div className="modal-overlay" onClick={() => setIsPacketModalOpen(false)}>
          <div
            className="modal-container packet-modal"
            onClick={(e) => e.stopPropagation()}
            role="dialog"
            aria-labelledby="modal-packet-title"
          >
            <div className="modal-header">
              <h2 id="modal-packet-title">
                Builder Implementation Packet (v{builderPacket.metadata.architecture_version})
              </h2>
              <button
                className="modal-close-btn"
                onClick={() => setIsPacketModalOpen(false)}
                aria-label="Close"
              >
                ✕
              </button>
            </div>

            <div className="modal-body">
              <div className="packet-meta-row">
                <span><strong>Epoch:</strong> <code>{builderPacket.metadata.builder_epoch_id.substring(0, 8)}</code></span>
                <span><strong>Git Commit:</strong> <code>{builderPacket.metadata.git_head_commit.substring(0, 8)}</code></span>
                <span><strong>Artifacts:</strong> {builderPacket.artifacts.length}</span>
                {builderPacket.is_truncated && <span className="warning-badge">Truncated to 200 KB Budget</span>}
              </div>

              <div className="packet-summary-section">
                <h4>Architecture Summary</h4>
                <p>{builderPacket.summary}</p>
              </div>

              <div className="packet-rules-section">
                <h4>Builder Invariants & Rules</h4>
                <pre className="packet-rules-box">{builderPacket.builder_rules}</pre>
              </div>

              <div className="packet-artifacts-section">
                <h4>Included Contract Artifacts</h4>
                <div className="packet-artifact-cards">
                  {builderPacket.artifacts.map((a) => (
                    <div key={a.path} className="packet-artifact-card">
                      <div className="packet-card-header">
                        <strong>{a.title}</strong>
                        <code>{a.path}</code>
                      </div>
                      <pre className="packet-artifact-content">{a.content}</pre>
                    </div>
                  ))}
                </div>
              </div>
            </div>

            <div className="modal-footer">
              <button
                className="primary-btn close-packet-btn"
                onClick={() => setIsPacketModalOpen(false)}
              >
                Close
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};

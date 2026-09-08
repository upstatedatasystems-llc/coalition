import React, { useEffect, useState, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  WorkspaceState,
  RelayPacket,
  ImportPreview,
  ReadinessReport,
  ArtifactReadinessItem,
  ArtifactContentDetails,
  CommandError,
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

  // Keyboard shortcuts: in-window Ctrl+Shift+R and Ctrl+Shift+I, and Tauri global shortcuts
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

    let unlistenCopy: (() => void) | undefined;
    let unlistenImport: (() => void) | undefined;
    const registerGlobalShortcuts = async () => {
      try {
        const { listen } = await import('@tauri-apps/api/event');
        unlistenCopy = await listen('coalition:shortcut-copy-relay', () => {
          handleCopyPacket();
        });
        unlistenImport = await listen('coalition:shortcut-import-clipboard', () => {
          handleImportClipboard();
        });
      } catch (_e) {
        // Ignored in non-Tauri / test environments
      }
    };
    registerGlobalShortcuts();

    return () => {
      window.removeEventListener('keydown', handleKeyDown);
      if (unlistenCopy) unlistenCopy();
      if (unlistenImport) unlistenImport();
    };
  }, [handleCopyPacket, handleImportClipboard]);

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
            {workspace?.readiness && (
              <span
                className={`readiness-badge readiness-${workspace.readiness.overall_readiness.toLowerCase()}`}
              >
                {workspace.readiness.overall_readiness === 'READY_TO_FREEZE'
                  ? `READY TO FREEZE (${workspace.readiness.ready_required_count ?? workspace.readiness.ready_count ?? 0}/${workspace.readiness.total_required_count ?? workspace.readiness.total_required ?? 0})`
                  : `INCOMPLETE (${workspace.readiness.ready_required_count ?? workspace.readiness.ready_count ?? 0}/${workspace.readiness.total_required_count ?? workspace.readiness.total_required ?? 0})`}
              </span>
            )}
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
                      <span className={`applicability-badge badge-${(art.applicability || 'REQUIRED').toLowerCase()}`}>
                        {art.applicability || 'REQUIRED'}
                      </span>
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
    </div>
  );
};

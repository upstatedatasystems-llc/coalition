import React, { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  ReviewCycleRecord,
  ReviewImportPreview,
  ReviewFindingRecord,
} from '../../types';

interface ReviewControlViewProps {
  projectId: string;
  projectName: string;
  workflowState: string;
  onRefreshProject: () => Promise<void>;
  onNavigateToBuilder?: () => void;
}

export const ReviewControlView: React.FC<ReviewControlViewProps> = ({
  projectId,
  projectName: _projectName,
  workflowState,
  onRefreshProject,
  onNavigateToBuilder,
}) => {
  const [latestCycle, setLatestCycle] = useState<ReviewCycleRecord | null>(null);
  const [selectedCycle, setSelectedCycle] = useState<ReviewCycleRecord | null>(null);
  const [history, setHistory] = useState<ReviewCycleRecord[]>([]);
  const [isLoading, setIsLoading] = useState<boolean>(true);
  const [isPreparingPacket, setIsPreparingPacket] = useState<boolean>(false);
  const [preparedPacketText, setPreparedPacketText] = useState<string | null>(null);
  const [copiedPacket, setCopiedPacket] = useState<boolean>(false);

  // Reviewer response import state
  const [reviewerResponseText, setReviewerResponseText] = useState<string>('');
  const [importPreview, setImportPreview] = useState<ReviewImportPreview | null>(null);
  const [isPreviewing, setIsPreviewing] = useState<boolean>(false);
  const [isConfirming, setIsConfirming] = useState<boolean>(false);

  // Builder correction turn state
  const [isStartingCorrections, setIsStartingCorrections] = useState<boolean>(false);

  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [successMessage, setSuccessMessage] = useState<string | null>(null);

  const activeCycle = selectedCycle || latestCycle;

  const loadData = useCallback(async () => {
    try {
      setErrorMessage(null);
      const [latest, hist] = await Promise.all([
        invoke<ReviewCycleRecord | null>('get_latest_review_cycle', { projectId }),
        invoke<ReviewCycleRecord[]>('list_review_cycles', { projectId, limit: 20 }),
      ]);
      setLatestCycle(latest);
      setHistory(hist);
      if (!selectedCycle && latest) {
        setSelectedCycle(latest);
      } else if (selectedCycle) {
        const updated = hist.find((c) => c.cycle_id === selectedCycle.cycle_id);
        if (updated) setSelectedCycle(updated);
      }
    } catch (e: unknown) {
      const err = e as { message?: string };
      setErrorMessage(err.message || 'Failed to load review cycle data');
    } finally {
      setIsLoading(false);
    }
  }, [projectId, selectedCycle]);

  useEffect(() => {
    loadData();
  }, [loadData]);

  const handlePreparePacket = async () => {
    try {
      setIsPreparingPacket(true);
      setErrorMessage(null);
      setSuccessMessage(null);
      const resp = await invoke<any>('prepare_review_packet', { projectId });
      const cycle: ReviewCycleRecord = resp?.cycle || resp;
      const packetText: string = resp?.packet || '';
      setPreparedPacketText(packetText);
      setLatestCycle(cycle);
      setSelectedCycle(cycle);
      setSuccessMessage(`Review packet prepared successfully for Cycle #${cycle.cycle_number}`);
      await loadData();
      await onRefreshProject();
    } catch (e: unknown) {
      const err = e as { message?: string };
      setErrorMessage(err.message || 'Failed to prepare review packet');
    } finally {
      setIsPreparingPacket(false);
    }
  };

  const handleCopyPacket = async () => {
    let textToCopy = preparedPacketText;
    const cycleToCopy = activeCycle || latestCycle;
    if (!textToCopy && cycleToCopy) {
      try {
        textToCopy = await invoke<string>('get_review_packet_content', {
          projectId,
          cycleId: cycleToCopy.cycle_id,
        });
      } catch (err) {
        console.warn('Failed to fetch review packet from disk:', err);
      }
    }
    if (!textToCopy) {
      textToCopy =
        latestCycle?.corrections_packet ||
        latestCycle?.summary ||
        (latestCycle ? `Review Packet for Cycle #${latestCycle.cycle_number}\nHash: ${latestCycle.review_packet_hash}` : '');
    }
    if (!textToCopy) return;
    try {
      await navigator.clipboard.writeText(textToCopy);
      setCopiedPacket(true);
      setTimeout(() => setCopiedPacket(false), 3000);
    } catch {
      // Fallback to desktop_clipboard_write
      try {
        await invoke('desktop_clipboard_write', {
          text: textToCopy,
        });
        setCopiedPacket(true);
        setTimeout(() => setCopiedPacket(false), 3000);
      } catch (err: unknown) {
        const e = err as { message?: string };
        setErrorMessage(e.message || 'Failed to copy to clipboard');
      }
    }
  };

  const handleOpenChatGpt = async () => {
    try {
      await invoke('open_chatgpt');
    } catch (e: unknown) {
      const err = e as { message?: string };
      setErrorMessage(err.message || 'Failed to open ChatGPT');
    }
  };

  const handlePreviewImport = async () => {
    if (!latestCycle) {
      setErrorMessage('No active review cycle found to import into');
      return;
    }
    if (!reviewerResponseText.trim()) {
      setErrorMessage('Please paste the reviewer response first');
      return;
    }

    try {
      setIsPreviewing(true);
      setErrorMessage(null);
      const preview = await invoke<ReviewImportPreview>('prepare_review_import', {
        projectId,
        cycleId: latestCycle.cycle_id,
        reviewerResponse: reviewerResponseText,
      });
      setImportPreview(preview);
    } catch (e: unknown) {
      const err = e as { message?: string };
      setErrorMessage(err.message || 'Failed to parse reviewer response');
    } finally {
      setIsPreviewing(false);
    }
  };

  const handleConfirmImport = async () => {
    if (!importPreview) return;
    try {
      setIsConfirming(true);
      setErrorMessage(null);
      const confirmedCycle = await invoke<ReviewCycleRecord>('confirm_review_import', {
        projectId,
        previewId: importPreview.preview_id,
      });
      setImportPreview(null);
      setReviewerResponseText('');
      setLatestCycle(confirmedCycle);
      setSelectedCycle(confirmedCycle);
      setSuccessMessage(
        `Review imported successfully! Verdict: ${confirmedCycle.verdict || confirmedCycle.status}`
      );
      await loadData();
      await onRefreshProject();
    } catch (e: unknown) {
      const err = e as { message?: string };
      setErrorMessage(err.message || 'Failed to confirm review import');
    } finally {
      setIsConfirming(false);
    }
  };

  const handleStartCorrections = async () => {
    if (!latestCycle) return;
    try {
      setIsStartingCorrections(true);
      setErrorMessage(null);
      await invoke('start_builder_turn', {
        payload: {
          projectId,
          instructionSource: {
            type: 'REVIEW_CORRECTION',
            review_cycle_id: latestCycle.cycle_id,
          },
        },
      });
      setSuccessMessage('Builder corrections turn started successfully!');
      if (onNavigateToBuilder) {
        onNavigateToBuilder();
      }
    } catch (e: unknown) {
      const err = e as { message?: string };
      setErrorMessage(err.message || 'Failed to start builder corrections turn');
    } finally {
      setIsStartingCorrections(false);
    }
  };

  return (
    <div className="review-control-container" data-testid="review-control-view">
      {/* Top Banner & Action Controls */}
      <div className="review-top-bar">
        <div className="review-status-info">
          <h2>Independent Review & Correction Loop</h2>
          <div className="status-badges">
            <span
              className={`workflow-badge badge-${workflowState.toLowerCase()}`}
              data-testid="workflow-state-badge"
            >
              Workflow: {workflowState}
            </span>
            {activeCycle && (
              <span
                className={`cycle-verdict-badge verdict-${(activeCycle.verdict || activeCycle.status).toLowerCase()}`}
                data-testid="cycle-verdict-badge"
              >
                Verdict: {activeCycle.verdict || activeCycle.status}
              </span>
            )}
          </div>
        </div>

        <div className="review-top-actions">
          <button
            className="primary-btn prepare-packet-btn"
            onClick={handlePreparePacket}
            disabled={isPreparingPacket || workflowState !== 'WAITING_FOR_REVIEW'}
            data-testid="prepare-review-packet-btn"
            title={
              workflowState !== 'WAITING_FOR_REVIEW'
                ? 'Workflow state must be WAITING_FOR_REVIEW to prepare packet'
                : 'Prepare a new review packet for ChatGPT relay'
            }
          >
            {isPreparingPacket ? 'Preparing Packet...' : 'Prepare Review Packet'}
          </button>
          <button
            className="secondary-btn refresh-review-btn"
            onClick={loadData}
            disabled={isLoading}
            data-testid="refresh-review-btn"
          >
            Refresh
          </button>
        </div>
      </div>

      {errorMessage && (
        <div className="error-banner" role="alert" data-testid="review-error-banner">
          {errorMessage}
          <button className="dismiss-btn" onClick={() => setErrorMessage(null)}>
            ×
          </button>
        </div>
      )}

      {successMessage && (
        <div className="success-banner" role="status" data-testid="review-success-banner">
          {successMessage}
          <button className="dismiss-btn" onClick={() => setSuccessMessage(null)}>
            ×
          </button>
        </div>
      )}

      <div className="review-main-grid">
        {/* Left Column: Active / Selected Review Cycle & Relay */}
        <div className="review-content-col">
          {activeCycle ? (
            <div className="review-cycle-card" data-testid="active-cycle-card">
              <div className="cycle-header">
                <h3>
                  Review Cycle #{activeCycle.cycle_number}
                  {activeCycle.cycle_id === latestCycle?.cycle_id && (
                    <span className="latest-tag"> (Latest)</span>
                  )}
                </h3>
                <span className="cycle-status-pill">{activeCycle.status}</span>
              </div>

              <div className="cycle-metadata-grid">
                <div>
                  <span className="label">Architecture Version:</span>
                  <span>{activeCycle.architecture_version}</span>
                </div>
                <div>
                  <span className="label">Git HEAD:</span>
                  <code>{activeCycle.git_head ? activeCycle.git_head.slice(0, 10) : 'None'}</code>
                </div>
                <div>
                  <span className="label">Packet Hash:</span>
                  <code title={activeCycle.review_packet_hash}>
                    {activeCycle.review_packet_hash.slice(0, 12)}...
                  </code>
                </div>
                <div>
                  <span className="label">Started:</span>
                  <span>{new Date(activeCycle.started_at).toLocaleString()}</span>
                </div>
              </div>

              {/* Review Packet Relay Actions */}
              <div className="relay-actions-panel" data-testid="relay-actions-panel">
                <h4>Human Relay (ChatGPT Reviewer)</h4>
                <p className="relay-desc">
                  Copy the review packet containing the architecture contract, validation evidence,
                  and bounded diff, then relay it to the independent reviewer.
                </p>
                <div className="relay-buttons-row">
                  <button
                    className="secondary-btn copy-packet-btn"
                    onClick={handleCopyPacket}
                    data-testid="copy-packet-btn"
                  >
                    {copiedPacket ? '✓ Copied to Clipboard!' : 'Copy Review Packet'}
                  </button>
                  <button
                    className="secondary-btn open-chatgpt-btn"
                    onClick={handleOpenChatGpt}
                    data-testid="open-chatgpt-btn"
                  >
                    Open ChatGPT ↗
                  </button>
                </div>
              </div>

              {/* Summary / Verdict if available */}
              {activeCycle.summary && (
                <div className="cycle-summary-panel" data-testid="cycle-summary-panel">
                  <h4>Reviewer Summary</h4>
                  <p>{activeCycle.summary}</p>
                </div>
              )}

              {/* Findings Section */}
              <div className="cycle-findings-section" data-testid="findings-section">
                <h4>Review Findings ({activeCycle.findings?.length || 0})</h4>
                {!activeCycle.findings || activeCycle.findings.length === 0 ? (
                  <p className="empty-findings-msg">No findings reported in this cycle.</p>
                ) : (
                  <div className="findings-list">
                    {activeCycle.findings.map((f: ReviewFindingRecord) => (
                      <div
                        key={f.finding_id}
                        className={`finding-item severity-${f.severity.toLowerCase()}`}
                        data-testid={`finding-item-${f.finding_id}`}
                      >
                        <div className="finding-header">
                          <span className={`severity-badge sev-${f.severity.toLowerCase()}`}>
                            {f.severity}
                          </span>
                          <span className="finding-title">{f.title}</span>
                          {f.is_repeat && (
                            <span
                              className="repeat-finding-badge"
                              data-testid="repeat-finding-badge"
                            >
                              REPEAT FINDING
                            </span>
                          )}
                          <span className="finding-status-tag">{f.status}</span>
                        </div>
                        {(f.file_path || f.line_range) && (
                          <div className="finding-location">
                            <code>
                              {f.file_path || 'Unknown file'}
                              {f.line_range ? `:${f.line_range}` : ''}
                            </code>
                          </div>
                        )}
                        {f.requirement_references && (
                          <div className="finding-req-refs" data-testid="finding-req-refs">
                            <strong>Requirement:</strong>{' '}
                            <code>
                              {Array.isArray(f.requirement_references)
                                ? f.requirement_references.join(', ')
                                : f.requirement_references}
                            </code>
                          </div>
                        )}
                        {f.problem_statement && (
                          <div className="finding-problem" data-testid="finding-problem">
                            <strong>Problem:</strong> <p>{f.problem_statement}</p>
                          </div>
                        )}
                        <p className="finding-desc">{f.description}</p>
                        {f.required_change && (
                          <div className="finding-required-change" data-testid="finding-required-change">
                            <strong>Required Change:</strong> <p>{f.required_change}</p>
                          </div>
                        )}
                        {f.required_test && (
                          <div className="finding-required-test" data-testid="finding-required-test">
                            <strong>Required Test:</strong> <p>{f.required_test}</p>
                          </div>
                        )}
                        {f.suggested_fix && (
                          <div className="finding-suggestion">
                            <strong>Suggested Fix:</strong>
                            <p>{f.suggested_fix}</p>
                          </div>
                        )}
                      </div>
                    ))}
                  </div>
                )}
              </div>

              {/* Corrections Packet / Builder Action */}
              {activeCycle.corrections_packet && (
                <div className="corrections-packet-section" data-testid="corrections-packet-section">
                  <div className="corrections-header">
                    <h4>Generated Corrections Packet</h4>
                    {workflowState === 'CORRECTIONS_REQUIRED' && (
                      <button
                        className="primary-btn start-corrections-btn"
                        onClick={handleStartCorrections}
                        disabled={isStartingCorrections}
                        data-testid="start-builder-corrections-btn"
                      >
                        {isStartingCorrections
                          ? 'Starting Builder...'
                          : 'Start Builder Corrections Turn'}
                      </button>
                    )}
                  </div>
                  <pre className="corrections-preview" data-testid="corrections-content">
                    {activeCycle.corrections_packet}
                  </pre>
                </div>
              )}
            </div>
          ) : (
            <div className="empty-cycle-card" data-testid="empty-cycle-card">
              <h3>No Review Cycles Yet</h3>
              <p>
                When your validation suite passes, submit the project for review and prepare the
                first review packet here.
              </p>
              {workflowState === 'WAITING_FOR_REVIEW' && (
                <button
                  className="primary-btn"
                  onClick={handlePreparePacket}
                  disabled={isPreparingPacket}
                  data-testid="empty-prepare-packet-btn"
                >
                  {isPreparingPacket ? 'Preparing...' : 'Prepare Initial Review Packet'}
                </button>
              )}
            </div>
          )}

          {/* Import Reviewer Response Form */}
          {activeCycle && activeCycle.status === 'PENDING' && (
            <div className="import-response-card" data-testid="import-response-card">
              <h3>Import Reviewer Response</h3>
              <p className="import-desc">
                Paste the markdown, YAML, or JSON verdict envelope received from the reviewer.
              </p>
              <textarea
                className="response-textarea"
                rows={8}
                placeholder="Paste ```verdict or ```yaml response block here..."
                value={reviewerResponseText}
                onChange={(e) => setReviewerResponseText(e.target.value)}
                data-testid="reviewer-response-input"
              />
              <div className="import-actions-row">
                <button
                  className="primary-btn preview-import-btn"
                  onClick={handlePreviewImport}
                  disabled={isPreviewing || !reviewerResponseText.trim()}
                  data-testid="preview-import-btn"
                >
                  {isPreviewing ? 'Parsing...' : 'Preview Import'}
                </button>
              </div>
            </div>
          )}
        </div>

        {/* Right Column: History Sidebar */}
        <div className="review-history-col">
          <div className="history-card" data-testid="review-history-card">
            <h3>Review History</h3>
            {history.length === 0 ? (
              <p className="empty-history-text">No past review cycles recorded.</p>
            ) : (
              <ul className="history-cycle-list">
                {history.map((cycle) => (
                  <li
                    key={cycle.cycle_id}
                    className={`history-cycle-item ${
                      selectedCycle?.cycle_id === cycle.cycle_id ? 'selected' : ''
                    }`}
                    onClick={() => setSelectedCycle(cycle)}
                    data-testid={`history-cycle-${cycle.cycle_number}`}
                  >
                    <div className="history-item-top">
                      <span className="cycle-num">Cycle #{cycle.cycle_number}</span>
                      <span
                        className={`history-verdict-tag verdict-${(cycle.verdict || cycle.status).toLowerCase()}`}
                      >
                        {cycle.verdict || cycle.status}
                      </span>
                    </div>
                    <div className="history-item-meta">
                      <span>{new Date(cycle.started_at).toLocaleDateString()}</span>
                      <span>{cycle.findings?.length || 0} findings</span>
                    </div>
                  </li>
                ))}
              </ul>
            )}
          </div>
        </div>
      </div>

      {/* Import Preview Modal */}
      {importPreview && (
        <div className="modal-overlay" data-testid="import-preview-modal">
          <div className="modal-card import-preview-modal-content">
            <div className="modal-header">
              <h3>Review Import Preview</h3>
              <button
                className="close-btn"
                onClick={() => setImportPreview(null)}
                data-testid="close-preview-modal-btn"
              >
                ×
              </button>
            </div>
            <div className="modal-body">
              <div className="preview-summary-box">
                <div className="verdict-banner">
                  <span className="label">Parsed Verdict:</span>
                  <span
                    className={`verdict-badge verdict-${importPreview.verdict.toLowerCase()}`}
                    data-testid="preview-verdict"
                  >
                    {importPreview.verdict}
                  </span>
                </div>
                <div className="summary-section">
                  <span className="label">Reviewer Summary:</span>
                  <p data-testid="preview-summary">{importPreview.summary}</p>
                </div>
              </div>

              <div className="preview-findings-box">
                <h4>Extracted Findings ({importPreview.findings.length})</h4>
                {importPreview.findings.length === 0 ? (
                  <p className="no-findings">No findings extracted.</p>
                ) : (
                  <ul className="preview-findings-list" data-testid="preview-findings-list">
                    {importPreview.findings.map((finding, idx) => (
                      <li key={idx} className="preview-finding-item">
                        <div className="preview-finding-header">
                          <span className="severity-badge">{finding.severity || 'MAJOR'}</span>
                          <strong>{finding.title}</strong>
                        </div>
                        {finding.file && (
                          <div className="finding-file">
                            <code>
                              {finding.file}
                              {finding.lines ? `:${finding.lines}` : ''}
                            </code>
                          </div>
                        )}
                        {finding.requirement_references && (
                          <div className="finding-req-refs">
                            <strong>Requirement:</strong>{' '}
                            <code>
                              {Array.isArray(finding.requirement_references)
                                ? finding.requirement_references.join(', ')
                                : finding.requirement_references}
                            </code>
                          </div>
                        )}
                        {finding.problem_statement && (
                          <div className="finding-problem">
                            <strong>Problem:</strong> <p>{finding.problem_statement}</p>
                          </div>
                        )}
                        <p>{finding.description}</p>
                        {finding.required_change && (
                          <div className="finding-required-change">
                            <strong>Required Change:</strong> <p>{finding.required_change}</p>
                          </div>
                        )}
                        {finding.required_test && (
                          <div className="finding-required-test">
                            <strong>Required Test:</strong> <p>{finding.required_test}</p>
                          </div>
                        )}
                        {finding.suggested_fix && (
                          <div className="suggested-fix">
                            <em>Suggested Fix:</em> {finding.suggested_fix}
                          </div>
                        )}
                      </li>
                    ))}
                  </ul>
                )}
              </div>

              {importPreview.corrections_preview && (
                <div className="preview-corrections-box">
                  <h4>Corrections Preview</h4>
                  <pre>{importPreview.corrections_preview}</pre>
                </div>
              )}
            </div>

            <div className="modal-footer">
              <button
                className="secondary-btn"
                onClick={() => setImportPreview(null)}
                disabled={isConfirming}
                data-testid="cancel-preview-btn"
              >
                Cancel
              </button>
              <button
                className="primary-btn confirm-import-btn"
                onClick={handleConfirmImport}
                disabled={isConfirming}
                data-testid="confirm-import-btn"
              >
                {isConfirming ? 'Confirming...' : 'Confirm Review Import'}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};

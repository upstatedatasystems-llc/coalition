import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { SystemDiagnosticInfo, ProofResult, BuilderTurnResponse } from '../../types';

export const DiagnosticsView: React.FC = () => {
  const [diagnostics, setDiagnostics] = useState<SystemDiagnosticInfo | null>(null);
  const [ipcStatus, setIpcStatus] = useState<string>('Connecting...');
  const [sqliteProof, setSqliteProof] = useState<ProofResult | null>(null);
  const [sqliteError, setSqliteError] = useState<string | null>(null);

  // Builder test state
  const [prompt, setPrompt] = useState<string>('respond with pong');
  const [selectedModel, setSelectedModel] = useState<string>('gemini-3.8-flash-high');
  const [effort, setEffort] = useState<string>('medium');
  const [useFakeAgy, setUseFakeAgy] = useState<boolean>(true);
  const [testIcarus, setTestIcarus] = useState<boolean>(false);
  const [activeConversationId, setActiveConversationId] = useState<string>('');
  const [isExecuting, setIsExecuting] = useState<boolean>(false);
  const [eventLogs, setEventLogs] = useState<string[]>([]);
  const [lastResponse, setLastResponse] = useState<BuilderTurnResponse | null>(null);

  // Desktop proofs
  const [clipboardText, setClipboardText] = useState<string>('Coalition-Proof-Token');
  const [readClipboardResult, setReadClipboardResult] = useState<string>('');
  const [urlToOpen, setUrlToOpen] = useState<string>('https://github.com/upstatedatasystems-llc/coalition');
  const [desktopMessage, setDesktopMessage] = useState<string>('');

  useEffect(() => {
    loadDiagnostics();

    let unlisten: (() => void) | undefined;
    let canceled = false;

    listen<unknown>('agy-stream-event', (event) => {
      const line = JSON.stringify(event.payload);
      setEventLogs((prev) => [...prev, line]);
    })
      .then((fn) => {
        if (canceled) {
          fn();
        } else {
          unlisten = fn;
        }
      })
      .catch((err) => {
        console.error('Failed to register stream listener:', err);
      });

    return () => {
      canceled = true;
      if (unlisten) {
        unlisten();
      }
    };
  }, []);

  const loadDiagnostics = async () => {
    try {
      const result = await invoke<SystemDiagnosticInfo>('get_system_diagnostics');
      setDiagnostics(result);
      setIpcStatus('Connected (Rust Tauri IPC OK)');
    } catch (err: unknown) {
      setIpcStatus(`IPC Error: ${err instanceof Error ? err.message : JSON.stringify(err)}`);
    }
  };

  const handleRunSqliteProof = async () => {
    try {
      setSqliteError(null);
      const result = await invoke<ProofResult>('run_sqlite_proof');
      setSqliteProof(result);
    } catch (err: unknown) {
      setSqliteError(err instanceof Error ? err.message : JSON.stringify(err));
    }
  };

  const handleExecuteTurn = async () => {
    setIsExecuting(true);
    setLastResponse(null);
    try {
      const resp = await invoke<BuilderTurnResponse>('start_builder_turn', {
        payload: {
          prompt,
          conversation_id: activeConversationId || null,
          model: selectedModel,
          effort,
          icarus_mode: testIcarus,
          use_fake_agy: useFakeAgy,
        },
      });
      setLastResponse(resp);
      if (resp.conversation_id) {
        setActiveConversationId(resp.conversation_id);
      }
    } catch (err: unknown) {
      alert(`Turn Execution Failed: ${err instanceof Error ? err.message : JSON.stringify(err)}`);
    } finally {
      setIsExecuting(false);
    }
  };

  const handleCancelTurn = async () => {
    try {
      await invoke('cancel_builder_turn');
    } catch (err: unknown) {
      console.error('Failed to cancel turn:', err);
    }
  };

  const handleClipboardWrite = async () => {
    try {
      await invoke('desktop_clipboard_write', { text: clipboardText });
      setDesktopMessage('Wrote text to OS clipboard.');
    } catch (err: unknown) {
      setDesktopMessage(`Clipboard write failed: ${err instanceof Error ? err.message : JSON.stringify(err)}`);
    }
  };

  const handleClipboardRead = async () => {
    try {
      const read = await invoke<string>('desktop_clipboard_read');
      setReadClipboardResult(read);
      setDesktopMessage('Read text from OS clipboard.');
    } catch (err: unknown) {
      setDesktopMessage(`Clipboard read failed: ${err instanceof Error ? err.message : JSON.stringify(err)}`);
    }
  };

  const handleOpenUrl = async () => {
    try {
      await invoke('desktop_open_url', { url: urlToOpen });
      setDesktopMessage(`Opened URL in external default browser.`);
    } catch (err: unknown) {
      setDesktopMessage(`Failed to open URL: ${err instanceof Error ? err.message : JSON.stringify(err)}`);
    }
  };

  return (
    <div className="diagnostics-view">
      <header className="diagnostics-header">
        <h2>Coalition — Phase 0 Technical Diagnostics</h2>
        <div className="ipc-badge">IPC Status: {ipcStatus}</div>
      </header>

      <section className="diagnostic-card">
        <h3>1. System &amp; Runtime Proofs</h3>
        <p><strong>Current Directory:</strong> <code>{diagnostics?.current_dir || 'Loading...'}</code></p>
        <p><strong>Git Detected:</strong> {diagnostics?.git ? 'Yes' : 'No'}</p>
        {diagnostics?.git && (
          <ul>
            <li>Version: {diagnostics.git.git_version}</li>
            <li>Repository Root: {diagnostics.git.root_dir || 'None'}</li>
            <li>Branch: {diagnostics.git.current_branch || 'None'}</li>
            <li>HEAD: {diagnostics.git.head_commit || 'None'}</li>
            <li>Staged: {diagnostics.git.status.staged}, Unstaged: {diagnostics.git.status.unstaged}, Untracked: {diagnostics.git.status.untracked}</li>
          </ul>
        )}
        <p><strong>Antigravity Detected:</strong> {diagnostics?.agy_detected ? 'Yes' : 'No'}</p>
        {diagnostics?.agy_detected && (
          <ul>
            <li>Path: <code>{diagnostics.agy_path}</code></li>
            <li>Version: {diagnostics.agy_version || 'Unknown'}</li>
            <li>Models Available: {diagnostics.agy_models.length}</li>
          </ul>
        )}
        {diagnostics?.agy_error && (
          <p className="error-text">CLI Error: {diagnostics.agy_error}</p>
        )}
      </section>

      <section className="diagnostic-card">
        <h3>2. SQLite Migration &amp; Operational State Proof</h3>
        <button className="primary-btn" onClick={handleRunSqliteProof}>
          Run SQLite Migration Proof
        </button>
        {sqliteError && <p className="error-text">SQLite Error: {sqliteError}</p>}
        {sqliteProof && (
          <div className="proof-details">
            <p><strong>Applied Migrations:</strong> {sqliteProof.applied_migrations.length}</p>
            <p><strong>Inserted Test Record ID:</strong> {sqliteProof.test_record_id}</p>
            <p><strong>Test Record Message:</strong> {sqliteProof.test_record_message}</p>
            <p><strong>Total Operational Records in Table:</strong> {sqliteProof.total_records}</p>
          </div>
        )}
      </section>

      <section className="diagnostic-card">
        <h3>3. Antigravity Builder Adapter &amp; NDJSON Stream Proof</h3>
        <div className="form-group">
          <label>Prompt:</label>
          <input
            type="text"
            className="text-input"
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
          />
        </div>

        <div className="form-row">
          <label>Model:</label>
          <select
            value={selectedModel}
            onChange={(e) => setSelectedModel(e.target.value)}
          >
            {diagnostics?.agy_models?.length ? (
              diagnostics.agy_models.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.name} ({m.id})
                </option>
              ))
            ) : (
              <option value="gemini-3.8-flash-high">Gemini 3.8 Flash (High)</option>
            )}
          </select>

          <label>Effort:</label>
          <select value={effort} onChange={(e) => setEffort(e.target.value)}>
            <option value="low">Low</option>
            <option value="medium">Medium</option>
            <option value="high">High</option>
          </select>

          <label>
            <input
              type="checkbox"
              checked={useFakeAgy}
              onChange={(e) => setUseFakeAgy(e.target.checked)}
            />
            Use fake-agy test double (0 quota)
          </label>

          <label>
            <input
              type="checkbox"
              checked={testIcarus}
              onChange={(e) => setTestIcarus(e.target.checked)}
            />
            Icarus Mode (Mock flag)
          </label>
        </div>

        <div className="btn-row">
          <button
            className="primary-btn"
            disabled={isExecuting}
            onClick={handleExecuteTurn}
          >
            {isExecuting ? 'Executing...' : 'Execute Turn'}
          </button>
          <button
            className="secondary-btn"
            disabled={!isExecuting}
            onClick={handleCancelTurn}
          >
            Cancel Active Turn
          </button>
          <button
            className="secondary-btn"
            onClick={() => setEventLogs([])}
          >
            Clear Stream Logs
          </button>
        </div>

        {lastResponse && (
          <div className="response-box">
            <h4>Last Response</h4>
            <p>Conversation ID: <code>{lastResponse.conversation_id}</code></p>
            <pre>{JSON.stringify(lastResponse, null, 2)}</pre>
          </div>
        )}

        <div className="stream-logs-box">
          <h4>Stream Logs (NDJSON Events)</h4>
          <textarea
            aria-label="Stream Logs"
            readOnly
            value={eventLogs.join('\n')}
            rows={8}
            className="log-textarea"
          />
        </div>
      </section>

      <section className="diagnostic-card">
        <h3>4. Desktop Clipboard &amp; Native Opener Proofs</h3>
        <div className="form-group">
          <label>Clipboard Text:</label>
          <input
            type="text"
            className="text-input"
            value={clipboardText}
            onChange={(e) => setClipboardText(e.target.value)}
          />
          <div className="btn-row">
            <button className="primary-btn" onClick={handleClipboardWrite}>
              Write to Clipboard
            </button>
            <button className="secondary-btn" onClick={handleClipboardRead}>
              Read from Clipboard
            </button>
          </div>
          {readClipboardResult && (
            <p><strong>Read back:</strong> <code>{readClipboardResult}</code></p>
          )}
        </div>

        <div className="form-group">
          <label>URL to open in external browser:</label>
          <input
            type="text"
            className="text-input"
            value={urlToOpen}
            onChange={(e) => setUrlToOpen(e.target.value)}
          />
          <button className="primary-btn" onClick={handleOpenUrl}>
            Open URL via OS
          </button>
        </div>

        {desktopMessage && <p className="info-text">{desktopMessage}</p>}
      </section>
    </div>
  );
};

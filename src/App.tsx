import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import './App.css';

interface GitStatusCounts {
  staged: number;
  unstaged: number;
  untracked: number;
}

interface GitRepoInfo {
  git_version: string;
  is_repo: boolean;
  root_dir: string | null;
  current_branch: string | null;
  head_commit: string | null;
  status: GitStatusCounts;
  diff_summary: string;
}

interface ModelInfo {
  id: string;
  name: string;
}

interface SystemDiagnosticInfo {
  current_dir: string;
  git: GitRepoInfo | null;
  agy_detected: boolean;
  agy_path: string | null;
  agy_version: string | null;
  agy_models: ModelInfo[];
  agy_error: string | null;
}

interface MigrationRecord {
  version: number;
  name: string;
  applied_at: string;
}

interface ProofResult {
  applied_migrations: MigrationRecord[];
  test_record_id: number;
  test_record_message: string;
  total_records: number;
}

interface AgyUsage {
  input_tokens: number;
  output_tokens: number;
  thinking_tokens: number;
  cache_read_tokens: number;
  total_tokens: number;
}

interface BuilderTurnResponse {
  conversation_id: string | null;
  status: string;
  text_response: string;
  cumulative_usage: AgyUsage;
  was_canceled: boolean;
}

export const App: React.FC = () => {
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
    // Check Tauri IPC & get diagnostics
    loadDiagnostics();

    // Listen for live NDJSON stream events
    let unlisten: (() => void) | undefined;
    listen<unknown>('agy-stream-event', (event) => {
      const line = JSON.stringify(event.payload);
      setEventLogs((prev) => [...prev, line]);
    }).then((unsub) => {
      unlisten = unsub;
    }).catch((err) => {
      console.error('Failed to register stream listener', err);
    });

    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  const loadDiagnostics = async () => {
    try {
      const diag = await invoke<SystemDiagnosticInfo>('get_system_diagnostics');
      setDiagnostics(diag);
      setIpcStatus('OK');
      if (diag.agy_models && diag.agy_models.length > 0) {
        setSelectedModel(diag.agy_models[0].id);
      }
    } catch (err: unknown) {
      setIpcStatus(`IPC Error: ${String(err)}`);
    }
  };

  const handleRunSqliteProof = async () => {
    try {
      setSqliteError(null);
      const res = await invoke<ProofResult>('run_sqlite_proof');
      setSqliteProof(res);
    } catch (err: unknown) {
      setSqliteError(String(err));
    }
  };

  const handleExecuteBuilderTurn = async () => {
    setIsExecuting(true);
    setLastResponse(null);
    setEventLogs([`[INFO] Starting turn with prompt: "${prompt}" (use_fake_agy=${useFakeAgy})`]);

    try {
      const resp = await invoke<BuilderTurnResponse>('start_builder_turn', {
        payload: {
          prompt,
          conversation_id: activeConversationId || null,
          model: selectedModel || null,
          effort,
          icarus_mode: testIcarus,
          use_fake_agy: useFakeAgy,
        },
      });
      setLastResponse(resp);
      if (resp.conversation_id) {
        setActiveConversationId(resp.conversation_id);
      }
      setEventLogs((prev) => [
        ...prev,
        `[COMPLETE] Status=${resp.status} Canceled=${resp.was_canceled} TotalTokens=${resp.cumulative_usage.total_tokens}`,
      ]);
    } catch (err: unknown) {
      setEventLogs((prev) => [...prev, `[ERROR] ${String(err)}`]);
    } finally {
      setIsExecuting(false);
    }
  };

  const handleCancelBuilderTurn = async () => {
    try {
      await invoke('cancel_builder_turn');
      setEventLogs((prev) => [...prev, '[INFO] Sent cancel signal to active turn']);
    } catch (err: unknown) {
      setEventLogs((prev) => [...prev, `[ERROR Canceling] ${String(err)}`]);
    }
  };

  const handleClipboardWrite = async () => {
    try {
      await invoke('desktop_clipboard_write', { text: clipboardText });
      setDesktopMessage('Wrote text to clipboard successfully');
    } catch (err: unknown) {
      setDesktopMessage(`Clipboard Write Error: ${String(err)}`);
    }
  };

  const handleClipboardRead = async () => {
    try {
      const readText = await invoke<string>('desktop_clipboard_read');
      setReadClipboardResult(readText);
      setDesktopMessage('Read text from clipboard successfully');
    } catch (err: unknown) {
      setDesktopMessage(`Clipboard Read Error: ${String(err)}`);
    }
  };

  const handleOpenUrl = async () => {
    try {
      await invoke('desktop_open_url', { url: urlToOpen });
      setDesktopMessage(`Opened URL: ${urlToOpen}`);
    } catch (err: unknown) {
      setDesktopMessage(`Open URL Error: ${String(err)}`);
    }
  };

  return (
    <div className="diagnostics-container">
      <header>
        <h1>Coalition — Phase 0 Technical Diagnostics</h1>
        <p className="subtitle">
          Verification harness for desktop boot, IPC, Git adapter, SQLite migration proof, and Antigravity adapter.
        </p>
      </header>

      {/* Panel 1: System & Runtime Diagnostics */}
      <section className="panel" aria-label="System Diagnostics">
        <h2>1. System & Runtime Proofs</h2>
        <div className="grid">
          <div className="metric-box">
            <div className="metric-label">React Render</div>
            <div className="metric-value">
              <span className="badge success">OK (Active)</span>
            </div>
          </div>
          <div className="metric-box">
            <div className="metric-label">Tauri IPC Status</div>
            <div className="metric-value">
              <span className={`badge ${ipcStatus === 'OK' ? 'success' : 'error'}`}>
                {ipcStatus}
              </span>
            </div>
          </div>
          <div className="metric-box">
            <div className="metric-label">Git Detected</div>
            <div className="metric-value">
              {diagnostics?.git ? (
                <span className="badge success">{diagnostics.git.git_version}</span>
              ) : (
                <span className="badge error">Not Found</span>
              )}
            </div>
          </div>
          <div className="metric-box">
            <div className="metric-label">Git Branch</div>
            <div className="metric-value">{diagnostics?.git?.current_branch || 'None'}</div>
          </div>
        </div>

        {diagnostics?.git && (
          <div className="metric-box" style={{ marginTop: '8px' }}>
            <div className="metric-label">Git Working Tree Status</div>
            <div className="metric-value" style={{ fontWeight: 'normal', fontSize: '0.85rem' }}>
              Staged: {diagnostics.git.status.staged} | Unstaged: {diagnostics.git.status.unstaged} | Untracked: {diagnostics.git.status.untracked} | Head: {diagnostics.git.head_commit?.substring(0, 8) || 'None'}
            </div>
          </div>
        )}
      </section>

      {/* Panel 2: SQLite Connection & Migration Proof */}
      <section className="panel" aria-label="SQLite Proof">
        <h2>2. SQLite Migration & Operational State Proof</h2>
        <p style={{ margin: '0 0 10px 0', fontSize: '0.875rem', color: '#4b5563' }}>
          Tests SQLite connection, schema migration tracking (`_coalition_migrations`), and transactional inserts.
        </p>
        <button onClick={handleRunSqliteProof}>Run SQLite Migration Proof</button>

        {sqliteError && <p style={{ color: '#dc2626', fontSize: '0.875rem' }}>Error: {sqliteError}</p>}

        {sqliteProof && (
          <div className="grid" style={{ marginTop: '12px' }}>
            <div className="metric-box">
              <div className="metric-label">New Migrations Applied</div>
              <div className="metric-value">{sqliteProof.applied_migrations.length}</div>
            </div>
            <div className="metric-box">
              <div className="metric-label">Test Record ID</div>
              <div className="metric-value">{sqliteProof.test_record_id}</div>
            </div>
            <div className="metric-box">
              <div className="metric-label">Total Operational Records</div>
              <div className="metric-value">{sqliteProof.total_records}</div>
            </div>
          </div>
        )}
      </section>

      {/* Panel 3: Antigravity CLI Adapter & Stream Runner */}
      <section className="panel" aria-label="Antigravity Builder Proof">
        <h2>3. Antigravity Builder Adapter & NDJSON Stream Proof</h2>
        <div className="grid">
          <div className="metric-box">
            <div className="metric-label">Antigravity CLI Detected</div>
            <div className="metric-value">
              {diagnostics?.agy_detected ? (
                <span className="badge success">{diagnostics.agy_version || 'Detected'}</span>
              ) : (
                <span className="badge warning">Fallback (fake-agy available)</span>
              )}
            </div>
          </div>
          <div className="metric-box">
            <div className="metric-label">Discovered Models</div>
            <div className="metric-value">{diagnostics?.agy_models?.length ?? 0} available</div>
          </div>
        </div>

        <div style={{ margin: '14px 0 10px 0' }}>
          <div className="input-row">
            <input
              type="text"
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              placeholder="Prompt to send over NDJSON stream"
              disabled={isExecuting}
            />
            <select
              value={selectedModel}
              onChange={(e) => setSelectedModel(e.target.value)}
              disabled={isExecuting}
            >
              {diagnostics?.agy_models && diagnostics.agy_models.length > 0 ? (
                diagnostics.agy_models.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.name}
                  </option>
                ))
              ) : (
                <option value="gemini-3.8-flash-high">Gemini 3.8 Flash (High)</option>
              )}
            </select>
            <select
              value={effort}
              onChange={(e) => setEffort(e.target.value)}
              disabled={isExecuting}
            >
              <option value="low">Effort: Low</option>
              <option value="medium">Effort: Medium</option>
              <option value="high">Effort: High</option>
            </select>
          </div>

          <div style={{ display: 'flex', gap: '16px', alignItems: 'center', marginBottom: '12px' }}>
            <label className="checkbox-label">
              <input
                type="checkbox"
                checked={useFakeAgy}
                onChange={(e) => setUseFakeAgy(e.target.checked)}
                disabled={isExecuting}
              />
              Use fake-agy (Zero Quota Double)
            </label>

            <label className="checkbox-label">
              <input
                type="checkbox"
                checked={testIcarus}
                onChange={(e) => setTestIcarus(e.target.checked)}
                disabled={isExecuting}
              />
              Test Icarus Flag (--dangerously-skip-permissions)
            </label>
          </div>

          <div style={{ display: 'flex', gap: '10px' }}>
            <button onClick={handleExecuteBuilderTurn} disabled={isExecuting}>
              {isExecuting ? 'Running Stream...' : 'Execute Turn'}
            </button>
            <button
              onClick={handleCancelBuilderTurn}
              disabled={!isExecuting}
              className="danger"
            >
              Cancel Active Turn
            </button>
          </div>
        </div>

        {lastResponse && (
          <div className="grid" style={{ marginTop: '10px' }}>
            <div className="metric-box">
              <div className="metric-label">Status</div>
              <div className="metric-value">
                <span className={`badge ${lastResponse.status === 'SUCCESS' ? 'success' : 'error'}`}>
                  {lastResponse.status}
                </span>
              </div>
            </div>
            <div className="metric-box">
              <div className="metric-label">Conversation ID</div>
              <div className="metric-value" style={{ fontSize: '0.8rem' }}>
                {lastResponse.conversation_id || 'None'}
              </div>
            </div>
            <div className="metric-box">
              <div className="metric-label">Total Tokens Reported</div>
              <div className="metric-value">{lastResponse.cumulative_usage.total_tokens}</div>
            </div>
          </div>
        )}

        <div className="log-box" aria-label="Stream Logs">
          {eventLogs.length === 0 ? '// Live stream NDJSON events will appear here...' : eventLogs.join('\n')}
        </div>
      </section>

      {/* Panel 4: Desktop Integration Proofs */}
      <section className="panel" aria-label="Desktop Integration Proofs">
        <h2>4. Desktop Clipboard & Native Opener Proofs</h2>
        <div style={{ display: 'flex', flexDirection: 'column', gap: '10px' }}>
          <div className="input-row">
            <input
              type="text"
              value={clipboardText}
              onChange={(e) => setClipboardText(e.target.value)}
              placeholder="Text to write to clipboard"
            />
            <button onClick={handleClipboardWrite}>Write Clipboard</button>
            <button onClick={handleClipboardRead} className="secondary">Read Clipboard</button>
          </div>
          {readClipboardResult && (
            <div className="metric-box">
              <div className="metric-label">Read from Clipboard:</div>
              <div className="metric-value">{readClipboardResult}</div>
            </div>
          )}

          <div className="input-row" style={{ marginTop: '8px' }}>
            <input
              type="text"
              value={urlToOpen}
              onChange={(e) => setUrlToOpen(e.target.value)}
              placeholder="Safe HTTPS URL to open in default browser"
            />
            <button onClick={handleOpenUrl} className="secondary">Open Browser URL</button>
          </div>

          {desktopMessage && (
            <p style={{ margin: 0, fontSize: '0.85rem', color: '#2563eb' }}>{desktopMessage}</p>
          )}
        </div>
      </section>
    </div>
  );
};

export default App;

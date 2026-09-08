import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, act } from '@testing-library/react';
import App from './App';

type EventHandler = (event: { payload: unknown }) => void;
let registeredHandlers: EventHandler[] = [];

// Mock Tauri API
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (cmd: string) => {
    if (cmd === 'get_system_diagnostics') {
      return {
        current_dir: '/test/workspace',
        git: {
          git_version: 'git version 2.53.0',
          is_repo: true,
          root_dir: '/test/workspace',
          current_branch: 'main',
          head_commit: 'abc12345',
          status: { staged: 0, unstaged: 0, untracked: 0 },
          diff_summary: '',
        },
        agy_detected: true,
        agy_path: '/fake/agy',
        agy_version: '1.1.27-fake',
        agy_models: [
          { id: 'gemini-3.8-flash-high', name: 'Gemini 3.8 Flash (High)' },
        ],
        agy_error: null,
      };
    }
    if (cmd === 'run_sqlite_proof') {
      return {
        applied_migrations: [{ version: 1, name: '001_initial', applied_at: '2026-09-08' }],
        test_record_id: 1,
        test_record_message: 'Proof OK',
        total_records: 1,
      };
    }
    return {};
  }),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (_eventName: string, handler: EventHandler) => {
    registeredHandlers.push(handler);
    return () => {
      registeredHandlers = registeredHandlers.filter((h) => h !== handler);
    };
  }),
}));

describe('Coalition Phase 0 Diagnostics App', () => {
  beforeEach(() => {
    registeredHandlers = [];
  });

  it('renders Phase 0 Diagnostics title and panels', async () => {
    await act(async () => {
      render(<App />);
    });

    expect(screen.getByText('Coalition — Phase 0 Technical Diagnostics')).toBeInTheDocument();
    expect(screen.getByText('1. System & Runtime Proofs')).toBeInTheDocument();
    expect(screen.getByText('2. SQLite Migration & Operational State Proof')).toBeInTheDocument();
    expect(screen.getByText('3. Antigravity Builder Adapter & NDJSON Stream Proof')).toBeInTheDocument();
    expect(screen.getByText('4. Desktop Clipboard & Native Opener Proofs')).toBeInTheDocument();

    // Verify diagnostic buttons exist
    expect(screen.getByRole('button', { name: 'Run SQLite Migration Proof' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Execute Turn' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Cancel Active Turn' })).toBeInTheDocument();
  });

  it('maintains exactly one stream event listener under React StrictMode and logs events without duplication', async () => {
    await act(async () => {
      render(
        <React.StrictMode>
          <App />
        </React.StrictMode>
      );
    });

    // Verify only 1 active event handler remains registered despite StrictMode remount
    expect(registeredHandlers.length).toBe(1);

    // Simulate an NDJSON init event arriving over the stream
    const testInitPayload = {
      event: 'init',
      conversation_id: 'unique-session-id-98765',
      init: { model: 'gemini-3.8-flash-high' },
    };

    await act(async () => {
      registeredHandlers[0]({ payload: testInitPayload });
    });

    const streamLogs = screen.getByLabelText('Stream Logs');
    const matches = streamLogs.textContent?.match(/unique-session-id-98765/g);

    // Must appear exactly once in the event log, NOT duplicated
    expect(matches).not.toBeNull();
    expect(matches?.length).toBe(1);
  });
});

import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import App from './App';

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
  listen: vi.fn(async () => {
    return () => {};
  }),
}));

describe('Coalition Phase 0 Diagnostics App', () => {
  it('renders Phase 0 Diagnostics title and panels', async () => {
    render(<App />);

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
});

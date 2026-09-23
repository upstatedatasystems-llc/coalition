import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import React from 'react';
import { ValidationView } from './ValidationView';
import { ValidationConfig, ValidationRunRecord } from '../../types';

// Mock Tauri invoke & listen APIs
const mockInvoke = vi.fn();
const mockListen = vi.fn().mockResolvedValue(() => {});

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: (...args: unknown[]) => mockListen(...args),
}));

const mockConfig: ValidationConfig = {
  schema_version: 1,
  enabled: true,
  commands: [
    {
      id: 'cmd-test',
      name: 'Cargo Test',
      command: 'cargo test',
      cwd: null,
      timeout_seconds: 300,
      required: true,
    },
    {
      id: 'cmd-lint',
      name: 'Clippy',
      command: 'cargo clippy',
      cwd: null,
      timeout_seconds: 120,
      required: false,
    },
  ],
  policy: {
    gate_review_on_required_failure: true,
    grace_period_seconds: 10,
  },
};

const mockHistoryRun: ValidationRunRecord = {
  run_id: 'val-run-123',
  project_id: 'proj-1',
  architecture_version: '1.0',
  epoch_id: 'epoch-1',
  trigger_source: 'MANUAL',
  status: 'FAIL',
  is_gate_passed: false,
  has_override: false,
  git_head: 'abc1234567890',
  git_dirty_fingerprint: 'dirty-fingerprint-xyz',
  config_fingerprint: 'config-sha256',
  log_path: '/path/to/log.txt',
  started_at: new Date().toISOString(),
  completed_at: new Date().toISOString(),
  duration_ms: 1234,
  commands: [
    {
      id: 1,
      run_id: 'val-run-123',
      command_id: 'cmd-test',
      name: 'Cargo Test',
      command: 'cargo test',
      cwd: '.',
      required: true,
      status: 'FAIL',
      exit_code: 1,
      duration_ms: 1200,
      stdout_preview: 'failures: test_defect',
      stderr_preview: null,
      log_path: '/path/to/cmd.log',
      started_at: new Date().toISOString(),
      completed_at: new Date().toISOString(),
    },
  ],
};

describe('ValidationView', () => {
  const onRefreshProject = vi.fn().mockResolvedValue(undefined);

  beforeEach(() => {
    vi.clearAllMocks();
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_validation_config') {
        return Promise.resolve(mockConfig);
      }
      if (cmd === 'get_active_validation_run') {
        return Promise.resolve(null);
      }
      if (cmd === 'get_validation_history') {
        return Promise.resolve([mockHistoryRun]);
      }
      return Promise.resolve(null);
    });
  });

  it('renders validation configuration and history correctly', async () => {
    render(
      <ValidationView
        projectId="proj-1"
        projectName="Test Project"
        workflowState="BUILDING"
        onRefreshProject={onRefreshProject}
      />
    );

    await waitFor(() => {
      expect(screen.getByTestId('validation-view')).toBeInTheDocument();
    });

    expect(screen.getByText('Required Gate (Blocks Review on Failure)')).toBeInTheDocument();
    expect(screen.getByText(/2 commands \(1 required\)/)).toBeInTheDocument();
    expect(screen.getByTestId('run-item-val-run-123')).toBeInTheDocument();
    expect(screen.getByTestId('override-gate-btn')).toBeInTheDocument();
  });

  it('handles unconfigured validation gracefully', async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_validation_config') {
        return Promise.resolve({
          schema_version: 1,
          enabled: false,
          commands: [],
          policy: { gate_review_on_required_failure: false, grace_period_seconds: 10 },
        });
      }
      if (cmd === 'get_active_validation_run') return Promise.resolve(null);
      if (cmd === 'get_validation_history') return Promise.resolve([]);
      return Promise.resolve(null);
    });

    render(
      <ValidationView
        projectId="proj-1"
        projectName="Test Project"
        workflowState="BUILDING"
        onRefreshProject={onRefreshProject}
      />
    );

    await waitFor(() => {
      expect(screen.getByTestId('validation-unconfigured-note')).toBeInTheDocument();
    });

    expect(screen.getByTestId('start-validation-btn')).toBeDisabled();
    expect(screen.getByTestId('no-runs-message')).toBeInTheDocument();
  });

  it('triggers a validation run on click', async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_validation_config') return Promise.resolve(mockConfig);
      if (cmd === 'get_active_validation_run') return Promise.resolve(null);
      if (cmd === 'get_validation_history') return Promise.resolve([]);
      if (cmd === 'start_validation_run') {
        return Promise.resolve({
          ...mockHistoryRun,
          run_id: 'val-run-new',
          status: 'RUNNING',
          commands: [],
        });
      }
      return Promise.resolve(null);
    });

    render(
      <ValidationView
        projectId="proj-1"
        projectName="Test Project"
        workflowState="BUILDING"
        onRefreshProject={onRefreshProject}
      />
    );

    await waitFor(() => {
      expect(screen.getByTestId('start-validation-btn')).not.toBeDisabled();
    });

    fireEvent.click(screen.getByTestId('start-validation-btn'));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith('start_validation_run', {
        payload: { projectId: 'proj-1', trigger: 'MANUAL' },
      });
    });
  });

  it('opens gate override modal and authorizes with rationale', async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_validation_config') return Promise.resolve(mockConfig);
      if (cmd === 'get_active_validation_run') return Promise.resolve(null);
      if (cmd === 'get_validation_history') return Promise.resolve([mockHistoryRun]);
      if (cmd === 'override_validation_gate') {
        return Promise.resolve({
          override_id: 'ovr-123',
          project_id: 'proj-1',
          run_id: 'val-run-123',
          git_fingerprint: 'dirty-fingerprint-xyz',
          reason: 'Test defect is acceptable in staging',
          authorized_by: 'HUMAN',
          created_at: new Date().toISOString(),
        });
      }
      return Promise.resolve(null);
    });

    render(
      <ValidationView
        projectId="proj-1"
        projectName="Test Project"
        workflowState="BUILDING"
        onRefreshProject={onRefreshProject}
      />
    );

    await waitFor(() => {
      expect(screen.getByTestId('override-gate-btn')).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId('override-gate-btn'));
    expect(screen.getByTestId('override-modal')).toBeInTheDocument();

    const rationaleInput = screen.getByTestId('override-rationale-input');
    const confirmBtn = screen.getByTestId('confirm-override-btn');

    expect(confirmBtn).toBeDisabled();

    fireEvent.change(rationaleInput, {
      target: { value: 'Test defect is acceptable in staging' },
    });
    expect(confirmBtn).not.toBeDisabled();

    fireEvent.click(confirmBtn);

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith('override_validation_gate', {
        payload: {
          projectId: 'proj-1',
          runId: 'val-run-123',
          reason: 'Test defect is acceptable in staging',
        },
      });
    });
  });

  it('displays error banner when submit for review is blocked by validation gate', async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_validation_config') return Promise.resolve(mockConfig);
      if (cmd === 'get_active_validation_run') return Promise.resolve(null);
      if (cmd === 'get_validation_history') return Promise.resolve([mockHistoryRun]);
      if (cmd === 'submit_for_review') {
        const error = new Error('Latest validation run val-run-123 failed required checks and has no human override');
        (error as any).code = 'VALIDATION_GATE_BLOCKED';
        return Promise.reject(error);
      }
      return Promise.resolve(null);
    });

    render(
      <ValidationView
        projectId="proj-1"
        projectName="Test Project"
        workflowState="BUILDING"
        onRefreshProject={onRefreshProject}
      />
    );

    await waitFor(() => {
      expect(screen.getByTestId('submit-review-btn')).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId('submit-review-btn'));

    await waitFor(() => {
      expect(screen.getByTestId('validation-error-banner')).toBeInTheDocument();
      expect(screen.getByText(/Submission Blocked by Validation Gate/)).toBeInTheDocument();
    });
  });
});

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { ReviewControlView } from './ReviewControlView';
import { ReviewCycleRecord, ReviewImportPreview } from '../../types';

// Mock Tauri invoke
const mockInvoke = vi.fn();

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

const mockPendingCycle: ReviewCycleRecord = {
  cycle_id: 'cycle-1',
  project_id: 'proj-1',
  cycle_number: 1,
  architecture_version: '1.0',
  epoch_id: 'epoch-1',
  validation_run_id: 'val-1',
  status: 'PENDING',
  verdict: null,
  reviewer_type: 'CHATGPT_RELAY',
  git_head: 'abcd1234ef5678',
  git_dirty_fingerprint: 'CLEAN',
  review_packet_hash: 'hash-1234567890abcdef',
  corrections_packet: null,
  summary: null,
  started_at: '2026-09-23T12:00:00Z',
  completed_at: null,
  created_at: '2026-09-23T12:00:00Z',
  findings: [],
};

const mockCorrectionsCycle: ReviewCycleRecord = {
  cycle_id: 'cycle-2',
  project_id: 'proj-1',
  cycle_number: 2,
  architecture_version: '1.0',
  epoch_id: 'epoch-1',
  validation_run_id: 'val-2',
  status: 'CORRECTIONS_REQUIRED',
  verdict: 'CORRECTIONS_REQUIRED',
  reviewer_type: 'CHATGPT_RELAY',
  git_head: 'abcd1234ef5678',
  git_dirty_fingerprint: 'CLEAN',
  review_packet_hash: 'hash-corrections-123',
  corrections_packet: '## Corrections Required\n- Fix buffer bounds check in main.rs',
  summary: 'Buffer bounds check is missing in main.rs.',
  started_at: '2026-09-23T13:00:00Z',
  completed_at: '2026-09-23T13:05:00Z',
  created_at: '2026-09-23T13:00:00Z',
  findings: [
    {
      finding_id: 'fnd-1',
      project_id: 'proj-1',
      first_cycle_id: 'cycle-1',
      last_cycle_id: 'cycle-2',
      fingerprint: 'fp-1',
      severity: 'CRITICAL',
      status: 'OPEN',
      file_path: 'src/main.rs',
      line_range: '42-45',
      title: 'Buffer overflow vulnerability',
      description: 'Input length is not validated before copying.',
      suggested_fix: 'Add bounds check',
      resolution_cycle_id: null,
      is_repeat: true,
      created_at: '2026-09-23T12:00:00Z',
      updated_at: '2026-09-23T13:05:00Z',
    },
  ],
};

const mockPreview: ReviewImportPreview = {
  preview_id: 'prev-1',
  project_id: 'proj-1',
  cycle_id: 'cycle-1',
  verdict: 'CORRECTIONS_REQUIRED',
  summary: 'One critical bug found.',
  findings: [
    {
      id: 'FND-1',
      severity: 'CRITICAL',
      title: 'Buffer overflow vulnerability',
      file: 'src/main.rs',
      lines: '42-45',
      description: 'Input length is not validated before copying.',
      suggested_fix: 'Add bounds check',
    },
  ],
  raw_response_hash: 'resp-hash-1',
  git_head: 'abcd1234ef5678',
  git_dirty_fingerprint: 'CLEAN',
  corrections_preview: '## Corrections Required\n- Fix buffer bounds check',
  created_at: '2026-09-23T12:05:00Z',
};

describe('ReviewControlView', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    Object.assign(navigator, {
      clipboard: {
        writeText: vi.fn().mockResolvedValue(undefined),
      },
    });
  });

  it('renders empty cycle card when no review cycles exist', async () => {
    mockInvoke.mockImplementation((cmd) => {
      if (cmd === 'get_latest_review_cycle') return Promise.resolve(null);
      if (cmd === 'list_review_cycles') return Promise.resolve([]);
      return Promise.resolve(null);
    });

    render(
      <ReviewControlView
        projectId="proj-1"
        projectName="Test Project"
        workflowState="BUILDING"
        onRefreshProject={vi.fn()}
      />
    );

    await waitFor(() => {
      expect(screen.getByTestId('empty-cycle-card')).toBeInTheDocument();
    });

    expect(screen.getByText(/No Review Cycles Yet/i)).toBeInTheDocument();
    expect(screen.getByTestId('workflow-state-badge')).toHaveTextContent('Workflow: BUILDING');
  });

  it('enables Prepare Review Packet button when workflowState is WAITING_FOR_REVIEW', async () => {
    mockInvoke.mockImplementation((cmd) => {
      if (cmd === 'get_latest_review_cycle') return Promise.resolve(null);
      if (cmd === 'list_review_cycles') return Promise.resolve([]);
      if (cmd === 'prepare_review_packet') return Promise.resolve(mockPendingCycle);
      return Promise.resolve(null);
    });

    const refreshMock = vi.fn().mockResolvedValue(undefined);

    render(
      <ReviewControlView
        projectId="proj-1"
        projectName="Test Project"
        workflowState="WAITING_FOR_REVIEW"
        onRefreshProject={refreshMock}
      />
    );

    await waitFor(() => {
      expect(screen.getByTestId('prepare-review-packet-btn')).not.toBeDisabled();
    });

    fireEvent.click(screen.getByTestId('prepare-review-packet-btn'));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith('prepare_review_packet', { projectId: 'proj-1' });
      expect(refreshMock).toHaveBeenCalled();
    });
  });

  it('disables Prepare Review Packet button when workflowState is not WAITING_FOR_REVIEW', async () => {
    mockInvoke.mockImplementation((cmd) => {
      if (cmd === 'get_latest_review_cycle') return Promise.resolve(null);
      if (cmd === 'list_review_cycles') return Promise.resolve([]);
      return Promise.resolve(null);
    });

    render(
      <ReviewControlView
        projectId="proj-1"
        projectName="Test Project"
        workflowState="VALIDATING"
        onRefreshProject={vi.fn()}
      />
    );

    await waitFor(() => {
      expect(screen.getByTestId('prepare-review-packet-btn')).toBeDisabled();
    });
  });

  it('renders pending review cycle, copies packet, and opens ChatGPT', async () => {
    mockInvoke.mockImplementation((cmd) => {
      if (cmd === 'get_latest_review_cycle') return Promise.resolve(mockPendingCycle);
      if (cmd === 'list_review_cycles') return Promise.resolve([mockPendingCycle]);
      if (cmd === 'open_chatgpt') return Promise.resolve();
      return Promise.resolve(null);
    });

    render(
      <ReviewControlView
        projectId="proj-1"
        projectName="Test Project"
        workflowState="WAITING_FOR_REVIEW"
        onRefreshProject={vi.fn()}
      />
    );

    await waitFor(() => {
      expect(screen.getByTestId('active-cycle-card')).toBeInTheDocument();
      expect(screen.getByText(/Review Cycle #1/i)).toBeInTheDocument();
    });

    // Copy packet
    fireEvent.click(screen.getByTestId('copy-packet-btn'));
    await waitFor(() => {
      expect(navigator.clipboard.writeText).toHaveBeenCalled();
    });

    // Open ChatGPT
    fireEvent.click(screen.getByTestId('open-chatgpt-btn'));
    expect(mockInvoke).toHaveBeenCalledWith('open_chatgpt');
  });

  it('previews and confirms reviewer response import', async () => {
    mockInvoke.mockImplementation((cmd) => {
      if (cmd === 'get_latest_review_cycle') return Promise.resolve(mockPendingCycle);
      if (cmd === 'list_review_cycles') return Promise.resolve([mockPendingCycle]);
      if (cmd === 'prepare_review_import') return Promise.resolve(mockPreview);
      if (cmd === 'confirm_review_import') return Promise.resolve(mockCorrectionsCycle);
      return Promise.resolve(null);
    });

    const refreshMock = vi.fn().mockResolvedValue(undefined);

    render(
      <ReviewControlView
        projectId="proj-1"
        projectName="Test Project"
        workflowState="WAITING_FOR_REVIEW"
        onRefreshProject={refreshMock}
      />
    );

    await waitFor(() => {
      expect(screen.getByTestId('import-response-card')).toBeInTheDocument();
    });

    const input = screen.getByTestId('reviewer-response-input');
    fireEvent.change(input, {
      target: { value: '```verdict\nverdict: CORRECTIONS_REQUIRED\n```' },
    });

    fireEvent.click(screen.getByTestId('preview-import-btn'));

    await waitFor(() => {
      expect(screen.getByTestId('import-preview-modal')).toBeInTheDocument();
      expect(screen.getByTestId('preview-verdict')).toHaveTextContent('CORRECTIONS_REQUIRED');
      expect(screen.getByTestId('preview-summary')).toHaveTextContent('One critical bug found.');
    });

    fireEvent.click(screen.getByTestId('confirm-import-btn'));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith('confirm_review_import', {
        projectId: 'proj-1',
        previewId: 'prev-1',
      });
      expect(refreshMock).toHaveBeenCalled();
    });
  });

  it('renders repeat findings badge and corrections packet with start corrections button', async () => {
    mockInvoke.mockImplementation((cmd) => {
      if (cmd === 'get_latest_review_cycle') return Promise.resolve(mockCorrectionsCycle);
      if (cmd === 'list_review_cycles') return Promise.resolve([mockPendingCycle, mockCorrectionsCycle]);
      if (cmd === 'start_builder_turn') return Promise.resolve({ status: 'RUNNING' });
      return Promise.resolve(null);
    });

    const navigateMock = vi.fn();

    render(
      <ReviewControlView
        projectId="proj-1"
        projectName="Test Project"
        workflowState="CORRECTIONS_REQUIRED"
        onRefreshProject={vi.fn()}
        onNavigateToBuilder={navigateMock}
      />
    );

    await waitFor(() => {
      expect(screen.getByTestId('repeat-finding-badge')).toBeInTheDocument();
      expect(screen.getByText('REPEAT FINDING')).toBeInTheDocument();
      expect(screen.getByTestId('corrections-content')).toBeInTheDocument();
      expect(screen.getByTestId('start-builder-corrections-btn')).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId('start-builder-corrections-btn'));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith('start_builder_turn', {
        payload: {
          projectId: 'proj-1',
          instructionSource: {
            type: 'REVIEW_CORRECTION',
            review_cycle_id: 'cycle-2',
          },
        },
      });
      expect(navigateMock).toHaveBeenCalled();
    });
  });
});

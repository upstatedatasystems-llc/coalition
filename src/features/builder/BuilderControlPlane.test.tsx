import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, act, fireEvent } from '@testing-library/react';
import { BuilderControlPlaneView } from './BuilderControlPlaneView';
import {
  ModelInfo,
  BuilderPacket,
  DriftReport,
  IcarusState,
  UsageTelemetryReport,
  PermissionRecord,
  BuilderTurnResponse,
} from '../../types';

// Mock Tauri invoke & event
const mockInvoke = vi.fn();
const mockListen = vi.fn();

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: (...args: unknown[]) => {
    mockListen(...args);
    return Promise.resolve(() => {});
  },
}));

const mockModels: ModelInfo[] = [
  { id: 'gemini-3.8-flash-high', name: 'Gemini 3.8 Flash (High)' },
  { id: 'gemini-3.7-flash-medium', name: 'Gemini 3.7 Flash (Medium)' },
  { id: 'claude-sonnet-4-6', name: 'Claude Sonnet 4.6 (Thinking)' },
];

const mockBuilderPacket: BuilderPacket = {
  metadata: {
    schema_version: 1,
    packet_id: 'builder-packet-uuid-1',
    project_id: 'proj-stage3',
    project_name: 'Stage3 Test Project',
    architecture_version: '1.0.0',
    builder_epoch_id: 'epoch-stage3-12345',
    created_at: '2026-09-22T12:00:00Z',
    contract_fingerprint: 'abcdef0123456789abcdef0123456789',
    git_head_commit: '1234567890abcdef1234567890abcdef12345678',
    git_branch: 'main',
  },
  summary: 'Stage 3 frozen packet summary',
  builder_rules: 'Strict rules for builder implementation',
  artifacts: [
    {
      path: 'design/product-vision.md',
      title: 'Product Vision',
      content: '# Vision\nBuilding test project',
      fingerprint: 'fp-1',
    },
    {
      path: 'design/architecture.md',
      title: 'Architecture',
      content: '# Architecture\nDetailed specs',
      fingerprint: 'fp-2',
    },
  ],
  is_truncated: false,
  prompt: '# Authoritative Builder Contract Instructions\nExecute implementation per frozen architecture.',
};

const mockCleanDriftReport: DriftReport = {
  has_drift: false,
  is_frozen: true,
  architecture_version: '1.0.0',
  drifted_artifacts: [],
  checked_at: '2026-09-22T12:00:00Z',
};

const mockIcarusInactive: IcarusState = {
  project_id: 'proj-stage3',
  enabled: false,
  enabled_at: null,
  enabled_by: null,
};

const mockIcarusActive: IcarusState = {
  project_id: 'proj-stage3',
  enabled: true,
  enabled_at: '2026-09-22T13:00:00Z',
  enabled_by: 'HUMAN',
};

const mockTelemetry: UsageTelemetryReport = {
  provider_antigravity_usage: {
    input_tokens: 1500,
    output_tokens: 300,
    thinking_tokens: 200,
    cache_read_tokens: 4000,
    total_tokens: 2000,
  },
  active_model: 'gemini-3.8-flash-high',
  active_effort: 'medium',
  chatgpt_estimated_usage: {
    rolling_5h_tokens: 4500,
    rolling_7d_tokens: 12000,
    total_tokens: 25000,
    total_packets_sent: 4,
    total_imports_received: 3,
    last_calibrated_at: '2026-09-22T10:00:00Z',
    disclaimer: 'Estimated (~4 chars/token heuristic). Does not reflect official OpenAI billing.',
    estimator_version: 1,
    chars_per_token: 4.0,
    sample_count: 0,
    estimated_5h_capacity_pct: 15.0,
    estimated_weekly_capacity_pct: 10.0,
  },
};

const mockPermissions: PermissionRecord[] = [
  {
    id: 1,
    project_id: 'proj-stage3',
    session_id: 'sess-1',
    tool_name: 'view_file',
    target: 'src/main.rs',
    risk_level: 'READ_ONLY',
    decision: 'ALLOWED',
    reason: null,
    created_at: '2026-09-22T13:30:00Z',
  },
  {
    id: 2,
    project_id: 'proj-stage3',
    session_id: 'sess-1',
    tool_name: 'run_command',
    target: 'cargo test',
    risk_level: 'HIGH_RISK',
    decision: 'BLOCKED',
    reason: 'Action requires review or permission was denied in headless execution',
    created_at: '2026-09-22T13:31:00Z',
  },
];

describe('BuilderControlPlaneView', () => {
  const onRefreshProject = vi.fn().mockResolvedValue(undefined);

  beforeEach(() => {
    vi.clearAllMocks();
    mockInvoke.mockImplementation((cmd: string) => {
      switch (cmd) {
        case 'list_builder_models':
          return Promise.resolve(mockModels);
        case 'get_builder_packet':
          return Promise.resolve(mockBuilderPacket);
        case 'get_contract_drift':
          return Promise.resolve(mockCleanDriftReport);
        case 'get_icarus_state':
          return Promise.resolve(mockIcarusInactive);
        case 'get_usage_telemetry':
          return Promise.resolve(mockTelemetry);
        case 'get_permission_history':
          return Promise.resolve(mockPermissions);
        case 'list_builder_sessions':
          return Promise.resolve([]);
        default:
          return Promise.resolve(null);
      }
    });
  });

  it('renders status bar, epoch, model selector and controls for frozen project', async () => {
    await act(async () => {
      render(
        <BuilderControlPlaneView
          projectId="proj-stage3"
          projectName="Stage3 Test Project"
          workflowState="FROZEN"
          onRefreshProject={onRefreshProject}
        />
      );
    });

    expect(screen.getByTestId('builder-control-plane')).toBeInTheDocument();
    expect(screen.getByText('epoch-stage3-12345')).toBeInTheDocument();
    expect(screen.getByText('1.0.0')).toBeInTheDocument();
    expect(screen.getByTestId('model-select')).toBeInTheDocument();
    expect(screen.getByTestId('effort-select')).toBeInTheDocument();
    expect(screen.getByTestId('run-turn-btn')).toBeEnabled();
  });

  it('disables run turn and warns if architecture is not frozen', async () => {
    await act(async () => {
      render(
        <BuilderControlPlaneView
          projectId="proj-stage3"
          projectName="Stage3 Test Project"
          workflowState="ARCHITECTING"
          onRefreshProject={onRefreshProject}
        />
      );
    });

    expect(screen.getByText(/Architecture Contract Not Frozen/i)).toBeInTheDocument();
    expect(screen.getByTestId('run-turn-btn')).toBeDisabled();
  });

  it('shows glowing Icarus warning banner when Icarus is enabled', async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_icarus_state') return Promise.resolve(mockIcarusActive);
      if (cmd === 'list_builder_models') return Promise.resolve(mockModels);
      if (cmd === 'get_builder_packet') return Promise.resolve(mockBuilderPacket);
      if (cmd === 'get_contract_drift') return Promise.resolve(mockCleanDriftReport);
      if (cmd === 'get_usage_telemetry') return Promise.resolve(mockTelemetry);
      if (cmd === 'get_permission_history') return Promise.resolve(mockPermissions);
      if (cmd === 'list_builder_sessions') return Promise.resolve([]);
      return Promise.resolve(null);
    });

    await act(async () => {
      render(
        <BuilderControlPlaneView
          projectId="proj-stage3"
          projectName="Stage3 Test Project"
          workflowState="BUILDING"
          onRefreshProject={onRefreshProject}
        />
      );
    });

    expect(screen.getByTestId('icarus-banner')).toBeInTheDocument();
    expect(screen.getByText(/ICARUS MODE ACTIVE/i)).toBeInTheDocument();
    expect(screen.getByTestId('disable-icarus-btn')).toBeInTheDocument();
  });

  it('toggles Icarus mode via confirmation modal', async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'get_icarus_state') return Promise.resolve(mockIcarusInactive);
      if (cmd === 'set_icarus_mode') return Promise.resolve(mockIcarusActive);
      if (cmd === 'list_builder_models') return Promise.resolve(mockModels);
      if (cmd === 'get_builder_packet') return Promise.resolve(mockBuilderPacket);
      if (cmd === 'get_contract_drift') return Promise.resolve(mockCleanDriftReport);
      if (cmd === 'get_usage_telemetry') return Promise.resolve(mockTelemetry);
      if (cmd === 'get_permission_history') return Promise.resolve(mockPermissions);
      if (cmd === 'list_builder_sessions') return Promise.resolve([]);
      return Promise.resolve(null);
    });

    await act(async () => {
      render(
        <BuilderControlPlaneView
          projectId="proj-stage3"
          projectName="Stage3 Test Project"
          workflowState="FROZEN"
          onRefreshProject={onRefreshProject}
        />
      );
    });

    // Open modal
    await act(async () => {
      fireEvent.click(screen.getByTestId('toggle-icarus-modal-btn'));
    });

    expect(screen.getByTestId('icarus-modal')).toBeInTheDocument();

    // Confirm enable
    await act(async () => {
      fireEvent.click(screen.getByTestId('confirm-enable-icarus'));
    });

    expect(mockInvoke).toHaveBeenCalledWith('set_icarus_mode', {
      projectId: 'proj-stage3',
      enabled: true,
    });
  });

  it('displays provider-reported Antigravity tokens and estimated ChatGPT tokens with disclaimer', async () => {
    await act(async () => {
      render(
        <BuilderControlPlaneView
          projectId="proj-stage3"
          projectName="Stage3 Test Project"
          workflowState="FROZEN"
          onRefreshProject={onRefreshProject}
        />
      );
    });

    expect(screen.getByText('Provider-Reported')).toBeInTheDocument();
    expect(screen.getByText('2,000')).toBeInTheDocument(); // total tokens
    expect(screen.getByText('4,500')).toBeInTheDocument(); // 5h rolling
    expect(screen.getByText(/Estimated \(~4 chars\/token heuristic\)/i)).toBeInTheDocument();
  });

  it('resets ChatGPT usage window when button is clicked', async () => {
    await act(async () => {
      render(
        <BuilderControlPlaneView
          projectId="proj-stage3"
          projectName="Stage3 Test Project"
          workflowState="FROZEN"
          onRefreshProject={onRefreshProject}
        />
      );
    });

    await act(async () => {
      fireEvent.click(screen.getByTestId('reset-chatgpt-btn'));
    });

    expect(mockInvoke).toHaveBeenCalledWith('reset_chatgpt_usage', {
      projectId: 'proj-stage3',
    });
  });

  it('executes a builder turn when Run Turn is clicked', async () => {
    const mockTurnResp: BuilderTurnResponse = {
      conversation_id: 'fake-conv-uuid-1',
      status: 'SUCCESS',
      text_response: 'Implementation complete',
      cumulative_usage: {
        input_tokens: 500,
        output_tokens: 50,
        thinking_tokens: 40,
        cache_read_tokens: 1000,
        total_tokens: 550,
      },
      was_canceled: false,
      stderr: '',
    };

    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'start_builder_turn') return Promise.resolve(mockTurnResp);
      if (cmd === 'list_builder_models') return Promise.resolve(mockModels);
      if (cmd === 'get_builder_packet') return Promise.resolve(mockBuilderPacket);
      if (cmd === 'get_contract_drift') return Promise.resolve(mockCleanDriftReport);
      if (cmd === 'get_icarus_state') return Promise.resolve(mockIcarusInactive);
      if (cmd === 'get_usage_telemetry') return Promise.resolve(mockTelemetry);
      if (cmd === 'get_permission_history') return Promise.resolve(mockPermissions);
      if (cmd === 'list_builder_sessions') return Promise.resolve([]);
      return Promise.resolve(null);
    });

    await act(async () => {
      render(
        <BuilderControlPlaneView
          projectId="proj-stage3"
          projectName="Stage3 Test Project"
          workflowState="FROZEN"
          onRefreshProject={onRefreshProject}
        />
      );
    });

    await act(async () => {
      fireEvent.click(screen.getByTestId('run-turn-btn'));
    });

    expect(mockInvoke).toHaveBeenCalledWith('start_builder_turn', {
      payload: {
        projectId: 'proj-stage3',
        model: 'gemini-3.8-flash-high',
        effort: 'high',
      },
    });

    expect(onRefreshProject).toHaveBeenCalled();
    // After successful turn, verify completion report is displayed
    expect(screen.getByTestId('completion-report')).toBeInTheDocument();
    expect(screen.getByText('Implementation complete')).toBeInTheDocument();
  });

  it('surfaces model discovery error cleanly without falling back to hardcoded models', async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'list_builder_models') return Promise.reject(new Error('agy binary not found'));
      if (cmd === 'get_builder_packet') return Promise.resolve(mockBuilderPacket);
      if (cmd === 'get_contract_drift') return Promise.resolve(mockCleanDriftReport);
      if (cmd === 'get_icarus_state') return Promise.resolve(mockIcarusInactive);
      if (cmd === 'get_usage_telemetry') return Promise.resolve(mockTelemetry);
      if (cmd === 'get_permission_history') return Promise.resolve(mockPermissions);
      if (cmd === 'list_builder_sessions') return Promise.resolve([]);
      return Promise.resolve(null);
    });

    await act(async () => {
      render(
        <BuilderControlPlaneView
          projectId="proj-stage3"
          projectName="Stage3 Test Project"
          workflowState="FROZEN"
          onRefreshProject={onRefreshProject}
        />
      );
    });

    expect(screen.getByTestId('model-error-banner')).toBeInTheDocument();
    expect(screen.getByText(/Live model discovery failed/i)).toBeInTheDocument();
    // Turn button should be disabled when models are not available
    expect(screen.getByTestId('run-turn-btn')).toBeDisabled();
  });

  it('calibrates ChatGPT usage estimator via modal', async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'calibrate_chatgpt_usage') return Promise.resolve(mockTelemetry.chatgpt_estimated_usage);
      if (cmd === 'list_builder_models') return Promise.resolve(mockModels);
      if (cmd === 'get_builder_packet') return Promise.resolve(mockBuilderPacket);
      if (cmd === 'get_contract_drift') return Promise.resolve(mockCleanDriftReport);
      if (cmd === 'get_icarus_state') return Promise.resolve(mockIcarusInactive);
      if (cmd === 'get_usage_telemetry') return Promise.resolve(mockTelemetry);
      if (cmd === 'get_permission_history') return Promise.resolve(mockPermissions);
      if (cmd === 'list_builder_sessions') return Promise.resolve([]);
      return Promise.resolve(null);
    });

    await act(async () => {
      render(
        <BuilderControlPlaneView
          projectId="proj-stage3"
          projectName="Stage3 Test Project"
          workflowState="FROZEN"
          onRefreshProject={onRefreshProject}
        />
      );
    });

    // Open calibration modal
    await act(async () => {
      fireEvent.click(screen.getByTestId('calibrate-chatgpt-btn'));
    });

    expect(screen.getByTestId('calibration-modal')).toBeInTheDocument();

    // Fill in characters and reported tokens
    const charsInput = screen.getByLabelText(/Sample Characters/i);
    const tokensInput = screen.getByLabelText(/Observed \/ Reported Tokens/i);

    await act(async () => {
      fireEvent.change(charsInput, { target: { value: '1000' } });
      fireEvent.change(tokensInput, { target: { value: '250' } });
    });

    // Submit calibration
    await act(async () => {
      fireEvent.click(screen.getByTestId('submit-calibration-btn'));
    });

    expect(mockInvoke).toHaveBeenCalledWith('calibrate_chatgpt_usage', {
      projectId: 'proj-stage3',
      sampleTokens: 250,
      sampleChars: 1000,
    });
  });

  it('inspects frozen builder packet via modal', async () => {
    await act(async () => {
      render(
        <BuilderControlPlaneView
          projectId="proj-stage3"
          projectName="Stage3 Test Project"
          workflowState="FROZEN"
          onRefreshProject={onRefreshProject}
        />
      );
    });

    await act(async () => {
      fireEvent.click(screen.getByTestId('inspect-packet-btn'));
    });

    expect(screen.getByTestId('packet-modal')).toBeInTheDocument();
    expect(screen.getByText(/Immutable Builder Prompt/i)).toBeInTheDocument();
    expect(screen.getByText(/Authoritative Builder Contract Instructions/i)).toBeInTheDocument();
  });

  it('shows permission history and blocked action guidance', async () => {
    await act(async () => {
      render(
        <BuilderControlPlaneView
          projectId="proj-stage3"
          projectName="Stage3 Test Project"
          workflowState="FROZEN"
          onRefreshProject={onRefreshProject}
        />
      );
    });

    expect(screen.getByTestId('permission-history')).toBeInTheDocument();
    expect(screen.getByText('BLOCKED')).toBeInTheDocument();
    expect(screen.getByText(/Why was a tool blocked\?/i)).toBeInTheDocument();
  });

  it('renders active-run Icarus banner exclusively based on session icarus_mode, decoupled from project preference', async () => {
    // Case 1: Active running session has icarus_mode = true, but project icarus preference is disabled (false)
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'list_builder_models') return Promise.resolve(mockModels);
      if (cmd === 'get_builder_packet') return Promise.resolve(mockBuilderPacket);
      if (cmd === 'get_contract_drift') return Promise.resolve(mockCleanDriftReport);
      if (cmd === 'get_icarus_state') return Promise.resolve(mockIcarusInactive); // disabled at project level
      if (cmd === 'get_usage_telemetry') return Promise.resolve(mockTelemetry);
      if (cmd === 'get_permission_history') return Promise.resolve([]);
      if (cmd === 'list_builder_sessions') {
        return Promise.resolve([
          {
            session_id: 'active-session-icarus',
            project_id: 'proj-stage3',
            epoch_id: 'epoch-stage3-12345',
            model: 'gemini-3.8-flash-high',
            icarus_mode: true, // run was launched in Icarus mode
            status: 'RUNNING',
            prompt: 'test prompt',
            started_at: '2026-09-22T14:00:00Z',
            duration_ms: 1000,
            usage: { input_tokens: 0, output_tokens: 0, thinking_tokens: 0, cache_read_tokens: 0, total_tokens: 0 },
          },
        ]);
      }
      return Promise.resolve(null);
    });

    await act(async () => {
      render(
        <BuilderControlPlaneView
          projectId="proj-stage3"
          projectName="Stage3 Test Project"
          workflowState="BUILDING"
          onRefreshProject={onRefreshProject}
        />
      );
    });

    // Active-run banner MUST be visible because session has icarus_mode === true
    expect(screen.getByTestId('active-run-icarus-banner')).toBeInTheDocument();
    // Project-level banner MUST NOT be shown
    expect(screen.queryByTestId('icarus-banner')).not.toBeInTheDocument();
  });

  it('does not render active-run Icarus banner if active session was least-privilege even if project preference was turned on later', async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'list_builder_models') return Promise.resolve(mockModels);
      if (cmd === 'get_builder_packet') return Promise.resolve(mockBuilderPacket);
      if (cmd === 'get_contract_drift') return Promise.resolve(mockCleanDriftReport);
      if (cmd === 'get_icarus_state') return Promise.resolve(mockIcarusActive); // enabled at project level
      if (cmd === 'get_usage_telemetry') return Promise.resolve(mockTelemetry);
      if (cmd === 'get_permission_history') return Promise.resolve([]);
      if (cmd === 'list_builder_sessions') {
        return Promise.resolve([
          {
            session_id: 'active-session-least-privilege',
            project_id: 'proj-stage3',
            epoch_id: 'epoch-stage3-12345',
            model: 'gemini-3.8-flash-high',
            icarus_mode: false, // run was NOT launched in Icarus mode
            status: 'RUNNING',
            prompt: 'test prompt',
            started_at: '2026-09-22T14:00:00Z',
            duration_ms: 1000,
            usage: { input_tokens: 0, output_tokens: 0, thinking_tokens: 0, cache_read_tokens: 0, total_tokens: 0 },
          },
        ]);
      }
      return Promise.resolve(null);
    });

    await act(async () => {
      render(
        <BuilderControlPlaneView
          projectId="proj-stage3"
          projectName="Stage3 Test Project"
          workflowState="BUILDING"
          onRefreshProject={onRefreshProject}
        />
      );
    });

    // Active-run banner MUST NOT be visible
    expect(screen.queryByTestId('active-run-icarus-banner')).not.toBeInTheDocument();
    // Project-level banner is shown because project preference is active
    expect(screen.getByTestId('icarus-banner')).toBeInTheDocument();
  });

  it('correctly displays TIMEOUT status in session history', async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'list_builder_models') return Promise.resolve(mockModels);
      if (cmd === 'get_builder_packet') return Promise.resolve(mockBuilderPacket);
      if (cmd === 'get_contract_drift') return Promise.resolve(mockCleanDriftReport);
      if (cmd === 'get_icarus_state') return Promise.resolve(mockIcarusInactive);
      if (cmd === 'get_usage_telemetry') return Promise.resolve(mockTelemetry);
      if (cmd === 'get_permission_history') return Promise.resolve([]);
      if (cmd === 'list_builder_sessions') {
        return Promise.resolve([
          {
            session_id: 'timed-out-session',
            project_id: 'proj-stage3',
            epoch_id: 'epoch-stage3-12345',
            model: 'gemini-3.8-flash-high',
            icarus_mode: false,
            status: 'TIMEOUT',
            prompt: 'timed out prompt',
            started_at: '2026-09-22T14:00:00Z',
            completed_at: '2026-09-22T14:10:00Z',
            duration_ms: 600000,
            usage: { input_tokens: 100, output_tokens: 0, thinking_tokens: 0, cache_read_tokens: 0, total_tokens: 100 },
          },
        ]);
      }
      return Promise.resolve(null);
    });

    await act(async () => {
      render(
        <BuilderControlPlaneView
          projectId="proj-stage3"
          projectName="Stage3 Test Project"
          workflowState="BUILDING"
          onRefreshProject={onRefreshProject}
        />
      );
    });

    expect(screen.getByText('TIMEOUT')).toBeInTheDocument();
  });

  it('shows terminating state when Cancel is pressed and disables duplicate cancellation', async () => {
    let resolveTurnPromise: (val: unknown) => void;
    const pendingTurnPromise = new Promise((resolve) => {
      resolveTurnPromise = resolve;
    });

    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'start_builder_turn') return pendingTurnPromise;
      if (cmd === 'cancel_builder_turn') return Promise.resolve('sess-cancelling');
      if (cmd === 'list_builder_models') return Promise.resolve(mockModels);
      if (cmd === 'get_builder_packet') return Promise.resolve(mockBuilderPacket);
      if (cmd === 'get_contract_drift') return Promise.resolve(mockCleanDriftReport);
      if (cmd === 'get_icarus_state') return Promise.resolve(mockIcarusInactive);
      if (cmd === 'get_usage_telemetry') return Promise.resolve(mockTelemetry);
      if (cmd === 'get_permission_history') return Promise.resolve([]);
      if (cmd === 'list_builder_sessions') return Promise.resolve([]);
      return Promise.resolve(null);
    });

    await act(async () => {
      render(
        <BuilderControlPlaneView
          projectId="proj-stage3"
          projectName="Stage3 Test Project"
          workflowState="FROZEN"
          onRefreshProject={onRefreshProject}
        />
      );
    });

    // Start turn
    await act(async () => {
      fireEvent.click(screen.getByTestId('run-turn-btn'));
    });

    // Verify turn is running and cancel button is visible
    const cancelBtn = screen.getByTestId('cancel-turn-btn');
    expect(cancelBtn).toBeInTheDocument();
    expect(cancelBtn).not.toBeDisabled();

    // Click cancel
    await act(async () => {
      fireEvent.click(cancelBtn);
    });

    // Assert button enters terminating state and becomes disabled
    expect(cancelBtn).toHaveTextContent('Cancellation requested / terminating…');
    expect(cancelBtn).toBeDisabled();

    // Verify cancel_builder_turn was invoked
    expect(mockInvoke).toHaveBeenCalledWith('cancel_builder_turn', {
      projectId: 'proj-stage3',
      sessionId: null,
    });

    // Resolve the turn
    await act(async () => {
      resolveTurnPromise!({
        session_id: 'sess-cancelling',
        conversation_id: 'conv-1',
        status: 'CANCELLED',
        text_response: 'Turn was cancelled',
        cumulative_usage: {
          input_tokens: 100,
          output_tokens: 10,
          thinking_tokens: 0,
          cache_read_tokens: 0,
          total_tokens: 110,
        },
        was_canceled: true,
        stderr: '',
      });
    });

    // After resolution, cancel button disappears and run button is ready
    expect(screen.queryByTestId('cancel-turn-btn')).not.toBeInTheDocument();
    expect(screen.getByTestId('run-turn-btn')).not.toBeDisabled();
  });

  it('synchronizes and locks reasoning effort when model variant encodes effort', async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'list_builder_models') return Promise.resolve(mockModels);
      if (cmd === 'get_builder_packet') return Promise.resolve(mockBuilderPacket);
      if (cmd === 'get_contract_drift') return Promise.resolve(mockCleanDriftReport);
      if (cmd === 'get_icarus_state') return Promise.resolve(mockIcarusInactive);
      if (cmd === 'get_usage_telemetry') return Promise.resolve(mockTelemetry);
      if (cmd === 'get_permission_history') return Promise.resolve([]);
      if (cmd === 'list_builder_sessions') return Promise.resolve([]);
      return Promise.resolve(null);
    });

    await act(async () => {
      render(
        <BuilderControlPlaneView
          projectId="proj-stage3"
          projectName="Stage3 Test Project"
          workflowState="FROZEN"
          onRefreshProject={vi.fn()}
        />
      );
    });

    const effortSelect = screen.getByTestId('effort-select') as HTMLSelectElement;
    const modelSelect = screen.getByTestId('model-select') as HTMLSelectElement;

    // gemini-3.8-flash-high was selected initially
    expect(modelSelect.value).toBe('gemini-3.8-flash-high');
    expect(effortSelect.value).toBe('high');
    expect(effortSelect).toBeDisabled();
    expect(screen.getByTestId('effort-derived-hint')).toBeInTheDocument();

    // Switch to gemini-3.7-flash-medium
    await act(async () => {
      fireEvent.change(modelSelect, { target: { value: 'gemini-3.7-flash-medium' } });
    });
    expect(effortSelect.value).toBe('medium');
    expect(effortSelect).toBeDisabled();
    expect(screen.getByTestId('effort-derived-hint')).toBeInTheDocument();

    // Switch to claude-sonnet-4-6 (no effort suffix)
    await act(async () => {
      fireEvent.change(modelSelect, { target: { value: 'claude-sonnet-4-6' } });
    });
    expect(effortSelect).not.toBeDisabled();
    expect(screen.queryByTestId('effort-derived-hint')).not.toBeInTheDocument();
  });

  it('renders separate Provider SUCCESS and Governance BLOCKED when least-privilege denied an action', async () => {
    const blockedTurnResp: BuilderTurnResponse = {
      conversation_id: 'conv-blocked-1',
      status: 'SUCCESS',
      text_response: 'Completed successfully but command was blocked.',
      cumulative_usage: {
        input_tokens: 500,
        output_tokens: 50,
        thinking_tokens: 0,
        cache_read_tokens: 0,
        total_tokens: 550,
      },
      was_canceled: false,
      stderr: '',
      has_blocked_actions: true,
    };

    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'start_builder_turn') return Promise.resolve(blockedTurnResp);
      if (cmd === 'list_builder_models') return Promise.resolve(mockModels);
      if (cmd === 'get_builder_packet') return Promise.resolve(mockBuilderPacket);
      if (cmd === 'get_contract_drift') return Promise.resolve(mockCleanDriftReport);
      if (cmd === 'get_icarus_state') return Promise.resolve(mockIcarusInactive);
      if (cmd === 'get_usage_telemetry') return Promise.resolve(mockTelemetry);
      if (cmd === 'get_permission_history') return Promise.resolve([]);
      if (cmd === 'list_builder_sessions') return Promise.resolve([]);
      return Promise.resolve(null);
    });

    await act(async () => {
      render(
        <BuilderControlPlaneView
          projectId="proj-stage3"
          projectName="Stage3 Test Project"
          workflowState="FROZEN"
          onRefreshProject={vi.fn()}
        />
      );
    });

    await act(async () => {
      fireEvent.click(screen.getByTestId('run-turn-btn'));
    });

    // Check that provider status is SUCCESS and governance is BLOCKED
    expect(screen.getByTestId('provider-status-badge')).toHaveTextContent('Provider: SUCCESS');
    expect(screen.getByTestId('governance-status-badge')).toHaveTextContent(
      'Governance: BLOCKED — Least-Privilege Denial'
    );
    expect(screen.getByTestId('governance-blocked-alert')).toBeInTheDocument();
  });

  it('restores Governance BLOCKED status from permission history upon reload', async () => {
    const pastSession = {
      session_id: 'sess-persisted-123',
      project_id: 'proj-stage3',
      epoch_id: 'epoch-1',
      architecture_version: '1.0.0',
      contract_fingerprint: 'fp-1',
      git_commit: 'commit-1',
      model: 'gemini-3.8-flash-high',
      effort: 'high',
      icarus_mode: false,
      status: 'SUCCESS',
      error_message: null,
      duration_ms: 12000,
      usage: {
        input_tokens: 200,
        output_tokens: 20,
        thinking_tokens: 0,
        cache_read_tokens: 0,
        total_tokens: 220,
      },
      created_at: '2026-09-23T10:00:00Z',
      completed_at: '2026-09-23T10:00:12Z',
    };

    const blockedPermission: PermissionRecord = {
      id: 99,
      project_id: 'proj-stage3',
      session_id: 'sess-persisted-123',
      tool_name: 'run_command',
      target: 'cargo test',
      risk_level: 'MUTATING',
      decision: 'BLOCKED',
      reason: 'Least-privilege policy auto-denied action without interactive prompt',
      created_at: '2026-09-23T10:00:05Z',
    };

    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'list_builder_models') return Promise.resolve(mockModels);
      if (cmd === 'get_builder_packet') return Promise.resolve(mockBuilderPacket);
      if (cmd === 'get_contract_drift') return Promise.resolve(mockCleanDriftReport);
      if (cmd === 'get_icarus_state') return Promise.resolve(mockIcarusInactive);
      if (cmd === 'get_usage_telemetry') return Promise.resolve(mockTelemetry);
      if (cmd === 'get_permission_history') return Promise.resolve([blockedPermission]);
      if (cmd === 'list_builder_sessions') return Promise.resolve([pastSession]);
      if (cmd === 'get_builder_events') return Promise.resolve([]);
      return Promise.resolve(null);
    });

    await act(async () => {
      render(
        <BuilderControlPlaneView
          projectId="proj-stage3"
          projectName="Stage3 Test Project"
          workflowState="FROZEN"
          onRefreshProject={vi.fn()}
        />
      );
    });

    // Correlated by session_id in permissionHistory, displays Governance: BLOCKED
    expect(screen.getByTestId('provider-status-badge')).toHaveTextContent('Provider: SUCCESS');
    expect(screen.getByTestId('governance-status-badge')).toHaveTextContent(
      'Governance: BLOCKED — Least-Privilege Denial'
    );
    expect(screen.getByTestId('governance-blocked-alert')).toBeInTheDocument();
  });

  it('reconciles stale orphan session and clears UI state on cancellation', async () => {
    const orphanSession = {
      session_id: 'sess-orphan-999',
      project_id: 'proj-stage3',
      epoch_id: 'epoch-1',
      architecture_version: '1.0.0',
      contract_fingerprint: 'fp-1',
      git_commit: 'commit-1',
      model: 'gemini-3.8-flash-high',
      effort: 'high',
      icarus_mode: true,
      status: 'RUNNING',
      error_message: null,
      duration_ms: 5000,
      usage: {
        input_tokens: 0,
        output_tokens: 0,
        thinking_tokens: 0,
        cache_read_tokens: 0,
        total_tokens: 0,
      },
      created_at: '2026-09-23T10:00:00Z',
      completed_at: null,
    };

    let sessionStatus = 'RUNNING';

    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'list_builder_models') return Promise.resolve(mockModels);
      if (cmd === 'get_builder_packet') return Promise.resolve(mockBuilderPacket);
      if (cmd === 'get_contract_drift') return Promise.resolve(mockCleanDriftReport);
      if (cmd === 'get_icarus_state') return Promise.resolve(mockIcarusInactive);
      if (cmd === 'get_usage_telemetry') return Promise.resolve(mockTelemetry);
      if (cmd === 'get_permission_history') return Promise.resolve([]);
      if (cmd === 'list_builder_sessions') {
        return Promise.resolve([{ ...orphanSession, status: sessionStatus }]);
      }
      if (cmd === 'cancel_builder_turn') {
        // Backend reconciles stale orphan session to INTERRUPTED
        sessionStatus = 'INTERRUPTED';
        return Promise.resolve('sess-orphan-999');
      }
      return Promise.resolve(null);
    });

    await act(async () => {
      render(
        <BuilderControlPlaneView
          projectId="proj-stage3"
          projectName="Stage3 Test Project"
          workflowState="FROZEN"
          onRefreshProject={vi.fn()}
        />
      );
    });

    // Initially shows running orphan session with cancel button
    const cancelBtn = screen.getByTestId('cancel-active-run-btn');
    expect(cancelBtn).toBeInTheDocument();

    await act(async () => {
      fireEvent.click(cancelBtn);
    });

    // cancel_builder_turn was invoked, sessions reloaded, and running state cleared
    expect(mockInvoke).toHaveBeenCalledWith('cancel_builder_turn', {
      projectId: 'proj-stage3',
      sessionId: 'sess-orphan-999',
    });
    expect(screen.queryByTestId('active-run-icarus-banner')).not.toBeInTheDocument();
  });

  it('exports project diagnostics and displays feedback with generated file path', async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'list_builder_models') return Promise.resolve(mockModels);
      if (cmd === 'get_builder_packet') return Promise.resolve(mockBuilderPacket);
      if (cmd === 'get_contract_drift') return Promise.resolve(mockCleanDriftReport);
      if (cmd === 'get_icarus_state') return Promise.resolve(mockIcarusInactive);
      if (cmd === 'get_usage_telemetry') return Promise.resolve(mockTelemetry);
      if (cmd === 'get_permission_history') return Promise.resolve([]);
      if (cmd === 'list_builder_sessions') return Promise.resolve([]);
      if (cmd === 'export_project_diagnostics') {
        return Promise.resolve('C:\\path\\to\\coalition-diagnostics-proj-stage3.zip');
      }
      return Promise.resolve(null);
    });

    await act(async () => {
      render(
        <BuilderControlPlaneView
          projectId="proj-stage3"
          projectName="Stage3 Test Project"
          workflowState="FROZEN"
          onRefreshProject={vi.fn()}
        />
      );
    });

    const exportBtn = screen.getByTestId('export-diagnostics-btn');
    expect(exportBtn).toBeInTheDocument();

    await act(async () => {
      fireEvent.click(exportBtn);
    });

    expect(mockInvoke).toHaveBeenCalledWith('export_project_diagnostics', {
      projectId: 'proj-stage3',
      destinationDir: null,
    });

    expect(screen.getByTestId('diagnostic-export-success')).toHaveTextContent(
      'coalition-diagnostics-proj-stage3.zip'
    );
  });

  it('preserves and displays raw provider ERROR separately while canonical status is FAILED', async () => {
    const errorTurnResp: BuilderTurnResponse = {
      conversation_id: 'conv-err-1',
      status: 'FAILED',
      provider_status: 'ERROR',
      text_response: 'Provider engine crashed unexpectedly',
      cumulative_usage: {
        input_tokens: 100,
        output_tokens: 0,
        thinking_tokens: 0,
        cache_read_tokens: 0,
        total_tokens: 100,
      },
      was_canceled: false,
      stderr: 'raw error',
      has_blocked_actions: true,
    };

    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'start_builder_turn') return Promise.resolve(errorTurnResp);
      if (cmd === 'list_builder_models') return Promise.resolve(mockModels);
      if (cmd === 'get_builder_packet') return Promise.resolve(mockBuilderPacket);
      if (cmd === 'get_contract_drift') return Promise.resolve(mockCleanDriftReport);
      if (cmd === 'get_icarus_state') return Promise.resolve(mockIcarusInactive);
      if (cmd === 'get_usage_telemetry') return Promise.resolve(mockTelemetry);
      if (cmd === 'get_permission_history') return Promise.resolve([]);
      if (cmd === 'list_builder_sessions') {
        return Promise.resolve([
          {
            session_id: 'sess-persisted-err',
            project_id: 'proj-stage3',
            epoch_id: 'epoch-1',
            architecture_version: '1.0.0',
            contract_fingerprint: 'fp-1',
            git_commit: 'commit-1',
            model: 'gemini-3.8-flash-high',
            effort: 'high',
            icarus_mode: false,
            status: 'FAILED',
            error_message: 'Provider error',
            duration_ms: 1000,
            usage: {
              input_tokens: 100,
              output_tokens: 0,
              thinking_tokens: 0,
              cache_read_tokens: 0,
              total_tokens: 100,
            },
            created_at: '2026-09-23T10:00:00Z',
            completed_at: '2026-09-23T10:00:01Z',
          },
        ]);
      }
      return Promise.resolve(null);
    });

    await act(async () => {
      render(
        <BuilderControlPlaneView
          projectId="proj-stage3"
          projectName="Stage3 Test Project"
          workflowState="FROZEN"
          onRefreshProject={vi.fn()}
        />
      );
    });

    await act(async () => {
      fireEvent.click(screen.getByTestId('run-turn-btn'));
    });

    expect(screen.getByTestId('provider-status-badge')).toHaveTextContent('Provider: ERROR');
    expect(screen.getByTestId('governance-status-badge')).toHaveTextContent(
      'Governance: BLOCKED — Least-Privilege Denial'
    );
  });
});


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
        effort: 'medium',
        followUpPrompt: null,
        useFakeAgy: false,
      },
    });

    expect(onRefreshProject).toHaveBeenCalled();
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
});

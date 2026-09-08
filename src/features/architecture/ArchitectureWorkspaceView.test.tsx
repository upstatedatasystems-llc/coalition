import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, act, fireEvent } from '@testing-library/react';
import { ArchitectureWorkspaceView } from './ArchitectureWorkspaceView';
import { WorkspaceState, ImportPreview, ReadinessReport } from '../../types';

// Mock Tauri invoke
const mockInvoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

const mockReadinessInitial: ReadinessReport = {
  overall_readiness: 'INCOMPLETE',
  ready_count: 0,
  total_required: 9,
  has_open_questions: false,
  artifacts: [
    { path: 'design/product-vision.md', title: 'Product Vision', status: 'MISSING', character_count: 0 },
    { path: 'design/requirements.md', title: 'Requirements', status: 'MISSING', character_count: 0 },
    { path: 'design/architecture.md', title: 'System Architecture', status: 'MISSING', character_count: 0 },
    { path: 'design/constraints.md', title: 'Constraints', status: 'MISSING', character_count: 0 },
    { path: 'design/interfaces.md', title: 'Interfaces & Protocols', status: 'MISSING', character_count: 0 },
    { path: 'design/security.md', title: 'Security & Boundaries', status: 'MISSING', character_count: 0 },
    { path: 'implementation/validation.yaml', title: 'Validation Commands', status: 'MISSING', character_count: 0 },
    { path: 'implementation/acceptance-criteria.yaml', title: 'Acceptance Criteria', status: 'MISSING', character_count: 0 },
    { path: 'implementation/test-plan.md', title: 'Test Plan', status: 'MISSING', character_count: 0 },
  ],
};

const mockInitialWorkspace: WorkspaceState = {
  readiness: mockReadinessInitial,
  pending_packet: null,
  pending_preview: null,
  history: [],
};

const mockPacketWorkspace: WorkspaceState = {
  ...mockInitialWorkspace,
  pending_packet: {
    metadata: {
      schema: 1,
      project_id: 'test-proj-1',
      packet_id: 'pkt-12345678-abcd',
      role: 'ARCHITECT',
      packet_type: 'ARCHITECT_INITIAL',
      architecture_version: '0.1',
      expected_response: 'ARCHITECT_UPDATE',
      created_at: '2026-09-08T12:00:00Z',
    },
    prompt: '# Coalition Governed Architecture Protocol\nPlease design product-vision.md',
    human_instructions: 'Copy this to ChatGPT',
    project_context_summary: 'Project Alpha context',
  },
};

const mockPreview: ImportPreview = {
  import_id: 'imp-87654321',
  project_id: 'test-proj-1',
  packet_id: 'pkt-12345678-abcd',
  summary: 'Drafted vision and requirements',
  artifacts: [
    {
      path: 'design/product-vision.md',
      title: 'Product Vision',
      action: 'CREATE',
      status: 'NEW',
      proposed_content: '# Vision\nSubstantive content...',
    },
    {
      path: 'design/requirements.md',
      title: 'Requirements',
      action: 'CREATE',
      status: 'NEW',
      proposed_content: '# Requirements\nSubstantive...',
    },
  ],
  open_questions: [
    { id: 'Q-1', question: 'Should we support sqlite only?', status: 'OPEN' },
  ],
  raw_response: 'raw...',
};

describe('ArchitectureWorkspaceView', () => {
  const onRefreshMock = vi.fn().mockResolvedValue(undefined);

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders initial workspace with 9 canonical artifacts and relay controls', async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_architecture_workspace_state') return mockInitialWorkspace;
      return {};
    });

    await act(async () => {
      render(
        <ArchitectureWorkspaceView
          projectId="test-proj-1"
          projectName="Alpha Project"
          workflowState="DRAFT"
          onRefreshProject={onRefreshMock}
        />
      );
    });

    // Check headers
    expect(screen.getByText('Architect Relay')).toBeInTheDocument();
    expect(screen.getByText('Project Design & Readiness')).toBeInTheDocument();
    expect(screen.getByText('INCOMPLETE (0/9)')).toBeInTheDocument();

    // Check relay buttons
    expect(screen.getByRole('button', { name: 'Prepare Architect Prompt' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Copy for ChatGPT (Ctrl+Shift+R)' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Open ChatGPT ↗' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Import from Clipboard (Ctrl+Shift+I)' })).toBeInTheDocument();

    // Check all 9 design artifacts rendered
    expect(screen.getByText('Product Vision')).toBeInTheDocument();
    expect(screen.getByText('Requirements')).toBeInTheDocument();
    expect(screen.getByText('System Architecture')).toBeInTheDocument();
    expect(screen.getByText('Constraints')).toBeInTheDocument();
    expect(screen.getByText('Interfaces & Protocols')).toBeInTheDocument();
    expect(screen.getByText('Security & Boundaries')).toBeInTheDocument();
    expect(screen.getByText('Validation Commands')).toBeInTheDocument();
    expect(screen.getByText('Acceptance Criteria')).toBeInTheDocument();
    expect(screen.getByText('Test Plan')).toBeInTheDocument();
  });

  it('prepares architect prompt and allows inspecting prompt content', async () => {
    let currentState = mockInitialWorkspace;
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_architecture_workspace_state') return currentState;
      if (cmd === 'prepare_architect_relay_packet') {
        currentState = mockPacketWorkspace;
        return currentState.pending_packet;
      }
      return {};
    });

    await act(async () => {
      render(
        <ArchitectureWorkspaceView
          projectId="test-proj-1"
          projectName="Alpha Project"
          workflowState="DRAFT"
          onRefreshProject={onRefreshMock}
        />
      );
    });

    const prepBtn = screen.getByRole('button', { name: 'Prepare Architect Prompt' });
    await act(async () => {
      fireEvent.click(prepBtn);
    });

    expect(mockInvoke).toHaveBeenCalledWith('prepare_architect_relay_packet', {
      projectId: 'test-proj-1',
      customNotes: null,
    });
    expect(onRefreshMock).toHaveBeenCalled();

    // Now copy button should be enabled
    const copyBtn = screen.getByRole('button', { name: 'Copy for ChatGPT (Ctrl+Shift+R)' });
    expect(copyBtn).not.toBeDisabled();

    // Inspector toggle
    const toggleBtn = screen.getByText('▼ Inspect Prompt');
    await act(async () => {
      fireEvent.click(toggleBtn);
    });

    expect(screen.getByText(/# Coalition Governed Architecture Protocol/)).toBeInTheDocument();
  });

  it('copies active packet to clipboard and opens chatgpt URL', async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_architecture_workspace_state') return mockPacketWorkspace;
      if (cmd === 'copy_relay_packet_to_clipboard') return {};
      if (cmd === 'desktop_open_url') return {};
      return {};
    });

    await act(async () => {
      render(
        <ArchitectureWorkspaceView
          projectId="test-proj-1"
          projectName="Alpha Project"
          workflowState="ARCHITECTING"
          onRefreshProject={onRefreshMock}
        />
      );
    });

    // Test Copy
    const copyBtn = screen.getByRole('button', { name: 'Copy for ChatGPT (Ctrl+Shift+R)' });
    await act(async () => {
      fireEvent.click(copyBtn);
    });

    expect(mockInvoke).toHaveBeenCalledWith('copy_relay_packet_to_clipboard', {
      packetId: 'pkt-12345678-abcd',
    });
    expect(screen.getByText('✓ Prompt copied to clipboard!')).toBeInTheDocument();

    // Test Open ChatGPT
    const openBtn = screen.getByRole('button', { name: 'Open ChatGPT ↗' });
    await act(async () => {
      fireEvent.click(openBtn);
    });

    expect(mockInvoke).toHaveBeenCalledWith('open_chatgpt');
  });

  it('imports clipboard response and allows accepting proposed changes', async () => {
    let currentState: WorkspaceState = mockPacketWorkspace;
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_architecture_workspace_state') return currentState;
      if (cmd === 'import_from_clipboard') {
        currentState = { ...currentState, pending_preview: mockPreview };
        return mockPreview;
      }
      if (cmd === 'accept_relay_import') {
        currentState = {
          ...currentState,
          pending_preview: null,
          readiness: {
            ...mockReadinessInitial,
            ready_count: 2,
          },
        };
        return currentState.readiness;
      }
      return {};
    });

    await act(async () => {
      render(
        <ArchitectureWorkspaceView
          projectId="test-proj-1"
          projectName="Alpha Project"
          workflowState="ARCHITECTING"
          onRefreshProject={onRefreshMock}
        />
      );
    });

    // Click Import
    const importBtn = screen.getByRole('button', { name: 'Import from Clipboard (Ctrl+Shift+I)' });
    await act(async () => {
      fireEvent.click(importBtn);
    });

    expect(mockInvoke).toHaveBeenCalledWith('import_from_clipboard', {
      projectId: 'test-proj-1',
    });

    // Proposed changes preview visible
    expect(screen.getByRole('region', { name: 'Proposed Changes Preview' })).toBeInTheDocument();
    expect(screen.getByText('Drafted vision and requirements')).toBeInTheDocument();
    expect(screen.getByText('Open Questions (1)')).toBeInTheDocument();

    // Click Accept
    const acceptBtn = screen.getByRole('button', { name: '✓ Accept & Apply Changes' });
    await act(async () => {
      fireEvent.click(acceptBtn);
    });

    expect(mockInvoke).toHaveBeenCalledWith('accept_relay_import', {
      projectId: 'test-proj-1',
      importId: 'imp-87654321',
    });
    expect(onRefreshMock).toHaveBeenCalled();
  });

  it('allows rejecting proposed changes without modifying artifacts', async () => {
    const stateWithPreview: WorkspaceState = {
      ...mockPacketWorkspace,
      pending_preview: mockPreview,
    };
    let currentState = stateWithPreview;

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_architecture_workspace_state') return currentState;
      if (cmd === 'reject_relay_import') {
        currentState = { ...currentState, pending_preview: null };
        return {};
      }
      return {};
    });

    await act(async () => {
      render(
        <ArchitectureWorkspaceView
          projectId="test-proj-1"
          projectName="Alpha Project"
          workflowState="ARCHITECTING"
          onRefreshProject={onRefreshMock}
        />
      );
    });

    expect(screen.getByRole('region', { name: 'Proposed Changes Preview' })).toBeInTheDocument();

    const rejectBtn = screen.getByRole('button', { name: '✕ Reject' });
    await act(async () => {
      fireEvent.click(rejectBtn);
    });

    expect(mockInvoke).toHaveBeenCalledWith('reject_relay_import', {
      projectId: 'test-proj-1',
      importId: 'imp-87654321',
    });
    expect(onRefreshMock).toHaveBeenCalled();
  });

  it('handles parse failure with manual recovery editor and retry parsing', async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_architecture_workspace_state') return mockPacketWorkspace;
      if (cmd === 'import_from_clipboard') {
        throw {
          code: 'RELAY_PARSE_FAILURE',
          message: 'Failed to find valid coalition_response',
          details: {
            import_id: 'imp-err-999',
            raw_content: 'Some malformed text without proper closing tags',
            message: 'Failed to find valid coalition_response',
          },
        };
      }
      if (cmd === 'retry_parse_import') {
        return mockPreview;
      }
      return {};
    });

    await act(async () => {
      render(
        <ArchitectureWorkspaceView
          projectId="test-proj-1"
          projectName="Alpha Project"
          workflowState="ARCHITECTING"
          onRefreshProject={onRefreshMock}
        />
      );
    });

    // Trigger import that fails parsing
    const importBtn = screen.getByRole('button', { name: 'Import from Clipboard (Ctrl+Shift+I)' });
    await act(async () => {
      fireEvent.click(importBtn);
    });

    // Recovery editor should be open
    expect(screen.getByText('Import Parse Error (Manual Recovery)')).toBeInTheDocument();
    const textarea = screen.getByRole('textbox') as HTMLTextAreaElement;
    expect(textarea.value).toBe('Some malformed text without proper closing tags');

    // Edit the text
    fireEvent.change(textarea, { target: { value: '```yaml\ncoalition_response:\n  schema_version: 1\n```' } });

    // Retry parsing
    const retryBtn = screen.getByRole('button', { name: 'Retry Parsing' });
    await act(async () => {
      fireEvent.click(retryBtn);
    });

    expect(mockInvoke).toHaveBeenCalledWith('retry_parse_import', {
      projectId: 'test-proj-1',
      importId: 'imp-err-999',
      editedRawText: '```yaml\ncoalition_response:\n  schema_version: 1\n```',
    });

    // Recovery panel closes upon success and preview appears
    expect(screen.queryByText('Import Parse Error (Manual Recovery)')).not.toBeInTheDocument();
    expect(screen.getByRole('region', { name: 'Proposed Changes Preview' })).toBeInTheDocument();
  });

  it('views artifact content in modal when clicking on an artifact card', async () => {
    mockInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === 'get_architecture_workspace_state') return mockInitialWorkspace;
      if (cmd === 'get_artifact_content' && args?.artifactPath === 'design/product-vision.md') {
        return '# Product Vision\nThis is the durable vision.';
      }
      return {};
    });

    await act(async () => {
      render(
        <ArchitectureWorkspaceView
          projectId="test-proj-1"
          projectName="Alpha Project"
          workflowState="DRAFT"
          onRefreshProject={onRefreshMock}
        />
      );
    });

    // Click Product Vision card
    const visionCard = screen.getByText('Product Vision');
    await act(async () => {
      fireEvent.click(visionCard);
    });

    expect(mockInvoke).toHaveBeenCalledWith('get_artifact_content', {
      projectId: 'test-proj-1',
      artifactPath: 'design/product-vision.md',
    });

    // Modal dialog rendered
    const modal = screen.getByRole('dialog');
    expect(modal).toBeInTheDocument();
    expect(screen.getByText(/This is the durable vision\./)).toBeInTheDocument();

    // Close modal
    const closeBtn = screen.getByRole('button', { name: 'Close' });
    await act(async () => {
      fireEvent.click(closeBtn);
    });

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('triggers copy and import via keyboard shortcuts Ctrl+Shift+R and Ctrl+Shift+I', async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_architecture_workspace_state') return mockPacketWorkspace;
      if (cmd === 'copy_relay_packet_to_clipboard') return {};
      if (cmd === 'import_from_clipboard') return mockPreview;
      return {};
    });

    await act(async () => {
      render(
        <ArchitectureWorkspaceView
          projectId="test-proj-1"
          projectName="Alpha Project"
          workflowState="ARCHITECTING"
          onRefreshProject={onRefreshMock}
        />
      );
    });

    // Press Ctrl+Shift+R
    await act(async () => {
      window.dispatchEvent(
        new KeyboardEvent('keydown', {
          key: 'R',
          ctrlKey: true,
          shiftKey: true,
          bubbles: true,
        })
      );
    });

    expect(mockInvoke).toHaveBeenCalledWith('copy_relay_packet_to_clipboard', {
      packetId: 'pkt-12345678-abcd',
    });

    // Press Ctrl+Shift+I
    await act(async () => {
      window.dispatchEvent(
        new KeyboardEvent('keydown', {
          key: 'I',
          ctrlKey: true,
          shiftKey: true,
          bubbles: true,
        })
      );
    });

    expect(mockInvoke).toHaveBeenCalledWith('import_from_clipboard', {
      projectId: 'test-proj-1',
    });
  });

  it('allows manual editing and saving artifact with expected fingerprint', async () => {
    mockInvoke.mockImplementation(async (cmd: string, _args: Record<string, unknown>) => {
      if (cmd === 'get_architecture_workspace_state') return mockInitialWorkspace;
      if (cmd === 'get_artifact_content') {
        return {
          path: 'design/product-vision.md',
          content: '# Vision\nExisting content',
          fingerprint: 'fp-12345',
          exists: true,
        };
      }
      if (cmd === 'save_artifact_content') {
        return {
          overall_readiness: 'INCOMPLETE',
          ready_required_count: 1,
          total_required_count: 9,
          artifacts: [],
          has_open_questions: false,
        };
      }
      return {};
    });

    await act(async () => {
      render(
        <ArchitectureWorkspaceView
          projectId="test-proj-1"
          projectName="Alpha Project"
          workflowState="ARCHITECTING"
          onRefreshProject={onRefreshMock}
        />
      );
    });

    // Open modal
    const visionCard = screen.getByText('Product Vision');
    await act(async () => {
      fireEvent.click(visionCard);
    });

    expect(screen.getByText(/Existing content/)).toBeInTheDocument();

    // Click Edit
    const editBtn = screen.getByRole('button', { name: 'Edit' });
    await act(async () => {
      fireEvent.click(editBtn);
    });

    const editor = screen.getByLabelText('Artifact Content Editor');
    expect(editor).toBeInTheDocument();

    // Update content
    await act(async () => {
      fireEvent.change(editor, {
        target: { value: '# Vision\nManually updated substantive content.' },
      });
    });

    // Save
    const saveBtn = screen.getByRole('button', { name: 'Save' });
    await act(async () => {
      fireEvent.click(saveBtn);
    });

    expect(mockInvoke).toHaveBeenCalledWith('save_artifact_content', {
      projectId: 'test-proj-1',
      path: 'design/product-vision.md',
      content: '# Vision\nManually updated substantive content.',
      expectedFingerprint: 'fp-12345',
    });
    expect(onRefreshMock).toHaveBeenCalled();
  });

  it('displays conflict error when saving stale artifact content', async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_architecture_workspace_state') return mockInitialWorkspace;
      if (cmd === 'get_artifact_content') {
        return {
          path: 'design/product-vision.md',
          content: '# Vision\nInitial',
          fingerprint: 'fp-stale',
          exists: true,
        };
      }
      if (cmd === 'save_artifact_content') {
        throw {
          code: 'STALE_ARTIFACT_CONTENT',
          message: 'Artifact has been modified on disk by an external process',
        };
      }
      return {};
    });

    await act(async () => {
      render(
        <ArchitectureWorkspaceView
          projectId="test-proj-1"
          projectName="Alpha Project"
          workflowState="ARCHITECTING"
          onRefreshProject={onRefreshMock}
        />
      );
    });

    const visionCard = screen.getByText('Product Vision');
    await act(async () => {
      fireEvent.click(visionCard);
    });

    const editBtn = screen.getByRole('button', { name: 'Edit' });
    await act(async () => {
      fireEvent.click(editBtn);
    });

    const saveBtn = screen.getByRole('button', { name: 'Save' });
    await act(async () => {
      fireEvent.click(saveBtn);
    });

    expect(screen.getByRole('alert')).toHaveTextContent(
      '[STALE_ARTIFACT_CONTENT] Artifact has been modified on disk by an external process'
    );
  });

  it('displays open questions warning banner when unresolved questions exist', async () => {
    const workspaceWithOpenQuestions: WorkspaceState = {
      ...mockInitialWorkspace,
      readiness: {
        ...mockReadinessInitial,
        has_open_questions: true,
        unresolved_open_questions_count: 2,
      },
    };

    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_architecture_workspace_state') return workspaceWithOpenQuestions;
      return {};
    });

    await act(async () => {
      render(
        <ArchitectureWorkspaceView
          projectId="test-proj-1"
          projectName="Alpha Project"
          workflowState="ARCHITECTING"
          onRefreshProject={onRefreshMock}
        />
      );
    });

    expect(
      screen.getByText(/Open Questions \(2 unresolved\):/)
    ).toBeInTheDocument();
  });
});

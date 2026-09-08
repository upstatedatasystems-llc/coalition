import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, act, fireEvent } from '@testing-library/react';
import App from '../../App';
import { ProjectSummary, ProjectDetails, ActivityEventRecord } from '../../types';

// Mock Tauri API core invoke
const mockInvoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

// Mock Tauri API event listen
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
}));

const sampleProjects: ProjectSummary[] = [
  {
    project_id: 'p-1',
    name: 'Sample Alpha',
    repository_path: '/repos/sample-alpha',
    workflow_state: 'DRAFT',
    git_branch: 'main',
    is_clean: true,
    last_opened_at: '2026-09-08T12:00:00Z',
    is_available: true,
  },
  {
    project_id: 'p-2',
    name: 'Missing Beta',
    repository_path: '/repos/missing-beta',
    workflow_state: 'FROZEN',
    git_branch: null,
    is_clean: null,
    last_opened_at: '2026-09-07T10:00:00Z',
    is_available: false,
  },
];

const sampleDetails: ProjectDetails = {
  project: {
    project_id: 'p-1',
    name: 'Sample Alpha',
    repository_path: '/repos/sample-alpha',
    created_at: '2026-09-08T00:00:00Z',
    updated_at: '2026-09-08T12:00:00Z',
    last_opened_at: '2026-09-08T12:00:00Z',
  },
  workflow_state: {
    project_id: 'p-1',
    state: 'DRAFT',
    resume_state: null,
    revision: 1,
    updated_at: '2026-09-08T12:00:00Z',
  },
  artifact: {
    schema_version: 1,
    project_id: 'p-1',
    name: 'Sample Alpha',
    current_architecture_version: null,
    architecture_state: 'draft',
    created_at: '2026-09-08T00:00:00Z',
  },
  git: {
    git_version: 'git version 2.53.0',
    is_repo: true,
    root_dir: '/repos/sample-alpha',
    current_branch: 'main',
    is_detached: false,
    head_commit: 'abcdef1234567890',
    status: {
      staged: 0,
      unstaged: 0,
      untracked: 0,
      is_clean: true,
    },
    diff_summary: '0 files changed',
  },
  is_available: true,
};

const sampleActivity: ActivityEventRecord[] = [
  {
    id: 1,
    project_id: 'p-1',
    timestamp: '2026-09-08T12:00:00Z',
    event_type: 'PROJECT_REGISTERED',
    actor: 'HUMAN',
    summary: "Governed new project 'Sample Alpha'",
    metadata_json: '{}',
  },
];

describe('Phase 1 Project Dashboard Frontend', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders empty project state when no projects exist', async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_projects') return [];
      if (cmd === 'get_last_opened_project_id') return null;
      return {};
    });

    await act(async () => {
      render(<App />);
    });

    expect(screen.getByText('No projects yet.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Open Existing Repository' })).toBeInTheDocument();
  });

  it('displays registered projects in the list with workflow and git state', async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_projects') return sampleProjects;
      if (cmd === 'get_last_opened_project_id') return null;
      return {};
    });

    await act(async () => {
      render(<App />);
    });

    expect(screen.getByText('Sample Alpha')).toBeInTheDocument();
    expect(screen.getByText('DRAFT')).toBeInTheDocument();
    expect(screen.getByText('⎇ main')).toBeInTheDocument();
    expect(screen.getByText('Clean')).toBeInTheDocument();

    // Unavailable project warning badge
    expect(screen.getByText('Missing Beta')).toBeInTheDocument();
    expect(screen.getByText('FROZEN')).toBeInTheDocument();
    expect(screen.getByText('Repository Missing or Moved')).toBeInTheDocument();
  });

  it('allows selecting a project and views project details, contract, and activity', async () => {
    mockInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === 'list_projects') return sampleProjects;
      if (cmd === 'get_last_opened_project_id') return null;
      if (cmd === 'get_project_details' && args?.projectId === 'p-1') return sampleDetails;
      if (cmd === 'get_project_activity' && args?.projectId === 'p-1') return sampleActivity;
      return {};
    });

    await act(async () => {
      render(<App />);
    });

    // Click on Sample Alpha card
    const card = screen.getByText('Sample Alpha');
    await act(async () => {
      fireEvent.click(card);
    });

    // Detail view rendered
    expect(screen.getByText('Durable Architecture Contract')).toBeInTheDocument();
    expect(screen.getByText('Git Repository State')).toBeInTheDocument();
    expect(screen.getByText('Recent Activity')).toBeInTheDocument();
    expect(screen.getByText("Governed new project 'Sample Alpha'")).toBeInTheDocument();
    expect(screen.getByText('← All Projects')).toBeInTheDocument();
  });

  it('surfaces backend structured error when opening an invalid repository', async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_projects') return [];
      if (cmd === 'get_last_opened_project_id') return null;
      if (cmd === 'register_or_open_project') {
        throw {
          code: 'NOT_A_GIT_REPOSITORY',
          message: 'Directory is not a Git repository',
          details: null,
        };
      }
      return {};
    });

    await act(async () => {
      render(<App />);
    });

    // Open modal
    const openBtn = screen.getByRole('button', { name: 'Open Existing Repository' });
    await act(async () => {
      fireEvent.click(openBtn);
    });

    // Enter path
    const input = screen.getByLabelText('Repository Path:');
    fireEvent.change(input, { target: { value: '/not/a/git/repo' } });

    // Submit
    const submitBtn = screen.getByRole('button', { name: 'Open Project' });
    await act(async () => {
      fireEvent.click(submitBtn);
    });

    // Error message rendered without crashing
    expect(screen.getByText('[NOT_A_GIT_REPOSITORY] Directory is not a Git repository')).toBeInTheDocument();
  });

  it('restores last-opened project automatically on startup', async () => {
    mockInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === 'list_projects') return sampleProjects;
      if (cmd === 'get_last_opened_project_id') return 'p-1';
      if (cmd === 'get_project_details' && args?.projectId === 'p-1') return sampleDetails;
      if (cmd === 'get_project_activity' && args?.projectId === 'p-1') return sampleActivity;
      return {};
    });

    await act(async () => {
      render(<App />);
    });

    // Sample Alpha should be directly opened on startup
    expect(screen.getByText('Durable Architecture Contract')).toBeInTheDocument();
    expect(screen.getByText('p-1')).toBeInTheDocument();

    // Click back button to return to project list
    const backBtn = screen.getByRole('button', { name: '← All Projects' });
    await act(async () => {
      fireEvent.click(backBtn);
    });

    expect(screen.getByText('Sample Alpha')).toBeInTheDocument();
    expect(screen.getByText('Missing Beta')).toBeInTheDocument();
  });

  it('allows switching between Projects and Diagnostics tabs', async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_projects') return [];
      if (cmd === 'get_last_opened_project_id') return null;
      return {};
    });

    await act(async () => {
      render(<App />);
    });

    const diagTab = screen.getByRole('button', { name: 'Diagnostics' });
    const projTab = screen.getByRole('button', { name: 'Projects' });

    await act(async () => {
      fireEvent.click(diagTab);
    });

    expect(diagTab).toHaveClass('active');

    await act(async () => {
      fireEvent.click(projTab);
    });

    expect(projTab).toHaveClass('active');
  });

  it('does not auto-restore an unavailable last-opened repository on startup', async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_projects') return sampleProjects;
      // Last opened is p-2, which is unavailable (is_available: false)
      if (cmd === 'get_last_opened_project_id') return 'p-2';
      return {};
    });

    await act(async () => {
      render(<App />);
    });

    // Should remain on the project list (not in detail view)
    expect(screen.getByText('Sample Alpha')).toBeInTheDocument();
    expect(screen.getByText('Missing Beta')).toBeInTheDocument();
    // Unavailable project is marked unavailable
    expect(screen.getByText('Repository Missing or Moved')).toBeInTheDocument();
    // Detail contract header should NOT be present
    expect(screen.queryByText('Durable Architecture Contract')).not.toBeInTheDocument();
    // No error dialog or banner
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('displays unavailable contract banner when repository is offline without fabricating draft', async () => {
    const unavailableDetails: ProjectDetails = {
      project: {
        project_id: 'p-2',
        name: 'Missing Beta',
        repository_path: '/repos/missing-beta',
        created_at: '2026-09-07T10:00:00Z',
        updated_at: '2026-09-07T10:00:00Z',
        last_opened_at: '2026-09-07T10:00:00Z',
      },
      workflow_state: {
        project_id: 'p-2',
        state: 'FROZEN',
        resume_state: null,
        revision: 2,
        updated_at: '2026-09-07T10:00:00Z',
      },
      artifact: null, // Contract unavailable on disk
      git: null,
      is_available: false,
    };

    mockInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === 'list_projects') return sampleProjects;
      if (cmd === 'get_last_opened_project_id') return null;
      if (cmd === 'get_project_details' && args?.projectId === 'p-2') return unavailableDetails;
      if (cmd === 'get_project_activity') return [];
      return {};
    });

    await act(async () => {
      render(<App />);
    });

    // Click on Missing Beta
    const missingCard = screen.getByText('Missing Beta');
    await act(async () => {
      fireEvent.click(missingCard);
    });

    // Repository unavailable banner
    expect(screen.getByRole('alert')).toHaveTextContent('Repository Unavailable:');
    // Durable contract unavailable message
    expect(screen.getByRole('status')).toHaveTextContent('Durable contract unavailable:');
    // Refresh button should be disabled
    expect(screen.getByRole('button', { name: 'Refresh' })).toBeDisabled();
    // Must NOT fabricate or display "draft" in the architecture contract
    expect(screen.queryByText('DRAFT')).not.toBeInTheDocument();
  });
});

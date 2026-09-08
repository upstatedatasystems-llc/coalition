import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  ProjectSummary,
  ProjectDetails,
  ActivityEventRecord,
  CommandError,
} from './types';
import { EmptyProjectsView } from './features/projects/EmptyProjectsView';
import { ProjectListView } from './features/projects/ProjectListView';
import { ProjectDetailView } from './features/projects/ProjectDetailView';
import { OpenProjectModal } from './features/projects/OpenProjectModal';
import { DiagnosticsView } from './features/diagnostics/DiagnosticsView';
import './App.css';

export const App: React.FC = () => {
  const [activeTab, setActiveTab] = useState<'projects' | 'diagnostics'>('projects');
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const [selectedProjectId, setSelectedProjectId] = useState<string | null>(null);
  const [projectDetails, setProjectDetails] = useState<ProjectDetails | null>(null);
  const [projectActivity, setProjectActivity] = useState<ActivityEventRecord[]>([]);

  const [isModalOpen, setIsModalOpen] = useState(false);
  const [modalError, setModalError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [globalError, setGlobalError] = useState<CommandError | null>(null);
  const [relayToast, setRelayToast] = useState<string | null>(null);

  // On mount: load projects and restore last-opened project if possible
  useEffect(() => {
    loadProjectsAndLastOpened();
  }, []);

  const showRelayToast = (msg: string) => {
    setRelayToast(msg);
    setTimeout(() => setRelayToast(null), 3000);
  };

  const handleGlobalCopyRelay = async () => {
    if (!selectedProjectId) return;
    try {
      const packet = await invoke<{ metadata: { packet_id: string } } | null>(
        'get_pending_relay_packet',
        { projectId: selectedProjectId }
      );
      if (packet) {
        await invoke('copy_relay_packet_to_clipboard', {
          packetId: packet.metadata.packet_id,
        });
        showRelayToast('Prompt copied to clipboard!');
        window.dispatchEvent(new CustomEvent('coalition:relay-packet-copied'));
      } else {
        showRelayToast('No pending relay packet to copy');
      }
    } catch (err: unknown) {
      handleError(err);
    }
  };

  const handleGlobalImportClipboard = async () => {
    if (!selectedProjectId) return;
    try {
      const preview = await invoke('import_from_clipboard', {
        projectId: selectedProjectId,
      });
      showRelayToast('Import preview generated from clipboard!');
      window.dispatchEvent(new CustomEvent('coalition:relay-imported', { detail: preview }));
      await handleRefresh();
    } catch (err: unknown) {
      handleError(err);
    }
  };

  // Top-level keyboard and Tauri global shortcut listeners
  useEffect(() => {
    if (!selectedProjectId) return;

    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.shiftKey) {
        if (e.key === 'R' || e.key === 'r') {
          e.preventDefault();
          handleGlobalCopyRelay();
        } else if (e.key === 'I' || e.key === 'i') {
          e.preventDefault();
          handleGlobalImportClipboard();
        }
      }
    };
    window.addEventListener('keydown', handleKeyDown);

    let unlistenCopy: (() => void) | undefined;
    let unlistenImport: (() => void) | undefined;
    const registerTauriShortcuts = async () => {
      try {
        const { listen } = await import('@tauri-apps/api/event');
        unlistenCopy = await listen('coalition:shortcut-copy-relay', () => {
          handleGlobalCopyRelay();
        });
        unlistenImport = await listen('coalition:shortcut-import-clipboard', () => {
          handleGlobalImportClipboard();
        });
      } catch (_e) {
        // Ignored in non-Tauri / test environments
      }
    };
    registerTauriShortcuts();

    return () => {
      window.removeEventListener('keydown', handleKeyDown);
      if (unlistenCopy) unlistenCopy();
      if (unlistenImport) unlistenImport();
    };
  }, [selectedProjectId]);

  const loadProjectsAndLastOpened = async () => {
    setIsLoading(true);
    setGlobalError(null);
    try {
      const list = await invoke<ProjectSummary[]>('list_projects');
      setProjects(Array.isArray(list) ? list : []);

      // Attempt to restore last-opened project only if it is currently available on disk
      const lastOpenedId = await invoke<string | null>('get_last_opened_project_id');
      if (lastOpenedId && Array.isArray(list)) {
        const found = list.find((p) => p.project_id === lastOpenedId);
        if (found && found.is_available) {
          await selectProject(lastOpenedId);
        }
      }
    } catch (err: unknown) {
      handleError(err);
    } finally {
      setIsLoading(false);
    }
  };

  const selectProject = async (projectId: string) => {
    setIsLoading(true);
    setGlobalError(null);
    try {
      const details = await invoke<ProjectDetails>('get_project_details', { projectId });
      setProjectDetails(details);
      setSelectedProjectId(projectId);
      invoke('set_active_project_id', { projectId }).catch(() => {});

      const activity = await invoke<ActivityEventRecord[]>('get_project_activity', {
        projectId,
        limit: 50,
      });
      setProjectActivity(activity);
    } catch (err: unknown) {
      handleError(err);
    } finally {
      setIsLoading(false);
    }
  };

  const handleRefresh = async () => {
    if (!selectedProjectId) return;
    setIsRefreshing(true);
    setGlobalError(null);
    try {
      const updated = await invoke<ProjectDetails>('refresh_project_git_state', {
        projectId: selectedProjectId,
      });
      setProjectDetails(updated);

      const activity = await invoke<ActivityEventRecord[]>('get_project_activity', {
        projectId: selectedProjectId,
        limit: 50,
      });
      setProjectActivity(activity);

      // Also refresh the summary list
      const list = await invoke<ProjectSummary[]>('list_projects');
      setProjects(list);
    } catch (err: unknown) {
      handleError(err);
    } finally {
      setIsRefreshing(false);
    }
  };

  const handleOpenProjectSubmit = async (path: string) => {
    setModalError(null);
    setIsLoading(true);
    try {
      const details = await invoke<ProjectDetails>('register_or_open_project', { path });
      setProjectDetails(details);
      setSelectedProjectId(details.project.project_id);

      const activity = await invoke<ActivityEventRecord[]>('get_project_activity', {
        projectId: details.project.project_id,
        limit: 50,
      });
      setProjectActivity(activity);

      const list = await invoke<ProjectSummary[]>('list_projects');
      setProjects(list);
      setIsModalOpen(false);
    } catch (err: unknown) {
      const formatted = formatErrorMessage(err);
      setModalError(formatted);
    } finally {
      setIsLoading(false);
    }
  };

  const handleError = (err: unknown) => {
    if (typeof err === 'object' && err !== null && 'code' in err && 'message' in err) {
      setGlobalError(err as CommandError);
    } else if (err instanceof Error) {
      setGlobalError({ code: 'CLIENT_ERROR', message: err.message });
    } else {
      setGlobalError({ code: 'UNKNOWN_ERROR', message: String(err) });
    }
  };

  const formatErrorMessage = (err: unknown): string => {
    if (typeof err === 'object' && err !== null && 'message' in err) {
      const cmdErr = err as CommandError;
      return `[${cmdErr.code || 'ERROR'}] ${cmdErr.message}`;
    }
    if (err instanceof Error) return err.message;
    return String(err);
  };

  return (
    <div className="app-shell">
      <header className="app-header">
        <div className="app-title-group">
          <span className="app-logo">COALITION</span>
          <span className="app-tagline">Governed AI Software Development</span>
        </div>
        <nav className="app-nav">
          <button
            className={`nav-btn ${activeTab === 'projects' ? 'active' : ''}`}
            onClick={() => setActiveTab('projects')}
          >
            Projects
          </button>
          <button
            className={`nav-btn ${activeTab === 'diagnostics' ? 'active' : ''}`}
            onClick={() => setActiveTab('diagnostics')}
          >
            Diagnostics
          </button>
        </nav>
      </header>

      {globalError && (
        <div className="global-error-banner" role="alert">
          <span>
            <strong>Error [{globalError.code}]:</strong> {globalError.message}
          </span>
          <button onClick={() => setGlobalError(null)} aria-label="Dismiss error">
            ✕
          </button>
        </div>
      )}

      <main className="main-content">
        <div className={`tab-panel ${activeTab === 'projects' ? 'is-active' : 'is-inactive'}`}>
          {projects.length === 0 && !isLoading && !selectedProjectId ? (
            <EmptyProjectsView onOpenClick={() => setIsModalOpen(true)} />
          ) : selectedProjectId && projectDetails ? (
            <ProjectDetailView
              details={projectDetails}
              activity={projectActivity}
              onBack={() => {
                setSelectedProjectId(null);
                setProjectDetails(null);
                invoke('set_active_project_id', { projectId: null }).catch(() => {});
              }}
              onRefresh={handleRefresh}
              isRefreshing={isRefreshing}
            />
          ) : (
            <ProjectListView
              projects={projects}
              onSelectProject={(id) => selectProject(id)}
              onOpenNewClick={() => setIsModalOpen(true)}
              isLoading={isLoading}
            />
          )}
        </div>

        <div className={`tab-panel ${activeTab === 'diagnostics' ? 'is-active' : 'is-inactive'}`}>
          <DiagnosticsView />
        </div>
      </main>

      <OpenProjectModal
        isOpen={isModalOpen}
        onClose={() => {
          setIsModalOpen(false);
          setModalError(null);
        }}
        onSubmit={handleOpenProjectSubmit}
        error={modalError}
        isLoading={isLoading}
      />

      {relayToast && (
        <div className="global-toast-banner" role="status">
          {relayToast}
        </div>
      )}
    </div>
  );
};

export default App;

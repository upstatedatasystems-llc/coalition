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

  // On mount: load projects and restore last-opened project if possible
  useEffect(() => {
    loadProjectsAndLastOpened();
  }, []);

  const loadProjectsAndLastOpened = async () => {
    setIsLoading(true);
    setGlobalError(null);
    try {
      const list = await invoke<ProjectSummary[]>('list_projects');
      setProjects(Array.isArray(list) ? list : []);

      // Attempt to restore last-opened project
      const lastOpenedId = await invoke<string | null>('get_last_opened_project_id');
      if (lastOpenedId && Array.isArray(list) && list.some((p) => p.project_id === lastOpenedId)) {
        await selectProject(lastOpenedId);
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
    </div>
  );
};

export default App;

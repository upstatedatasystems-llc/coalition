import React from 'react';
import { ProjectSummary } from '../../types';

interface ProjectListViewProps {
  projects: ProjectSummary[];
  onSelectProject: (projectId: string) => void;
  onOpenNewClick: () => void;
  isLoading?: boolean;
}

export const ProjectListView: React.FC<ProjectListViewProps> = ({
  projects,
  onSelectProject,
  onOpenNewClick,
  isLoading,
}) => {
  return (
    <div className="project-list-container">
      <div className="project-list-header">
        <div>
          <h2>Projects</h2>
          <p className="subtitle">Governed local repositories</p>
        </div>
        <button className="primary-btn" onClick={onOpenNewClick}>
          Open Existing Repository
        </button>
      </div>

      {isLoading && <p>Loading projects...</p>}

      <div className="projects-grid">
        {projects.map((proj) => (
          <div
            key={proj.project_id}
            className={`project-card ${!proj.is_available ? 'unavailable-card' : ''}`}
            onClick={() => onSelectProject(proj.project_id)}
            role="button"
            tabIndex={0}
            onKeyDown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                onSelectProject(proj.project_id);
              }
            }}
          >
            <div className="project-card-header">
              <h3 className="project-name">{proj.name}</h3>
              {proj.workflow_state && (
                <span className={`workflow-badge badge-${proj.workflow_state.toLowerCase()}`}>
                  {proj.workflow_state}
                </span>
              )}
            </div>

            <div className="project-card-path" title={proj.repository_path}>
              {proj.repository_path}
            </div>

            {!proj.is_available && (
              <div className="unavailable-warning-badge">
                Repository Missing or Moved
              </div>
            )}

            <div className="project-card-footer">
              <div className="git-badges">
                {proj.git_branch && (
                  <span className="git-branch-badge">
                    ⎇ {proj.git_branch}
                  </span>
                )}
                {proj.is_clean !== undefined && proj.is_clean !== null && (
                  <span className={`clean-badge ${proj.is_clean ? 'is-clean' : 'is-dirty'}`}>
                    {proj.is_clean ? 'Clean' : 'Changes'}
                  </span>
                )}
              </div>
              <div className="last-opened">
                Opened {new Date(proj.last_opened_at).toLocaleDateString()}
              </div>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
};

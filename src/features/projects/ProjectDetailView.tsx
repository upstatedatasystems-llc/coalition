import React from 'react';
import { ProjectDetails, ActivityEventRecord } from '../../types';
import { ActivityLogView } from '../activity/ActivityLogView';

interface ProjectDetailViewProps {
  details: ProjectDetails;
  activity: ActivityEventRecord[];
  onBack: () => void;
  onRefresh: () => Promise<void>;
  isRefreshing?: boolean;
}

export const ProjectDetailView: React.FC<ProjectDetailViewProps> = ({
  details,
  activity,
  onBack,
  onRefresh,
  isRefreshing,
}) => {
  const { project, workflow_state, artifact, git, is_available } = details;

  return (
    <div className="project-detail-container">
      <div className="detail-top-nav">
        <button className="secondary-btn back-btn" onClick={onBack}>
          ← All Projects
        </button>
        <button
          className="primary-btn refresh-btn"
          onClick={onRefresh}
          disabled={isRefreshing || !is_available}
        >
          {isRefreshing ? 'Refreshing...' : 'Refresh'}
        </button>
      </div>

      {!is_available && (
        <div className="unavailable-banner" role="alert">
          <strong>Repository Unavailable:</strong> The repository at{' '}
          <code>{project.repository_path}</code> could not be located on disk. The registration has
          been preserved. Check if the folder was moved or renamed.
        </div>
      )}

      <header className="project-detail-header">
        <div className="title-row">
          <h1>{project.name}</h1>
          <span className={`workflow-badge badge-${workflow_state.state.toLowerCase()}`}>
            {workflow_state.state}
          </span>
        </div>
        <p className="repo-path">
          <strong>Path:</strong> <code>{project.repository_path}</code>
        </p>
      </header>

      <div className="detail-sections-grid">
        <section className="detail-card">
          <h3>Durable Architecture Contract</h3>
          {artifact ? (
            <div className="info-grid">
              <div>
                <span className="label">Project ID:</span>
                <code>{project.project_id}</code>
              </div>
              <div>
                <span className="label">Schema Version:</span>
                <span>v{artifact.schema_version}</span>
              </div>
              <div>
                <span className="label">Architecture State:</span>
                <span className={`arch-state-badge arch-${artifact.architecture_state}`}>
                  {artifact.architecture_state.toUpperCase()}
                </span>
              </div>
              <div>
                <span className="label">Architecture Version:</span>
                <span>{artifact.current_architecture_version || 'None (Draft)'}</span>
              </div>
              <div>
                <span className="label">Workflow Revision:</span>
                <span>#{workflow_state.revision}</span>
              </div>
              {workflow_state.resume_state && (
                <div>
                  <span className="label">Resume State:</span>
                  <span>{workflow_state.resume_state}</span>
                </div>
              )}
            </div>
          ) : (
            <div className="unavailable-contract-message" role="status">
              <p>
                <strong>Durable contract unavailable:</strong> The repository is inaccessible on disk.
                Durable architecture state exists only in <code>.coalition/project.yaml</code> and
                cannot be read or assumed while the repository is offline.
              </p>
            </div>
          )}
        </section>

        <section className="detail-card">
          <h3>Git Repository State</h3>
          {git ? (
            <div className="info-grid">
              <div>
                <span className="label">Branch:</span>
                <span>{git.current_branch || (git.is_detached ? '(Detached HEAD)' : 'None')}</span>
              </div>
              <div>
                <span className="label">HEAD Commit:</span>
                <span>{git.head_commit ? <code>{git.head_commit.substring(0, 8)}</code> : 'None (Empty repository)'}</span>
              </div>
              <div>
                <span className="label">Working Tree:</span>
                <span className={`clean-badge ${git.status.is_clean ? 'is-clean' : 'is-dirty'}`}>
                  {git.status.is_clean ? 'Clean' : 'Dirty'}
                </span>
              </div>
              <div>
                <span className="label">Changes:</span>
                <span>
                  {git.status.staged} staged, {git.status.unstaged} unstaged, {git.status.untracked} untracked
                </span>
              </div>
              {git.diff_summary && (
                <div className="full-width">
                  <span className="label">Diff Summary:</span>
                  <pre className="diff-summary-box">{git.diff_summary}</pre>
                </div>
              )}
            </div>
          ) : (
            <p className="subdued-text">
              {is_available ? 'Git state unavailable.' : 'Repository missing on disk.'}
            </p>
          )}
        </section>
      </div>

      <section className="detail-card activity-section">
        <h3>Recent Activity</h3>
        <ActivityLogView events={activity} />
      </section>
    </div>
  );
};

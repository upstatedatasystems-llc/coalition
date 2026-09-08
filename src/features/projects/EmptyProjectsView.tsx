import React from 'react';

interface EmptyProjectsViewProps {
  onOpenClick: () => void;
}

export const EmptyProjectsView: React.FC<EmptyProjectsViewProps> = ({ onOpenClick }) => {
  return (
    <div className="empty-projects-view">
      <div className="empty-projects-card">
        <h1 className="empty-title">COALITION</h1>
        <p className="empty-subtitle">Local-first desktop control plane for governed AI development</p>
        <p className="empty-message">No projects yet.</p>
        <button className="primary-btn open-btn" onClick={onOpenClick}>
          Open Existing Repository
        </button>
      </div>
    </div>
  );
};

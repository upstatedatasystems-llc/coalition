import React, { useState } from 'react';

interface OpenProjectModalProps {
  isOpen: boolean;
  onClose: () => void;
  onSubmit: (path: string) => Promise<void>;
  error?: string | null;
  isLoading?: boolean;
}

export const OpenProjectModal: React.FC<OpenProjectModalProps> = ({
  isOpen,
  onClose,
  onSubmit,
  error,
  isLoading,
}) => {
  const [inputPath, setInputPath] = useState('');

  if (!isOpen) return null;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!inputPath.trim()) return;
    await onSubmit(inputPath.trim());
  };

  return (
    <div className="modal-overlay" role="dialog" aria-modal="true" aria-labelledby="modal-title">
      <div className="modal-container">
        <h3 id="modal-title">Open Git Repository</h3>
        <p className="modal-desc">
          Select or enter the absolute path to a local Git repository to govern with Coalition.
        </p>
        <form onSubmit={handleSubmit}>
          <div className="form-group">
            <label htmlFor="repo-path-input">Repository Path:</label>
            <input
              id="repo-path-input"
              type="text"
              className="text-input"
              placeholder="e.g. C:\projects\my-app or /home/user/my-app"
              value={inputPath}
              onChange={(e) => setInputPath(e.target.value)}
              disabled={isLoading}
              autoFocus
            />
          </div>

          {error && <div className="error-alert">{error}</div>}

          <div className="modal-actions">
            <button
              type="button"
              className="secondary-btn"
              onClick={onClose}
              disabled={isLoading}
            >
              Cancel
            </button>
            <button
              type="submit"
              className="primary-btn"
              disabled={isLoading || !inputPath.trim()}
            >
              {isLoading ? 'Opening...' : 'Open Project'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};

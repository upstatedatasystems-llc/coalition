import React from 'react';
import { ActivityEventRecord } from '../../types';

interface ActivityLogViewProps {
  events: ActivityEventRecord[];
  isLoading?: boolean;
}

export const ActivityLogView: React.FC<ActivityLogViewProps> = ({ events, isLoading }) => {
  if (isLoading) {
    return <div className="activity-loading">Loading recent activity...</div>;
  }

  if (events.length === 0) {
    return <div className="activity-empty">No activity events recorded yet.</div>;
  }

  return (
    <div className="activity-timeline">
      <ul className="activity-list">
        {events.map((event) => (
          <li key={event.id} className="activity-item">
            <div className="activity-header">
              <span className={`activity-badge badge-${event.actor.toLowerCase()}`}>
                {event.actor}
              </span>
              <span className="activity-type">{event.event_type}</span>
              <span className="activity-time" title={event.timestamp}>
                {new Date(event.timestamp).toLocaleString()}
              </span>
            </div>
            <div className="activity-summary">{event.summary}</div>
          </li>
        ))}
      </ul>
    </div>
  );
};

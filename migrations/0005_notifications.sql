CREATE TABLE pending_agent_notifications (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id INTEGER NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    body TEXT NOT NULL CHECK (length(body) BETWEEN 1 AND 50000),
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE INDEX pending_agent_notifications_agent ON pending_agent_notifications(agent_id, id);

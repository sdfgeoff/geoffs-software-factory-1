CREATE TABLE agent_reminders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id INTEGER NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    message TEXT NOT NULL CHECK (length(trim(message)) BETWEEN 1 AND 50000),
    deliver_at INTEGER NOT NULL,
    state TEXT NOT NULL DEFAULT 'scheduled' CHECK (state IN ('scheduled', 'enqueued', 'cancelled')),
    wake_event_id INTEGER UNIQUE REFERENCES agent_wake_events(id) ON DELETE SET NULL,
    enqueued_at INTEGER,
    cancelled_at INTEGER,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE INDEX agent_reminders_due
    ON agent_reminders(state, deliver_at, id);

CREATE INDEX agent_reminders_agent
    ON agent_reminders(agent_id, id);

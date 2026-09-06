ALTER TABLE tickets ADD COLUMN created_by_agent_id INTEGER REFERENCES agents(id) ON DELETE RESTRICT;
ALTER TABLE tickets ADD COLUMN reserved_for_agent_id INTEGER REFERENCES agents(id) ON DELETE SET NULL;

CREATE INDEX tickets_reserved_for_agent ON tickets(reserved_for_agent_id, state);

CREATE TABLE ticket_activity (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ticket_id INTEGER NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
    action TEXT NOT NULL CHECK (length(action) BETWEEN 1 AND 100),
    actor_agent_id INTEGER REFERENCES agents(id) ON DELETE RESTRICT,
    previous_reserved_for_agent_id INTEGER REFERENCES agents(id) ON DELETE RESTRICT,
    reserved_for_agent_id INTEGER REFERENCES agents(id) ON DELETE RESTRICT,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE INDEX ticket_activity_ticket_time ON ticket_activity(ticket_id, created_at, id);

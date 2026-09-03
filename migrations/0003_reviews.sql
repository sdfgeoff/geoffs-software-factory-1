CREATE TABLE tickets_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 200),
    description TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL DEFAULT 'todo' CHECK (state IN ('todo', 'in_progress', 'review', 'done')),
    assignee_id INTEGER,
    review_commit TEXT,
    reviewer_id INTEGER REFERENCES agents(id),
    review_feedback TEXT,
    revision INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);

INSERT INTO tickets_new (id, title, description, state, assignee_id, revision, created_at, updated_at)
SELECT id, title, description, state, assignee_id, revision, created_at, updated_at FROM tickets;

DROP TABLE tickets;
ALTER TABLE tickets_new RENAME TO tickets;

CREATE TABLE agent_scopes (
    agent_id INTEGER NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    scope TEXT NOT NULL CHECK (length(scope) BETWEEN 1 AND 100),
    PRIMARY KEY (agent_id, scope)
);

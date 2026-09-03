CREATE TABLE agents (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    state TEXT NOT NULL DEFAULT 'idle' CHECK (state IN ('paused', 'idle', 'running', 'awaiting_approval', 'error')),
    thread_id TEXT UNIQUE,
    worktree_path TEXT NOT NULL UNIQUE,
    active_turn_id TEXT,
    last_error TEXT,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE agent_messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id INTEGER NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('user', 'agent', 'system')),
    body TEXT NOT NULL CHECK (length(body) BETWEEN 1 AND 50000),
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE INDEX agent_messages_agent_time ON agent_messages(agent_id, created_at, id);


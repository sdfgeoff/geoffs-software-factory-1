CREATE TABLE projects (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    path TEXT NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE tickets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 200),
    description TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL DEFAULT 'todo' CHECK (state IN ('todo', 'in_progress', 'done')),
    assignee_id INTEGER,
    revision INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE channels (
    name TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    channel_name TEXT NOT NULL REFERENCES channels(name) ON DELETE CASCADE,
    author TEXT NOT NULL CHECK (length(author) BETWEEN 1 AND 100),
    body TEXT NOT NULL CHECK (length(body) BETWEEN 1 AND 10000),
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE INDEX messages_channel_time ON messages(channel_name, created_at, id);

INSERT INTO channels (name) VALUES ('general');


CREATE TABLE messages_with_optional_body (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    channel_name TEXT NOT NULL REFERENCES channels(name) ON DELETE CASCADE,
    author TEXT NOT NULL CHECK (length(author) BETWEEN 1 AND 100),
    body TEXT NOT NULL CHECK (length(body) BETWEEN 0 AND 10000),
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    agent_id INTEGER REFERENCES agents(id) ON DELETE SET NULL,
    deduplication_key TEXT
);
INSERT INTO messages_with_optional_body
    (id, channel_name, author, body, created_at, agent_id, deduplication_key)
SELECT id, channel_name, author, body, created_at, agent_id, deduplication_key FROM messages;
DROP TABLE messages;
ALTER TABLE messages_with_optional_body RENAME TO messages;
CREATE INDEX messages_channel_time ON messages(channel_name, created_at, id);
CREATE INDEX messages_agent ON messages(agent_id);
CREATE UNIQUE INDEX messages_deduplication ON messages(deduplication_key)
    WHERE deduplication_key IS NOT NULL;

CREATE TABLE agent_messages_with_optional_body (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id INTEGER NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('user', 'agent', 'system')),
    body TEXT NOT NULL CHECK (length(body) BETWEEN 0 AND 50000),
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
INSERT INTO agent_messages_with_optional_body (id, agent_id, role, body, created_at)
SELECT id, agent_id, role, body, created_at FROM agent_messages;
DROP TABLE agent_messages;
ALTER TABLE agent_messages_with_optional_body RENAME TO agent_messages;
CREATE INDEX agent_messages_agent_time ON agent_messages(agent_id, created_at, id);

CREATE TABLE attachments (
    id TEXT PRIMARY KEY NOT NULL,
    owner_agent_id INTEGER REFERENCES agents(id) ON DELETE CASCADE,
    owner_is_human INTEGER NOT NULL CHECK (owner_is_human IN (0, 1)),
    original_filename TEXT NOT NULL,
    media_type TEXT NOT NULL,
    byte_size INTEGER NOT NULL CHECK (byte_size >= 0),
    storage_name TEXT NOT NULL UNIQUE,
    content_kind TEXT CHECK (content_kind IN ('channel_message', 'agent_message', 'ticket_activity')),
    content_id INTEGER,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    linked_at INTEGER,
    CHECK ((owner_is_human = 1 AND owner_agent_id IS NULL) OR
           (owner_is_human = 0 AND owner_agent_id IS NOT NULL)),
    CHECK ((content_kind IS NULL AND content_id IS NULL AND linked_at IS NULL) OR
           (content_kind IS NOT NULL AND content_id IS NOT NULL AND linked_at IS NOT NULL))
);

CREATE INDEX attachments_content_idx ON attachments(content_kind, content_id);
CREATE INDEX attachments_orphan_idx ON attachments(created_at) WHERE content_kind IS NULL;

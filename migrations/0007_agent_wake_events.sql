CREATE TABLE agent_wake_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id INTEGER NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (length(kind) BETWEEN 1 AND 100),
    payload TEXT NOT NULL CHECK (json_valid(payload) AND length(payload) <= 100000),
    deduplication_key TEXT CHECK (deduplication_key IS NULL OR length(deduplication_key) BETWEEN 1 AND 500),
    delivery_state TEXT NOT NULL DEFAULT 'pending' CHECK (delivery_state IN ('pending', 'delivering', 'delivered')),
    delivery_attempts INTEGER NOT NULL DEFAULT 0,
    last_attempt_at INTEGER,
    delivered_at INTEGER,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE UNIQUE INDEX agent_wake_events_deduplication
    ON agent_wake_events(agent_id, deduplication_key)
    WHERE deduplication_key IS NOT NULL;

CREATE INDEX agent_wake_events_delivery
    ON agent_wake_events(agent_id, delivery_state, id);

INSERT INTO agent_wake_events (agent_id, kind, payload, created_at, updated_at)
SELECT agent_id, 'notification', json_object('message', body), created_at, created_at
FROM pending_agent_notifications;

DROP TABLE pending_agent_notifications;

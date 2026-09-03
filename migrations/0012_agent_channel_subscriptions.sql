CREATE TABLE agent_channel_subscriptions (
    agent_id INTEGER NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    channel_name TEXT NOT NULL REFERENCES channels(name) ON DELETE CASCADE,
    PRIMARY KEY (agent_id, channel_name)
);

ALTER TABLE messages ADD COLUMN agent_id INTEGER REFERENCES agents(id) ON DELETE SET NULL;

CREATE INDEX messages_agent ON messages(agent_id);

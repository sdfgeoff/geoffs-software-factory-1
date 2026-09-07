CREATE TABLE agent_project_subscriptions (
    agent_id INTEGER NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    PRIMARY KEY (agent_id, project_id)
);

INSERT INTO agent_project_subscriptions (agent_id, project_id)
SELECT agents.id, projects.id
FROM agents CROSS JOIN projects
WHERE projects.archived_at IS NULL;

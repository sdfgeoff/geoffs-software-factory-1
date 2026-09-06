ALTER TABLE agents RENAME COLUMN worktree_path TO home_path;

CREATE TABLE agent_project_worktrees (
    agent_id INTEGER NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    path TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (agent_id, project_id)
);

INSERT INTO agent_project_worktrees (agent_id, project_id, path)
SELECT agents.id, board_state.current_project_id, agents.home_path
FROM agents
JOIN board_state ON board_state.id = 1
WHERE board_state.current_project_id IS NOT NULL;

UPDATE agents SET home_path = home_path || '.agent-home';

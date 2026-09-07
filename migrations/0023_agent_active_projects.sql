ALTER TABLE agents ADD COLUMN active_project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL;

UPDATE agents
SET active_project_id = (SELECT current_project_id FROM board_state WHERE id = 1);

ALTER TABLE projects ADD COLUMN name TEXT NOT NULL DEFAULT '';
ALTER TABLE projects ADD COLUMN default_branch TEXT NOT NULL DEFAULT 'HEAD';

UPDATE projects SET name = path WHERE name = '';

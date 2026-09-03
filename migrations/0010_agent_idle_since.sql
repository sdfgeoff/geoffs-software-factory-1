ALTER TABLE agents ADD COLUMN idle_since INTEGER;

UPDATE agents SET idle_since = updated_at WHERE state = 'idle';

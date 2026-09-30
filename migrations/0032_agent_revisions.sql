ALTER TABLE agents ADD COLUMN revision INTEGER NOT NULL DEFAULT 1;

ALTER TABLE board_change_events ADD COLUMN resource_type TEXT;
ALTER TABLE board_change_events ADD COLUMN resource_id INTEGER;
ALTER TABLE board_change_events ADD COLUMN resource_revision INTEGER;
CREATE UNIQUE INDEX board_change_events_resource_revision
    ON board_change_events(resource_type, resource_id, resource_revision)
    WHERE resource_type IS NOT NULL;

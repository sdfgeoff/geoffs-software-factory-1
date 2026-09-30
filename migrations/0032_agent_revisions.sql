ALTER TABLE agents ADD COLUMN revision INTEGER NOT NULL DEFAULT 1;
ALTER TABLE board_settings ADD COLUMN revision INTEGER NOT NULL DEFAULT 1;

-- Older agent and settings snapshots do not contain the newly required
-- revisions. Drop the incompatible replay window: reconnecting clients with
-- one of those event IDs receive a snapshot reset from the new high-water mark.
DELETE FROM board_change_events;

ALTER TABLE board_change_events ADD COLUMN resource_type TEXT;
ALTER TABLE board_change_events ADD COLUMN resource_id INTEGER;
ALTER TABLE board_change_events ADD COLUMN resource_revision INTEGER;
CREATE UNIQUE INDEX board_change_events_resource_revision
    ON board_change_events(resource_type, resource_id, resource_revision)
    WHERE resource_type IS NOT NULL;

CREATE TRIGGER tickets_live_sync_insert AFTER INSERT ON tickets BEGIN
    INSERT INTO board_change_events(event_json, resource_type, resource_id, resource_revision)
    VALUES (json_object('kind', 'projection_invalidated', 'scope', 'tickets',
        'key', CAST(NEW.id AS TEXT), 'revision', NEW.revision), 'tickets', NEW.id, NEW.revision);
END;
CREATE TRIGGER tickets_live_sync_update AFTER UPDATE ON tickets
WHEN OLD.revision != NEW.revision BEGIN
    INSERT INTO board_change_events(event_json, resource_type, resource_id, resource_revision)
    VALUES (json_object('kind', 'projection_invalidated', 'scope', 'tickets',
        'key', CAST(NEW.id AS TEXT), 'revision', NEW.revision), 'tickets', NEW.id, NEW.revision);
END;
CREATE TRIGGER ticket_activity_live_sync AFTER INSERT ON ticket_activity BEGIN
    INSERT INTO board_change_events(event_json, resource_type, resource_id, resource_revision)
    VALUES (json_object('kind', 'projection_invalidated', 'scope', 'ticket_activity',
        'key', CAST(NEW.ticket_id AS TEXT), 'revision', NEW.id),
        'ticket_activity', NEW.ticket_id, NEW.id);
END;
CREATE TRIGGER projects_live_sync_insert AFTER INSERT ON projects BEGIN
    INSERT INTO board_change_events(event_json, resource_type, resource_id)
    VALUES (json_object('kind', 'projection_invalidated', 'scope', 'projects',
        'key', CAST(NEW.id AS TEXT)), 'projects', NEW.id);
END;
CREATE TRIGGER projects_live_sync_update AFTER UPDATE ON projects BEGIN
    INSERT INTO board_change_events(event_json, resource_type, resource_id)
    VALUES (json_object('kind', 'projection_invalidated', 'scope', 'projects',
        'key', CAST(NEW.id AS TEXT)), 'projects', NEW.id);
END;
CREATE TRIGGER channels_live_sync_insert AFTER INSERT ON channels BEGIN
    INSERT INTO board_change_events(event_json, resource_type)
    VALUES (json_object('kind', 'projection_invalidated', 'scope', 'channels',
        'key', NEW.name), 'channels');
END;
CREATE TRIGGER board_settings_live_sync_insert AFTER INSERT ON board_settings BEGIN
    INSERT INTO board_change_events(event_json, resource_type, resource_id, resource_revision)
    VALUES (json_object('kind', 'projection_invalidated', 'scope', 'settings',
        'key', 'board', 'revision', NEW.revision), 'settings', NEW.id, NEW.revision);
END;
CREATE TRIGGER board_settings_live_sync_update AFTER UPDATE ON board_settings BEGIN
    INSERT INTO board_change_events(event_json, resource_type, resource_id, resource_revision)
    VALUES (json_object('kind', 'projection_invalidated', 'scope', 'settings',
        'key', 'board', 'revision', NEW.revision), 'settings', NEW.id, NEW.revision);
END;
CREATE TRIGGER agent_scopes_live_sync_insert AFTER INSERT ON agent_scopes BEGIN
    INSERT INTO board_change_events(event_json, resource_type, resource_id)
    VALUES (json_object('kind', 'projection_invalidated', 'scope', 'agent_details',
        'key', CAST(NEW.agent_id AS TEXT)), 'agent_details', NEW.agent_id);
END;
CREATE TRIGGER agent_scopes_live_sync_delete AFTER DELETE ON agent_scopes BEGIN
    INSERT INTO board_change_events(event_json, resource_type, resource_id)
    VALUES (json_object('kind', 'projection_invalidated', 'scope', 'agent_details',
        'key', CAST(OLD.agent_id AS TEXT)), 'agent_details', OLD.agent_id);
END;
CREATE TRIGGER agent_channels_live_sync_insert AFTER INSERT ON agent_channel_subscriptions BEGIN
    INSERT INTO board_change_events(event_json, resource_type, resource_id)
    VALUES (json_object('kind', 'projection_invalidated', 'scope', 'agent_details',
        'key', CAST(NEW.agent_id AS TEXT)), 'agent_details', NEW.agent_id);
END;
CREATE TRIGGER agent_channels_live_sync_delete AFTER DELETE ON agent_channel_subscriptions BEGIN
    INSERT INTO board_change_events(event_json, resource_type, resource_id)
    VALUES (json_object('kind', 'projection_invalidated', 'scope', 'agent_details',
        'key', CAST(OLD.agent_id AS TEXT)), 'agent_details', OLD.agent_id);
END;
CREATE TRIGGER agent_projects_live_sync_insert AFTER INSERT ON agent_project_subscriptions BEGIN
    INSERT INTO board_change_events(event_json, resource_type, resource_id)
    VALUES (json_object('kind', 'projection_invalidated', 'scope', 'agent_details',
        'key', CAST(NEW.agent_id AS TEXT)), 'agent_details', NEW.agent_id);
END;
CREATE TRIGGER agent_projects_live_sync_delete AFTER DELETE ON agent_project_subscriptions BEGIN
    INSERT INTO board_change_events(event_json, resource_type, resource_id)
    VALUES (json_object('kind', 'projection_invalidated', 'scope', 'agent_details',
        'key', CAST(OLD.agent_id AS TEXT)), 'agent_details', OLD.agent_id);
END;
CREATE TRIGGER board_messages_live_sync AFTER INSERT ON messages BEGIN
    INSERT INTO board_change_events(event_json, resource_type)
    VALUES (json_object('kind', 'projection_invalidated', 'scope', 'board_messages',
        'key', NEW.channel_name), 'board_messages');
END;
CREATE TRIGGER agent_messages_live_sync AFTER INSERT ON agent_messages BEGIN
    INSERT INTO board_change_events(event_json, resource_type, resource_id)
    VALUES (json_object('kind', 'projection_invalidated', 'scope', 'agent_messages',
        'key', CAST(NEW.agent_id AS TEXT)), 'agent_messages', NEW.agent_id);
END;

CREATE TRIGGER board_change_events_retention AFTER INSERT ON board_change_events BEGIN
    DELETE FROM board_change_events
    WHERE id <= COALESCE((
        SELECT id FROM board_change_events ORDER BY id DESC LIMIT 1 OFFSET 1024
    ), 0);
END;

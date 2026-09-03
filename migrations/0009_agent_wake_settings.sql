ALTER TABLE agents ADD COLUMN wake_on_pending_task INTEGER NOT NULL DEFAULT 0
    CHECK (wake_on_pending_task IN (0, 1));

ALTER TABLE agents ADD COLUMN idle_heartbeat_minutes INTEGER
    CHECK (idle_heartbeat_minutes IS NULL OR idle_heartbeat_minutes BETWEEN 1 AND 10080);

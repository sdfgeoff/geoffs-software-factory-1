CREATE TABLE board_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    merge_notification_channel TEXT,
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);

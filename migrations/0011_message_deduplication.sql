ALTER TABLE messages ADD COLUMN deduplication_key TEXT;

CREATE UNIQUE INDEX messages_deduplication
    ON messages(deduplication_key)
    WHERE deduplication_key IS NOT NULL;

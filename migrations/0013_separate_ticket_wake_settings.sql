ALTER TABLE agents ADD COLUMN wake_on_pending_work INTEGER NOT NULL DEFAULT 0
    CHECK (wake_on_pending_work IN (0, 1));

ALTER TABLE agents ADD COLUMN wake_on_pending_review INTEGER NOT NULL DEFAULT 0
    CHECK (wake_on_pending_review IN (0, 1));

UPDATE agents
SET wake_on_pending_work = wake_on_pending_task,
    wake_on_pending_review = wake_on_pending_task;

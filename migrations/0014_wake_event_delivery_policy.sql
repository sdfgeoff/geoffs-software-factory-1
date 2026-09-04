ALTER TABLE agent_wake_events
ADD COLUMN delivery_policy TEXT NOT NULL DEFAULT 'immediate'
CHECK (delivery_policy IN ('idle_only', 'immediate'));

UPDATE agent_wake_events
SET delivery_policy = 'idle_only'
WHERE kind IN ('ticket.available', 'ticket.assigned', 'ticket.review_requested');

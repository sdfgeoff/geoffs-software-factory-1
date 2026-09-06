CREATE TABLE ticket_dependencies (
    ticket_id INTEGER NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
    blocker_id INTEGER NOT NULL REFERENCES tickets(id) ON DELETE RESTRICT,
    PRIMARY KEY (ticket_id, blocker_id),
    CHECK (ticket_id != blocker_id)
);

CREATE INDEX ticket_dependencies_blocker ON ticket_dependencies(blocker_id);

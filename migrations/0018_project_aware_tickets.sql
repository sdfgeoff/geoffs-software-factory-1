CREATE TABLE projects_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL UNIQUE,
    archived_at INTEGER,
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);

INSERT INTO projects_new (id, path, updated_at)
SELECT id, path, updated_at FROM projects;

DROP TABLE projects;
ALTER TABLE projects_new RENAME TO projects;

CREATE TABLE board_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    current_project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL
);

INSERT INTO board_state (id, current_project_id)
VALUES (1, (SELECT id FROM projects ORDER BY id LIMIT 1))
ON CONFLICT(id) DO UPDATE SET current_project_id = excluded.current_project_id;

CREATE TABLE ticket_dependencies_backup AS
SELECT ticket_id, blocker_id FROM ticket_dependencies;
DROP TABLE ticket_dependencies;

CREATE TABLE ticket_activity_backup AS
SELECT id, ticket_id, action, actor_agent_id, previous_reserved_for_agent_id,
       reserved_for_agent_id, created_at
FROM ticket_activity;
DROP TABLE ticket_activity;

CREATE TABLE tickets_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE RESTRICT,
    title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 200),
    description TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL DEFAULT 'todo' CHECK (state IN ('todo', 'in_progress', 'review', 'done')),
    assignee_id INTEGER,
    review_commit TEXT,
    reviewer_id INTEGER REFERENCES agents(id),
    review_feedback TEXT,
    revision INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
    review_target_branch TEXT,
    implementation_branch TEXT,
    created_by_agent_id INTEGER REFERENCES agents(id) ON DELETE RESTRICT,
    reserved_for_agent_id INTEGER REFERENCES agents(id) ON DELETE SET NULL,
    review_project_id INTEGER REFERENCES projects(id) ON DELETE RESTRICT,
    review_project_path TEXT,
    CHECK ((review_project_id IS NULL) = (review_project_path IS NULL))
);

INSERT INTO tickets_new (
    id, project_id, title, description, state, assignee_id, review_commit,
    reviewer_id, review_feedback, revision, created_at, updated_at,
    review_target_branch, implementation_branch, created_by_agent_id,
    reserved_for_agent_id, review_project_id, review_project_path
)
SELECT id, (SELECT current_project_id FROM board_state WHERE id = 1),
       title, description, state, assignee_id, review_commit,
       reviewer_id, review_feedback, revision, created_at, updated_at,
       review_target_branch, implementation_branch, created_by_agent_id,
       reserved_for_agent_id,
       CASE WHEN state = 'review' THEN (SELECT current_project_id FROM board_state WHERE id = 1) END,
       CASE WHEN state = 'review' THEN (
           SELECT path FROM projects
           WHERE id = (SELECT current_project_id FROM board_state WHERE id = 1)
       ) END
FROM tickets;

DROP TABLE tickets;
ALTER TABLE tickets_new RENAME TO tickets;

CREATE INDEX tickets_project ON tickets(project_id, id);
CREATE INDEX tickets_reserved_for_agent ON tickets(reserved_for_agent_id, state);

CREATE TABLE ticket_dependencies (
    ticket_id INTEGER NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
    blocker_id INTEGER NOT NULL REFERENCES tickets(id) ON DELETE RESTRICT,
    PRIMARY KEY (ticket_id, blocker_id),
    CHECK (ticket_id != blocker_id)
);
INSERT INTO ticket_dependencies SELECT ticket_id, blocker_id FROM ticket_dependencies_backup;
DROP TABLE ticket_dependencies_backup;
CREATE INDEX ticket_dependencies_blocker ON ticket_dependencies(blocker_id);

CREATE TABLE ticket_activity (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ticket_id INTEGER NOT NULL REFERENCES tickets(id) ON DELETE CASCADE,
    action TEXT NOT NULL CHECK (length(action) BETWEEN 1 AND 100),
    actor_agent_id INTEGER REFERENCES agents(id) ON DELETE RESTRICT,
    previous_reserved_for_agent_id INTEGER REFERENCES agents(id) ON DELETE RESTRICT,
    reserved_for_agent_id INTEGER REFERENCES agents(id) ON DELETE RESTRICT,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
INSERT INTO ticket_activity SELECT id, ticket_id, action, actor_agent_id,
    previous_reserved_for_agent_id, reserved_for_agent_id, created_at
FROM ticket_activity_backup;
DROP TABLE ticket_activity_backup;
CREATE INDEX ticket_activity_ticket_time ON ticket_activity(ticket_id, created_at, id);

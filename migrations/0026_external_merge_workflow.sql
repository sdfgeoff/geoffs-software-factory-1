ALTER TABLE projects ADD COLUMN completion_workflow TEXT NOT NULL DEFAULT 'board_managed'
    CHECK (completion_workflow IN ('board_managed', 'external'));

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
    state TEXT NOT NULL DEFAULT 'todo' CHECK (state IN ('todo', 'in_progress', 'review', 'awaiting_external_merge', 'done', 'cancelled')),
    assignee_id INTEGER,
    review_commit TEXT,
    reviewer_id INTEGER REFERENCES agents(id),
    review_feedback TEXT,
    review_completion_workflow TEXT CHECK (review_completion_workflow IN ('board_managed', 'external')),
    external_change_url TEXT,
    external_merge_reference TEXT,
    external_merged_at INTEGER,
    revision INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
    review_target_branch TEXT,
    implementation_branch TEXT,
    created_by_agent_id INTEGER REFERENCES agents(id) ON DELETE RESTRICT,
    reserved_for_agent_id INTEGER REFERENCES agents(id) ON DELETE SET NULL,
    review_project_id INTEGER REFERENCES projects(id) ON DELETE RESTRICT,
    review_project_path TEXT,
    CHECK ((review_project_id IS NULL) = (review_project_path IS NULL)),
    CHECK (state != 'awaiting_external_merge' OR review_completion_workflow = 'external')
);

INSERT INTO tickets_new (
    id, project_id, title, description, state, assignee_id, review_commit,
    reviewer_id, review_feedback, review_completion_workflow, revision,
    created_at, updated_at, review_target_branch, implementation_branch,
    created_by_agent_id, reserved_for_agent_id, review_project_id,
    review_project_path
)
SELECT id, project_id, title, description, state, assignee_id, review_commit,
       reviewer_id, review_feedback,
       CASE WHEN state = 'review' THEN 'board_managed' ELSE NULL END,
       revision, created_at, updated_at, review_target_branch,
       implementation_branch, created_by_agent_id, reserved_for_agent_id,
       review_project_id, review_project_path
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
    detail TEXT,
    actor_agent_id INTEGER REFERENCES agents(id) ON DELETE RESTRICT,
    previous_reserved_for_agent_id INTEGER REFERENCES agents(id) ON DELETE RESTRICT,
    reserved_for_agent_id INTEGER REFERENCES agents(id) ON DELETE RESTRICT,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
INSERT INTO ticket_activity (
    id, ticket_id, action, actor_agent_id, previous_reserved_for_agent_id,
    reserved_for_agent_id, created_at
)
SELECT id, ticket_id, action, actor_agent_id, previous_reserved_for_agent_id,
       reserved_for_agent_id, created_at
FROM ticket_activity_backup;
DROP TABLE ticket_activity_backup;
CREATE INDEX ticket_activity_ticket_time ON ticket_activity(ticket_id, created_at, id);

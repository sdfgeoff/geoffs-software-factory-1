use sqlx::{Sqlite, Transaction};

use crate::{
    domain::{Message, Project, Ticket, TicketState},
    error::AppError,
};

use super::{MessageWrite, Store, insert_message};

#[derive(Debug)]
pub struct NewTicket {
    pub title: String,
    pub description: String,
}

#[derive(Debug, Default)]
pub struct TicketChanges {
    pub title: Option<String>,
    pub description: Option<String>,
    pub state: Option<TicketState>,
    pub assignee_id: Option<Option<i64>>,
    pub expected_revision: Option<i64>,
}

pub(crate) struct MergeNotification<'a> {
    pub channel: &'a str,
    pub author: &'a str,
    pub body: &'a str,
    pub deduplication_key: &'a str,
}

#[derive(Clone, Copy)]
enum TicketWakeReason {
    Available,
    Assigned,
    ReviewRequested,
}

impl Store {
    pub async fn project(&self) -> Result<Option<Project>, sqlx::Error> {
        sqlx::query_as("SELECT path, updated_at FROM projects WHERE id = 1")
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn set_project(&self, path: &str) -> Result<Project, sqlx::Error> {
        sqlx::query(
            "INSERT INTO projects (id, path) VALUES (1, ?) \
             ON CONFLICT(id) DO UPDATE SET path = excluded.path, updated_at = unixepoch()",
        )
        .bind(path)
        .execute(&self.pool)
        .await?;
        Ok(self.project().await?.expect("project was just inserted"))
    }

    pub async fn tickets(&self) -> Result<Vec<Ticket>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id, title, description, state, assignee_id, implementation_branch, review_commit, review_target_branch, reviewer_id, review_feedback, revision, created_at, updated_at \
             FROM tickets ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn ticket(&self, id: i64) -> Result<Option<Ticket>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id, title, description, state, assignee_id, implementation_branch, review_commit, review_target_branch, reviewer_id, review_feedback, revision, created_at, updated_at \
             FROM tickets WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn create_ticket(&self, new: NewTicket) -> Result<Ticket, AppError> {
        Ok(self.create_ticket_with_wakes(new).await?.0)
    }

    pub async fn create_ticket_with_wakes(
        &self,
        new: NewTicket,
    ) -> Result<(Ticket, Vec<i64>), AppError> {
        validate_title(&new.title)?;
        let mut transaction = self.pool.begin().await?;
        let result = sqlx::query("INSERT INTO tickets (title, description) VALUES (?, ?)")
            .bind(new.title.trim())
            .bind(new.description)
            .execute(&mut *transaction)
            .await?;
        let ticket = ticket_in_transaction(&mut transaction, result.last_insert_rowid()).await?;
        let wake_agent_ids = insert_pending_ticket_wakes(
            &mut transaction,
            None,
            &ticket,
            TicketWakeReason::Available,
        )
        .await?;
        transaction.commit().await?;
        Ok((ticket, wake_agent_ids))
    }

    pub async fn update_ticket(&self, id: i64, changes: TicketChanges) -> Result<Ticket, AppError> {
        Ok(self.update_ticket_with_wakes(id, changes).await?.0)
    }

    pub async fn update_ticket_with_wakes(
        &self,
        id: i64,
        changes: TicketChanges,
    ) -> Result<(Ticket, Vec<i64>), AppError> {
        let current = self
            .ticket(id)
            .await?
            .ok_or_else(|| AppError::not_found("ticket not found"))?;
        if let Some(expected) = changes.expected_revision
            && expected != current.revision
        {
            return Err(AppError::conflict("ticket was changed by another user"));
        }
        let title = changes.title.unwrap_or_else(|| current.title.clone());
        validate_title(&title)?;
        let description = changes
            .description
            .unwrap_or_else(|| current.description.clone());
        let state = changes.state.unwrap_or(current.state);
        let assignee_id = changes.assignee_id.unwrap_or(current.assignee_id);
        if let Some(agent_id) = assignee_id
            && self.agent(agent_id).await?.is_none()
        {
            return Err(AppError::bad_request("assignee agent does not exist"));
        }
        let mut transaction = self.pool.begin().await?;
        let result = sqlx::query(
            "UPDATE tickets SET title = ?, description = ?, state = ?, assignee_id = ?, revision = revision + 1, \
             updated_at = unixepoch() WHERE id = ? AND revision = ?",
        )
        .bind(title.trim())
        .bind(description)
        .bind(state)
        .bind(assignee_id)
        .bind(id)
        .bind(current.revision)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::conflict("ticket was changed by another user"));
        }
        let ticket = ticket_in_transaction(&mut transaction, id).await?;
        let reason = if current.assignee_id != ticket.assignee_id && ticket.assignee_id.is_some() {
            Some(TicketWakeReason::Assigned)
        } else if !ticket_is_available(&current) && ticket_is_available(&ticket) {
            Some(TicketWakeReason::Available)
        } else {
            None
        };
        let wake_agent_ids = if let Some(reason) = reason {
            insert_pending_ticket_wakes(&mut transaction, Some(&current), &ticket, reason).await?
        } else {
            Vec::new()
        };
        transaction.commit().await?;
        Ok((ticket, wake_agent_ids))
    }

    pub async fn claim_ticket(
        &self,
        id: i64,
        agent_id: i64,
        expected_revision: Option<i64>,
    ) -> Result<Ticket, AppError> {
        Ok(self
            .claim_ticket_with_wakes(id, agent_id, expected_revision)
            .await?
            .0)
    }

    pub async fn claim_ticket_with_wakes(
        &self,
        id: i64,
        agent_id: i64,
        expected_revision: Option<i64>,
    ) -> Result<(Ticket, Vec<i64>), AppError> {
        if self.agent(agent_id).await?.is_none() {
            return Err(AppError::bad_request("claiming agent does not exist"));
        }
        let current = self
            .ticket(id)
            .await?
            .ok_or_else(|| AppError::not_found("ticket not found"))?;
        let mut transaction = self.pool.begin().await?;
        let claimed: Option<Ticket> = sqlx::query_as(
            "UPDATE tickets SET assignee_id = ?, state = 'in_progress', revision = revision + 1, \
             updated_at = unixepoch() WHERE id = ? AND (assignee_id IS NULL OR assignee_id = ?) \
             AND (? IS NULL OR revision = ?) \
             RETURNING id, title, description, state, assignee_id, implementation_branch, review_commit, review_target_branch, reviewer_id, review_feedback, revision, created_at, updated_at",
        )
        .bind(agent_id)
        .bind(id)
        .bind(agent_id)
        .bind(expected_revision)
        .bind(expected_revision)
        .fetch_optional(&mut *transaction)
        .await?;
        match claimed {
            Some(ticket) => {
                let wake_agent_ids = if current.assignee_id != ticket.assignee_id {
                    insert_pending_ticket_wakes(
                        &mut transaction,
                        Some(&current),
                        &ticket,
                        TicketWakeReason::Assigned,
                    )
                    .await?
                } else {
                    Vec::new()
                };
                transaction.commit().await?;
                Ok((ticket, wake_agent_ids))
            }
            None => Err(AppError::conflict(
                "ticket is already claimed or has changed",
            )),
        }
    }

    pub async fn submit_review(
        &self,
        id: i64,
        agent_id: i64,
        implementation_branch: Option<&str>,
        commit: &str,
        target_branch: &str,
        expected_revision: Option<i64>,
    ) -> Result<Ticket, AppError> {
        Ok(self
            .submit_review_with_wakes(
                id,
                agent_id,
                implementation_branch,
                commit,
                target_branch,
                expected_revision,
            )
            .await?
            .0)
    }

    pub async fn submit_review_with_wakes(
        &self,
        id: i64,
        agent_id: i64,
        implementation_branch: Option<&str>,
        commit: &str,
        target_branch: &str,
        expected_revision: Option<i64>,
    ) -> Result<(Ticket, Vec<i64>), AppError> {
        let current = self
            .ticket(id)
            .await?
            .ok_or_else(|| AppError::not_found("ticket not found"))?;
        let mut transaction = self.pool.begin().await?;
        let ticket = sqlx::query_as(
            "UPDATE tickets SET state = 'review', implementation_branch = ?, review_commit = ?, review_target_branch = ?, reviewer_id = NULL, \
             revision = revision + 1, updated_at = unixepoch() \
             WHERE id = ? AND state = 'in_progress' AND assignee_id = ? \
             AND (? IS NULL OR revision = ?) \
             RETURNING id, title, description, state, assignee_id, implementation_branch, review_commit, review_target_branch, reviewer_id, review_feedback, revision, created_at, updated_at",
        )
        .bind(implementation_branch)
        .bind(commit)
        .bind(target_branch)
        .bind(id)
        .bind(agent_id)
        .bind(expected_revision)
        .bind(expected_revision)
        .fetch_optional(&mut *transaction)
        .await?;
        let ticket = ticket.ok_or_else(|| {
            AppError::conflict(
                "ticket is not assigned to this agent, is not in progress, or has changed",
            )
        })?;
        let wake_agent_ids = insert_pending_ticket_wakes(
            &mut transaction,
            Some(&current),
            &ticket,
            TicketWakeReason::ReviewRequested,
        )
        .await?;
        transaction.commit().await?;
        Ok((ticket, wake_agent_ids))
    }

    pub async fn claim_review(
        &self,
        id: i64,
        reviewer_id: i64,
        expected_revision: Option<i64>,
    ) -> Result<Ticket, AppError> {
        if !self.has_agent_scope(reviewer_id, "approve_review").await? {
            return Err(AppError::forbidden("agent lacks approve_review scope"));
        }
        let ticket = sqlx::query_as(
            "UPDATE tickets SET reviewer_id = ?, revision = revision + 1, updated_at = unixepoch() \
             WHERE id = ? AND state = 'review' AND assignee_id != ? \
             AND (reviewer_id IS NULL OR reviewer_id = ?) AND (? IS NULL OR revision = ?) \
             RETURNING id, title, description, state, assignee_id, implementation_branch, review_commit, review_target_branch, reviewer_id, review_feedback, revision, created_at, updated_at",
        )
        .bind(reviewer_id)
        .bind(id)
        .bind(reviewer_id)
        .bind(reviewer_id)
        .bind(expected_revision)
        .bind(expected_revision)
        .fetch_optional(&self.pool)
        .await?;
        ticket.ok_or_else(|| {
            AppError::conflict("review is unavailable, belongs to this agent, or has changed")
        })
    }

    pub async fn request_changes(
        &self,
        id: i64,
        reviewer_id: i64,
        feedback: &str,
        expected_revision: Option<i64>,
    ) -> Result<Ticket, AppError> {
        if !self.has_agent_scope(reviewer_id, "approve_review").await? {
            return Err(AppError::forbidden("agent lacks approve_review scope"));
        }
        if feedback.trim().is_empty() || feedback.chars().count() > 10_000 {
            return Err(AppError::bad_request(
                "review feedback must contain between 1 and 10000 characters",
            ));
        }
        let ticket = sqlx::query_as(
            "UPDATE tickets SET state = 'in_progress', review_feedback = ?, revision = revision + 1, \
             updated_at = unixepoch() WHERE id = ? AND state = 'review' AND reviewer_id = ? \
             AND (? IS NULL OR revision = ?) \
             RETURNING id, title, description, state, assignee_id, implementation_branch, review_commit, review_target_branch, reviewer_id, review_feedback, revision, created_at, updated_at",
        )
        .bind(feedback.trim())
        .bind(id)
        .bind(reviewer_id)
        .bind(expected_revision)
        .bind(expected_revision)
        .fetch_optional(&self.pool)
        .await?;
        ticket.ok_or_else(|| {
            AppError::conflict("review is not assigned to this agent or has changed")
        })
    }

    pub async fn complete_review(
        &self,
        id: i64,
        reviewer_id: i64,
        commit: &str,
        expected_revision: Option<i64>,
    ) -> Result<Ticket, AppError> {
        Ok(self
            .complete_review_with_message(id, reviewer_id, commit, expected_revision, None)
            .await?
            .0)
    }

    pub(crate) async fn complete_review_and_post_message(
        &self,
        id: i64,
        reviewer_id: i64,
        commit: &str,
        expected_revision: Option<i64>,
        notification: MergeNotification<'_>,
    ) -> Result<(Ticket, Message, Vec<i64>), AppError> {
        let (ticket, message) = self
            .complete_review_with_message(
                id,
                reviewer_id,
                commit,
                expected_revision,
                Some(MessageWrite {
                    channel: notification.channel,
                    agent_id: None,
                    author: notification.author,
                    body: notification.body,
                    deduplication_key: Some(notification.deduplication_key),
                }),
            )
            .await?;
        let (message, wake_agent_ids) = message.expect("notification was requested");
        Ok((ticket, message, wake_agent_ids))
    }

    async fn complete_review_with_message(
        &self,
        id: i64,
        reviewer_id: i64,
        commit: &str,
        expected_revision: Option<i64>,
        message: Option<MessageWrite<'_>>,
    ) -> Result<(Ticket, Option<(Message, Vec<i64>)>), AppError> {
        if !self.has_agent_scope(reviewer_id, "approve_review").await? {
            return Err(AppError::forbidden("agent lacks approve_review scope"));
        }
        let mut transaction = self.pool.begin().await?;
        let ticket = sqlx::query_as(
            "UPDATE tickets SET state = 'done', revision = revision + 1, updated_at = unixepoch() \
             WHERE id = ? AND state = 'review' AND reviewer_id = ? AND review_commit = ? \
             AND (? IS NULL OR revision = ?) \
             RETURNING id, title, description, state, assignee_id, implementation_branch, review_commit, review_target_branch, reviewer_id, review_feedback, revision, created_at, updated_at",
        )
        .bind(id)
        .bind(reviewer_id)
        .bind(commit)
        .bind(expected_revision)
        .bind(expected_revision)
        .fetch_optional(&mut *transaction)
        .await?;
        let ticket = ticket.ok_or_else(|| {
            AppError::conflict("review is not assigned to this agent or has changed")
        })?;
        let message = match message {
            Some(message) => Some(insert_message(&mut transaction, message).await?),
            None => None,
        };
        transaction.commit().await?;
        Ok((ticket, message))
    }
}

async fn ticket_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    id: i64,
) -> Result<Ticket, AppError> {
    Ok(sqlx::query_as(
        "SELECT id, title, description, state, assignee_id, implementation_branch, review_commit, \
         review_target_branch, reviewer_id, review_feedback, revision, created_at, updated_at \
         FROM tickets WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&mut **transaction)
    .await?)
}

fn ticket_is_available(ticket: &Ticket) -> bool {
    ticket.state == TicketState::Todo && ticket.assignee_id.is_none()
}

fn ticket_changed_fields(previous: Option<&Ticket>, ticket: &Ticket) -> Vec<&'static str> {
    let Some(previous) = previous else {
        return vec!["title", "description", "state", "assignee_id"];
    };
    let mut fields = Vec::new();
    if previous.title != ticket.title {
        fields.push("title");
    }
    if previous.description != ticket.description {
        fields.push("description");
    }
    if previous.state != ticket.state {
        fields.push("state");
    }
    if previous.assignee_id != ticket.assignee_id {
        fields.push("assignee_id");
    }
    if previous.implementation_branch != ticket.implementation_branch {
        fields.push("implementation_branch");
    }
    if previous.review_commit != ticket.review_commit {
        fields.push("review_commit");
    }
    if previous.review_target_branch != ticket.review_target_branch {
        fields.push("review_target_branch");
    }
    if previous.reviewer_id != ticket.reviewer_id {
        fields.push("reviewer_id");
    }
    if previous.review_feedback != ticket.review_feedback {
        fields.push("review_feedback");
    }
    fields
}

async fn insert_pending_ticket_wakes(
    transaction: &mut Transaction<'_, Sqlite>,
    previous: Option<&Ticket>,
    ticket: &Ticket,
    reason: TicketWakeReason,
) -> Result<Vec<i64>, AppError> {
    let (kind, message, agent_ids) = match reason {
        TicketWakeReason::Available => (
            "ticket.available",
            format!(
                "Ticket #{} '{}' is available. Claim it atomically before starting work.",
                ticket.id, ticket.title
            ),
            sqlx::query_scalar("SELECT id FROM agents WHERE wake_on_pending_work = 1 ORDER BY id")
                .fetch_all(&mut **transaction)
                .await?,
        ),
        TicketWakeReason::Assigned => (
            "ticket.assigned",
            format!(
                "Ticket #{} '{}' was assigned to you.",
                ticket.id, ticket.title
            ),
            sqlx::query_scalar("SELECT id FROM agents WHERE id = ? AND wake_on_pending_work = 1")
                .bind(ticket.assignee_id)
                .fetch_all(&mut **transaction)
                .await?,
        ),
        TicketWakeReason::ReviewRequested => (
            "ticket.review_requested",
            format!(
                "Ticket #{} '{}' from branch '{}' is ready for review. Claim its review before inspecting it.",
                ticket.id,
                ticket.title,
                ticket.implementation_branch.as_deref().unwrap_or("unknown")
            ),
            sqlx::query_scalar(
                "SELECT a.id FROM agents a JOIN agent_scopes s ON s.agent_id = a.id \
                 WHERE a.wake_on_pending_review = 1 AND s.scope = 'approve_review' \
                 AND a.id != ? ORDER BY a.id",
            )
            .bind(ticket.assignee_id)
            .fetch_all(&mut **transaction)
            .await?,
        ),
    };
    let payload = serde_json::to_string(&serde_json::json!({
        "message": message,
        "ticket_id": ticket.id,
        "revision": ticket.revision,
        "changed_fields": ticket_changed_fields(previous, ticket),
        "ticket": {
            "title": ticket.title,
            "state": ticket.state,
            "assignee_id": ticket.assignee_id,
            "implementation_branch": ticket.implementation_branch,
            "review_commit": ticket.review_commit,
            "review_target_branch": ticket.review_target_branch,
            "reviewer_id": ticket.reviewer_id,
        }
    }))
    .map_err(|error| AppError::internal(format!("failed to encode ticket wake event: {error}")))?;
    let deduplication_key = format!("ticket:{}:{}:{kind}", ticket.id, ticket.revision);
    let mut queued = Vec::new();
    for agent_id in agent_ids {
        let result = sqlx::query(
            "INSERT OR IGNORE INTO agent_wake_events \
             (agent_id, kind, payload, deduplication_key) VALUES (?, ?, ?, ?)",
        )
        .bind(agent_id)
        .bind(kind)
        .bind(&payload)
        .bind(&deduplication_key)
        .execute(&mut **transaction)
        .await?;
        if result.rows_affected() == 1 {
            queued.push(agent_id);
        }
    }
    Ok(queued)
}

fn validate_title(title: &str) -> Result<(), AppError> {
    if title.trim().is_empty() || title.chars().count() > 200 {
        return Err(AppError::bad_request(
            "title must contain between 1 and 200 characters",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ticket_updates_use_revisions() {
        let store = Store::memory().await.unwrap();
        let ticket = store
            .create_ticket(NewTicket {
                title: "First".into(),
                description: String::new(),
            })
            .await
            .unwrap();
        let updated = store
            .update_ticket(
                ticket.id,
                TicketChanges {
                    state: Some(TicketState::InProgress),
                    expected_revision: Some(ticket.revision),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(updated.state, TicketState::InProgress);
        assert_eq!(updated.revision, 2);
        assert!(
            store
                .update_ticket(
                    ticket.id,
                    TicketChanges {
                        expected_revision: Some(1),
                        ..Default::default()
                    }
                )
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn separate_work_and_review_wake_settings_select_independent_targets() {
        let store = Store::memory().await.unwrap();
        let implementer = store
            .create_agent("implementer", "/tmp/implementer")
            .await
            .unwrap();
        let work_only = store
            .create_agent("work-only", "/tmp/work-only")
            .await
            .unwrap();
        let review_only = store
            .create_agent("review-only", "/tmp/review-only")
            .await
            .unwrap();
        let both = store.create_agent("both", "/tmp/both").await.unwrap();
        let neither = store.create_agent("neither", "/tmp/neither").await.unwrap();
        store
            .set_agent_wake_settings(work_only.id, true, false, None)
            .await
            .unwrap();
        store
            .set_agent_wake_settings(review_only.id, false, true, None)
            .await
            .unwrap();
        store
            .set_agent_wake_settings(both.id, true, true, None)
            .await
            .unwrap();
        for agent_id in [work_only.id, review_only.id, both.id, neither.id] {
            store
                .set_agent_scopes(agent_id, &["approve_review".into()])
                .await
                .unwrap();
        }

        let (ticket, mut available_targets) = store
            .create_ticket_with_wakes(NewTicket {
                title: "Build it".into(),
                description: "Implement the change".into(),
            })
            .await
            .unwrap();
        available_targets.sort();
        assert_eq!(available_targets, vec![work_only.id, both.id]);
        assert!(store.wake_events(review_only.id).await.unwrap().is_empty());
        assert!(store.wake_events(neither.id).await.unwrap().is_empty());

        store
            .set_agent_wake_settings(implementer.id, true, false, None)
            .await
            .unwrap();
        let (ticket, assigned_targets) = store
            .claim_ticket_with_wakes(ticket.id, implementer.id, Some(ticket.revision))
            .await
            .unwrap();
        assert_eq!(assigned_targets, vec![implementer.id]);
        assert_eq!(
            store.wake_events(implementer.id).await.unwrap()[0].kind,
            "ticket.assigned"
        );

        let (ticket, review_targets) = store
            .submit_review_with_wakes(
                ticket.id,
                implementer.id,
                Some("agents/implementer"),
                "abc123",
                "main",
                Some(ticket.revision),
            )
            .await
            .unwrap();
        assert_eq!(review_targets, vec![review_only.id, both.id]);
        assert_eq!(store.wake_events(work_only.id).await.unwrap().len(), 1);
        let reviewer_events = store.wake_events(review_only.id).await.unwrap();
        assert_eq!(reviewer_events[0].kind, "ticket.review_requested");
        assert_eq!(reviewer_events[0].payload["ticket_id"], ticket.id);
        assert_eq!(
            reviewer_events[0].deduplication_key.as_deref(),
            Some("ticket:1:3:ticket.review_requested")
        );
        assert_eq!(
            store
                .wake_events(both.id)
                .await
                .unwrap()
                .iter()
                .map(|event| event.kind.as_str())
                .collect::<Vec<_>>(),
            vec!["ticket.available", "ticket.review_requested"]
        );
        assert!(store.wake_events(neither.id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn review_transitions_require_scope_and_ownership() {
        let store = Store::memory().await.unwrap();
        let worker = store.create_agent("worker", "/tmp/worker").await.unwrap();
        let reviewer = store
            .create_agent("reviewer", "/tmp/reviewer")
            .await
            .unwrap();
        let outsider = store
            .create_agent("outsider", "/tmp/outsider")
            .await
            .unwrap();
        let ticket = store
            .create_ticket(NewTicket {
                title: "Review me".into(),
                description: String::new(),
            })
            .await
            .unwrap();
        let ticket = store
            .claim_ticket(ticket.id, worker.id, Some(ticket.revision))
            .await
            .unwrap();
        let ticket = store
            .submit_review(
                ticket.id,
                worker.id,
                Some("agents/worker"),
                "abc123",
                "main",
                Some(ticket.revision),
            )
            .await
            .unwrap();

        assert_eq!(
            ticket.implementation_branch.as_deref(),
            Some("agents/worker")
        );

        assert!(
            store
                .claim_review(ticket.id, outsider.id, Some(ticket.revision))
                .await
                .is_err()
        );
        assert_eq!(
            store
                .set_agent_scopes(
                    reviewer.id,
                    &["approve_review".into(), "approve_review".into()]
                )
                .await
                .unwrap(),
            vec!["approve_review"]
        );
        let ticket = store
            .claim_review(ticket.id, reviewer.id, Some(ticket.revision))
            .await
            .unwrap();
        let ticket = store
            .request_changes(
                ticket.id,
                reviewer.id,
                "Please add a test",
                Some(ticket.revision),
            )
            .await
            .unwrap();

        assert_eq!(ticket.state, TicketState::InProgress);
        assert_eq!(ticket.assignee_id, Some(worker.id));
        assert_eq!(ticket.review_feedback.as_deref(), Some("Please add a test"));
    }
}

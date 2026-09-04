use std::{path::Path, time::Duration};

use sqlx::{
    Sqlite, SqlitePool, Transaction,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};

use crate::{
    domain::{
        Agent, AgentMessage, AgentState, Channel, Message, Project, Ticket, TicketState, WakeEvent,
        WakeEventState,
    },
    error::AppError,
};

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

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

#[derive(sqlx::FromRow)]
struct WakeEventRow {
    id: i64,
    agent_id: i64,
    kind: String,
    payload: String,
    deduplication_key: Option<String>,
    delivery_state: WakeEventState,
    delivery_attempts: i64,
    last_attempt_at: Option<i64>,
    delivered_at: Option<i64>,
    created_at: i64,
    updated_at: i64,
}

struct MessageWrite<'a> {
    channel: &'a str,
    agent_id: Option<i64>,
    author: &'a str,
    body: &'a str,
    deduplication_key: Option<&'a str>,
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
    pub async fn open(path: &Path) -> Result<Self, sqlx::Error> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    pub async fn memory() -> Result<Self, sqlx::Error> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    pub async fn close(self) {
        self.pool.close().await;
    }

    #[cfg(test)]
    pub(crate) async fn execute_test_sql(&self, statement: &str) -> Result<(), sqlx::Error> {
        sqlx::query(statement).execute(&self.pool).await?;
        Ok(())
    }

    async fn migrate(&self) -> Result<(), sqlx::Error> {
        sqlx::migrate!().run(&self.pool).await.map_err(Into::into)
    }

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

    pub async fn agent_scopes(&self, agent_id: i64) -> Result<Vec<String>, AppError> {
        if self.agent(agent_id).await?.is_none() {
            return Err(AppError::not_found("agent not found"));
        }
        Ok(
            sqlx::query_scalar("SELECT scope FROM agent_scopes WHERE agent_id = ? ORDER BY scope")
                .bind(agent_id)
                .fetch_all(&self.pool)
                .await?,
        )
    }

    pub async fn set_agent_scopes(
        &self,
        agent_id: i64,
        scopes: &[String],
    ) -> Result<Vec<String>, AppError> {
        if self.agent(agent_id).await?.is_none() {
            return Err(AppError::not_found("agent not found"));
        }
        for scope in scopes {
            validate_scope(scope)?;
        }
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM agent_scopes WHERE agent_id = ?")
            .bind(agent_id)
            .execute(&mut *transaction)
            .await?;
        for scope in scopes {
            sqlx::query("INSERT OR IGNORE INTO agent_scopes (agent_id, scope) VALUES (?, ?)")
                .bind(agent_id)
                .bind(scope)
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        self.agent_scopes(agent_id).await
    }

    pub async fn has_agent_scope(&self, agent_id: i64, scope: &str) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM agent_scopes WHERE agent_id = ? AND scope = ?)",
        )
        .bind(agent_id)
        .bind(scope)
        .fetch_one(&self.pool)
        .await
    }

    pub async fn agents_with_scope(
        &self,
        scope: &str,
        exclude_agent_id: i64,
    ) -> Result<Vec<Agent>, sqlx::Error> {
        sqlx::query_as(
            "SELECT a.id, a.name, a.state, a.thread_id, a.worktree_path, a.active_turn_id, \
             a.last_error, a.wake_on_pending_work, a.wake_on_pending_review, a.idle_heartbeat_minutes, \
             a.idle_since, a.created_at, a.updated_at FROM agents a \
             JOIN agent_scopes s ON s.agent_id = a.id \
             WHERE s.scope = ? AND a.id != ? AND a.state != 'paused' \
             ORDER BY CASE a.state WHEN 'idle' THEN 0 WHEN 'error' THEN 1 ELSE 2 END, a.id",
        )
        .bind(scope)
        .bind(exclude_agent_id)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn enqueue_wake_event(
        &self,
        agent_id: i64,
        kind: &str,
        payload: serde_json::Value,
        deduplication_key: Option<&str>,
    ) -> Result<WakeEvent, AppError> {
        if self.agent(agent_id).await?.is_none() {
            return Err(AppError::not_found("agent not found"));
        }
        validate_wake_event(kind, deduplication_key)?;
        let payload = serde_json::to_string(&payload).map_err(|error| {
            AppError::bad_request(format!("invalid wake event payload: {error}"))
        })?;
        if payload.len() > 100_000 {
            return Err(AppError::bad_request("wake event payload is too large"));
        }
        let result = sqlx::query(
            "INSERT OR IGNORE INTO agent_wake_events \
             (agent_id, kind, payload, deduplication_key) VALUES (?, ?, ?, ?)",
        )
        .bind(agent_id)
        .bind(kind)
        .bind(payload)
        .bind(deduplication_key)
        .execute(&self.pool)
        .await?;
        let id = if result.rows_affected() == 1 {
            result.last_insert_rowid()
        } else {
            sqlx::query_scalar(
                "SELECT id FROM agent_wake_events \
                 WHERE agent_id = ? AND deduplication_key = ?",
            )
            .bind(agent_id)
            .bind(deduplication_key)
            .fetch_one(&self.pool)
            .await?
        };
        self.wake_event(id)
            .await?
            .ok_or_else(|| AppError::internal("wake event disappeared after insertion"))
    }

    pub async fn wake_events(&self, agent_id: i64) -> Result<Vec<WakeEvent>, AppError> {
        if self.agent(agent_id).await?.is_none() {
            return Err(AppError::not_found("agent not found"));
        }
        let rows: Vec<WakeEventRow> = sqlx::query_as(
            "SELECT id, agent_id, kind, payload, deduplication_key, delivery_state, \
             delivery_attempts, last_attempt_at, delivered_at, created_at, updated_at \
             FROM agent_wake_events WHERE agent_id = ? ORDER BY id",
        )
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(wake_event_from_row).collect()
    }

    pub async fn claim_pending_wake_event(
        &self,
        agent_id: i64,
    ) -> Result<Option<WakeEvent>, AppError> {
        let row: Option<WakeEventRow> = sqlx::query_as(
            "UPDATE agent_wake_events SET delivery_state = 'delivering', \
             delivery_attempts = delivery_attempts + 1, last_attempt_at = unixepoch(), \
             updated_at = unixepoch() WHERE id = (SELECT id FROM agent_wake_events \
             WHERE agent_id = ? AND delivery_state = 'pending' ORDER BY id LIMIT 1) \
             RETURNING id, agent_id, kind, payload, deduplication_key, delivery_state, \
             delivery_attempts, last_attempt_at, delivered_at, created_at, updated_at",
        )
        .bind(agent_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(wake_event_from_row).transpose()
    }

    pub async fn complete_wake_event(&self, id: i64) -> Result<(), AppError> {
        let result = sqlx::query(
            "UPDATE agent_wake_events SET delivery_state = 'delivered', \
             delivered_at = unixepoch(), updated_at = unixepoch() \
             WHERE id = ? AND delivery_state = 'delivering'",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::conflict("wake event is not being delivered"));
        }
        Ok(())
    }

    pub async fn retry_wake_event(&self, id: i64) -> Result<(), AppError> {
        let result = sqlx::query(
            "UPDATE agent_wake_events SET delivery_state = 'pending', updated_at = unixepoch() \
             WHERE id = ? AND delivery_state = 'delivering'",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::conflict("wake event is not being delivered"));
        }
        Ok(())
    }

    pub async fn recover_wake_events_after_restart(&self) -> Result<u64, sqlx::Error> {
        Ok(sqlx::query(
            "UPDATE agent_wake_events SET delivery_state = 'pending', updated_at = unixepoch() \
             WHERE delivery_state = 'delivering'",
        )
        .execute(&self.pool)
        .await?
        .rows_affected())
    }

    async fn wake_event(&self, id: i64) -> Result<Option<WakeEvent>, AppError> {
        let row: Option<WakeEventRow> = sqlx::query_as(
            "SELECT id, agent_id, kind, payload, deduplication_key, delivery_state, \
             delivery_attempts, last_attempt_at, delivered_at, created_at, updated_at \
             FROM agent_wake_events WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(wake_event_from_row).transpose()
    }

    pub async fn enqueue_notification(&self, agent_id: i64, body: &str) -> Result<(), AppError> {
        if body.trim().is_empty() || body.chars().count() > 50_000 {
            return Err(AppError::bad_request("invalid agent notification"));
        }
        self.enqueue_wake_event(
            agent_id,
            "notification",
            serde_json::json!({"message": body.trim()}),
            None,
        )
        .await?;
        Ok(())
    }

    pub async fn pending_notification(
        &self,
        agent_id: i64,
    ) -> Result<Option<(i64, String)>, AppError> {
        let row: Option<WakeEventRow> = sqlx::query_as(
            "UPDATE agent_wake_events SET delivery_state = 'delivering', \
             delivery_attempts = delivery_attempts + 1, last_attempt_at = unixepoch(), \
             updated_at = unixepoch() WHERE id = (SELECT id FROM agent_wake_events \
             WHERE agent_id = ? AND kind = 'notification' AND delivery_state = 'pending' \
             AND json_type(payload, '$.message') = 'text' ORDER BY id LIMIT 1) \
             RETURNING id, agent_id, kind, payload, deduplication_key, delivery_state, \
             delivery_attempts, last_attempt_at, delivered_at, created_at, updated_at",
        )
        .bind(agent_id)
        .fetch_optional(&self.pool)
        .await?;
        let event = row.map(wake_event_from_row).transpose()?;
        event
            .map(|event| {
                let message = event
                    .payload
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| AppError::internal("notification payload has no message"))?
                    .to_owned();
                Ok((event.id, message))
            })
            .transpose()
    }

    pub async fn delete_notification(&self, id: i64) -> Result<(), AppError> {
        self.complete_wake_event(id).await
    }

    pub async fn channels(&self) -> Result<Vec<Channel>, sqlx::Error> {
        sqlx::query_as(
            "SELECT c.name, MAX(m.created_at) AS most_recent_message_at FROM channels c \
             LEFT JOIN messages m ON m.channel_name = c.name GROUP BY c.name ORDER BY c.name",
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn messages(
        &self,
        channel: &str,
        start: Option<i64>,
        end: Option<i64>,
    ) -> Result<Vec<Message>, AppError> {
        validate_channel(channel)?;
        if !self.channel_exists(channel).await? {
            return Err(AppError::not_found("channel not found"));
        }
        Ok(sqlx::query_as(
            "SELECT id, channel_name, agent_id, author, body, created_at FROM messages \
             WHERE channel_name = ? AND (? IS NULL OR created_at >= ?) \
             AND (? IS NULL OR created_at <= ?) ORDER BY created_at, id",
        )
        .bind(channel)
        .bind(start)
        .bind(start)
        .bind(end)
        .bind(end)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn post_message(
        &self,
        channel: &str,
        agent_id: Option<i64>,
        author: &str,
        body: &str,
    ) -> Result<Message, AppError> {
        Ok(self
            .post_message_with_key(channel, agent_id, author, body, None)
            .await?
            .0)
    }

    pub async fn post_message_once(
        &self,
        channel: &str,
        agent_id: Option<i64>,
        author: &str,
        body: &str,
        deduplication_key: &str,
    ) -> Result<Message, AppError> {
        if deduplication_key.is_empty() || deduplication_key.len() > 500 {
            return Err(AppError::bad_request(
                "message deduplication key must contain between 1 and 500 characters",
            ));
        }
        Ok(self
            .post_message_with_key(channel, agent_id, author, body, Some(deduplication_key))
            .await?
            .0)
    }

    pub async fn post_message_with_wakes(
        &self,
        channel: &str,
        agent_id: Option<i64>,
        author: &str,
        body: &str,
    ) -> Result<(Message, Vec<i64>), AppError> {
        self.post_message_with_key(channel, agent_id, author, body, None)
            .await
    }

    async fn post_message_with_key(
        &self,
        channel: &str,
        agent_id: Option<i64>,
        author: &str,
        body: &str,
        deduplication_key: Option<&str>,
    ) -> Result<(Message, Vec<i64>), AppError> {
        let mut transaction = self.pool.begin().await?;
        let message = insert_message(
            &mut transaction,
            MessageWrite {
                channel,
                agent_id,
                author,
                body,
                deduplication_key,
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(message)
    }

    async fn channel_exists(&self, channel: &str) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM channels WHERE name = ?)")
            .bind(channel)
            .fetch_one(&self.pool)
            .await
    }

    pub async fn agent_channel_subscriptions(
        &self,
        agent_id: i64,
    ) -> Result<Vec<String>, AppError> {
        if self.agent(agent_id).await?.is_none() {
            return Err(AppError::not_found("agent not found"));
        }
        Ok(sqlx::query_scalar(
            "SELECT channel_name FROM agent_channel_subscriptions \
             WHERE agent_id = ? ORDER BY channel_name",
        )
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn set_agent_channel_subscriptions(
        &self,
        agent_id: i64,
        channels: &[String],
    ) -> Result<Vec<String>, AppError> {
        if self.agent(agent_id).await?.is_none() {
            return Err(AppError::not_found("agent not found"));
        }
        let mut channels = channels.to_vec();
        channels.sort();
        channels.dedup();
        for channel in &channels {
            validate_channel(channel)?;
            if !self.channel_exists(channel).await? {
                return Err(AppError::bad_request(format!(
                    "channel #{channel} does not exist"
                )));
            }
        }
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM agent_channel_subscriptions WHERE agent_id = ?")
            .bind(agent_id)
            .execute(&mut *transaction)
            .await?;
        for channel in &channels {
            sqlx::query(
                "INSERT INTO agent_channel_subscriptions (agent_id, channel_name) VALUES (?, ?)",
            )
            .bind(agent_id)
            .bind(channel)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(channels)
    }

    pub async fn agents(&self) -> Result<Vec<Agent>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id, name, state, thread_id, worktree_path, active_turn_id, last_error, \
             wake_on_pending_work, wake_on_pending_review, idle_heartbeat_minutes, idle_since, created_at, updated_at \
             FROM agents ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn agents_due_for_idle_heartbeat(&self, now: i64) -> Result<Vec<Agent>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id, name, state, thread_id, worktree_path, active_turn_id, last_error, \
             wake_on_pending_work, wake_on_pending_review, idle_heartbeat_minutes, idle_since, created_at, updated_at \
             FROM agents WHERE state = 'idle' AND idle_heartbeat_minutes IS NOT NULL \
             AND idle_since IS NOT NULL AND idle_since + idle_heartbeat_minutes * 60 <= ? \
             ORDER BY id",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn agent(&self, id: i64) -> Result<Option<Agent>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id, name, state, thread_id, worktree_path, active_turn_id, last_error, \
             wake_on_pending_work, wake_on_pending_review, idle_heartbeat_minutes, idle_since, created_at, updated_at \
             FROM agents WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn agent_named(&self, name: &str) -> Result<Option<Agent>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id, name, state, thread_id, worktree_path, active_turn_id, last_error, \
             wake_on_pending_work, wake_on_pending_review, idle_heartbeat_minutes, idle_since, created_at, updated_at \
             FROM agents WHERE name = ?",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn create_agent(&self, name: &str, worktree: &str) -> Result<Agent, AppError> {
        let result = sqlx::query(
            "INSERT INTO agents (name, worktree_path, idle_since) VALUES (?, ?, unixepoch())",
        )
        .bind(name)
        .bind(worktree)
        .execute(&self.pool)
        .await?;
        Ok(self.agent(result.last_insert_rowid()).await?.unwrap())
    }

    pub async fn rename_agent(&self, id: i64, name: &str) -> Result<Agent, AppError> {
        let result =
            sqlx::query("UPDATE agents SET name = ?, updated_at = unixepoch() WHERE id = ?")
                .bind(name)
                .bind(id)
                .execute(&self.pool)
                .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("agent not found"));
        }
        Ok(self.agent(id).await?.unwrap())
    }

    pub async fn set_agent_wake_settings(
        &self,
        id: i64,
        wake_on_pending_work: bool,
        wake_on_pending_review: bool,
        idle_heartbeat_minutes: Option<i64>,
    ) -> Result<Agent, AppError> {
        if idle_heartbeat_minutes.is_some_and(|minutes| !(1..=10_080).contains(&minutes)) {
            return Err(AppError::bad_request(
                "idle heartbeat must be between 1 and 10080 minutes",
            ));
        }
        let result = sqlx::query(
            "UPDATE agents SET wake_on_pending_work = ?, wake_on_pending_review = ?, idle_heartbeat_minutes = ?, \
             updated_at = unixepoch() WHERE id = ?",
        )
        .bind(wake_on_pending_work)
        .bind(wake_on_pending_review)
        .bind(idle_heartbeat_minutes)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("agent not found"));
        }
        Ok(self.agent(id).await?.unwrap())
    }

    pub async fn update_agent_runtime(
        &self,
        id: i64,
        state: AgentState,
        thread_id: Option<&str>,
        active_turn_id: Option<&str>,
        last_error: Option<&str>,
    ) -> Result<Agent, AppError> {
        let result = sqlx::query(
            "UPDATE agents SET state = ?, thread_id = COALESCE(?, thread_id), \
             active_turn_id = ?, last_error = ?, \
             idle_since = CASE WHEN ? = 'idle' THEN \
                 CASE WHEN state = 'idle' THEN idle_since ELSE unixepoch() END \
                 ELSE NULL END, updated_at = unixepoch() WHERE id = ?",
        )
        .bind(state)
        .bind(thread_id)
        .bind(active_turn_id)
        .bind(last_error)
        .bind(state)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::not_found("agent not found"));
        }
        Ok(self.agent(id).await?.unwrap())
    }

    pub async fn finish_agent_turn(
        &self,
        id: i64,
        turn_id: &str,
        state: AgentState,
        last_error: Option<&str>,
    ) -> Result<Option<Agent>, AppError> {
        let result = sqlx::query(
            "UPDATE agents SET state = ?, active_turn_id = NULL, last_error = ?, \
             idle_since = CASE WHEN ? = 'idle' THEN unixepoch() ELSE NULL END, \
             updated_at = unixepoch() WHERE id = ? AND active_turn_id = ?",
        )
        .bind(state)
        .bind(last_error)
        .bind(state)
        .bind(id)
        .bind(turn_id)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Ok(None);
        }
        Ok(self.agent(id).await?)
    }

    pub async fn agent_messages(&self, agent_id: i64) -> Result<Vec<AgentMessage>, AppError> {
        if self.agent(agent_id).await?.is_none() {
            return Err(AppError::not_found("agent not found"));
        }
        Ok(sqlx::query_as(
            "SELECT id, agent_id, role, body, created_at FROM agent_messages \
             WHERE agent_id = ? ORDER BY created_at, id",
        )
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn add_agent_message(
        &self,
        agent_id: i64,
        role: &str,
        body: &str,
    ) -> Result<AgentMessage, AppError> {
        if body.trim().is_empty() || body.chars().count() > 50_000 {
            return Err(AppError::bad_request(
                "agent message must contain between 1 and 50000 characters",
            ));
        }
        let result =
            sqlx::query("INSERT INTO agent_messages (agent_id, role, body) VALUES (?, ?, ?)")
                .bind(agent_id)
                .bind(role)
                .bind(body.trim())
                .execute(&self.pool)
                .await?;
        Ok(sqlx::query_as(
            "SELECT id, agent_id, role, body, created_at FROM agent_messages WHERE id = ?",
        )
        .bind(result.last_insert_rowid())
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn enqueue_agent_message(
        &self,
        agent_id: i64,
        body: &str,
    ) -> Result<AgentMessage, AppError> {
        if body.trim().is_empty() || body.chars().count() > 50_000 {
            return Err(AppError::bad_request(
                "agent message must contain between 1 and 50000 characters",
            ));
        }
        let mut transaction = self.pool.begin().await?;
        let result =
            sqlx::query("INSERT INTO agent_messages (agent_id, role, body) VALUES (?, 'user', ?)")
                .bind(agent_id)
                .bind(body.trim())
                .execute(&mut *transaction)
                .await?;
        let message: AgentMessage = sqlx::query_as(
            "SELECT id, agent_id, role, body, created_at FROM agent_messages WHERE id = ?",
        )
        .bind(result.last_insert_rowid())
        .fetch_one(&mut *transaction)
        .await?;
        let payload = serde_json::json!({"message": &message.body, "message_id": message.id});
        sqlx::query(
            "INSERT INTO agent_wake_events (agent_id, kind, payload) VALUES (?, 'message', ?)",
        )
        .bind(agent_id)
        .bind(payload.to_string())
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(message)
    }

    pub async fn recover_agents_after_restart(&self) -> Result<u64, sqlx::Error> {
        Ok(sqlx::query(
            "UPDATE agents SET state = 'error', active_turn_id = NULL, \
             last_error = 'The board restarted during an active turn; send a message to retry.', \
             updated_at = unixepoch() WHERE state IN ('running', 'awaiting_approval')",
        )
        .execute(&self.pool)
        .await?
        .rows_affected())
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

async fn insert_message(
    transaction: &mut Transaction<'_, Sqlite>,
    message: MessageWrite<'_>,
) -> Result<(Message, Vec<i64>), AppError> {
    validate_channel(message.channel)?;
    validate_message(message.author, message.body)?;
    if message
        .deduplication_key
        .is_some_and(|key| key.is_empty() || key.len() > 500)
    {
        return Err(AppError::bad_request(
            "message deduplication key must contain between 1 and 500 characters",
        ));
    }
    sqlx::query("INSERT OR IGNORE INTO channels (name) VALUES (?)")
        .bind(message.channel)
        .execute(&mut **transaction)
        .await?;
    let result = sqlx::query(
        "INSERT OR IGNORE INTO messages \
         (channel_name, agent_id, author, body, deduplication_key) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(message.channel)
    .bind(message.agent_id)
    .bind(message.author.trim())
    .bind(message.body.trim())
    .bind(message.deduplication_key)
    .execute(&mut **transaction)
    .await?;
    let id = if result.rows_affected() == 1 {
        result.last_insert_rowid()
    } else {
        sqlx::query_scalar("SELECT id FROM messages WHERE deduplication_key = ?")
            .bind(message.deduplication_key)
            .fetch_one(&mut **transaction)
            .await?
    };
    let message: Message = sqlx::query_as(
        "SELECT id, channel_name, agent_id, author, body, created_at FROM messages WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&mut **transaction)
    .await?;
    let agents: Vec<(i64, String, bool)> = sqlx::query_as(
        "SELECT a.id, a.name, EXISTS(SELECT 1 FROM agent_channel_subscriptions s \
         WHERE s.agent_id = a.id AND s.channel_name = ?) AS subscribed FROM agents a",
    )
    .bind(&message.channel_name)
    .fetch_all(&mut **transaction)
    .await?;
    let mut wake_agent_ids = Vec::new();
    for (target_id, name, subscribed) in agents {
        if message.agent_id == Some(target_id) {
            continue;
        }
        let mentioned = mentions_agent(&message.body, &name);
        if !subscribed && !mentioned {
            continue;
        }
        let prompt = format!(
            "A message was posted in #{} by {}:\n{}",
            message.channel_name, message.author, message.body
        );
        let payload = serde_json::to_string(&serde_json::json!({
            "message": prompt,
            "channel": message.channel_name,
            "message_id": message.id,
            "author": message.author,
            "body": message.body,
            "mentioned": mentioned,
            "subscribed": subscribed,
        }))
        .map_err(|error| AppError::internal(format!("failed to encode wake event: {error}")))?;
        sqlx::query(
            "INSERT OR IGNORE INTO agent_wake_events \
             (agent_id, kind, payload, deduplication_key) \
             VALUES (?, 'channel.message', ?, ?)",
        )
        .bind(target_id)
        .bind(payload)
        .bind(format!("channel-message:{}", message.id))
        .execute(&mut **transaction)
        .await?;
        wake_agent_ids.push(target_id);
    }
    Ok((message, wake_agent_ids))
}

fn validate_title(title: &str) -> Result<(), AppError> {
    if title.trim().is_empty() || title.chars().count() > 200 {
        return Err(AppError::bad_request(
            "title must contain between 1 and 200 characters",
        ));
    }
    Ok(())
}

pub(crate) fn validate_channel(channel: &str) -> Result<(), AppError> {
    if channel.is_empty()
        || channel.len() > 64
        || !channel
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    {
        return Err(AppError::bad_request(
            "channel names may contain 1-64 letters, numbers, '-', '_', or '.'",
        ));
    }
    Ok(())
}

fn validate_message(author: &str, body: &str) -> Result<(), AppError> {
    if author.trim().is_empty() || author.chars().count() > 100 {
        return Err(AppError::bad_request(
            "author must contain between 1 and 100 characters",
        ));
    }
    if body.trim().is_empty() || body.chars().count() > 10_000 {
        return Err(AppError::bad_request(
            "message must contain between 1 and 10000 characters",
        ));
    }
    Ok(())
}

fn mentions_agent(body: &str, name: &str) -> bool {
    let mention = format!("@{name}");
    body.split(|character: char| {
        !(character.is_ascii_alphanumeric() || matches!(character, '@' | '-' | '_'))
    })
    .any(|token| token == mention)
}

fn validate_scope(scope: &str) -> Result<(), AppError> {
    if scope.is_empty()
        || scope.len() > 100
        || !scope.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':' | b'/')
        })
    {
        return Err(AppError::bad_request("invalid agent scope"));
    }
    Ok(())
}

fn validate_wake_event(kind: &str, deduplication_key: Option<&str>) -> Result<(), AppError> {
    if kind.is_empty()
        || kind.len() > 100
        || !kind
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
    {
        return Err(AppError::bad_request("invalid wake event kind"));
    }
    if deduplication_key.is_some_and(|key| key.is_empty() || key.len() > 500) {
        return Err(AppError::bad_request(
            "invalid wake event deduplication key",
        ));
    }
    Ok(())
}

fn wake_event_from_row(row: WakeEventRow) -> Result<WakeEvent, AppError> {
    Ok(WakeEvent {
        id: row.id,
        agent_id: row.agent_id,
        kind: row.kind,
        payload: serde_json::from_str(&row.payload)
            .map_err(|error| AppError::internal(format!("invalid stored wake event: {error}")))?,
        deduplication_key: row.deduplication_key,
        delivery_state: row.delivery_state,
        delivery_attempts: row.delivery_attempts,
        last_attempt_at: row.last_attempt_at,
        delivered_at: row.delivered_at,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

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
    async fn separate_wake_settings_migration_preserves_existing_preferences() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE agents (id INTEGER PRIMARY KEY, wake_on_pending_task INTEGER NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO agents VALUES (1, 1), (2, 0)")
            .execute(&pool)
            .await
            .unwrap();

        sqlx::raw_sql(include_str!(
            "../migrations/0013_separate_ticket_wake_settings.sql"
        ))
        .execute(&pool)
        .await
        .unwrap();

        let settings: Vec<(bool, bool, bool)> = sqlx::query_as(
            "SELECT wake_on_pending_task, wake_on_pending_work, wake_on_pending_review \
             FROM agents ORDER BY id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(settings, vec![(true, true, true), (false, false, false)]);
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
    async fn posting_creates_a_channel() {
        let store = Store::memory().await.unwrap();
        store
            .post_message("backend", None, "Ada", "Schema is ready")
            .await
            .unwrap();

        assert_eq!(store.channels().await.unwrap().len(), 2);
        assert_eq!(
            store
                .messages("backend", None, None)
                .await
                .unwrap()
                .first()
                .unwrap()
                .body,
            "Schema is ready"
        );
    }

    #[tokio::test]
    async fn idempotent_messages_are_inserted_once() {
        let store = Store::memory().await.unwrap();

        let first = store
            .post_message_once("merges", None, "board", "Merged", "merge:1")
            .await
            .unwrap();
        let retry = store
            .post_message_once("merges", None, "board", "Merged", "merge:1")
            .await
            .unwrap();

        assert_eq!(retry.id, first.id);
        assert_eq!(store.messages("merges", None, None).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn channel_posts_wake_subscribers_and_exact_mentions() {
        let store = Store::memory().await.unwrap();
        let subscriber = store
            .create_agent("subscriber", "/tmp/subscriber")
            .await
            .unwrap();
        let mentioned = store
            .create_agent("mentioned", "/tmp/mentioned")
            .await
            .unwrap();
        let partial = store.create_agent("mention", "/tmp/mention").await.unwrap();
        store
            .set_agent_channel_subscriptions(subscriber.id, &["general".into()])
            .await
            .unwrap();

        let (message, mut targets) = store
            .post_message_with_wakes("general", None, "human", "Please check this, @mentioned.")
            .await
            .unwrap();
        targets.sort();

        assert_eq!(targets, vec![subscriber.id, mentioned.id]);
        assert!(store.wake_events(partial.id).await.unwrap().is_empty());
        let subscriber_events = store.wake_events(subscriber.id).await.unwrap();
        let subscriber_event = &subscriber_events[0];
        assert_eq!(subscriber_event.kind, "channel.message");
        assert_eq!(subscriber_event.payload["channel"], "general");
        assert_eq!(subscriber_event.payload["message_id"], message.id);
        assert_eq!(subscriber_event.payload["subscribed"], true);
        let mention_events = store.wake_events(mentioned.id).await.unwrap();
        let mention_event = &mention_events[0];
        assert_eq!(mention_event.payload["mentioned"], true);
    }

    #[tokio::test]
    async fn agent_can_be_renamed() {
        let store = Store::memory().await.unwrap();
        let agent = store.create_agent("human", "/tmp/worker").await.unwrap();
        store
            .post_message("general", None, "human", "Unrelated")
            .await
            .unwrap();
        store
            .post_message("general", Some(agent.id), "human", "Finished")
            .await
            .unwrap();

        let renamed = store.rename_agent(agent.id, "builder").await.unwrap();

        assert_eq!(renamed.name, "builder");
        assert_eq!(renamed.worktree_path, "/tmp/worker");
        let messages = store.messages("general", None, None).await.unwrap();
        assert_eq!(messages[0].author, "human");
        assert_eq!(messages[0].agent_id, None);
        assert_eq!(messages[1].author, "human");
        assert_eq!(messages[1].agent_id, Some(agent.id));
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

    #[tokio::test]
    async fn reviewer_notifications_are_selected_and_queued_in_order() {
        let store = Store::memory().await.unwrap();
        let worker = store.create_agent("worker", "/tmp/worker").await.unwrap();
        let reviewer = store
            .create_agent("reviewer", "/tmp/reviewer")
            .await
            .unwrap();
        store
            .set_agent_scopes(reviewer.id, &["approve_review".into()])
            .await
            .unwrap();

        assert_eq!(
            store
                .agents_with_scope("approve_review", worker.id)
                .await
                .unwrap()[0]
                .id,
            reviewer.id
        );
        store
            .enqueue_notification(reviewer.id, "first")
            .await
            .unwrap();
        store
            .enqueue_notification(reviewer.id, "second")
            .await
            .unwrap();
        let first = store
            .pending_notification(reviewer.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.1, "first");
        store.delete_notification(first.0).await.unwrap();
        assert_eq!(
            store
                .pending_notification(reviewer.id)
                .await
                .unwrap()
                .unwrap()
                .1,
            "second"
        );
    }

    #[tokio::test]
    async fn notification_consumer_skips_generic_and_malformed_events() {
        let store = Store::memory().await.unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        store
            .enqueue_wake_event(
                agent.id,
                "channel.message",
                serde_json::json!({"message_id": 1}),
                None,
            )
            .await
            .unwrap();
        store
            .enqueue_wake_event(
                agent.id,
                "notification",
                serde_json::json!({"unexpected": true}),
                None,
            )
            .await
            .unwrap();
        store
            .enqueue_notification(agent.id, "review requested")
            .await
            .unwrap();

        let notification = store.pending_notification(agent.id).await.unwrap().unwrap();
        assert_eq!(notification.1, "review requested");
        let events = store.wake_events(agent.id).await.unwrap();
        assert_eq!(events[0].kind, "channel.message");
        assert_eq!(events[0].delivery_state, WakeEventState::Pending);
        assert_eq!(events[1].kind, "notification");
        assert_eq!(events[1].delivery_state, WakeEventState::Pending);
        assert_eq!(events[2].id, notification.0);
        assert_eq!(events[2].delivery_state, WakeEventState::Delivering);
    }

    #[tokio::test]
    async fn wake_events_are_deduplicated_delivered_and_recovered() {
        let store = Store::memory().await.unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        let first = store
            .enqueue_wake_event(
                agent.id,
                "ticket.available",
                serde_json::json!({"ticket_id": 3}),
                Some("ticket:3:available"),
            )
            .await
            .unwrap();
        let duplicate = store
            .enqueue_wake_event(
                agent.id,
                "ticket.available",
                serde_json::json!({"ticket_id": 3, "ignored": true}),
                Some("ticket:3:available"),
            )
            .await
            .unwrap();
        assert_eq!(duplicate.id, first.id);
        assert_eq!(duplicate.payload, first.payload);

        let delivering = store
            .claim_pending_wake_event(agent.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(delivering.delivery_state, WakeEventState::Delivering);
        assert_eq!(delivering.delivery_attempts, 1);
        assert_eq!(store.recover_wake_events_after_restart().await.unwrap(), 1);

        let retry = store
            .claim_pending_wake_event(agent.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retry.id, first.id);
        assert_eq!(retry.delivery_attempts, 2);
        store.complete_wake_event(retry.id).await.unwrap();
        let events = store.wake_events(agent.id).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].delivery_state, WakeEventState::Delivered);
        assert!(events[0].delivered_at.is_some());
        assert!(
            store
                .claim_pending_wake_event(agent.id)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn file_store_survives_reopen() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("multi-agent-board-{unique}.sqlite3"));
        let store = Store::open(&path).await.unwrap();
        store
            .create_ticket(NewTicket {
                title: "Persistent".into(),
                description: String::new(),
            })
            .await
            .unwrap();
        let agent = store
            .create_agent("persistent-agent", "/tmp/persistent-agent")
            .await
            .unwrap();
        store
            .set_agent_wake_settings(agent.id, true, false, Some(20))
            .await
            .unwrap();
        store.close().await;

        let reopened = Store::open(&path).await.unwrap();
        assert_eq!(reopened.tickets().await.unwrap().len(), 1);
        let agent = reopened.agent(agent.id).await.unwrap().unwrap();
        assert!(agent.wake_on_pending_work);
        assert!(!agent.wake_on_pending_review);
        assert_eq!(agent.idle_heartbeat_minutes, Some(20));
        reopened.close().await;
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn interrupted_agents_are_recovered_as_errors() {
        let store = Store::memory().await.unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        store
            .update_agent_runtime(
                agent.id,
                AgentState::Running,
                Some("thread-1"),
                Some("turn-1"),
                None,
            )
            .await
            .unwrap();

        assert_eq!(store.recover_agents_after_restart().await.unwrap(), 1);
        let recovered = store.agent(agent.id).await.unwrap().unwrap();
        assert_eq!(recovered.state, AgentState::Error);
        assert!(recovered.active_turn_id.is_none());
        assert!(recovered.thread_id.is_some());
    }

    #[tokio::test]
    async fn idle_heartbeat_timing_resets_on_each_idle_period() {
        let store = Store::memory().await.unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        let first_idle_since = agent.idle_since.unwrap();
        assert!(
            store
                .agents_due_for_idle_heartbeat(i64::MAX)
                .await
                .unwrap()
                .is_empty()
        );
        store
            .set_agent_wake_settings(agent.id, false, false, Some(20))
            .await
            .unwrap();

        assert!(
            store
                .agents_due_for_idle_heartbeat(first_idle_since + 1_199)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store
                .agents_due_for_idle_heartbeat(first_idle_since + 1_200)
                .await
                .unwrap()[0]
                .id,
            agent.id
        );

        store
            .update_agent_runtime(
                agent.id,
                AgentState::Running,
                Some("thread-1"),
                Some("turn-1"),
                None,
            )
            .await
            .unwrap();
        assert!(
            store
                .agent(agent.id)
                .await
                .unwrap()
                .unwrap()
                .idle_since
                .is_none()
        );
        let idle = store
            .finish_agent_turn(agent.id, "turn-1", AgentState::Idle, None)
            .await
            .unwrap()
            .unwrap();
        let next_idle_since = idle.idle_since.unwrap();
        assert!(
            store
                .agents_due_for_idle_heartbeat(next_idle_since + 1_199)
                .await
                .unwrap()
                .is_empty()
        );

        for state in [AgentState::Paused, AgentState::Running, AgentState::Error] {
            store
                .update_agent_runtime(agent.id, state, None, None, None)
                .await
                .unwrap();
            assert!(
                store
                    .agents_due_for_idle_heartbeat(i64::MAX)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
    }
}

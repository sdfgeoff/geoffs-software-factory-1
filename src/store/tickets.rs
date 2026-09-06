use std::collections::HashSet;

use sqlx::{Sqlite, Transaction};

use crate::{
    domain::{
        Message, Project, Ticket, TicketActivity, TicketBlocker, TicketState, WakeDeliveryPolicy,
    },
    error::AppError,
};

use super::{
    Store,
    messages::{MessageWrite, insert_message},
};

#[derive(Debug)]
pub struct NewTicket {
    pub title: String,
    pub description: String,
    pub blocker_ids: Vec<i64>,
    pub created_by_agent_id: Option<i64>,
}

#[derive(Debug, Default)]
pub struct TicketChanges {
    pub title: Option<String>,
    pub description: Option<String>,
    pub state: Option<TicketState>,
    pub assignee_id: Option<Option<i64>>,
    pub blocker_ids: Option<Vec<i64>>,
    pub expected_revision: Option<i64>,
}

pub(crate) struct MergeNotification<'a> {
    pub channel: &'a str,
    pub author: &'a str,
    pub body: &'a str,
    pub deduplication_key: &'a str,
}

pub struct ReviewSubmission<'a> {
    pub implementation_branch: Option<&'a str>,
    pub commit: &'a str,
    pub target_branch: &'a str,
    pub project_id: i64,
    pub project_path: &'a str,
}

#[derive(Clone, Copy)]
enum TicketWakeReason {
    Available,
    Assigned,
    ReviewRequested,
}

impl Store {
    pub async fn project(&self) -> Result<Option<Project>, sqlx::Error> {
        sqlx::query_as(
            "SELECT p.id, p.name, p.path, p.default_branch, p.archived_at, p.updated_at FROM projects p \
             JOIN board_state state ON state.current_project_id = p.id WHERE state.id = 1",
        )
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn project_by_id(&self, id: i64) -> Result<Option<Project>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id, name, path, default_branch, archived_at, updated_at FROM projects WHERE id = ?",
        )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn project_by_path(&self, path: &str) -> Result<Option<Project>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id, name, path, default_branch, archived_at, updated_at FROM projects WHERE path = ?",
        )
        .bind(path)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn select_project(&self, id: i64) -> Result<Project, AppError> {
        let project = self
            .project_by_id(id)
            .await?
            .ok_or_else(|| AppError::not_found("project not found"))?;
        if project.archived_at.is_some() {
            return Err(AppError::conflict("project is archived"));
        }
        sqlx::query(
            "INSERT INTO board_state (id, current_project_id) VALUES (1, ?) \
             ON CONFLICT(id) DO UPDATE SET current_project_id = excluded.current_project_id",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(project)
    }

    pub async fn set_project(&self, path: &str) -> Result<Project, sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        let name = std::path::Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(path);
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO projects (name, path, default_branch) VALUES (?, ?, 'HEAD') \
             ON CONFLICT(path) DO UPDATE SET updated_at = unixepoch() RETURNING id",
        )
        .bind(name)
        .bind(path)
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO board_state (id, current_project_id) VALUES (1, ?) \
             ON CONFLICT(id) DO UPDATE SET current_project_id = excluded.current_project_id",
        )
        .bind(id)
        .execute(&mut *transaction)
        .await?;
        let project =
            sqlx::query_as("SELECT id, name, path, default_branch, archived_at, updated_at FROM projects WHERE id = ?")
                .bind(id)
                .fetch_one(&mut *transaction)
                .await?;
        transaction.commit().await?;
        Ok(project)
    }

    pub async fn projects(&self) -> Result<Vec<Project>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id, name, path, default_branch, archived_at, updated_at FROM projects ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn register_project(
        &self,
        name: &str,
        path: &str,
        default_branch: &str,
    ) -> Result<Project, AppError> {
        let name = name.trim();
        let default_branch = default_branch.trim();
        if name.is_empty() || name.len() > 100 {
            return Err(AppError::bad_request(
                "project name must contain 1-100 characters",
            ));
        }
        if default_branch.is_empty() {
            return Err(AppError::bad_request("default branch is required"));
        }
        let mut transaction = self.pool.begin().await?;
        if sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM projects WHERE path = ?)")
            .bind(path)
            .fetch_one(&mut *transaction)
            .await?
        {
            return Err(AppError::conflict("project root is already registered"));
        }
        let id = sqlx::query("INSERT INTO projects (name, path, default_branch) VALUES (?, ?, ?)")
            .bind(name)
            .bind(path)
            .bind(default_branch)
            .execute(&mut *transaction)
            .await?
            .last_insert_rowid();
        sqlx::query(
            "INSERT INTO board_state (id, current_project_id) VALUES (1, ?) \
             ON CONFLICT(id) DO UPDATE SET current_project_id = COALESCE(current_project_id, excluded.current_project_id)",
        )
        .bind(id)
        .execute(&mut *transaction)
        .await?;
        let project = sqlx::query_as(
            "SELECT id, name, path, default_branch, archived_at, updated_at FROM projects WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(project)
    }

    pub async fn archive_project(&self, id: i64) -> Result<Project, AppError> {
        let mut transaction = self.pool.begin().await?;
        let project: Option<Project> = sqlx::query_as(
            "SELECT id, name, path, default_branch, archived_at, updated_at FROM projects WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&mut *transaction)
        .await?;
        let project = project.ok_or_else(|| AppError::not_found("project not found"))?;
        if project.archived_at.is_none() {
            let active_work: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM tickets WHERE project_id = ? AND state IN ('in_progress', 'review'))",
            )
            .bind(id)
            .fetch_one(&mut *transaction)
            .await?;
            if active_work {
                return Err(AppError::conflict(
                    "cannot archive a project with active or review work",
                ));
            }
            sqlx::query(
                "UPDATE projects SET archived_at = unixepoch(), updated_at = unixepoch() WHERE id = ?",
            )
            .bind(id)
            .execute(&mut *transaction)
            .await?;
            sqlx::query(
                "UPDATE board_state SET current_project_id = (SELECT id FROM projects WHERE archived_at IS NULL AND id != ? ORDER BY id LIMIT 1) WHERE id = 1 AND current_project_id = ?",
            )
            .bind(id)
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        }
        let archived = sqlx::query_as(
            "SELECT id, name, path, default_branch, archived_at, updated_at FROM projects WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(archived)
    }

    pub(crate) async fn active_ticket_project(&self, ticket: &Ticket) -> Result<Project, AppError> {
        let project = self
            .project_by_id(ticket.project_id)
            .await?
            .ok_or_else(|| AppError::conflict("ticket project no longer exists"))?;
        if project.archived_at.is_some() {
            return Err(AppError::conflict("ticket project is archived"));
        }
        Ok(project)
    }

    pub(crate) async fn review_ticket_project(&self, ticket: &Ticket) -> Result<Project, AppError> {
        let project = self.active_ticket_project(ticket).await?;
        if ticket.review_project_id != Some(project.id)
            || ticket.review_project_path.as_deref() != Some(project.path.as_str())
        {
            return Err(AppError::conflict(
                "ticket project no longer matches its recorded review context",
            ));
        }
        Ok(project)
    }

    pub async fn tickets(&self) -> Result<Vec<Ticket>, sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        let mut tickets: Vec<Ticket> = sqlx::query_as(
            "SELECT id, project_id, title, description, state, created_by_agent_id, reserved_for_agent_id, assignee_id, implementation_branch, review_commit, review_target_branch, review_project_id, review_project_path, reviewer_id, review_feedback, revision, created_at, updated_at \
             FROM tickets ORDER BY id",
        )
        .fetch_all(&mut *transaction)
        .await?;
        for ticket in &mut tickets {
            load_ticket_dependencies(&mut transaction, ticket).await?;
        }
        transaction.commit().await?;
        Ok(tickets)
    }

    pub async fn ticket(&self, id: i64) -> Result<Option<Ticket>, sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        let ticket: Option<Ticket> = sqlx::query_as(
            "SELECT id, project_id, title, description, state, created_by_agent_id, reserved_for_agent_id, assignee_id, implementation_branch, review_commit, review_target_branch, review_project_id, review_project_path, reviewer_id, review_feedback, revision, created_at, updated_at \
             FROM tickets WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&mut *transaction)
        .await?;
        let ticket = if let Some(mut ticket) = ticket {
            load_ticket_dependencies(&mut transaction, &mut ticket).await?;
            Some(ticket)
        } else {
            None
        };
        transaction.commit().await?;
        Ok(ticket)
    }

    pub async fn create_ticket(&self, new: NewTicket) -> Result<Ticket, AppError> {
        Ok(self.create_ticket_with_wakes(new).await?.0)
    }

    pub async fn create_ticket_with_wakes(
        &self,
        new: NewTicket,
    ) -> Result<(Ticket, Vec<i64>), AppError> {
        self.create_ticket_for_project_with_wakes(new, None).await
    }

    pub async fn create_ticket_for_project_with_wakes(
        &self,
        new: NewTicket,
        project_id: Option<i64>,
    ) -> Result<(Ticket, Vec<i64>), AppError> {
        validate_title(&new.title)?;
        let project = match project_id {
            Some(id) => self.project_by_id(id).await?,
            None => self.project().await?,
        }
        .ok_or_else(|| AppError::bad_request("ticket project does not exist"))?;
        if project.archived_at.is_some() {
            return Err(AppError::conflict("ticket project is archived"));
        }
        if let Some(agent_id) = new.created_by_agent_id
            && self.agent(agent_id).await?.is_none()
        {
            return Err(AppError::bad_request("creating agent does not exist"));
        }
        let mut transaction = self.pool.begin().await?;
        let result = sqlx::query(
            "INSERT INTO tickets (project_id, title, description, created_by_agent_id) VALUES (?, ?, ?, ?)",
        )
        .bind(project.id)
        .bind(new.title.trim())
        .bind(new.description)
        .bind(new.created_by_agent_id)
        .execute(&mut *transaction)
        .await?;
        replace_ticket_dependencies(
            &mut transaction,
            result.last_insert_rowid(),
            &new.blocker_ids,
        )
        .await?;
        let ticket = ticket_in_transaction(&mut transaction, result.last_insert_rowid()).await?;
        let wake_agent_ids = if ticket_is_available(&ticket) {
            insert_pending_ticket_wakes(
                &mut transaction,
                None,
                &ticket,
                TicketWakeReason::Available,
            )
            .await?
        } else {
            Vec::new()
        };
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
        self.active_ticket_project(&current).await?;
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
        if current.reserved_for_agent_id.is_some()
            && assignee_id != current.assignee_id
            && assignee_id != current.reserved_for_agent_id
        {
            return Err(AppError::conflict(
                "ticket is reserved for another agent; override the reservation first",
            ));
        }
        let blocker_ids = changes
            .blocker_ids
            .unwrap_or_else(|| current.blocker_ids.clone());
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
        replace_ticket_dependencies(&mut transaction, id, &blocker_ids).await?;
        let ticket = ticket_in_transaction(&mut transaction, id).await?;
        if !ticket.unresolved_blockers.is_empty()
            && ((current.state == TicketState::Todo && ticket.state == TicketState::InProgress)
                || (current.assignee_id.is_none() && ticket.assignee_id.is_some()))
        {
            return Err(blocked_ticket_error(&ticket));
        }
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

    pub async fn reserve_tickets(
        &self,
        ticket_ids: &[i64],
        agent_id: i64,
        actor_agent_id: Option<i64>,
        administrative_override: bool,
    ) -> Result<Vec<Ticket>, AppError> {
        if ticket_ids.is_empty() {
            return Err(AppError::bad_request("at least one ticket ID is required"));
        }
        if ticket_ids.iter().copied().collect::<HashSet<_>>().len() != ticket_ids.len() {
            return Err(AppError::bad_request("duplicate ticket ID"));
        }
        if self.agent(agent_id).await?.is_none() {
            return Err(AppError::bad_request("reservation agent does not exist"));
        }
        let mut transaction = self.pool.begin().await?;
        let mut tickets = Vec::with_capacity(ticket_ids.len());
        for ticket_id in ticket_ids {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tickets WHERE id = ?)")
                    .bind(ticket_id)
                    .fetch_one(&mut *transaction)
                    .await?;
            if !exists {
                return Err(AppError::not_found(format!(
                    "ticket #{ticket_id} not found"
                )));
            }
            let ticket = ticket_in_transaction(&mut transaction, *ticket_id).await?;
            let archived_at: Option<i64> =
                sqlx::query_scalar("SELECT archived_at FROM projects WHERE id = ?")
                    .bind(ticket.project_id)
                    .fetch_optional(&mut *transaction)
                    .await?
                    .ok_or_else(|| AppError::conflict("ticket project no longer exists"))?;
            if archived_at.is_some() {
                return Err(AppError::conflict("ticket project is archived"));
            }
            if ticket.state != TicketState::Todo || ticket.assignee_id.is_some() {
                return Err(AppError::conflict(format!(
                    "ticket #{} is not unassigned todo work",
                    ticket.id
                )));
            }
            if !administrative_override
                && ticket
                    .reserved_for_agent_id
                    .is_some_and(|owner| owner != agent_id)
            {
                return Err(AppError::conflict(format!(
                    "ticket #{} is reserved for another agent",
                    ticket.id
                )));
            }
            tickets.push(ticket);
        }

        let mut reserved = Vec::with_capacity(tickets.len());
        for ticket in tickets {
            if ticket.reserved_for_agent_id != Some(agent_id) {
                let result = sqlx::query(
                    "UPDATE tickets SET reserved_for_agent_id = ?, revision = revision + 1, \
                     updated_at = unixepoch() WHERE id = ? AND state = 'todo' \
                     AND assignee_id IS NULL AND revision = ? \
                     AND (? OR reserved_for_agent_id IS NULL OR reserved_for_agent_id = ?)",
                )
                .bind(agent_id)
                .bind(ticket.id)
                .bind(ticket.revision)
                .bind(administrative_override)
                .bind(agent_id)
                .execute(&mut *transaction)
                .await?;
                if result.rows_affected() == 0 {
                    return Err(AppError::conflict(format!(
                        "ticket #{} changed while it was being reserved",
                        ticket.id
                    )));
                }
                if administrative_override {
                    sqlx::query(
                        "INSERT INTO ticket_activity \
                         (ticket_id, action, actor_agent_id, previous_reserved_for_agent_id, reserved_for_agent_id) \
                         VALUES (?, 'reservation_override', ?, ?, ?)",
                    )
                    .bind(ticket.id)
                    .bind(actor_agent_id)
                    .bind(ticket.reserved_for_agent_id)
                    .bind(agent_id)
                    .execute(&mut *transaction)
                    .await?;
                }
            }
            reserved.push(ticket_in_transaction(&mut transaction, ticket.id).await?);
        }
        transaction.commit().await?;
        Ok(reserved)
    }

    pub async fn ticket_activity(&self, ticket_id: i64) -> Result<Vec<TicketActivity>, AppError> {
        if self.ticket(ticket_id).await?.is_none() {
            return Err(AppError::not_found("ticket not found"));
        }
        Ok(sqlx::query_as(
            "SELECT id, ticket_id, action, actor_agent_id, previous_reserved_for_agent_id, \
             reserved_for_agent_id, created_at FROM ticket_activity \
             WHERE ticket_id = ? ORDER BY created_at, id",
        )
        .bind(ticket_id)
        .fetch_all(&self.pool)
        .await?)
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
        self.active_ticket_project(&current).await?;
        if !current.unresolved_blockers.is_empty() {
            return Err(blocked_ticket_error(&current));
        }
        if current
            .reserved_for_agent_id
            .is_some_and(|owner| owner != agent_id)
        {
            return Err(AppError::conflict("ticket is reserved for another agent"));
        }
        let mut transaction = self.pool.begin().await?;
        let claimed: Option<Ticket> = sqlx::query_as(
            "UPDATE tickets SET assignee_id = ?, reserved_for_agent_id = NULL, state = 'in_progress', revision = revision + 1, \
             updated_at = unixepoch() WHERE id = ? AND (assignee_id IS NULL OR assignee_id = ?) \
             AND (reserved_for_agent_id IS NULL OR reserved_for_agent_id = ?) \
             AND NOT EXISTS (SELECT 1 FROM ticket_dependencies d JOIN tickets blocker ON blocker.id = d.blocker_id \
             WHERE d.ticket_id = tickets.id AND blocker.state != 'done') \
             AND (? IS NULL OR revision = ?) \
             RETURNING id, project_id, title, description, state, created_by_agent_id, reserved_for_agent_id, assignee_id, implementation_branch, review_commit, review_target_branch, review_project_id, review_project_path, reviewer_id, review_feedback, revision, created_at, updated_at",
        )
        .bind(agent_id)
        .bind(id)
        .bind(agent_id)
        .bind(agent_id)
        .bind(expected_revision)
        .bind(expected_revision)
        .fetch_optional(&mut *transaction)
        .await?;
        match claimed {
            Some(mut ticket) => {
                load_ticket_dependencies(&mut transaction, &mut ticket).await?;
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
        submission: ReviewSubmission<'_>,
        expected_revision: Option<i64>,
    ) -> Result<Ticket, AppError> {
        Ok(self
            .submit_review_with_wakes(id, agent_id, submission, expected_revision)
            .await?
            .0)
    }

    pub async fn submit_review_with_wakes(
        &self,
        id: i64,
        agent_id: i64,
        submission: ReviewSubmission<'_>,
        expected_revision: Option<i64>,
    ) -> Result<(Ticket, Vec<i64>), AppError> {
        let current = self
            .ticket(id)
            .await?
            .ok_or_else(|| AppError::not_found("ticket not found"))?;
        let project = self.active_ticket_project(&current).await?;
        if project.id != submission.project_id || project.path != submission.project_path {
            return Err(AppError::conflict(
                "ticket project changed during review submission",
            ));
        }
        let mut transaction = self.pool.begin().await?;
        let ticket = sqlx::query_as(
            "UPDATE tickets SET state = 'review', implementation_branch = ?, review_commit = ?, review_target_branch = ?, review_project_id = ?, review_project_path = ?, reviewer_id = NULL, \
             revision = revision + 1, updated_at = unixepoch() \
             WHERE id = ? AND state = 'in_progress' AND assignee_id = ? \
             AND project_id = ? \
             AND (? IS NULL OR revision = ?) \
             RETURNING id, project_id, title, description, state, created_by_agent_id, reserved_for_agent_id, assignee_id, implementation_branch, review_commit, review_target_branch, review_project_id, review_project_path, reviewer_id, review_feedback, revision, created_at, updated_at",
        )
        .bind(submission.implementation_branch)
        .bind(submission.commit)
        .bind(submission.target_branch)
        .bind(submission.project_id)
        .bind(submission.project_path)
        .bind(id)
        .bind(agent_id)
        .bind(submission.project_id)
        .bind(expected_revision)
        .bind(expected_revision)
        .fetch_optional(&mut *transaction)
        .await?;
        let mut ticket = ticket.ok_or_else(|| {
            AppError::conflict(
                "ticket is not assigned to this agent, is not in progress, or has changed",
            )
        })?;
        load_ticket_dependencies(&mut transaction, &mut ticket).await?;
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
        let current = self
            .ticket(id)
            .await?
            .ok_or_else(|| AppError::not_found("ticket not found"))?;
        self.review_ticket_project(&current).await?;
        let mut transaction = self.pool.begin().await?;
        let ticket = sqlx::query_as(
            "UPDATE tickets SET reviewer_id = ?, revision = revision + 1, updated_at = unixepoch() \
             WHERE id = ? AND state = 'review' AND assignee_id != ? \
             AND review_project_id = project_id AND review_project_path = \
                 (SELECT path FROM projects WHERE id = tickets.project_id AND archived_at IS NULL) \
             AND (reviewer_id IS NULL OR reviewer_id = ?) AND (? IS NULL OR revision = ?) \
             RETURNING id, project_id, title, description, state, created_by_agent_id, reserved_for_agent_id, assignee_id, implementation_branch, review_commit, review_target_branch, review_project_id, review_project_path, reviewer_id, review_feedback, revision, created_at, updated_at",
        )
        .bind(reviewer_id)
        .bind(id)
        .bind(reviewer_id)
        .bind(reviewer_id)
        .bind(expected_revision)
        .bind(expected_revision)
        .fetch_optional(&mut *transaction)
        .await?;
        let mut ticket = ticket.ok_or_else(|| {
            AppError::conflict("review is unavailable, belongs to this agent, or has changed")
        })?;
        load_ticket_dependencies(&mut transaction, &mut ticket).await?;
        transaction.commit().await?;
        Ok(ticket)
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
        let current = self
            .ticket(id)
            .await?
            .ok_or_else(|| AppError::not_found("ticket not found"))?;
        self.review_ticket_project(&current).await?;
        let mut transaction = self.pool.begin().await?;
        let ticket = sqlx::query_as(
            "UPDATE tickets SET state = 'in_progress', review_feedback = ?, revision = revision + 1, \
             updated_at = unixepoch() WHERE id = ? AND state = 'review' AND reviewer_id = ? \
             AND review_project_id = project_id AND review_project_path = \
                 (SELECT path FROM projects WHERE id = tickets.project_id AND archived_at IS NULL) \
             AND (? IS NULL OR revision = ?) \
             RETURNING id, project_id, title, description, state, created_by_agent_id, reserved_for_agent_id, assignee_id, implementation_branch, review_commit, review_target_branch, review_project_id, review_project_path, reviewer_id, review_feedback, revision, created_at, updated_at",
        )
        .bind(feedback.trim())
        .bind(id)
        .bind(reviewer_id)
        .bind(expected_revision)
        .bind(expected_revision)
        .fetch_optional(&mut *transaction)
        .await?;
        let mut ticket = ticket.ok_or_else(|| {
            AppError::conflict("review is not assigned to this agent or has changed")
        })?;
        load_ticket_dependencies(&mut transaction, &mut ticket).await?;
        transaction.commit().await?;
        Ok(ticket)
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
        let current = self
            .ticket(id)
            .await?
            .ok_or_else(|| AppError::not_found("ticket not found"))?;
        self.review_ticket_project(&current).await?;
        let mut transaction = self.pool.begin().await?;
        let ticket = sqlx::query_as(
            "UPDATE tickets SET state = 'done', revision = revision + 1, updated_at = unixepoch() \
             WHERE id = ? AND state = 'review' AND reviewer_id = ? AND review_commit = ? \
             AND review_project_id = project_id AND review_project_path = \
                 (SELECT path FROM projects WHERE id = tickets.project_id AND archived_at IS NULL) \
             AND (? IS NULL OR revision = ?) \
             RETURNING id, project_id, title, description, state, created_by_agent_id, reserved_for_agent_id, assignee_id, implementation_branch, review_commit, review_target_branch, review_project_id, review_project_path, reviewer_id, review_feedback, revision, created_at, updated_at",
        )
        .bind(id)
        .bind(reviewer_id)
        .bind(commit)
        .bind(expected_revision)
        .bind(expected_revision)
        .fetch_optional(&mut *transaction)
        .await?;
        let mut ticket = ticket.ok_or_else(|| {
            AppError::conflict("review is not assigned to this agent or has changed")
        })?;
        load_ticket_dependencies(&mut transaction, &mut ticket).await?;
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
    let mut ticket = sqlx::query_as(
        "SELECT id, project_id, title, description, state, created_by_agent_id, reserved_for_agent_id, assignee_id, implementation_branch, review_commit, \
         review_target_branch, review_project_id, review_project_path, reviewer_id, review_feedback, revision, created_at, updated_at \
         FROM tickets WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&mut **transaction)
    .await?;
    load_ticket_dependencies(transaction, &mut ticket).await?;
    Ok(ticket)
}

async fn load_ticket_dependencies(
    transaction: &mut Transaction<'_, Sqlite>,
    ticket: &mut Ticket,
) -> Result<(), sqlx::Error> {
    let blockers: Vec<(i64, String, TicketState)> = sqlx::query_as(
        "SELECT blocker.id, blocker.title, blocker.state FROM ticket_dependencies dependency \
         JOIN tickets blocker ON blocker.id = dependency.blocker_id \
         WHERE dependency.ticket_id = ? ORDER BY blocker.id",
    )
    .bind(ticket.id)
    .fetch_all(&mut **transaction)
    .await?;
    ticket.blocker_ids = blockers.iter().map(|(id, _, _)| *id).collect();
    ticket.unresolved_blockers = blockers
        .into_iter()
        .filter(|(_, _, state)| *state != TicketState::Done)
        .map(|(id, title, _)| TicketBlocker { id, title })
        .collect();
    Ok(())
}

async fn replace_ticket_dependencies(
    transaction: &mut Transaction<'_, Sqlite>,
    ticket_id: i64,
    blocker_ids: &[i64],
) -> Result<(), AppError> {
    let unique: HashSet<_> = blocker_ids.iter().copied().collect();
    if unique.len() != blocker_ids.len() {
        return Err(AppError::bad_request("duplicate blocker ticket ID"));
    }
    for blocker_id in blocker_ids {
        if *blocker_id == ticket_id {
            return Err(AppError::bad_request("a ticket cannot block itself"));
        }
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tickets WHERE id = ?)")
            .bind(blocker_id)
            .fetch_one(&mut **transaction)
            .await?;
        if !exists {
            return Err(AppError::bad_request(format!(
                "blocker ticket #{blocker_id} does not exist"
            )));
        }
        let creates_cycle: bool = sqlx::query_scalar(
            "WITH RECURSIVE reachable(id) AS ( \
             SELECT ? UNION SELECT dependency.blocker_id FROM ticket_dependencies dependency \
             JOIN reachable ON dependency.ticket_id = reachable.id) \
             SELECT EXISTS(SELECT 1 FROM reachable WHERE id = ?)",
        )
        .bind(blocker_id)
        .bind(ticket_id)
        .fetch_one(&mut **transaction)
        .await?;
        if creates_cycle {
            return Err(AppError::bad_request(format!(
                "blocker ticket #{blocker_id} creates a dependency cycle"
            )));
        }
    }
    sqlx::query("DELETE FROM ticket_dependencies WHERE ticket_id = ?")
        .bind(ticket_id)
        .execute(&mut **transaction)
        .await?;
    for blocker_id in blocker_ids {
        sqlx::query("INSERT INTO ticket_dependencies (ticket_id, blocker_id) VALUES (?, ?)")
            .bind(ticket_id)
            .bind(blocker_id)
            .execute(&mut **transaction)
            .await?;
    }
    Ok(())
}

fn ticket_is_available(ticket: &Ticket) -> bool {
    ticket.state == TicketState::Todo
        && ticket.assignee_id.is_none()
        && ticket.reserved_for_agent_id.is_none()
        && ticket.unresolved_blockers.is_empty()
}

fn blocked_ticket_error(ticket: &Ticket) -> AppError {
    AppError::conflict(format!(
        "ticket is blocked by {}",
        ticket
            .unresolved_blockers
            .iter()
            .map(|blocker| format!("#{} '{}'", blocker.id, blocker.title))
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

fn ticket_changed_fields(previous: Option<&Ticket>, ticket: &Ticket) -> Vec<&'static str> {
    let Some(previous) = previous else {
        return vec![
            "project_id",
            "title",
            "description",
            "state",
            "assignee_id",
            "created_by_agent_id",
            "reserved_for_agent_id",
            "blocker_ids",
        ];
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
    if previous.reserved_for_agent_id != ticket.reserved_for_agent_id {
        fields.push("reserved_for_agent_id");
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
    if previous.blocker_ids != ticket.blocker_ids {
        fields.push("blocker_ids");
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
            "project_id": ticket.project_id,
            "title": ticket.title,
            "state": ticket.state,
            "assignee_id": ticket.assignee_id,
            "created_by_agent_id": ticket.created_by_agent_id,
            "reserved_for_agent_id": ticket.reserved_for_agent_id,
            "implementation_branch": ticket.implementation_branch,
            "review_commit": ticket.review_commit,
            "review_target_branch": ticket.review_target_branch,
            "review_project_id": ticket.review_project_id,
            "review_project_path": ticket.review_project_path,
            "reviewer_id": ticket.reviewer_id,
            "blocker_ids": ticket.blocker_ids,
            "unresolved_blockers": ticket.unresolved_blockers,
        }
    }))
    .map_err(|error| AppError::internal(format!("failed to encode ticket wake event: {error}")))?;
    let deduplication_key = format!("ticket:{}:{}:{kind}", ticket.id, ticket.revision);
    let mut queued = Vec::new();
    for agent_id in agent_ids {
        let result = sqlx::query(
            "INSERT OR IGNORE INTO agent_wake_events \
             (agent_id, kind, payload, deduplication_key, delivery_policy) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(agent_id)
        .bind(kind)
        .bind(&payload)
        .bind(&deduplication_key)
        .bind(WakeDeliveryPolicy::IdleOnly)
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
    async fn tickets_keep_project_identity_and_reject_archived_projects() {
        let store = Store::memory().await.unwrap();
        let project = store.project().await.unwrap().unwrap();
        let watcher = store.create_agent("watcher", "/tmp/watcher").await.unwrap();
        store
            .set_agent_wake_settings(watcher.id, true, false, None)
            .await
            .unwrap();
        let (ticket, _) = store
            .create_ticket_with_wakes(NewTicket {
                title: "Project work".into(),
                description: String::new(),
                blocker_ids: Vec::new(),
                created_by_agent_id: None,
            })
            .await
            .unwrap();

        assert_eq!(ticket.project_id, project.id);
        assert_eq!(
            store.wake_events(watcher.id).await.unwrap()[0].payload["ticket"]["project_id"],
            project.id
        );
        let other = store.set_project("/tmp/another-project").await.unwrap();
        assert_ne!(other.id, project.id);
        assert_eq!(
            store.ticket(ticket.id).await.unwrap().unwrap().project_id,
            project.id
        );
        assert_eq!(
            store.project_by_id(project.id).await.unwrap().unwrap().path,
            project.path
        );

        store
            .execute_test_sql("UPDATE projects SET archived_at = unixepoch() WHERE id = 1")
            .await
            .unwrap();
        let error = store
            .claim_ticket(ticket.id, watcher.id, Some(ticket.revision))
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "ticket project is archived");
    }

    #[tokio::test]
    async fn review_operations_reject_changed_project_context() {
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
        let ticket = store
            .create_ticket(NewTicket {
                title: "Pinned context".into(),
                description: String::new(),
                blocker_ids: Vec::new(),
                created_by_agent_id: None,
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
                ReviewSubmission {
                    implementation_branch: Some("tickets/work"),
                    commit: "abc123",
                    target_branch: "main",
                    project_id: ticket.project_id,
                    project_path: "/tmp/multi-agent-board-test-project",
                },
                Some(ticket.revision),
            )
            .await
            .unwrap();
        store
            .execute_test_sql("UPDATE tickets SET review_project_path = '/tmp/other-project'")
            .await
            .unwrap();

        let error = store
            .claim_review(ticket.id, reviewer.id, Some(ticket.revision))
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "ticket project no longer matches its recorded review context"
        );
    }

    #[tokio::test]
    async fn ticket_updates_use_revisions() {
        let store = Store::memory().await.unwrap();
        let ticket = store
            .create_ticket(NewTicket {
                title: "First".into(),
                description: String::new(),
                blocker_ids: Vec::new(),
                created_by_agent_id: None,
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
    async fn dependencies_are_validated_and_block_claims_until_done() {
        let store = Store::memory().await.unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        store
            .set_agent_wake_settings(agent.id, true, false, None)
            .await
            .unwrap();
        let blocker = store
            .create_ticket(NewTicket {
                title: "Foundation".into(),
                description: String::new(),
                blocker_ids: Vec::new(),
                created_by_agent_id: None,
            })
            .await
            .unwrap();
        let dependent = store
            .create_ticket(NewTicket {
                title: "Follow-up".into(),
                description: String::new(),
                blocker_ids: vec![blocker.id],
                created_by_agent_id: None,
            })
            .await
            .unwrap();

        assert_eq!(dependent.blocker_ids, vec![blocker.id]);
        assert_eq!(dependent.unresolved_blockers[0].title, "Foundation");
        assert_eq!(store.wake_events(agent.id).await.unwrap().len(), 1);
        assert!(
            store
                .claim_ticket(dependent.id, agent.id, Some(dependent.revision))
                .await
                .unwrap_err()
                .to_string()
                .contains("blocked by")
        );
        for changes in [
            TicketChanges {
                state: Some(TicketState::InProgress),
                expected_revision: Some(dependent.revision),
                ..Default::default()
            },
            TicketChanges {
                assignee_id: Some(Some(agent.id)),
                expected_revision: Some(dependent.revision),
                ..Default::default()
            },
        ] {
            assert!(store.update_ticket(dependent.id, changes).await.is_err());
        }

        store
            .update_ticket(
                blocker.id,
                TicketChanges {
                    state: Some(TicketState::Done),
                    expected_revision: Some(blocker.revision),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let refreshed = store.ticket(dependent.id).await.unwrap().unwrap();
        assert!(refreshed.unresolved_blockers.is_empty());
        assert_eq!(refreshed.blocker_ids, vec![blocker.id]);
        assert_eq!(
            store
                .claim_ticket(dependent.id, agent.id, Some(refreshed.revision))
                .await
                .unwrap()
                .state,
            TicketState::InProgress
        );

        for blocker_ids in [vec![999], vec![dependent.id], vec![blocker.id, blocker.id]] {
            assert!(
                store
                    .update_ticket(
                        dependent.id,
                        TicketChanges {
                            blocker_ids: Some(blocker_ids),
                            ..Default::default()
                        },
                    )
                    .await
                    .is_err()
            );
        }
        assert!(
            store
                .update_ticket(
                    blocker.id,
                    TicketChanges {
                        blocker_ids: Some(vec![dependent.id]),
                        ..Default::default()
                    },
                )
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn reservations_preserve_provenance_enforce_claims_and_roll_back_in_bulk() {
        let store = Store::memory().await.unwrap();
        let creator = store.create_agent("creator", "/tmp/creator").await.unwrap();
        let reserved = store
            .create_agent("reserved", "/tmp/reserved")
            .await
            .unwrap();
        let other = store.create_agent("other", "/tmp/other").await.unwrap();
        let first = store
            .create_ticket(NewTicket {
                title: "First".into(),
                description: String::new(),
                blocker_ids: Vec::new(),
                created_by_agent_id: Some(creator.id),
            })
            .await
            .unwrap();
        let second = store
            .create_ticket(NewTicket {
                title: "Second".into(),
                description: String::new(),
                blocker_ids: Vec::new(),
                created_by_agent_id: None,
            })
            .await
            .unwrap();

        let tickets = store
            .reserve_tickets(
                &[first.id, second.id],
                reserved.id,
                Some(reserved.id),
                false,
            )
            .await
            .unwrap();
        assert_eq!(tickets[0].created_by_agent_id, Some(creator.id));
        assert_eq!(tickets[0].reserved_for_agent_id, Some(reserved.id));
        assert_eq!(tickets[0].state, TicketState::Todo);
        assert_eq!(tickets[0].assignee_id, None);
        assert!(
            store
                .claim_ticket(first.id, other.id, Some(tickets[0].revision))
                .await
                .is_err()
        );
        assert!(
            store
                .update_ticket(
                    first.id,
                    TicketChanges {
                        assignee_id: Some(Some(other.id)),
                        expected_revision: Some(tickets[0].revision),
                        ..Default::default()
                    },
                )
                .await
                .is_err()
        );
        let claimed = store
            .claim_ticket(first.id, reserved.id, Some(tickets[0].revision))
            .await
            .unwrap();
        assert_eq!(claimed.assignee_id, Some(reserved.id));
        assert_eq!(claimed.reserved_for_agent_id, None);
        assert_eq!(claimed.created_by_agent_id, Some(creator.id));

        let rollback = store
            .create_ticket(NewTicket {
                title: "Rollback".into(),
                description: String::new(),
                blocker_ids: Vec::new(),
                created_by_agent_id: None,
            })
            .await
            .unwrap();
        assert!(
            store
                .reserve_tickets(&[rollback.id, claimed.id], other.id, Some(other.id), false,)
                .await
                .is_err()
        );
        assert_eq!(
            store
                .ticket(rollback.id)
                .await
                .unwrap()
                .unwrap()
                .reserved_for_agent_id,
            None
        );

        let overridden = store
            .reserve_tickets(&[second.id], other.id, None, true)
            .await
            .unwrap();
        assert_eq!(overridden[0].reserved_for_agent_id, Some(other.id));
        let activity = store.ticket_activity(second.id).await.unwrap();
        assert_eq!(activity.len(), 1);
        assert_eq!(activity[0].action, "reservation_override");
        assert_eq!(
            activity[0].previous_reserved_for_agent_id,
            Some(reserved.id)
        );
        assert_eq!(activity[0].reserved_for_agent_id, Some(other.id));
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
                blocker_ids: Vec::new(),
                created_by_agent_id: None,
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
                ReviewSubmission {
                    implementation_branch: Some("agents/implementer"),
                    commit: "abc123",
                    target_branch: "main",
                    project_id: ticket.project_id,
                    project_path: "/tmp/multi-agent-board-test-project",
                },
                Some(ticket.revision),
            )
            .await
            .unwrap();
        assert_eq!(review_targets, vec![review_only.id, both.id]);
        assert_eq!(store.wake_events(work_only.id).await.unwrap().len(), 1);
        let reviewer_events = store.wake_events(review_only.id).await.unwrap();
        assert_eq!(reviewer_events[0].kind, "ticket.review_requested");
        assert_eq!(
            reviewer_events[0].delivery_policy,
            WakeDeliveryPolicy::IdleOnly
        );
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
                blocker_ids: Vec::new(),
                created_by_agent_id: None,
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
                ReviewSubmission {
                    implementation_branch: Some("agents/worker"),
                    commit: "abc123",
                    target_branch: "main",
                    project_id: ticket.project_id,
                    project_path: "/tmp/multi-agent-board-test-project",
                },
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

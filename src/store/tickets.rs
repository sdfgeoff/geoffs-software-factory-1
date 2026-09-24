use std::collections::{HashMap, HashSet};

use sqlx::{QueryBuilder, Sqlite, Transaction};

use crate::{
    domain::{
        Message, Project, ProjectCompletionWorkflow, Ticket, TicketActivity, TicketBlocker,
        TicketState, TicketSummary, TicketWakePayload, TicketWakeTicket, WakeDeliveryPolicy,
        WakeEventData,
    },
    error::AppError,
};

use super::{
    Store,
    attachments::{AttachmentOwner, attachments_for, link_attachments},
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
    pub reserved_for_agent_id: Option<Option<i64>>,
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

pub const DEFAULT_TICKET_PAGE_SIZE: i64 = 50;
pub const MAX_TICKET_PAGE_SIZE: i64 = 100;
pub const DEFAULT_TICKET_ACTIVITY_PAGE_SIZE: i64 = 50;
pub const MAX_TICKET_ACTIVITY_PAGE_SIZE: i64 = 100;

#[derive(Clone, Copy, Debug, Default)]
pub struct TicketSummaryFilter {
    pub project_id: Option<i64>,
    pub state: Option<TicketState>,
    pub before_id: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Clone, Copy)]
enum TicketWakeReason {
    Available,
    Assigned,
    Reserved,
    ReviewRequested,
    ExternalMergeReady,
}

impl Store {
    // Legacy setup compatibility for migrations and tests. Runtime API and MCP
    // paths require explicit project IDs and never read this board-wide value.
    pub async fn project(&self) -> Result<Option<Project>, sqlx::Error> {
        sqlx::query_as(
            "SELECT p.id, p.name, p.path, p.default_branch, p.completion_workflow, p.archived_at, p.updated_at FROM projects p \
             JOIN board_state state ON state.current_project_id = p.id WHERE state.id = 1",
        )
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn project_by_id(&self, id: i64) -> Result<Option<Project>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id, name, path, default_branch, completion_workflow, archived_at, updated_at FROM projects WHERE id = ?",
        )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
    }

    // Creates a project fixture while preserving old migration-test semantics.
    // Product registration uses register_project and does not select a default.
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
            sqlx::query_as("SELECT id, name, path, default_branch, completion_workflow, archived_at, updated_at FROM projects WHERE id = ?")
                .bind(id)
                .fetch_one(&mut *transaction)
                .await?;
        transaction.commit().await?;
        Ok(project)
    }

    pub async fn projects(&self) -> Result<Vec<Project>, sqlx::Error> {
        sqlx::query_as(
            "SELECT id, name, path, default_branch, completion_workflow, archived_at, updated_at FROM projects ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn set_project_completion_workflow(
        &self,
        id: i64,
        completion_workflow: ProjectCompletionWorkflow,
    ) -> Result<Project, AppError> {
        sqlx::query_as(
            "UPDATE projects SET completion_workflow = ?, updated_at = unixepoch() WHERE id = ? \
             RETURNING id, name, path, default_branch, completion_workflow, archived_at, updated_at",
        )
        .bind(completion_workflow)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| AppError::not_found("project not found"))
    }

    pub async fn register_project(
        &self,
        name: &str,
        path: &str,
        default_branch: &str,
        completion_workflow: ProjectCompletionWorkflow,
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
        let id = sqlx::query("INSERT INTO projects (name, path, default_branch, completion_workflow) VALUES (?, ?, ?, ?)")
            .bind(name)
            .bind(path)
            .bind(default_branch)
            .bind(completion_workflow)
            .execute(&mut *transaction)
            .await?
            .last_insert_rowid();
        let project = sqlx::query_as(
            "SELECT id, name, path, default_branch, completion_workflow, archived_at, updated_at FROM projects WHERE id = ?",
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
            "SELECT id, name, path, default_branch, completion_workflow, archived_at, updated_at FROM projects WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&mut *transaction)
        .await?;
        let project = project.ok_or_else(|| AppError::not_found("project not found"))?;
        if project.archived_at.is_none() {
            let active_work: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM tickets WHERE project_id = ? AND state IN ('in_progress', 'review', 'awaiting_external_merge'))",
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
            sqlx::query("DELETE FROM agent_project_subscriptions WHERE project_id = ?")
                .bind(id)
                .execute(&mut *transaction)
                .await?;
            sqlx::query(
                "DELETE FROM agent_wake_events WHERE delivery_state = 'pending' \
                 AND kind IN ('ticket.available', 'ticket.review_requested') \
                 AND json_extract(payload, '$.ticket.project_id') = ?",
            )
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        }
        let archived = sqlx::query_as(
            "SELECT id, name, path, default_branch, completion_workflow, archived_at, updated_at FROM projects WHERE id = ?",
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
            "SELECT id, project_id, title, description, state, created_by_agent_id, reserved_for_agent_id, assignee_id, implementation_branch, review_commit, review_target_branch, review_project_id, review_project_path, reviewer_id, review_feedback, review_completion_workflow, external_change_url, external_merge_reference, external_merged_at, revision, created_at, updated_at \
             FROM tickets ORDER BY id",
        )
        .fetch_all(&mut *transaction)
        .await?;
        load_ticket_dependencies_batch(&mut transaction, &mut tickets).await?;
        transaction.commit().await?;
        Ok(tickets)
    }

    pub async fn ticket_summaries(
        &self,
        filter: TicketSummaryFilter,
    ) -> Result<(Vec<TicketSummary>, bool), AppError> {
        if filter.project_id.is_some_and(|id| id < 1)
            || filter.before_id.is_some_and(|id| id < 1)
            || filter.limit.is_some_and(|limit| limit < 1)
        {
            return Err(AppError::bad_request(
                "ticket page parameters must be positive",
            ));
        }
        let limit = filter
            .limit
            .unwrap_or(DEFAULT_TICKET_PAGE_SIZE)
            .min(MAX_TICKET_PAGE_SIZE);
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT id, project_id, title, state, created_by_agent_id, reserved_for_agent_id, assignee_id, reviewer_id, review_completion_workflow, review_feedback IS NOT NULL AS has_review_feedback, (SELECT COUNT(*) FROM ticket_activity WHERE ticket_id = tickets.id) AS activity_count, (SELECT MAX(id) FROM ticket_activity WHERE ticket_id = tickets.id) AS latest_activity_id, revision, created_at, updated_at FROM tickets WHERE 1 = 1",
        );
        if let Some(project_id) = filter.project_id {
            query.push(" AND project_id = ").push_bind(project_id);
        }
        if let Some(state) = filter.state {
            query.push(" AND state = ").push_bind(state);
        }
        if let Some(before_id) = filter.before_id {
            query.push(" AND id < ").push_bind(before_id);
        }
        query.push(" ORDER BY id DESC LIMIT ").push_bind(limit + 1);
        let mut summaries: Vec<TicketSummary> =
            query.build_query_as().fetch_all(&self.pool).await?;
        let has_more = summaries.len() > limit as usize;
        summaries.truncate(limit as usize);
        load_summary_dependencies(&self.pool, &mut summaries).await?;
        Ok((summaries, has_more))
    }

    pub async fn ticket(&self, id: i64) -> Result<Option<Ticket>, sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        let ticket: Option<Ticket> = sqlx::query_as(
            "SELECT id, project_id, title, description, state, created_by_agent_id, reserved_for_agent_id, assignee_id, implementation_branch, review_commit, review_target_branch, review_project_id, review_project_path, reviewer_id, review_feedback, review_completion_workflow, external_change_url, external_merge_reference, external_merged_at, revision, created_at, updated_at \
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
        let project_id = self
            .project()
            .await?
            .ok_or_else(|| AppError::bad_request("ticket project is required"))?
            .id;
        self.create_ticket_for_project_with_wakes(new, project_id)
            .await
    }

    pub async fn create_ticket_for_project_with_wakes(
        &self,
        new: NewTicket,
        project_id: i64,
    ) -> Result<(Ticket, Vec<i64>), AppError> {
        validate_title(&new.title)?;
        let project = self
            .project_by_id(project_id)
            .await?
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
    ) -> Result<(Ticket, Vec<i64>, Vec<TicketActivity>), AppError> {
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
        if current.state != state {
            match (current.state, state) {
                (
                    TicketState::Todo
                    | TicketState::InProgress
                    | TicketState::AwaitingExternalMerge,
                    TicketState::Cancelled,
                ) => {}
                (TicketState::Review, TicketState::Cancelled) => {
                    return Err(AppError::conflict(
                        "a ticket under review cannot be cancelled",
                    ));
                }
                (TicketState::Done, TicketState::Cancelled) => {
                    return Err(AppError::conflict("a completed ticket cannot be cancelled"));
                }
                (TicketState::Cancelled, _) => {
                    return Err(AppError::conflict("a cancelled ticket is terminal"));
                }
                (TicketState::AwaitingExternalMerge, _) => {
                    return Err(AppError::conflict(
                        "use an external merge operation to return or complete this ticket",
                    ));
                }
                (_, TicketState::AwaitingExternalMerge) => {
                    return Err(AppError::conflict(
                        "only external review approval can enter awaiting external merge",
                    ));
                }
                _ => {}
            }
        }
        let assignee_id = changes.assignee_id.unwrap_or(current.assignee_id);
        let reserved_for_agent_id = changes
            .reserved_for_agent_id
            .unwrap_or(current.reserved_for_agent_id);
        if reserved_for_agent_id.is_some()
            && assignee_id != current.assignee_id
            && assignee_id != reserved_for_agent_id
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
            "UPDATE tickets SET title = ?, description = ?, state = ?, assignee_id = ?, reserved_for_agent_id = ?, revision = revision + 1, \
             updated_at = unixepoch() WHERE id = ? AND revision = ?",
        )
        .bind(title.trim())
        .bind(description)
        .bind(state)
        .bind(assignee_id)
        .bind(reserved_for_agent_id)
        .bind(id)
        .bind(current.revision)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::conflict("ticket was changed by another user"));
        }
        replace_ticket_dependencies(&mut transaction, id, &blocker_ids).await?;
        let ticket = ticket_in_transaction(&mut transaction, id).await?;
        let mut activities = Vec::new();
        if current.reserved_for_agent_id.is_some() && ticket.reserved_for_agent_id.is_none() {
            let activity = sqlx::query_as(
                "INSERT INTO ticket_activity \
                 (ticket_id, action, previous_reserved_for_agent_id, reserved_for_agent_id) \
                 VALUES (?, 'reservation_cleared', ?, NULL) \
                 RETURNING id, ticket_id, action, detail, author, actor_agent_id, \
                 previous_reserved_for_agent_id, reserved_for_agent_id, implementation_branch, \
                 review_commit, review_target_branch, created_at",
            )
            .bind(ticket.id)
            .bind(current.reserved_for_agent_id)
            .fetch_one(&mut *transaction)
            .await?;
            activities.push(activity);
            sqlx::query(
                "DELETE FROM agent_wake_events WHERE delivery_state = 'pending' \
                 AND kind = 'ticket.reserved' \
                 AND json_extract(payload, '$.ticket_id') = ?",
            )
            .bind(ticket.id)
            .execute(&mut *transaction)
            .await?;
        }
        if current.state != TicketState::Cancelled && ticket.state == TicketState::Cancelled {
            let activity = sqlx::query_as(
                "INSERT INTO ticket_activity (ticket_id, action) VALUES (?, 'cancelled') \
                 RETURNING id, ticket_id, action, detail, author, actor_agent_id, \
                 previous_reserved_for_agent_id, reserved_for_agent_id, implementation_branch, \
                 review_commit, review_target_branch, created_at",
            )
            .bind(ticket.id)
            .fetch_one(&mut *transaction)
            .await?;
            activities.push(activity);
            sqlx::query(
                "DELETE FROM agent_wake_events WHERE delivery_state = 'pending' \
                 AND kind LIKE 'ticket.%' \
                 AND json_extract(payload, '$.ticket_id') = ?",
            )
            .bind(ticket.id)
            .execute(&mut *transaction)
            .await?;
        }
        if !ticket.unresolved_blockers.is_empty()
            && ((current.state == TicketState::Todo && ticket.state == TicketState::InProgress)
                || (current.assignee_id.is_none() && ticket.assignee_id.is_some()))
        {
            return Err(blocked_ticket_error(&ticket));
        }
        let reason = if ticket.state == TicketState::Cancelled {
            None
        } else if current.assignee_id != ticket.assignee_id && ticket.assignee_id.is_some() {
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
        Ok((ticket, wake_agent_ids, activities))
    }

    pub async fn reserve_tickets(
        &self,
        ticket_ids: &[i64],
        agent_id: i64,
        actor_agent_id: Option<i64>,
        administrative_override: bool,
    ) -> Result<Vec<Ticket>, AppError> {
        Ok(self
            .reserve_tickets_with_wakes(
                ticket_ids,
                agent_id,
                actor_agent_id,
                administrative_override,
            )
            .await?
            .0)
    }

    pub async fn reserve_tickets_with_wakes(
        &self,
        ticket_ids: &[i64],
        agent_id: i64,
        actor_agent_id: Option<i64>,
        administrative_override: bool,
    ) -> Result<(Vec<Ticket>, Vec<i64>, Vec<TicketActivity>), AppError> {
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
        let mut wake_agent_ids = HashSet::new();
        let mut activities = Vec::new();
        for ticket in tickets {
            let changed = ticket.reserved_for_agent_id != Some(agent_id);
            if changed {
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
                    let activity = sqlx::query_as(
                        "INSERT INTO ticket_activity \
                         (ticket_id, action, actor_agent_id, previous_reserved_for_agent_id, reserved_for_agent_id) \
                         VALUES (?, 'reservation_override', ?, ?, ?) \
                         RETURNING id, ticket_id, action, detail, author, actor_agent_id, \
                         previous_reserved_for_agent_id, reserved_for_agent_id, implementation_branch, \
                         review_commit, review_target_branch, created_at",
                    )
                    .bind(ticket.id)
                    .bind(actor_agent_id)
                    .bind(ticket.reserved_for_agent_id)
                    .bind(agent_id)
                    .fetch_one(&mut *transaction)
                    .await?;
                    activities.push(activity);
                }
            }
            let updated = ticket_in_transaction(&mut transaction, ticket.id).await?;
            if changed {
                wake_agent_ids.extend(
                    insert_pending_ticket_wakes(
                        &mut transaction,
                        Some(&ticket),
                        &updated,
                        TicketWakeReason::Reserved,
                    )
                    .await?,
                );
            }
            reserved.push(updated);
        }
        transaction.commit().await?;
        let mut wake_agent_ids = wake_agent_ids.into_iter().collect::<Vec<_>>();
        wake_agent_ids.sort_unstable();
        Ok((reserved, wake_agent_ids, activities))
    }

    pub async fn ticket_activity(&self, ticket_id: i64) -> Result<Vec<TicketActivity>, AppError> {
        if self.ticket(ticket_id).await?.is_none() {
            return Err(AppError::not_found("ticket not found"));
        }
        let mut entries: Vec<TicketActivity> = sqlx::query_as(
            "SELECT id, ticket_id, action, detail, author, actor_agent_id, previous_reserved_for_agent_id, \
             reserved_for_agent_id, implementation_branch, review_commit, review_target_branch, created_at FROM ticket_activity \
             WHERE ticket_id = ? ORDER BY created_at, id",
        )
        .bind(ticket_id)
        .fetch_all(&self.pool)
        .await?;
        for entry in &mut entries {
            entry.attachments = attachments_for(&self.pool, "ticket_activity", entry.id).await?;
        }
        Ok(entries)
    }

    pub async fn ticket_activity_page(
        &self,
        ticket_id: i64,
        before_id: Option<i64>,
        limit: Option<i64>,
    ) -> Result<(Vec<TicketActivity>, bool), AppError> {
        if self.ticket(ticket_id).await?.is_none() {
            return Err(AppError::not_found("ticket not found"));
        }
        if before_id.is_some_and(|id| id <= 0) {
            return Err(AppError::bad_request("activity cursor ID must be positive"));
        }
        let limit = limit.unwrap_or(DEFAULT_TICKET_ACTIVITY_PAGE_SIZE);
        if !(1..=MAX_TICKET_ACTIVITY_PAGE_SIZE).contains(&limit) {
            return Err(AppError::bad_request(format!(
                "activity page size must be between 1 and {MAX_TICKET_ACTIVITY_PAGE_SIZE}",
            )));
        }
        let mut entries: Vec<TicketActivity> = sqlx::query_as(
            "SELECT id, ticket_id, action, detail, author, actor_agent_id, previous_reserved_for_agent_id, \
             reserved_for_agent_id, implementation_branch, review_commit, review_target_branch, created_at FROM ticket_activity \
             WHERE ticket_id = ? AND (? IS NULL OR id < ?) ORDER BY id DESC LIMIT ?",
        )
        .bind(ticket_id)
        .bind(before_id)
        .bind(before_id)
        .bind(limit + 1)
        .fetch_all(&self.pool)
        .await?;
        let has_more = entries.len() > limit as usize;
        entries.truncate(limit as usize);
        entries.reverse();
        for entry in &mut entries {
            entry.attachments = attachments_for(&self.pool, "ticket_activity", entry.id).await?;
        }
        Ok((entries, has_more))
    }

    pub async fn add_ticket_comment(
        &self,
        ticket_id: i64,
        actor_agent_id: Option<i64>,
        body: &str,
    ) -> Result<(TicketActivity, Vec<i64>), AppError> {
        self.add_ticket_comment_with_attachments(ticket_id, actor_agent_id, body, &[])
            .await
    }

    pub async fn add_ticket_comment_with_attachments(
        &self,
        ticket_id: i64,
        actor_agent_id: Option<i64>,
        body: &str,
        attachment_ids: &[String],
    ) -> Result<(TicketActivity, Vec<i64>), AppError> {
        let body = body.trim();
        if (body.is_empty() && attachment_ids.is_empty()) || body.chars().count() > 10_000 {
            return Err(AppError::bad_request(
                "comment must contain between 1 and 10000 characters",
            ));
        }
        let mut transaction = self.pool.begin().await?;
        let ticket = ticket_in_transaction_optional(&mut transaction, ticket_id)
            .await?
            .ok_or_else(|| AppError::not_found("ticket not found"))?;
        let author = if let Some(agent_id) = actor_agent_id {
            sqlx::query_scalar("SELECT name FROM agents WHERE id = ?")
                .bind(agent_id)
                .fetch_optional(&mut *transaction)
                .await?
                .ok_or_else(|| AppError::not_found("agent not found"))?
        } else {
            "human".to_owned()
        };
        let mut activity: TicketActivity = sqlx::query_as(
            "INSERT INTO ticket_activity (ticket_id, action, detail, author, actor_agent_id) \
             VALUES (?, 'comment', ?, ?, ?) RETURNING id, ticket_id, action, detail, author, \
             actor_agent_id, previous_reserved_for_agent_id, reserved_for_agent_id, \
             implementation_branch, review_commit, review_target_branch, created_at",
        )
        .bind(ticket_id)
        .bind(body)
        .bind(&author)
        .bind(actor_agent_id)
        .fetch_one(&mut *transaction)
        .await?;
        activity.attachments = link_attachments(
            &mut transaction,
            attachment_ids,
            actor_agent_id.map_or(AttachmentOwner::Human, AttachmentOwner::Agent),
            "ticket_activity",
            activity.id,
        )
        .await?;
        let agents: Vec<(i64, String)> = sqlx::query_as("SELECT id, name FROM agents ORDER BY id")
            .fetch_all(&mut *transaction)
            .await?;
        let previous_commenters: Vec<i64> = sqlx::query_scalar(
            "SELECT DISTINCT actor_agent_id FROM ticket_activity WHERE ticket_id = ? \
             AND action = 'comment' AND actor_agent_id IS NOT NULL",
        )
        .bind(ticket_id)
        .fetch_all(&mut *transaction)
        .await?;
        let mut recipients = previous_commenters.into_iter().collect::<HashSet<_>>();
        recipients.extend(ticket.assignee_id);
        recipients.extend(ticket.reviewer_id);
        for (agent_id, name) in &agents {
            if mentions_agent(body, name) {
                recipients.insert(*agent_id);
            }
        }
        if let Some(author_id) = actor_agent_id {
            recipients.remove(&author_id);
        }
        let payload = serde_json::to_string(&serde_json::json!({
            "message": format!("A comment was added to ticket #{} '{}':\n{}", ticket.id, ticket.title, body)
        }))
        .map_err(|error| AppError::internal(format!("failed to encode comment wake: {error}")))?;
        let mut recipients = recipients.into_iter().collect::<Vec<_>>();
        recipients.sort_unstable();
        for agent_id in &recipients {
            sqlx::query(
                "INSERT OR IGNORE INTO agent_wake_events \
                 (agent_id, kind, payload, deduplication_key, delivery_policy) \
                 VALUES (?, 'notification', ?, ?, 'immediate')",
            )
            .bind(agent_id)
            .bind(&payload)
            .bind(format!("ticket-comment:{}:{}", ticket_id, activity.id))
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok((activity, recipients))
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
             AND state = 'todo' \
             AND (reserved_for_agent_id IS NULL OR reserved_for_agent_id = ?) \
             AND NOT EXISTS (SELECT 1 FROM ticket_dependencies d JOIN tickets blocker ON blocker.id = d.blocker_id \
             WHERE d.ticket_id = tickets.id AND blocker.state != 'done') \
             AND (? IS NULL OR revision = ?) \
             RETURNING id, project_id, title, description, state, created_by_agent_id, reserved_for_agent_id, assignee_id, implementation_branch, review_commit, review_target_branch, review_project_id, review_project_path, reviewer_id, review_feedback, review_completion_workflow, external_change_url, external_merge_reference, external_merged_at, revision, created_at, updated_at",
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
    ) -> Result<(Ticket, Vec<i64>, TicketActivity), AppError> {
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
            "UPDATE tickets SET state = 'review', implementation_branch = ?, review_commit = ?, review_target_branch = ?, review_project_id = ?, review_project_path = ?, review_completion_workflow = ?, reviewer_id = NULL, \
             revision = revision + 1, updated_at = unixepoch() \
             WHERE id = ? AND state = 'in_progress' AND assignee_id = ? \
             AND project_id = ? \
             AND (? IS NULL OR revision = ?) \
             RETURNING id, project_id, title, description, state, created_by_agent_id, reserved_for_agent_id, assignee_id, implementation_branch, review_commit, review_target_branch, review_project_id, review_project_path, reviewer_id, review_feedback, review_completion_workflow, external_change_url, external_merge_reference, external_merged_at, revision, created_at, updated_at",
        )
        .bind(submission.implementation_branch)
        .bind(submission.commit)
        .bind(submission.target_branch)
        .bind(submission.project_id)
        .bind(submission.project_path)
        .bind(project.completion_workflow)
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
        let activity = sqlx::query_as(
            "INSERT INTO ticket_activity (ticket_id, action, actor_agent_id, implementation_branch, review_commit, review_target_branch) \
             VALUES (?, 'review_submitted', ?, ?, ?, ?) \
             RETURNING id, ticket_id, action, detail, author, actor_agent_id, \
             previous_reserved_for_agent_id, reserved_for_agent_id, implementation_branch, \
             review_commit, review_target_branch, created_at",
        )
        .bind(id)
        .bind(agent_id)
        .bind(&ticket.implementation_branch)
        .bind(&ticket.review_commit)
        .bind(&ticket.review_target_branch)
        .fetch_one(&mut *transaction)
        .await?;
        let wake_agent_ids = insert_pending_ticket_wakes(
            &mut transaction,
            Some(&current),
            &ticket,
            TicketWakeReason::ReviewRequested,
        )
        .await?;
        transaction.commit().await?;
        Ok((ticket, wake_agent_ids, activity))
    }

    pub async fn claim_review(
        &self,
        id: i64,
        reviewer_id: i64,
        expected_revision: Option<i64>,
    ) -> Result<Ticket, AppError> {
        Ok(self
            .claim_review_with_activity(id, reviewer_id, expected_revision)
            .await?
            .0)
    }

    pub(crate) async fn claim_review_with_activity(
        &self,
        id: i64,
        reviewer_id: i64,
        expected_revision: Option<i64>,
    ) -> Result<(Ticket, TicketActivity), AppError> {
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
             RETURNING id, project_id, title, description, state, created_by_agent_id, reserved_for_agent_id, assignee_id, implementation_branch, review_commit, review_target_branch, review_project_id, review_project_path, reviewer_id, review_feedback, review_completion_workflow, external_change_url, external_merge_reference, external_merged_at, revision, created_at, updated_at",
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
        let activity = sqlx::query_as(
            "INSERT INTO ticket_activity (ticket_id, action, actor_agent_id, implementation_branch, review_commit, review_target_branch) \
             VALUES (?, 'review_claimed', ?, ?, ?, ?) \
             RETURNING id, ticket_id, action, detail, author, actor_agent_id, \
             previous_reserved_for_agent_id, reserved_for_agent_id, implementation_branch, \
             review_commit, review_target_branch, created_at",
        )
        .bind(id)
        .bind(reviewer_id)
        .bind(&ticket.implementation_branch)
        .bind(&ticket.review_commit)
        .bind(&ticket.review_target_branch)
        .fetch_one(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok((ticket, activity))
    }

    pub async fn request_changes(
        &self,
        id: i64,
        reviewer_id: i64,
        feedback: &str,
        expected_revision: Option<i64>,
    ) -> Result<Ticket, AppError> {
        Ok(self
            .request_changes_with_activity(id, reviewer_id, feedback, expected_revision)
            .await?
            .0)
    }

    pub(crate) async fn request_changes_with_activity(
        &self,
        id: i64,
        reviewer_id: i64,
        feedback: &str,
        expected_revision: Option<i64>,
    ) -> Result<(Ticket, TicketActivity), AppError> {
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
             RETURNING id, project_id, title, description, state, created_by_agent_id, reserved_for_agent_id, assignee_id, implementation_branch, review_commit, review_target_branch, review_project_id, review_project_path, reviewer_id, review_feedback, review_completion_workflow, external_change_url, external_merge_reference, external_merged_at, revision, created_at, updated_at",
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
        let activity = sqlx::query_as(
            "INSERT INTO ticket_activity (ticket_id, action, detail, actor_agent_id, implementation_branch, review_commit, review_target_branch) \
             VALUES (?, 'changes_requested', ?, ?, ?, ?, ?) \
             RETURNING id, ticket_id, action, detail, author, actor_agent_id, \
             previous_reserved_for_agent_id, reserved_for_agent_id, implementation_branch, \
             review_commit, review_target_branch, created_at",
        )
        .bind(id)
        .bind(feedback.trim())
        .bind(reviewer_id)
        .bind(&ticket.implementation_branch)
        .bind(&ticket.review_commit)
        .bind(&ticket.review_target_branch)
        .fetch_one(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok((ticket, activity))
    }

    pub async fn approve_external_review(
        &self,
        id: i64,
        reviewer_id: i64,
        commit: &str,
        expected_revision: Option<i64>,
    ) -> Result<Ticket, AppError> {
        Ok(self
            .approve_external_review_with_wakes(id, reviewer_id, commit, expected_revision, None)
            .await?
            .0)
    }

    pub(crate) async fn approve_external_review_with_wakes(
        &self,
        id: i64,
        reviewer_id: i64,
        commit: &str,
        expected_revision: Option<i64>,
        notification: Option<MergeNotification<'_>>,
    ) -> Result<(Ticket, Option<Message>, Vec<i64>, TicketActivity), AppError> {
        if !self.has_agent_scope(reviewer_id, "approve_review").await? {
            return Err(AppError::forbidden("agent lacks approve_review scope"));
        }
        let current = self
            .ticket(id)
            .await?
            .ok_or_else(|| AppError::not_found("ticket not found"))?;
        self.review_ticket_project(&current).await?;
        let mut transaction = self.pool.begin().await?;
        let mut ticket: Ticket = sqlx::query_as(
            "UPDATE tickets SET state = 'awaiting_external_merge', revision = revision + 1, updated_at = unixepoch() \
             WHERE id = ? AND state = 'review' AND reviewer_id = ? AND review_commit = ? \
             AND review_completion_workflow = 'external' \
             AND review_project_id = project_id AND review_project_path = \
                 (SELECT path FROM projects WHERE id = tickets.project_id AND archived_at IS NULL) \
             AND (? IS NULL OR revision = ?) \
             RETURNING id, project_id, title, description, state, created_by_agent_id, reserved_for_agent_id, assignee_id, implementation_branch, review_commit, review_target_branch, review_project_id, review_project_path, reviewer_id, review_feedback, review_completion_workflow, external_change_url, external_merge_reference, external_merged_at, revision, created_at, updated_at",
        )
        .bind(id)
        .bind(reviewer_id)
        .bind(commit)
        .bind(expected_revision)
        .bind(expected_revision)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| {
            AppError::conflict("external review is not assigned to this agent or has changed")
        })?;
        let activity = sqlx::query_as(
            "INSERT INTO ticket_activity (ticket_id, action, actor_agent_id, implementation_branch, review_commit, review_target_branch) \
             VALUES (?, 'external_review_approved', ?, ?, ?, ?) \
             RETURNING id, ticket_id, action, detail, author, actor_agent_id, \
             previous_reserved_for_agent_id, reserved_for_agent_id, implementation_branch, \
             review_commit, review_target_branch, created_at",
        )
        .bind(id)
        .bind(reviewer_id)
        .bind(&ticket.implementation_branch)
        .bind(&ticket.review_commit)
        .bind(&ticket.review_target_branch)
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query(
            "DELETE FROM agent_wake_events WHERE delivery_state = 'pending' \
             AND kind = 'ticket.review_requested' \
             AND json_extract(payload, '$.ticket_id') = ?",
        )
        .bind(id)
        .execute(&mut *transaction)
        .await?;
        load_ticket_dependencies(&mut transaction, &mut ticket).await?;
        let mut wake_agent_ids = insert_pending_ticket_wakes(
            &mut transaction,
            Some(&current),
            &ticket,
            TicketWakeReason::ExternalMergeReady,
        )
        .await?;
        let message = if let Some(notification) = notification {
            let (message, message_targets) = insert_message(
                &mut transaction,
                MessageWrite {
                    channel: notification.channel,
                    agent_id: None,
                    author: notification.author,
                    body: notification.body,
                    deduplication_key: Some(notification.deduplication_key),
                    attachment_ids: &[],
                },
            )
            .await?;
            wake_agent_ids.extend(message_targets);
            Some(message)
        } else {
            None
        };
        wake_agent_ids.sort_unstable();
        wake_agent_ids.dedup();
        transaction.commit().await?;
        Ok((ticket, message, wake_agent_ids, activity))
    }

    pub async fn set_external_change_url(
        &self,
        id: i64,
        external_change_url: &str,
        actor_agent_id: Option<i64>,
        expected_revision: Option<i64>,
    ) -> Result<Ticket, AppError> {
        Ok(self
            .set_external_change_url_with_activity(
                id,
                external_change_url,
                actor_agent_id,
                expected_revision,
            )
            .await?
            .0)
    }

    pub(crate) async fn set_external_change_url_with_activity(
        &self,
        id: i64,
        external_change_url: &str,
        actor_agent_id: Option<i64>,
        expected_revision: Option<i64>,
    ) -> Result<(Ticket, TicketActivity), AppError> {
        let url = external_change_url.trim();
        if url.is_empty() || url.chars().count() > 2_048 {
            return Err(AppError::bad_request(
                "external change URL must contain between 1 and 2048 characters",
            ));
        }
        self.update_external_merge_ticket(
            id,
            expected_revision,
            "external_change_url = ?",
            url,
            "external_change_url_set",
            actor_agent_id,
        )
        .await
    }

    async fn update_external_merge_ticket(
        &self,
        id: i64,
        expected_revision: Option<i64>,
        assignment: &str,
        value: &str,
        action: &str,
        actor_agent_id: Option<i64>,
    ) -> Result<(Ticket, TicketActivity), AppError> {
        let current = self
            .ticket(id)
            .await?
            .ok_or_else(|| AppError::not_found("ticket not found"))?;
        if let Some(agent_id) = actor_agent_id
            && current.assignee_id != Some(agent_id)
            && current.reviewer_id != Some(agent_id)
        {
            return Err(AppError::forbidden(
                "agent is not involved in this external change",
            ));
        }
        self.active_ticket_project(&current).await?;
        if current.review_completion_workflow != Some(ProjectCompletionWorkflow::External)
            || !matches!(
                current.state,
                TicketState::Review | TicketState::AwaitingExternalMerge
            )
        {
            return Err(AppError::conflict(
                "ticket is not in the external completion workflow",
            ));
        }
        let sql = format!(
            "UPDATE tickets SET {assignment}, revision = revision + 1, updated_at = unixepoch() \
             WHERE id = ? AND revision = ?"
        );
        let mut transaction = self.pool.begin().await?;
        let result = sqlx::query(&sql)
            .bind(value)
            .bind(id)
            .bind(expected_revision.unwrap_or(current.revision))
            .execute(&mut *transaction)
            .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::conflict("ticket was changed by another user"));
        }
        let activity = sqlx::query_as(
            "INSERT INTO ticket_activity (ticket_id, action, detail, actor_agent_id, implementation_branch, review_commit, review_target_branch) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             RETURNING id, ticket_id, action, detail, author, actor_agent_id, \
             previous_reserved_for_agent_id, reserved_for_agent_id, implementation_branch, \
             review_commit, review_target_branch, created_at",
        )
        .bind(id)
        .bind(action)
        .bind(value)
        .bind(actor_agent_id)
        .bind(&current.implementation_branch)
        .bind(&current.review_commit)
        .bind(&current.review_target_branch)
        .fetch_one(&mut *transaction)
        .await?;
        let ticket = ticket_in_transaction(&mut transaction, id).await?;
        transaction.commit().await?;
        Ok((ticket, activity))
    }

    pub async fn return_external_merge(
        &self,
        id: i64,
        reason: &str,
        actor_agent_id: Option<i64>,
        expected_revision: Option<i64>,
    ) -> Result<Ticket, AppError> {
        Ok(self
            .return_external_merge_with_activity(id, reason, actor_agent_id, expected_revision)
            .await?
            .0)
    }

    pub(crate) async fn return_external_merge_with_activity(
        &self,
        id: i64,
        reason: &str,
        actor_agent_id: Option<i64>,
        expected_revision: Option<i64>,
    ) -> Result<(Ticket, TicketActivity), AppError> {
        let reason = reason.trim();
        if reason.is_empty() || reason.chars().count() > 10_000 {
            return Err(AppError::bad_request(
                "return reason must contain between 1 and 10000 characters",
            ));
        }
        let current = self
            .ticket(id)
            .await?
            .ok_or_else(|| AppError::not_found("ticket not found"))?;
        self.require_external_lifecycle_authority(actor_agent_id, &current)
            .await?;
        self.active_ticket_project(&current).await?;
        let mut transaction = self.pool.begin().await?;
        let result = sqlx::query(
            "UPDATE tickets SET state = 'in_progress', review_feedback = ?, revision = revision + 1, updated_at = unixepoch() \
             WHERE id = ? AND state = 'awaiting_external_merge' AND review_completion_workflow = 'external' \
             AND revision = ?",
        )
        .bind(reason)
        .bind(id)
        .bind(expected_revision.unwrap_or(current.revision))
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::conflict(
                "ticket is not awaiting an external merge or has changed",
            ));
        }
        let activity = sqlx::query_as(
            "INSERT INTO ticket_activity (ticket_id, action, detail, actor_agent_id, implementation_branch, review_commit, review_target_branch) \
             VALUES (?, 'external_merge_returned', ?, ?, ?, ?, ?) \
             RETURNING id, ticket_id, action, detail, author, actor_agent_id, \
             previous_reserved_for_agent_id, reserved_for_agent_id, implementation_branch, \
             review_commit, review_target_branch, created_at",
        )
        .bind(id)
        .bind(reason)
        .bind(actor_agent_id)
        .bind(&current.implementation_branch)
        .bind(&current.review_commit)
        .bind(&current.review_target_branch)
        .fetch_one(&mut *transaction)
        .await?;
        let ticket = ticket_in_transaction(&mut transaction, id).await?;
        transaction.commit().await?;
        Ok((ticket, activity))
    }

    pub async fn complete_external_merge(
        &self,
        id: i64,
        confirmation: &str,
        merge_reference: Option<&str>,
        actor_agent_id: Option<i64>,
        expected_revision: Option<i64>,
    ) -> Result<Ticket, AppError> {
        Ok(self
            .complete_external_merge_with_message(
                id,
                confirmation,
                merge_reference,
                actor_agent_id,
                expected_revision,
                None,
            )
            .await?
            .0)
    }

    pub(crate) async fn complete_external_merge_with_message(
        &self,
        id: i64,
        confirmation: &str,
        merge_reference: Option<&str>,
        actor_agent_id: Option<i64>,
        expected_revision: Option<i64>,
        notification: Option<MergeNotification<'_>>,
    ) -> Result<(Ticket, Option<Message>, Vec<i64>, TicketActivity), AppError> {
        let confirmation = confirmation.trim();
        if confirmation.is_empty() || confirmation.chars().count() > 10_000 {
            return Err(AppError::bad_request(
                "confirmation must contain between 1 and 10000 characters",
            ));
        }
        let merge_reference = merge_reference
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if merge_reference.is_some_and(|value| value.chars().count() > 2_048) {
            return Err(AppError::bad_request(
                "external merge reference cannot exceed 2048 characters",
            ));
        }
        let current = self
            .ticket(id)
            .await?
            .ok_or_else(|| AppError::not_found("ticket not found"))?;
        self.require_external_lifecycle_authority(actor_agent_id, &current)
            .await?;
        self.active_ticket_project(&current).await?;
        let mut transaction = self.pool.begin().await?;
        let result = sqlx::query(
            "UPDATE tickets SET state = 'done', external_merge_reference = ?, external_merged_at = unixepoch(), \
             revision = revision + 1, updated_at = unixepoch() WHERE id = ? \
             AND state = 'awaiting_external_merge' AND review_completion_workflow = 'external' AND revision = ?",
        )
        .bind(merge_reference)
        .bind(id)
        .bind(expected_revision.unwrap_or(current.revision))
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() == 0 {
            return Err(AppError::conflict(
                "ticket is not awaiting an external merge or has changed",
            ));
        }
        let activity = sqlx::query_as(
            "INSERT INTO ticket_activity (ticket_id, action, detail, actor_agent_id, implementation_branch, review_commit, review_target_branch) \
             VALUES (?, 'external_merge_completed', ?, ?, ?, ?, ?) \
             RETURNING id, ticket_id, action, detail, author, actor_agent_id, \
             previous_reserved_for_agent_id, reserved_for_agent_id, implementation_branch, \
             review_commit, review_target_branch, created_at",
        )
        .bind(id)
        .bind(confirmation)
        .bind(actor_agent_id)
        .bind(&current.implementation_branch)
        .bind(&current.review_commit)
        .bind(&current.review_target_branch)
        .fetch_one(&mut *transaction)
        .await?;
        let ticket = ticket_in_transaction(&mut transaction, id).await?;
        let (message, wake_agent_ids) = if let Some(notification) = notification {
            let (message, wake_agent_ids) = insert_message(
                &mut transaction,
                MessageWrite {
                    channel: notification.channel,
                    agent_id: None,
                    author: notification.author,
                    body: notification.body,
                    deduplication_key: Some(notification.deduplication_key),
                    attachment_ids: &[],
                },
            )
            .await?;
            (Some(message), wake_agent_ids)
        } else {
            (None, Vec::new())
        };
        transaction.commit().await?;
        Ok((ticket, message, wake_agent_ids, activity))
    }

    async fn require_external_lifecycle_authority(
        &self,
        actor_agent_id: Option<i64>,
        ticket: &Ticket,
    ) -> Result<(), AppError> {
        if let Some(agent_id) = actor_agent_id
            && (!self.has_agent_scope(agent_id, "approve_review").await?
                || ticket.reviewer_id != Some(agent_id))
        {
            return Err(AppError::forbidden(
                "agent is not the scoped reviewer for this external lifecycle",
            ));
        }
        Ok(())
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

    pub(crate) async fn complete_review_with_activity(
        &self,
        id: i64,
        reviewer_id: i64,
        commit: &str,
        expected_revision: Option<i64>,
    ) -> Result<(Ticket, TicketActivity), AppError> {
        let (ticket, _message, activity) = self
            .complete_review_with_message(id, reviewer_id, commit, expected_revision, None)
            .await?;
        Ok((ticket, activity))
    }

    pub(crate) async fn complete_review_and_post_message(
        &self,
        id: i64,
        reviewer_id: i64,
        commit: &str,
        expected_revision: Option<i64>,
        notification: MergeNotification<'_>,
    ) -> Result<(Ticket, Message, Vec<i64>, TicketActivity), AppError> {
        let (ticket, message, activity) = self
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
                    attachment_ids: &[],
                }),
            )
            .await?;
        let (message, wake_agent_ids) = message.expect("notification was requested");
        Ok((ticket, message, wake_agent_ids, activity))
    }

    async fn complete_review_with_message(
        &self,
        id: i64,
        reviewer_id: i64,
        commit: &str,
        expected_revision: Option<i64>,
        message: Option<MessageWrite<'_>>,
    ) -> Result<(Ticket, Option<(Message, Vec<i64>)>, TicketActivity), AppError> {
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
             AND (review_completion_workflow IS NULL OR review_completion_workflow = 'board_managed') \
             AND review_project_id = project_id AND review_project_path = \
                 (SELECT path FROM projects WHERE id = tickets.project_id AND archived_at IS NULL) \
             AND (? IS NULL OR revision = ?) \
             RETURNING id, project_id, title, description, state, created_by_agent_id, reserved_for_agent_id, assignee_id, implementation_branch, review_commit, review_target_branch, review_project_id, review_project_path, reviewer_id, review_feedback, review_completion_workflow, external_change_url, external_merge_reference, external_merged_at, revision, created_at, updated_at",
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
        let activity = sqlx::query_as(
            "INSERT INTO ticket_activity (ticket_id, action, actor_agent_id, implementation_branch, review_commit, review_target_branch) \
             VALUES (?, 'review_approved', ?, ?, ?, ?) \
             RETURNING id, ticket_id, action, detail, author, actor_agent_id, \
             previous_reserved_for_agent_id, reserved_for_agent_id, implementation_branch, \
             review_commit, review_target_branch, created_at",
        )
        .bind(id)
        .bind(reviewer_id)
        .bind(&ticket.implementation_branch)
        .bind(&ticket.review_commit)
        .bind(&ticket.review_target_branch)
        .fetch_one(&mut *transaction)
        .await?;
        let message = match message {
            Some(message) => Some(insert_message(&mut transaction, message).await?),
            None => None,
        };
        transaction.commit().await?;
        Ok((ticket, message, activity))
    }
}

async fn ticket_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    id: i64,
) -> Result<Ticket, AppError> {
    ticket_in_transaction_optional(transaction, id)
        .await?
        .ok_or_else(|| sqlx::Error::RowNotFound.into())
}

async fn ticket_in_transaction_optional(
    transaction: &mut Transaction<'_, Sqlite>,
    id: i64,
) -> Result<Option<Ticket>, AppError> {
    let ticket = sqlx::query_as(
        "SELECT id, project_id, title, description, state, created_by_agent_id, reserved_for_agent_id, assignee_id, implementation_branch, review_commit, \
         review_target_branch, review_project_id, review_project_path, reviewer_id, review_feedback, review_completion_workflow, external_change_url, external_merge_reference, external_merged_at, revision, created_at, updated_at \
         FROM tickets WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut **transaction)
    .await?;
    let Some(mut ticket) = ticket else {
        return Ok(None);
    };
    load_ticket_dependencies(transaction, &mut ticket).await?;
    Ok(Some(ticket))
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

async fn load_ticket_dependencies_batch(
    transaction: &mut Transaction<'_, Sqlite>,
    tickets: &mut [Ticket],
) -> Result<(), sqlx::Error> {
    if tickets.is_empty() {
        return Ok(());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT dependency.ticket_id, blocker.id, blocker.title, blocker.state FROM ticket_dependencies dependency JOIN tickets blocker ON blocker.id = dependency.blocker_id WHERE dependency.ticket_id IN (",
    );
    let mut separated = query.separated(", ");
    for ticket in tickets.iter() {
        separated.push_bind(ticket.id);
    }
    separated.push_unseparated(") ORDER BY dependency.ticket_id, blocker.id");
    let blockers: Vec<(i64, i64, String, TicketState)> =
        query.build_query_as().fetch_all(&mut **transaction).await?;
    let by_id: HashMap<_, _> = tickets
        .iter()
        .enumerate()
        .map(|(index, ticket)| (ticket.id, index))
        .collect();
    for (ticket_id, blocker_id, title, state) in blockers {
        let ticket = &mut tickets[*by_id.get(&ticket_id).expect("queried ticket exists")];
        ticket.blocker_ids.push(blocker_id);
        if state != TicketState::Done {
            ticket.unresolved_blockers.push(TicketBlocker {
                id: blocker_id,
                title,
            });
        }
    }
    Ok(())
}

async fn load_summary_dependencies(
    pool: &sqlx::SqlitePool,
    summaries: &mut [TicketSummary],
) -> Result<(), sqlx::Error> {
    if summaries.is_empty() {
        return Ok(());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT dependency.ticket_id, blocker.id, blocker.title FROM ticket_dependencies dependency JOIN tickets blocker ON blocker.id = dependency.blocker_id WHERE blocker.state != 'done' AND dependency.ticket_id IN (",
    );
    let mut separated = query.separated(", ");
    for summary in summaries.iter() {
        separated.push_bind(summary.id);
    }
    separated.push_unseparated(") ORDER BY dependency.ticket_id, blocker.id");
    let blockers: Vec<(i64, i64, String)> = query.build_query_as().fetch_all(pool).await?;
    let by_id: HashMap<_, _> = summaries
        .iter()
        .enumerate()
        .map(|(index, ticket)| (ticket.id, index))
        .collect();
    for (ticket_id, id, title) in blockers {
        summaries[*by_id.get(&ticket_id).expect("queried ticket exists")]
            .unresolved_blockers
            .push(TicketBlocker { id, title });
    }
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
            "review_completion_workflow",
            "external_change_url",
            "external_merge_reference",
            "external_merged_at",
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
    if previous.review_completion_workflow != ticket.review_completion_workflow {
        fields.push("review_completion_workflow");
    }
    if previous.external_change_url != ticket.external_change_url {
        fields.push("external_change_url");
    }
    if previous.external_merge_reference != ticket.external_merge_reference {
        fields.push("external_merge_reference");
    }
    if previous.external_merged_at != ticket.external_merged_at {
        fields.push("external_merged_at");
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
    let (message, agent_ids) = match reason {
        TicketWakeReason::Available => (
            format!(
                "Ticket #{} '{}' is available. Claim it atomically before starting work.",
                ticket.id, ticket.title
            ),
            sqlx::query_scalar(
                "SELECT a.id FROM agents a JOIN agent_project_subscriptions subscription \
                 ON subscription.agent_id = a.id WHERE a.wake_on_pending_work = 1 \
                 AND subscription.project_id = ? ORDER BY a.id",
            )
            .bind(ticket.project_id)
            .fetch_all(&mut **transaction)
            .await?,
        ),
        TicketWakeReason::Assigned => (
            format!(
                "Ticket #{} '{}' was assigned to you.",
                ticket.id, ticket.title
            ),
            sqlx::query_scalar("SELECT id FROM agents WHERE id = ? AND wake_on_pending_work = 1")
                .bind(ticket.assignee_id)
                .fetch_all(&mut **transaction)
                .await?,
        ),
        TicketWakeReason::Reserved => (
            format!(
                "Ticket #{} '{}' was reserved for you.",
                ticket.id, ticket.title
            ),
            sqlx::query_scalar("SELECT id FROM agents WHERE id = ? AND wake_on_pending_work = 1")
                .bind(ticket.reserved_for_agent_id)
                .fetch_all(&mut **transaction)
                .await?,
        ),
        TicketWakeReason::ReviewRequested => (
            format!(
                "Ticket #{} '{}' from branch '{}' is ready for review. Claim its review before inspecting it.",
                ticket.id,
                ticket.title,
                ticket.implementation_branch.as_deref().unwrap_or("unknown")
            ),
            sqlx::query_scalar(
                "SELECT a.id FROM agents a JOIN agent_scopes s ON s.agent_id = a.id \
                 JOIN agent_project_subscriptions subscription ON subscription.agent_id = a.id \
                 WHERE a.wake_on_pending_review = 1 AND s.scope = 'approve_review' \
                 AND subscription.project_id = ? AND a.id != ? ORDER BY a.id",
            )
            .bind(ticket.project_id)
            .bind(ticket.assignee_id)
            .fetch_all(&mut **transaction)
            .await?,
        ),
        TicketWakeReason::ExternalMergeReady => (
            format!(
                "Ticket #{} '{}' was approved for external merge. The board did not merge Git. Follow the external change through completion.",
                ticket.id, ticket.title
            ),
            sqlx::query_scalar("SELECT id FROM agents WHERE id = ?")
                .bind(ticket.assignee_id)
                .fetch_all(&mut **transaction)
                .await?,
        ),
    };
    let payload = TicketWakePayload {
        message,
        ticket_id: ticket.id,
        revision: Some(ticket.revision),
        changed_fields: ticket_changed_fields(previous, ticket)
            .into_iter()
            .map(str::to_owned)
            .collect(),
        ticket: TicketWakeTicket::from(ticket),
    };
    let event = match reason {
        TicketWakeReason::Available => WakeEventData::TicketAvailable(payload),
        TicketWakeReason::Assigned => WakeEventData::TicketAssigned(payload),
        TicketWakeReason::Reserved => WakeEventData::TicketReserved(payload),
        TicketWakeReason::ReviewRequested => WakeEventData::TicketReviewRequested(payload),
        TicketWakeReason::ExternalMergeReady => WakeEventData::TicketExternalMergeReady(payload),
    };
    let payload = serde_json::to_string(&event.payload()).map_err(|error| {
        AppError::internal(format!("failed to encode ticket wake event: {error}"))
    })?;
    let deduplication_key = format!("ticket:{}:{}:{}", ticket.id, ticket.revision, event.kind());
    let mut queued = Vec::new();
    for agent_id in agent_ids {
        let result = sqlx::query(
            "INSERT OR IGNORE INTO agent_wake_events \
             (agent_id, kind, payload, deduplication_key, delivery_policy) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(agent_id)
        .bind(event.kind())
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

fn mentions_agent(body: &str, name: &str) -> bool {
    let mention = format!("@{name}");
    body.split(|character: char| {
        !(character.is_ascii_alphanumeric() || matches!(character, '@' | '-' | '_'))
    })
    .any(|token| token == mention)
}

#[cfg(test)]
mod tests {
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    use sqlx::SqlitePool;

    use super::*;

    #[tokio::test]
    async fn comments_preserve_revision_notify_participants_and_paginate() {
        let store = Store::memory().await.unwrap();
        let author = store.create_agent("author", "/tmp/author").await.unwrap();
        let assignee = store
            .create_agent("assignee", "/tmp/assignee")
            .await
            .unwrap();
        let reviewer = store
            .create_agent("reviewer", "/tmp/reviewer")
            .await
            .unwrap();
        let prior = store.create_agent("prior", "/tmp/prior").await.unwrap();
        let mentioned = store
            .create_agent("mentioned", "/tmp/mentioned")
            .await
            .unwrap();
        let unrelated = store
            .create_agent("unrelated", "/tmp/unrelated")
            .await
            .unwrap();
        let ticket = store
            .create_ticket(NewTicket {
                title: "Discuss safely".into(),
                description: String::new(),
                blocker_ids: Vec::new(),
                created_by_agent_id: None,
            })
            .await
            .unwrap();
        sqlx::query("UPDATE tickets SET assignee_id = ?, reviewer_id = ? WHERE id = ?")
            .bind(assignee.id)
            .bind(reviewer.id)
            .bind(ticket.id)
            .execute(&store.pool)
            .await
            .unwrap();
        let (first, _) = store
            .add_ticket_comment(ticket.id, Some(prior.id), "First")
            .await
            .unwrap();
        sqlx::query("DELETE FROM agent_wake_events")
            .execute(&store.pool)
            .await
            .unwrap();

        let (second, recipients) = store
            .add_ticket_comment(
                ticket.id,
                Some(author.id),
                "Please verify this, @mentioned.",
            )
            .await
            .unwrap();

        assert_eq!(
            recipients,
            vec![assignee.id, reviewer.id, prior.id, mentioned.id]
        );
        assert!(!recipients.contains(&author.id));
        assert!(!recipients.contains(&unrelated.id));
        assert_eq!(
            store.ticket(ticket.id).await.unwrap().unwrap().revision,
            ticket.revision
        );
        let (newest, has_more) = store
            .ticket_activity_page(ticket.id, None, Some(1))
            .await
            .unwrap();
        assert!(has_more);
        assert_eq!(newest[0], second);
        let (older, has_more) = store
            .ticket_activity_page(ticket.id, Some(second.id), Some(1))
            .await
            .unwrap();
        assert!(!has_more);
        assert_eq!(older[0], first);
    }

    #[tokio::test]
    async fn review_rounds_keep_each_feedback_and_submission_snapshot() {
        let store = Store::memory().await.unwrap();
        let project = store.project().await.unwrap().unwrap();
        let worker = store.create_agent("worker", "/tmp/worker").await.unwrap();
        let reviewer = store
            .create_agent("reviewer", "/tmp/reviewer")
            .await
            .unwrap();
        store
            .set_agent_scopes(reviewer.id, &["approve_review".into()])
            .await
            .unwrap();
        let mut ticket = store
            .create_ticket(NewTicket {
                title: "Iterate review".into(),
                description: String::new(),
                blocker_ids: Vec::new(),
                created_by_agent_id: None,
            })
            .await
            .unwrap();
        ticket = store
            .claim_ticket(ticket.id, worker.id, Some(ticket.revision))
            .await
            .unwrap();
        for (commit, feedback) in [("first", "First feedback"), ("second", "Second feedback")] {
            ticket = store
                .submit_review(
                    ticket.id,
                    worker.id,
                    ReviewSubmission {
                        implementation_branch: Some("tickets/iterate"),
                        commit,
                        target_branch: "main",
                        project_id: project.id,
                        project_path: &project.path,
                    },
                    Some(ticket.revision),
                )
                .await
                .unwrap();
            ticket = store
                .claim_review(ticket.id, reviewer.id, Some(ticket.revision))
                .await
                .unwrap();
            ticket = store
                .request_changes(ticket.id, reviewer.id, feedback, Some(ticket.revision))
                .await
                .unwrap();
        }

        assert_eq!(ticket.review_feedback.as_deref(), Some("Second feedback"));
        let activity = store.ticket_activity(ticket.id).await.unwrap();
        let feedback = activity
            .iter()
            .filter(|entry| entry.action == "changes_requested")
            .map(|entry| entry.detail.as_deref().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(feedback, ["First feedback", "Second feedback"]);
        let commits = activity
            .iter()
            .filter(|entry| entry.action == "review_submitted")
            .map(|entry| entry.review_commit.as_deref().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(commits, ["first", "second"]);
    }

    #[tokio::test]
    async fn ticket_summary_pages_filter_order_and_scale_to_1000_tickets() {
        let store = Store::memory().await.unwrap();
        sqlx::query(
            "WITH RECURSIVE sequence(id) AS (SELECT 1 UNION ALL SELECT id + 1 FROM sequence WHERE id < 1000) \
             INSERT INTO tickets (id, project_id, title, state) SELECT id, 1, 'Ticket ' || id, CASE WHEN id = 1 THEN 'todo' ELSE 'done' END FROM sequence",
        )
        .execute(&store.pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO ticket_dependencies (ticket_id, blocker_id) VALUES (1000, 1)")
            .execute(&store.pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO ticket_activity (ticket_id, action) VALUES (1000, 'comment'), (1000, 'review_submitted')",
        )
        .execute(&store.pool)
        .await
        .unwrap();

        let started = Instant::now();
        let (first, has_more) = store
            .ticket_summaries(TicketSummaryFilter {
                state: Some(TicketState::Done),
                limit: Some(25),
                ..Default::default()
            })
            .await
            .unwrap();
        eprintln!("1,000-ticket summary page: {:?}", started.elapsed());
        assert!(has_more);
        assert_eq!(first.len(), 25);
        assert_eq!((first[0].id, first[24].id), (1000, 976));
        assert_eq!(first[0].unresolved_blockers[0].id, 1);
        assert_eq!(first[0].activity_count, 2);
        assert!(first[0].latest_activity_id.is_some());

        sqlx::query("UPDATE tickets SET review_completion_workflow = 'external' WHERE id = 1000")
            .execute(&store.pool)
            .await
            .unwrap();
        let (external, _) = store
            .ticket_summaries(TicketSummaryFilter {
                before_id: Some(1001),
                limit: Some(1),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(
            external[0].review_completion_workflow,
            Some(ProjectCompletionWorkflow::External)
        );

        let (second, _) = store
            .ticket_summaries(TicketSummaryFilter {
                state: Some(TicketState::Done),
                before_id: Some(976),
                limit: Some(25),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!((second[0].id, second[24].id), (975, 951));

        let (capped, _) = store
            .ticket_summaries(TicketSummaryFilter {
                limit: Some(1_000),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(capped.len(), MAX_TICKET_PAGE_SIZE as usize);
    }

    #[tokio::test]
    async fn cancelled_state_migration_preserves_existing_tickets() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let migrator = sqlx::migrate!();
        for migration in migrator.iter().filter(|migration| migration.version < 24) {
            sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
        }
        let project_id = sqlx::query(
            "INSERT INTO projects (name, path, default_branch) VALUES ('Board', '/board', 'main')",
        )
        .execute(&pool)
        .await
        .unwrap()
        .last_insert_rowid();
        sqlx::query(
            "INSERT INTO tickets (id, project_id, title, state) VALUES (7, ?, 'Existing', 'done')",
        )
        .bind(project_id)
        .execute(&pool)
        .await
        .unwrap();
        let migration = migrator
            .iter()
            .find(|migration| migration.version == 24)
            .unwrap();
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();

        let preserved: (String, String) =
            sqlx::query_as("SELECT title, state FROM tickets WHERE id = 7")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(preserved, ("Existing".into(), "done".into()));
        sqlx::query(
            "INSERT INTO tickets (project_id, title, state) VALUES (?, 'Obsolete', 'cancelled')",
        )
        .bind(project_id)
        .execute(&pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn external_workflow_migration_preserves_and_snapshots_existing_reviews() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let migrator = sqlx::migrate!();
        for migration in migrator.iter().filter(|migration| migration.version < 26) {
            sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
        }
        let project_id = sqlx::query(
            "INSERT INTO projects (name, path, default_branch) VALUES ('Board', '/board', 'main')",
        )
        .execute(&pool)
        .await
        .unwrap()
        .last_insert_rowid();
        sqlx::query(
            "INSERT INTO tickets (id, project_id, title, state) VALUES \
             (7, ?, 'Existing review', 'review'), (8, ?, 'Existing done', 'done')",
        )
        .bind(project_id)
        .bind(project_id)
        .execute(&pool)
        .await
        .unwrap();
        let migration = migrator
            .iter()
            .find(|migration| migration.version == 26)
            .unwrap();
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();

        let workflow: String =
            sqlx::query_scalar("SELECT completion_workflow FROM projects WHERE id = ?")
                .bind(project_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(workflow, "board_managed");
        let snapshots: Vec<(String, Option<String>)> =
            sqlx::query_as("SELECT state, review_completion_workflow FROM tickets ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            snapshots,
            vec![
                ("review".into(), Some("board_managed".into())),
                ("done".into(), None),
            ]
        );
        let foreign_key_errors: Vec<(String, i64, String, i64)> =
            sqlx::query_as("PRAGMA foreign_key_check")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert!(foreign_key_errors.is_empty());
    }

    #[tokio::test]
    async fn awaiting_external_merge_survives_restart() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("mab-external-workflow-{unique}.sqlite3"));
        let store = Store::open(&path).await.unwrap();
        store
            .set_project("/tmp/multi-agent-board-test-project")
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO tickets (project_id, title, state, review_completion_workflow, external_change_url) \
             VALUES (1, 'Waiting outside', 'awaiting_external_merge', 'external', 'https://example.test/change')",
        )
        .execute(&store.pool)
        .await
        .unwrap();
        store.close().await;

        let reopened = Store::open(&path).await.unwrap();
        let ticket = reopened.ticket(1).await.unwrap().unwrap();
        assert_eq!(ticket.state, TicketState::AwaitingExternalMerge);
        assert_eq!(
            ticket.external_change_url.as_deref(),
            Some("https://example.test/change")
        );
        let cancelled = reopened
            .update_ticket(
                ticket.id,
                TicketChanges {
                    state: Some(TicketState::Cancelled),
                    expected_revision: Some(ticket.revision),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(cancelled.state, TicketState::Cancelled);
        reopened.close().await;
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn external_merge_ready_wake_survives_restart_and_is_claimed_once() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("mab-external-wake-{unique}.sqlite3"));
        let store = Store::open(&path).await.unwrap();
        let project = store
            .set_project("/tmp/multi-agent-board-external-wake")
            .await
            .unwrap();
        store
            .set_project_completion_workflow(project.id, ProjectCompletionWorkflow::External)
            .await
            .unwrap();
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
                title: "External wake".into(),
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
        while let Some(event) = store
            .claim_pending_wake_event(worker.id, true)
            .await
            .unwrap()
        {
            store.complete_wake_event(event.id).await.unwrap();
        }
        let ticket = store
            .submit_review(
                ticket.id,
                worker.id,
                ReviewSubmission {
                    implementation_branch: Some("tickets/external-wake"),
                    commit: "reviewed-commit",
                    target_branch: "main",
                    project_id: project.id,
                    project_path: &project.path,
                },
                Some(ticket.revision),
            )
            .await
            .unwrap();
        let ticket = store
            .claim_review(ticket.id, reviewer.id, Some(ticket.revision))
            .await
            .unwrap();
        store
            .approve_external_review(
                ticket.id,
                reviewer.id,
                "reviewed-commit",
                Some(ticket.revision),
            )
            .await
            .unwrap();
        store.close().await;

        let reopened = Store::open(&path).await.unwrap();
        let event = reopened
            .claim_pending_wake_event(worker.id, true)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.kind(), "ticket.external_merge_ready");
        assert!(matches!(
            event.event,
            WakeEventData::TicketExternalMergeReady(ref payload)
                if payload.ticket_id == ticket.id
        ));
        reopened.complete_wake_event(event.id).await.unwrap();
        assert!(
            reopened
                .claim_pending_wake_event(worker.id, true)
                .await
                .unwrap()
                .is_none()
        );
        reopened.close().await;
        std::fs::remove_file(path).unwrap();
    }

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
            store.wake_events(watcher.id).await.unwrap()[0].payload()["ticket"]["project_id"],
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
    async fn cancellation_is_terminal_audited_and_not_claimable_or_wakeable() {
        let store = Store::memory().await.unwrap();
        let worker = store.create_agent("worker", "/tmp/worker").await.unwrap();
        store
            .set_agent_wake_settings(worker.id, true, false, None)
            .await
            .unwrap();
        let (blocker, targets) = store
            .create_ticket_with_wakes(NewTicket {
                title: "Duplicate".into(),
                description: String::new(),
                blocker_ids: Vec::new(),
                created_by_agent_id: None,
            })
            .await
            .unwrap();
        assert_eq!(targets, vec![worker.id]);
        let dependent = store
            .create_ticket(NewTicket {
                title: "Dependent".into(),
                description: String::new(),
                blocker_ids: vec![blocker.id],
                created_by_agent_id: None,
            })
            .await
            .unwrap();
        store
            .enqueue_wake_event(
                worker.id,
                WakeEventData::TicketChangesRequested(TicketWakePayload {
                    message: "Changes requested".into(),
                    ticket_id: blocker.id,
                    revision: None,
                    changed_fields: Vec::new(),
                    ticket: TicketWakeTicket::project(blocker.project_id),
                }),
                Some("cancel-test-changes-requested"),
                WakeDeliveryPolicy::IdleOnly,
            )
            .await
            .unwrap();
        assert_eq!(
            store
                .wake_events(worker.id)
                .await
                .unwrap()
                .iter()
                .map(|event| event.kind())
                .collect::<Vec<_>>(),
            vec!["ticket.available", "ticket.changes_requested"]
        );

        let (cancelled, wake_targets, activities) = store
            .update_ticket_with_wakes(
                blocker.id,
                TicketChanges {
                    state: Some(TicketState::Cancelled),
                    assignee_id: Some(Some(worker.id)),
                    expected_revision: Some(blocker.revision),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(cancelled.state, TicketState::Cancelled);
        assert_eq!(cancelled.assignee_id, Some(worker.id));
        assert!(wake_targets.is_empty());
        assert_eq!(activities.len(), 1);
        assert_eq!(activities[0].action, "cancelled");
        assert!(store.wake_events(worker.id).await.unwrap().is_empty());
        assert_eq!(
            store.ticket_activity(blocker.id).await.unwrap()[0].action,
            "cancelled"
        );
        assert!(
            store
                .claim_ticket(cancelled.id, worker.id, Some(cancelled.revision))
                .await
                .is_err()
        );
        assert!(
            store
                .reserve_tickets(&[cancelled.id], worker.id, None, false)
                .await
                .is_err()
        );
        assert!(
            store
                .update_ticket(
                    cancelled.id,
                    TicketChanges {
                        state: Some(TicketState::Todo),
                        expected_revision: Some(cancelled.revision),
                        ..Default::default()
                    },
                )
                .await
                .unwrap_err()
                .to_string()
                .contains("terminal")
        );
        let dependent = store.ticket(dependent.id).await.unwrap().unwrap();
        assert_eq!(dependent.unresolved_blockers[0].id, cancelled.id);

        for state in [TicketState::Review, TicketState::Done] {
            let ticket = store
                .create_ticket(NewTicket {
                    title: format!("Cannot cancel {state:?}"),
                    description: String::new(),
                    blocker_ids: Vec::new(),
                    created_by_agent_id: None,
                })
                .await
                .unwrap();
            let ticket = store
                .update_ticket(
                    ticket.id,
                    TicketChanges {
                        state: Some(state),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            assert!(
                store
                    .update_ticket(
                        ticket.id,
                        TicketChanges {
                            state: Some(TicketState::Cancelled),
                            ..Default::default()
                        },
                    )
                    .await
                    .is_err()
            );
        }
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
        store
            .set_agent_wake_settings(reserved.id, true, false, None)
            .await
            .unwrap();
        store
            .set_agent_project_subscriptions(reserved.id, &[])
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

        let (tickets, wake_targets, activities) = store
            .reserve_tickets_with_wakes(
                &[first.id, second.id],
                reserved.id,
                Some(reserved.id),
                false,
            )
            .await
            .unwrap();
        assert_eq!(wake_targets, vec![reserved.id]);
        assert!(activities.is_empty());
        assert_eq!(
            store
                .wake_events(reserved.id)
                .await
                .unwrap()
                .iter()
                .map(|event| event.kind())
                .collect::<Vec<_>>(),
            vec!["ticket.reserved", "ticket.reserved"]
        );
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
        let unsubscribed = store
            .create_agent("unsubscribed", "/tmp/unsubscribed")
            .await
            .unwrap();
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
        store
            .set_agent_wake_settings(unsubscribed.id, true, true, None)
            .await
            .unwrap();
        store
            .set_agent_project_subscriptions(unsubscribed.id, &[])
            .await
            .unwrap();
        for agent_id in [
            work_only.id,
            review_only.id,
            both.id,
            unsubscribed.id,
            neither.id,
        ] {
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
        assert!(store.wake_events(unsubscribed.id).await.unwrap().is_empty());
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
            store.wake_events(implementer.id).await.unwrap()[0].kind(),
            "ticket.assigned"
        );
        store
            .set_agent_project_subscriptions(implementer.id, &[])
            .await
            .unwrap();
        assert_eq!(
            store.wake_events(implementer.id).await.unwrap()[0].kind(),
            "ticket.assigned"
        );

        let (ticket, review_targets, activity) = store
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
        assert_eq!(activity.action, "review_submitted");
        assert_eq!(review_targets, vec![review_only.id, both.id]);
        assert_eq!(store.wake_events(work_only.id).await.unwrap().len(), 1);
        let reviewer_events = store.wake_events(review_only.id).await.unwrap();
        assert_eq!(reviewer_events[0].kind(), "ticket.review_requested");
        assert_eq!(
            reviewer_events[0].delivery_policy,
            WakeDeliveryPolicy::IdleOnly
        );
        assert_eq!(reviewer_events[0].payload()["ticket_id"], ticket.id);
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
                .map(|event| event.kind())
                .collect::<Vec<_>>(),
            vec!["ticket.available", "ticket.review_requested"]
        );
        assert!(store.wake_events(neither.id).await.unwrap().is_empty());
        assert!(store.wake_events(unsubscribed.id).await.unwrap().is_empty());
        store
            .set_agent_project_subscriptions(both.id, &[])
            .await
            .unwrap();
        assert!(store.wake_events(both.id).await.unwrap().is_empty());
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

    #[tokio::test]
    async fn external_review_snapshot_survives_project_changes_and_records_completion() {
        let store = Store::memory().await.unwrap();
        let project = store
            .set_project_completion_workflow(1, ProjectCompletionWorkflow::External)
            .await
            .unwrap();
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
                title: "Merge elsewhere".into(),
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
                    implementation_branch: Some("tickets/external"),
                    commit: "reviewed-commit",
                    target_branch: "main",
                    project_id: project.id,
                    project_path: &project.path,
                },
                Some(ticket.revision),
            )
            .await
            .unwrap();
        assert_eq!(
            ticket.review_completion_workflow,
            Some(ProjectCompletionWorkflow::External)
        );

        store
            .set_project_completion_workflow(project.id, ProjectCompletionWorkflow::BoardManaged)
            .await
            .unwrap();
        let ticket = store
            .claim_review(ticket.id, reviewer.id, Some(ticket.revision))
            .await
            .unwrap();
        let ticket = store
            .set_external_change_url(
                ticket.id,
                "https://example.test/change/123",
                Some(worker.id),
                Some(ticket.revision),
            )
            .await
            .unwrap();
        let ticket = store
            .approve_external_review(
                ticket.id,
                reviewer.id,
                "reviewed-commit",
                Some(ticket.revision),
            )
            .await
            .unwrap();
        assert_eq!(ticket.state, TicketState::AwaitingExternalMerge);
        assert!(
            store
                .claim_review(ticket.id, reviewer.id, Some(ticket.revision))
                .await
                .is_err()
        );

        let returned = store
            .return_external_merge(
                ticket.id,
                "External checks failed",
                Some(reviewer.id),
                Some(ticket.revision),
            )
            .await
            .unwrap();
        assert_eq!(returned.state, TicketState::InProgress);
        assert_eq!(returned.external_change_url, ticket.external_change_url);

        store
            .set_project_completion_workflow(project.id, ProjectCompletionWorkflow::External)
            .await
            .unwrap();
        let resubmitted = store
            .submit_review(
                returned.id,
                worker.id,
                ReviewSubmission {
                    implementation_branch: Some("tickets/external"),
                    commit: "second-reviewed-commit",
                    target_branch: "main",
                    project_id: project.id,
                    project_path: &project.path,
                },
                Some(returned.revision),
            )
            .await
            .unwrap();
        assert_eq!(
            resubmitted.review_completion_workflow,
            Some(ProjectCompletionWorkflow::External)
        );
        let approved = store
            .claim_review(resubmitted.id, reviewer.id, Some(resubmitted.revision))
            .await
            .unwrap();
        let approved = store
            .approve_external_review(
                approved.id,
                reviewer.id,
                "second-reviewed-commit",
                Some(approved.revision),
            )
            .await
            .unwrap();
        let done = store
            .complete_external_merge(
                approved.id,
                "Merged after external checks",
                Some("squash-commit-not-equal-to-reviewed-commit"),
                Some(reviewer.id),
                Some(approved.revision),
            )
            .await
            .unwrap();
        assert_eq!(done.state, TicketState::Done);
        assert_eq!(
            done.external_merge_reference.as_deref(),
            Some("squash-commit-not-equal-to-reviewed-commit")
        );
        assert!(done.external_merged_at.is_some());
        let activity = store.ticket_activity(done.id).await.unwrap();
        assert!(activity.iter().any(|entry| {
            entry.action == "external_merge_returned"
                && entry.detail.as_deref() == Some("External checks failed")
        }));
        assert!(activity.iter().any(|entry| {
            entry.action == "external_merge_completed"
                && entry.detail.as_deref() == Some("Merged after external checks")
        }));

        store
            .set_project_completion_workflow(project.id, ProjectCompletionWorkflow::BoardManaged)
            .await
            .unwrap();
        let board_ticket = store
            .create_ticket(NewTicket {
                title: "Keep board merge snapshot".into(),
                description: String::new(),
                blocker_ids: Vec::new(),
                created_by_agent_id: None,
            })
            .await
            .unwrap();
        let board_ticket = store
            .claim_ticket(board_ticket.id, worker.id, Some(board_ticket.revision))
            .await
            .unwrap();
        let board_ticket = store
            .submit_review(
                board_ticket.id,
                worker.id,
                ReviewSubmission {
                    implementation_branch: Some("tickets/board"),
                    commit: "board-reviewed-commit",
                    target_branch: "main",
                    project_id: project.id,
                    project_path: &project.path,
                },
                Some(board_ticket.revision),
            )
            .await
            .unwrap();
        store
            .set_project_completion_workflow(project.id, ProjectCompletionWorkflow::External)
            .await
            .unwrap();
        let board_ticket = store
            .claim_review(board_ticket.id, reviewer.id, Some(board_ticket.revision))
            .await
            .unwrap();
        let board_ticket = store
            .complete_review(
                board_ticket.id,
                reviewer.id,
                "board-reviewed-commit",
                Some(board_ticket.revision),
            )
            .await
            .unwrap();
        assert_eq!(board_ticket.state, TicketState::Done);
    }
}

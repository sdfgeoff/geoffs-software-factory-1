use tokio::process::Command;

use crate::{
    domain::{
        ProjectCompletionWorkflow, TicketWakePayload, TicketWakeTicket, WakeDeliveryPolicy,
        WakeEventData,
    },
    error::AppError,
    store::{MergeNotification, ReviewSubmission},
};

use super::{AgentEvent, AgentManager};

impl AgentManager {
    pub async fn submit_review(
        &self,
        ticket_id: i64,
        agent_id: i64,
        expected_revision: Option<i64>,
    ) -> Result<crate::domain::Ticket, AppError> {
        let agent = self
            .store
            .agent(agent_id)
            .await?
            .ok_or_else(|| AppError::not_found("agent not found"))?;
        let ticket = self
            .store
            .ticket(ticket_id)
            .await?
            .ok_or_else(|| AppError::not_found("ticket not found"))?;
        let project = self.store.active_ticket_project(&ticket).await?;
        let worktree = self.worktree_for_project(agent.id, project.id).await?;
        if git_common_dir(&worktree.path).await? != git_common_dir(&project.path).await? {
            return Err(AppError::conflict(
                "agent worktree does not belong to the ticket project",
            ));
        }
        let implementation_branch = current_branch(&worktree.path).await?;
        let commit = git_text(&worktree.path, &["rev-parse", "HEAD"]).await?;
        if git_text(&project.path, &["rev-parse", &implementation_branch]).await? != commit {
            return Err(AppError::conflict(
                "implementation branch does not belong to the ticket project",
            ));
        }
        let target_branch = git_text(&project.path, &["symbolic-ref", "--short", "HEAD"]).await?;
        let (ticket, wake_agent_ids, activity) = self
            .store
            .submit_review_with_wakes(
                ticket_id,
                agent_id,
                ReviewSubmission {
                    implementation_branch: Some(&implementation_branch),
                    commit: &commit,
                    target_branch: &target_branch,
                    project_id: project.id,
                    project_path: &project.path,
                },
                expected_revision,
            )
            .await?;
        self.emit_ticket_activity(agent_id, activity).await;
        self.emit(AgentEvent::TicketUpdated {
            agent_id,
            data: ticket.clone(),
        })
        .await;
        for agent_id in wake_agent_ids {
            self.deliver_pending(agent_id).await;
        }
        Ok(ticket)
    }

    pub async fn claim_review(
        &self,
        ticket_id: i64,
        agent_id: i64,
        expected_revision: Option<i64>,
    ) -> Result<crate::domain::Ticket, AppError> {
        let (ticket, activity) = self
            .store
            .claim_review_with_activity(ticket_id, agent_id, expected_revision)
            .await?;
        self.emit_ticket_activity(agent_id, activity).await;
        self.emit(AgentEvent::TicketUpdated {
            agent_id,
            data: ticket.clone(),
        })
        .await;
        Ok(ticket)
    }

    pub async fn claim_review_with_transition(
        &self,
        ticket_id: i64,
        agent_id: i64,
        expected_revision: Option<i64>,
    ) -> Result<crate::domain::TicketClaimResponse, AppError> {
        self.prepare_ticket_project(agent_id, ticket_id).await?;
        let ticket = self
            .claim_review(ticket_id, agent_id, expected_revision)
            .await?;
        let project_transition = self.transition_to_ticket_project(agent_id, &ticket).await?;
        Ok(crate::domain::TicketClaimResponse {
            ticket,
            project_transition,
        })
    }

    pub async fn request_changes(
        &self,
        ticket_id: i64,
        agent_id: i64,
        feedback: &str,
        expected_revision: Option<i64>,
    ) -> Result<crate::domain::Ticket, AppError> {
        let (ticket, activity) = self
            .store
            .request_changes_with_activity(ticket_id, agent_id, feedback, expected_revision)
            .await?;
        self.emit_ticket_activity(agent_id, activity).await;
        self.emit(AgentEvent::TicketUpdated {
            agent_id,
            data: ticket.clone(),
        })
        .await;
        if let Some(worker_id) = ticket.assignee_id {
            let branch_instruction = ticket
                .implementation_branch
                .as_deref()
                .map(|branch| format!(" Switch to branch '{branch}' before updating it."))
                .unwrap_or_default();
            self.store
                .enqueue_wake_event(
                    worker_id,
                    WakeEventData::TicketChangesRequested(TicketWakePayload {
                        message: format!(
                            "Changes were requested on ticket #{} '{}'.{} Feedback: {}",
                            ticket.id, ticket.title, branch_instruction, feedback
                        ),
                        ticket_id: ticket.id,
                        revision: None,
                        changed_fields: Vec::new(),
                        ticket: TicketWakeTicket::project(ticket.project_id),
                    }),
                    Some(&format!(
                        "ticket:{}:{}:changes-requested",
                        ticket.id, ticket.revision
                    )),
                    WakeDeliveryPolicy::IdleOnly,
                )
                .await?;
            self.deliver_pending(worker_id).await;
        }
        Ok(ticket)
    }

    pub async fn approve_review(
        &self,
        ticket_id: i64,
        reviewer_id: i64,
        expected_revision: Option<i64>,
    ) -> Result<crate::domain::Ticket, AppError> {
        let operation = self.operations.lock().await;
        if !self
            .store
            .has_agent_scope(reviewer_id, "approve_review")
            .await?
        {
            return Err(AppError::forbidden("agent lacks approve_review scope"));
        }
        let ticket = self
            .store
            .ticket(ticket_id)
            .await?
            .ok_or_else(|| AppError::not_found("ticket not found"))?;
        if ticket.state != crate::domain::TicketState::Review
            || ticket.reviewer_id != Some(reviewer_id)
            || expected_revision.is_some_and(|revision| revision != ticket.revision)
        {
            return Err(AppError::conflict(
                "review is not assigned to this agent or has changed",
            ));
        }
        let merge_notification_channel = self.settings().await?.merge_notification_channel;
        let commit = ticket
            .review_commit
            .as_deref()
            .ok_or_else(|| AppError::conflict("review has no commit"))?;
        let target_branch = ticket
            .review_target_branch
            .as_deref()
            .ok_or_else(|| AppError::conflict("review has no target branch"))?;
        let project = self.store.review_ticket_project(&ticket).await?;

        if ticket.review_completion_workflow == Some(ProjectCompletionWorkflow::External) {
            if let Some(implementation_branch) = ticket.implementation_branch.as_deref()
                && git_text(&project.path, &["rev-parse", implementation_branch]).await? != commit
            {
                return Err(AppError::conflict(
                    "implementation branch changed after review submission",
                ));
            }
            let reviewer = self
                .store
                .agent(reviewer_id)
                .await?
                .ok_or_else(|| AppError::not_found("reviewer not found"))?;
            let source_branch = ticket.implementation_branch.as_deref().unwrap_or("unknown");
            let external_url = ticket
                .external_change_url
                .as_deref()
                .unwrap_or("not attached");
            let body = format!(
                "Ticket #{} “{}” was approved for external merge. The board did not merge Git.\n\n1. Source branch: `{source_branch}`\n2. Reviewed commit: `{commit}`\n3. Target branch: `{target_branch}`\n4. External change: {external_url}\n5. Approved by: {}",
                ticket.id, ticket.title, reviewer.name
            );
            let key = format!("ticket-external-approved:{}:{commit}", ticket.id);
            let notification =
                merge_notification_channel
                    .as_deref()
                    .map(|channel| MergeNotification {
                        channel,
                        author: "Multi-agent Board",
                        body: &body,
                        deduplication_key: &key,
                    });
            let (ticket, message, wake_agent_ids, activity) = self
                .store
                .approve_external_review_with_wakes(
                    ticket_id,
                    reviewer_id,
                    commit,
                    expected_revision,
                    notification,
                )
                .await?;
            self.emit_ticket_activity(reviewer_id, activity).await;
            drop(operation);
            if let Some(message) = message.as_ref() {
                self.emit_board_message(message).await;
            }
            for agent_id in wake_agent_ids {
                self.deliver_pending(agent_id).await;
            }
            self.emit(AgentEvent::ExternalReviewApproved {
                agent_id: reviewer_id,
                data: ticket.clone(),
            })
            .await;
            return Ok(ticket);
        }

        if !git_text(&project.path, &["status", "--porcelain"])
            .await?
            .is_empty()
        {
            return Err(AppError::conflict("primary worktree is dirty"));
        }
        if git_text(&project.path, &["symbolic-ref", "--short", "HEAD"]).await? != target_branch {
            return Err(AppError::conflict(
                "primary branch changed after review submission",
            ));
        }
        if let Some(implementation_branch) = ticket.implementation_branch.as_deref()
            && git_text(&project.path, &["rev-parse", implementation_branch]).await? != commit
        {
            return Err(AppError::conflict(
                "implementation branch changed after review submission",
            ));
        }
        let can_fast_forward = git_output(
            &project.path,
            &["merge-base", "--is-ancestor", "HEAD", commit],
        )
        .await?;
        let merge = if can_fast_forward.status.success() {
            git_output(&project.path, &["merge", "--ff-only", commit]).await?
        } else {
            git_output(&project.path, &["merge", "--no-ff", "--no-edit", commit]).await?
        };
        if !merge.status.success() {
            let conflicts = git_text(&project.path, &["diff", "--name-only", "--diff-filter=U"])
                .await
                .unwrap_or_default();
            let merge_in_progress = git_output(
                &project.path,
                &["rev-parse", "--verify", "--quiet", "MERGE_HEAD"],
            )
            .await?
            .status
            .success();
            if merge_in_progress {
                let abort = git_output(&project.path, &["merge", "--abort"]).await?;
                if !abort.status.success() {
                    return Err(AppError::internal(format!(
                        "merge failed and could not be aborted: {}",
                        String::from_utf8_lossy(&abort.stderr).trim()
                    )));
                }
            }
            if !git_text(&project.path, &["status", "--porcelain"])
                .await?
                .is_empty()
            {
                return Err(AppError::internal(
                    "merge failed and left the primary worktree dirty",
                ));
            }
            let detail = if conflicts.is_empty() {
                String::from_utf8_lossy(&merge.stderr).trim().to_owned()
            } else {
                format!(
                    "conflicts in {}; merge {target_branch} into the implementation branch and resubmit",
                    conflicts.lines().collect::<Vec<_>>().join(", ")
                )
            };
            return Err(AppError::conflict(format!("review merge failed: {detail}")));
        }
        let resulting_commit = git_text(&project.path, &["rev-parse", "HEAD"]).await?;
        let reviewer = self
            .store
            .agent(reviewer_id)
            .await?
            .ok_or_else(|| AppError::not_found("reviewer not found"))?;
        let (ticket, wake_agent_ids, activity) = if let Some(channel) =
            merge_notification_channel.as_deref()
        {
            let source_branch = ticket.implementation_branch.as_deref().unwrap_or("unknown");
            let body = format!(
                "Ticket #{} “{}” was merged.\n\n1. Source branch: `{source_branch}`\n2. Target branch: `{target_branch}`\n3. Merge commit: `{resulting_commit}`\n4. Approved by: @{}",
                ticket.id, ticket.title, reviewer.name
            );
            let key = format!("ticket-merge:{}:{commit}", ticket.id);
            let (ticket, _message, wake_agent_ids, activity) = self
                .store
                .complete_review_and_post_message(
                    ticket_id,
                    reviewer_id,
                    commit,
                    expected_revision,
                    MergeNotification {
                        channel,
                        author: "Multi-agent Board",
                        body: &body,
                        deduplication_key: &key,
                    },
                )
                .await?;
            (ticket, wake_agent_ids, activity)
        } else {
            let (ticket, activity) = self
                .store
                .complete_review_with_activity(ticket_id, reviewer_id, commit, expected_revision)
                .await?;
            (ticket, Vec::new(), activity)
        };
        self.emit_ticket_activity(reviewer_id, activity).await;
        drop(operation);
        for agent_id in wake_agent_ids {
            self.deliver_pending(agent_id).await;
        }
        self.emit(AgentEvent::TicketUpdated {
            agent_id: reviewer_id,
            data: ticket.clone(),
        })
        .await;
        Ok(ticket)
    }

    pub async fn set_external_change_url(
        &self,
        ticket_id: i64,
        external_change_url: &str,
        actor_agent_id: Option<i64>,
        expected_revision: Option<i64>,
    ) -> Result<crate::domain::Ticket, AppError> {
        let (ticket, activity) = self
            .store
            .set_external_change_url_with_activity(
                ticket_id,
                external_change_url,
                actor_agent_id,
                expected_revision,
            )
            .await?;
        self.emit_ticket_activity(actor_agent_id.unwrap_or_default(), activity)
            .await;
        self.emit(AgentEvent::ExternalChangeUpdated {
            agent_id: actor_agent_id.unwrap_or_default(),
            data: ticket.clone(),
        })
        .await;
        Ok(ticket)
    }

    pub async fn return_external_merge(
        &self,
        ticket_id: i64,
        reason: &str,
        actor_agent_id: Option<i64>,
        expected_revision: Option<i64>,
    ) -> Result<crate::domain::Ticket, AppError> {
        let (ticket, activity) = self
            .store
            .return_external_merge_with_activity(
                ticket_id,
                reason,
                actor_agent_id,
                expected_revision,
            )
            .await?;
        self.emit_ticket_activity(actor_agent_id.unwrap_or_default(), activity)
            .await;
        self.emit(AgentEvent::ExternalMergeReturned {
            agent_id: actor_agent_id.unwrap_or_default(),
            data: ticket.clone(),
        })
        .await;
        Ok(ticket)
    }

    pub async fn complete_external_merge(
        &self,
        ticket_id: i64,
        confirmation: &str,
        merge_reference: Option<&str>,
        actor_agent_id: Option<i64>,
        expected_revision: Option<i64>,
    ) -> Result<crate::domain::Ticket, AppError> {
        let operation = self.operations.lock().await;
        let current = self
            .store
            .ticket(ticket_id)
            .await?
            .ok_or_else(|| AppError::not_found("ticket not found"))?;
        let merge_notification_channel = self.settings().await?.merge_notification_channel;
        let actor = if let Some(agent_id) = actor_agent_id {
            self.store
                .agent(agent_id)
                .await?
                .ok_or_else(|| AppError::not_found("agent not found"))?
                .name
        } else {
            "human".to_owned()
        };
        let source_branch = current
            .implementation_branch
            .as_deref()
            .unwrap_or("unknown");
        let reviewed_commit = current.review_commit.as_deref().unwrap_or("unknown");
        let target_branch = current.review_target_branch.as_deref().unwrap_or("unknown");
        let external_url = current
            .external_change_url
            .as_deref()
            .unwrap_or("not attached");
        let final_reference = merge_reference.unwrap_or("not supplied");
        let body = format!(
            "Ticket #{} “{}” was confirmed externally merged.\n\n1. Source branch: `{source_branch}`\n2. Reviewed commit: `{reviewed_commit}`\n3. Target branch: `{target_branch}`\n4. External change: {external_url}\n5. Final reference: `{final_reference}`\n6. Confirmed by: {actor}\n7. Confirmation: {confirmation}",
            current.id, current.title
        );
        let key = format!(
            "ticket-external-completed:{}:{}",
            current.id, current.revision
        );
        let notification = merge_notification_channel
            .as_deref()
            .map(|channel| MergeNotification {
                channel,
                author: "Multi-agent Board",
                body: &body,
                deduplication_key: &key,
            });
        let (ticket, message, wake_agent_ids, activity) = self
            .store
            .complete_external_merge_with_message(
                ticket_id,
                confirmation,
                merge_reference,
                actor_agent_id,
                expected_revision,
                notification,
            )
            .await?;
        self.emit_ticket_activity(actor_agent_id.unwrap_or_default(), activity)
            .await;
        drop(operation);
        if let Some(message) = message.as_ref() {
            self.emit_board_message(message).await;
        }
        for agent_id in wake_agent_ids {
            self.deliver_pending(agent_id).await;
        }
        self.emit(AgentEvent::ExternalMergeCompleted {
            agent_id: actor_agent_id.unwrap_or_default(),
            data: ticket.clone(),
        })
        .await;
        Ok(ticket)
    }
}

async fn git_output(cwd: &str, arguments: &[&str]) -> Result<std::process::Output, AppError> {
    Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(arguments)
        .output()
        .await
        .map_err(|error| AppError::internal(format!("failed to start Git: {error}")))
}

async fn git_text(cwd: &str, arguments: &[&str]) -> Result<String, AppError> {
    let output = git_output(cwd, arguments).await?;
    if !output.status.success() {
        return Err(AppError::bad_request(format!(
            "Git command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

async fn current_branch(cwd: &str) -> Result<String, AppError> {
    let output = git_output(cwd, &["symbolic-ref", "--quiet", "--short", "HEAD"]).await?;
    if !output.status.success() {
        return Err(AppError::bad_request(
            "cannot submit a review from a detached HEAD; check out an implementation branch first",
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

async fn git_common_dir(cwd: &str) -> Result<std::path::PathBuf, AppError> {
    let path = std::path::PathBuf::from(git_text(cwd, &["rev-parse", "--git-common-dir"]).await?);
    let path = if path.is_absolute() {
        path
    } else {
        std::path::Path::new(cwd).join(path)
    };
    path.canonicalize()
        .map_err(|error| AppError::bad_request(format!("invalid Git repository: {error}")))
}

#[cfg(test)]
mod tests {
    use std::{
        path::Path,
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };

    use serde_json::Value;
    use tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader, duplex, split},
        process::Command,
        sync::Mutex,
    };

    use crate::{
        agent_runtime::{AgentEvent, AgentManager, RpcPeer, RunningAgent},
        domain::{AgentState, ProjectCompletionWorkflow, TicketState},
        store::{NewTicket, ReviewSubmission, Store},
    };

    use super::{git_output, git_text};

    #[tokio::test]
    async fn review_event_uses_the_activity_inserted_by_its_mutation() {
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
                title: "Exact activity".into(),
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
        let project = store
            .project_by_id(ticket.project_id)
            .await
            .unwrap()
            .unwrap();
        let ticket = store
            .submit_review(
                ticket.id,
                worker.id,
                ReviewSubmission {
                    implementation_branch: Some("tickets/exact-activity"),
                    commit: "review-commit",
                    target_branch: "main",
                    project_id: project.id,
                    project_path: &project.path,
                },
                Some(ticket.revision),
            )
            .await
            .unwrap();
        store
            .execute_test_sql(
                "CREATE TRIGGER append_activity_after_review_claim \
                 AFTER INSERT ON ticket_activity WHEN NEW.action = 'review_claimed' \
                 BEGIN INSERT INTO ticket_activity (ticket_id, action, detail) \
                 VALUES (NEW.ticket_id, 'comment', 'concurrent activity'); END",
            )
            .await
            .unwrap();
        let manager = AgentManager::new(
            store.clone(),
            "codex".into(),
            "multi-agent-board".into(),
            "http://127.0.0.1:1/api".into(),
        );
        let mut events = manager.subscribe();

        manager
            .claim_review(ticket.id, reviewer.id, Some(ticket.revision))
            .await
            .unwrap();

        let event = events.recv().await.unwrap();
        let AgentEvent::TicketActivityCreated { data, .. } = event.event else {
            panic!("expected review activity event");
        };
        assert_eq!(data.action, "review_claimed");
        assert_eq!(data.actor_agent_id, Some(reviewer.id));
        let (latest, _) = store
            .ticket_activity_page(ticket.id, None, Some(1))
            .await
            .unwrap();
        assert_eq!(latest[0].action, "comment");
        assert_ne!(data.id, latest[0].id);
    }

    #[tokio::test]
    async fn board_merges_valid_reviews_after_the_target_advances() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!("mab-merge-test-{unique}"));
        let repository = base.join("repo");
        std::fs::create_dir_all(&repository).unwrap();
        for arguments in [
            &["init", "-q"][..],
            &["config", "user.name", "Test"][..],
            &["config", "user.email", "test@example.com"][..],
            &["commit", "--allow-empty", "-qm", "initial"][..],
            &["branch", "-M", "main"][..],
        ] {
            run_git(&repository, arguments);
        }

        let store = Store::memory().await.unwrap();
        let project = store
            .set_project(repository.to_str().unwrap())
            .await
            .unwrap();
        let manager = AgentManager::new(
            store.clone(),
            "codex".into(),
            "multi-agent-board".into(),
            "http://127.0.0.1:1/api".into(),
        )
        .with_merge_notification_channel(Some("environment-merges".into()))
        .unwrap();
        assert!(store.channel("merges").await.unwrap().is_none());
        manager.set_settings(Some("merges")).await.unwrap();
        assert!(store.channel("merges").await.unwrap().is_some());
        let worker = manager.create("worker", project.id).await.unwrap();
        let worker_worktree = agent_worktree(&store, worker.id).await;
        let reviewer = manager.create("reviewer", project.id).await.unwrap();
        let subscriber = manager.create("subscriber", project.id).await.unwrap();
        store
            .set_agent_scopes(reviewer.id, &["approve_review".into()])
            .await
            .unwrap();
        for agent_id in [worker.id, reviewer.id] {
            store
                .update_agent_runtime(agent_id, AgentState::Error, None, None, Some("test"))
                .await
                .unwrap();
        }

        std::fs::write(Path::new(&worker_worktree).join("work.txt"), "ready").unwrap();
        run_git(Path::new(&worker_worktree), &["add", "work.txt"]);
        run_git(
            Path::new(&worker_worktree),
            &["commit", "-qm", "ready for review"],
        );
        let ticket = store
            .create_ticket(NewTicket {
                title: "Merge me".into(),
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
        store
            .set_agent_wake_settings(reviewer.id, false, true, None)
            .await
            .unwrap();
        let ticket = manager
            .submit_review(ticket.id, worker.id, Some(ticket.revision))
            .await
            .unwrap();
        assert_eq!(
            ticket.implementation_branch.as_deref(),
            Some(format!("agents/{}", worker.id).as_str())
        );
        run_git(
            Path::new(&worker_worktree),
            &["switch", "-c", "tickets/next"],
        );
        let reviewer_events = store.wake_events(reviewer.id).await.unwrap();
        assert_eq!(reviewer_events.len(), 1);
        assert_eq!(reviewer_events[0].kind(), "ticket.review_requested");
        assert_eq!(
            reviewer_events[0].delivery_state,
            crate::domain::WakeEventState::Pending
        );
        let ticket = manager
            .claim_review(ticket.id, reviewer.id, Some(ticket.revision))
            .await
            .unwrap();

        std::fs::write(repository.join("dirty.txt"), "dirty").unwrap();
        assert!(
            manager
                .approve_review(ticket.id, reviewer.id, Some(ticket.revision))
                .await
                .is_err()
        );
        std::fs::remove_file(repository.join("dirty.txt")).unwrap();
        store
            .execute_test_sql(
                "CREATE TRIGGER fail_merge_notification BEFORE INSERT ON messages \
                 WHEN NEW.deduplication_key LIKE 'ticket-merge:%' \
                 BEGIN SELECT RAISE(ABORT, 'simulated notification failure'); END",
            )
            .await
            .unwrap();
        let mut events = manager.subscribe();
        let error = manager
            .approve_review(ticket.id, reviewer.id, Some(ticket.revision))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("simulated notification failure"));
        let unchanged = store.ticket(ticket.id).await.unwrap().unwrap();
        assert_eq!(unchanged.state, TicketState::Review);
        assert_eq!(unchanged.revision, ticket.revision);
        assert!(
            store
                .messages("merges", None, None, None)
                .await
                .unwrap()
                .0
                .is_empty()
        );
        assert!(events.try_recv().is_err());
        store
            .execute_test_sql("DROP TRIGGER fail_merge_notification")
            .await
            .unwrap();
        manager
            .set_channel_subscriptions(subscriber.id, &["merges".into()])
            .await
            .unwrap();
        while events.try_recv().is_ok() {}
        store
            .update_agent_runtime(
                subscriber.id,
                AgentState::Running,
                Some("subscriber-thread"),
                Some("subscriber-turn"),
                None,
            )
            .await
            .unwrap();
        let (client, server) = duplex(8192);
        let (reader, writer) = split(client);
        let (rpc, _events) = RpcPeer::attach(BufReader::new(reader), writer);
        let child = Command::new("sh")
            .args(["-c", "sleep 60"])
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        manager.processes.lock().await.insert(
            subscriber.id,
            Arc::new(RunningAgent {
                rpc,
                child: Mutex::new(child),
            }),
        );
        tokio::spawn(async move {
            let (reader, mut writer) = split(server);
            let mut lines = BufReader::new(reader).lines();
            let request: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(request["method"], "turn/steer");
            assert!(
                request["params"]["input"][0]["text"]
                    .as_str()
                    .unwrap()
                    .contains("Ticket #1 “Merge me” was merged")
            );
            writer
                .write_all(
                    format!(
                        "{{\"id\":{},\"result\":{{\"turnId\":\"subscriber-turn\"}}}}\n",
                        request["id"]
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let ticket = manager
            .approve_review(ticket.id, reviewer.id, Some(ticket.revision))
            .await
            .unwrap();
        while !matches!(
            events.recv().await.unwrap().event,
            AgentEvent::TicketUpdated { .. }
        ) {}
        assert_eq!(ticket.state, TicketState::Done);
        assert_eq!(
            std::fs::read_to_string(repository.join("work.txt")).unwrap(),
            "ready"
        );
        let resulting_commit = git_text(repository.to_str().unwrap(), &["rev-parse", "HEAD"])
            .await
            .unwrap();
        let (messages, _) = store.messages("merges", None, None, None).await.unwrap();
        assert_eq!(messages.len(), 1);
        let source_branch = format!("Source branch: `agents/{}`", worker.id);
        for expected in [
            "Ticket #1 “Merge me”",
            &source_branch,
            "Target branch: `main`",
            &format!("Merge commit: `{resulting_commit}`"),
            "Approved by: @reviewer",
        ] {
            assert!(messages[0].body.contains(expected));
        }
        let subscriber_events = store.wake_events(subscriber.id).await.unwrap();
        assert_eq!(subscriber_events.len(), 1);
        assert_eq!(subscriber_events[0].kind(), "channel.message");
        assert_eq!(
            subscriber_events[0].delivery_state,
            crate::domain::WakeEventState::Delivered
        );
        manager
            .set_channel_subscriptions(subscriber.id, &[])
            .await
            .unwrap();
        manager.set_settings(None).await.unwrap();

        let second = store
            .create_ticket(NewTicket {
                title: "Stale review".into(),
                description: String::new(),
                blocker_ids: Vec::new(),
                created_by_agent_id: None,
            })
            .await
            .unwrap();
        let second = store
            .claim_ticket(second.id, worker.id, Some(second.revision))
            .await
            .unwrap();
        let second = manager
            .submit_review(second.id, worker.id, Some(second.revision))
            .await
            .unwrap();
        let second = manager
            .claim_review(second.id, reviewer.id, Some(second.revision))
            .await
            .unwrap();
        std::fs::write(Path::new(&worker_worktree).join("later.txt"), "later").unwrap();
        run_git(Path::new(&worker_worktree), &["add", "later.txt"]);
        run_git(
            Path::new(&worker_worktree),
            &["commit", "-qm", "changed after review"],
        );
        assert!(
            manager
                .approve_review(second.id, reviewer.id, Some(second.revision))
                .await
                .is_err()
        );

        let second = manager
            .request_changes(
                second.id,
                reviewer.id,
                "Review the new commit",
                Some(second.revision),
            )
            .await
            .unwrap();
        assert_eq!(
            store
                .wake_events(worker.id)
                .await
                .unwrap()
                .last()
                .unwrap()
                .kind(),
            "ticket.changes_requested"
        );
        let second = manager
            .submit_review(second.id, worker.id, Some(second.revision))
            .await
            .unwrap();
        let second = manager
            .claim_review(second.id, reviewer.id, Some(second.revision))
            .await
            .unwrap();
        std::fs::write(repository.join("main.txt"), "diverged").unwrap();
        run_git(&repository, &["add", "main.txt"]);
        run_git(&repository, &["commit", "-qm", "main moved"]);
        let second = manager
            .approve_review(second.id, reviewer.id, Some(second.revision))
            .await
            .unwrap();
        assert_eq!(second.state, TicketState::Done);
        assert_eq!(
            store
                .messages("merges", None, None, None)
                .await
                .unwrap()
                .0
                .len(),
            1
        );
        assert_eq!(
            git_text(
                repository.to_str().unwrap(),
                &["rev-list", "--parents", "-n", "1", "HEAD"]
            )
            .await
            .unwrap()
            .split_whitespace()
            .count(),
            3
        );
        assert_eq!(
            std::fs::read_to_string(repository.join("later.txt")).unwrap(),
            "later"
        );
        assert_eq!(
            std::fs::read_to_string(repository.join("main.txt")).unwrap(),
            "diverged"
        );

        std::fs::write(Path::new(&worker_worktree).join("work.txt"), "worker").unwrap();
        run_git(Path::new(&worker_worktree), &["add", "work.txt"]);
        run_git(
            Path::new(&worker_worktree),
            &["commit", "-qm", "worker conflict"],
        );
        let conflicting = store
            .create_ticket(NewTicket {
                title: "Conflicting review".into(),
                description: String::new(),
                blocker_ids: Vec::new(),
                created_by_agent_id: None,
            })
            .await
            .unwrap();
        let conflicting = store
            .claim_ticket(conflicting.id, worker.id, Some(conflicting.revision))
            .await
            .unwrap();
        let conflicting = manager
            .submit_review(conflicting.id, worker.id, Some(conflicting.revision))
            .await
            .unwrap();
        let conflicting = manager
            .claim_review(conflicting.id, reviewer.id, Some(conflicting.revision))
            .await
            .unwrap();
        std::fs::write(repository.join("work.txt"), "main").unwrap();
        run_git(&repository, &["add", "work.txt"]);
        run_git(&repository, &["commit", "-qm", "main conflict"]);

        let error = manager
            .approve_review(conflicting.id, reviewer.id, Some(conflicting.revision))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("conflicts in work.txt"));
        assert!(
            git_text(repository.to_str().unwrap(), &["status", "--porcelain"])
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            git_text(
                repository.to_str().unwrap(),
                &["symbolic-ref", "--short", "HEAD"]
            )
            .await
            .unwrap(),
            "main"
        );
        assert_eq!(
            std::fs::read_to_string(repository.join("work.txt")).unwrap(),
            "main"
        );
        assert_eq!(
            store.ticket(conflicting.id).await.unwrap().unwrap().state,
            TicketState::Review
        );
        assert!(
            !git_output(
                repository.to_str().unwrap(),
                &["rev-parse", "--verify", "--quiet", "MERGE_HEAD"]
            )
            .await
            .unwrap()
            .status
            .success()
        );

        let detached = store
            .create_ticket(NewTicket {
                title: "Detached review".into(),
                description: String::new(),
                blocker_ids: Vec::new(),
                created_by_agent_id: None,
            })
            .await
            .unwrap();
        let detached = store
            .claim_ticket(detached.id, worker.id, Some(detached.revision))
            .await
            .unwrap();
        run_git(Path::new(&worker_worktree), &["checkout", "--detach"]);
        let error = manager
            .submit_review(detached.id, worker.id, Some(detached.revision))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("detached HEAD"));

        std::fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn legacy_review_survives_worker_rename() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!("mab-legacy-review-test-{unique}"));
        let repository = base.join("repo");
        std::fs::create_dir_all(&repository).unwrap();
        for arguments in [
            &["init", "-q"][..],
            &["config", "user.name", "Test"][..],
            &["config", "user.email", "test@example.com"][..],
            &["commit", "--allow-empty", "-qm", "initial"][..],
            &["branch", "-M", "main"][..],
        ] {
            run_git(&repository, arguments);
        }

        let store = Store::memory().await.unwrap();
        let project = store
            .set_project(repository.to_str().unwrap())
            .await
            .unwrap();
        let manager = AgentManager::new(
            store.clone(),
            "codex".into(),
            "multi-agent-board".into(),
            "http://127.0.0.1:1/api".into(),
        );
        let worker = manager.create("old-name", project.id).await.unwrap();
        let worker_worktree = agent_worktree(&store, worker.id).await;
        let reviewer = manager.create("reviewer", project.id).await.unwrap();
        store
            .set_agent_scopes(reviewer.id, &["approve_review".into()])
            .await
            .unwrap();
        std::fs::write(Path::new(&worker_worktree).join("legacy.txt"), "legacy").unwrap();
        run_git(Path::new(&worker_worktree), &["add", "legacy.txt"]);
        run_git(
            Path::new(&worker_worktree),
            &["commit", "-qm", "legacy review"],
        );

        let ticket = store
            .create_ticket(NewTicket {
                title: "Legacy review".into(),
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
        let commit = git_text(&worker_worktree, &["rev-parse", "HEAD"])
            .await
            .unwrap();
        let ticket = store
            .submit_review(
                ticket.id,
                worker.id,
                ReviewSubmission {
                    implementation_branch: None,
                    commit: &commit,
                    target_branch: "main",
                    project_id: ticket.project_id,
                    project_path: repository.to_str().unwrap(),
                },
                Some(ticket.revision),
            )
            .await
            .unwrap();
        store.rename_agent(worker.id, "new-name").await.unwrap();
        let ticket = manager
            .claim_review(ticket.id, reviewer.id, Some(ticket.revision))
            .await
            .unwrap();
        let ticket = manager
            .approve_review(ticket.id, reviewer.id, Some(ticket.revision))
            .await
            .unwrap();

        assert_eq!(ticket.state, TicketState::Done);
        assert_eq!(
            std::fs::read_to_string(repository.join("legacy.txt")).unwrap(),
            "legacy"
        );
        assert!(
            store
                .messages("general", None, None, None)
                .await
                .unwrap()
                .0
                .is_empty()
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn review_merges_its_ticket_project_after_another_project_is_selected() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!("mab-project-review-test-{unique}"));
        let repository = base.join("ticket-repo");
        let other_repository = base.join("other-repo");
        for repository in [&repository, &other_repository] {
            std::fs::create_dir_all(repository).unwrap();
            for arguments in [
                &["init", "-q"][..],
                &["config", "user.name", "Test"][..],
                &["config", "user.email", "test@example.com"][..],
                &["commit", "--allow-empty", "-qm", "initial"][..],
                &["branch", "-M", "main"][..],
            ] {
                run_git(repository, arguments);
            }
        }

        let store = Store::memory().await.unwrap();
        let project = store
            .set_project(repository.to_str().unwrap())
            .await
            .unwrap();
        let manager = AgentManager::new(
            store.clone(),
            "codex".into(),
            "multi-agent-board".into(),
            "http://127.0.0.1:1/api".into(),
        );
        let worker = manager.create("project-worker", project.id).await.unwrap();
        let worker_worktree = agent_worktree(&store, worker.id).await;
        let reviewer = manager
            .create("project-reviewer", project.id)
            .await
            .unwrap();
        store
            .set_agent_scopes(reviewer.id, &["approve_review".into()])
            .await
            .unwrap();
        std::fs::write(Path::new(&worker_worktree).join("work.txt"), "ticket repo").unwrap();
        run_git(Path::new(&worker_worktree), &["add", "work.txt"]);
        run_git(
            Path::new(&worker_worktree),
            &["commit", "-qm", "project work"],
        );
        let ticket = store
            .create_ticket(NewTicket {
                title: "Project-pinned review".into(),
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
        let ticket = manager
            .submit_review(ticket.id, worker.id, Some(ticket.revision))
            .await
            .unwrap();
        let ticket = manager
            .claim_review(ticket.id, reviewer.id, Some(ticket.revision))
            .await
            .unwrap();
        let other_head = git_text(other_repository.to_str().unwrap(), &["rev-parse", "HEAD"])
            .await
            .unwrap();

        store
            .set_project(other_repository.to_str().unwrap())
            .await
            .unwrap();
        let ticket = manager
            .approve_review(ticket.id, reviewer.id, Some(ticket.revision))
            .await
            .unwrap();

        assert_eq!(ticket.state, TicketState::Done);
        assert_eq!(
            std::fs::read_to_string(repository.join("work.txt")).unwrap(),
            "ticket repo"
        );
        assert_eq!(
            git_text(other_repository.to_str().unwrap(), &["rev-parse", "HEAD"])
                .await
                .unwrap(),
            other_head
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    #[tokio::test]
    async fn external_approval_does_not_modify_or_require_a_clean_primary_worktree() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!("mab-external-review-{unique}"));
        let repository = base.join("repo");
        std::fs::create_dir_all(&repository).unwrap();
        for arguments in [
            &["init", "-q"][..],
            &["config", "user.name", "Test"][..],
            &["config", "user.email", "test@example.com"][..],
            &["commit", "--allow-empty", "-qm", "initial"][..],
            &["branch", "-M", "main"][..],
        ] {
            run_git(&repository, arguments);
        }
        let store = Store::memory().await.unwrap();
        let project = store
            .set_project(repository.to_str().unwrap())
            .await
            .unwrap();
        store
            .set_project_completion_workflow(project.id, ProjectCompletionWorkflow::External)
            .await
            .unwrap();
        let manager = AgentManager::new(
            store.clone(),
            "codex".into(),
            "multi-agent-board".into(),
            "http://127.0.0.1:1/api".into(),
        )
        .with_merge_notification_channel(Some("merges".into()))
        .unwrap();
        let worker = manager.create("worker", project.id).await.unwrap();
        let worker_worktree = agent_worktree(&store, worker.id).await;
        let reviewer = manager.create("reviewer", project.id).await.unwrap();
        store
            .set_agent_scopes(reviewer.id, &["approve_review".into()])
            .await
            .unwrap();
        store
            .update_agent_runtime(worker.id, AgentState::Error, None, None, Some("test"))
            .await
            .unwrap();
        std::fs::write(Path::new(&worker_worktree).join("work.txt"), "ready").unwrap();
        run_git(Path::new(&worker_worktree), &["add", "work.txt"]);
        run_git(
            Path::new(&worker_worktree),
            &["commit", "-qm", "ready for external review"],
        );
        let ticket = store
            .create_ticket(NewTicket {
                title: "Merge externally".into(),
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
        let ticket = manager
            .submit_review(ticket.id, worker.id, Some(ticket.revision))
            .await
            .unwrap();
        let ticket = manager
            .claim_review(ticket.id, reviewer.id, Some(ticket.revision))
            .await
            .unwrap();
        let ticket = manager
            .set_external_change_url(
                ticket.id,
                "https://example.test/change/61",
                Some(worker.id),
                Some(ticket.revision),
            )
            .await
            .unwrap();
        let primary_head = git_text(repository.to_str().unwrap(), &["rev-parse", "HEAD"])
            .await
            .unwrap();
        std::fs::write(repository.join("user-owned.txt"), "leave me alone").unwrap();
        let mut events = manager.subscribe();

        let ticket = manager
            .approve_review(ticket.id, reviewer.id, Some(ticket.revision))
            .await
            .unwrap();

        assert_eq!(ticket.state, TicketState::AwaitingExternalMerge);
        assert_eq!(
            git_text(repository.to_str().unwrap(), &["rev-parse", "HEAD"])
                .await
                .unwrap(),
            primary_head
        );
        assert_eq!(
            std::fs::read_to_string(repository.join("user-owned.txt")).unwrap(),
            "leave me alone"
        );
        let ready = store
            .wake_events(worker.id)
            .await
            .unwrap()
            .into_iter()
            .filter(|event| event.kind() == "ticket.external_merge_ready")
            .collect::<Vec<_>>();
        assert_eq!(ready.len(), 1);
        assert!(
            store
                .wake_events(reviewer.id)
                .await
                .unwrap()
                .iter()
                .all(|event| event.kind() != "ticket.external_merge_ready"
                    && !(event.kind() == "ticket.review_requested"
                        && event.delivery_state == crate::domain::WakeEventState::Pending))
        );
        assert!(
            manager
                .approve_review(ticket.id, reviewer.id, Some(ticket.revision))
                .await
                .is_err()
        );
        assert_eq!(
            store
                .wake_events(worker.id)
                .await
                .unwrap()
                .iter()
                .filter(|event| event.kind() == "ticket.external_merge_ready")
                .count(),
            1
        );
        let approval_messages = store.messages("merges", None, None, None).await.unwrap().0;
        assert_eq!(approval_messages.len(), 1);
        assert!(
            approval_messages[0]
                .body
                .contains("approved for external merge")
        );
        assert!(
            approval_messages[0]
                .body
                .contains("The board did not merge Git")
        );
        assert!(
            approval_messages[0]
                .body
                .contains("https://example.test/change/61")
        );
        let approval_events = (0..8)
            .filter_map(|_| events.try_recv().ok())
            .collect::<Vec<_>>();
        assert!(
            approval_events
                .iter()
                .any(|event| matches!(&event.event, AgentEvent::BoardMessage { .. }))
        );
        assert!(
            approval_events
                .iter()
                .any(|event| matches!(&event.event, AgentEvent::ExternalReviewApproved { .. }))
        );

        let done = manager
            .complete_external_merge(
                ticket.id,
                "Provider reports merged",
                Some("external-merge-commit"),
                Some(reviewer.id),
                Some(ticket.revision),
            )
            .await
            .unwrap();
        assert_eq!(done.state, TicketState::Done);
        let messages = store.messages("merges", None, None, None).await.unwrap().0;
        assert_eq!(messages.len(), 2);
        assert!(messages[1].body.contains("confirmed externally merged"));
        assert!(messages[1].body.contains("external-merge-commit"));
        let completion_events = (0..8)
            .filter_map(|_| events.try_recv().ok())
            .collect::<Vec<_>>();
        assert!(
            completion_events
                .iter()
                .any(|event| matches!(&event.event, AgentEvent::BoardMessage { .. }))
        );
        assert!(
            completion_events
                .iter()
                .any(|event| matches!(&event.event, AgentEvent::ExternalMergeCompleted { .. }))
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    fn run_git(cwd: &Path, arguments: &[&str]) {
        assert!(
            std::process::Command::new("git")
                .args(arguments)
                .current_dir(cwd)
                .status()
                .unwrap()
                .success()
        );
    }

    async fn agent_worktree(store: &Store, agent_id: i64) -> String {
        let project = store.project().await.unwrap().unwrap();
        store
            .agent_project_worktree(agent_id, project.id)
            .await
            .unwrap()
            .unwrap()
            .path
    }
}

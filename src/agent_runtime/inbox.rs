use std::{process::Stdio, sync::Arc, time::Duration};

use jiff::Timestamp;
use serde_json::{Value, json};
use tokio::{
    io::BufReader,
    process::Command,
    sync::{Mutex, mpsc},
};

use crate::{
    domain::{
        Agent, AgentState, IdleHeartbeatWakePayload, WakeDeliveryPolicy, WakeEvent, WakeEventData,
    },
    error::AppError,
};

use super::{
    AgentEvent, AgentManager, AgentMessageDelta, RpcPeer, RunningAgent, validate_agent_name,
};

const COORDINATION_INSTRUCTIONS: &str = "You are a persistent coding agent coordinated by Multi-agent Board. Work only in your assigned Git worktree. Use the multi_agent_board MCP tools to inspect and update tickets and to communicate through message-board channels. Claim a ticket before starting it, commit cohesive changes, and submit it for review when ready. If you have approve_review scope, claim reviews before reviewing them. For external-workflow tickets, approval does not merge Git: attach the external change URL and confirm completion only after the external merge. Keep ticket state accurate. Do not modify another agent's worktree or the primary worktree.";

impl AgentManager {
    pub async fn deliver_pending_events(&self) {
        if let Ok(agents) = self.store.agents().await {
            for agent in agents {
                self.deliver_pending(agent.id).await;
            }
        }
    }

    pub async fn run_heartbeat_scheduler(&self) {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            let now = Timestamp::now();
            self.reminder_tick(now).await;
            self.heartbeat_tick(now).await;
        }
    }

    async fn reminder_tick(&self, now: Timestamp) {
        let agent_ids = {
            let _operation = self.operations.lock().await;
            self.store
                .enqueue_due_reminders(now.as_second())
                .await
                .unwrap_or_default()
        };
        for agent_id in agent_ids {
            self.deliver_pending(agent_id).await;
        }
    }

    async fn heartbeat_tick(&self, now: Timestamp) {
        let queued = {
            let _operation = self.operations.lock().await;
            let Ok(agents) = self
                .store
                .agents_due_for_idle_heartbeat(now.as_second())
                .await
            else {
                return;
            };
            let mut queued = Vec::with_capacity(agents.len());
            for agent in agents {
                let Some(minutes) = agent.idle_heartbeat_minutes else {
                    continue;
                };
                let message = format!("{minutes} minutes have passed. It is now {now}.");
                if self
                    .store
                    .enqueue_wake_event(
                        agent.id,
                        WakeEventData::IdleHeartbeat(IdleHeartbeatWakePayload {
                            message,
                            minutes,
                            occurred_at: now.to_string(),
                        }),
                        None,
                        WakeDeliveryPolicy::IdleOnly,
                    )
                    .await
                    .is_ok()
                {
                    queued.push(agent.id);
                }
            }
            queued
        };
        for agent_id in queued {
            self.deliver_pending(agent_id).await;
        }
    }

    pub async fn rename(&self, agent_id: i64, name: &str) -> Result<Agent, AppError> {
        let _operation = self.operations.lock().await;
        validate_agent_name(name)?;
        let agent = self
            .store
            .agent(agent_id)
            .await?
            .ok_or_else(|| AppError::not_found("agent not found"))?;
        if agent.name == name {
            return Ok(agent);
        }
        if matches!(
            agent.state,
            AgentState::Running | AgentState::AwaitingApproval
        ) {
            return Err(AppError::conflict(
                "wait for the active turn to finish before renaming the agent",
            ));
        }
        if self.store.agent_named(name).await?.is_some() {
            return Err(AppError::conflict("agent name already exists"));
        }
        let agent = self.store.rename_agent(agent_id, name).await?;
        if let Some(running) = self.processes.lock().await.remove(&agent_id) {
            let _ = running.child.lock().await.kill().await;
        }
        self.emit(AgentEvent::AgentUpdated {
            agent_id: agent.id,
            data: agent.clone(),
        })
        .await;
        Ok(agent)
    }

    pub async fn messages(
        &self,
        agent_id: i64,
        before_id: Option<i64>,
        after_id: Option<i64>,
        limit: Option<i64>,
    ) -> Result<(Vec<crate::domain::AgentMessage>, bool), AppError> {
        self.store
            .agent_messages(agent_id, before_id, after_id, limit)
            .await
    }

    pub async fn send_message(&self, agent_id: i64, message: &str) -> Result<Agent, AppError> {
        self.send_message_with_attachments(
            agent_id,
            message,
            &[],
            crate::store::AttachmentOwner::Human,
        )
        .await
    }

    pub async fn send_message_with_attachments(
        &self,
        agent_id: i64,
        message: &str,
        attachment_ids: &[String],
        owner: crate::store::AttachmentOwner,
    ) -> Result<Agent, AppError> {
        {
            let _operation = self.operations.lock().await;
            if self.store.agent(agent_id).await?.is_none() {
                return Err(AppError::not_found("agent not found"));
            }
            let user_message = self
                .store
                .enqueue_agent_message_with_attachments(agent_id, message, attachment_ids, owner)
                .await?;
            self.emit(AgentEvent::Message {
                agent_id,
                data: user_message,
            })
            .await;
        }
        self.deliver_pending(agent_id).await;
        self.store
            .agent(agent_id)
            .await?
            .ok_or_else(|| AppError::not_found("agent not found"))
    }

    async fn start_inbox_turn(&self, agent: &Agent, message: &str) -> Result<(), AppError> {
        let running = match self.ensure_process(agent).await {
            Ok(running) => running,
            Err(error) => {
                self.fail_agent(agent.id, &error.to_string()).await;
                return Err(error);
            }
        };
        let thread_id = self
            .store
            .agent(agent.id)
            .await?
            .and_then(|agent| agent.thread_id)
            .ok_or_else(|| AppError::internal("agent has no Codex thread"))?;
        let worktree = self.current_worktree(agent.id).await?;
        let result = running
            .rpc
            .request(
                "turn/start",
                turn_params(&thread_id, message, &worktree.path),
            )
            .await;
        match result {
            Ok(result) => {
                let turn_id = result
                    .pointer("/turn/id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| AppError::internal("Codex did not return a turn ID"))?;
                self.store
                    .update_agent_runtime(agent.id, AgentState::Running, None, Some(turn_id), None)
                    .await?;
                Ok(())
            }
            Err(error) => {
                self.fail_agent(agent.id, &error.to_string()).await;
                Err(error)
            }
        }
    }

    async fn steer_inbox_turn(&self, agent: &Agent, message: &str) -> Result<(), AppError> {
        let (Some(thread_id), Some(turn_id)) = (&agent.thread_id, &agent.active_turn_id) else {
            return self.start_inbox_turn(agent, message).await;
        };
        let running = self.ensure_process(agent).await?;
        if running
            .rpc
            .request("turn/steer", steer_params(thread_id, turn_id, message))
            .await
            .is_ok()
        {
            return Ok(());
        }

        let agent = self
            .store
            .agent(agent.id)
            .await?
            .ok_or_else(|| AppError::not_found("agent not found"))?;
        self.start_inbox_turn(&agent, message).await
    }

    pub async fn deliver_pending(&self, agent_id: i64) {
        let _operation = self.operations.lock().await;
        loop {
            let Ok(Some(agent)) = self.store.agent(agent_id).await else {
                return;
            };
            if matches!(agent.state, AgentState::Paused | AgentState::Error) {
                return;
            }
            let Ok(Some(event)) = self
                .store
                .claim_pending_wake_event(agent_id, agent.state == AgentState::Idle)
                .await
            else {
                return;
            };
            let message = match self
                .wake_message_with_project_transition(&agent, &event)
                .await
            {
                Ok(message) => message,
                Err(_) => {
                    let _ = self.store.retry_wake_event(event.id).await;
                    return;
                }
            };
            let delivered = match agent.state {
                AgentState::Idle => self.start_inbox_turn(&agent, &message).await,
                AgentState::Running | AgentState::AwaitingApproval => {
                    self.steer_inbox_turn(&agent, &message).await
                }
                AgentState::Paused | AgentState::Error => unreachable!(),
            };
            if delivered.is_err() {
                if ticket_transition_id(&event).is_some() {
                    let _ = self
                        .store
                        .set_agent_active_project(agent.id, agent.active_project_id)
                        .await;
                }
                let _ = self.store.retry_wake_event(event.id).await;
                return;
            }
            if !matches!(event.event, WakeEventData::Message(_))
                && let Ok(message) = self
                    .store
                    .add_agent_message(agent_id, "system", &message)
                    .await
            {
                self.emit(AgentEvent::Message {
                    agent_id,
                    data: message,
                })
                .await;
            }
            if self.store.complete_wake_event(event.id).await.is_err() {
                return;
            }
        }
    }

    pub async fn pause(&self, agent_id: i64) -> Result<Agent, AppError> {
        let _operation = self.operations.lock().await;
        let agent = self
            .store
            .agent(agent_id)
            .await?
            .ok_or_else(|| AppError::not_found("agent not found"))?;
        let active_turn = (agent.thread_id.clone(), agent.active_turn_id.clone());
        let agent = self
            .store
            .update_agent_runtime(agent_id, AgentState::Paused, None, None, None)
            .await?;
        let running = self.processes.lock().await.remove(&agent_id);
        if let Some(running) = running {
            if let (Some(thread_id), Some(turn_id)) = active_turn {
                let _ = running
                    .rpc
                    .request(
                        "turn/interrupt",
                        json!({"threadId": thread_id, "turnId": turn_id}),
                    )
                    .await;
            }
            let _ = running.child.lock().await.kill().await;
        }
        self.emit(AgentEvent::AgentUpdated {
            agent_id,
            data: agent.clone(),
        })
        .await;
        Ok(agent)
    }

    pub async fn resume(&self, agent_id: i64) -> Result<Agent, AppError> {
        let operation = self.operations.lock().await;
        let agent = self
            .store
            .update_agent_runtime(agent_id, AgentState::Idle, None, None, None)
            .await?;
        self.emit(AgentEvent::AgentUpdated {
            agent_id,
            data: agent.clone(),
        })
        .await;
        drop(operation);
        let manager = self.clone();
        tokio::spawn(async move { manager.deliver_pending(agent_id).await });
        Ok(agent)
    }

    async fn ensure_process(&self, agent: &Agent) -> Result<Arc<RunningAgent>, AppError> {
        if let Some(running) = self.processes.lock().await.get(&agent.id).cloned() {
            return Ok(running);
        }
        let worktree = self.current_worktree(agent.id).await?;
        let command_value = serde_json::to_string(&self.executable_path.to_string_lossy().as_ref())
            .map_err(|error| AppError::internal(error.to_string()))?;
        let board_url = serde_json::to_string(&self.board_url)
            .map_err(|error| AppError::internal(error.to_string()))?;
        let agent_id = serde_json::to_string(&agent.id.to_string())
            .map_err(|error| AppError::internal(error.to_string()))?;
        let mut child = Command::new(&self.codex_path)
            .args(["app-server", "--listen", "stdio://", "-c"])
            .arg(format!(
                "mcp_servers.multi_agent_board.command={command_value}"
            ))
            .args(["-c", "mcp_servers.multi_agent_board.args=[\"mcp\"]"])
            .args([
                "-c",
                "mcp_servers.multi_agent_board.default_tools_approval_mode=\"approve\"",
            ])
            .args(["-c"])
            .arg(format!(
                "mcp_servers.multi_agent_board.env={{MULTI_AGENT_BOARD_URL={board_url},MULTI_AGENT_BOARD_AGENT_ID={agent_id}}}"
            ))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| AppError::internal(format!("failed to start Codex: {error}")))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (rpc, events) = RpcPeer::attach(BufReader::new(stdout), stdin);
        self.consume_events(agent.id, rpc.clone(), events);
        rpc.request(
            "initialize",
            json!({"clientInfo": {"name": "multi_agent_board", "title": "Multi-agent Board", "version": env!("CARGO_PKG_VERSION")}}),
        )
        .await?;
        rpc.notify("initialized", json!({})).await?;

        let instructions = agent_instructions(agent);
        let params = if let Some(thread_id) = &agent.thread_id {
            json!({
                "threadId": thread_id, "cwd": worktree.path,
                "approvalPolicy": "never", "sandbox": "danger-full-access",
                "developerInstructions": instructions
            })
        } else {
            json!({
                "cwd": worktree.path, "approvalPolicy": "never", "sandbox": "danger-full-access",
                "developerInstructions": instructions, "serviceName": "multi_agent_board"
            })
        };
        let method = if agent.thread_id.is_some() {
            "thread/resume"
        } else {
            "thread/start"
        };
        let result = rpc.request(method, params).await?;
        let thread_id = result
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::internal("Codex did not return a thread ID"))?;
        self.store
            .update_agent_runtime(agent.id, AgentState::Running, Some(thread_id), None, None)
            .await?;
        let _ = rpc
            .request(
                "thread/name/set",
                json!({"threadId": thread_id, "name": agent.name}),
            )
            .await;
        let running = Arc::new(RunningAgent {
            rpc,
            child: Mutex::new(child),
        });
        self.processes
            .lock()
            .await
            .insert(agent.id, running.clone());
        Ok(running)
    }

    fn consume_events(&self, agent_id: i64, rpc: RpcPeer, mut events: mpsc::Receiver<Value>) {
        let manager = self.clone();
        tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                manager.handle_event(agent_id, &rpc, event).await;
            }
            manager.processes.lock().await.remove(&agent_id);
            if let Ok(Some(agent)) = manager.store.agent(agent_id).await
                && matches!(
                    agent.state,
                    AgentState::Running | AgentState::AwaitingApproval
                )
            {
                manager
                    .fail_agent(agent_id, "Codex process exited unexpectedly")
                    .await;
            }
        });
    }

    async fn handle_event(&self, agent_id: i64, rpc: &RpcPeer, event: Value) {
        let method = event.get("method").and_then(Value::as_str).unwrap_or("");
        if event.get("id").is_some() && method.contains("request") {
            let _ = self
                .store
                .update_agent_runtime(agent_id, AgentState::AwaitingApproval, None, None, None)
                .await;
            self.emit(AgentEvent::ApprovalDeclined { agent_id }).await;
            let _ = rpc
                .respond(event["id"].clone(), json!({"decision": "decline"}))
                .await;
            return;
        }
        match method {
            "item/agentMessage/delta" => {
                if let Some(delta) = event.pointer("/params/delta").and_then(Value::as_str) {
                    self.emit(AgentEvent::AgentMessageDelta {
                        agent_id,
                        data: AgentMessageDelta {
                            delta: delta.to_owned(),
                        },
                    })
                    .await;
                }
            }
            "item/completed"
                if event.pointer("/params/item/type").and_then(Value::as_str)
                    == Some("agentMessage") =>
            {
                if let Some(text) = event.pointer("/params/item/text").and_then(Value::as_str)
                    && let Ok(message) = self.store.add_agent_message(agent_id, "agent", text).await
                {
                    self.emit(AgentEvent::Message {
                        agent_id,
                        data: message,
                    })
                    .await;
                }
            }
            "turn/completed" => {
                let Some(turn_id) = event.pointer("/params/turn/id").and_then(Value::as_str) else {
                    return;
                };
                let status = event
                    .pointer("/params/turn/status")
                    .and_then(Value::as_str)
                    .unwrap_or("failed");
                let error = event
                    .pointer("/params/turn/error/message")
                    .and_then(Value::as_str);
                let state = if status == "completed" || status == "interrupted" {
                    AgentState::Idle
                } else {
                    AgentState::Error
                };
                if let Ok(Some(agent)) = self
                    .store
                    .finish_agent_turn(agent_id, turn_id, state, error)
                    .await
                {
                    self.emit(AgentEvent::AgentUpdated {
                        agent_id,
                        data: agent,
                    })
                    .await;
                    if state == AgentState::Idle {
                        let manager = self.clone();
                        tokio::spawn(async move { manager.deliver_pending(agent_id).await });
                    }
                }
            }
            "error" => {
                let message = event
                    .pointer("/params/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("Codex error");
                self.fail_agent(agent_id, message).await;
            }
            _ => {
                self.emit(AgentEvent::CodexEvent {
                    agent_id,
                    data: event,
                })
                .await
            }
        }
    }

    async fn fail_agent(&self, agent_id: i64, message: &str) {
        if let Ok(agent) = self
            .store
            .update_agent_runtime(agent_id, AgentState::Error, None, None, Some(message))
            .await
        {
            self.emit(AgentEvent::AgentUpdated {
                agent_id,
                data: agent,
            })
            .await;
        }
    }

    async fn wake_message_with_project_transition(
        &self,
        agent: &Agent,
        event: &WakeEvent,
    ) -> Result<String, AppError> {
        let message = wake_event_message(event);
        let Some(ticket_id) = ticket_transition_id(event) else {
            return Ok(message);
        };
        let ticket = self
            .store
            .ticket(ticket_id)
            .await?
            .ok_or_else(|| AppError::not_found("ticket not found"))?;
        Ok(
            match self.transition_to_ticket_project(agent.id, &ticket).await? {
                Some(transition) => format!("{}\n\n{message}", transition.message),
                None => message,
            },
        )
    }
}

fn agent_instructions(agent: &Agent) -> String {
    format!(
        "{COORDINATION_INSTRUCTIONS} Your board agent ID is {} and your agent name is '{}'.",
        agent.id, agent.name
    )
}

fn turn_params(thread_id: &str, message: &str, worktree_path: &str) -> Value {
    json!({
        "threadId": thread_id,
        "input": [{"type": "text", "text": message}],
        "cwd": worktree_path,
        "approvalPolicy": "never",
        "sandboxPolicy": {"type": "dangerFullAccess"}
    })
}

fn steer_params(thread_id: &str, turn_id: &str, message: &str) -> Value {
    json!({
        "threadId": thread_id,
        "expectedTurnId": turn_id,
        "input": [{"type": "text", "text": message}]
    })
}

fn wake_event_message(event: &WakeEvent) -> String {
    if let WakeEventData::Reminder(payload) = &event.event
        && let Ok(requested) = Timestamp::from_second(payload.requested_delivery_at)
    {
        return format!(
            "Reminder scheduled for {requested}. Actual delivery time: {}.\n\n{}",
            Timestamp::now(),
            payload.message
        );
    }
    event.event.message().to_owned()
}

fn ticket_transition_id(event: &WakeEvent) -> Option<i64> {
    event.event.ticket_id()
}

#[cfg(test)]
mod tests {
    use tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, duplex, split},
        process::Command,
        sync::Mutex,
    };

    use crate::{
        domain::{TicketWakePayload, TicketWakeTicket},
        store::{NewTicket, Store},
    };

    use super::*;

    #[test]
    fn turns_run_without_approval_or_sandbox_prompts() {
        let params = turn_params("thread-1", "Run tests", "/repo");

        assert_eq!(params["approvalPolicy"], "never");
        assert_eq!(params["sandboxPolicy"]["type"], "dangerFullAccess");
    }

    async fn manager_with_fake_agent(
        state: AgentState,
        active_turn_id: Option<&str>,
    ) -> (AgentManager, Agent, RpcPeer, DuplexStream) {
        let store = Store::memory().await.unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        let agent = store
            .update_agent_runtime(agent.id, state, Some("thread-1"), active_turn_id, None)
            .await
            .unwrap();
        let manager = AgentManager::new(
            store,
            "codex".into(),
            "multi-agent-board".into(),
            "http://127.0.0.1:1/api".into(),
        );
        let (client, server) = duplex(8192);
        let (reader, writer) = split(client);
        let (rpc, _events) = RpcPeer::attach(BufReader::new(reader), writer);
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 60"]).kill_on_drop(true);
        let child = command.spawn().unwrap();
        manager.processes.lock().await.insert(
            agent.id,
            Arc::new(RunningAgent {
                rpc: rpc.clone(),
                child: Mutex::new(child),
            }),
        );
        (manager, agent, rpc, server)
    }

    #[tokio::test]
    async fn messages_steer_an_active_turn() {
        let (manager, agent, _rpc, server) =
            manager_with_fake_agent(AgentState::Running, Some("turn-1")).await;
        tokio::spawn(async move {
            let (reader, mut writer) = split(server);
            let mut lines = BufReader::new(reader).lines();
            let request: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(request["method"], "turn/steer");
            assert_eq!(request["params"]["threadId"], "thread-1");
            assert_eq!(request["params"]["expectedTurnId"], "turn-1");
            assert_eq!(request["params"]["input"][0]["text"], "New context");
            writer
                .write_all(
                    format!(
                        "{{\"id\":{},\"result\":{{\"turnId\":\"turn-1\"}}}}\n",
                        request["id"]
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });

        let updated = manager.send_message(agent.id, "New context").await.unwrap();

        assert_eq!(updated.state, AgentState::Running);
        assert_eq!(updated.active_turn_id.as_deref(), Some("turn-1"));
        assert_eq!(
            manager
                .store
                .agent_messages(agent.id, None, None, None)
                .await
                .unwrap()
                .0
                .len(),
            1
        );
        assert_eq!(
            manager.store.wake_events(agent.id).await.unwrap()[0].delivery_state,
            crate::domain::WakeEventState::Delivered
        );
    }

    #[tokio::test]
    async fn subscribed_channel_messages_steer_an_active_turn() {
        let (manager, agent, _rpc, server) =
            manager_with_fake_agent(AgentState::Running, Some("turn-1")).await;
        manager
            .set_channel_subscriptions(agent.id, &["general".into()])
            .await
            .unwrap();
        tokio::spawn(async move {
            let (reader, mut writer) = split(server);
            let mut lines = BufReader::new(reader).lines();
            let request: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(request["method"], "turn/steer");
            assert_eq!(
                request["params"]["input"][0]["text"],
                "A message was posted in #general by human:\nStatus changed"
            );
            writer
                .write_all(
                    format!(
                        "{{\"id\":{},\"result\":{{\"turnId\":\"turn-1\"}}}}\n",
                        request["id"]
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });

        let (_, targets) = manager
            .store
            .post_message_with_wakes("general", None, "human", "Status changed")
            .await
            .unwrap();
        for agent_id in targets {
            manager.deliver_pending(agent_id).await;
        }

        assert_eq!(
            manager.store.wake_events(agent.id).await.unwrap()[0].delivery_state,
            crate::domain::WakeEventState::Delivered
        );
    }

    #[tokio::test]
    async fn ticket_events_change_project_but_channel_messages_keep_it() {
        let (manager, agent, _rpc, server) = manager_with_fake_agent(AgentState::Idle, None).await;
        let project = manager
            .store
            .set_project("/tmp/second-project")
            .await
            .unwrap();
        manager
            .store
            .add_agent_project_worktree(agent.id, project.id, "/tmp/worker-second")
            .await
            .unwrap();
        let (ticket, _) = manager
            .store
            .create_ticket_for_project_with_wakes(
                NewTicket {
                    title: "Project transition".into(),
                    description: String::new(),
                    blocker_ids: Vec::new(),
                    created_by_agent_id: None,
                },
                project.id,
            )
            .await
            .unwrap();
        manager
            .set_channel_subscriptions(agent.id, &["general".into()])
            .await
            .unwrap();
        manager
            .store
            .enqueue_wake_event(
                agent.id,
                WakeEventData::TicketAssigned(TicketWakePayload {
                    message: format!("Ticket #{} was assigned to you.", ticket.id),
                    ticket_id: ticket.id,
                    revision: None,
                    changed_fields: Vec::new(),
                    ticket: TicketWakeTicket::project(project.id),
                }),
                None,
                WakeDeliveryPolicy::IdleOnly,
            )
            .await
            .unwrap();
        let ticket_id = ticket.id;
        let server_task = tokio::spawn(async move {
            let (reader, mut writer) = split(server);
            let mut lines = BufReader::new(reader).lines();
            let transitioned: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(transitioned["method"], "turn/start");
            assert_eq!(transitioned["params"]["threadId"], "thread-1");
            assert_eq!(transitioned["params"]["cwd"], "/tmp/worker-second");
            let text = transitioned["params"]["input"][0]["text"].as_str().unwrap();
            assert!(text.contains(&format!("Project context changed for ticket #{ticket_id}")));
            assert!(text.contains("second-project"));
            assert!(text.contains("Working directory: /tmp/worker-second"));
            writer
                .write_all(
                    format!(
                        "{{\"id\":{},\"result\":{{\"turn\":{{\"id\":\"turn-2\"}}}}}}\n",
                        transitioned["id"]
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();

            let channel: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(channel["method"], "turn/start");
            assert_eq!(channel["params"]["threadId"], "thread-1");
            assert_eq!(channel["params"]["cwd"], "/tmp/worker-second");
            assert_eq!(
                channel["params"]["input"][0]["text"],
                "A message was posted in #general by human:\nStill here"
            );
            writer
                .write_all(
                    format!(
                        "{{\"id\":{},\"result\":{{\"turn\":{{\"id\":\"turn-3\"}}}}}}\n",
                        channel["id"]
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });

        manager.deliver_pending(agent.id).await;
        let active = manager.store.agent(agent.id).await.unwrap().unwrap();
        assert_eq!(active.active_project_id, Some(project.id));
        manager
            .store
            .update_agent_runtime(agent.id, AgentState::Idle, Some("thread-1"), None, None)
            .await
            .unwrap();
        let (_, targets) = manager
            .store
            .post_message_with_wakes("general", None, "human", "Still here")
            .await
            .unwrap();
        for agent_id in targets {
            manager.deliver_pending(agent_id).await;
        }
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn ticket_wakes_wait_for_idle_without_blocking_channel_messages() {
        let (manager, agent, _rpc, server) =
            manager_with_fake_agent(AgentState::Running, Some("turn-1")).await;
        for event in [
            WakeEventData::TicketAvailable(TicketWakePayload {
                message: "Pending implementation work".into(),
                ticket_id: 1,
                revision: None,
                changed_fields: Vec::new(),
                ticket: TicketWakeTicket::project(1),
            }),
            WakeEventData::TicketReviewRequested(TicketWakePayload {
                message: "Pending review".into(),
                ticket_id: 1,
                revision: None,
                changed_fields: Vec::new(),
                ticket: TicketWakeTicket::project(1),
            }),
        ] {
            manager
                .store
                .enqueue_wake_event(agent.id, event, None, WakeDeliveryPolicy::IdleOnly)
                .await
                .unwrap();
        }
        manager
            .set_channel_subscriptions(agent.id, &["general".into()])
            .await
            .unwrap();
        let server_task = tokio::spawn(async move {
            let (reader, mut writer) = split(server);
            let mut lines = BufReader::new(reader).lines();
            let steer: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(steer["method"], "turn/steer");
            assert_eq!(
                steer["params"]["input"][0]["text"],
                "A message was posted in #general by human:\nStatus changed"
            );
            writer
                .write_all(
                    format!(
                        "{{\"id\":{},\"result\":{{\"turnId\":\"turn-1\"}}}}\n",
                        steer["id"]
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();

            let start: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(start["method"], "turn/start");
            assert_eq!(
                start["params"]["input"][0]["text"],
                "Pending implementation work"
            );
            writer
                .write_all(
                    format!(
                        "{{\"id\":{},\"result\":{{\"turn\":{{\"id\":\"turn-2\"}}}}}}\n",
                        start["id"]
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });

        let (_, targets) = manager
            .store
            .post_message_with_wakes("general", None, "human", "Status changed")
            .await
            .unwrap();
        for agent_id in targets {
            manager.deliver_pending(agent_id).await;
        }
        let events = manager.store.wake_events(agent.id).await.unwrap();
        assert_eq!(
            events[0].delivery_state,
            crate::domain::WakeEventState::Pending
        );
        assert_eq!(
            events[1].delivery_state,
            crate::domain::WakeEventState::Pending
        );
        assert_eq!(
            events[2].delivery_state,
            crate::domain::WakeEventState::Delivered
        );

        manager
            .store
            .update_agent_runtime(agent.id, AgentState::Idle, Some("thread-1"), None, None)
            .await
            .unwrap();
        manager.deliver_pending(agent.id).await;
        server_task.await.unwrap();

        let events = manager.store.wake_events(agent.id).await.unwrap();
        assert_eq!(
            events[0].delivery_state,
            crate::domain::WakeEventState::Delivered
        );
        assert_eq!(
            events[1].delivery_state,
            crate::domain::WakeEventState::Pending
        );
        let agent = manager.store.agent(agent.id).await.unwrap().unwrap();
        assert_eq!(agent.state, AgentState::Running);
        assert_eq!(agent.active_turn_id.as_deref(), Some("turn-2"));
    }

    #[tokio::test]
    async fn messages_start_a_turn_for_an_idle_agent() {
        let (manager, agent, _rpc, server) = manager_with_fake_agent(AgentState::Idle, None).await;
        tokio::spawn(async move {
            let (reader, mut writer) = split(server);
            let mut lines = BufReader::new(reader).lines();
            let request: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(request["method"], "turn/start");
            writer
                .write_all(
                    format!(
                        "{{\"id\":{},\"result\":{{\"turn\":{{\"id\":\"turn-2\"}}}}}}\n",
                        request["id"]
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });

        let updated = manager.send_message(agent.id, "Start again").await.unwrap();

        assert_eq!(updated.state, AgentState::Running);
        assert_eq!(updated.active_turn_id.as_deref(), Some("turn-2"));
    }

    #[tokio::test]
    async fn due_heartbeat_starts_a_normal_turn() {
        let (manager, agent, _rpc, server) = manager_with_fake_agent(AgentState::Idle, None).await;
        let agent = manager
            .set_wake_settings(agent.id, false, false, Some(20))
            .await
            .unwrap();
        let now = Timestamp::from_second(agent.idle_since.unwrap() + 1_200).unwrap();
        let expected_time = now.to_string();
        tokio::spawn(async move {
            let (reader, mut writer) = split(server);
            let mut lines = BufReader::new(reader).lines();
            let request: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(request["method"], "turn/start");
            assert_eq!(
                request["params"]["input"][0]["text"],
                format!("20 minutes have passed. It is now {expected_time}.")
            );
            writer
                .write_all(
                    format!(
                        "{{\"id\":{},\"result\":{{\"turn\":{{\"id\":\"turn-2\"}}}}}}\n",
                        request["id"]
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });

        manager.heartbeat_tick(now).await;

        let agent = manager.store.agent(agent.id).await.unwrap().unwrap();
        assert_eq!(agent.state, AgentState::Running);
        assert!(agent.idle_since.is_none());
        let events = manager.store.wake_events(agent.id).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind(), "idle.heartbeat");
        assert_eq!(
            events[0].delivery_state,
            crate::domain::WakeEventState::Delivered
        );
    }

    #[tokio::test]
    async fn due_reminder_starts_an_idle_turn_and_is_recorded_once() {
        let (manager, agent, _rpc, server) = manager_with_fake_agent(AgentState::Idle, None).await;
        let now = Timestamp::now();
        manager
            .store
            .create_reminder(
                agent.id,
                "Check the deployment",
                now.as_second(),
                now.as_second() - 1,
            )
            .await
            .unwrap();
        let requested = Timestamp::from_second(now.as_second()).unwrap().to_string();
        tokio::spawn(async move {
            let (reader, mut writer) = split(server);
            let mut lines = BufReader::new(reader).lines();
            let request: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            let text = request["params"]["input"][0]["text"].as_str().unwrap();
            assert_eq!(request["method"], "turn/start");
            assert!(text.contains(&format!("Reminder scheduled for {requested}.")));
            assert!(text.contains("Actual delivery time:"));
            assert!(text.ends_with("Check the deployment"));
            writer
                .write_all(
                    format!(
                        "{{\"id\":{},\"result\":{{\"turn\":{{\"id\":\"turn-2\"}}}}}}\n",
                        request["id"]
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });

        manager.reminder_tick(now).await;
        manager.reminder_tick(now).await;

        let events = manager.store.wake_events(agent.id).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].delivery_state,
            crate::domain::WakeEventState::Delivered
        );
        let (history, _) = manager
            .store
            .agent_messages(agent.id, None, None, None)
            .await
            .unwrap();
        assert_eq!(history.len(), 1);
        assert!(history[0].body.contains("Actual delivery time:"));
        assert!(history[0].body.ends_with("Check the deployment"));
    }

    #[tokio::test]
    async fn due_reminder_steers_an_active_turn() {
        let (manager, agent, _rpc, server) =
            manager_with_fake_agent(AgentState::Running, Some("turn-1")).await;
        let now = Timestamp::now();
        manager
            .store
            .create_reminder(agent.id, "Check logs", now.as_second(), now.as_second() - 1)
            .await
            .unwrap();
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
                    .ends_with("Check logs")
            );
            writer
                .write_all(
                    format!(
                        "{{\"id\":{},\"result\":{{\"turnId\":\"turn-1\"}}}}\n",
                        request["id"]
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });

        manager.reminder_tick(now).await;

        assert_eq!(manager.store.wake_events(agent.id).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn due_reminders_stay_pending_while_agents_cannot_run() {
        for state in [AgentState::Paused, AgentState::Error] {
            let store = Store::memory().await.unwrap();
            let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
            store
                .update_agent_runtime(agent.id, state, None, None, Some("not ready"))
                .await
                .unwrap();
            let manager = AgentManager::new(
                store.clone(),
                "codex".into(),
                "multi-agent-board".into(),
                "http://127.0.0.1:1/api".into(),
            );
            let now = Timestamp::now();
            store
                .create_reminder(agent.id, "Wait", now.as_second(), now.as_second() - 1)
                .await
                .unwrap();

            manager.reminder_tick(now).await;

            let events = store.wake_events(agent.id).await.unwrap();
            assert_eq!(events.len(), 1);
            assert_eq!(
                events[0].delivery_state,
                crate::domain::WakeEventState::Pending
            );
        }
    }

    #[tokio::test]
    async fn steer_race_starts_a_replacement_turn_and_ignores_stale_completion() {
        let (manager, agent, rpc, server) =
            manager_with_fake_agent(AgentState::Running, Some("turn-1")).await;
        tokio::spawn(async move {
            let (reader, mut writer) = split(server);
            let mut lines = BufReader::new(reader).lines();
            let steer: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(steer["method"], "turn/steer");
            writer
                .write_all(
                    format!(
                        "{{\"id\":{},\"error\":{{\"message\":\"turn completed\"}}}}\n",
                        steer["id"]
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            let start: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(start["method"], "turn/start");
            writer
                .write_all(
                    format!(
                        "{{\"id\":{},\"result\":{{\"turn\":{{\"id\":\"turn-2\"}}}}}}\n",
                        start["id"]
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });

        manager.send_message(agent.id, "Race safely").await.unwrap();
        manager
            .handle_event(
                agent.id,
                &rpc,
                json!({
                    "method": "turn/completed",
                    "params": {"turn": {"id": "turn-1", "status": "completed"}}
                }),
            )
            .await;

        let updated = manager.store.agent(agent.id).await.unwrap().unwrap();
        assert_eq!(updated.state, AgentState::Running);
        assert_eq!(updated.active_turn_id.as_deref(), Some("turn-2"));
    }

    #[tokio::test]
    async fn messages_stay_pending_while_paused_or_in_error() {
        for state in [AgentState::Paused, AgentState::Error] {
            let store = Store::memory().await.unwrap();
            let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
            store
                .update_agent_runtime(agent.id, state, None, None, Some("not ready"))
                .await
                .unwrap();
            let manager = AgentManager::new(
                store.clone(),
                "codex".into(),
                "multi-agent-board".into(),
                "http://127.0.0.1:1/api".into(),
            );

            let unchanged = manager.send_message(agent.id, "Wait here").await.unwrap();

            assert_eq!(unchanged.state, state);
            assert_eq!(
                store
                    .agent_messages(agent.id, None, None, None)
                    .await
                    .unwrap()
                    .0
                    .len(),
                1
            );
            assert_eq!(
                store.wake_events(agent.id).await.unwrap()[0].delivery_state,
                crate::domain::WakeEventState::Pending
            );
        }
    }

    #[tokio::test]
    async fn renaming_a_started_agent_refreshes_its_runtime_identity() {
        let store = Store::memory().await.unwrap();
        let agent = store.create_agent("worker", "/tmp/worker").await.unwrap();
        let agent = store
            .update_agent_runtime(agent.id, AgentState::Idle, Some("thread-1"), None, None)
            .await
            .unwrap();
        let manager = AgentManager::new(
            store,
            "codex".into(),
            "multi-agent-board".into(),
            "http://127.0.0.1:1/api".into(),
        );
        let (client, _server) = duplex(4096);
        let (reader, writer) = split(client);
        let (rpc, _events) = RpcPeer::attach(BufReader::new(reader), writer);
        let child = Command::new("sh").args(["-c", "sleep 60"]).spawn().unwrap();
        manager.processes.lock().await.insert(
            agent.id,
            Arc::new(RunningAgent {
                rpc,
                child: Mutex::new(child),
            }),
        );

        let renamed = manager.rename(agent.id, "builder").await.unwrap();

        assert!(!manager.processes.lock().await.contains_key(&agent.id));
        assert!(agent_instructions(&renamed).contains("agent name is 'builder'"));
        assert!(!agent_instructions(&renamed).contains("agent name is 'worker'"));
    }
}

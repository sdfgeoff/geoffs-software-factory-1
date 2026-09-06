use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
    time::Duration,
};

use jiff::Timestamp;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::{Mutex, broadcast, mpsc, oneshot},
    time::timeout,
};

use crate::{
    domain::{Agent, AgentState, WakeDeliveryPolicy, WakeEvent},
    error::AppError,
    store::{Store, validate_channel},
};

mod reviews;

const COORDINATION_INSTRUCTIONS: &str = "You are a persistent coding agent coordinated by Multi-agent Board. Work only in your assigned Git worktree. Use the multi_agent_board MCP tools to inspect and update tickets and to communicate through message-board channels. Claim a ticket before starting it, commit cohesive changes, and submit it for review when ready. If you have approve_review scope, claim reviews before reviewing them. Keep ticket state accurate. Do not modify another agent's worktree or the primary worktree.";

#[derive(Clone, Debug, Serialize)]
pub struct AgentEvent {
    pub agent_id: i64,
    pub kind: String,
    pub data: Value,
}

#[derive(Clone)]
pub struct AgentManager {
    store: Store,
    codex_path: PathBuf,
    executable_path: PathBuf,
    board_url: String,
    merge_notification_channel: Option<String>,
    processes: Arc<Mutex<HashMap<i64, Arc<RunningAgent>>>>,
    // ponytail: a global operation lock is enough for a local MVP; use per-agent locks if contention appears.
    operations: Arc<Mutex<()>>,
    events: broadcast::Sender<AgentEvent>,
}

struct RunningAgent {
    rpc: RpcPeer,
    child: Mutex<Child>,
}

#[derive(Clone)]
struct RpcPeer {
    writer: Arc<Mutex<Box<dyn AsyncWrite + Send + Unpin>>>,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Value>>>>,
    next_id: Arc<AtomicI64>,
}

impl RpcPeer {
    fn attach<R, W>(reader: R, writer: W) -> (Self, mpsc::Receiver<Value>)
    where
        R: AsyncBufRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let peer = Self {
            writer: Arc::new(Mutex::new(Box::new(writer))),
            pending: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicI64::new(1)),
        };
        let pending = peer.pending.clone();
        let (events_tx, events_rx) = mpsc::channel(256);
        tokio::spawn(async move {
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                let response_id = message.get("id").and_then(Value::as_i64);
                let is_response = message.get("method").is_none();
                if let Some(id) = response_id
                    && is_response
                    && let Some(sender) = pending.lock().await.remove(&id)
                {
                    let _ = sender.send(message);
                    continue;
                }
                if events_tx.send(message).await.is_err() {
                    break;
                }
            }
        });
        (peer, events_rx)
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, AppError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(id, sender);
        if let Err(error) = self
            .send(json!({"method": method, "id": id, "params": params}))
            .await
        {
            self.pending.lock().await.remove(&id);
            return Err(error);
        }
        let response = timeout(Duration::from_secs(30), receiver)
            .await
            .map_err(|_| AppError::internal(format!("Codex timed out responding to {method}")))?
            .map_err(|_| AppError::internal("Codex connection closed"))?;
        if let Some(error) = response.get("error") {
            return Err(AppError::internal(format!(
                "Codex {method} failed: {error}"
            )));
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| AppError::internal("Codex response did not contain a result"))
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), AppError> {
        self.send(json!({"method": method, "params": params})).await
    }

    async fn respond(&self, id: Value, result: Value) -> Result<(), AppError> {
        self.send(json!({"id": id, "result": result})).await
    }

    async fn send(&self, message: Value) -> Result<(), AppError> {
        let mut writer = self.writer.lock().await;
        writer
            .write_all(format!("{message}\n").as_bytes())
            .await
            .map_err(|error| AppError::internal(format!("failed to write to Codex: {error}")))?;
        writer
            .flush()
            .await
            .map_err(|error| AppError::internal(format!("failed to flush Codex input: {error}")))
    }
}

impl AgentManager {
    pub fn new(
        store: Store,
        codex_path: PathBuf,
        executable_path: PathBuf,
        board_url: String,
    ) -> Self {
        let (events, _) = broadcast::channel(512);
        Self {
            store,
            codex_path,
            executable_path,
            board_url,
            merge_notification_channel: None,
            processes: Arc::new(Mutex::new(HashMap::new())),
            operations: Arc::new(Mutex::new(())),
            events,
        }
    }

    pub fn with_merge_notification_channel(
        mut self,
        channel: Option<String>,
    ) -> Result<Self, AppError> {
        if let Some(channel) = channel.as_deref() {
            validate_channel(channel)?;
        }
        self.merge_notification_channel = channel;
        Ok(self)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AgentEvent> {
        self.events.subscribe()
    }

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
            self.heartbeat_tick(Timestamp::now()).await;
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
                        "idle.heartbeat",
                        json!({"message": message, "minutes": minutes, "occurred_at": now.to_string()}),
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

    pub async fn list(&self) -> Result<Vec<Agent>, AppError> {
        Ok(self.store.agents().await?)
    }

    pub async fn scopes(&self, agent_id: i64) -> Result<Vec<String>, AppError> {
        self.store.agent_scopes(agent_id).await
    }

    pub async fn set_scopes(
        &self,
        agent_id: i64,
        scopes: &[String],
    ) -> Result<Vec<String>, AppError> {
        self.store.set_agent_scopes(agent_id, scopes).await
    }

    pub async fn channel_subscriptions(&self, agent_id: i64) -> Result<Vec<String>, AppError> {
        self.store.agent_channel_subscriptions(agent_id).await
    }

    pub async fn set_channel_subscriptions(
        &self,
        agent_id: i64,
        channels: &[String],
    ) -> Result<Vec<String>, AppError> {
        let channels = self
            .store
            .set_agent_channel_subscriptions(agent_id, channels)
            .await?;
        self.emit(agent_id, "channel_subscriptions", json!(channels));
        Ok(channels)
    }

    pub async fn add_channel_subscriptions(
        &self,
        agent_id: i64,
        channels: &[String],
    ) -> Result<Vec<String>, AppError> {
        let channels = self
            .store
            .add_agent_channel_subscriptions(agent_id, channels)
            .await?;
        self.emit(agent_id, "channel_subscriptions", json!(channels));
        Ok(channels)
    }

    pub async fn remove_channel_subscriptions(
        &self,
        agent_id: i64,
        channels: &[String],
    ) -> Result<Vec<String>, AppError> {
        let channels = self
            .store
            .remove_agent_channel_subscriptions(agent_id, channels)
            .await?;
        self.emit(agent_id, "channel_subscriptions", json!(channels));
        Ok(channels)
    }

    pub async fn set_wake_settings(
        &self,
        agent_id: i64,
        wake_on_pending_work: bool,
        wake_on_pending_review: bool,
        idle_heartbeat_minutes: Option<i64>,
    ) -> Result<Agent, AppError> {
        let _operation = self.operations.lock().await;
        let agent = self
            .store
            .set_agent_wake_settings(
                agent_id,
                wake_on_pending_work,
                wake_on_pending_review,
                idle_heartbeat_minutes,
            )
            .await?;
        self.emit(agent_id, "agent_updated", json!(agent));
        Ok(agent)
    }

    pub async fn create(&self, name: &str) -> Result<Agent, AppError> {
        let _operation = self.operations.lock().await;
        validate_agent_name(name)?;
        if self.store.agent_named(name).await?.is_some() {
            return Err(AppError::conflict("agent name already exists"));
        }
        let project =
            self.store.project().await?.ok_or_else(|| {
                AppError::bad_request("select a project before creating an agent")
            })?;
        let root = PathBuf::from(project.path);
        let worktree = worktree_path(&root, name)?;
        if worktree.exists() {
            return Err(AppError::conflict("agent worktree path already exists"));
        }
        std::fs::create_dir_all(worktree.parent().unwrap()).map_err(|error| {
            AppError::internal(format!("failed to create worktree directory: {error}"))
        })?;
        let output = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["worktree", "add", "-b"])
            .arg(format!("agents/{name}"))
            .arg(&worktree)
            .arg("HEAD")
            .output()
            .await
            .map_err(|error| AppError::internal(format!("failed to start Git: {error}")))?;
        if !output.status.success() {
            return Err(AppError::bad_request(format!(
                "failed to create worktree: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        let path = worktree
            .to_str()
            .ok_or_else(|| AppError::bad_request("worktree path is not valid UTF-8"))?;
        let agent = self.store.create_agent(name, path).await?;
        self.emit(agent.id, "agent_created", json!(agent));
        Ok(agent)
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
        self.emit(agent.id, "agent_updated", json!(agent));
        Ok(agent)
    }

    pub async fn messages(
        &self,
        agent_id: i64,
    ) -> Result<Vec<crate::domain::AgentMessage>, AppError> {
        self.store.agent_messages(agent_id).await
    }

    pub async fn send_message(&self, agent_id: i64, message: &str) -> Result<Agent, AppError> {
        {
            let _operation = self.operations.lock().await;
            if self.store.agent(agent_id).await?.is_none() {
                return Err(AppError::not_found("agent not found"));
            }
            let user_message = self.store.enqueue_agent_message(agent_id, message).await?;
            self.emit(agent_id, "message", json!(user_message));
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
        let result = running
            .rpc
            .request(
                "turn/start",
                turn_params(&thread_id, message, &agent.worktree_path),
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

    async fn notify_agent(&self, agent_id: i64, message: &str) -> Result<(), AppError> {
        self.store.enqueue_notification(agent_id, message).await?;
        self.deliver_pending(agent_id).await;
        Ok(())
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
            let message = wake_event_message(&event);
            let delivered = match agent.state {
                AgentState::Idle => self.start_inbox_turn(&agent, &message).await,
                AgentState::Running | AgentState::AwaitingApproval => {
                    self.steer_inbox_turn(&agent, &message).await
                }
                AgentState::Paused | AgentState::Error => unreachable!(),
            };
            if delivered.is_err() {
                let _ = self.store.retry_wake_event(event.id).await;
                return;
            }
            if event.kind != "message"
                && let Ok(message) = self
                    .store
                    .add_agent_message(agent_id, "system", &message)
                    .await
            {
                self.emit(agent_id, "message", json!(message));
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
        self.emit(agent_id, "agent_updated", json!(agent));
        Ok(agent)
    }

    pub async fn resume(&self, agent_id: i64) -> Result<Agent, AppError> {
        let operation = self.operations.lock().await;
        let agent = self
            .store
            .update_agent_runtime(agent_id, AgentState::Idle, None, None, None)
            .await?;
        self.emit(agent_id, "agent_updated", json!(agent));
        drop(operation);
        let manager = self.clone();
        tokio::spawn(async move { manager.deliver_pending(agent_id).await });
        Ok(agent)
    }

    async fn ensure_process(&self, agent: &Agent) -> Result<Arc<RunningAgent>, AppError> {
        if let Some(running) = self.processes.lock().await.get(&agent.id).cloned() {
            return Ok(running);
        }
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
                "threadId": thread_id, "cwd": agent.worktree_path,
                "approvalPolicy": "never", "sandbox": "danger-full-access",
                "developerInstructions": instructions
            })
        } else {
            json!({
                "cwd": agent.worktree_path, "approvalPolicy": "never", "sandbox": "danger-full-access",
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
            self.emit(agent_id, "approval_declined", event.clone());
            let _ = rpc
                .respond(event["id"].clone(), json!({"decision": "decline"}))
                .await;
            return;
        }
        match method {
            "item/agentMessage/delta" => {
                self.emit(agent_id, "agent_message_delta", event["params"].clone())
            }
            "item/completed"
                if event.pointer("/params/item/type").and_then(Value::as_str)
                    == Some("agentMessage") =>
            {
                if let Some(text) = event.pointer("/params/item/text").and_then(Value::as_str)
                    && let Ok(message) = self.store.add_agent_message(agent_id, "agent", text).await
                {
                    self.emit(agent_id, "message", json!(message));
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
                    self.emit(agent_id, "agent_updated", json!(agent));
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
            _ => self.emit(agent_id, "codex_event", event),
        }
    }

    async fn fail_agent(&self, agent_id: i64, message: &str) {
        if let Ok(agent) = self
            .store
            .update_agent_runtime(agent_id, AgentState::Error, None, None, Some(message))
            .await
        {
            self.emit(agent_id, "agent_updated", json!(agent));
        }
    }

    fn emit(&self, agent_id: i64, kind: &str, data: Value) {
        let _ = self.events.send(AgentEvent {
            agent_id,
            kind: kind.into(),
            data,
        });
    }
}

fn validate_agent_name(name: &str) -> Result<(), AppError> {
    if name.is_empty()
        || name.len() > 50
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(AppError::bad_request(
            "agent names may contain 1-50 letters, numbers, '-' or '_'",
        ));
    }
    Ok(())
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
    event
        .payload
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("Agent inbox event `{}`:\n{}", event.kind, event.payload))
}

fn worktree_path(root: &Path, name: &str) -> Result<PathBuf, AppError> {
    let parent = root
        .parent()
        .ok_or_else(|| AppError::bad_request("project root cannot be a filesystem root"))?;
    let project = root
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| AppError::bad_request("project folder name is not valid UTF-8"))?;
    Ok(parent
        .join(format!(".{project}-agent-worktrees"))
        .join(name))
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, duplex, split};

    use super::*;

    #[tokio::test]
    async fn rpc_peer_talks_to_a_fake_app_server() {
        let (client, server) = duplex(4096);
        let (client_read, client_write) = split(client);
        let (server_read, mut server_write) = split(server);
        let (peer, _events) = RpcPeer::attach(BufReader::new(client_read), client_write);
        tokio::spawn(async move {
            let mut lines = BufReader::new(server_read).lines();
            let request: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(request["method"], "initialize");
            server_write
                .write_all(format!("{{\"id\":{},\"result\":{{}}}}\n", request["id"]).as_bytes())
                .await
                .unwrap();
        });

        assert_eq!(
            peer.request("initialize", json!({})).await.unwrap(),
            json!({})
        );
    }

    #[test]
    fn agent_names_are_safe_for_branches_and_paths() {
        assert!(validate_agent_name("backend-1").is_ok());
        assert!(validate_agent_name("../escape").is_err());
        assert!(validate_agent_name("spaces are bad").is_err());
    }

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
            manager.store.agent_messages(agent.id).await.unwrap().len(),
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
    async fn ticket_wakes_wait_for_idle_without_blocking_channel_messages() {
        let (manager, agent, _rpc, server) =
            manager_with_fake_agent(AgentState::Running, Some("turn-1")).await;
        for (kind, message) in [
            ("ticket.available", "Pending implementation work"),
            ("ticket.review_requested", "Pending review"),
        ] {
            manager
                .store
                .enqueue_wake_event(
                    agent.id,
                    kind,
                    json!({"message": message}),
                    None,
                    WakeDeliveryPolicy::IdleOnly,
                )
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
        assert_eq!(events[0].kind, "idle.heartbeat");
        assert_eq!(
            events[0].delivery_state,
            crate::domain::WakeEventState::Delivered
        );
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
            assert_eq!(store.agent_messages(agent.id).await.unwrap().len(), 1);
            assert_eq!(
                store.wake_events(agent.id).await.unwrap()[0].delivery_state,
                crate::domain::WakeEventState::Pending
            );
        }
    }

    #[tokio::test]
    async fn agents_receive_separate_git_worktrees() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!("mab-agent-test-{unique}"));
        let repository = base.join("repo");
        std::fs::create_dir_all(&repository).unwrap();
        for arguments in [
            vec!["init", "-q"],
            vec!["config", "user.name", "Test"],
            vec!["config", "user.email", "test@example.com"],
            vec!["commit", "--allow-empty", "-qm", "initial"],
        ] {
            assert!(
                std::process::Command::new("git")
                    .args(arguments)
                    .current_dir(&repository)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        let store = Store::memory().await.unwrap();
        store
            .set_project(repository.to_str().unwrap())
            .await
            .unwrap();
        let manager = AgentManager::new(
            store,
            "codex".into(),
            "multi-agent-board".into(),
            "http://127.0.0.1:1/api".into(),
        );

        let first = manager.create("backend").await.unwrap();
        let second = manager.create("frontend").await.unwrap();

        assert_ne!(first.worktree_path, second.worktree_path);
        assert!(Path::new(&first.worktree_path).join(".git").exists());
        assert!(Path::new(&second.worktree_path).join(".git").exists());
        std::fs::remove_dir_all(base).unwrap();
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

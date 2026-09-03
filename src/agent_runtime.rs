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

use serde::Serialize;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    process::{Child, Command},
    sync::{Mutex, broadcast, mpsc, oneshot},
    time::timeout,
};

use crate::{
    domain::{Agent, AgentState},
    error::AppError,
    store::Store,
};

const COORDINATION_INSTRUCTIONS: &str = "You are a persistent coding agent coordinated by Multi-agent Board. Work only in your assigned Git worktree. Use the multi_agent_board MCP tools to inspect and update tickets and to communicate through message-board channels. Claim a ticket before starting it and keep its state accurate. Commit cohesive changes often. Do not modify another agent's worktree.";

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
    data_path: PathBuf,
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
        data_path: PathBuf,
    ) -> Self {
        let (events, _) = broadcast::channel(512);
        Self {
            store,
            codex_path,
            executable_path,
            data_path,
            processes: Arc::new(Mutex::new(HashMap::new())),
            operations: Arc::new(Mutex::new(())),
            events,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AgentEvent> {
        self.events.subscribe()
    }

    pub async fn list(&self) -> Result<Vec<Agent>, AppError> {
        Ok(self.store.agents().await?)
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

    pub async fn messages(
        &self,
        agent_id: i64,
    ) -> Result<Vec<crate::domain::AgentMessage>, AppError> {
        self.store.agent_messages(agent_id).await
    }

    pub async fn send_message(&self, agent_id: i64, message: &str) -> Result<Agent, AppError> {
        let _operation = self.operations.lock().await;
        let agent = self
            .store
            .agent(agent_id)
            .await?
            .ok_or_else(|| AppError::not_found("agent not found"))?;
        match agent.state {
            AgentState::Paused => {
                return Err(AppError::conflict("resume the agent before messaging it"));
            }
            AgentState::Running | AgentState::AwaitingApproval => {
                return Err(AppError::conflict("agent already has an active turn"));
            }
            AgentState::Idle | AgentState::Error => {}
        }
        let user_message = self
            .store
            .add_agent_message(agent_id, "user", message)
            .await?;
        self.emit(agent_id, "message", json!(user_message));
        self.store
            .update_agent_runtime(agent_id, AgentState::Running, None, None, None)
            .await?;

        let running = match self.ensure_process(&agent).await {
            Ok(running) => running,
            Err(error) => {
                self.fail_agent(agent_id, &error.to_string()).await;
                return Err(error);
            }
        };
        let thread_id = self
            .store
            .agent(agent_id)
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
                let turn_id = result.pointer("/turn/id").and_then(Value::as_str);
                let updated = self
                    .store
                    .update_agent_runtime(agent_id, AgentState::Running, None, turn_id, None)
                    .await?;
                Ok(updated)
            }
            Err(error) => {
                self.fail_agent(agent_id, &error.to_string()).await;
                Err(error)
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
        let _operation = self.operations.lock().await;
        let agent = self
            .store
            .update_agent_runtime(agent_id, AgentState::Idle, None, None, None)
            .await?;
        self.emit(agent_id, "agent_updated", json!(agent));
        Ok(agent)
    }

    async fn ensure_process(&self, agent: &Agent) -> Result<Arc<RunningAgent>, AppError> {
        if let Some(running) = self.processes.lock().await.get(&agent.id).cloned() {
            return Ok(running);
        }
        let command_value = serde_json::to_string(&self.executable_path.to_string_lossy().as_ref())
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
            .env("MULTI_AGENT_BOARD_DATA", &self.data_path)
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

        let instructions = format!(
            "{COORDINATION_INSTRUCTIONS} Your board agent ID is {} and your agent name is '{}'.",
            agent.id, agent.name
        );
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
        if agent.thread_id.is_none() {
            let _ = rpc
                .request(
                    "thread/name/set",
                    json!({"threadId": thread_id, "name": agent.name}),
                )
                .await;
        }
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
                if let Ok(agent) = self
                    .store
                    .update_agent_runtime(agent_id, state, None, None, error)
                    .await
                {
                    self.emit(agent_id, "agent_updated", json!(agent));
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

fn turn_params(thread_id: &str, message: &str, worktree_path: &str) -> Value {
    json!({
        "threadId": thread_id,
        "input": [{"type": "text", "text": message}],
        "cwd": worktree_path,
        "approvalPolicy": "never",
        "sandboxPolicy": {"type": "dangerFullAccess"}
    })
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

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, duplex, split};

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
            ":memory:".into(),
        );

        let first = manager.create("backend").await.unwrap();
        let second = manager.create("frontend").await.unwrap();

        assert_ne!(first.worktree_path, second.worktree_path);
        assert!(Path::new(&first.worktree_path).join(".git").exists());
        assert!(Path::new(&second.worktree_path).join(".git").exists());
        std::fs::remove_dir_all(base).unwrap();
    }
}

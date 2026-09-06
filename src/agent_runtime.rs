use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::Serialize;
use serde_json::{Value, json};
use tokio::{
    process::{Child, Command},
    sync::{Mutex, broadcast},
};

use crate::{
    domain::Agent,
    error::AppError,
    store::{Store, validate_channel},
};

mod inbox;
mod reviews;
mod rpc;

use rpc::RpcPeer;

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

    use super::*;

    #[test]
    fn agent_names_are_safe_for_branches_and_paths() {
        assert!(validate_agent_name("backend-1").is_ok());
        assert!(validate_agent_name("../escape").is_err());
        assert!(validate_agent_name("spaces are bad").is_err());
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
}

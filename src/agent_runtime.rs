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
    domain::{Agent, AgentProjectWorktree, BoardSettings, ProjectTransition, Ticket},
    error::AppError,
    store::{MERGE_NOTIFICATION_CHANNEL_DESCRIPTION, Store, validate_channel},
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
    merge_notification_channel_fallback: Option<String>,
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
            merge_notification_channel_fallback: None,
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
        self.merge_notification_channel_fallback = channel;
        Ok(self)
    }

    pub async fn settings(&self) -> Result<BoardSettings, AppError> {
        let merge_notification_channel = self
            .store
            .saved_merge_notification_channel()
            .await?
            .unwrap_or_else(|| self.merge_notification_channel_fallback.clone());
        if let Some(channel) = merge_notification_channel.as_deref() {
            self.store
                .ensure_channel(channel, MERGE_NOTIFICATION_CHANNEL_DESCRIPTION)
                .await?;
        }
        Ok(BoardSettings {
            merge_notification_channel,
        })
    }

    pub async fn set_settings(
        &self,
        merge_notification_channel: Option<&str>,
    ) -> Result<BoardSettings, AppError> {
        let _operation = self.operations.lock().await;
        let merge_notification_channel = self
            .store
            .set_merge_notification_channel(merge_notification_channel)
            .await?;
        let settings = BoardSettings {
            merge_notification_channel,
        };
        self.emit(0, "settings_updated", json!(settings));
        Ok(settings)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AgentEvent> {
        self.events.subscribe()
    }

    pub async fn list(&self) -> Result<Vec<Agent>, AppError> {
        Ok(self.store.agents().await?)
    }

    pub async fn project_worktrees(
        &self,
        agent_id: i64,
    ) -> Result<Vec<AgentProjectWorktree>, AppError> {
        self.store.agent_project_worktrees(agent_id).await
    }

    pub(crate) async fn prepare_ticket_project(
        &self,
        agent_id: i64,
        ticket_id: i64,
    ) -> Result<(), AppError> {
        let ticket = self
            .store
            .ticket(ticket_id)
            .await?
            .ok_or_else(|| AppError::not_found("ticket not found"))?;
        let project = self.store.active_ticket_project(&ticket).await?;
        self.worktree_for_project(agent_id, project.id).await?;
        Ok(())
    }

    pub(crate) async fn transition_to_ticket_project(
        &self,
        agent_id: i64,
        ticket: &Ticket,
    ) -> Result<Option<ProjectTransition>, AppError> {
        let agent = self
            .store
            .agent(agent_id)
            .await?
            .ok_or_else(|| AppError::not_found("agent not found"))?;
        if agent.active_project_id == Some(ticket.project_id) {
            return Ok(None);
        }
        let project = self.store.active_ticket_project(ticket).await?;
        let worktree = self.worktree_for_project(agent_id, project.id).await?;
        let old_project = match agent.active_project_id {
            Some(id) => self
                .store
                .project_by_id(id)
                .await?
                .map(|project| format!("{} (#{})", project.name, project.id))
                .unwrap_or_else(|| format!("unknown project (#{id})")),
            None => "no project".into(),
        };
        let message = format!(
            "Project context changed for ticket #{}: {old_project} → {} (#{}). Working directory: {}",
            ticket.id, project.name, project.id, worktree.path
        );
        let updated = self
            .store
            .set_agent_active_project(agent_id, Some(project.id))
            .await?;
        self.emit(agent_id, "agent_updated", json!(updated));
        Ok(Some(ProjectTransition {
            agent_id,
            ticket_id: ticket.id,
            previous_project_id: agent.active_project_id,
            project_id: project.id,
            project_name: project.name,
            worktree_path: worktree.path,
            message,
        }))
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

    pub async fn project_subscriptions(&self, agent_id: i64) -> Result<Vec<i64>, AppError> {
        self.store.agent_project_subscriptions(agent_id).await
    }

    pub async fn set_project_subscriptions(
        &self,
        agent_id: i64,
        project_ids: &[i64],
    ) -> Result<Vec<i64>, AppError> {
        let project_ids = self
            .store
            .set_agent_project_subscriptions(agent_id, project_ids)
            .await?;
        self.emit(agent_id, "project_subscriptions", json!(project_ids));
        Ok(project_ids)
    }

    pub async fn add_project_subscriptions(
        &self,
        agent_id: i64,
        project_ids: &[i64],
    ) -> Result<Vec<i64>, AppError> {
        let project_ids = self
            .store
            .add_agent_project_subscriptions(agent_id, project_ids)
            .await?;
        self.emit(agent_id, "project_subscriptions", json!(project_ids));
        Ok(project_ids)
    }

    pub async fn remove_project_subscriptions(
        &self,
        agent_id: i64,
        project_ids: &[i64],
    ) -> Result<Vec<i64>, AppError> {
        let project_ids = self
            .store
            .remove_agent_project_subscriptions(agent_id, project_ids)
            .await?;
        self.emit(agent_id, "project_subscriptions", json!(project_ids));
        Ok(project_ids)
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
        let homes_root = agent_homes_root(&root)?;
        let homes_root = homes_root
            .to_str()
            .ok_or_else(|| AppError::bad_request("agent home root is not valid UTF-8"))?;
        let (agent, worktree) = self
            .store
            .create_agent_for_project(name, homes_root, project.id)
            .await?;
        if let Err(error) = std::fs::create_dir_all(&agent.home_path) {
            self.store.delete_agent(agent.id).await?;
            return Err(AppError::internal(format!(
                "failed to create agent home: {error}"
            )));
        }
        let worktree_path = PathBuf::from(&worktree.path);
        if let Err(error) = std::fs::create_dir_all(worktree_path.parent().unwrap()) {
            self.store.delete_agent(agent.id).await?;
            return Err(AppError::internal(format!(
                "failed to create worktree directory: {error}"
            )));
        }
        let output = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["worktree", "add", "-b"])
            .arg(format!("agents/{}", agent.id))
            .arg(&worktree_path)
            .arg(&project.default_branch)
            .output()
            .await
            .map_err(|error| AppError::internal(format!("failed to start Git: {error}")))?;
        if !output.status.success() {
            self.store.delete_agent(agent.id).await?;
            return Err(AppError::bad_request(format!(
                "failed to create worktree: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        self.emit(agent.id, "agent_created", json!(agent));
        Ok(agent)
    }

    pub(crate) async fn worktree_for_project(
        &self,
        agent_id: i64,
        project_id: i64,
    ) -> Result<AgentProjectWorktree, AppError> {
        if let Some(worktree) = self
            .store
            .agent_project_worktree(agent_id, project_id)
            .await?
        {
            return Ok(worktree);
        }
        let agent = self
            .store
            .agent(agent_id)
            .await?
            .ok_or_else(|| AppError::not_found("agent not found"))?;
        let project = self
            .store
            .project_by_id(project_id)
            .await?
            .ok_or_else(|| AppError::not_found("project not found"))?;
        if project.archived_at.is_some() {
            return Err(AppError::conflict("agent project is archived"));
        }
        let path = PathBuf::from(&agent.home_path)
            .join("projects")
            .join(project.id.to_string());
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|error| {
            AppError::internal(format!("failed to create worktree directory: {error}"))
        })?;
        let output = Command::new("git")
            .arg("-C")
            .arg(&project.path)
            .args(["worktree", "add", "-b"])
            .arg(format!("agents/{agent_id}"))
            .arg(&path)
            .arg(&project.default_branch)
            .output()
            .await
            .map_err(|error| AppError::internal(format!("failed to start Git: {error}")))?;
        if !output.status.success() {
            return Err(AppError::bad_request(format!(
                "failed to create worktree: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        let path = path
            .to_str()
            .ok_or_else(|| AppError::bad_request("agent worktree path is not valid UTF-8"))?;
        self.store
            .add_agent_project_worktree(agent_id, project_id, path)
            .await
    }

    async fn current_worktree(&self, agent_id: i64) -> Result<AgentProjectWorktree, AppError> {
        let agent = self
            .store
            .agent(agent_id)
            .await?
            .ok_or_else(|| AppError::not_found("agent not found"))?;
        let project_id = match agent.active_project_id {
            Some(project_id) => project_id,
            None => {
                self.store
                    .project()
                    .await?
                    .ok_or_else(|| AppError::bad_request("project is not selected"))?
                    .id
            }
        };
        self.worktree_for_project(agent_id, project_id).await
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

fn agent_homes_root(root: &Path) -> Result<PathBuf, AppError> {
    let parent = root
        .parent()
        .ok_or_else(|| AppError::bad_request("project root cannot be a filesystem root"))?;
    let project = root
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| AppError::bad_request("project folder name is not valid UTF-8"))?;
    Ok(parent.join(format!(".{project}-agent-homes")))
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn initialize_repository(path: &Path) -> String {
        std::fs::create_dir_all(path).unwrap();
        for arguments in [
            vec!["init", "-q"],
            vec!["config", "user.name", "Test"],
            vec!["config", "user.email", "test@example.com"],
            vec!["commit", "--allow-empty", "-qm", "initial"],
        ] {
            assert!(
                std::process::Command::new("git")
                    .args(arguments)
                    .current_dir(path)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        String::from_utf8(
            std::process::Command::new("git")
                .args(["branch", "--show-current"])
                .current_dir(path)
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .into()
    }

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
        initialize_repository(&repository);
        let store = Store::open(&base.join("board.sqlite3")).await.unwrap();
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

        let first = manager.create("backend").await.unwrap();
        let second = manager.create("frontend").await.unwrap();
        let first_worktree = store
            .agent_project_worktree(first.id, project.id)
            .await
            .unwrap()
            .unwrap();
        let second_worktree = store
            .agent_project_worktree(second.id, project.id)
            .await
            .unwrap()
            .unwrap();

        assert_ne!(first.home_path, second.home_path);
        assert_eq!(
            Path::new(&first.home_path)
                .file_name()
                .unwrap()
                .to_string_lossy(),
            first.id.to_string()
        );
        assert!(!Path::new(&first.home_path).join(".git").exists());
        assert!(Path::new(&first_worktree.path).join(".git").exists());
        assert!(Path::new(&second_worktree.path).join(".git").exists());
        let renamed = store.rename_agent(first.id, "renamed").await.unwrap();
        assert_eq!(renamed.home_path, first.home_path);
        assert_eq!(
            store
                .agent_project_worktree(first.id, project.id)
                .await
                .unwrap()
                .unwrap()
                .path,
            first_worktree.path
        );

        let other_repository = base.join("other-repo");
        let other_default_branch = initialize_repository(&other_repository);
        let other = store
            .register_project(
                "Other",
                other_repository.to_str().unwrap(),
                &other_default_branch,
            )
            .await
            .unwrap();
        let first_other = manager
            .worktree_for_project(first.id, other.id)
            .await
            .unwrap();
        let reused = manager
            .worktree_for_project(first.id, other.id)
            .await
            .unwrap();
        let second_other = manager
            .worktree_for_project(second.id, other.id)
            .await
            .unwrap();
        assert_eq!(first_other, reused);
        assert_ne!(first_other.path, second_other.path);
        assert!(Path::new(&first_other.path).join(".git").exists());

        drop(manager);
        store.close().await;
        let reopened = Store::open(&base.join("board.sqlite3")).await.unwrap();
        assert_eq!(
            reopened
                .agent_project_worktree(first.id, other.id)
                .await
                .unwrap()
                .unwrap(),
            first_other
        );
        reopened.close().await;
        std::fs::remove_dir_all(base).unwrap();
    }
}

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde_json::Value;
use tokio::{
    process::{Child, Command},
    sync::{Mutex, broadcast},
};

use crate::{
    domain::{
        Agent, AgentProjectWorktree, BoardSettings, Message, ProjectTransition, Ticket,
        TicketActivity,
    },
    error::AppError,
    store::{
        BoardEventReplay, MERGE_NOTIFICATION_CHANNEL_DESCRIPTION, Store, StoredBoardEvent,
        validate_channel,
    },
};

mod inbox;
mod reviews;
mod rpc;

use rpc::RpcPeer;

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum AgentEvent {
    Refresh,
    ProjectsChanged,
    ChannelsChanged,
    ProjectionInvalidated {
        scope: multi_agent_board_protocol::ProjectionScope,
        key: String,
        revision: Option<i64>,
    },
    AgentChanged {
        agent_id: i64,
        revision: i64,
    },
    AgentCreated {
        agent_id: i64,
        data: Agent,
    },
    AgentUpdated {
        agent_id: i64,
        data: Agent,
    },
    AgentMessageDelta {
        agent_id: i64,
        data: AgentMessageDelta,
    },
    Message {
        agent_id: i64,
        data: crate::domain::AgentMessage,
    },
    TicketUpdated {
        agent_id: i64,
        data: Ticket,
    },
    TicketActivityCreated {
        agent_id: i64,
        data: TicketActivity,
    },
    ExternalReviewApproved {
        agent_id: i64,
        data: Ticket,
    },
    ExternalChangeUpdated {
        agent_id: i64,
        data: Ticket,
    },
    ExternalMergeReturned {
        agent_id: i64,
        data: Ticket,
    },
    ExternalMergeCompleted {
        agent_id: i64,
        data: Ticket,
    },
    SettingsUpdated {
        agent_id: i64,
        data: BoardSettings,
    },
    BoardMessage {
        agent_id: i64,
        data: Message,
    },
    ChannelSubscriptions {
        agent_id: i64,
        data: Vec<String>,
    },
    ProjectSubscriptions {
        agent_id: i64,
        data: Vec<i64>,
    },
    ApprovalDeclined {
        agent_id: i64,
    },
    CodexEvent {
        agent_id: i64,
        data: Value,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentMessageDelta {
    pub delta: String,
}

#[derive(Clone, Debug)]
pub struct PublishedAgentEvent {
    pub id: Option<i64>,
    pub event: AgentEvent,
}

pub(crate) enum AgentEventReplay {
    Events(Vec<StoredBoardEvent>),
    Reset { latest_id: i64 },
}

impl TryFrom<AgentEvent> for multi_agent_board_protocol::AgentEvent {
    type Error = String;

    fn try_from(value: AgentEvent) -> Result<Self, Self::Error> {
        Ok(match value {
            AgentEvent::Refresh => Self::Refresh,
            AgentEvent::ProjectsChanged => Self::ProjectsChanged,
            AgentEvent::ChannelsChanged => Self::ChannelsChanged,
            AgentEvent::ProjectionInvalidated {
                scope,
                key,
                revision,
            } => Self::ProjectionInvalidated {
                scope,
                key,
                revision,
            },
            AgentEvent::AgentChanged { agent_id, revision } => {
                Self::AgentChanged { agent_id, revision }
            }
            AgentEvent::AgentCreated { agent_id, data } => Self::AgentCreated {
                agent_id,
                data: data.into(),
            },
            AgentEvent::AgentUpdated { agent_id, data } => Self::AgentUpdated {
                agent_id,
                data: data.into(),
            },
            AgentEvent::AgentMessageDelta { agent_id, data } => Self::AgentMessageDelta {
                agent_id,
                data: multi_agent_board_protocol::AgentMessageDelta { delta: data.delta },
            },
            AgentEvent::Message { agent_id, data } => Self::Message {
                agent_id,
                data: data.try_into()?,
            },
            AgentEvent::TicketUpdated { agent_id, data } => Self::TicketUpdated {
                agent_id,
                data: data.into(),
            },
            AgentEvent::TicketActivityCreated { agent_id, data } => Self::TicketActivityCreated {
                agent_id,
                data: data.into(),
            },
            AgentEvent::ExternalReviewApproved { agent_id, data } => Self::ExternalReviewApproved {
                agent_id,
                data: data.into(),
            },
            AgentEvent::ExternalChangeUpdated { agent_id, data } => Self::ExternalChangeUpdated {
                agent_id,
                data: data.into(),
            },
            AgentEvent::ExternalMergeReturned { agent_id, data } => Self::ExternalMergeReturned {
                agent_id,
                data: data.into(),
            },
            AgentEvent::ExternalMergeCompleted { agent_id, data } => Self::ExternalMergeCompleted {
                agent_id,
                data: data.into(),
            },
            AgentEvent::SettingsUpdated { agent_id, data } => Self::SettingsUpdated {
                agent_id,
                data: data.into(),
            },
            AgentEvent::BoardMessage { agent_id, data } => Self::BoardMessage {
                agent_id,
                data: data.into(),
            },
            AgentEvent::ChannelSubscriptions { agent_id, data } => {
                Self::ChannelSubscriptions { agent_id, data }
            }
            AgentEvent::ProjectSubscriptions { agent_id, data } => {
                Self::ProjectSubscriptions { agent_id, data }
            }
            AgentEvent::ApprovalDeclined { agent_id } => Self::ApprovalDeclined { agent_id },
            AgentEvent::CodexEvent { agent_id, data } => Self::CodexEvent { agent_id, data },
        })
    }
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
    events: broadcast::Sender<PublishedAgentEvent>,
    event_publish: Arc<Mutex<()>>,
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
            event_publish: Arc::new(Mutex::new(())),
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
        let saved = self.store.board_settings().await?;
        let merge_notification_channel = saved
            .as_ref()
            .map(|settings| settings.merge_notification_channel.clone())
            .unwrap_or_else(|| self.merge_notification_channel_fallback.clone());
        if let Some(channel) = merge_notification_channel.as_deref() {
            self.store
                .ensure_channel(channel, MERGE_NOTIFICATION_CHANNEL_DESCRIPTION)
                .await?;
        }
        Ok(BoardSettings {
            merge_notification_channel,
            revision: saved.map_or(0, |settings| settings.revision),
        })
    }

    pub async fn set_settings(
        &self,
        merge_notification_channel: Option<&str>,
    ) -> Result<BoardSettings, AppError> {
        let _operation = self.operations.lock().await;
        let settings = self
            .store
            .set_merge_notification_channel(merge_notification_channel)
            .await?;
        self.publish_projection_invalidated(
            multi_agent_board_protocol::ProjectionScope::Settings,
            "board".into(),
            Some(settings.revision),
        )
        .await;
        Ok(settings)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<PublishedAgentEvent> {
        self.events.subscribe()
    }

    pub async fn add_ticket_comment(
        &self,
        ticket_id: i64,
        actor_agent_id: Option<i64>,
        body: &str,
    ) -> Result<TicketActivity, AppError> {
        self.add_ticket_comment_with_attachments(ticket_id, actor_agent_id, body, &[])
            .await
    }

    pub async fn add_ticket_comment_with_attachments(
        &self,
        ticket_id: i64,
        actor_agent_id: Option<i64>,
        body: &str,
        attachment_ids: &[String],
    ) -> Result<TicketActivity, AppError> {
        let (activity, recipients) = self
            .store
            .add_ticket_comment_with_attachments(ticket_id, actor_agent_id, body, attachment_ids)
            .await?;
        self.emit(AgentEvent::TicketActivityCreated {
            agent_id: actor_agent_id.unwrap_or_default(),
            data: activity.clone(),
        })
        .await;
        for agent_id in recipients {
            self.deliver_pending(agent_id).await;
        }
        Ok(activity)
    }

    pub(crate) async fn emit_ticket_activity(&self, agent_id: i64, activity: TicketActivity) {
        self.emit(AgentEvent::TicketActivityCreated {
            agent_id,
            data: activity,
        })
        .await;
    }

    pub(crate) async fn emit_ticket_activities(
        &self,
        agent_id: i64,
        activities: Vec<TicketActivity>,
    ) {
        for activity in activities {
            self.emit_ticket_activity(agent_id, activity).await;
        }
    }

    pub(crate) async fn replay_events(
        &self,
        after_id: Option<i64>,
    ) -> Result<AgentEventReplay, AppError> {
        let _publish = self.event_publish.lock().await;
        Ok(match self.store.replay_board_events(after_id).await? {
            BoardEventReplay::Events(events) => AgentEventReplay::Events(events),
            BoardEventReplay::Reset { latest_id } => AgentEventReplay::Reset { latest_id },
        })
    }

    pub(crate) async fn emit_board_message(&self, message: &Message) {
        self.emit(AgentEvent::BoardMessage {
            agent_id: message.agent_id.unwrap_or_default(),
            data: message.clone(),
        })
        .await;
    }

    pub(crate) async fn emit_projects_changed(&self) {
        self.emit(AgentEvent::ProjectsChanged).await;
    }

    pub(crate) async fn emit_channels_changed(&self) {
        self.emit(AgentEvent::ChannelsChanged).await;
    }

    pub(crate) async fn emit_ticket_updated(&self, agent_id: i64, ticket: &Ticket) {
        self.emit(AgentEvent::TicketUpdated {
            agent_id,
            data: ticket.clone(),
        })
        .await;
    }

    pub async fn list(&self) -> Result<Vec<Agent>, AppError> {
        Ok(self.store.agents().await?)
    }

    pub async fn agent(&self, agent_id: i64) -> Result<Agent, AppError> {
        self.store
            .agent(agent_id)
            .await?
            .ok_or_else(|| AppError::not_found("agent not found"))
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
        self.publish_agent_changed(&updated).await;
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
        let scopes = self.store.set_agent_scopes(agent_id, scopes).await?;
        self.publish_projection_invalidated(
            multi_agent_board_protocol::ProjectionScope::AgentDetails,
            agent_id.to_string(),
            None,
        )
        .await;
        Ok(scopes)
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
        self.emit(AgentEvent::ProjectSubscriptions {
            agent_id,
            data: project_ids.clone(),
        })
        .await;
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
        self.emit(AgentEvent::ProjectSubscriptions {
            agent_id,
            data: project_ids.clone(),
        })
        .await;
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
        self.emit(AgentEvent::ProjectSubscriptions {
            agent_id,
            data: project_ids.clone(),
        })
        .await;
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
        self.emit(AgentEvent::ChannelSubscriptions {
            agent_id,
            data: channels.clone(),
        })
        .await;
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
        self.emit(AgentEvent::ChannelSubscriptions {
            agent_id,
            data: channels.clone(),
        })
        .await;
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
        self.emit(AgentEvent::ChannelSubscriptions {
            agent_id,
            data: channels.clone(),
        })
        .await;
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
        self.publish_agent_changed(&agent).await;
        Ok(agent)
    }

    pub async fn create(&self, name: &str, project_id: i64) -> Result<Agent, AppError> {
        let _operation = self.operations.lock().await;
        validate_agent_name(name)?;
        if self.store.agent_named(name).await?.is_some() {
            return Err(AppError::conflict("agent name already exists"));
        }
        let project = self
            .store
            .project_by_id(project_id)
            .await?
            .ok_or_else(|| AppError::bad_request("agent project does not exist"))?;
        if project.archived_at.is_some() {
            return Err(AppError::conflict("agent project is archived"));
        }
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
        self.publish_agent_changed(&agent).await;
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
        let project_id = agent
            .active_project_id
            .ok_or_else(|| AppError::bad_request("agent has no active project"))?;
        self.worktree_for_project(agent_id, project_id).await
    }

    async fn emit(&self, event: AgentEvent) {
        let _publish = self.event_publish.lock().await;
        let (id, event) = if event.is_replayable() {
            match multi_agent_board_protocol::AgentEvent::try_from(event.clone()) {
                Ok(protocol_event) => match self.store.append_board_event(&protocol_event).await {
                    Ok(id) => (Some(id), event),
                    Err(_) => (None, AgentEvent::Refresh),
                },
                Err(_) => (None, AgentEvent::Refresh),
            }
        } else {
            (None, event)
        };
        let _ = self.events.send(PublishedAgentEvent { id, event });
    }

    async fn publish_agent_changed(&self, agent: &Agent) {
        let _publish = self.event_publish.lock().await;
        let event = AgentEvent::AgentChanged {
            agent_id: agent.id,
            revision: agent.revision,
        };
        let published = match self
            .store
            .agent_change_event_id(agent.id, agent.revision)
            .await
        {
            Ok(id) => PublishedAgentEvent {
                id: Some(id),
                event,
            },
            Err(_) => PublishedAgentEvent {
                id: None,
                event: AgentEvent::Refresh,
            },
        };
        let _ = self.events.send(published);
    }

    async fn publish_projection_invalidated(
        &self,
        scope: multi_agent_board_protocol::ProjectionScope,
        key: String,
        revision: Option<i64>,
    ) {
        let _publish = self.event_publish.lock().await;
        let resource_type = scope.resource_type();
        let resource_id = key.parse().ok();
        let event = AgentEvent::ProjectionInvalidated {
            scope,
            key,
            revision,
        };
        let published = match self
            .store
            .latest_projection_event_id(resource_type, resource_id)
            .await
        {
            Ok(id) => PublishedAgentEvent {
                id: Some(id),
                event,
            },
            Err(_) => PublishedAgentEvent {
                id: None,
                event: AgentEvent::Refresh,
            },
        };
        let _ = self.events.send(published);
    }
}

impl AgentEvent {
    fn is_replayable(&self) -> bool {
        !matches!(
            self,
            Self::Refresh
                | Self::AgentMessageDelta { .. }
                | Self::ApprovalDeclined { .. }
                | Self::CodexEvent { .. }
        )
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

    #[test]
    fn typed_agent_events_keep_the_sse_wire_contract() {
        let agent = Agent {
            id: 1,
            name: "worker".into(),
            instructions: String::new(),
            state: crate::domain::AgentState::Idle,
            thread_id: None,
            home_path: "/tmp/worker".into(),
            active_project_id: None,
            active_turn_id: None,
            last_error: None,
            wake_on_pending_work: false,
            wake_on_pending_review: false,
            idle_heartbeat_minutes: None,
            idle_since: None,
            revision: 1,
            created_at: 0,
            updated_at: 0,
        };
        let event = AgentEvent::AgentUpdated {
            agent_id: agent.id,
            data: agent,
        };
        let event = multi_agent_board_protocol::AgentEvent::try_from(event).unwrap();
        let value = serde_json::to_value(&event).unwrap();

        assert_eq!(value["kind"], "agent_updated");
        assert_eq!(value["agent_id"], 1);
        assert_eq!(value["data"]["name"], "worker");
        assert_eq!(
            serde_json::from_value::<multi_agent_board_protocol::AgentEvent>(value).unwrap(),
            event
        );
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

        let first = manager.create("backend", project.id).await.unwrap();
        let second = manager.create("frontend", project.id).await.unwrap();
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
                Default::default(),
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

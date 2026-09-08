use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct BoardSettings {
    pub merge_notification_channel: Option<String>,
}

#[derive(Clone, Debug, Serialize, FromRow, PartialEq, Eq)]
pub struct AgentProjectWorktree {
    pub agent_id: i64,
    pub project_id: i64,
    pub path: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, FromRow, PartialEq, Eq)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub default_branch: String,
    pub archived_at: Option<i64>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, FromRow, PartialEq, Eq)]
pub struct Ticket {
    pub id: i64,
    pub project_id: i64,
    pub title: String,
    pub description: String,
    pub state: TicketState,
    pub created_by_agent_id: Option<i64>,
    pub reserved_for_agent_id: Option<i64>,
    pub assignee_id: Option<i64>,
    pub implementation_branch: Option<String>,
    pub review_commit: Option<String>,
    pub review_target_branch: Option<String>,
    pub review_project_id: Option<i64>,
    pub review_project_path: Option<String>,
    pub reviewer_id: Option<i64>,
    pub review_feedback: Option<String>,
    #[sqlx(skip)]
    pub blocker_ids: Vec<i64>,
    #[sqlx(skip)]
    pub unresolved_blockers: Vec<TicketBlocker>,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, FromRow, PartialEq, Eq)]
pub struct TicketSummary {
    pub id: i64,
    pub project_id: i64,
    pub title: String,
    pub state: TicketState,
    pub created_by_agent_id: Option<i64>,
    pub reserved_for_agent_id: Option<i64>,
    pub assignee_id: Option<i64>,
    pub reviewer_id: Option<i64>,
    pub has_review_feedback: bool,
    #[sqlx(skip)]
    pub unresolved_blockers: Vec<TicketBlocker>,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, FromRow, PartialEq, Eq)]
pub struct TicketActivity {
    pub id: i64,
    pub ticket_id: i64,
    pub action: String,
    pub actor_agent_id: Option<i64>,
    pub previous_reserved_for_agent_id: Option<i64>,
    pub reserved_for_agent_id: Option<i64>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TicketBlocker {
    pub id: i64,
    pub title: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ProjectTransition {
    pub agent_id: i64,
    pub ticket_id: i64,
    pub previous_project_id: Option<i64>,
    pub project_id: i64,
    pub project_name: String,
    pub worktree_path: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct TicketClaimResponse {
    #[serde(flatten)]
    pub ticket: Ticket,
    pub project_transition: Option<ProjectTransition>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum TicketState {
    Todo,
    InProgress,
    Review,
    Done,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, FromRow, PartialEq, Eq)]
pub struct Channel {
    pub name: String,
    pub description: String,
    pub most_recent_message_at: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, FromRow, PartialEq, Eq)]
pub struct Message {
    pub id: i64,
    pub channel_name: String,
    pub agent_id: Option<i64>,
    pub author: String,
    pub body: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, FromRow, PartialEq, Eq)]
pub struct Agent {
    pub id: i64,
    pub name: String,
    pub state: AgentState,
    pub thread_id: Option<String>,
    pub home_path: String,
    pub active_project_id: Option<i64>,
    pub active_turn_id: Option<String>,
    pub last_error: Option<String>,
    pub wake_on_pending_work: bool,
    pub wake_on_pending_review: bool,
    pub idle_heartbeat_minutes: Option<i64>,
    pub idle_since: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum AgentState {
    Paused,
    Idle,
    Running,
    AwaitingApproval,
    Error,
}

#[derive(Clone, Debug, Deserialize, Serialize, FromRow, PartialEq, Eq)]
pub struct AgentMessage {
    pub id: i64,
    pub agent_id: i64,
    pub role: String,
    pub body: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct WakeEvent {
    pub id: i64,
    pub agent_id: i64,
    #[serde(flatten)]
    pub event: WakeEventData,
    pub deduplication_key: Option<String>,
    pub delivery_policy: WakeDeliveryPolicy,
    pub delivery_state: WakeEventState,
    pub delivery_attempts: i64,
    pub last_attempt_at: Option<i64>,
    pub delivered_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl WakeEvent {
    pub fn kind(&self) -> &'static str {
        self.event.kind()
    }

    pub fn payload(&self) -> serde_json::Value {
        self.event.payload()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "payload")]
pub enum WakeEventData {
    #[serde(rename = "notification")]
    Notification(NotificationWakePayload),
    #[serde(rename = "message")]
    Message(AgentMessageWakePayload),
    #[serde(rename = "channel.message")]
    ChannelMessage(ChannelMessageWakePayload),
    #[serde(rename = "idle.heartbeat")]
    IdleHeartbeat(IdleHeartbeatWakePayload),
    #[serde(rename = "reminder")]
    Reminder(ReminderWakePayload),
    #[serde(rename = "ticket.available")]
    TicketAvailable(TicketWakePayload),
    #[serde(rename = "ticket.assigned")]
    TicketAssigned(TicketWakePayload),
    #[serde(rename = "ticket.reserved")]
    TicketReserved(TicketWakePayload),
    #[serde(rename = "ticket.review_requested")]
    TicketReviewRequested(TicketWakePayload),
    #[serde(rename = "ticket.review_claimed")]
    TicketReviewClaimed(TicketWakePayload),
    #[serde(rename = "ticket.changes_requested")]
    TicketChangesRequested(TicketWakePayload),
}

impl WakeEventData {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Notification(_) => "notification",
            Self::Message(_) => "message",
            Self::ChannelMessage(_) => "channel.message",
            Self::IdleHeartbeat(_) => "idle.heartbeat",
            Self::Reminder(_) => "reminder",
            Self::TicketAvailable(_) => "ticket.available",
            Self::TicketAssigned(_) => "ticket.assigned",
            Self::TicketReserved(_) => "ticket.reserved",
            Self::TicketReviewRequested(_) => "ticket.review_requested",
            Self::TicketReviewClaimed(_) => "ticket.review_claimed",
            Self::TicketChangesRequested(_) => "ticket.changes_requested",
        }
    }

    pub fn payload(&self) -> serde_json::Value {
        let value = serde_json::to_value(self).expect("wake event payloads serialize");
        value
            .get("payload")
            .cloned()
            .expect("wake event has payload")
    }

    pub fn from_parts(kind: &str, payload: serde_json::Value) -> Result<Self, serde_json::Error> {
        serde_json::from_value(serde_json::json!({"kind": kind, "payload": payload}))
    }

    pub fn message(&self) -> &str {
        match self {
            Self::Notification(payload) => &payload.message,
            Self::Message(payload) => &payload.message,
            Self::ChannelMessage(payload) => &payload.message,
            Self::IdleHeartbeat(payload) => &payload.message,
            Self::Reminder(payload) => &payload.message,
            Self::TicketAvailable(payload)
            | Self::TicketAssigned(payload)
            | Self::TicketReserved(payload)
            | Self::TicketReviewRequested(payload)
            | Self::TicketReviewClaimed(payload)
            | Self::TicketChangesRequested(payload) => &payload.message,
        }
    }

    pub fn ticket_id(&self) -> Option<i64> {
        match self {
            Self::TicketAssigned(payload)
            | Self::TicketReviewClaimed(payload)
            | Self::TicketChangesRequested(payload) => Some(payload.ticket_id),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NotificationWakePayload {
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentMessageWakePayload {
    pub message: String,
    pub message_id: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChannelMessageWakePayload {
    pub message: String,
    pub channel: String,
    pub message_id: i64,
    pub author: String,
    pub body: String,
    pub mentioned: bool,
    pub subscribed: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IdleHeartbeatWakePayload {
    pub message: String,
    pub minutes: i64,
    pub occurred_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReminderWakePayload {
    pub message: String,
    pub requested_delivery_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TicketWakePayload {
    pub message: String,
    pub ticket_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed_fields: Vec<String>,
    pub ticket: TicketWakeTicket,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TicketWakeTicket {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<TicketState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by_agent_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reserved_for_agent_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implementation_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_target_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_project_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_project_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewer_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocker_ids: Vec<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved_blockers: Vec<TicketBlocker>,
}

impl TicketWakeTicket {
    pub fn project(project_id: i64) -> Self {
        Self {
            project_id: Some(project_id),
            title: None,
            state: None,
            assignee_id: None,
            created_by_agent_id: None,
            reserved_for_agent_id: None,
            implementation_branch: None,
            review_commit: None,
            review_target_branch: None,
            review_project_id: None,
            review_project_path: None,
            reviewer_id: None,
            blocker_ids: Vec::new(),
            unresolved_blockers: Vec::new(),
        }
    }
}

impl From<&Ticket> for TicketWakeTicket {
    fn from(ticket: &Ticket) -> Self {
        Self {
            project_id: Some(ticket.project_id),
            title: Some(ticket.title.clone()),
            state: Some(ticket.state),
            assignee_id: ticket.assignee_id,
            created_by_agent_id: ticket.created_by_agent_id,
            reserved_for_agent_id: ticket.reserved_for_agent_id,
            implementation_branch: ticket.implementation_branch.clone(),
            review_commit: ticket.review_commit.clone(),
            review_target_branch: ticket.review_target_branch.clone(),
            review_project_id: ticket.review_project_id,
            review_project_path: ticket.review_project_path.clone(),
            reviewer_id: ticket.reviewer_id,
            blocker_ids: ticket.blocker_ids.clone(),
            unresolved_blockers: ticket.unresolved_blockers.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, FromRow, PartialEq, Eq)]
pub struct Reminder {
    pub id: i64,
    pub agent_id: i64,
    pub message: String,
    pub deliver_at: i64,
    pub state: ReminderState,
    pub wake_event_id: Option<i64>,
    pub enqueued_at: Option<i64>,
    pub cancelled_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum ReminderState {
    Scheduled,
    Enqueued,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum WakeDeliveryPolicy {
    IdleOnly,
    Immediate,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum WakeEventState {
    Pending,
    Delivering,
    Delivered,
}

use multi_agent_board_protocol as wire;

impl From<BoardSettings> for wire::BoardSettings {
    fn from(value: BoardSettings) -> Self {
        Self {
            merge_notification_channel: value.merge_notification_channel,
        }
    }
}

impl From<Project> for wire::Project {
    fn from(value: Project) -> Self {
        Self {
            id: value.id,
            name: value.name,
            path: value.path,
            default_branch: value.default_branch,
            archived_at: value.archived_at,
            updated_at: value.updated_at,
        }
    }
}

impl From<TicketState> for wire::TicketState {
    fn from(value: TicketState) -> Self {
        match value {
            TicketState::Todo => Self::Todo,
            TicketState::InProgress => Self::InProgress,
            TicketState::Review => Self::Review,
            TicketState::Done => Self::Done,
            TicketState::Cancelled => Self::Cancelled,
        }
    }
}

impl From<wire::TicketState> for TicketState {
    fn from(value: wire::TicketState) -> Self {
        match value {
            wire::TicketState::Todo => Self::Todo,
            wire::TicketState::InProgress => Self::InProgress,
            wire::TicketState::Review => Self::Review,
            wire::TicketState::Done => Self::Done,
            wire::TicketState::Cancelled => Self::Cancelled,
        }
    }
}

impl From<TicketBlocker> for wire::TicketBlocker {
    fn from(value: TicketBlocker) -> Self {
        Self {
            id: value.id,
            title: value.title,
        }
    }
}

impl From<wire::TicketBlocker> for TicketBlocker {
    fn from(value: wire::TicketBlocker) -> Self {
        Self {
            id: value.id,
            title: value.title,
        }
    }
}

impl From<Ticket> for wire::Ticket {
    fn from(value: Ticket) -> Self {
        Self {
            id: value.id,
            project_id: value.project_id,
            title: value.title,
            description: value.description,
            state: value.state.into(),
            created_by_agent_id: value.created_by_agent_id,
            reserved_for_agent_id: value.reserved_for_agent_id,
            assignee_id: value.assignee_id,
            implementation_branch: value.implementation_branch,
            review_commit: value.review_commit,
            review_target_branch: value.review_target_branch,
            review_project_id: value.review_project_id,
            review_project_path: value.review_project_path,
            reviewer_id: value.reviewer_id,
            review_feedback: value.review_feedback,
            blocker_ids: value.blocker_ids,
            unresolved_blockers: value
                .unresolved_blockers
                .into_iter()
                .map(Into::into)
                .collect(),
            revision: value.revision,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

impl From<TicketSummary> for wire::TicketSummary {
    fn from(value: TicketSummary) -> Self {
        Self {
            id: value.id,
            project_id: value.project_id,
            title: value.title,
            state: value.state.into(),
            created_by_agent_id: value.created_by_agent_id,
            reserved_for_agent_id: value.reserved_for_agent_id,
            assignee_id: value.assignee_id,
            reviewer_id: value.reviewer_id,
            has_review_feedback: value.has_review_feedback,
            unresolved_blockers: value
                .unresolved_blockers
                .into_iter()
                .map(Into::into)
                .collect(),
            revision: value.revision,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

impl From<TicketActivity> for wire::TicketActivity {
    fn from(value: TicketActivity) -> Self {
        Self {
            id: value.id,
            ticket_id: value.ticket_id,
            action: value.action,
            actor_agent_id: value.actor_agent_id,
            previous_reserved_for_agent_id: value.previous_reserved_for_agent_id,
            reserved_for_agent_id: value.reserved_for_agent_id,
            created_at: value.created_at,
        }
    }
}

impl From<ProjectTransition> for wire::ProjectTransition {
    fn from(value: ProjectTransition) -> Self {
        Self {
            agent_id: value.agent_id,
            ticket_id: value.ticket_id,
            previous_project_id: value.previous_project_id,
            project_id: value.project_id,
            project_name: value.project_name,
            worktree_path: value.worktree_path,
            message: value.message,
        }
    }
}

impl From<TicketClaimResponse> for wire::TicketClaimResponse {
    fn from(value: TicketClaimResponse) -> Self {
        Self {
            ticket: value.ticket.into(),
            project_transition: value.project_transition.map(Into::into),
        }
    }
}

impl From<Channel> for wire::Channel {
    fn from(value: Channel) -> Self {
        Self {
            name: value.name,
            description: value.description,
            most_recent_message_at: value.most_recent_message_at,
        }
    }
}

impl From<Message> for wire::Message {
    fn from(value: Message) -> Self {
        Self {
            id: value.id,
            channel_name: value.channel_name,
            agent_id: value.agent_id,
            author: value.author,
            body: value.body,
            created_at: value.created_at,
        }
    }
}

impl From<AgentState> for wire::AgentState {
    fn from(value: AgentState) -> Self {
        match value {
            AgentState::Paused => Self::Paused,
            AgentState::Idle => Self::Idle,
            AgentState::Running => Self::Running,
            AgentState::AwaitingApproval => Self::AwaitingApproval,
            AgentState::Error => Self::Error,
        }
    }
}

impl From<Agent> for wire::Agent {
    fn from(value: Agent) -> Self {
        Self {
            id: value.id,
            name: value.name,
            state: value.state.into(),
            thread_id: value.thread_id,
            home_path: value.home_path,
            active_project_id: value.active_project_id,
            active_turn_id: value.active_turn_id,
            last_error: value.last_error,
            wake_on_pending_work: value.wake_on_pending_work,
            wake_on_pending_review: value.wake_on_pending_review,
            idle_heartbeat_minutes: value.idle_heartbeat_minutes,
            idle_since: value.idle_since,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

impl From<AgentProjectWorktree> for wire::AgentProjectWorktree {
    fn from(value: AgentProjectWorktree) -> Self {
        Self {
            agent_id: value.agent_id,
            project_id: value.project_id,
            path: value.path,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

impl TryFrom<AgentMessage> for wire::AgentMessage {
    type Error = String;

    fn try_from(value: AgentMessage) -> Result<Self, Self::Error> {
        Ok(Self {
            id: value.id,
            agent_id: value.agent_id,
            role: wire::AgentMessageRole::try_from(value.role.as_str())?,
            body: value.body,
            created_at: value.created_at,
        })
    }
}

impl From<ReminderState> for wire::ReminderState {
    fn from(value: ReminderState) -> Self {
        match value {
            ReminderState::Scheduled => Self::Scheduled,
            ReminderState::Enqueued => Self::Enqueued,
            ReminderState::Cancelled => Self::Cancelled,
        }
    }
}

impl From<Reminder> for wire::Reminder {
    fn from(value: Reminder) -> Self {
        Self {
            id: value.id,
            agent_id: value.agent_id,
            message: value.message,
            deliver_at: value.deliver_at,
            state: value.state.into(),
            wake_event_id: value.wake_event_id,
            enqueued_at: value.enqueued_at,
            cancelled_at: value.cancelled_at,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

impl From<WakeDeliveryPolicy> for wire::WakeDeliveryPolicy {
    fn from(value: WakeDeliveryPolicy) -> Self {
        match value {
            WakeDeliveryPolicy::IdleOnly => Self::IdleOnly,
            WakeDeliveryPolicy::Immediate => Self::Immediate,
        }
    }
}

impl From<WakeEventState> for wire::WakeEventState {
    fn from(value: WakeEventState) -> Self {
        match value {
            WakeEventState::Pending => Self::Pending,
            WakeEventState::Delivering => Self::Delivering,
            WakeEventState::Delivered => Self::Delivered,
        }
    }
}

impl From<NotificationWakePayload> for wire::NotificationWakePayload {
    fn from(value: NotificationWakePayload) -> Self {
        Self {
            message: value.message,
        }
    }
}

impl From<AgentMessageWakePayload> for wire::AgentMessageWakePayload {
    fn from(value: AgentMessageWakePayload) -> Self {
        Self {
            message: value.message,
            message_id: value.message_id,
        }
    }
}

impl From<ChannelMessageWakePayload> for wire::ChannelMessageWakePayload {
    fn from(value: ChannelMessageWakePayload) -> Self {
        Self {
            message: value.message,
            channel: value.channel,
            message_id: value.message_id,
            author: value.author,
            body: value.body,
            mentioned: value.mentioned,
            subscribed: value.subscribed,
        }
    }
}

impl From<IdleHeartbeatWakePayload> for wire::IdleHeartbeatWakePayload {
    fn from(value: IdleHeartbeatWakePayload) -> Self {
        Self {
            message: value.message,
            minutes: value.minutes,
            occurred_at: value.occurred_at,
        }
    }
}

impl From<ReminderWakePayload> for wire::ReminderWakePayload {
    fn from(value: ReminderWakePayload) -> Self {
        Self {
            message: value.message,
            requested_delivery_at: value.requested_delivery_at,
        }
    }
}

impl From<TicketWakeTicket> for wire::TicketWakeTicket {
    fn from(value: TicketWakeTicket) -> Self {
        Self {
            project_id: value.project_id,
            title: value.title,
            state: value.state.map(Into::into),
            assignee_id: value.assignee_id,
            created_by_agent_id: value.created_by_agent_id,
            reserved_for_agent_id: value.reserved_for_agent_id,
            implementation_branch: value.implementation_branch,
            review_commit: value.review_commit,
            review_target_branch: value.review_target_branch,
            review_project_id: value.review_project_id,
            review_project_path: value.review_project_path,
            reviewer_id: value.reviewer_id,
            blocker_ids: value.blocker_ids,
            unresolved_blockers: value
                .unresolved_blockers
                .into_iter()
                .map(Into::into)
                .collect(),
        }
    }
}

impl From<TicketWakePayload> for wire::TicketWakePayload {
    fn from(value: TicketWakePayload) -> Self {
        Self {
            message: value.message,
            ticket_id: value.ticket_id,
            revision: value.revision,
            changed_fields: value.changed_fields,
            ticket: value.ticket.into(),
        }
    }
}

impl From<WakeEventData> for wire::WakeEventData {
    fn from(value: WakeEventData) -> Self {
        match value {
            WakeEventData::Notification(payload) => Self::Notification(payload.into()),
            WakeEventData::Message(payload) => Self::Message(payload.into()),
            WakeEventData::ChannelMessage(payload) => Self::ChannelMessage(payload.into()),
            WakeEventData::IdleHeartbeat(payload) => Self::IdleHeartbeat(payload.into()),
            WakeEventData::Reminder(payload) => Self::Reminder(payload.into()),
            WakeEventData::TicketAvailable(payload) => Self::TicketAvailable(payload.into()),
            WakeEventData::TicketAssigned(payload) => Self::TicketAssigned(payload.into()),
            WakeEventData::TicketReserved(payload) => Self::TicketReserved(payload.into()),
            WakeEventData::TicketReviewRequested(payload) => {
                Self::TicketReviewRequested(payload.into())
            }
            WakeEventData::TicketReviewClaimed(payload) => {
                Self::TicketReviewClaimed(payload.into())
            }
            WakeEventData::TicketChangesRequested(payload) => {
                Self::TicketChangesRequested(payload.into())
            }
        }
    }
}

impl From<wire::WakeEventData> for WakeEventData {
    fn from(value: wire::WakeEventData) -> Self {
        match value {
            wire::WakeEventData::Notification(payload) => {
                Self::Notification(NotificationWakePayload {
                    message: payload.message,
                })
            }
            wire::WakeEventData::Message(payload) => Self::Message(AgentMessageWakePayload {
                message: payload.message,
                message_id: payload.message_id,
            }),
            wire::WakeEventData::ChannelMessage(payload) => {
                Self::ChannelMessage(ChannelMessageWakePayload {
                    message: payload.message,
                    channel: payload.channel,
                    message_id: payload.message_id,
                    author: payload.author,
                    body: payload.body,
                    mentioned: payload.mentioned,
                    subscribed: payload.subscribed,
                })
            }
            wire::WakeEventData::IdleHeartbeat(payload) => {
                Self::IdleHeartbeat(IdleHeartbeatWakePayload {
                    message: payload.message,
                    minutes: payload.minutes,
                    occurred_at: payload.occurred_at,
                })
            }
            wire::WakeEventData::Reminder(payload) => Self::Reminder(ReminderWakePayload {
                message: payload.message,
                requested_delivery_at: payload.requested_delivery_at,
            }),
            wire::WakeEventData::TicketAvailable(payload) => Self::TicketAvailable(payload.into()),
            wire::WakeEventData::TicketAssigned(payload) => Self::TicketAssigned(payload.into()),
            wire::WakeEventData::TicketReserved(payload) => Self::TicketReserved(payload.into()),
            wire::WakeEventData::TicketReviewRequested(payload) => {
                Self::TicketReviewRequested(payload.into())
            }
            wire::WakeEventData::TicketReviewClaimed(payload) => {
                Self::TicketReviewClaimed(payload.into())
            }
            wire::WakeEventData::TicketChangesRequested(payload) => {
                Self::TicketChangesRequested(payload.into())
            }
        }
    }
}

impl From<wire::TicketWakePayload> for TicketWakePayload {
    fn from(value: wire::TicketWakePayload) -> Self {
        Self {
            message: value.message,
            ticket_id: value.ticket_id,
            revision: value.revision,
            changed_fields: value.changed_fields,
            ticket: value.ticket.into(),
        }
    }
}

impl From<wire::TicketWakeTicket> for TicketWakeTicket {
    fn from(value: wire::TicketWakeTicket) -> Self {
        Self {
            project_id: value.project_id,
            title: value.title,
            state: value.state.map(Into::into),
            assignee_id: value.assignee_id,
            created_by_agent_id: value.created_by_agent_id,
            reserved_for_agent_id: value.reserved_for_agent_id,
            implementation_branch: value.implementation_branch,
            review_commit: value.review_commit,
            review_target_branch: value.review_target_branch,
            review_project_id: value.review_project_id,
            review_project_path: value.review_project_path,
            reviewer_id: value.reviewer_id,
            blocker_ids: value.blocker_ids,
            unresolved_blockers: value
                .unresolved_blockers
                .into_iter()
                .map(Into::into)
                .collect(),
        }
    }
}

impl From<WakeEvent> for wire::WakeEvent {
    fn from(value: WakeEvent) -> Self {
        Self {
            id: value.id,
            agent_id: value.agent_id,
            event: value.event.into(),
            deduplication_key: value.deduplication_key,
            delivery_policy: value.delivery_policy.into(),
            delivery_state: value.delivery_state.into(),
            delivery_attempts: value.delivery_attempts,
            last_attempt_at: value.last_attempt_at,
            delivered_at: value.delivered_at,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_event_variants_round_trip_with_stable_kinds() {
        let ticket = TicketWakePayload {
            message: "Ticket changed".into(),
            ticket_id: 7,
            revision: Some(2),
            changed_fields: vec!["state".into()],
            ticket: TicketWakeTicket::project(3),
        };
        let events = vec![
            WakeEventData::Notification(NotificationWakePayload {
                message: "Notice".into(),
            }),
            WakeEventData::Message(AgentMessageWakePayload {
                message: "Hello".into(),
                message_id: 1,
            }),
            WakeEventData::ChannelMessage(ChannelMessageWakePayload {
                message: "Post".into(),
                channel: "general".into(),
                message_id: 2,
                author: "human".into(),
                body: "Post".into(),
                mentioned: false,
                subscribed: true,
            }),
            WakeEventData::IdleHeartbeat(IdleHeartbeatWakePayload {
                message: "Tick".into(),
                minutes: 20,
                occurred_at: "2026-01-01T00:00:00Z".into(),
            }),
            WakeEventData::Reminder(ReminderWakePayload {
                message: "Remember".into(),
                requested_delivery_at: 1,
            }),
            WakeEventData::TicketAvailable(ticket.clone()),
            WakeEventData::TicketAssigned(ticket.clone()),
            WakeEventData::TicketReserved(ticket.clone()),
            WakeEventData::TicketReviewRequested(ticket.clone()),
            WakeEventData::TicketReviewClaimed(ticket.clone()),
            WakeEventData::TicketChangesRequested(ticket),
        ];

        for event in events {
            let value = serde_json::to_value(&event).unwrap();
            assert_eq!(value["kind"], event.kind());
            assert_eq!(
                serde_json::from_value::<WakeEventData>(value).unwrap(),
                event
            );
        }
    }

    #[test]
    fn invalid_wake_event_kind_or_payload_is_rejected() {
        assert!(WakeEventData::from_parts("unknown", serde_json::json!({})).is_err());
        assert!(
            WakeEventData::from_parts("notification", serde_json::json!({"wrong": true})).is_err()
        );
    }

    #[test]
    fn ticket_state_converts_across_the_wire_boundary() {
        let wire_state = wire::TicketState::from(TicketState::InProgress);
        assert_eq!(wire_state, wire::TicketState::InProgress);
        assert_eq!(TicketState::from(wire_state), TicketState::InProgress);
    }

    #[test]
    fn invalid_persisted_agent_message_role_is_rejected() {
        let message = AgentMessage {
            id: 1,
            agent_id: 2,
            role: "assistant".into(),
            body: "hello".into(),
            created_at: 3,
        };

        assert_eq!(
            wire::AgentMessage::try_from(message).unwrap_err(),
            "invalid agent message role: assistant"
        );
    }
}

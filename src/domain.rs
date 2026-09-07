use serde::{Deserialize, Serialize};
use sqlx::FromRow;

pub use multi_agent_board_protocol::AgentProjectWorktree;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct BoardSettings {
    pub merge_notification_channel: Option<String>,
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

#[derive(Clone, Debug, Serialize, FromRow, PartialEq, Eq)]
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

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct TicketBlocker {
    pub id: i64,
    pub title: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum TicketState {
    Todo,
    InProgress,
    Review,
    Done,
}

#[derive(Clone, Debug, Serialize, FromRow, PartialEq, Eq)]
pub struct Channel {
    pub name: String,
    pub description: String,
    pub most_recent_message_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize, FromRow, PartialEq, Eq)]
pub struct Message {
    pub id: i64,
    pub channel_name: String,
    pub agent_id: Option<i64>,
    pub author: String,
    pub body: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, FromRow, PartialEq, Eq)]
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

#[derive(Clone, Debug, Serialize, FromRow, PartialEq, Eq)]
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
    pub kind: String,
    pub payload: serde_json::Value,
    pub deduplication_key: Option<String>,
    pub delivery_policy: WakeDeliveryPolicy,
    pub delivery_state: WakeEventState,
    pub delivery_attempts: i64,
    pub last_attempt_at: Option<i64>,
    pub delivered_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
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

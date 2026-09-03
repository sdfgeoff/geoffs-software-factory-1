use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Clone, Debug, Serialize, FromRow, PartialEq, Eq)]
pub struct Project {
    pub path: String,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, FromRow, PartialEq, Eq)]
pub struct Ticket {
    pub id: i64,
    pub title: String,
    pub description: String,
    pub state: TicketState,
    pub assignee_id: Option<i64>,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum TicketState {
    Todo,
    InProgress,
    Done,
}

#[derive(Clone, Debug, Serialize, FromRow, PartialEq, Eq)]
pub struct Channel {
    pub name: String,
    pub most_recent_message_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize, FromRow, PartialEq, Eq)]
pub struct Message {
    pub id: i64,
    pub channel_name: String,
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
    pub worktree_path: String,
    pub active_turn_id: Option<String>,
    pub last_error: Option<String>,
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

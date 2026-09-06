use serde::{Deserialize, Serialize};

pub const AGENT_ID_HEADER: &str = "x-multi-agent-board-agent-id";

#[derive(Debug, Deserialize, Serialize)]
pub struct SetProjectRequest {
    pub path: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CreateTicketRequest {
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub blocker_ids: Vec<i64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ReserveTicketsRequest {
    pub ticket_ids: Vec<i64>,
    pub agent_id: Option<i64>,
    #[serde(default)]
    pub administrative_override: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ClaimTicketRequest {
    pub agent_id: Option<i64>,
    pub expected_revision: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MessageRangeQuery {
    pub start_time: Option<i64>,
    pub end_time: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PostMessageRequest {
    pub author: String,
    pub body: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CreateAgentRequest {
    pub name: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RenameAgentRequest {
    pub name: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SendAgentMessageRequest {
    pub message: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CreateWakeEventRequest {
    pub kind: String,
    pub payload: serde_json::Value,
    pub deduplication_key: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SetAgentScopesRequest {
    pub scopes: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SetAgentWakeSettingsRequest {
    pub wake_on_pending_work: bool,
    pub wake_on_pending_review: bool,
    pub idle_heartbeat_minutes: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SetAgentChannelSubscriptionsRequest {
    pub channels: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ReviewRevisionRequest {
    pub expected_revision: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RequestChangesRequest {
    pub feedback: String,
    pub expected_revision: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

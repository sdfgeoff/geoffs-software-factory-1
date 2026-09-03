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
pub struct SendAgentMessageRequest {
    pub message: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SetAgentScopesRequest {
    pub scopes: Vec<String>,
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

use serde::{Deserialize, Serialize};

pub const AGENT_ID_HEADER: &str = "x-multi-agent-board-agent-id";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct BoardSettings {
    pub merge_notification_channel: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SetBoardSettingsRequest {
    pub merge_notification_channel: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub default_branch: String,
    pub archived_at: Option<i64>,
    pub updated_at: i64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TicketState {
    Todo,
    InProgress,
    Review,
    Done,
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TicketBlocker {
    pub id: i64,
    pub title: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
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
    pub blocker_ids: Vec<i64>,
    pub unresolved_blockers: Vec<TicketBlocker>,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
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
pub struct ProjectTransition {
    pub agent_id: i64,
    pub ticket_id: i64,
    pub previous_project_id: Option<i64>,
    pub project_id: i64,
    pub project_name: String,
    pub worktree_path: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TicketClaimResponse {
    #[serde(flatten)]
    pub ticket: Ticket,
    pub project_transition: Option<ProjectTransition>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Channel {
    pub name: String,
    pub description: String,
    pub most_recent_message_at: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Message {
    pub id: i64,
    pub channel_name: String,
    pub agent_id: Option<i64>,
    pub author: String,
    pub body: String,
    pub created_at: i64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Paused,
    Idle,
    Running,
    AwaitingApproval,
    Error,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
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

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AgentProjectWorktree {
    pub agent_id: i64,
    pub project_id: i64,
    pub path: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentMessageRole {
    User,
    Agent,
    System,
}

impl TryFrom<&str> for AgentMessageRole {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "user" => Ok(Self::User),
            "agent" => Ok(Self::Agent),
            "system" => Ok(Self::System),
            _ => Err(format!("invalid agent message role: {value}")),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AgentMessage {
    pub id: i64,
    pub agent_id: i64,
    pub role: AgentMessageRole,
    pub body: String,
    pub created_at: i64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReminderState {
    Scheduled,
    Enqueued,
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
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

#[derive(Debug, Deserialize, Serialize)]
pub struct SetProjectRequest {
    pub path: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RegisterProjectRequest {
    pub name: String,
    pub path: String,
    pub default_branch: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ProjectIdRequest {
    pub project_id: i64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CreateTicketRequest {
    pub project_id: Option<i64>,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub blocker_ids: Vec<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Patch<T> {
    #[default]
    Missing,
    Null,
    Value(T),
}

impl<T> Patch<T> {
    pub fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }
}

impl<T: Serialize> Serialize for Patch<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Missing => serializer.serialize_unit(),
            Self::Null => serializer.serialize_none(),
            Self::Value(value) => value.serialize(serializer),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Patch<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(match Option::<T>::deserialize(deserializer)? {
            Some(value) => Self::Value(value),
            None => Self::Null,
        })
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct UpdateTicketRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<TicketState>,
    #[serde(default, skip_serializing_if = "Patch::is_missing")]
    pub assignee_id: Patch<i64>,
    #[serde(default, skip_serializing_if = "Patch::is_missing")]
    pub reserved_for_agent_id: Patch<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocker_ids: Option<Vec<i64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TicketIdRequest {
    pub ticket_id: i64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SetTicketDetailsRequest {
    pub ticket_id: i64,
    #[serde(flatten)]
    pub changes: UpdateTicketRequest,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TicketRevisionRequest {
    pub ticket_id: i64,
    pub expected_revision: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TicketChangesRequest {
    pub ticket_id: i64,
    pub feedback: String,
    pub expected_revision: Option<i64>,
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
pub struct ReadChannelRequest {
    pub channel_name: String,
    pub start_time: Option<i64>,
    pub end_time: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ReadChannelResponse {
    pub channel: Channel,
    pub messages: Vec<Message>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PostBoardMessageRequest {
    pub channel_name: String,
    pub author: String,
    pub message: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CreateChannelRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
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
pub struct ScheduleSelfReminderRequest {
    pub message: String,
    pub deliver_at: Option<String>,
    pub delay_seconds: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ReminderIdRequest {
    pub reminder_id: i64,
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
pub struct SetAgentProjectSubscriptionsRequest {
    pub project_ids: Vec<i64>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ticket() -> Ticket {
        Ticket {
            id: 47,
            project_id: 1,
            title: "Typed wire contract".into(),
            description: "Keep every boundary checked".into(),
            state: TicketState::InProgress,
            created_by_agent_id: Some(2),
            reserved_for_agent_id: None,
            assignee_id: Some(2),
            implementation_branch: Some("tickets/47-complete-wire-contract".into()),
            review_commit: None,
            review_target_branch: Some("main".into()),
            review_project_id: Some(1),
            review_project_path: Some("/tmp/project".into()),
            reviewer_id: None,
            review_feedback: None,
            blocker_ids: vec![46],
            unresolved_blockers: vec![TicketBlocker {
                id: 46,
                title: "Typed events".into(),
            }],
            revision: 2,
            created_at: 10,
            updated_at: 20,
        }
    }

    #[test]
    fn representative_response_round_trips() {
        let response = TicketClaimResponse {
            ticket: ticket(),
            project_transition: Some(ProjectTransition {
                agent_id: 2,
                ticket_id: 47,
                previous_project_id: None,
                project_id: 1,
                project_name: "board".into(),
                worktree_path: "/tmp/worktree".into(),
                message: "Switched project".into(),
            }),
        };

        let encoded = serde_json::to_value(&response).unwrap();
        assert_eq!(encoded["state"], "in_progress");
        assert_eq!(encoded["id"], 47);
        assert_eq!(
            serde_json::from_value::<TicketClaimResponse>(encoded).unwrap(),
            response
        );
    }

    #[test]
    fn ticket_patch_distinguishes_missing_null_and_value() {
        let missing: UpdateTicketRequest = serde_json::from_value(json!({})).unwrap();
        let null: UpdateTicketRequest =
            serde_json::from_value(json!({"assignee_id": null})).unwrap();
        let value: UpdateTicketRequest = serde_json::from_value(json!({"assignee_id": 2})).unwrap();

        assert_eq!(missing.assignee_id, Patch::Missing);
        assert_eq!(null.assignee_id, Patch::Null);
        assert_eq!(value.assignee_id, Patch::Value(2));
        assert_eq!(serde_json::to_value(missing).unwrap(), json!({}));
    }

    #[test]
    fn agent_message_roles_are_checked() {
        assert_eq!(
            AgentMessageRole::try_from("agent").unwrap(),
            AgentMessageRole::Agent
        );
        assert_eq!(
            AgentMessageRole::try_from("assistant").unwrap_err(),
            "invalid agent message role: assistant"
        );
    }
}

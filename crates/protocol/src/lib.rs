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
    pub completion_workflow: ProjectCompletionWorkflow,
    pub archived_at: Option<i64>,
    pub updated_at: i64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectCompletionWorkflow {
    #[default]
    BoardManaged,
    External,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TicketState {
    Todo,
    InProgress,
    Review,
    AwaitingExternalMerge,
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
    pub review_completion_workflow: Option<ProjectCompletionWorkflow>,
    pub external_change_url: Option<String>,
    pub external_merge_reference: Option<String>,
    pub external_merged_at: Option<i64>,
    pub blocker_ids: Vec<i64>,
    pub unresolved_blockers: Vec<TicketBlocker>,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TicketSummary {
    pub id: i64,
    pub project_id: i64,
    pub title: String,
    pub state: TicketState,
    pub created_by_agent_id: Option<i64>,
    pub reserved_for_agent_id: Option<i64>,
    pub assignee_id: Option<i64>,
    pub reviewer_id: Option<i64>,
    pub review_completion_workflow: Option<ProjectCompletionWorkflow>,
    pub has_review_feedback: bool,
    pub unresolved_blockers: Vec<TicketBlocker>,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TicketSummaryPage {
    pub tickets: Vec<TicketSummary>,
    pub has_more: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct TicketSummaryQuery {
    pub project_id: Option<i64>,
    pub state: Option<TicketState>,
    pub before_id: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TicketActivity {
    pub id: i64,
    pub ticket_id: i64,
    pub action: String,
    pub detail: Option<String>,
    pub author: Option<String>,
    pub actor_agent_id: Option<i64>,
    pub previous_reserved_for_agent_id: Option<i64>,
    pub reserved_for_agent_id: Option<i64>,
    pub implementation_branch: Option<String>,
    pub review_commit: Option<String>,
    pub review_target_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TicketActivityPage {
    pub entries: Vec<TicketActivity>,
    pub has_more: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct TicketActivityQuery {
    pub before_id: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PostTicketCommentRequest {
    pub body: String,
    #[serde(default)]
    pub attachment_ids: Vec<String>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Attachment {
    pub id: String,
    pub original_filename: String,
    pub media_type: String,
    pub byte_size: i64,
    pub download_url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct MessagePage {
    pub messages: Vec<Message>,
    pub has_more: bool,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AgentMessagePage {
    pub messages: Vec<AgentMessage>,
    pub has_more: bool,
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

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WakeDeliveryPolicy {
    IdleOnly,
    Immediate,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WakeEventState {
    Pending,
    Delivering,
    Delivered,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
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
    #[serde(rename = "ticket.external_merge_ready")]
    TicketExternalMergeReady(TicketWakePayload),
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

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum AgentEvent {
    Refresh,
    SnapshotReset {
        data: SnapshotReset,
    },
    ProjectsChanged,
    ChannelsChanged,
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
        data: AgentMessage,
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
        data: serde_json::Value,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SnapshotReset {
    pub scopes: Vec<SnapshotScope>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotScope {
    Board,
    Messages,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AgentMessageDelta {
    pub delta: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RegisterProjectRequest {
    pub name: String,
    pub path: String,
    pub default_branch: String,
    #[serde(default)]
    pub completion_workflow: ProjectCompletionWorkflow,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SetProjectCompletionWorkflowRequest {
    pub completion_workflow: ProjectCompletionWorkflow,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CreateTicketRequest {
    pub project_id: i64,
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
pub struct MessagePageQuery {
    pub before_id: Option<i64>,
    pub after_id: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PostMessageRequest {
    pub author: String,
    pub body: String,
    #[serde(default)]
    pub attachment_ids: Vec<String>,
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
    pub project_id: i64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RenameAgentRequest {
    pub name: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SendAgentMessageRequest {
    pub message: String,
    #[serde(default)]
    pub attachment_ids: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CreateWakeEventRequest {
    #[serde(flatten)]
    pub event: WakeEventData,
    pub deduplication_key: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ScheduleSelfReminderRequest {
    pub message: String,
    pub deliver_at: Option<String>,
    pub delay_seconds: Option<i64>,
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
pub struct SetExternalChangeRequest {
    pub external_change_url: String,
    pub expected_revision: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ReturnExternalMergeRequest {
    pub reason: String,
    pub expected_revision: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CompleteExternalMergeRequest {
    pub confirmation: String,
    pub merge_reference: Option<String>,
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
            review_completion_workflow: None,
            external_change_url: None,
            external_merge_reference: None,
            external_merged_at: None,
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
    fn ticket_summary_page_omits_full_review_and_description_fields() {
        let page = TicketSummaryPage {
            tickets: vec![TicketSummary {
                id: 47,
                project_id: 1,
                title: "Typed wire contract".into(),
                state: TicketState::Done,
                created_by_agent_id: Some(2),
                reserved_for_agent_id: None,
                assignee_id: Some(2),
                reviewer_id: Some(3),
                review_completion_workflow: Some(ProjectCompletionWorkflow::External),
                has_review_feedback: true,
                unresolved_blockers: Vec::new(),
                revision: 4,
                created_at: 10,
                updated_at: 20,
            }],
            has_more: true,
        };
        let encoded = serde_json::to_value(&page).unwrap();
        assert_eq!(encoded["tickets"][0]["has_review_feedback"], true);
        assert_eq!(
            encoded["tickets"][0]["review_completion_workflow"],
            "external"
        );
        assert!(encoded["tickets"][0].get("description").is_none());
        assert_eq!(
            serde_json::from_value::<TicketSummaryPage>(encoded).unwrap(),
            page
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

    #[test]
    fn typed_events_keep_the_existing_wire_shape() {
        let event = AgentEvent::TicketUpdated {
            agent_id: 2,
            data: ticket(),
        };
        let encoded = serde_json::to_value(&event).unwrap();
        assert_eq!(encoded["kind"], "ticket_updated");
        assert_eq!(encoded["data"]["id"], 47);
        assert_eq!(
            serde_json::from_value::<AgentEvent>(encoded).unwrap(),
            event
        );

        let legacy_wake = json!({
            "kind": "ticket.available",
            "payload": {
                "message": "Available",
                "ticket_id": 47,
                "ticket": {}
            }
        });
        let decoded = serde_json::from_value::<WakeEventData>(legacy_wake).unwrap();
        let WakeEventData::TicketAvailable(payload) = decoded else {
            panic!("wrong wake variant");
        };
        assert_eq!(payload.ticket.project_id, None);
    }

    #[test]
    fn external_lifecycle_events_have_distinct_typed_wire_kinds() {
        for (event, kind) in [
            (
                AgentEvent::ExternalReviewApproved {
                    agent_id: 2,
                    data: ticket(),
                },
                "external_review_approved",
            ),
            (
                AgentEvent::ExternalChangeUpdated {
                    agent_id: 2,
                    data: ticket(),
                },
                "external_change_updated",
            ),
            (
                AgentEvent::ExternalMergeReturned {
                    agent_id: 2,
                    data: ticket(),
                },
                "external_merge_returned",
            ),
            (
                AgentEvent::ExternalMergeCompleted {
                    agent_id: 2,
                    data: ticket(),
                },
                "external_merge_completed",
            ),
        ] {
            let encoded = serde_json::to_value(&event).unwrap();
            assert_eq!(encoded["kind"], kind);
            assert_eq!(
                serde_json::from_value::<AgentEvent>(encoded).unwrap(),
                event
            );
        }
    }
}

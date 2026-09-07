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
    pub project_id: i64,
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
            project_id,
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
            project_id: ticket.project_id,
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
}

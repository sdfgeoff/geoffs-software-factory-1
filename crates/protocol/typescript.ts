// Generated from crates/protocol/src/lib.rs. Do not edit.
export interface BoardSettings {
  merge_notification_channel: string | null;
}

export interface SetBoardSettingsRequest {
  merge_notification_channel?: string | null;
}

export interface Project {
  id: number;
  name: string;
  path: string;
  default_branch: string;
  completion_workflow: ProjectCompletionWorkflow;
  archived_at: number | null;
  updated_at: number;
}

export type ProjectCompletionWorkflow = "board_managed" | "external";

export type TicketState = "draft" | "todo" | "in_progress" | "review" | "awaiting_external_merge" | "done" | "cancelled";

export interface TicketBlocker {
  id: number;
  title: string;
}

export interface Ticket {
  id: number;
  project_id: number;
  title: string;
  description: string;
  state: TicketState;
  created_by_agent_id: number | null;
  reserved_for_agent_id: number | null;
  assignee_id: number | null;
  implementation_branch: string | null;
  review_commit: string | null;
  review_target_branch: string | null;
  review_project_id: number | null;
  review_project_path: string | null;
  reviewer_id: number | null;
  review_feedback: string | null;
  review_completion_workflow: ProjectCompletionWorkflow | null;
  external_change_url: string | null;
  external_merge_reference: string | null;
  external_merged_at: number | null;
  blocker_ids: number[];
  unresolved_blockers: TicketBlocker[];
  revision: number;
  created_at: number;
  updated_at: number;
}

export interface TicketSummary {
  id: number;
  project_id: number;
  title: string;
  state: TicketState;
  created_by_agent_id: number | null;
  reserved_for_agent_id: number | null;
  assignee_id: number | null;
  reviewer_id: number | null;
  review_completion_workflow: ProjectCompletionWorkflow | null;
  has_review_feedback: boolean;
  activity_count: number;
  latest_activity_id: number | null;
  unresolved_blockers: TicketBlocker[];
  revision: number;
  created_at: number;
  updated_at: number;
}

export interface TicketSummaryPage {
  tickets: TicketSummary[];
  has_more: boolean;
}

export interface TicketSummaryQuery {
  project_id?: number | null;
  state?: TicketState | null;
  before_id?: number | null;
  limit?: number | null;
}

export interface TicketActivity {
  id: number;
  ticket_id: number;
  action: string;
  detail: string | null;
  author: string | null;
  actor_agent_id: number | null;
  previous_reserved_for_agent_id: number | null;
  reserved_for_agent_id: number | null;
  implementation_branch: string | null;
  review_commit: string | null;
  review_target_branch: string | null;
  attachments?: Attachment[];
  created_at: number;
}

export interface TicketActivityPage {
  entries: TicketActivity[];
  has_more: boolean;
}

export interface TicketActivityQuery {
  before_id?: number | null;
  limit?: number | null;
}

export interface PostTicketCommentRequest {
  body: string;
  attachment_ids?: string[];
}

export interface ProjectTransition {
  agent_id: number;
  ticket_id: number;
  previous_project_id: number | null;
  project_id: number;
  project_name: string;
  worktree_path: string;
  message: string;
}

export type TicketClaimResponse = Ticket & {
  project_transition: ProjectTransition | null;
};

export interface Channel {
  name: string;
  description: string;
  most_recent_message_at: number | null;
}

export interface Message {
  id: number;
  channel_name: string;
  agent_id: number | null;
  author: string;
  body: string;
  attachments?: Attachment[];
  created_at: number;
}

export type BoardMessage = Message;

export interface Attachment {
  id: string;
  original_filename: string;
  media_type: string;
  byte_size: number;
  download_url: string;
}

export interface MessagePage {
  messages: Message[];
  has_more: boolean;
}

export type AgentState = "paused" | "idle" | "running" | "awaiting_approval" | "error";

export interface Agent {
  id: number;
  name: string;
  instructions: string;
  state: AgentState;
  thread_id: string | null;
  home_path: string;
  active_project_id: number | null;
  active_turn_id: string | null;
  last_error: string | null;
  wake_on_pending_work: boolean;
  wake_on_pending_review: boolean;
  idle_heartbeat_minutes: number | null;
  idle_since: number | null;
  created_at: number;
  updated_at: number;
}

export interface AgentProjectWorktree {
  agent_id: number;
  project_id: number;
  path: string;
  created_at: number;
  updated_at: number;
}

export type AgentMessageRole = "user" | "agent" | "system";

export interface AgentMessage {
  id: number;
  agent_id: number;
  role: AgentMessageRole;
  body: string;
  attachments?: Attachment[];
  created_at: number;
}

export interface AgentMessagePage {
  messages: AgentMessage[];
  has_more: boolean;
}

export type ReminderState = "scheduled" | "enqueued" | "cancelled";

export interface Reminder {
  id: number;
  agent_id: number;
  message: string;
  deliver_at: number;
  state: ReminderState;
  wake_event_id: number | null;
  enqueued_at: number | null;
  cancelled_at: number | null;
  created_at: number;
  updated_at: number;
}

export type WakeDeliveryPolicy = "idle_only" | "immediate";

export type WakeEventState = "pending" | "delivering" | "delivered";

export type WakeEvent = WakeEventData & {
  id: number;
  agent_id: number;
  deduplication_key: string | null;
  delivery_policy: WakeDeliveryPolicy;
  delivery_state: WakeEventState;
  delivery_attempts: number;
  last_attempt_at: number | null;
  delivered_at: number | null;
  created_at: number;
  updated_at: number;
};

export type WakeEventData =
  | { kind: "notification"; payload: NotificationWakePayload }
  | { kind: "message"; payload: AgentMessageWakePayload }
  | { kind: "channel.message"; payload: ChannelMessageWakePayload }
  | { kind: "idle.heartbeat"; payload: IdleHeartbeatWakePayload }
  | { kind: "reminder"; payload: ReminderWakePayload }
  | { kind: "ticket.available"; payload: TicketWakePayload }
  | { kind: "ticket.assigned"; payload: TicketWakePayload }
  | { kind: "ticket.reserved"; payload: TicketWakePayload }
  | { kind: "ticket.review_requested"; payload: TicketWakePayload }
  | { kind: "ticket.review_claimed"; payload: TicketWakePayload }
  | { kind: "ticket.changes_requested"; payload: TicketWakePayload }
  | { kind: "ticket.external_merge_ready"; payload: TicketWakePayload };

export interface NotificationWakePayload {
  message: string;
  activity_id?: number | null;
}

export interface AgentMessageWakePayload {
  message: string;
  message_id: number;
}

export interface ChannelMessageWakePayload {
  message: string;
  channel: string;
  message_id: number;
  author: string;
  body: string;
  mentioned: boolean;
  subscribed: boolean;
}

export interface IdleHeartbeatWakePayload {
  message: string;
  minutes: number;
  occurred_at: string;
}

export interface ReminderWakePayload {
  message: string;
  requested_delivery_at: number;
}

export interface TicketWakePayload {
  message: string;
  ticket_id: number;
  activity_id?: number | null;
  revision?: number | null;
  changed_fields?: string[];
  ticket: TicketWakeTicket;
}

export interface TicketWakeTicket {
  project_id?: number | null;
  title?: string | null;
  state?: TicketState | null;
  assignee_id?: number | null;
  created_by_agent_id?: number | null;
  reserved_for_agent_id?: number | null;
  implementation_branch?: string | null;
  review_commit?: string | null;
  review_target_branch?: string | null;
  review_project_id?: number | null;
  review_project_path?: string | null;
  reviewer_id?: number | null;
  blocker_ids?: number[];
  unresolved_blockers?: TicketBlocker[];
}

export type AgentEvent =
  | { kind: "refresh" }
  | { kind: "snapshot_reset"; data: SnapshotReset }
  | { kind: "projects_changed" }
  | { kind: "channels_changed" }
  | { kind: "agent_created"; agent_id: number; data: Agent }
  | { kind: "agent_updated"; agent_id: number; data: Agent }
  | { kind: "agent_message_delta"; agent_id: number; data: AgentMessageDelta }
  | { kind: "message"; agent_id: number; data: AgentMessage }
  | { kind: "ticket_updated"; agent_id: number; data: Ticket }
  | { kind: "ticket_activity_created"; agent_id: number; data: TicketActivity }
  | { kind: "external_review_approved"; agent_id: number; data: Ticket }
  | { kind: "external_change_updated"; agent_id: number; data: Ticket }
  | { kind: "external_merge_returned"; agent_id: number; data: Ticket }
  | { kind: "external_merge_completed"; agent_id: number; data: Ticket }
  | { kind: "settings_updated"; agent_id: number; data: BoardSettings }
  | { kind: "board_message"; agent_id: number; data: Message }
  | { kind: "channel_subscriptions"; agent_id: number; data: string[] }
  | { kind: "project_subscriptions"; agent_id: number; data: number[] }
  | { kind: "approval_declined"; agent_id: number }
  | { kind: "codex_event"; agent_id: number; data: unknown };

export interface SnapshotReset {
  scopes: SnapshotScope[];
}

export type SnapshotScope = "board" | "messages";

export interface AgentMessageDelta {
  delta: string;
}

export interface RegisterProjectRequest {
  name: string;
  path: string;
  default_branch: string;
  completion_workflow?: ProjectCompletionWorkflow;
}

export interface SetProjectCompletionWorkflowRequest {
  completion_workflow: ProjectCompletionWorkflow;
}

export interface CreateTicketRequest {
  project_id: number;
  title: string;
  description?: string;
  blocker_ids?: number[];
  state?: TicketState | null;
}

export interface UpdateTicketRequest {
  title?: string | null;
  description?: string | null;
  state?: TicketState | null;
  assignee_id?: number | null;
  reserved_for_agent_id?: number | null;
  blocker_ids?: number[] | null;
  expected_revision?: number | null;
}

export interface ReserveTicketsRequest {
  ticket_ids: number[];
  agent_id?: number | null;
  administrative_override?: boolean;
}

export interface ClaimTicketRequest {
  agent_id?: number | null;
  expected_revision?: number | null;
}

export interface MessagePageQuery {
  before_id?: number | null;
  after_id?: number | null;
  limit?: number | null;
}

export interface PostMessageRequest {
  author: string;
  body: string;
  attachment_ids?: string[];
}

export interface CreateChannelRequest {
  name: string;
  description?: string;
}

export interface CreateAgentRequest {
  name: string;
  project_id: number;
}

export interface RenameAgentRequest {
  name: string;
}

export interface SetAgentInstructionsRequest {
  instructions: string;
}

export interface SendAgentMessageRequest {
  message: string;
  attachment_ids?: string[];
}

export type CreateWakeEventRequest = WakeEventData & {
  deduplication_key?: string | null;
};

export interface ScheduleSelfReminderRequest {
  message: string;
  deliver_at?: string | null;
  delay_seconds?: number | null;
}

export interface SetAgentScopesRequest {
  scopes: string[];
}

export interface SetAgentWakeSettingsRequest {
  wake_on_pending_work: boolean;
  wake_on_pending_review: boolean;
  idle_heartbeat_minutes?: number | null;
}

export interface SetAgentChannelSubscriptionsRequest {
  channels: string[];
}

export interface SetAgentProjectSubscriptionsRequest {
  project_ids: number[];
}

export interface ReviewRevisionRequest {
  expected_revision?: number | null;
}

export interface RequestChangesRequest {
  feedback: string;
  expected_revision?: number | null;
  attachment_ids?: string[];
}

export interface SetExternalChangeRequest {
  external_change_url: string;
  expected_revision?: number | null;
}

export interface ReturnExternalMergeRequest {
  reason: string;
  expected_revision?: number | null;
}

export interface CompleteExternalMergeRequest {
  confirmation: string;
  merge_reference?: string | null;
  expected_revision?: number | null;
}

export interface ErrorResponse {
  error: string;
}

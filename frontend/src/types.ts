export type TicketState =
  | "todo"
  | "in_progress"
  | "review"
  | "done"
  | "cancelled";
export type AgentState =
  "paused" | "idle" | "running" | "awaiting_approval" | "error";
export interface BoardSettings {
  merge_notification_channel: string | null;
}
export interface Project {
  id: number;
  name: string;
  path: string;
  default_branch: string;
  archived_at: number | null;
  updated_at: number;
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
  blocker_ids: number[];
  unresolved_blockers: TicketBlocker[];
  revision: number;
  created_at: number;
  updated_at: number;
}
export interface TicketBlocker {
  id: number;
  title: string;
}
export interface Channel {
  name: string;
  description: string;
  most_recent_message_at: number | null;
}
export interface BoardMessage {
  id: number;
  channel_name: string;
  agent_id: number | null;
  author: string;
  body: string;
  created_at: number;
}
export interface Agent {
  id: number;
  name: string;
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
export interface AgentMessage {
  id: number;
  agent_id: number;
  role: "user" | "agent" | "system";
  body: string;
  created_at: number;
}
export type AgentEvent =
  | { kind: "refresh" }
  | {
      kind: "agent_created" | "agent_updated";
      agent_id: number;
      data: Agent;
    }
  | { kind: "agent_message_delta"; agent_id: number; data: { delta: string } }
  | { kind: "message"; agent_id: number; data: AgentMessage }
  | { kind: "ticket_updated"; agent_id: number; data: Ticket }
  | { kind: "settings_updated"; agent_id: number; data: BoardSettings }
  | { kind: "board_message"; agent_id: number; data: BoardMessage }
  | { kind: "channel_subscriptions"; agent_id: number; data: string[] }
  | { kind: "project_subscriptions"; agent_id: number; data: number[] }
  | { kind: "approval_declined"; agent_id: number }
  | { kind: "codex_event"; agent_id: number; data: unknown };

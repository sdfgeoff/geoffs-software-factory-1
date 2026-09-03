export type TicketState = "todo" | "in_progress" | "review" | "done";
export type AgentState =
  "paused" | "idle" | "running" | "awaiting_approval" | "error";
export interface Project {
  path: string;
  updated_at: number;
}
export interface Ticket {
  id: number;
  title: string;
  description: string;
  state: TicketState;
  assignee_id: number | null;
  review_commit: string | null;
  review_target_branch: string | null;
  reviewer_id: number | null;
  review_feedback: string | null;
  revision: number;
  created_at: number;
  updated_at: number;
}
export interface Channel {
  name: string;
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
  worktree_path: string;
  active_turn_id: string | null;
  last_error: string | null;
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
export interface AgentEvent {
  agent_id?: number;
  kind: string;
  data?: { delta?: string };
}

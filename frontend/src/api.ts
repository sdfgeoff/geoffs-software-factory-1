import type {
  Agent,
  AgentMessagePage,
  AgentProjectWorktree,
  BoardMessage,
  BoardSettings,
  Channel,
  MessagePage,
  MessagePageQuery,
  Project,
  Ticket,
  TicketSummaryPage,
  TicketSummaryQuery,
} from "./types";

function pageQuery(query: MessagePageQuery = {}) {
  const search = new URLSearchParams();
  if (query.before_id !== undefined && query.before_id !== null) {
    search.set("before_id", String(query.before_id));
  }
  if (query.after_id !== undefined && query.after_id !== null) {
    search.set("after_id", String(query.after_id));
  }
  if (query.limit !== undefined && query.limit !== null) {
    search.set("limit", String(query.limit));
  }
  const suffix = search.toString();
  return suffix ? `?${suffix}` : "";
}

function ticketPageQuery(query: TicketSummaryQuery = {}) {
  const search = new URLSearchParams();
  if (query.project_id != null) search.set("project_id", String(query.project_id));
  if (query.state != null) search.set("state", query.state);
  if (query.before_id != null) search.set("before_id", String(query.before_id));
  if (query.limit != null) search.set("limit", String(query.limit));
  const suffix = search.toString();
  return suffix ? `?${suffix}` : "";
}

async function request<T>(path: string, options?: RequestInit): Promise<T> {
  const response = await fetch(`/api${path}`, {
    ...options,
    headers: options?.body
      ? { "Content-Type": "application/json", ...options.headers }
      : options?.headers,
  });
  if (!response.ok) {
    const body = (await response.json().catch(() => null)) as {
      error?: string;
    } | null;
    throw new Error(body?.error ?? `${response.status} ${response.statusText}`);
  }
  return response.status === 204
    ? (undefined as T)
    : ((await response.json()) as T);
}

export const api = {
  settings: () => request<BoardSettings>("/settings"),
  setSettings: (mergeNotificationChannel: string | null) =>
    request<BoardSettings>("/settings", {
      method: "PUT",
      body: JSON.stringify({
        merge_notification_channel: mergeNotificationChannel,
      }),
    }),
  project: () => request<Project | null>("/project"),
  setProject: (path: string) =>
    request<Project>("/project", {
      method: "PUT",
      body: JSON.stringify({ path }),
    }),
  projects: () => request<Project[]>("/projects"),
  registerProject: (name: string, path: string, defaultBranch: string) =>
    request<Project>("/projects", {
      method: "POST",
      body: JSON.stringify({ name, path, default_branch: defaultBranch }),
    }),
  archiveProject: (id: number) =>
    request<Project>(`/projects/${id}/archive`, { method: "POST" }),
  tickets: (query: TicketSummaryQuery = {}) =>
    request<TicketSummaryPage>(`/tickets${ticketPageQuery(query)}`),
  ticket: (id: number) => request<Ticket>(`/tickets/${id}`),
  createTicket: (title: string, projectId?: number) =>
    request<Ticket>("/tickets", {
      method: "POST",
      body: JSON.stringify({ title, project_id: projectId }),
    }),
  updateTicket: (ticket: Ticket, changes: Partial<Ticket>) =>
    request<Ticket>(`/tickets/${ticket.id}`, {
      method: "PATCH",
      body: JSON.stringify({ ...changes, expected_revision: ticket.revision }),
    }),
  clearTicketReservation: (ticket: Ticket) =>
    request<Ticket>(`/tickets/${ticket.id}`, {
      method: "PATCH",
      body: JSON.stringify({
        reserved_for_agent_id: null,
        expected_revision: ticket.revision,
      }),
    }),
  reserveTickets: (
    ticketIds: number[],
    agentId: number,
    administrativeOverride = false,
  ) =>
    request<Ticket[]>("/ticket-reservations", {
      method: "POST",
      body: JSON.stringify({
        ticket_ids: ticketIds,
        agent_id: agentId,
        administrative_override: administrativeOverride,
      }),
    }),
  channels: () => request<Channel[]>("/channels"),
  createChannel: (name: string, description: string) =>
    request<Channel>("/channels", {
      method: "POST",
      body: JSON.stringify({ name, description }),
    }),
  messages: (channel: string, query: MessagePageQuery = {}) =>
    request<MessagePage>(
      `/channels/${encodeURIComponent(channel)}/messages${pageQuery(query)}`,
    ),
  postMessage: (channel: string, body: string) =>
    request<BoardMessage>(`/channels/${encodeURIComponent(channel)}/messages`, {
      method: "POST",
      body: JSON.stringify({ author: "human", body }),
    }),
  agents: () => request<Agent[]>("/agents"),
  createAgent: (name: string) =>
    request<Agent>("/agents", {
      method: "POST",
      body: JSON.stringify({ name }),
    }),
  renameAgent: (id: number, name: string) =>
    request<Agent>(`/agents/${id}`, {
      method: "PATCH",
      body: JSON.stringify({ name }),
    }),
  agentScopes: (id: number) => request<string[]>(`/agents/${id}/scopes`),
  setAgentScopes: (id: number, scopes: string[]) =>
    request<string[]>(`/agents/${id}/scopes`, {
      method: "PUT",
      body: JSON.stringify({ scopes }),
    }),
  setAgentWakeSettings: (
    id: number,
    wakeOnPendingWork: boolean,
    wakeOnPendingReview: boolean,
    idleHeartbeatMinutes: number | null,
  ) =>
    request<Agent>(`/agents/${id}/wake-settings`, {
      method: "PUT",
      body: JSON.stringify({
        wake_on_pending_work: wakeOnPendingWork,
        wake_on_pending_review: wakeOnPendingReview,
        idle_heartbeat_minutes: idleHeartbeatMinutes,
      }),
    }),
  agentChannelSubscriptions: (id: number) =>
    request<string[]>(`/agents/${id}/channel-subscriptions`),
  setAgentChannelSubscriptions: (id: number, channels: string[]) =>
    request<string[]>(`/agents/${id}/channel-subscriptions`, {
      method: "PUT",
      body: JSON.stringify({ channels }),
    }),
  agentProjectSubscriptions: (id: number) =>
    request<number[]>(`/agents/${id}/project-subscriptions`),
  setAgentProjectSubscriptions: (id: number, projectIds: number[]) =>
    request<number[]>(`/agents/${id}/project-subscriptions`, {
      method: "PUT",
      body: JSON.stringify({ project_ids: projectIds }),
    }),
  pauseAgent: (id: number) =>
    request<Agent>(`/agents/${id}/pause`, { method: "POST" }),
  resumeAgent: (id: number) =>
    request<Agent>(`/agents/${id}/resume`, { method: "POST" }),
  agentMessages: (id: number, query: MessagePageQuery = {}) =>
    request<AgentMessagePage>(`/agents/${id}/messages${pageQuery(query)}`),
  agentProjectWorktrees: (id: number) =>
    request<AgentProjectWorktree[]>(`/agents/${id}/project-worktrees`),
  sendAgentMessage: (id: number, message: string) =>
    request<Agent>(`/agents/${id}/messages`, {
      method: "POST",
      body: JSON.stringify({ message }),
    }),
};

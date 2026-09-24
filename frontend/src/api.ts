import type {
  Agent,
  Attachment,
  AgentMessagePage,
  AgentProjectWorktree,
  BoardMessage,
  BoardSettings,
  Channel,
  MessagePage,
  MessagePageQuery,
  Project,
  Ticket,
  TicketActivity,
  TicketActivityPage,
  TicketSummaryPage,
  TicketSummaryQuery,
  ProjectCompletionWorkflow,
} from "./types";

export class ApiError extends Error {
  constructor(message: string, readonly status: number) {
    super(message);
  }
}

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
    throw new ApiError(
      body?.error ?? `${response.status} ${response.statusText}`,
      response.status,
    );
  }
  return response.status === 204
    ? (undefined as T)
    : ((await response.json()) as T);
}

export const api = {
  uploadAttachment: async (file: File) => {
    const body = new FormData();
    body.append("file", file, file.name);
    const response = await fetch("/api/attachments", { method: "POST", body });
    if (!response.ok) {
      const error = (await response.json().catch(() => null)) as { error?: string } | null;
      throw new ApiError(error?.error ?? `${response.status} ${response.statusText}`, response.status);
    }
    return (await response.json()) as Attachment;
  },
  settings: () => request<BoardSettings>("/settings"),
  setSettings: (mergeNotificationChannel: string | null) =>
    request<BoardSettings>("/settings", {
      method: "PUT",
      body: JSON.stringify({
        merge_notification_channel: mergeNotificationChannel,
      }),
    }),
  projects: () => request<Project[]>("/projects"),
  registerProject: (
    name: string,
    path: string,
    defaultBranch: string,
    completionWorkflow: ProjectCompletionWorkflow = "board_managed",
  ) =>
    request<Project>("/projects", {
      method: "POST",
      body: JSON.stringify({
        name,
        path,
        default_branch: defaultBranch,
        completion_workflow: completionWorkflow,
      }),
    }),
  setProjectCompletionWorkflow: (
    id: number,
    completionWorkflow: ProjectCompletionWorkflow,
  ) =>
    request<Project>(`/projects/${id}/completion-workflow`, {
      method: "PUT",
      body: JSON.stringify({ completion_workflow: completionWorkflow }),
    }),
  archiveProject: (id: number) =>
    request<Project>(`/projects/${id}/archive`, { method: "POST" }),
  tickets: (query: TicketSummaryQuery = {}) =>
    request<TicketSummaryPage>(`/tickets${ticketPageQuery(query)}`),
  ticket: (id: number) => request<Ticket>(`/tickets/${id}`),
  ticketActivity: (id: number, beforeId?: number, limit = 50) => {
    const query = new URLSearchParams({ limit: String(limit) });
    if (beforeId != null) query.set("before_id", String(beforeId));
    return request<TicketActivityPage>(`/tickets/${id}/activity?${query}`);
  },
  addTicketComment: (id: number, body: string, attachmentIds: string[] = []) =>
    request<TicketActivity>(`/tickets/${id}/activity`, {
      method: "POST",
      body: JSON.stringify({ body, attachment_ids: attachmentIds }),
    }),
  createTicket: (ticket: {
    title: string;
    description: string;
    blocker_ids: number[];
    project_id: number;
  }) =>
    request<Ticket>("/tickets", {
      method: "POST",
      body: JSON.stringify(ticket),
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
  setExternalChange: (ticket: Ticket, externalChangeUrl: string) =>
    request<Ticket>(`/tickets/${ticket.id}/external-change`, {
      method: "PUT",
      body: JSON.stringify({
        external_change_url: externalChangeUrl,
        expected_revision: ticket.revision,
      }),
    }),
  returnExternalMerge: (ticket: Ticket, reason: string) =>
    request<Ticket>(`/tickets/${ticket.id}/return-external-merge`, {
      method: "POST",
      body: JSON.stringify({ reason, expected_revision: ticket.revision }),
    }),
  completeExternalMerge: (
    ticket: Ticket,
    confirmation: string,
    mergeReference?: string,
  ) =>
    request<Ticket>(`/tickets/${ticket.id}/complete-external-merge`, {
      method: "POST",
      body: JSON.stringify({
        confirmation,
        merge_reference: mergeReference || null,
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
  postMessage: (channel: string, body: string, attachmentIds: string[] = []) =>
    request<BoardMessage>(`/channels/${encodeURIComponent(channel)}/messages`, {
      method: "POST",
      body: JSON.stringify({ author: "human", body, attachment_ids: attachmentIds }),
    }),
  agents: () => request<Agent[]>("/agents"),
  createAgent: (name: string, projectId: number) =>
    request<Agent>("/agents", {
      method: "POST",
      body: JSON.stringify({ name, project_id: projectId }),
    }),
  renameAgent: (id: number, name: string) =>
    request<Agent>(`/agents/${id}`, {
      method: "PATCH",
      body: JSON.stringify({ name }),
    }),
  setAgentInstructions: (id: number, instructions: string) =>
    request<Agent>(`/agents/${id}/instructions`, {
      method: "PUT",
      body: JSON.stringify({ instructions }),
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
  sendAgentMessage: (id: number, message: string, attachmentIds: string[] = []) =>
    request<Agent>(`/agents/${id}/messages`, {
      method: "POST",
      body: JSON.stringify({ message, attachment_ids: attachmentIds }),
    }),
};

import type {
  Agent,
  AgentMessage,
  BoardMessage,
  Channel,
  Project,
  Ticket,
} from "./types";

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
  project: () => request<Project | null>("/project"),
  setProject: (path: string) =>
    request<Project>("/project", {
      method: "PUT",
      body: JSON.stringify({ path }),
    }),
  tickets: () => request<Ticket[]>("/tickets"),
  createTicket: (title: string) =>
    request<Ticket>("/tickets", {
      method: "POST",
      body: JSON.stringify({ title }),
    }),
  updateTicket: (ticket: Ticket, changes: Partial<Ticket>) =>
    request<Ticket>(`/tickets/${ticket.id}`, {
      method: "PATCH",
      body: JSON.stringify({ ...changes, expected_revision: ticket.revision }),
    }),
  channels: () => request<Channel[]>("/channels"),
  messages: (channel: string) =>
    request<BoardMessage[]>(
      `/channels/${encodeURIComponent(channel)}/messages`,
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
  pauseAgent: (id: number) =>
    request<Agent>(`/agents/${id}/pause`, { method: "POST" }),
  resumeAgent: (id: number) =>
    request<Agent>(`/agents/${id}/resume`, { method: "POST" }),
  agentMessages: (id: number) =>
    request<AgentMessage[]>(`/agents/${id}/messages`),
  sendAgentMessage: (id: number, message: string) =>
    request<Agent>(`/agents/${id}/messages`, {
      method: "POST",
      body: JSON.stringify({ message }),
    }),
};

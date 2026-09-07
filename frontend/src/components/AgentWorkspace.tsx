import { useEffect, useState, type FormEvent } from "react";
import { api } from "../api";
import type {
  Agent,
  AgentMessage,
  AgentProjectWorktree,
  Channel,
  Project,
  Ticket,
} from "../types";
import { MessageContent } from "./MessageContent";

export type AgentView = "chat" | "settings";

interface Props {
  agents: Agent[];
  channels: Channel[];
  tickets: Ticket[];
  projects?: Project[];
  selectedAgentId: number | null;
  view: AgentView;
  refresh: number;
  detailRefresh?: number;
  deltas: Record<number, string>;
  onSelectAgent: (id: number) => void;
  onViewChange: (view: AgentView) => void;
  onChanged: (agent: Agent) => void | Promise<void>;
  onError: (error: unknown) => void;
}

interface AgentSettingsDraft {
  name: string;
  wakeOnPendingWork: boolean;
  wakeOnPendingReview: boolean;
  heartbeatEnabled: boolean;
  heartbeatMinutes: string;
}

export function AgentWorkspace({
  agents,
  channels,
  tickets,
  projects = [],
  selectedAgentId,
  view,
  refresh,
  detailRefresh = 0,
  deltas,
  onSelectAgent,
  onViewChange,
  onChanged,
  onError,
}: Props) {
  const [messages, setMessages] = useState<AgentMessage[]>([]);
  const [messagesLoading, setMessagesLoading] = useState(false);
  const [messagesAgentId, setMessagesAgentId] = useState<number | null>(null);
  const [worktrees, setWorktrees] = useState<AgentProjectWorktree[]>([]);
  const [detailsAgentId, setDetailsAgentId] = useState<number | null>(null);
  const [name, setName] = useState("");
  const [prompts, setPrompts] = useState<Record<number, string>>({});
  const [settingsDrafts, setSettingsDrafts] = useState<
    Record<number, AgentSettingsDraft>
  >({});
  const [scopes, setScopes] = useState<string[]>([]);
  const [subscriptions, setSubscriptions] = useState<string[]>([]);
  const selected =
    agents.find((agent) => agent.id === selectedAgentId) ?? null;
  const prompt = selected ? prompts[selected.id] ?? "" : "";
  const settingsDraft = selected
    ? settingsDrafts[selected.id] ?? {
        name: selected.name,
        wakeOnPendingWork: selected.wake_on_pending_work,
        wakeOnPendingReview: selected.wake_on_pending_review,
        heartbeatEnabled: selected.idle_heartbeat_minutes !== null,
        heartbeatMinutes: String(selected.idle_heartbeat_minutes ?? 20),
      }
    : null;
  const currentTicket = tickets.find(
    (ticket) =>
      ticket.assignee_id === selected?.id &&
      (ticket.state === "in_progress" || ticket.state === "review"),
  );
  const activeProject = projects.find(
    (project) => project.id === selected?.active_project_id,
  );
  const currentWorktrees = detailsAgentId === selectedAgentId ? worktrees : [];
  const currentScopes = detailsAgentId === selectedAgentId ? scopes : [];
  const currentSubscriptions =
    detailsAgentId === selectedAgentId ? subscriptions : [];
  const activeWorktree = currentWorktrees.find(
    (worktree) => worktree.project_id === selected?.active_project_id,
  );
  useEffect(() => {
    if (selectedAgentId === null && agents[0]) onSelectAgent(agents[0].id);
  }, [agents, selectedAgentId, onSelectAgent]);
  useEffect(() => {
    if (!selected) return;
    setSettingsDrafts((current) =>
      current[selected.id]
        ? current
        : {
            ...current,
            [selected.id]: {
              name: selected.name,
              wakeOnPendingWork: selected.wake_on_pending_work,
              wakeOnPendingReview: selected.wake_on_pending_review,
              heartbeatEnabled: selected.idle_heartbeat_minutes !== null,
              heartbeatMinutes: String(selected.idle_heartbeat_minutes ?? 20),
            },
          },
    );
  }, [selected]);
  useEffect(() => {
    let current = true;
    setMessages([]);
    setMessagesLoading(Boolean(selectedAgentId));
    if (selectedAgentId) {
      void api
        .agentMessages(selectedAgentId)
        .then((next) => {
          if (current) {
            setMessages(next);
            setMessagesAgentId(selectedAgentId);
          }
        })
        .catch(onError)
        .finally(() => {
          if (current) setMessagesLoading(false);
        });
    }
    return () => {
      current = false;
    };
  }, [selectedAgentId, refresh, onError]);
  useEffect(() => {
    let current = true;
    setWorktrees([]);
    setScopes([]);
    setSubscriptions([]);
    setDetailsAgentId(null);
    if (selectedAgentId) {
      void Promise.all([
        api.agentProjectWorktrees(selectedAgentId),
        api.agentScopes(selectedAgentId),
        api.agentChannelSubscriptions(selectedAgentId),
      ]).then(([nextWorktrees, nextScopes, nextSubscriptions]) => {
        if (!current) return;
        setWorktrees(nextWorktrees);
        setScopes(nextScopes);
        setSubscriptions(nextSubscriptions);
        setDetailsAgentId(selectedAgentId);
      }).catch(onError);
    }
    return () => {
      current = false;
    };
  }, [selectedAgentId, detailRefresh, onError]);
  function updateSettings(changes: Partial<AgentSettingsDraft>) {
    if (!selected || !settingsDraft) return;
    setSettingsDrafts((current) => ({
      ...current,
      [selected.id]: { ...settingsDraft, ...changes },
    }));
  }
  async function create(event: FormEvent) {
    event.preventDefault();
    try {
      const agent = await api.createAgent(name);
      setName("");
      onSelectAgent(agent.id);
      await onChanged(agent);
    } catch (error) {
      onError(error);
    }
  }
  async function send(event: FormEvent) {
    event.preventDefault();
    if (!selected) return;
    try {
      const updated = await api.sendAgentMessage(selected.id, prompt);
      setPrompts((current) => ({ ...current, [selected.id]: "" }));
      await onChanged(updated);
    } catch (error) {
      onError(error);
    }
  }
  async function rename(event: FormEvent) {
    event.preventDefault();
    if (!selected) return;
    try {
      const updated = await api.renameAgent(selected.id, settingsDraft!.name);
      updateSettings({ name: updated.name });
      await onChanged(updated);
    } catch (error) {
      onError(error);
    }
  }
  async function toggle() {
    if (!selected) return;
    try {
      const updated = selected.state === "paused" || selected.state === "error"
        ? await api.resumeAgent(selected.id)
        : await api.pauseAgent(selected.id);
      await onChanged(updated);
    } catch (error) {
      onError(error);
    }
  }
  async function setScope(scope: string, enabled: boolean) {
    if (!selected) return;
    const next = enabled
      ? [...new Set([...currentScopes, scope])]
      : currentScopes.filter((item) => item !== scope);
    try {
      setScopes(await api.setAgentScopes(selected.id, next));
    } catch (error) {
      onError(error);
    }
  }
  async function setSubscription(channel: string, enabled: boolean) {
    if (!selected) return;
    const next = enabled
      ? [...new Set([...currentSubscriptions, channel])]
      : currentSubscriptions.filter((item) => item !== channel);
    try {
      setSubscriptions(
        await api.setAgentChannelSubscriptions(selected.id, next),
      );
    } catch (error) {
      onError(error);
    }
  }
  async function saveWakeSettings(event: FormEvent) {
    event.preventDefault();
    if (!selected) return;
    try {
      const updated = await api.setAgentWakeSettings(
        selected.id,
        settingsDraft!.wakeOnPendingWork,
        settingsDraft!.wakeOnPendingReview,
        settingsDraft!.heartbeatEnabled
          ? Number(settingsDraft!.heartbeatMinutes)
          : null,
      );
      await onChanged(updated);
    } catch (error) {
      onError(error);
    }
  }
  return (
    <section className="agents-layout">
      <aside>
        <h2>Agents</h2>
        {agents.map((agent) => (
          <button
            className={
              selected?.id === agent.id
                ? "selected agent-button"
                : "ghost agent-button"
            }
            key={agent.id}
            aria-current={selected?.id === agent.id ? "page" : undefined}
            onClick={() => onSelectAgent(agent.id)}
          >
            <span>{agent.name}</span>
            <small data-state={agent.state}>
              {agent.state.replace("_", " ")}
            </small>
          </button>
        ))}
        <form onSubmit={create}>
          <label>
            New agent
            <input
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder="backend"
              pattern="[A-Za-z0-9_-]+"
              required
            />
          </label>
          <button type="submit">Create agent</button>
        </form>
      </aside>
      <div className="conversation">
        {selected ? (
          <>
            <div className="conversation-title">
              <div>
                <div className="agent-heading">
                  <h2>{selected.name}</h2>
                  <a
                    className="secondary-link"
                    href={`#agents/${selected.id}${view === "chat" ? "/settings" : ""}`}
                    onClick={(event) => {
                      event.preventDefault();
                      onViewChange(view === "chat" ? "settings" : "chat");
                    }}
                  >
                    {view === "chat" ? "Settings" : "Back to chat"}
                  </a>
                </div>
                <p className="agent-operation">
                  <span data-state={selected.state}>
                    {selected.state.replace("_", " ")}
                  </span>
                  {currentTicket
                    ? `Ticket #${currentTicket.id}: ${currentTicket.title}`
                    : "No active ticket"}
                  {activeProject ? ` · ${activeProject.name}` : ""}
                </p>
                <p
                  className="worktree-path"
                  title={activeWorktree?.path ?? selected.home_path}
                >
                  {activeWorktree?.path ?? selected.home_path}
                </p>
              </div>
              <button className="secondary" onClick={toggle}>
                {selected.state === "paused" || selected.state === "error"
                  ? "Resume"
                  : "Pause"}
              </button>
            </div>
            {selected.last_error && (
              <p className="inline-error">{selected.last_error}</p>
            )}
            <section hidden={view !== "settings"}>
              <h3>Agent settings</h3>
              <section>
                <h4>Project worktrees</h4>
                <dl>
                  {currentWorktrees.map((worktree) => (
                    <div key={worktree.project_id}>
                      <dt>{projects.find((project) => project.id === worktree.project_id)?.name ?? `Project #${worktree.project_id}`}</dt>
                      <dd className="worktree-path">{worktree.path}</dd>
                    </div>
                  ))}
                </dl>
              </section>
              <form className="rename-agent" onSubmit={rename}>
                <label>
                  Agent name
                  <input
                    value={settingsDraft?.name ?? ""}
                    onChange={(event) => updateSettings({ name: event.target.value })}
                    pattern="[A-Za-z0-9_-]+"
                    maxLength={50}
                    required
                  />
                </label>
                <button
                  type="submit"
                  disabled={
                    settingsDraft?.name === selected.name ||
                    selected.state === "running" ||
                    selected.state === "awaiting_approval"
                  }
                >
                  Rename
                </button>
              </form>
              <fieldset className="capabilities">
                <legend>Capabilities</legend>
                <label>
                  <input
                    type="checkbox"
                      checked={currentScopes.includes("approve_review")}
                    onChange={(event) =>
                      void setScope("approve_review", event.target.checked)
                    }
                  />
                  <span>
                    <strong>approve_review</strong>
                    Review changes and authorize board-owned merges
                  </span>
                </label>
              </fieldset>
              <form className="wake-settings" onSubmit={saveWakeSettings}>
                <fieldset className="capabilities">
                  <legend>Wake settings</legend>
                  <label>
                    <input
                      type="checkbox"
                      checked={settingsDraft?.wakeOnPendingWork ?? false}
                      onChange={(event) =>
                        updateSettings({ wakeOnPendingWork: event.target.checked })
                      }
                    />
                    <span>
                      <strong>Pending work</strong>
                      Wake for available implementation work
                    </span>
                  </label>
                  <label>
                    <input
                      type="checkbox"
                      checked={settingsDraft?.wakeOnPendingReview ?? false}
                      onChange={(event) =>
                        updateSettings({ wakeOnPendingReview: event.target.checked })
                      }
                    />
                    <span>
                      <strong>Pending reviews</strong>
                      Wake when eligible to review
                    </span>
                  </label>
                  <label>
                    <input
                      type="checkbox"
                      checked={settingsDraft?.heartbeatEnabled ?? false}
                      onChange={(event) =>
                        updateSettings({ heartbeatEnabled: event.target.checked })
                      }
                    />
                    <span>
                      <strong>Idle heartbeat</strong>
                      Wake periodically while idle
                    </span>
                  </label>
                  <label className="heartbeat-minutes">
                    Minutes
                    <input
                      type="number"
                      min="1"
                      max="10080"
                      required={settingsDraft?.heartbeatEnabled ?? false}
                      disabled={!settingsDraft?.heartbeatEnabled}
                      value={settingsDraft?.heartbeatMinutes ?? "20"}
                      onChange={(event) =>
                        updateSettings({ heartbeatMinutes: event.target.value })
                      }
                    />
                  </label>
                </fieldset>
                <button type="submit">Save wake settings</button>
              </form>
              <fieldset className="capabilities">
                <legend>Channel subscriptions</legend>
                {channels.map((channel) => (
                  <label key={channel.name}>
                    <input
                      type="checkbox"
                      checked={currentSubscriptions.includes(channel.name)}
                      onChange={(event) =>
                        void setSubscription(channel.name, event.target.checked)
                      }
                    />
                    <span>
                      <strong>#{channel.name}</strong>
                      Wake for new messages
                    </span>
                  </label>
                ))}
              </fieldset>
            </section>
            <section className="agent-chat-panel" hidden={view !== "chat"}>
              <div className="message-list agent-chat">
                {(messagesLoading || messagesAgentId !== selected.id) && (
                  <p className="empty">Loading messages…</p>
                )}
                {messagesAgentId === selected.id && messages.map((message) => (
                  <article className={message.role} key={message.id}>
                    <strong>{message.role}</strong>
                    <MessageContent source={message.body} />
                  </article>
                ))}
                {deltas[selected.id] && (
                  <article className="agent streaming">
                    <strong>agent</strong>
                    <MessageContent source={deltas[selected.id]} />
                  </article>
                )}
              </div>
              <form onSubmit={send}>
                <label>
                  Message
                  <textarea
                    value={prompt}
                    onChange={(event) =>
                      setPrompts((current) => ({
                        ...current,
                        [selected.id]: event.target.value,
                      }))
                    }
                    required
                  />
                </label>
                <button type="submit">Send</button>
              </form>
            </section>
          </>
        ) : (
          <div className="empty">
            <h2>No agents yet</h2>
            <p>Create one after selecting a project.</p>
          </div>
        )}
      </div>
    </section>
  );
}

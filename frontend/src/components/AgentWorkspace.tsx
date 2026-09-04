import { useEffect, useState, type FormEvent } from "react";
import { api } from "../api";
import type { Agent, AgentMessage, Channel, Ticket } from "../types";
import { MessageContent } from "./MessageContent";

export type AgentView = "chat" | "settings";

interface Props {
  agents: Agent[];
  channels: Channel[];
  tickets: Ticket[];
  selectedAgentId: number | null;
  view: AgentView;
  refresh: number;
  deltas: Record<number, string>;
  onSelectAgent: (id: number) => void;
  onViewChange: (view: AgentView) => void;
  onChanged: () => Promise<void>;
  onError: (error: unknown) => void;
}

export function AgentWorkspace({
  agents,
  channels,
  tickets,
  selectedAgentId,
  view,
  refresh,
  deltas,
  onSelectAgent,
  onViewChange,
  onChanged,
  onError,
}: Props) {
  const [messages, setMessages] = useState<AgentMessage[]>([]);
  const [name, setName] = useState("");
  const [renamedName, setRenamedName] = useState("");
  const [prompt, setPrompt] = useState("");
  const [scopes, setScopes] = useState<string[]>([]);
  const [subscriptions, setSubscriptions] = useState<string[]>([]);
  const [wakeOnPendingTask, setWakeOnPendingTask] = useState(false);
  const [heartbeatEnabled, setHeartbeatEnabled] = useState(false);
  const [heartbeatMinutes, setHeartbeatMinutes] = useState("20");
  const selected =
    agents.find((agent) => agent.id === selectedAgentId) ?? null;
  const currentTicket = tickets.find(
    (ticket) =>
      ticket.assignee_id === selected?.id &&
      (ticket.state === "in_progress" || ticket.state === "review"),
  );
  useEffect(() => {
    if (!selected && agents[0]) onSelectAgent(agents[0].id);
  }, [agents, selected, onSelectAgent]);
  useEffect(() => setRenamedName(selected?.name ?? ""), [selected?.name]);
  useEffect(() => {
    setWakeOnPendingTask(selected?.wake_on_pending_task ?? false);
    setHeartbeatEnabled(selected?.idle_heartbeat_minutes !== null);
    setHeartbeatMinutes(String(selected?.idle_heartbeat_minutes ?? 20));
  }, [
    selected?.id,
    selected?.idle_heartbeat_minutes,
    selected?.wake_on_pending_task,
  ]);
  useEffect(() => {
    if (selectedAgentId)
      void api.agentMessages(selectedAgentId).then(setMessages).catch(onError);
  }, [selectedAgentId, refresh, onError]);
  useEffect(() => {
    if (selectedAgentId)
      void api.agentScopes(selectedAgentId).then(setScopes).catch(onError);
  }, [selectedAgentId, refresh, onError]);
  useEffect(() => {
    if (selectedAgentId)
      void api
        .agentChannelSubscriptions(selectedAgentId)
        .then(setSubscriptions)
        .catch(onError);
  }, [selectedAgentId, refresh, onError]);
  async function create(event: FormEvent) {
    event.preventDefault();
    try {
      const agent = await api.createAgent(name);
      setName("");
      onSelectAgent(agent.id);
      await onChanged();
    } catch (error) {
      onError(error);
    }
  }
  async function send(event: FormEvent) {
    event.preventDefault();
    if (!selected) return;
    try {
      await api.sendAgentMessage(selected.id, prompt);
      setPrompt("");
      await onChanged();
    } catch (error) {
      onError(error);
    }
  }
  async function rename(event: FormEvent) {
    event.preventDefault();
    if (!selected) return;
    try {
      await api.renameAgent(selected.id, renamedName);
      await onChanged();
    } catch (error) {
      onError(error);
    }
  }
  async function toggle() {
    if (!selected) return;
    try {
      selected.state === "paused" || selected.state === "error"
        ? await api.resumeAgent(selected.id)
        : await api.pauseAgent(selected.id);
      await onChanged();
    } catch (error) {
      onError(error);
    }
  }
  async function setScope(scope: string, enabled: boolean) {
    if (!selected) return;
    const next = enabled
      ? [...new Set([...scopes, scope])]
      : scopes.filter((item) => item !== scope);
    try {
      setScopes(await api.setAgentScopes(selected.id, next));
      await onChanged();
    } catch (error) {
      onError(error);
    }
  }
  async function setSubscription(channel: string, enabled: boolean) {
    if (!selected) return;
    const next = enabled
      ? [...new Set([...subscriptions, channel])]
      : subscriptions.filter((item) => item !== channel);
    try {
      setSubscriptions(
        await api.setAgentChannelSubscriptions(selected.id, next),
      );
      await onChanged();
    } catch (error) {
      onError(error);
    }
  }
  async function saveWakeSettings(event: FormEvent) {
    event.preventDefault();
    if (!selected) return;
    try {
      await api.setAgentWakeSettings(
        selected.id,
        wakeOnPendingTask,
        heartbeatEnabled ? Number(heartbeatMinutes) : null,
      );
      await onChanged();
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
                </p>
                <p className="worktree-path" title={selected.worktree_path}>
                  {selected.worktree_path}
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
              <form className="rename-agent" onSubmit={rename}>
                <label>
                  Agent name
                  <input
                    value={renamedName}
                    onChange={(event) => setRenamedName(event.target.value)}
                    pattern="[A-Za-z0-9_-]+"
                    maxLength={50}
                    required
                  />
                </label>
                <button
                  type="submit"
                  disabled={
                    renamedName === selected.name ||
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
                    checked={scopes.includes("approve_review")}
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
                      checked={wakeOnPendingTask}
                      onChange={(event) =>
                        setWakeOnPendingTask(event.target.checked)
                      }
                    />
                    <span>
                      <strong>Pending tasks</strong>
                      Wake for available work and reviews
                    </span>
                  </label>
                  <label>
                    <input
                      type="checkbox"
                      checked={heartbeatEnabled}
                      onChange={(event) =>
                        setHeartbeatEnabled(event.target.checked)
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
                      required={heartbeatEnabled}
                      disabled={!heartbeatEnabled}
                      value={heartbeatMinutes}
                      onChange={(event) =>
                        setHeartbeatMinutes(event.target.value)
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
                      checked={subscriptions.includes(channel.name)}
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
                {messages.map((message) => (
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
                    onChange={(event) => setPrompt(event.target.value)}
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

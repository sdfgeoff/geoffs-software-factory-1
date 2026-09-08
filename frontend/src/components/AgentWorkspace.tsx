import { memo, useEffect, useState, type FormEvent } from "react";
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
  canReview?: boolean;
  channelSubscriptions?: string[];
  projectSubscriptions?: number[];
}

type SettingsSaveState = "saved" | "dirty" | "saving" | "error";

const AgentConversation = memo(function AgentConversation({
  agentId,
  delta,
  loading,
  messages,
  messagesAgentId,
}: {
  agentId: number;
  delta: string;
  loading: boolean;
  messages: AgentMessage[];
  messagesAgentId: number | null;
}) {
  return (
    <div className="message-list agent-chat">
      {(loading || messagesAgentId !== agentId) && (
        <p className="empty">Loading messages…</p>
      )}
      {messagesAgentId === agentId && messages.map((message) => (
        <article className={message.role} key={message.id}>
          <strong>{message.role}</strong>
          <MessageContent source={message.body} />
        </article>
      ))}
      {delta && (
        <article className="agent streaming">
          <strong>agent</strong>
          <div className="message-content streaming-text">{delta}</div>
        </article>
      )}
    </div>
  );
});

export const AgentWorkspace = memo(function AgentWorkspace({
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
  const [settingsSaveStates, setSettingsSaveStates] = useState<
    Record<number, SettingsSaveState>
  >({});
  const [settingsErrors, setSettingsErrors] = useState<Record<number, string>>(
    {},
  );
  const [scopes, setScopes] = useState<string[]>([]);
  const [subscriptions, setSubscriptions] = useState<string[]>([]);
  const [projectSubscriptions, setProjectSubscriptions] = useState<number[]>([]);
  const [creating, setCreating] = useState(false);
  const [createStatus, setCreateStatus] = useState("");
  const [sendingAgentId, setSendingAgentId] = useState<number | null>(null);
  const [sendStatuses, setSendStatuses] = useState<Record<number, string>>({});
  const [togglingAgentId, setTogglingAgentId] = useState<number | null>(null);
  const [toggleStatuses, setToggleStatuses] = useState<Record<number, string>>({});
  const [showAgentCreator, setShowAgentCreator] = useState(false);
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
  const currentProjectSubscriptions =
    detailsAgentId === selectedAgentId ? projectSubscriptions : [];
  const canReview =
    settingsDraft?.canReview ?? currentScopes.includes("approve_review");
  const selectedChannels =
    settingsDraft?.channelSubscriptions ?? currentSubscriptions;
  const selectedProjects =
    settingsDraft?.projectSubscriptions ?? currentProjectSubscriptions;
  const settingsSaveState = selected
    ? settingsSaveStates[selected.id] ?? "saved"
    : "saved";
  const settingsReady = detailsAgentId === selectedAgentId;
  const nameLocked =
    selected?.state === "running" || selected?.state === "awaiting_approval";
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
    setProjectSubscriptions([]);
    setDetailsAgentId(null);
    if (selectedAgentId) {
      void Promise.all([
        api.agentProjectWorktrees(selectedAgentId),
        api.agentScopes(selectedAgentId),
        api.agentChannelSubscriptions(selectedAgentId),
        api.agentProjectSubscriptions(selectedAgentId),
      ])
        .then(
          ([
            nextWorktrees,
            nextScopes,
            nextSubscriptions,
            nextProjectSubscriptions,
          ]) => {
            if (!current) return;
            setWorktrees(nextWorktrees);
            setScopes(nextScopes);
            setSubscriptions(nextSubscriptions);
            setProjectSubscriptions(nextProjectSubscriptions);
            setDetailsAgentId(selectedAgentId);
          },
        )
        .catch(onError);
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
    setSettingsSaveStates((current) => ({
      ...current,
      [selected.id]: "dirty",
    }));
    setSettingsErrors((current) => ({ ...current, [selected.id]: "" }));
  }
  async function create(event: FormEvent) {
    event.preventDefault();
    if (creating) return;
    setCreating(true);
    setCreateStatus("Creating…");
    try {
      const agent = await api.createAgent(name);
      setName("");
      setShowAgentCreator(false);
      onSelectAgent(agent.id);
      await onChanged(agent);
      setCreateStatus("Agent created");
    } catch (error) {
      setCreateStatus(
        `Could not create agent: ${error instanceof Error ? error.message : "Unknown error"}`,
      );
    } finally {
      setCreating(false);
    }
  }
  async function send(event: FormEvent) {
    event.preventDefault();
    if (!selected || sendingAgentId !== null) return;
    const agentId = selected.id;
    setSendingAgentId(agentId);
    setSendStatuses((current) => ({ ...current, [agentId]: "Sending…" }));
    try {
      const updated = await api.sendAgentMessage(agentId, prompt);
      setPrompts((current) => ({ ...current, [agentId]: "" }));
      await onChanged(updated);
      setSendStatuses((current) => ({ ...current, [agentId]: "Sent" }));
    } catch (error) {
      setSendStatuses((current) => ({
        ...current,
        [agentId]: `Could not send: ${error instanceof Error ? error.message : "Unknown error"}`,
      }));
    } finally {
      setSendingAgentId((current) => current === agentId ? null : current);
    }
  }
  async function toggle() {
    if (!selected || togglingAgentId !== null) return;
    const agentId = selected.id;
    const action = selected.state === "paused" || selected.state === "error"
      ? "Resuming"
      : "Pausing";
    const actionVerb = action === "Resuming" ? "resume" : "pause";
    setTogglingAgentId(agentId);
    setToggleStatuses((current) => ({ ...current, [agentId]: `${action}…` }));
    try {
      const updated = selected.state === "paused" || selected.state === "error"
        ? await api.resumeAgent(agentId)
        : await api.pauseAgent(agentId);
      await onChanged(updated);
      setToggleStatuses((current) => ({
        ...current,
        [agentId]: action === "Resuming" ? "Resumed" : "Paused",
      }));
    } catch (error) {
      setToggleStatuses((current) => ({
        ...current,
        [agentId]: `Could not ${actionVerb}: ${error instanceof Error ? error.message : "Unknown error"}`,
      }));
    } finally {
      setTogglingAgentId((current) => current === agentId ? null : current);
    }
  }
  async function saveSettings(event: FormEvent) {
    event.preventDefault();
    if (!selected || !settingsDraft || !settingsReady) return;
    const agentId = selected.id;
    setSettingsSaveStates((current) => ({
      ...current,
      [agentId]: "saving",
    }));
    setSettingsErrors((current) => ({ ...current, [agentId]: "" }));
    try {
      let savedName = selected.name;
      if (settingsDraft.name !== selected.name) {
        savedName = (await api.renameAgent(agentId, settingsDraft.name)).name;
      }
      const nextScopes = canReview
        ? [...new Set([...currentScopes, "approve_review"])]
        : currentScopes.filter((scope) => scope !== "approve_review");
      const [nextSavedScopes, nextChannels, nextProjects] = await Promise.all([
        api.setAgentScopes(agentId, nextScopes),
        api.setAgentChannelSubscriptions(agentId, selectedChannels),
        api.setAgentProjectSubscriptions(agentId, selectedProjects),
      ]);
      const updated = await api.setAgentWakeSettings(
        agentId,
        settingsDraft.wakeOnPendingWork,
        settingsDraft.wakeOnPendingReview,
        settingsDraft.heartbeatEnabled
          ? Number(settingsDraft.heartbeatMinutes)
          : null,
      );
      setScopes(nextSavedScopes);
      setSubscriptions(nextChannels);
      setProjectSubscriptions(nextProjects);
      setSettingsDrafts((current) => ({
        ...current,
        [agentId]: {
          ...settingsDraft,
          name: savedName,
          canReview: nextSavedScopes.includes("approve_review"),
          channelSubscriptions: nextChannels,
          projectSubscriptions: nextProjects,
        },
      }));
      await onChanged(updated);
      setSettingsSaveStates((current) => ({
        ...current,
        [agentId]: "saved",
      }));
    } catch (error) {
      setSettingsSaveStates((current) => ({
        ...current,
        [agentId]: "error",
      }));
      setSettingsErrors((current) => ({
        ...current,
        [agentId]: error instanceof Error ? error.message : "Could not save settings",
      }));
    }
  }
  return (
    <section className="agents-layout">
      <aside className="context-sidebar">
        <h2>Agents</h2>
        <label className="mobile-context-picker">
          Agent
          <select
            value={selected?.id ?? ""}
            onChange={(event) => onSelectAgent(Number(event.target.value))}
          >
            {agents.map((agent) => (
              <option key={agent.id} value={agent.id}>{agent.name}</option>
            ))}
          </select>
        </label>
        {agents.map((agent) => (
          <button
            className={
              selected?.id === agent.id
                ? "selected agent-button context-option"
                : "ghost agent-button context-option"
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
        <button
          type="button"
          className="ghost mobile-context-create"
          aria-expanded={showAgentCreator}
          onClick={() => setShowAgentCreator((current) => !current)}
        >
          {showAgentCreator ? "Hide new agent" : "+ New agent"}
        </button>
        <form
          className={`agent-create-form${showAgentCreator ? " mobile-open" : ""}`}
          onSubmit={create}
        >
          <label>
            New agent
            <input
              value={name}
              disabled={creating}
              onChange={(event) => {
                setName(event.target.value);
                setCreateStatus("");
              }}
              placeholder="backend"
              pattern="[A-Za-z0-9_-]+"
              required
            />
          </label>
          <button type="submit" disabled={creating}>
            {creating ? "Creating…" : "Create agent"}
          </button>
          {createStatus && (
            <p
              className={createStatus.startsWith("Could not") ? "inline-error" : undefined}
              role="status"
              aria-live="polite"
            >
              {createStatus}
            </p>
          )}
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
              </div>
              <button
                className="secondary"
                onClick={toggle}
                disabled={togglingAgentId !== null}
              >
                {togglingAgentId === selected.id
                  ? "Working…"
                  : selected.state === "paused" || selected.state === "error"
                  ? "Resume"
                  : "Pause"}
              </button>
            </div>
            {selected.last_error && (
              <p className="inline-error">{selected.last_error}</p>
            )}
            {toggleStatuses[selected.id] && (
              <p
                className={toggleStatuses[selected.id].startsWith("Could not") ? "inline-error" : undefined}
                role="status"
                aria-live="polite"
              >
                {toggleStatuses[selected.id]}
              </p>
            )}
            <section hidden={view !== "settings"}>
              <h3>Agent settings</h3>
              <form className="agent-settings-form" onSubmit={saveSettings}>
                <fieldset
                  className="capabilities"
                  disabled={settingsSaveState === "saving"}
                >
                  <legend>Identity</legend>
                  <label className="settings-text-field">
                    Agent name
                    <input
                      value={settingsDraft?.name ?? ""}
                      onChange={(event) =>
                        updateSettings({ name: event.target.value })
                      }
                      pattern="[A-Za-z0-9_-]+"
                      maxLength={50}
                      disabled={nameLocked}
                      required
                    />
                  </label>
                  {nameLocked && (
                    <small>Pause the agent before changing its name.</small>
                  )}
                </fieldset>
                <fieldset
                  className="capabilities"
                  disabled={!settingsReady || settingsSaveState === "saving"}
                >
                  <legend>Work preferences</legend>
                  <label>
                    <input
                      type="checkbox"
                      checked={canReview}
                      onChange={(event) =>
                        updateSettings({ canReview: event.target.checked })
                      }
                    />
                    <span>
                      <strong>Can review tickets</strong>
                      Review changes and authorize board-owned merges
                    </span>
                  </label>
                  {projects.filter((project) => !project.archived_at).map((project) => (
                    <label key={project.id}>
                      <input
                        type="checkbox"
                        checked={selectedProjects.includes(project.id)}
                        onChange={(event) =>
                          updateSettings({
                            projectSubscriptions: event.target.checked
                              ? [...new Set([...selectedProjects, project.id])]
                              : selectedProjects.filter((id) => id !== project.id),
                          })
                        }
                      />
                      <span>
                        <strong>{project.name}</strong>
                        Prefer work and reviews from this project
                      </span>
                    </label>
                  ))}
                </fieldset>
                <fieldset
                  className="capabilities"
                  disabled={!settingsReady || settingsSaveState === "saving"}
                >
                  <legend>Notifications</legend>
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
                  {channels.map((channel) => (
                    <label key={channel.name}>
                      <input
                        type="checkbox"
                        checked={selectedChannels.includes(channel.name)}
                        onChange={(event) =>
                          updateSettings({
                            channelSubscriptions: event.target.checked
                              ? [...new Set([...selectedChannels, channel.name])]
                              : selectedChannels.filter(
                                  (name) => name !== channel.name,
                                ),
                          })
                        }
                      />
                      <span>
                        <strong>#{channel.name}</strong>
                        Wake for new messages
                      </span>
                    </label>
                  ))}
                </fieldset>
                <details className="advanced-settings">
                  <summary>Advanced</summary>
                  <dl>
                    <div>
                      <dt>Agent home</dt>
                      <dd className="worktree-path">{selected.home_path}</dd>
                    </div>
                    {currentWorktrees.map((worktree) => (
                      <div key={worktree.project_id}>
                        <dt>
                          {projects.find(
                            (project) => project.id === worktree.project_id,
                          )?.name ?? `Project #${worktree.project_id}`} worktree
                        </dt>
                        <dd className="worktree-path">{worktree.path}</dd>
                      </div>
                    ))}
                  </dl>
                </details>
                <div className="settings-save-bar">
                  <p
                    className={settingsSaveState === "error" ? "inline-error" : undefined}
                    role="status"
                    aria-live="polite"
                  >
                    {settingsSaveState === "dirty" && "Unsaved changes"}
                    {settingsSaveState === "saving" && "Saving…"}
                    {settingsSaveState === "saved" && "Saved"}
                    {settingsSaveState === "error" &&
                      `Could not save: ${settingsErrors[selected.id]}`}
                  </p>
                  <button
                    type="submit"
                    disabled={
                      !settingsReady ||
                      settingsSaveState === "saved" ||
                      settingsSaveState === "saving" ||
                      (nameLocked && settingsDraft?.name !== selected.name)
                    }
                  >
                    Save settings
                  </button>
                </div>
              </form>
            </section>
            <section className="agent-chat-panel" hidden={view !== "chat"}>
              <AgentConversation
                agentId={selected.id}
                delta={deltas[selected.id] ?? ""}
                loading={messagesLoading}
                messages={messages}
                messagesAgentId={messagesAgentId}
              />
              <form onSubmit={send}>
                <label>
                  Message
                  <textarea
                    value={prompt}
                    disabled={sendingAgentId !== null}
                    onChange={(event) =>
                      {
                        setPrompts((current) => ({
                          ...current,
                          [selected.id]: event.target.value,
                        }));
                        setSendStatuses((current) => ({
                          ...current,
                          [selected.id]: "",
                        }));
                      }
                    }
                    required
                  />
                </label>
                <button
                  type="submit"
                  disabled={sendingAgentId !== null}
                >
                  {sendingAgentId === selected.id ? "Sending…" : "Send"}
                </button>
                {sendStatuses[selected.id] && (
                  <p
                    className={sendStatuses[selected.id].startsWith("Could not") ? "inline-error" : undefined}
                    role="status"
                    aria-live="polite"
                  >
                    {sendStatuses[selected.id]}
                  </p>
                )}
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
});

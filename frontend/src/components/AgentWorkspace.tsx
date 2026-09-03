import { useEffect, useState, type FormEvent } from "react";
import { api } from "../api";
import type { Agent, AgentMessage } from "../types";
import { MessageContent } from "./MessageContent";
interface Props {
  agents: Agent[];
  selectedAgentId: number | null;
  refresh: number;
  deltas: Record<number, string>;
  onSelectAgent: (id: number) => void;
  onChanged: () => Promise<void>;
  onError: (error: unknown) => void;
}

export function AgentWorkspace({
  agents,
  selectedAgentId,
  refresh,
  deltas,
  onSelectAgent,
  onChanged,
  onError,
}: Props) {
  const [messages, setMessages] = useState<AgentMessage[]>([]);
  const [name, setName] = useState("");
  const [renamedName, setRenamedName] = useState("");
  const [prompt, setPrompt] = useState("");
  const [scopes, setScopes] = useState<string[]>([]);
  const selected =
    agents.find((agent) => agent.id === selectedAgentId) ?? null;
  useEffect(() => {
    if (!selectedAgentId && agents[0]) onSelectAgent(agents[0].id);
  }, [agents, selectedAgentId, onSelectAgent]);
  useEffect(() => setRenamedName(selected?.name ?? ""), [selected?.name]);
  useEffect(() => {
    if (selectedAgentId)
      void api.agentMessages(selectedAgentId).then(setMessages).catch(onError);
  }, [selectedAgentId, refresh, onError]);
  useEffect(() => {
    if (selectedAgentId)
      void api.agentScopes(selectedAgentId).then(setScopes).catch(onError);
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
                <h2>{selected.name}</h2>
                <p title={selected.worktree_path}>{selected.worktree_path}</p>
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

import { useEffect, useState, type FormEvent } from "react";
import { api } from "../api";
import type { Agent, AgentMessage } from "../types";
interface Props {
  agents: Agent[];
  refresh: number;
  deltas: Record<number, string>;
  onChanged: () => Promise<void>;
  onError: (error: unknown) => void;
}

export function AgentWorkspace({
  agents,
  refresh,
  deltas,
  onChanged,
  onError,
}: Props) {
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [messages, setMessages] = useState<AgentMessage[]>([]);
  const [name, setName] = useState("");
  const [prompt, setPrompt] = useState("");
  const selected = agents.find((agent) => agent.id === selectedId) ?? null;
  useEffect(() => {
    if (!selectedId && agents[0]) setSelectedId(agents[0].id);
  }, [agents, selectedId]);
  useEffect(() => {
    if (selectedId)
      void api.agentMessages(selectedId).then(setMessages).catch(onError);
  }, [selectedId, refresh, onError]);
  async function create(event: FormEvent) {
    event.preventDefault();
    try {
      const agent = await api.createAgent(name);
      setName("");
      setSelectedId(agent.id);
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
  async function toggle() {
    if (!selected) return;
    try {
      selected.state === "paused"
        ? await api.resumeAgent(selected.id)
        : await api.pauseAgent(selected.id);
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
            onClick={() => setSelectedId(agent.id)}
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
                {selected.state === "paused" ? "Resume" : "Pause"}
              </button>
            </div>
            {selected.last_error && (
              <p className="inline-error">{selected.last_error}</p>
            )}
            <div className="message-list agent-chat">
              {messages.map((message) => (
                <article className={message.role} key={message.id}>
                  <strong>{message.role}</strong>
                  <p>{message.body}</p>
                </article>
              ))}
              {deltas[selected.id] && (
                <article className="agent streaming">
                  <strong>agent</strong>
                  <p>{deltas[selected.id]}</p>
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
                  disabled={
                    selected.state !== "idle" && selected.state !== "error"
                  }
                />
              </label>
              <button
                type="submit"
                disabled={
                  selected.state !== "idle" && selected.state !== "error"
                }
              >
                Send
              </button>
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

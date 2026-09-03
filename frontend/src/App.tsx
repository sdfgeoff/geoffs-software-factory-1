import { useCallback, useEffect, useState } from "react";
import { api } from "./api";
import { AgentWorkspace } from "./components/AgentWorkspace";
import { KanbanBoard } from "./components/KanbanBoard";
import { MessageBoard } from "./components/MessageBoard";
import { ProjectPicker } from "./components/ProjectPicker";
import type { Agent, AgentEvent, Channel, Project, Ticket } from "./types";

type View = "board" | "messages" | "agents";

export function App() {
  const [project, setProject] = useState<Project | null>(null);
  const [tickets, setTickets] = useState<Ticket[]>([]);
  const [channels, setChannels] = useState<Channel[]>([]);
  const [agents, setAgents] = useState<Agent[]>([]);
  const [view, setView] = useState<View>("board");
  const [selectedAgentId, setSelectedAgentId] = useState<number | null>(null);
  const [refresh, setRefresh] = useState(0);
  const [deltas, setDeltas] = useState<Record<number, string>>({});
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    try {
      const [nextProject, nextTickets, nextChannels, nextAgents] =
        await Promise.all([
          api.project(),
          api.tickets(),
          api.channels(),
          api.agents(),
        ]);
      setProject(nextProject);
      setTickets(nextTickets);
      setChannels(nextChannels);
      setAgents(nextAgents);
      setRefresh((value) => value + 1);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }, []);

  useEffect(() => {
    void reload();
    const source = new EventSource("/api/events");
    source.onmessage = (message) => {
      const event = JSON.parse(message.data) as AgentEvent;
      if (
        event.kind === "agent_message_delta" &&
        event.agent_id &&
        event.data?.delta
      ) {
        setDeltas((current) => ({
          ...current,
          [event.agent_id!]:
            (current[event.agent_id!] ?? "") + event.data!.delta,
        }));
      } else {
        if (event.kind === "message" && event.agent_id)
          setDeltas((current) => ({ ...current, [event.agent_id!]: "" }));
        void reload();
      }
    };
    return () => source.close();
  }, [reload]);

  const report = useCallback(
    (cause: unknown) =>
      setError(cause instanceof Error ? cause.message : String(cause)),
    [],
  );
  return (
    <div className="app-shell">
      <header>
        <div>
          <p className="eyebrow">LOCAL WORKSPACE</p>
          <h1>Multi-agent board</h1>
        </div>
        <ProjectPicker project={project} onChanged={reload} onError={report} />
      </header>
      <nav aria-label="Main views">
        {(["board", "messages", "agents"] as View[]).map((item) => (
          <button
            key={item}
            className={view === item ? "active" : "ghost"}
            onClick={() => setView(item)}
          >
            {item}
          </button>
        ))}
      </nav>
      {error && (
        <div className="error" role="alert">
          <span>{error}</span>
          <button aria-label="Dismiss error" onClick={() => setError(null)}>
            ×
          </button>
        </div>
      )}
      {view === "board" && (
        <KanbanBoard
          tickets={tickets}
          agents={agents}
          onChanged={reload}
          onError={report}
        />
      )}
      {view === "messages" && (
        <MessageBoard
          agents={agents}
          channels={channels}
          refresh={refresh}
          onSelectAgent={(id) => {
            setSelectedAgentId(id);
            setView("agents");
          }}
          onChanged={reload}
          onError={report}
        />
      )}
      {view === "agents" && (
        <AgentWorkspace
          agents={agents}
          channels={channels}
          selectedAgentId={selectedAgentId}
          refresh={refresh}
          deltas={deltas}
          onSelectAgent={setSelectedAgentId}
          onChanged={reload}
          onError={report}
        />
      )}
    </div>
  );
}

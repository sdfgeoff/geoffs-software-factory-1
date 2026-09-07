import { useCallback, useEffect, useState } from "react";
import { api } from "./api";
import {
  AgentWorkspace,
  type AgentView,
} from "./components/AgentWorkspace";
import { BoardSettings } from "./components/BoardSettings";
import { KanbanBoard } from "./components/KanbanBoard";
import { MessageBoard } from "./components/MessageBoard";
import { ProjectPicker } from "./components/ProjectPicker";
import type {
  Agent,
  AgentEvent,
  BoardSettings as Settings,
  Channel,
  Project,
  Ticket,
} from "./types";

type Route =
  | { view: "board"; projectId: number | null; ticketId: number | null }
  | { view: "messages"; channel: string }
  | { view: "agents"; agentId: number | null; agentView: AgentView }
  | { view: "settings" }
  | { view: "not_found" };

function positiveId(value: string | null) {
  const id = Number(value);
  return value && Number.isSafeInteger(id) && id > 0 ? id : null;
}

function parseRoute(hash: string): Route {
  const value = hash.replace(/^#\/?/, "");
  if (!value || value === "board") {
    return { view: "board", projectId: null, ticketId: null };
  }
  if (value.startsWith("board?")) {
    const params = new URLSearchParams(value.slice(6));
    const projectId = positiveId(params.get("project"));
    const ticketId = positiveId(params.get("ticket"));
    if (
      (params.has("project") && !projectId) ||
      (params.has("ticket") && !ticketId)
    ) {
      return { view: "not_found" };
    }
    return { view: "board", projectId, ticketId };
  }
  const messageMatch = /^messages\/([^/]+)$/.exec(value);
  if (messageMatch) {
    try {
      return {
        view: "messages",
        channel: decodeURIComponent(messageMatch[1]),
      };
    } catch {
      return { view: "not_found" };
    }
  }
  const agentMatch = /^agents(?:\/(\d+)(\/settings)?)?$/.exec(value);
  if (agentMatch) {
    return {
      view: "agents",
      agentId: positiveId(agentMatch[1] ?? null),
      agentView: agentMatch[2] ? "settings" : "chat",
    };
  }
  if (value === "settings") return { view: "settings" };
  return { view: "not_found" };
}

function routeHash(route: Route) {
  if (route.view === "board") {
    const params = new URLSearchParams();
    if (route.projectId) params.set("project", String(route.projectId));
    if (route.ticketId) params.set("ticket", String(route.ticketId));
    const query = params.toString();
    return `#board${query ? `?${query}` : ""}`;
  }
  if (route.view === "messages") {
    return `#messages/${encodeURIComponent(route.channel)}`;
  }
  if (route.view === "agents") {
    return route.agentId
      ? `#agents/${route.agentId}${route.agentView === "settings" ? "/settings" : ""}`
      : "#agents";
  }
  return route.view === "settings" ? "#settings" : "#not-found";
}

export function App() {
  const [route, setRoute] = useState(() => parseRoute(window.location.hash));
  const [project, setProject] = useState<Project | null>(null);
  const [projects, setProjects] = useState<Project[]>([]);
  const [tickets, setTickets] = useState<Ticket[]>([]);
  const [channels, setChannels] = useState<Channel[]>([]);
  const [agents, setAgents] = useState<Agent[]>([]);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [refresh, setRefresh] = useState(0);
  const [deltas, setDeltas] = useState<Record<number, string>>({});
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    try {
      const [nextProject, nextProjects, nextTickets, nextChannels, nextAgents, nextSettings] =
        await Promise.all([
          api.project(),
          api.projects(),
          api.tickets(),
          api.channels(),
          api.agents(),
          api.settings(),
        ]);
      setProject(nextProject);
      setProjects(nextProjects);
      setTickets(nextTickets);
      setChannels(nextChannels);
      setAgents(nextAgents);
      setSettings(nextSettings);
      setRefresh((value) => value + 1);
      setLoaded(true);
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

  useEffect(() => {
    const followRoute = () => setRoute(parseRoute(window.location.hash));
    window.addEventListener("popstate", followRoute);
    window.addEventListener("hashchange", followRoute);
    return () => {
      window.removeEventListener("popstate", followRoute);
      window.removeEventListener("hashchange", followRoute);
    };
  }, []);

  const navigate = useCallback((nextRoute: Route) => {
    const hash = routeHash(nextRoute);
    if (window.location.hash !== hash) {
      window.history.pushState(null, "", hash);
    }
    setRoute(nextRoute);
  }, []);
  const report = useCallback(
    (cause: unknown) =>
      setError(cause instanceof Error ? cause.message : String(cause)),
    [],
  );
  const activeView = route.view === "not_found" ? null : route.view;
  const selectedAgentId = route.view === "agents" ? route.agentId : null;
  const agentView = route.view === "agents" ? route.agentView : "chat";
  const selectedTicket =
    route.view === "board" && route.ticketId
      ? tickets.find((ticket) => ticket.id === route.ticketId)
      : null;
  const missingEntity = loaded && (
    route.view === "not_found"
      ? "Page not found"
      : route.view === "board" &&
          route.projectId &&
          !projects.some((item) => item.id === route.projectId)
        ? "Project not found"
        : route.view === "board" && route.ticketId && !selectedTicket
          ? "Ticket not found"
          : route.view === "board" &&
              route.projectId &&
              selectedTicket &&
              selectedTicket.project_id !== route.projectId
            ? "Ticket is not in the selected project"
            : route.view === "messages" &&
                !channels.some((item) => item.name === route.channel)
              ? "Channel not found"
              : route.view === "agents" &&
                  route.agentId &&
                  !agents.some((item) => item.id === route.agentId)
                ? "Agent not found"
                : null
  );

  function navigationRoute(view: "board" | "messages" | "agents" | "settings"): Route {
    if (view === "board") return { view, projectId: null, ticketId: null };
    if (view === "messages") {
      return {
        view,
        channel:
          route.view === "messages"
            ? route.channel
            : channels[0]?.name ?? "general",
      };
    }
    if (view === "agents") {
      return {
        view,
        agentId: selectedAgentId ?? agents[0]?.id ?? null,
        agentView: "chat",
      };
    }
    return { view };
  }

  return (
    <div className="app-shell">
      <header>
        <div>
          <p className="eyebrow">LOCAL WORKSPACE</p>
          <h1>Multi-agent board</h1>
        </div>
        <ProjectPicker
          project={project}
          projects={projects}
          onChanged={reload}
          onError={report}
        />
      </header>
      <nav aria-label="Main views">
        {(["board", "messages", "agents", "settings"] as const).map((item) => {
          const target = navigationRoute(item);
          return (
            <a
              key={item}
              className={activeView === item ? "active" : "ghost"}
              href={routeHash(target)}
              aria-current={activeView === item ? "page" : undefined}
              onClick={(event) => {
                event.preventDefault();
                navigate(target);
              }}
            >
              {item}
            </a>
          );
        })}
      </nav>
      {error && (
        <div className="error" role="alert">
          <span>{error}</span>
          <button aria-label="Dismiss error" onClick={() => setError(null)}>
            ×
          </button>
        </div>
      )}
      {!loaded && <p className="empty">Loading…</p>}
      {missingEntity && (
        <section className="route-fallback">
          <h2>{missingEntity}</h2>
          <a
            href="#board"
            onClick={(event) => {
              event.preventDefault();
              navigate({ view: "board", projectId: null, ticketId: null });
            }}
          >
            Return to board
          </a>
        </section>
      )}
      {loaded && !missingEntity && route.view === "board" && (
        <KanbanBoard
          tickets={tickets}
          agents={agents}
          projects={projects}
          currentProjectId={project?.id ?? null}
          filterProjectId={route.projectId}
          selectedTicketId={route.ticketId}
          onFilterProject={(projectId) =>
            navigate({ ...route, projectId, ticketId: null })
          }
          onSelectTicket={(ticketId) => navigate({ ...route, ticketId })}
          onChanged={reload}
          onError={report}
        />
      )}
      {loaded && !missingEntity && route.view === "messages" && (
        <MessageBoard
          agents={agents}
          channels={channels}
          selectedChannelName={route.channel}
          refresh={refresh}
          onSelectChannel={(channel) => navigate({ view: "messages", channel })}
          onSelectAgent={(agentId) =>
            navigate({ view: "agents", agentId, agentView: "chat" })
          }
          onChanged={reload}
          onError={report}
        />
      )}
      {loaded && !missingEntity && route.view === "agents" && (
        <AgentWorkspace
          agents={agents}
          channels={channels}
          tickets={tickets}
          projects={projects}
          selectedAgentId={selectedAgentId}
          view={agentView}
          refresh={refresh}
          deltas={deltas}
          onSelectAgent={(agentId) =>
            navigate({ view: "agents", agentId, agentView: "chat" })
          }
          onViewChange={(nextView) => {
            if (selectedAgentId) {
              navigate({
                view: "agents",
                agentId: selectedAgentId,
                agentView: nextView,
              });
            }
          }}
          onChanged={reload}
          onError={report}
        />
      )}
      {loaded && !missingEntity && route.view === "settings" && settings && (
        <BoardSettings
          settings={settings}
          channels={channels}
          onChanged={reload}
          onError={report}
        />
      )}
    </div>
  );
}

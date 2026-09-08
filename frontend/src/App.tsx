import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
  AgentMessage,
  AgentEvent,
  BoardMessage,
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
  const [agentDetailRefresh, setAgentDetailRefresh] = useState(0);
  const [boardMessages, setBoardMessages] = useState<Record<number, BoardMessage>>({});
  const [agentMessages, setAgentMessages] = useState<Record<number, AgentMessage>>({});
  const [messageCatchUp, setMessageCatchUp] = useState(1);
  const [deltas, setDeltas] = useState<Record<number, string>>({});
  const [error, setError] = useState<string | null>(null);
  const reloadTimer = useRef<number | null>(null);
  const deltaTimer = useRef<number | null>(null);
  const pendingDelta = useRef<{ agentId: number; text: string } | null>(null);
  const activeAgentId =
    route.view === "agents" && route.agentView === "chat"
      ? route.agentId
      : null;
  const activeAgentIdRef = useRef(activeAgentId);
  activeAgentIdRef.current = activeAgentId;
  const activeChannel = route.view === "messages" ? route.channel : null;
  const activeChannelRef = useRef(activeChannel);
  activeChannelRef.current = activeChannel;
  const [visitedViews, setVisitedViews] = useState(() => new Set([route.view]));
  const [lastBoardRoute, setLastBoardRoute] = useState(() =>
    route.view === "board"
      ? route
      : { view: "board", projectId: null, ticketId: null } as const,
  );
  const [lastMessageChannel, setLastMessageChannel] = useState(() =>
    route.view === "messages" ? route.channel : "general",
  );
  const [lastAgentRoute, setLastAgentRoute] = useState(() =>
    route.view === "agents"
      ? route
      : { view: "agents", agentId: null, agentView: "chat" } as const,
  );

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
      setLoaded(true);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }, []);

  const reloadProjects = useCallback(async () => {
    try {
      const [nextProject, nextProjects] = await Promise.all([
        api.project(),
        api.projects(),
      ]);
      setProject(nextProject);
      setProjects(nextProjects);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }, []);

  const mergeTickets = useCallback((updates: Ticket | Ticket[]) => {
    const next = Array.isArray(updates) ? updates : [updates];
    setTickets((current) => {
      const byId = new Map(current.map((ticket) => [ticket.id, ticket]));
      next.forEach((ticket) => byId.set(ticket.id, ticket));
      return [...byId.values()];
    });
  }, []);
  const mergeAgent = useCallback((updated: Agent) => {
    setAgents((current) => {
      const index = current.findIndex((agent) => agent.id === updated.id);
      if (index < 0) return [...current, updated];
      return current.map((agent) => agent.id === updated.id ? updated : agent);
    });
  }, []);
  const mergeChannel = useCallback((updated?: Channel) => {
    if (!updated) return;
    setChannels((current) =>
      current.some((channel) => channel.name === updated.name)
        ? current.map((channel) =>
            channel.name === updated.name ? updated : channel,
          )
        : [...current, updated],
    );
  }, []);
  const scheduleReload = useCallback(() => {
    if (reloadTimer.current !== null) return;
    reloadTimer.current = window.setTimeout(() => {
      reloadTimer.current = null;
      void reload();
    }, 50);
  }, [reload]);

  useEffect(() => {
    void reload();
    const source = new EventSource("/api/events");
    let opened = false;
    source.onopen = () => {
      if (opened) {
        scheduleReload();
        setMessageCatchUp((value) => value + 1);
      }
      opened = true;
    };
    source.onmessage = (message) => {
      const event = JSON.parse(message.data) as AgentEvent;
      switch (event.kind) {
        case "agent_message_delta":
          if (event.agent_id !== activeAgentIdRef.current) break;
          if (pendingDelta.current?.agentId === event.agent_id) {
            pendingDelta.current.text += event.data.delta;
          } else {
            pendingDelta.current = {
              agentId: event.agent_id,
              text: event.data.delta,
            };
          }
          if (deltaTimer.current === null) {
            deltaTimer.current = window.setTimeout(() => {
              deltaTimer.current = null;
              const pending = pendingDelta.current;
              pendingDelta.current = null;
              if (!pending || pending.agentId !== activeAgentIdRef.current) return;
              setDeltas((current) => ({
                ...current,
                [pending.agentId]:
                  (current[pending.agentId] ?? "") + pending.text,
              }));
            }, 16);
          }
          break;
        case "agent_updated":
        case "agent_created":
          mergeAgent(event.data);
          break;
        case "ticket_updated":
          mergeTickets(event.data);
          break;
        case "settings_updated":
          setSettings(event.data);
          break;
        case "board_message":
          if (event.data.channel_name !== activeChannelRef.current) break;
          setBoardMessages((current) => ({
            ...current,
            [event.data.id]: event.data,
          }));
          break;
        case "message":
          if (event.agent_id !== activeAgentIdRef.current) break;
          if (pendingDelta.current?.agentId === event.agent_id) {
            pendingDelta.current = null;
          }
          setDeltas((current) => ({ ...current, [event.agent_id]: "" }));
          setAgentMessages((current) => ({
            ...current,
            [event.data.id]: event.data,
          }));
          break;
        case "channel_subscriptions":
        case "project_subscriptions":
          setAgentDetailRefresh((value) => value + 1);
          break;
        case "refresh":
          scheduleReload();
          setMessageCatchUp((value) => value + 1);
          break;
        case "codex_event":
        case "approval_declined":
          break;
        default: {
          const exhaustive: never = event;
          void exhaustive;
          scheduleReload();
        }
      }
    };
    return () => {
      source.close();
      if (reloadTimer.current !== null) window.clearTimeout(reloadTimer.current);
      if (deltaTimer.current !== null) window.clearTimeout(deltaTimer.current);
    };
  }, [mergeAgent, mergeTickets, reload, scheduleReload]);

  useEffect(() => {
    pendingDelta.current = null;
    setDeltas((current) => Object.keys(current).length === 0 ? current : {});
  }, [activeAgentId]);

  useEffect(() => {
    const followRoute = () => setRoute(parseRoute(window.location.hash));
    window.addEventListener("popstate", followRoute);
    window.addEventListener("hashchange", followRoute);
    return () => {
      window.removeEventListener("popstate", followRoute);
      window.removeEventListener("hashchange", followRoute);
    };
  }, []);

  useEffect(() => {
    if (route.view === "not_found") return;
    setVisitedViews((current) => new Set(current).add(route.view));
    if (route.view === "board") setLastBoardRoute(route);
    if (route.view === "messages") setLastMessageChannel(route.channel);
    if (route.view === "agents") setLastAgentRoute(route);
  }, [route]);

  const navigate = useCallback((nextRoute: Route) => {
    const hash = routeHash(nextRoute);
    if (window.location.hash !== hash) {
      window.history.pushState(null, "", hash);
    }
    setRoute(nextRoute);
  }, []);
  const selectMessageChannel = useCallback(
    (channel: string) => navigate({ view: "messages", channel }),
    [navigate],
  );
  const selectAgent = useCallback(
    (agentId: number) => navigate({ view: "agents", agentId, agentView: "chat" }),
    [navigate],
  );
  const report = useCallback(
    (cause: unknown) =>
      setError(cause instanceof Error ? cause.message : String(cause)),
    [],
  );
  const activeView = route.view === "not_found" ? null : route.view;
  const selectedAgentId =
    route.view === "agents" ? route.agentId : lastAgentRoute.agentId;
  const agentView =
    route.view === "agents" ? route.agentView : lastAgentRoute.agentView;
  const selectedTicket =
    route.view === "board" && route.ticketId
      ? tickets.find((ticket) => ticket.id === route.ticketId)
      : null;
  const changeAgentView = useCallback(
    (nextView: AgentView) => {
      if (selectedAgentId) {
        navigate({ view: "agents", agentId: selectedAgentId, agentView: nextView });
      }
    },
    [navigate, selectedAgentId],
  );
  const liveBoardMessages = useMemo(
    () => Object.values(boardMessages),
    [boardMessages],
  );
  const liveAgentMessages = useMemo(
    () => Object.values(agentMessages),
    [agentMessages],
  );
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
    if (view === "board") return lastBoardRoute;
    if (view === "messages") {
      return {
        view,
        channel: route.view === "messages" ? route.channel : lastMessageChannel,
      };
    }
    if (view === "agents") {
      return {
        view,
        agentId: selectedAgentId ?? agents[0]?.id ?? null,
        agentView: route.view === "agents" ? route.agentView : lastAgentRoute.agentView,
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
          onChanged={reloadProjects}
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
      {loaded && visitedViews.has("board") && (
        <div hidden={route.view !== "board" || Boolean(missingEntity)}>
        <KanbanBoard
          tickets={tickets}
          agents={agents}
          projects={projects}
          currentProjectId={project?.id ?? null}
          filterProjectId={route.view === "board" ? route.projectId : lastBoardRoute.projectId}
          selectedTicketId={route.view === "board" ? route.ticketId : lastBoardRoute.ticketId}
          onFilterProject={(projectId) =>
            navigate({ view: "board", projectId, ticketId: null })
          }
          onSelectTicket={(ticketId) =>
            navigate({
              view: "board",
              projectId: route.view === "board" ? route.projectId : lastBoardRoute.projectId,
              ticketId,
            })
          }
          onChanged={mergeTickets}
          onError={report}
        />
        </div>
      )}
      {loaded && visitedViews.has("messages") && (
        <div hidden={route.view !== "messages" || Boolean(missingEntity)}>
        <MessageBoard
          agents={agents}
          channels={channels}
          selectedChannelName={route.view === "messages" ? route.channel : lastMessageChannel}
          refresh={0}
          catchUp={route.view === "messages" ? messageCatchUp : -1}
          liveMessages={liveBoardMessages}
          onSelectChannel={selectMessageChannel}
          onSelectAgent={selectAgent}
          onChanged={mergeChannel}
          onError={report}
        />
        </div>
      )}
      {loaded && visitedViews.has("agents") && (
        <div hidden={route.view !== "agents" || Boolean(missingEntity)}>
        <AgentWorkspace
          agents={agents}
          channels={channels}
          tickets={tickets}
          projects={projects}
          selectedAgentId={selectedAgentId}
          view={agentView}
          refresh={0}
          catchUp={activeAgentId === null ? -1 : messageCatchUp}
          detailRefresh={route.view === "agents" ? agentDetailRefresh : -1}
          deltas={deltas}
          liveMessages={liveAgentMessages}
          onSelectAgent={selectAgent}
          onViewChange={changeAgentView}
          onChanged={mergeAgent}
          onError={report}
        />
        </div>
      )}
      {loaded && visitedViews.has("settings") && settings && (
        <div hidden={route.view !== "settings" || Boolean(missingEntity)}>
        <BoardSettings
          settings={settings}
          channels={channels}
          onChanged={setSettings}
          onError={report}
        />
        </div>
      )}
    </div>
  );
}

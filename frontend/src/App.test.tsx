import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "./api";
import { App } from "./App";
import type { Agent, Project, Ticket } from "./types";

vi.mock("./api", () => ({
  api: {
    agentChannelSubscriptions: vi.fn(),
    agentMessages: vi.fn(),
    agentProjectSubscriptions: vi.fn(),
    agentProjectWorktrees: vi.fn(),
    agentScopes: vi.fn(),
    agents: vi.fn(),
    channels: vi.fn(),
    project: vi.fn(),
    projects: vi.fn(),
    settings: vi.fn(),
    tickets: vi.fn(),
    ticket: vi.fn(),
    messages: vi.fn(),
  },
}));

const agent: Agent = {
  id: 1,
  name: "worker",
  state: "idle",
  thread_id: null,
  home_path: "/repo/worker-home",
  active_project_id: null,
  active_turn_id: null,
  last_error: null,
  wake_on_pending_work: false,
  wake_on_pending_review: false,
  idle_heartbeat_minutes: null,
  idle_since: 0,
  created_at: 0,
  updated_at: 0,
};

const project: Project = {
  id: 2,
  name: "Router",
  path: "/repo/router",
  default_branch: "main",
  archived_at: null,
  updated_at: 0,
};

const ticket: Ticket = {
  id: 7,
  project_id: project.id,
  title: "Durable routes",
  description: "",
  state: "in_progress",
  assignee_id: null,
  created_by_agent_id: null,
  reserved_for_agent_id: null,
  blocker_ids: [],
  unresolved_blockers: [],
  implementation_branch: null,
  review_commit: null,
  review_target_branch: null,
  review_project_id: null,
  review_project_path: null,
  reviewer_id: null,
  review_feedback: null,
  revision: 1,
  created_at: 0,
  updated_at: 0,
};

class MockEventSource {
  onopen: (() => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  constructor() {
    latestEventSource = this;
  }
  close() {}
}
let latestEventSource: MockEventSource;

describe("App agent routes", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.stubGlobal("EventSource", MockEventSource);
    window.history.replaceState(null, "", "#agents/1/settings");
    vi.mocked(api.project).mockResolvedValue(null);
    vi.mocked(api.projects).mockResolvedValue([]);
    vi.mocked(api.settings).mockResolvedValue({
      merge_notification_channel: null,
    });
    vi.mocked(api.tickets).mockResolvedValue({ tickets: [], has_more: false });
    vi.mocked(api.ticket).mockResolvedValue(ticket);
    vi.mocked(api.channels).mockResolvedValue([]);
    vi.mocked(api.agents).mockResolvedValue([agent]);
    vi.mocked(api.agentMessages).mockResolvedValue({ messages: [], has_more: false });
    vi.mocked(api.agentProjectSubscriptions).mockResolvedValue([]);
    vi.mocked(api.agentProjectWorktrees).mockResolvedValue([]);
    vi.mocked(api.agentScopes).mockResolvedValue([]);
    vi.mocked(api.agentChannelSubscriptions).mockResolvedValue([]);
    vi.mocked(api.messages).mockResolvedValue({ messages: [], has_more: false });
  });

  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    window.history.replaceState(null, "", window.location.pathname);
  });

  it("opens a linkable settings route and returns to chat", async () => {
    render(<App />);

    expect(
      await screen.findByRole("heading", { name: "Agent settings" }),
    ).toBeTruthy();
    expect(window.location.hash).toBe("#agents/1/settings");
    const chat = screen.getByRole("link", { name: "Back to chat" });
    expect(chat.getAttribute("href")).toBe("#agents/1");

    fireEvent.click(chat);

    expect(window.location.hash).toBe("#agents/1");
    expect(screen.getByRole("textbox", { name: "Message" })).toBeTruthy();
  });

  it("opens board settings from the main navigation", async () => {
    render(<App />);
    await screen.findByRole("heading", { name: "Agent settings" });

    fireEvent.click(screen.getByRole("link", { name: "settings" }));

    expect(
      screen.getByRole("heading", { name: "Board settings" }),
    ).toBeTruthy();
    expect(window.location.hash).toBe("#settings");
    expect(
      screen
        .getByRole("link", { name: "settings" })
        .getAttribute("aria-current"),
    ).toBe("page");
  });

  it("restores a directly linked channel across reloads", async () => {
    window.history.replaceState(null, "", "#messages/releases");
    vi.mocked(api.channels).mockResolvedValue([
      { name: "general", description: "", most_recent_message_at: 0 },
      {
        name: "releases",
        description: "Shipped work",
        most_recent_message_at: 0,
      },
    ]);

    const first = render(<App />);
    expect(
      await screen.findByRole("heading", { name: "#releases" }),
    ).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "#general" }));
    expect(window.location.hash).toBe("#messages/general");
    fireEvent.click(screen.getByRole("button", { name: "#releases" }));
    expect(window.location.hash).toBe("#messages/releases");
    first.unmount();
    render(<App />);

    expect(
      await screen.findByRole("heading", { name: "#releases" }),
    ).toBeTruthy();
    expect(api.messages).toHaveBeenLastCalledWith("releases", { limit: 50 });
  });

  it("restores a project filter and open ticket from a direct link", async () => {
    window.history.replaceState(null, "", "#board?project=2&ticket=7");
    vi.mocked(api.projects).mockResolvedValue([project]);
    vi.mocked(api.tickets).mockImplementation(async ({ state } = {}) => ({
      tickets: state === ticket.state ? [{ ...ticket, has_review_feedback: false }] : [],
      has_more: false,
    }));

    render(<App />);

    expect(await screen.findByDisplayValue("Durable routes")).toBeTruthy();
    expect(
      (screen.getByLabelText("Filter projects") as HTMLSelectElement).value,
    ).toBe("2");
    expect(
      screen.getByRole("heading", { name: "Durable routes" }),
    ).toBeTruthy();
    expect(api.ticket).toHaveBeenCalledWith(ticket.id);
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(window.location.hash).toBe("#board?project=2");
    fireEvent.click(screen.getByText("Durable routes").closest("button")!);
    expect(window.location.hash).toBe("#board?project=2&ticket=7");
  });

  it("retries transient ticket detail failures without marking the route missing", async () => {
    window.history.replaceState(null, "", "#board?project=2&ticket=7");
    vi.mocked(api.projects).mockResolvedValue([project]);
    vi.mocked(api.ticket)
      .mockRejectedValueOnce(new Error("Temporary detail failure"))
      .mockResolvedValue(ticket);

    render(<App />);

    expect(await screen.findByDisplayValue(ticket.title)).toBeTruthy();
    expect(api.ticket).toHaveBeenCalledTimes(2);
    expect(screen.queryByRole("heading", { name: "Ticket not found" })).toBeNull();
    expect(screen.getByRole("alert").textContent).toContain("Temporary detail failure");
  });

  it("marks only a 404 ticket detail response as missing", async () => {
    window.history.replaceState(null, "", "#board?project=2&ticket=7");
    vi.mocked(api.projects).mockResolvedValue([project]);
    vi.mocked(api.ticket).mockRejectedValue(
      Object.assign(new Error("ticket not found"), { status: 404 }),
    );

    render(<App />);

    expect(await screen.findByRole("heading", { name: "Ticket not found" })).toBeTruthy();
    expect(api.ticket).toHaveBeenCalledTimes(1);
  });

  it("revalidates an open ticket detail during repair reloads", async () => {
    window.history.replaceState(null, "", "#board?project=2&ticket=7");
    const repaired = { ...ticket, title: "Changed while disconnected", revision: 2 };
    vi.mocked(api.projects).mockResolvedValue([project]);
    vi.mocked(api.ticket)
      .mockResolvedValueOnce(ticket)
      .mockResolvedValue(repaired);

    render(<App />);
    expect(await screen.findByDisplayValue(ticket.title)).toBeTruthy();

    act(() => {
      latestEventSource.onmessage?.({
        data: JSON.stringify({ kind: "refresh" }),
      } as MessageEvent);
    });

    expect(await screen.findByDisplayValue(repaired.title)).toBeTruthy();
    expect(api.ticket).toHaveBeenCalledTimes(2);
    expect(api.tickets).toHaveBeenCalledTimes(6);
  });

  it("loads terminal history on demand and details only when opened", async () => {
    window.history.replaceState(null, "", "#board");
    const completed = { ...ticket, id: 80, title: "Historical work", state: "done" as const };
    vi.mocked(api.projects).mockResolvedValue([project]);
    vi.mocked(api.tickets).mockImplementation(async ({ state } = {}) => ({
      tickets: state === "done" ? [{ ...completed, has_review_feedback: false }] : [],
      has_more: false,
    }));
    vi.mocked(api.ticket).mockResolvedValue(completed);

    render(<App />);
    await screen.findByLabelText("Filter projects");
    expect(screen.queryByText(completed.title)).toBeNull();
    expect(api.ticket).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Load older tickets" }));
    expect(await screen.findByText(completed.title)).toBeTruthy();
    expect(api.ticket).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText(completed.title).closest("button")!);
    expect(await screen.findByDisplayValue(completed.title)).toBeTruthy();
    expect(api.ticket).toHaveBeenCalledWith(completed.id);
  });

  it("uses browser Back and Forward to traverse views", async () => {
    window.history.replaceState(null, "", "#board");
    vi.mocked(api.channels).mockResolvedValue([
      { name: "general", description: "", most_recent_message_at: 0 },
    ]);
    render(<App />);
    await screen.findByLabelText("Filter projects");

    fireEvent.click(screen.getByRole("link", { name: "messages" }));
    await screen.findByRole("heading", { name: "#general" });
    fireEvent.click(screen.getByRole("link", { name: "settings" }));
    expect(window.location.hash).toBe("#settings");

    act(() => window.history.back());
    await waitFor(() => expect(window.location.hash).toBe("#messages/general"));
    expect(screen.getByRole("heading", { name: "#general" })).toBeTruthy();

    act(() => window.history.forward());
    await waitFor(() => expect(window.location.hash).toBe("#settings"));
    expect(screen.getByRole("heading", { name: "Board settings" })).toBeTruthy();
  });

  it("refreshes history when chat becomes visible after a completion", async () => {
    vi.mocked(api.agentMessages).mockResolvedValue({
      messages: [{
        id: 1,
        agent_id: agent.id,
        role: "agent",
        body: "Current history",
        created_at: 1,
      }],
      has_more: false,
    });
    render(<App />);
    await screen.findByRole("heading", { name: "Agent settings" });
    expect(api.agentMessages).not.toHaveBeenCalled();

    act(() => {
      latestEventSource.onmessage?.({
        data: JSON.stringify({
          agent_id: agent.id,
          kind: "message",
          data: {},
        }),
      } as MessageEvent);
    });
    expect(api.agentMessages).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("link", { name: "Back to chat" }));
    expect(await screen.findByText("Current history")).toBeTruthy();
    expect(api.agentMessages).toHaveBeenCalledTimes(1);
    expect(api.agentMessages).toHaveBeenCalledWith(agent.id, { limit: 50 });
  });

  it("shows a useful fallback for a missing entity", async () => {
    window.history.replaceState(null, "", "#agents/999");
    render(<App />);

    expect(
      await screen.findByRole("heading", { name: "Agent not found" }),
    ).toBeTruthy();
    expect(screen.getByRole("link", { name: "Return to board" })).toBeTruthy();
  });

  it("applies known events locally and coalesces unknown invalidations", async () => {
    window.history.replaceState(null, "", "#agents/1");
    render(<App />);
    await screen.findByRole("heading", { name: "worker" });

    act(() => {
      latestEventSource.onmessage?.({
        data: JSON.stringify({
          agent_id: agent.id,
          kind: "agent_message_delta",
          data: { delta: "Working" },
        }),
      } as MessageEvent);
      latestEventSource.onmessage?.({
        data: JSON.stringify({
          agent_id: agent.id,
          kind: "agent_updated",
          data: { ...agent, name: "architect" },
        }),
      } as MessageEvent);
      latestEventSource.onmessage?.({
        data: JSON.stringify({
          agent_id: agent.id,
          kind: "ticket_updated",
          data: ticket,
        }),
      } as MessageEvent);
      latestEventSource.onmessage?.({
        data: JSON.stringify({
          agent_id: 0,
          kind: "settings_updated",
          data: { merge_notification_channel: "general" },
        }),
      } as MessageEvent);
    });

    expect(await screen.findByText("Working")).toBeTruthy();
    expect(screen.getByRole("heading", { name: "architect" })).toBeTruthy();
    await act(() => new Promise((resolve) => window.setTimeout(resolve, 75)));
    expect(api.agents).toHaveBeenCalledTimes(1);
    expect(api.tickets).toHaveBeenCalledTimes(3);
    expect(api.settings).toHaveBeenCalledTimes(1);

    act(() => {
      latestEventSource.onmessage?.({
        data: JSON.stringify({ kind: "refresh" }),
      } as MessageEvent);
    });

    await waitFor(() => expect(api.agents).toHaveBeenCalledTimes(2));
    expect(api.project).toHaveBeenCalledTimes(2);
    expect(api.projects).toHaveBeenCalledTimes(2);
    expect(api.tickets).toHaveBeenCalledTimes(6);
    expect(api.channels).toHaveBeenCalledTimes(2);
    expect(api.settings).toHaveBeenCalledTimes(2);

    act(() => {
      latestEventSource.onmessage?.({
        data: JSON.stringify({ agent_id: 0, kind: "mystery", data: {} }),
      } as MessageEvent);
      latestEventSource.onmessage?.({
        data: JSON.stringify({ agent_id: 0, kind: "another_mystery", data: {} }),
      } as MessageEvent);
    });

    expect(api.agents).toHaveBeenCalledTimes(2);
    await waitFor(() => expect(api.agents).toHaveBeenCalledTimes(3));
    expect(api.project).toHaveBeenCalledTimes(3);
    expect(api.projects).toHaveBeenCalledTimes(3);
    expect(api.tickets).toHaveBeenCalledTimes(9);
    expect(api.channels).toHaveBeenCalledTimes(3);
    expect(api.settings).toHaveBeenCalledTimes(3);
  });

  it("coalesces visible-agent deltas and ignores unrelated agents", async () => {
    window.history.replaceState(null, "", "#agents/1");
    render(<App />);
    await screen.findByRole("heading", { name: "worker" });

    act(() => {
      latestEventSource.onmessage?.({
        data: JSON.stringify({
          agent_id: 2,
          kind: "agent_message_delta",
          data: { delta: "Hidden" },
        }),
      } as MessageEvent);
      latestEventSource.onmessage?.({
        data: JSON.stringify({ agent_id: 2, kind: "message", data: {} }),
      } as MessageEvent);
      for (const delta of ["One ", "two ", "three"]) {
        latestEventSource.onmessage?.({
          data: JSON.stringify({
            agent_id: agent.id,
            kind: "agent_message_delta",
            data: { delta },
          }),
        } as MessageEvent);
      }
    });

    expect(screen.queryByText("Hidden")).toBeNull();
    expect(screen.queryByText("One two three")).toBeNull();
    expect(await screen.findByText("One two three")).toBeTruthy();
    expect(screen.queryByText("Hidden")).toBeNull();
    expect(api.agentMessages).toHaveBeenCalledTimes(1);
  });

  it("renders incomplete streaming text without the rich-content pipeline", async () => {
    window.history.replaceState(null, "", "#agents/1");
    render(<App />);
    await screen.findByRole("heading", { name: "worker" });

    act(() => {
      latestEventSource.onmessage?.({
        data: JSON.stringify({
          agent_id: agent.id,
          kind: "agent_message_delta",
          data: { delta: "**unfinished**" },
        }),
      } as MessageEvent);
    });

    const stream = await waitFor(() => {
      const element = document.querySelector<HTMLElement>(".streaming-text");
      expect(element?.textContent).toBe("**unfinished**");
      return element!;
    });
    expect(stream.querySelector("strong")).toBeNull();
  });

  it("merges board messages without treating agent-thread messages as channel posts", async () => {
    window.history.replaceState(null, "", "#messages/general");
    vi.mocked(api.channels).mockResolvedValue([
      { name: "general", description: "", most_recent_message_at: 0 },
    ]);
    vi.mocked(api.messages).mockResolvedValue({ messages: [], has_more: false });
    render(<App />);
    await screen.findByText("No messages in #general yet.");
    fireEvent.change(screen.getByLabelText("Message"), {
      target: { value: "Unsent reply" },
    });

    act(() => {
      latestEventSource.onmessage?.({
        data: JSON.stringify({ agent_id: agent.id, kind: "message", data: {} }),
      } as MessageEvent);
    });
    await act(() => new Promise((resolve) => window.setTimeout(resolve, 25)));
    expect(api.messages).toHaveBeenCalledTimes(1);
    expect(screen.queryByText("Remote post")).toBeNull();

    act(() => {
      const event = {
        data: JSON.stringify({
          agent_id: 0,
          kind: "board_message",
          data: {
            id: 4,
            channel_name: "general",
            agent_id: null,
            author: "human",
            body: "Remote post",
            created_at: 0,
          },
        }),
      } as MessageEvent;
      latestEventSource.onmessage?.(event);
      latestEventSource.onmessage?.(event);
      latestEventSource.onmessage?.({
        data: JSON.stringify({
          agent_id: 0,
          kind: "board_message",
          data: {
            id: 5,
            channel_name: "backend",
            agent_id: null,
            author: "human",
            body: "Other channel",
            created_at: 0,
          },
        }),
      } as MessageEvent);
    });

    expect(screen.getAllByText("Remote post")).toHaveLength(1);
    expect(screen.queryByText("Other channel")).toBeNull();
    expect((screen.getByLabelText("Message") as HTMLTextAreaElement).value).toBe(
      "Unsent reply",
    );
    expect(api.messages).toHaveBeenCalledTimes(1);
  });

  it("appends and deduplicates completed messages for only the visible agent", async () => {
    window.history.replaceState(null, "", "#agents/1");
    render(<App />);
    await screen.findByRole("heading", { name: "worker" });
    await waitFor(() => expect(api.agentMessages).toHaveBeenCalledTimes(1));

    act(() => {
      const visible = {
        data: JSON.stringify({
          agent_id: agent.id,
          kind: "message",
          data: {
            id: 7,
            agent_id: agent.id,
            role: "agent",
            body: "Completed answer",
            created_at: 7,
          },
        }),
      } as MessageEvent;
      latestEventSource.onmessage?.(visible);
      latestEventSource.onmessage?.(visible);
      latestEventSource.onmessage?.({
        data: JSON.stringify({
          agent_id: 2,
          kind: "message",
          data: {
            id: 8,
            agent_id: 2,
            role: "agent",
            body: "Other agent",
            created_at: 8,
          },
        }),
      } as MessageEvent);
    });

    expect(screen.getAllByText("Completed answer")).toHaveLength(1);
    expect(screen.queryByText("Other agent")).toBeNull();
    expect(api.agentMessages).toHaveBeenCalledTimes(1);
  });

  it("catches up the visible conversation once after reconnect and preserves its draft", async () => {
    window.history.replaceState(null, "", "#messages/general");
    vi.mocked(api.channels).mockResolvedValue([
      { name: "general", description: "", most_recent_message_at: 0 },
    ]);
    vi.mocked(api.messages)
      .mockResolvedValueOnce({
        messages: [{
          id: 10,
          channel_name: "general",
          agent_id: null,
          author: "human",
          body: "Before disconnect",
          created_at: 10,
        }],
        has_more: false,
      })
      .mockResolvedValueOnce({
        messages: [
          {
            id: 10,
            channel_name: "general",
            agent_id: null,
            author: "human",
            body: "Before disconnect",
            created_at: 10,
          },
          {
            id: 11,
            channel_name: "general",
            agent_id: null,
            author: "human",
            body: "While disconnected",
            created_at: 11,
          },
        ],
        has_more: false,
      });
    render(<App />);
    await screen.findByText("Before disconnect");
    fireEvent.change(screen.getByLabelText("Message"), {
      target: { value: "Unsent reply" },
    });

    act(() => latestEventSource.onopen?.());
    act(() => latestEventSource.onopen?.());

    expect(await screen.findByText("While disconnected")).toBeTruthy();
    expect(api.messages).toHaveBeenNthCalledWith(2, "general", {
      after_id: 10,
      limit: 100,
    });
    expect(screen.getAllByText("Before disconnect")).toHaveLength(1);
    expect((screen.getByLabelText("Message") as HTMLTextAreaElement).value).toBe(
      "Unsent reply",
    );
  });

  it("does not fan periodic refreshes out through hidden visited views", async () => {
    window.history.replaceState(null, "", "#messages/general");
    vi.mocked(api.channels).mockResolvedValue([
      { name: "general", description: "", most_recent_message_at: 0 },
    ]);
    render(<App />);
    await screen.findByText("No messages in #general yet.");
    fireEvent.click(screen.getByRole("link", { name: "agents" }));
    await screen.findByRole("heading", { name: "worker" });
    await waitFor(() => expect(api.agentProjectSubscriptions).toHaveBeenCalledTimes(1));

    act(() => {
      latestEventSource.onmessage?.({
        data: JSON.stringify({ kind: "refresh" }),
      } as MessageEvent);
    });

    await waitFor(() => expect(api.agents).toHaveBeenCalledTimes(2));
    expect(api.messages).toHaveBeenCalledTimes(1);
    expect(api.agentMessages).toHaveBeenCalledTimes(2);
    expect(api.agentMessages).toHaveBeenLastCalledWith(agent.id, { limit: 50 });
    expect(api.agentProjectWorktrees).toHaveBeenCalledTimes(1);
    expect(api.agentScopes).toHaveBeenCalledTimes(1);
    expect(api.agentChannelSubscriptions).toHaveBeenCalledTimes(1);
    expect(api.agentProjectSubscriptions).toHaveBeenCalledTimes(1);
  });

  it("repairs once after reconnect without reloading for the initial open", async () => {
    render(<App />);
    await screen.findByRole("heading", { name: "Agent settings" });

    act(() => latestEventSource.onopen?.());
    await act(() => new Promise((resolve) => window.setTimeout(resolve, 75)));
    expect(api.agents).toHaveBeenCalledTimes(1);

    act(() => latestEventSource.onopen?.());
    await waitFor(() => expect(api.agents).toHaveBeenCalledTimes(2));
    expect(api.project).toHaveBeenCalledTimes(2);
    expect(api.projects).toHaveBeenCalledTimes(2);
    expect(api.tickets).toHaveBeenCalledTimes(6);
    expect(api.channels).toHaveBeenCalledTimes(2);
    expect(api.settings).toHaveBeenCalledTimes(2);
  });

  it("refreshes agent details for the project_subscriptions event", async () => {
    window.history.replaceState(null, "", "#agents/1/settings");
    vi.mocked(api.projects).mockResolvedValue([project]);
    vi.mocked(api.agentProjectSubscriptions)
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce([project.id]);
    render(<App />);
    const subscription = await screen.findByRole("checkbox", {
      name: new RegExp(project.name),
    });
    expect((subscription as HTMLInputElement).checked).toBe(false);

    act(() => {
      latestEventSource.onmessage?.({
        data: JSON.stringify({
          agent_id: agent.id,
          kind: "project_subscriptions",
          data: [project.id],
        }),
      } as MessageEvent);
    });

    await waitFor(() =>
      expect((subscription as HTMLInputElement).checked).toBe(true),
    );
    expect(api.agentProjectSubscriptions).toHaveBeenCalledTimes(2);
    expect(api.projects).toHaveBeenCalledTimes(1);
  });

  it("keeps a settings draft when switching views", async () => {
    window.history.replaceState(null, "", "#settings");
    render(<App />);
    fireEvent.change(
      await screen.findByLabelText("Merge notification channel"),
      { target: { value: "release-draft" } },
    );
    fireEvent.click(screen.getByRole("link", { name: "agents" }));
    await screen.findByRole("heading", { name: "worker" });
    fireEvent.click(screen.getByRole("link", { name: "settings" }));

    expect(
      (screen.getByLabelText("Merge notification channel") as HTMLInputElement)
        .value,
    ).toBe("release-draft");
  });

  it("preserves a dirty ticket when that ticket changes remotely", async () => {
    window.history.replaceState(null, "", "#board?project=2&ticket=7");
    vi.mocked(api.projects).mockResolvedValue([project]);
    vi.mocked(api.tickets).mockImplementation(async ({ state } = {}) => ({
      tickets: state === ticket.state ? [{ ...ticket, has_review_feedback: false }] : [],
      has_more: false,
    }));
    render(<App />);
    const title = await screen.findByDisplayValue(ticket.title);
    fireEvent.change(title, { target: { value: "My unfinished title" } });
    await waitFor(() =>
      expect(screen.getByDisplayValue("My unfinished title")).toBeTruthy(),
    );

    act(() => {
      latestEventSource.onmessage?.({
        data: JSON.stringify({
          agent_id: 1,
          kind: "ticket_updated",
          data: { ...ticket, title: "Remote title", revision: 2 },
        }),
      } as MessageEvent);
    });

    expect(screen.getByDisplayValue("My unfinished title")).toBeTruthy();
    expect(screen.getByRole("status").textContent).toContain("updated elsewhere");
    expect(api.tickets).toHaveBeenCalledTimes(3);
  });
});

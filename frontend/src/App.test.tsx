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
    agentProjectWorktrees: vi.fn(),
    agentScopes: vi.fn(),
    agents: vi.fn(),
    channels: vi.fn(),
    project: vi.fn(),
    projects: vi.fn(),
    settings: vi.fn(),
    tickets: vi.fn(),
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
  onmessage: ((event: MessageEvent) => void) | null = null;
  close() {}
}

describe("App agent routes", () => {
  beforeEach(() => {
    vi.stubGlobal("EventSource", MockEventSource);
    window.history.replaceState(null, "", "#agents/1/settings");
    vi.mocked(api.project).mockResolvedValue(null);
    vi.mocked(api.projects).mockResolvedValue([]);
    vi.mocked(api.settings).mockResolvedValue({
      merge_notification_channel: null,
    });
    vi.mocked(api.tickets).mockResolvedValue([]);
    vi.mocked(api.channels).mockResolvedValue([]);
    vi.mocked(api.agents).mockResolvedValue([agent]);
    vi.mocked(api.agentMessages).mockResolvedValue([]);
    vi.mocked(api.agentProjectWorktrees).mockResolvedValue([]);
    vi.mocked(api.agentScopes).mockResolvedValue([]);
    vi.mocked(api.agentChannelSubscriptions).mockResolvedValue([]);
    vi.mocked(api.messages).mockResolvedValue([]);
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
    expect(api.messages).toHaveBeenLastCalledWith("releases");
  });

  it("restores a project filter and open ticket from a direct link", async () => {
    window.history.replaceState(null, "", "#board?project=2&ticket=7");
    vi.mocked(api.projects).mockResolvedValue([project]);
    vi.mocked(api.tickets).mockResolvedValue([ticket]);

    render(<App />);

    expect(await screen.findByDisplayValue("Durable routes")).toBeTruthy();
    expect(
      (screen.getByLabelText("Filter projects") as HTMLSelectElement).value,
    ).toBe("2");
    const summary = screen.getByText("Durable routes").closest("summary")!;
    expect(summary.closest("details")?.open).toBe(true);
    fireEvent.click(summary);
    expect(window.location.hash).toBe("#board?project=2");
    fireEvent.click(screen.getByText("Durable routes").closest("summary")!);
    expect(window.location.hash).toBe("#board?project=2&ticket=7");
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

  it("shows a useful fallback for a missing entity", async () => {
    window.history.replaceState(null, "", "#agents/999");
    render(<App />);

    expect(
      await screen.findByRole("heading", { name: "Agent not found" }),
    ).toBeTruthy();
    expect(screen.getByRole("link", { name: "Return to board" })).toBeTruthy();
  });
});

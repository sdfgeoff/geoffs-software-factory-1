import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "./api";
import { App } from "./App";
import type { Agent } from "./types";

vi.mock("./api", () => ({
  api: {
    agentChannelSubscriptions: vi.fn(),
    agentMessages: vi.fn(),
    agentScopes: vi.fn(),
    agents: vi.fn(),
    channels: vi.fn(),
    project: vi.fn(),
    tickets: vi.fn(),
  },
}));

const agent: Agent = {
  id: 1,
  name: "worker",
  state: "idle",
  thread_id: null,
  worktree_path: "/repo/worker",
  active_turn_id: null,
  last_error: null,
  wake_on_pending_task: false,
  idle_heartbeat_minutes: null,
  idle_since: 0,
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
    vi.mocked(api.tickets).mockResolvedValue([]);
    vi.mocked(api.channels).mockResolvedValue([]);
    vi.mocked(api.agents).mockResolvedValue([agent]);
    vi.mocked(api.agentMessages).mockResolvedValue([]);
    vi.mocked(api.agentScopes).mockResolvedValue([]);
    vi.mocked(api.agentChannelSubscriptions).mockResolvedValue([]);
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
});

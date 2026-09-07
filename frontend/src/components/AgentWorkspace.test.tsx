import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import type { Agent, Ticket } from "../types";
import { AgentWorkspace } from "./AgentWorkspace";

vi.mock("../api", () => ({
  api: {
    agentChannelSubscriptions: vi.fn(),
    agentMessages: vi.fn(),
    agentProjectWorktrees: vi.fn(),
    agentScopes: vi.fn(),
    pauseAgent: vi.fn(),
    renameAgent: vi.fn(),
    resumeAgent: vi.fn(),
    sendAgentMessage: vi.fn(),
    setAgentScopes: vi.fn(),
    setAgentChannelSubscriptions: vi.fn(),
    setAgentWakeSettings: vi.fn(),
  },
}));

const agent: Agent = {
  id: 1,
  name: "reviewer",
  state: "idle",
  thread_id: null,
  home_path: "/repo/reviewer-home",
  active_project_id: 1,
  active_turn_id: null,
  last_error: null,
  wake_on_pending_work: false,
  wake_on_pending_review: false,
  idle_heartbeat_minutes: null,
  idle_since: 0,
  created_at: 0,
  updated_at: 0,
};

const ticket: Ticket = {
  id: 17,
  project_id: 1,
  title: "Move agent settings",
  description: "",
  state: "in_progress",
  created_by_agent_id: null,
  reserved_for_agent_id: null,
  assignee_id: agent.id,
  implementation_branch: null,
  review_commit: null,
  review_target_branch: null,
  review_project_id: null,
  review_project_path: null,
  reviewer_id: null,
  review_feedback: null,
  blocker_ids: [],
  unresolved_blockers: [],
  revision: 1,
  created_at: 0,
  updated_at: 0,
};

describe("AgentWorkspace", () => {
  afterEach(cleanup);

  beforeEach(() => {
    vi.mocked(api.agentChannelSubscriptions).mockResolvedValue([]);
    vi.mocked(api.agentMessages).mockResolvedValue([]);
    vi.mocked(api.agentProjectWorktrees).mockResolvedValue([
      {
        agent_id: agent.id,
        project_id: 1,
        path: "/repo/reviewer",
        created_at: 0,
        updated_at: 0,
      },
    ]);
    vi.mocked(api.agentScopes).mockResolvedValue([]);
    vi.mocked(api.pauseAgent).mockResolvedValue(agent);
    vi.mocked(api.renameAgent).mockResolvedValue(agent);
    vi.mocked(api.resumeAgent).mockResolvedValue(agent);
    vi.mocked(api.sendAgentMessage).mockResolvedValue(agent);
    vi.mocked(api.setAgentScopes).mockResolvedValue(["approve_review"]);
    vi.mocked(api.setAgentChannelSubscriptions).mockResolvedValue(["general"]);
    vi.mocked(api.setAgentWakeSettings).mockResolvedValue(agent);
  });

  it("assigns the approve_review capability", async () => {
    render(
      <AgentWorkspace
        agents={[agent]}
        channels={[{ name: "general", description: "", most_recent_message_at: 0 }]}
        tickets={[]}
        projects={[{ id: 1, name: "Board", path: "/repo", default_branch: "main", archived_at: null, updated_at: 0 }]}
        selectedAgentId={agent.id}
        view="settings"
        refresh={0}
        deltas={{}}
        onSelectAgent={() => {}}
        onViewChange={() => {}}
        onChanged={async () => {}}
        onError={() => {}}
      />,
    );

    fireEvent.click(
      await screen.findByRole("checkbox", { name: /approve_review/i }),
    );
    expect(screen.getByText("Board")).toBeTruthy();
    expect(screen.getAllByText("/repo/reviewer")).toHaveLength(2);
    await waitFor(() =>
      expect(api.setAgentScopes).toHaveBeenCalledWith(1, ["approve_review"]),
    );
  });

  it("subscribes to a message-board channel", async () => {
    render(
      <AgentWorkspace
        agents={[agent]}
        channels={[{ name: "general", description: "", most_recent_message_at: 0 }]}
        tickets={[]}
        selectedAgentId={agent.id}
        view="settings"
        refresh={0}
        deltas={{}}
        onSelectAgent={() => {}}
        onViewChange={() => {}}
        onChanged={async () => {}}
        onError={() => {}}
      />,
    );

    fireEvent.click(
      await screen.findByRole("checkbox", { name: /#general/i }),
    );
    await waitFor(() =>
      expect(api.setAgentChannelSubscriptions).toHaveBeenCalledWith(1, [
        "general",
      ]),
    );
  });

  it("saves independent work, review, and heartbeat settings", async () => {
    render(
      <AgentWorkspace
        agents={[agent]}
        channels={[{ name: "general", description: "", most_recent_message_at: 0 }]}
        tickets={[]}
        selectedAgentId={agent.id}
        view="settings"
        refresh={0}
        deltas={{}}
        onSelectAgent={() => {}}
        onViewChange={() => {}}
        onChanged={async () => {}}
        onError={() => {}}
      />,
    );

    fireEvent.click(screen.getByRole("checkbox", { name: /pending work/i }));
    fireEvent.click(screen.getByRole("checkbox", { name: /idle heartbeat/i }));
    fireEvent.change(screen.getByRole("spinbutton", { name: "Minutes" }), {
      target: { value: "30" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save wake settings" }));

    await waitFor(() =>
      expect(api.setAgentWakeSettings).toHaveBeenCalledWith(
        agent.id,
        true,
        false,
        30,
      ),
    );

    vi.mocked(api.setAgentWakeSettings).mockClear();
    fireEvent.click(screen.getByRole("checkbox", { name: /pending work/i }));
    fireEvent.click(screen.getByRole("checkbox", { name: /pending reviews/i }));
    fireEvent.click(screen.getByRole("button", { name: "Save wake settings" }));

    await waitFor(() =>
      expect(api.setAgentWakeSettings).toHaveBeenCalledWith(
        agent.id,
        false,
        true,
        30,
      ),
    );
  });

  it("renames the selected agent", async () => {
    const onChanged = vi.fn();
    render(
      <AgentWorkspace
        agents={[agent]}
        channels={[{ name: "general", description: "", most_recent_message_at: 0 }]}
        tickets={[]}
        selectedAgentId={agent.id}
        view="settings"
        refresh={0}
        deltas={{}}
        onSelectAgent={() => {}}
        onViewChange={() => {}}
        onChanged={onChanged}
        onError={() => {}}
      />,
    );

    fireEvent.change(screen.getByRole("textbox", { name: "Agent name" }), {
      target: { value: "architect" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Rename" }));

    await waitFor(() =>
      expect(api.renameAgent).toHaveBeenCalledWith(agent.id, "architect"),
    );
    expect(onChanged).toHaveBeenCalled();
  });

  it("sends a message while an agent is running", async () => {
    render(
      <AgentWorkspace
        agents={[{ ...agent, state: "running", active_turn_id: "turn-1" }]}
        channels={[{ name: "general", description: "", most_recent_message_at: 0 }]}
        tickets={[]}
        selectedAgentId={agent.id}
        view="chat"
        refresh={0}
        deltas={{}}
        onSelectAgent={() => {}}
        onViewChange={() => {}}
        onChanged={async () => {}}
        onError={() => {}}
      />,
    );

    fireEvent.change(screen.getByRole("textbox", { name: "Message" }), {
      target: { value: "Check the failing test" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    await waitFor(() =>
      expect(api.sendAgentMessage).toHaveBeenCalledWith(
        agent.id,
        "Check the failing test",
      ),
    );
  });

  it("requires an explicit resume for an error-state agent", async () => {
    render(
      <AgentWorkspace
        agents={[{ ...agent, state: "error", last_error: "Turn failed" }]}
        channels={[{ name: "general", description: "", most_recent_message_at: 0 }]}
        tickets={[]}
        selectedAgentId={agent.id}
        view="chat"
        refresh={0}
        deltas={{}}
        onSelectAgent={() => {}}
        onViewChange={() => {}}
        onChanged={async () => {}}
        onError={() => {}}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Resume" }));

    await waitFor(() => expect(api.resumeAgent).toHaveBeenCalledWith(agent.id));
  });

  it("opens settings from an operational chat header", () => {
    const onViewChange = vi.fn();
    render(
      <AgentWorkspace
        agents={[agent]}
        channels={[]}
        tickets={[ticket]}
        projects={[{ id: 1, name: "Board", path: "/repo", default_branch: "main", archived_at: null, updated_at: 0 }]}
        selectedAgentId={agent.id}
        view="chat"
        refresh={0}
        deltas={{}}
        onSelectAgent={() => {}}
        onViewChange={onViewChange}
        onChanged={async () => {}}
        onError={() => {}}
      />,
    );

    expect(
      screen.getByText((_, element) =>
        Boolean(element?.classList.contains("agent-operation")),
      ).textContent,
    ).toContain("Board");
    const settings = screen.getByRole("link", { name: "Settings" });
    expect(settings.getAttribute("href")).toBe("#agents/1/settings");
    fireEvent.click(settings);
    expect(onViewChange).toHaveBeenCalledWith("settings");
  });

  it("preserves the chat draft and scroll position while settings are open", () => {
    const props = {
      agents: [agent],
      channels: [],
      tickets: [],
      selectedAgentId: agent.id,
      refresh: 0,
      deltas: {},
      onSelectAgent: () => {},
      onViewChange: () => {},
      onChanged: async () => {},
      onError: () => {},
    };
    const { container, rerender } = render(
      <AgentWorkspace {...props} view="chat" />,
    );
    const draft = screen.getByRole("textbox", { name: "Message" });
    const chat = container.querySelector<HTMLElement>(".agent-chat")!;
    fireEvent.change(draft, { target: { value: "Unsent work" } });
    chat.scrollTop = 80;

    rerender(<AgentWorkspace {...props} view="settings" />);
    expect(screen.getByRole("heading", { name: "Agent settings" })).toBeTruthy();
    expect((screen.getByLabelText("Message") as HTMLTextAreaElement).value).toBe(
      "Unsent work",
    );
    expect(chat.scrollTop).toBe(80);

    rerender(<AgentWorkspace {...props} view="chat" />);
    expect((screen.getByLabelText("Message") as HTMLTextAreaElement).value).toBe(
      "Unsent work",
    );
    expect(chat.scrollTop).toBe(80);
  });

  it("keeps chat and settings drafts separate between agents", () => {
    const second = { ...agent, id: 2, name: "builder" };
    const props = {
      agents: [agent, second],
      channels: [],
      tickets: [],
      view: "chat" as const,
      refresh: 0,
      deltas: {},
      onSelectAgent: () => {},
      onViewChange: () => {},
      onChanged: async () => {},
      onError: () => {},
    };
    const { rerender } = render(
      <AgentWorkspace {...props} selectedAgentId={agent.id} />,
    );
    fireEvent.change(screen.getByLabelText("Message"), {
      target: { value: "review draft" },
    });
    rerender(<AgentWorkspace {...props} selectedAgentId={second.id} />);
    expect((screen.getByLabelText("Message") as HTMLTextAreaElement).value).toBe("");
    fireEvent.change(screen.getByLabelText("Message"), {
      target: { value: "build draft" },
    });
    rerender(<AgentWorkspace {...props} selectedAgentId={agent.id} />);
    expect((screen.getByLabelText("Message") as HTMLTextAreaElement).value).toBe(
      "review draft",
    );

    rerender(
      <AgentWorkspace
        {...props}
        selectedAgentId={second.id}
        view="settings"
      />,
    );
    fireEvent.change(screen.getByLabelText("Agent name"), {
      target: { value: "builder-draft" },
    });
    rerender(
      <AgentWorkspace
        {...props}
        selectedAgentId={agent.id}
        view="settings"
      />,
    );
    expect((screen.getByLabelText("Agent name") as HTMLInputElement).value).toBe(
      "reviewer",
    );
  });
});

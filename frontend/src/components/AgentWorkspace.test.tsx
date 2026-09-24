import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import type { Agent, AgentMessagePage, Project, Ticket } from "../types";
import { AgentWorkspace } from "./AgentWorkspace";

const messageContentRender = vi.hoisted(() => vi.fn());

vi.mock("./MessageContent", () => ({
  MessageContent: ({ source }: { source: string }) => {
    messageContentRender(source);
    return <div>{source}</div>;
  },
}));

vi.mock("../api", () => ({
  api: {
    agentChannelSubscriptions: vi.fn(),
    agentMessages: vi.fn(),
    agentProjectSubscriptions: vi.fn(),
    agentProjectWorktrees: vi.fn(),
    agentScopes: vi.fn(),
    createAgent: vi.fn(),
    pauseAgent: vi.fn(),
    renameAgent: vi.fn(),
    setAgentInstructions: vi.fn(),
    resumeAgent: vi.fn(),
    sendAgentMessage: vi.fn(),
    setAgentScopes: vi.fn(),
    setAgentChannelSubscriptions: vi.fn(),
    setAgentProjectSubscriptions: vi.fn(),
    setAgentWakeSettings: vi.fn(),
  },
}));

const agent: Agent = {
  id: 1,
  name: "reviewer",
  instructions: "",
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
  review_completion_workflow: null,
  external_change_url: null,
  external_merge_reference: null,
  external_merged_at: null,
  blocker_ids: [],
  unresolved_blockers: [],
  revision: 1,
  created_at: 0,
  updated_at: 0,
};

describe("AgentWorkspace", () => {
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(api.agentChannelSubscriptions).mockResolvedValue([]);
    vi.mocked(api.agentMessages).mockResolvedValue({ messages: [], has_more: false });
    vi.mocked(api.agentProjectSubscriptions).mockResolvedValue([1]);
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
    vi.mocked(api.createAgent).mockResolvedValue(agent);
    vi.mocked(api.pauseAgent).mockResolvedValue(agent);
    vi.mocked(api.renameAgent).mockResolvedValue(agent);
    vi.mocked(api.setAgentInstructions).mockResolvedValue(agent);
    vi.mocked(api.resumeAgent).mockResolvedValue(agent);
    vi.mocked(api.sendAgentMessage).mockResolvedValue(agent);
    vi.mocked(api.setAgentScopes).mockResolvedValue(["approve_review"]);
    vi.mocked(api.setAgentChannelSubscriptions).mockResolvedValue(["general"]);
    vi.mocked(api.setAgentProjectSubscriptions).mockResolvedValue([]);
    vi.mocked(api.setAgentWakeSettings).mockResolvedValue(agent);
  });

  it("requires an explicit initial project when creating an agent", async () => {
    const project: Project = {
      id: 4,
      name: "Fieldscan",
      path: "/repo/fieldscan",
      default_branch: "main",
      completion_workflow: "external",
      archived_at: null,
      updated_at: 0,
    };
    render(
      <AgentWorkspace
        agents={[]}
        channels={[]}
        tickets={[]}
        projects={[project]}
        selectedAgentId={null}
        view="chat"
        refresh={0}
        deltas={{}}
        onSelectAgent={() => {}}
        onViewChange={() => {}}
        onChanged={async () => {}}
        onError={() => {}}
      />,
    );

    fireEvent.change(screen.getByLabelText("New agent"), { target: { value: "builder" } });
    expect((screen.getByRole("button", { name: "Create agent" }) as HTMLButtonElement).form?.checkValidity()).toBe(false);
    fireEvent.change(screen.getByLabelText("Initial project"), { target: { value: "4" } });
    fireEvent.click(screen.getByRole("button", { name: "Create agent" }));
    await waitFor(() => expect(api.createAgent).toHaveBeenCalledWith("builder", 4));
  });

  it("does not rerender a 500-message history while typing", async () => {
    vi.mocked(api.agentMessages).mockResolvedValue({
      messages: Array.from({ length: 500 }, (_, index) => ({
        id: index + 1,
        agent_id: agent.id,
        role: index % 2 === 0 ? "user" : "agent",
        body: `History ${index + 1}`,
        created_at: index,
      })),
      has_more: false,
    });
    render(
      <AgentWorkspace
        agents={[agent]}
        channels={[]}
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

    expect(await screen.findByText("History 500")).toBeTruthy();
    expect(messageContentRender).toHaveBeenCalledTimes(500);
    messageContentRender.mockClear();
    fireEvent.change(screen.getByRole("textbox", { name: "Message" }), {
      target: { value: "draft" },
    });
    expect(messageContentRender).not.toHaveBeenCalled();
  });

  it("prepends older agent messages without losing the draft or scroll position", async () => {
    const initial = [
      { id: 51, agent_id: agent.id, role: "agent" as const, body: "Message 51", created_at: 51 },
      { id: 52, agent_id: agent.id, role: "agent" as const, body: "Message 52", created_at: 52 },
    ];
    let resolveOlder!: (value: AgentMessagePage) => void;
    vi.mocked(api.agentMessages)
      .mockResolvedValueOnce({ messages: initial, has_more: true })
      .mockReturnValueOnce(new Promise((resolve) => {
        resolveOlder = resolve;
      }));
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      callback(0);
      return 1;
    });
    const { container } = render(
      <AgentWorkspace
        agents={[agent]}
        channels={[]}
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
    const loadOlder = await screen.findByRole("button", { name: "Load older messages" });
    const draft = screen.getByRole("textbox", { name: "Message" });
    fireEvent.change(draft, { target: { value: "Unsent draft" } });
    const list = container.querySelector<HTMLElement>(".agent-chat")!;
    let height = 1_000;
    Object.defineProperty(list, "scrollHeight", { configurable: true, get: () => height });
    list.scrollTop = 100;

    fireEvent.click(loadOlder);
    expect(api.agentMessages).toHaveBeenLastCalledWith(agent.id, {
      before_id: 51,
      limit: 50,
    });
    height = 1_400;
    await act(async () => {
      resolveOlder({
        messages: [
          { id: 49, agent_id: agent.id, role: "user", body: "Message 49", created_at: 49 },
          { id: 50, agent_id: agent.id, role: "agent", body: "Message 50", created_at: 50 },
          initial[0],
        ],
        has_more: false,
      });
    });

    expect((draft as HTMLTextAreaElement).value).toBe("Unsent draft");
    expect(list.scrollTop).toBe(500);
    expect(screen.getAllByText("Message 51")).toHaveLength(1);
    expect(screen.queryByRole("button", { name: "Load older messages" })).toBeNull();
  });

  it("ignores an older-page response after switching agents", async () => {
    const second = { ...agent, id: 2, name: "builder" };
    let resolveOlder!: (value: AgentMessagePage) => void;
    vi.mocked(api.agentMessages)
      .mockResolvedValueOnce({
        messages: [{ id: 20, agent_id: agent.id, role: "agent", body: "Reviewer current", created_at: 20 }],
        has_more: true,
      })
      .mockReturnValueOnce(new Promise((resolve) => {
        resolveOlder = resolve;
      }))
      .mockResolvedValueOnce({
        messages: [{ id: 30, agent_id: second.id, role: "agent", body: "Builder current", created_at: 30 }],
        has_more: false,
      });
    const animationFrame = vi.fn(() => 1);
    vi.stubGlobal("requestAnimationFrame", animationFrame);
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

    fireEvent.click(await screen.findByRole("button", { name: "Load older messages" }));
    rerender(<AgentWorkspace {...props} selectedAgentId={second.id} />);
    expect(await screen.findByText("Builder current")).toBeTruthy();
    await act(async () => {
      resolveOlder({
        messages: [{ id: 19, agent_id: agent.id, role: "agent", body: "Reviewer older", created_at: 19 }],
        has_more: false,
      });
    });

    expect(screen.queryByText("Reviewer older")).toBeNull();
    expect(screen.getByText("Builder current")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Load older messages" })).toBeNull();
    expect(animationFrame).not.toHaveBeenCalled();
  });

  it("catches up agent messages after reconnect without losing the draft", async () => {
    const first = {
      id: 10,
      agent_id: agent.id,
      role: "agent" as const,
      body: "Before disconnect",
      created_at: 10,
    };
    vi.mocked(api.agentMessages)
      .mockResolvedValueOnce({ messages: [first], has_more: false })
      .mockResolvedValueOnce({
        messages: [
          first,
          { ...first, id: 11, body: "While disconnected", created_at: 11 },
        ],
        has_more: false,
      });
    const props = {
      agents: [agent],
      channels: [],
      tickets: [],
      selectedAgentId: agent.id,
      view: "chat" as const,
      refresh: 0,
      deltas: {},
      onSelectAgent: () => {},
      onViewChange: () => {},
      onChanged: async () => {},
      onError: () => {},
    };
    const { rerender } = render(<AgentWorkspace {...props} catchUp={0} />);
    await screen.findByText("Before disconnect");
    fireEvent.change(screen.getByRole("textbox", { name: "Message" }), {
      target: { value: "Unsent draft" },
    });

    rerender(<AgentWorkspace {...props} catchUp={1} />);

    expect(await screen.findByText("While disconnected")).toBeTruthy();
    expect(api.agentMessages).toHaveBeenNthCalledWith(2, agent.id, {
      after_id: 10,
      limit: 100,
    });
    expect(screen.getAllByText("Before disconnect")).toHaveLength(1);
    expect((screen.getByRole("textbox", { name: "Message" }) as HTMLTextAreaElement).value)
      .toBe("Unsent draft");
  });

  it("assigns the review capability with one explicit save", async () => {
    render(
      <AgentWorkspace
        agents={[agent]}
        channels={[{ name: "general", description: "", most_recent_message_at: 0 }]}
        tickets={[]}
        projects={[{ id: 1, name: "Board", path: "/repo", default_branch: "main", completion_workflow: "board_managed", archived_at: null, updated_at: 0 }]}
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

    const reviewCapability = await screen.findByRole("checkbox", {
      name: /Can review tickets/i,
    });
    await waitFor(() =>
      expect((reviewCapability as HTMLInputElement).disabled).toBe(false),
    );
    fireEvent.click(reviewCapability);
    expect(api.setAgentScopes).not.toHaveBeenCalled();
    expect(screen.getByRole("status").textContent).toBe("Unsaved changes");
    fireEvent.click(screen.getByRole("button", { name: "Save settings" }));
    await waitFor(() =>
      expect(api.setAgentScopes).toHaveBeenCalledWith(1, ["approve_review"]),
    );
    await waitFor(() => expect(screen.getByRole("status").textContent).toBe("Saved"));
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

    const channelSubscription = await screen.findByRole("checkbox", {
      name: /#general/i,
    });
    await waitFor(() =>
      expect((channelSubscription as HTMLInputElement).disabled).toBe(false),
    );
    fireEvent.click(channelSubscription);
    expect(api.setAgentChannelSubscriptions).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Save settings" }));
    await waitFor(() =>
      expect(api.setAgentChannelSubscriptions).toHaveBeenCalledWith(1, [
        "general",
      ]),
    );
  });

  it("persists project subscriptions", async () => {
    render(
      <AgentWorkspace
        agents={[agent]}
        channels={[]}
        tickets={[]}
        projects={[
          { id: 1, name: "Board", path: "/repo", default_branch: "main", completion_workflow: "board_managed", archived_at: null, updated_at: 0 },
          { id: 2, name: "API", path: "/api", default_branch: "main", completion_workflow: "board_managed", archived_at: null, updated_at: 0 },
        ]}
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

    const projectSubscription = await screen.findByRole("checkbox", {
      name: /API/i,
    });
    await waitFor(() =>
      expect((projectSubscription as HTMLInputElement).disabled).toBe(false),
    );
    fireEvent.click(projectSubscription);
    fireEvent.click(screen.getByRole("button", { name: "Save settings" }));
    await waitFor(() =>
      expect(api.setAgentProjectSubscriptions).toHaveBeenCalledWith(1, [1, 2]),
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

    const pendingWork = screen.getByRole("checkbox", { name: /pending work/i });
    await waitFor(() =>
      expect((pendingWork as HTMLInputElement).disabled).toBe(false),
    );
    fireEvent.click(pendingWork);
    fireEvent.click(screen.getByRole("checkbox", { name: /idle heartbeat/i }));
    fireEvent.change(screen.getByRole("spinbutton", { name: "Minutes" }), {
      target: { value: "30" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save settings" }));

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
    fireEvent.click(screen.getByRole("button", { name: "Save settings" }));

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
    await waitFor(() =>
      expect(
        (screen.getByRole("button", { name: "Save settings" }) as HTMLButtonElement)
          .disabled,
      ).toBe(false),
    );
    fireEvent.click(screen.getByRole("button", { name: "Save settings" }));

    await waitFor(() =>
      expect(api.renameAgent).toHaveBeenCalledWith(agent.id, "architect"),
    );
    expect(onChanged).toHaveBeenCalled();
  });

  it("saves agent instructions and preserves the draft when saving fails", async () => {
    vi.mocked(api.setAgentInstructions).mockRejectedValueOnce(
      new Error("Could not restart Codex"),
    );
    render(
      <AgentWorkspace
        agents={[agent]}
        channels={[]}
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
    const instructions = screen.getByRole("textbox", {
      name: /role, temperament/i,
    });
    await waitFor(() =>
      expect(
        (screen.getByRole("checkbox", {
          name: /pending work/i,
        }) as HTMLInputElement).disabled,
      ).toBe(false),
    );
    fireEvent.change(instructions, {
      target: { value: "Challenge assumptions and explain tradeoffs." },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save settings" }));

    await waitFor(() =>
      expect(api.setAgentInstructions).toHaveBeenCalledWith(
        agent.id,
        "Challenge assumptions and explain tradeoffs.",
      ),
    );
    expect(await screen.findByText(/Could not save: Could not restart Codex/)).toBeTruthy();
    expect((instructions as HTMLTextAreaElement).value).toBe(
      "Challenge assumptions and explain tradeoffs.",
    );
  });

  it("keeps dirty instructions through remote updates and locks them during a turn", () => {
    const props = {
      channels: [],
      tickets: [],
      selectedAgentId: agent.id,
      view: "settings" as const,
      refresh: 0,
      deltas: {},
      onSelectAgent: () => {},
      onViewChange: () => {},
      onChanged: async () => {},
      onError: () => {},
    };
    const { rerender } = render(
      <AgentWorkspace {...props} agents={[agent]} />,
    );
    const instructions = screen.getByRole("textbox", {
      name: /role, temperament/i,
    }) as HTMLTextAreaElement;
    fireEvent.change(instructions, { target: { value: "My unsaved persona" } });

    rerender(
      <AgentWorkspace
        {...props}
        agents={[{
          ...agent,
          state: "running",
          instructions: "Remote persona",
          updated_at: 1,
        }]}
      />,
    );

    expect(instructions.value).toBe("My unsaved persona");
    expect(instructions.disabled).toBe(true);
    expect(screen.getByText(/Wait for the active turn to finish/)).toBeTruthy();
    expect(
      (screen.getByRole("button", { name: "Save settings" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
  });

  it("adopts remote instructions when the local form is clean", async () => {
    const props = {
      channels: [],
      tickets: [],
      selectedAgentId: agent.id,
      view: "settings" as const,
      refresh: 0,
      deltas: {},
      onSelectAgent: () => {},
      onViewChange: () => {},
      onChanged: async () => {},
      onError: () => {},
    };
    const { rerender } = render(
      <AgentWorkspace {...props} agents={[agent]} />,
    );

    rerender(
      <AgentWorkspace
        {...props}
        agents={[{ ...agent, instructions: "Remote persona", updated_at: 1 }]}
      />,
    );

    await waitFor(() =>
      expect(
        (screen.getByRole("textbox", {
          name: /role, temperament/i,
        }) as HTMLTextAreaElement).value,
      ).toBe("Remote persona"),
    );
  });

  it("groups settings and keeps implementation paths in Advanced", async () => {
    render(
      <AgentWorkspace
        agents={[agent]}
        channels={[]}
        tickets={[]}
        projects={[{ id: 1, name: "Board", path: "/repo", default_branch: "main", completion_workflow: "board_managed", archived_at: null, updated_at: 0 }]}
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

    expect(screen.getByRole("group", { name: "Identity" })).toBeTruthy();
    expect(screen.getByRole("group", { name: "Agent instructions" })).toBeTruthy();
    expect(screen.getByRole("group", { name: "Work preferences" })).toBeTruthy();
    expect(screen.getByRole("group", { name: "Notifications" })).toBeTruthy();
    expect(screen.queryByText("approve_review")).toBeNull();
    const worktreePath = await screen.findByText("/repo/reviewer");
    expect(worktreePath.closest("details")?.open).toBe(false);
    expect(screen.getByText("Advanced")).toBeTruthy();
  });

  it("shows saving and failure feedback", async () => {
    let rejectSave: (error: Error) => void = () => {};
    vi.mocked(api.setAgentWakeSettings).mockReturnValueOnce(
      new Promise((_, reject) => {
        rejectSave = reject;
      }),
    );
    const onError = vi.fn();
    render(
      <AgentWorkspace
        agents={[agent]}
        channels={[]}
        tickets={[]}
        selectedAgentId={agent.id}
        view="settings"
        refresh={0}
        deltas={{}}
        onSelectAgent={() => {}}
        onViewChange={() => {}}
        onChanged={async () => {}}
        onError={onError}
      />,
    );

    const pendingWork = screen.getByRole("checkbox", { name: /pending work/i });
    await waitFor(() =>
      expect((pendingWork as HTMLInputElement).disabled).toBe(false),
    );
    fireEvent.click(pendingWork);
    fireEvent.click(screen.getByRole("button", { name: "Save settings" }));
    expect(screen.getByRole("status").textContent).toBe("Saving…");
    const pendingReviews = screen.getByRole("checkbox", {
      name: /pending reviews/i,
    }) as HTMLInputElement;
    expect(pendingReviews.closest("fieldset")?.disabled).toBe(true);
    pendingReviews.click();
    expect(pendingReviews.checked).toBe(false);

    rejectSave(new Error("Network unavailable"));
    await waitFor(() =>
      expect(screen.getByRole("status").textContent).toBe(
        "Could not save: Network unavailable",
      ),
    );
    expect(onError).not.toHaveBeenCalled();
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

  it("prevents duplicate sends and preserves a failed message for retry", async () => {
    let rejectSend: (error: Error) => void = () => {};
    vi.mocked(api.sendAgentMessage).mockReturnValueOnce(
      new Promise((_, reject) => {
        rejectSend = reject;
      }),
    );
    const onError = vi.fn();
    render(
      <AgentWorkspace
        agents={[agent]}
        channels={[]}
        tickets={[]}
        selectedAgentId={agent.id}
        view="chat"
        refresh={0}
        deltas={{}}
        onSelectAgent={() => {}}
        onViewChange={() => {}}
        onChanged={async () => {}}
        onError={onError}
      />,
    );

    const draft = screen.getByRole("textbox", { name: "Message" });
    fireEvent.change(draft, { target: { value: "Retry this" } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    const sending = screen.getByRole("button", { name: "Sending…" });
    expect((sending as HTMLButtonElement).disabled).toBe(true);
    expect((draft as HTMLTextAreaElement).disabled).toBe(true);
    fireEvent.click(sending);
    expect(api.sendAgentMessage).toHaveBeenCalledTimes(1);

    rejectSend(new Error("Agent unavailable"));
    expect(await screen.findByText("Could not send: Agent unavailable")).toBeTruthy();
    expect((draft as HTMLTextAreaElement).value).toBe("Retry this");
    expect(onError).not.toHaveBeenCalled();

    vi.mocked(api.sendAgentMessage).mockResolvedValueOnce(agent);
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    expect(await screen.findByText("Sent")).toBeTruthy();
    expect((draft as HTMLTextAreaElement).value).toBe("");
  });

  it("keeps agent sends globally locked after switching agents", async () => {
    const second = { ...agent, id: 2, name: "builder" };
    let resolveSend!: (updated: Agent) => void;
    vi.mocked(api.sendAgentMessage).mockReturnValueOnce(
      new Promise((resolve) => {
        resolveSend = resolve;
      }),
    );
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
    fireEvent.change(screen.getByRole("textbox", { name: "Message" }), {
      target: { value: "First request" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    rerender(<AgentWorkspace {...props} selectedAgentId={second.id} />);
    const secondSend = screen.getByRole("button", { name: "Send" });
    expect((secondSend as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("textbox", { name: "Message" }) as HTMLTextAreaElement).disabled)
      .toBe(true);
    fireEvent.click(secondSend);
    expect(api.sendAgentMessage).toHaveBeenCalledTimes(1);

    rerender(<AgentWorkspace {...props} selectedAgentId={agent.id} />);
    resolveSend(agent);
    expect(await screen.findByText("Sent")).toBeTruthy();
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
        projects={[{ id: 1, name: "Board", path: "/repo", default_branch: "main", completion_workflow: "board_managed", archived_at: null, updated_at: 0 }]}
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

  it("selects agents through the compact navigation control", () => {
    const second = { ...agent, id: 2, name: "builder" };
    const onSelectAgent = vi.fn();
    render(
      <AgentWorkspace
        agents={[agent, second]}
        channels={[]}
        tickets={[]}
        selectedAgentId={agent.id}
        view="chat"
        refresh={0}
        deltas={{}}
        onSelectAgent={onSelectAgent}
        onViewChange={() => {}}
        onChanged={async () => {}}
        onError={() => {}}
      />,
    );

    fireEvent.change(screen.getByRole("combobox", { name: "Agent" }), {
      target: { value: "2" },
    });
    expect(onSelectAgent).toHaveBeenCalledWith(2);
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

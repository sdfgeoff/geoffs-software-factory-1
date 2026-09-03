import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import type { Agent } from "../types";
import { AgentWorkspace } from "./AgentWorkspace";

vi.mock("../api", () => ({
  api: {
    agentMessages: vi.fn(),
    agentScopes: vi.fn(),
    pauseAgent: vi.fn(),
    renameAgent: vi.fn(),
    resumeAgent: vi.fn(),
    sendAgentMessage: vi.fn(),
    setAgentScopes: vi.fn(),
    setAgentWakeSettings: vi.fn(),
  },
}));

const agent: Agent = {
  id: 1,
  name: "reviewer",
  state: "idle",
  thread_id: null,
  worktree_path: "/repo/reviewer",
  active_turn_id: null,
  last_error: null,
  wake_on_pending_task: false,
  idle_heartbeat_minutes: null,
  idle_since: 0,
  created_at: 0,
  updated_at: 0,
};

describe("AgentWorkspace", () => {
  afterEach(cleanup);

  beforeEach(() => {
    vi.mocked(api.agentMessages).mockResolvedValue([]);
    vi.mocked(api.agentScopes).mockResolvedValue([]);
    vi.mocked(api.pauseAgent).mockResolvedValue(agent);
    vi.mocked(api.renameAgent).mockResolvedValue(agent);
    vi.mocked(api.resumeAgent).mockResolvedValue(agent);
    vi.mocked(api.sendAgentMessage).mockResolvedValue(agent);
    vi.mocked(api.setAgentScopes).mockResolvedValue(["approve_review"]);
    vi.mocked(api.setAgentWakeSettings).mockResolvedValue(agent);
  });

  it("assigns the approve_review capability", async () => {
    render(
      <AgentWorkspace
        agents={[agent]}
        selectedAgentId={agent.id}
        refresh={0}
        deltas={{}}
        onSelectAgent={() => {}}
        onChanged={async () => {}}
        onError={() => {}}
      />,
    );

    fireEvent.click(
      await screen.findByRole("checkbox", { name: /approve_review/i }),
    );
    await waitFor(() =>
      expect(api.setAgentScopes).toHaveBeenCalledWith(1, ["approve_review"]),
    );
  });

  it("saves pending-task and heartbeat settings", async () => {
    render(
      <AgentWorkspace
        agents={[agent]}
        selectedAgentId={agent.id}
        refresh={0}
        deltas={{}}
        onSelectAgent={() => {}}
        onChanged={async () => {}}
        onError={() => {}}
      />,
    );

    fireEvent.click(screen.getByRole("checkbox", { name: /pending tasks/i }));
    fireEvent.click(screen.getByRole("checkbox", { name: /idle heartbeat/i }));
    fireEvent.change(screen.getByRole("spinbutton", { name: "Minutes" }), {
      target: { value: "30" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save wake settings" }));

    await waitFor(() =>
      expect(api.setAgentWakeSettings).toHaveBeenCalledWith(agent.id, true, 30),
    );
  });

  it("renames the selected agent", async () => {
    const onChanged = vi.fn();
    render(
      <AgentWorkspace
        agents={[agent]}
        selectedAgentId={agent.id}
        refresh={0}
        deltas={{}}
        onSelectAgent={() => {}}
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
        selectedAgentId={agent.id}
        refresh={0}
        deltas={{}}
        onSelectAgent={() => {}}
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
        selectedAgentId={agent.id}
        refresh={0}
        deltas={{}}
        onSelectAgent={() => {}}
        onChanged={async () => {}}
        onError={() => {}}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Resume" }));

    await waitFor(() => expect(api.resumeAgent).toHaveBeenCalledWith(agent.id));
  });
});

import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import type { Agent } from "../types";
import { AgentWorkspace } from "./AgentWorkspace";

vi.mock("../api", () => ({
  api: {
    agentMessages: vi.fn(),
    agentScopes: vi.fn(),
    setAgentScopes: vi.fn(),
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
  created_at: 0,
  updated_at: 0,
};

describe("AgentWorkspace", () => {
  beforeEach(() => {
    vi.mocked(api.agentMessages).mockResolvedValue([]);
    vi.mocked(api.agentScopes).mockResolvedValue([]);
    vi.mocked(api.setAgentScopes).mockResolvedValue(["approve_review"]);
  });

  it("assigns the approve_review capability", async () => {
    render(
      <AgentWorkspace
        agents={[agent]}
        refresh={0}
        deltas={{}}
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
});

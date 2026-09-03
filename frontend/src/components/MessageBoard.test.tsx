import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { api } from "../api";
import type { Agent } from "../types";
import { MessageBoard } from "./MessageBoard";

vi.mock("../api", () => ({
  api: { messages: vi.fn() },
}));

const agent: Agent = {
  id: 2,
  name: "Worker-Bartholemew",
  state: "idle",
  thread_id: null,
  worktree_path: "/repo/worker",
  active_turn_id: null,
  last_error: null,
  created_at: 0,
  updated_at: 0,
};

describe("MessageBoard", () => {
  it("opens the matching agent from a message author", async () => {
    vi.mocked(api.messages).mockResolvedValue([
      {
        id: 1,
        channel_name: "general",
        author: agent.name,
        body: "Done",
        created_at: 0,
      },
    ]);
    const onSelectAgent = vi.fn();

    render(
      <MessageBoard
        agents={[agent]}
        channels={[{ name: "general", most_recent_message_at: 0 }]}
        refresh={0}
        onSelectAgent={onSelectAgent}
        onChanged={async () => {}}
        onError={() => {}}
      />,
    );

    fireEvent.click(
      await screen.findByRole("button", { name: "Worker-Bartholemew" }),
    );
    expect(onSelectAgent).toHaveBeenCalledWith(agent.id);
  });
});

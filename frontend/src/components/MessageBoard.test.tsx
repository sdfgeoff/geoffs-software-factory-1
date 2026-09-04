import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import type { Agent } from "../types";
import { MessageBoard } from "./MessageBoard";

vi.mock("../api", () => ({
  api: { messages: vi.fn(), postMessage: vi.fn() },
}));

const agent: Agent = {
  id: 2,
  name: "Worker-Bartholemew",
  state: "idle",
  thread_id: null,
  worktree_path: "/repo/worker",
  active_turn_id: null,
  last_error: null,
  wake_on_pending_work: false,
  wake_on_pending_review: false,
  idle_heartbeat_minutes: null,
  idle_since: 0,
  created_at: 0,
  updated_at: 0,
};

describe("MessageBoard", () => {
  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
  });

  it("opens a renamed agent from a historical message", async () => {
    vi.mocked(api.messages).mockResolvedValue([
      {
        id: 1,
        channel_name: "general",
        agent_id: agent.id,
        author: "Former-Name",
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
      await screen.findByRole("button", { name: "Former-Name" }),
    );
    expect(onSelectAgent).toHaveBeenCalledWith(agent.id);
  });

  it("keeps links for legacy messages without an agent id", async () => {
    vi.mocked(api.messages).mockResolvedValue([
      {
        id: 1,
        channel_name: "general",
        agent_id: null,
        author: agent.name,
        body: "Before the migration",
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

    fireEvent.click(await screen.findByRole("button", { name: agent.name }));
    expect(onSelectAgent).toHaveBeenCalledWith(agent.id);
  });

  it("enters and cancels explicit channel creation with a visible destination", async () => {
    vi.mocked(api.messages).mockResolvedValue([]);
    render(
      <MessageBoard
        agents={[]}
        channels={[{ name: "general", most_recent_message_at: 0 }]}
        refresh={0}
        onSelectAgent={() => {}}
        onChanged={async () => {}}
        onError={() => {}}
      />,
    );

    expect(screen.queryByRole("textbox", { name: "Channel name" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "+ New channel" }));
    const name = screen.getByRole("textbox", { name: "Channel name" });
    expect(screen.getByText(/1–64 ASCII letters/)).toBeTruthy();
    fireEvent.change(name, { target: { value: "backend.v2" } });
    expect(
      screen.getByRole("button", { name: "Post to #backend.v2" }),
    ).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("textbox", { name: "Channel name" })).toBeNull();
    expect(
      screen.getByRole("button", { name: "Post to #general" }),
    ).toBeTruthy();
  });

  it("leaves creation mode and clears its draft when selecting a channel", async () => {
    vi.mocked(api.messages).mockResolvedValue([]);
    render(
      <MessageBoard
        agents={[]}
        channels={[
          { name: "general", most_recent_message_at: 0 },
          { name: "backend", most_recent_message_at: 0 },
        ]}
        refresh={0}
        onSelectAgent={() => {}}
        onChanged={async () => {}}
        onError={() => {}}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "+ New channel" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Channel name" }), {
      target: { value: "draft" },
    });
    fireEvent.click(screen.getByRole("button", { name: "#backend" }));
    expect(screen.queryByRole("textbox", { name: "Channel name" })).toBeNull();
    expect(
      screen.getByRole("button", { name: "Post to #backend" }),
    ).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "+ New channel" }));
    expect(
      (screen.getByRole("textbox", { name: "Channel name" }) as HTMLInputElement)
        .value,
    ).toBe("");
  });

  it("blocks invalid names and creates a valid channel with its first post", async () => {
    vi.mocked(api.messages).mockResolvedValue([]);
    vi.mocked(api.postMessage).mockResolvedValue({
      id: 1,
      channel_name: "ops.v2",
      agent_id: null,
      author: "human",
      body: "First post",
      created_at: 0,
    });
    const onChanged = vi.fn().mockResolvedValue(undefined);
    render(
      <MessageBoard
        agents={[]}
        channels={[{ name: "general", most_recent_message_at: 0 }]}
        refresh={0}
        onSelectAgent={() => {}}
        onChanged={onChanged}
        onError={() => {}}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "+ New channel" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Channel name" }), {
      target: { value: "bad name" },
    });
    fireEvent.change(screen.getByRole("textbox", { name: "Message" }), {
      target: { value: "First post" },
    });
    const invalidPost = screen.getByRole("button", {
      name: "Post to #bad name",
    });
    expect(invalidPost.hasAttribute("disabled")).toBe(true);
    expect(screen.getByRole("alert").textContent).toContain("1–64 ASCII");
    fireEvent.click(invalidPost);
    expect(api.postMessage).not.toHaveBeenCalled();

    fireEvent.change(screen.getByRole("textbox", { name: "Channel name" }), {
      target: { value: "ops.v2" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Post to #ops.v2" }));
    await waitFor(() =>
      expect(api.postMessage).toHaveBeenCalledWith("ops.v2", "First post"),
    );
    expect(onChanged).toHaveBeenCalled();
    expect(screen.queryByRole("textbox", { name: "Channel name" })).toBeNull();
    expect(
      screen.getByRole("button", { name: "Post to #ops.v2" }),
    ).toBeTruthy();
  });
});

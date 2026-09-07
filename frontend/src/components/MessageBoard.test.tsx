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
  api: { createChannel: vi.fn(), messages: vi.fn(), postMessage: vi.fn() },
}));

const agent: Agent = {
  id: 2,
  name: "Worker-Bartholemew",
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
        channels={[{ name: "general", description: "", most_recent_message_at: 0 }]}
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
        channels={[{ name: "general", description: "", most_recent_message_at: 0 }]}
        refresh={0}
        onSelectAgent={onSelectAgent}
        onChanged={async () => {}}
        onError={() => {}}
      />,
    );

    fireEvent.click(await screen.findByRole("button", { name: agent.name }));
    expect(onSelectAgent).toHaveBeenCalledWith(agent.id);
  });

  it("enters and cancels explicit channel creation", async () => {
    vi.mocked(api.messages).mockResolvedValue([]);
    render(
      <MessageBoard
        agents={[]}
        channels={[{ name: "general", description: "", most_recent_message_at: 0 }]}
        refresh={0}
        onSelectAgent={() => {}}
        onChanged={async () => {}}
        onError={() => {}}
      />,
    );

    expect(screen.queryByRole("textbox", { name: "Name" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "+ New channel" }));
    const name = screen.getByRole("textbox", { name: "Name" });
    expect(screen.getByRole("textbox", { name: "Description" })).toBeTruthy();
    expect(screen.getByText(/1–64 ASCII letters/)).toBeTruthy();
    fireEvent.change(name, { target: { value: "backend.v2" } });
    expect(screen.getByRole("button", { name: "Save" })).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("textbox", { name: "Name" })).toBeNull();
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
          { name: "general", description: "", most_recent_message_at: 0 },
          { name: "backend", description: "", most_recent_message_at: 0 },
        ]}
        refresh={0}
        onSelectAgent={() => {}}
        onChanged={async () => {}}
        onError={() => {}}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "+ New channel" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Name" }), {
      target: { value: "draft" },
    });
    fireEvent.change(screen.getByRole("textbox", { name: "Description" }), {
      target: { value: "Draft description" },
    });
    fireEvent.click(screen.getByRole("button", { name: "#backend" }));
    expect(screen.queryByRole("textbox", { name: "Name" })).toBeNull();
    expect(
      screen.getByRole("button", { name: "Post to #backend" }),
    ).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "+ New channel" }));
    expect(
      (screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement)
        .value,
    ).toBe("");
    expect(
      (screen.getByRole("textbox", { name: "Description" }) as HTMLTextAreaElement)
        .value,
    ).toBe("");
  });

  it("creates and selects an empty channel with its description", async () => {
    vi.mocked(api.messages).mockResolvedValue([]);
    vi.mocked(api.createChannel).mockResolvedValue({
      name: "ops.v2",
      description: "Operations coordination",
      most_recent_message_at: null,
    });
    const onChanged = vi.fn().mockResolvedValue(undefined);
    render(
      <MessageBoard
        agents={[]}
        channels={[{ name: "general", description: "", most_recent_message_at: 0 }]}
        refresh={0}
        onSelectAgent={() => {}}
        onChanged={onChanged}
        onError={() => {}}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "+ New channel" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Name" }), {
      target: { value: "bad name" },
    });
    const save = screen.getByRole("button", { name: "Save" });
    expect(save.hasAttribute("disabled")).toBe(true);
    expect(screen.getByRole("alert").textContent).toContain("1–64 ASCII");
    fireEvent.click(save);
    expect(api.createChannel).not.toHaveBeenCalled();

    fireEvent.change(screen.getByRole("textbox", { name: "Name" }), {
      target: { value: "ops.v2" },
    });
    fireEvent.change(screen.getByRole("textbox", { name: "Description" }), {
      target: { value: "x".repeat(501) },
    });
    expect(save.hasAttribute("disabled")).toBe(true);
    expect(screen.getByRole("alert").textContent).toContain("500 characters");
    fireEvent.change(screen.getByRole("textbox", { name: "Description" }), {
      target: { value: "Operations coordination" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(api.createChannel).toHaveBeenCalledWith(
        "ops.v2",
        "Operations coordination",
      ),
    );
    expect(onChanged).toHaveBeenCalled();
    expect(screen.queryByRole("textbox", { name: "Name" })).toBeNull();
    expect(
      screen.getByRole("button", { name: "Post to #ops.v2" }),
    ).toBeTruthy();
    expect(await screen.findByText("No messages in #ops.v2 yet.")).toBeTruthy();
  });

  it("shows a channel description beneath its name", async () => {
    vi.mocked(api.messages).mockResolvedValue([]);
    render(
      <MessageBoard
        agents={[]}
        channels={[
          {
            name: "general",
            description: "Team-wide coordination",
            most_recent_message_at: null,
          },
        ]}
        refresh={0}
        onSelectAgent={() => {}}
        onChanged={async () => {}}
        onError={() => {}}
      />,
    );

    expect(screen.getByText("Team-wide coordination")).toBeTruthy();
  });

  it("keeps separate message drafts for each channel", async () => {
    vi.mocked(api.messages).mockResolvedValue([]);
    render(
      <MessageBoard
        agents={[]}
        channels={[
          { name: "general", description: "", most_recent_message_at: 0 },
          { name: "backend", description: "", most_recent_message_at: 0 },
        ]}
        refresh={0}
        onSelectAgent={() => {}}
        onChanged={async () => {}}
        onError={() => {}}
      />,
    );
    await screen.findByText("No messages in #general yet.");
    fireEvent.change(screen.getByLabelText("Message"), {
      target: { value: "general draft" },
    });
    fireEvent.click(screen.getByRole("button", { name: "#backend" }));
    expect((screen.getByLabelText("Message") as HTMLTextAreaElement).value).toBe("");
    fireEvent.change(screen.getByLabelText("Message"), {
      target: { value: "backend draft" },
    });
    fireEvent.click(screen.getByRole("button", { name: "#general" }));
    expect((screen.getByLabelText("Message") as HTMLTextAreaElement).value).toBe(
      "general draft",
    );
  });

  it("does not show a previous channel while the next channel loads", async () => {
    let resolveGeneral!: (
      messages: Awaited<ReturnType<typeof api.messages>>,
    ) => void;
    vi.mocked(api.messages).mockImplementation((channel) =>
      channel === "general"
        ? new Promise((resolve) => {
            resolveGeneral = resolve;
          })
        : Promise.resolve([
            {
              id: 2,
              channel_name: "backend",
              agent_id: null,
              author: "human",
              body: "Backend message",
              created_at: 0,
            },
          ]),
    );
    render(
      <MessageBoard
        agents={[]}
        channels={[
          { name: "general", description: "", most_recent_message_at: 0 },
          { name: "backend", description: "", most_recent_message_at: 0 },
        ]}
        refresh={0}
        onSelectAgent={() => {}}
        onChanged={async () => {}}
        onError={() => {}}
      />,
    );
    expect(screen.getByText("Loading messages…")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "#backend" }));
    expect(screen.getByText("Loading messages…")).toBeTruthy();
    expect(await screen.findByText("Backend message")).toBeTruthy();
    resolveGeneral([
      {
        id: 1,
        channel_name: "general",
        agent_id: null,
        author: "human",
        body: "Stale general message",
        created_at: 0,
      },
    ]);
    await Promise.resolve();
    expect(screen.queryByText("Stale general message")).toBeNull();
  });

  it("reports duplicate channel creation errors", async () => {
    vi.mocked(api.messages).mockResolvedValue([]);
    vi.mocked(api.createChannel).mockRejectedValue(
      new Error("channel already exists"),
    );
    const onError = vi.fn();
    render(
      <MessageBoard
        agents={[]}
        channels={[
          { name: "general", description: "", most_recent_message_at: null },
        ]}
        refresh={0}
        onSelectAgent={() => {}}
        onChanged={async () => {}}
        onError={onError}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "+ New channel" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Name" }), {
      target: { value: "general" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(onError).toHaveBeenCalledWith(
        expect.objectContaining({ message: "channel already exists" }),
      ),
    );
  });
});

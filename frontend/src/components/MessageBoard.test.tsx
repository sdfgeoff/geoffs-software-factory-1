import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import type { Agent, BoardMessage, MessagePage } from "../types";
import { MessageBoard } from "./MessageBoard";

const messageContentRender = vi.hoisted(() => vi.fn());
const page = (messages: BoardMessage[], has_more = false): MessagePage => ({
  messages,
  has_more,
});

vi.mock("./MessageContent", () => ({
  MessageContent: ({ source }: { source: string }) => {
    messageContentRender(source);
    return <div>{source}</div>;
  },
}));

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
    vi.unstubAllGlobals();
  });

  it("does not rerender a 500-message history while typing", async () => {
    vi.mocked(api.messages).mockResolvedValue(page(
      Array.from({ length: 500 }, (_, index) => ({
        id: index + 1,
        channel_name: "general",
        agent_id: null,
        author: "human",
        body: `History ${index + 1}`,
        created_at: index,
      })),
    ));
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

    expect(await screen.findByText("History 500")).toBeTruthy();
    expect(messageContentRender).toHaveBeenCalledTimes(500);
    messageContentRender.mockClear();
    fireEvent.change(screen.getByRole("textbox", { name: "Message" }), {
      target: { value: "draft" },
    });
    expect(messageContentRender).not.toHaveBeenCalled();
  });

  it("prepends older messages without losing the draft or scroll position", async () => {
    const initial = [
      { id: 51, channel_name: "general", agent_id: null, author: "human", body: "Message 51", created_at: 51 },
      { id: 52, channel_name: "general", agent_id: null, author: "human", body: "Message 52", created_at: 52 },
    ];
    let resolveOlder!: (value: MessagePage) => void;
    vi.mocked(api.messages)
      .mockResolvedValueOnce(page(initial, true))
      .mockReturnValueOnce(new Promise((resolve) => {
        resolveOlder = resolve;
      }));
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      callback(0);
      return 1;
    });
    const { container } = render(
      <MessageBoard
        agents={[]}
        channels={[{ name: "general", description: "", most_recent_message_at: 0 }]}
        refresh={0}
        onSelectAgent={() => {}}
        onChanged={async () => {}}
        onError={() => {}}
      />,
    );
    const loadOlder = await screen.findByRole("button", { name: "Load older messages" });
    const draft = screen.getByRole("textbox", { name: "Message" });
    fireEvent.change(draft, { target: { value: "Unsent draft" } });
    const list = container.querySelector<HTMLElement>(".conversation .message-list")!;
    let height = 1_000;
    Object.defineProperty(list, "scrollHeight", { configurable: true, get: () => height });
    list.scrollTop = 100;

    fireEvent.click(loadOlder);
    expect(api.messages).toHaveBeenLastCalledWith("general", {
      before_id: 51,
      limit: 50,
    });
    height = 1_400;
    await act(async () => {
      resolveOlder(page([
        { id: 49, channel_name: "general", agent_id: null, author: "human", body: "Message 49", created_at: 49 },
        { id: 50, channel_name: "general", agent_id: null, author: "human", body: "Message 50", created_at: 50 },
        initial[0],
      ]));
    });

    expect((draft as HTMLTextAreaElement).value).toBe("Unsent draft");
    expect(list.scrollTop).toBe(500);
    expect(screen.getAllByText("Message 51")).toHaveLength(1);
    expect(screen.queryByRole("button", { name: "Load older messages" })).toBeNull();
  });

  it("opens a renamed agent from a historical message", async () => {
    vi.mocked(api.messages).mockResolvedValue(page([
      {
        id: 1,
        channel_name: "general",
        agent_id: agent.id,
        author: "Former-Name",
        body: "Done",
        created_at: 0,
      },
    ]));
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
    vi.mocked(api.messages).mockResolvedValue(page([
      {
        id: 1,
        channel_name: "general",
        agent_id: null,
        author: agent.name,
        body: "Before the migration",
        created_at: 0,
      },
    ]));
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
    vi.mocked(api.messages).mockResolvedValue(page([]));
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
    vi.mocked(api.messages).mockResolvedValue(page([]));
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
    vi.mocked(api.messages).mockResolvedValue(page([]));
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
    vi.mocked(api.messages).mockResolvedValue(page([]));
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
    vi.mocked(api.messages).mockResolvedValue(page([]));
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

  it("selects channels through the compact navigation control", async () => {
    vi.mocked(api.messages).mockResolvedValue(page([]));
    const onSelectChannel = vi.fn();
    render(
      <MessageBoard
        agents={[]}
        channels={[
          { name: "general", description: "", most_recent_message_at: 0 },
          { name: "backend", description: "", most_recent_message_at: 0 },
        ]}
        refresh={0}
        onSelectChannel={onSelectChannel}
        onSelectAgent={() => {}}
        onChanged={async () => {}}
        onError={() => {}}
      />,
    );

    fireEvent.change(screen.getByRole("combobox", { name: "Channel" }), {
      target: { value: "backend" },
    });
    expect(onSelectChannel).toHaveBeenCalledWith("backend");
    expect(await screen.findByRole("heading", { name: "#backend" })).toBeTruthy();
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
        : Promise.resolve(page([
            {
              id: 2,
              channel_name: "backend",
              agent_id: null,
              author: "human",
              body: "Backend message",
              created_at: 0,
            },
          ])),
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
    resolveGeneral(page([
      {
        id: 1,
        channel_name: "general",
        agent_id: null,
        author: "human",
        body: "Stale general message",
        created_at: 0,
      },
    ]));
    await Promise.resolve();
    expect(screen.queryByText("Stale general message")).toBeNull();
  });

  it("reports duplicate channel creation errors", async () => {
    vi.mocked(api.messages).mockResolvedValue(page([]));
    vi.mocked(api.createChannel).mockRejectedValue(
      new Error("channel already exists"),
    );
    render(
      <MessageBoard
        agents={[]}
        channels={[
          { name: "general", description: "", most_recent_message_at: null },
        ]}
        refresh={0}
        onSelectAgent={() => {}}
        onChanged={async () => {}}
        onError={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "+ New channel" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Name" }), {
      target: { value: "general" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(
      await screen.findByText("Could not create channel: channel already exists"),
    ).toBeTruthy();
    expect((screen.getByRole("textbox", { name: "Name" }) as HTMLInputElement).value)
      .toBe("general");
  });

  it("prevents duplicate posts and preserves a failed draft for retry", async () => {
    vi.mocked(api.messages).mockResolvedValue(page([]));
    let rejectPost: (error: Error) => void = () => {};
    vi.mocked(api.postMessage).mockReturnValueOnce(
      new Promise((_, reject) => {
        rejectPost = reject;
      }),
    );
    render(
      <MessageBoard
        agents={[]}
        channels={[{ name: "general", description: "", most_recent_message_at: 0 }]}
        refresh={0}
        onSelectAgent={() => {}}
        onChanged={async () => {}}
        onError={vi.fn()}
      />,
    );

    const draft = screen.getByRole("textbox", { name: "Message" });
    fireEvent.change(draft, { target: { value: "Please retry me" } });
    fireEvent.click(screen.getByRole("button", { name: "Post to #general" }));
    const posting = screen.getByRole("button", { name: "Posting…" });
    expect((posting as HTMLButtonElement).disabled).toBe(true);
    expect((draft as HTMLTextAreaElement).disabled).toBe(true);
    fireEvent.click(posting);
    expect(api.postMessage).toHaveBeenCalledTimes(1);

    rejectPost(new Error("Network unavailable"));
    expect(await screen.findByText("Could not post: Network unavailable")).toBeTruthy();
    expect((draft as HTMLTextAreaElement).value).toBe("Please retry me");

    vi.mocked(api.postMessage).mockResolvedValueOnce({
      id: 1,
      channel_name: "general",
      agent_id: null,
      author: "human",
      body: "Please retry me",
      created_at: 0,
    });
    fireEvent.click(screen.getByRole("button", { name: "Post to #general" }));
    expect(await screen.findByText("Posted")).toBeTruthy();
    expect((draft as HTMLTextAreaElement).value).toBe("");
  });

  it("keeps a pending post locked after switching channels", async () => {
    vi.mocked(api.messages).mockResolvedValue(page([]));
    let resolvePost!: (
      message: Awaited<ReturnType<typeof api.postMessage>>,
    ) => void;
    vi.mocked(api.postMessage).mockReturnValueOnce(
      new Promise((resolve) => {
        resolvePost = resolve;
      }),
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
        onError={vi.fn()}
      />,
    );

    fireEvent.change(screen.getByRole("textbox", { name: "Message" }), {
      target: { value: "One post only" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Post to #general" }));
    fireEvent.click(screen.getByRole("button", { name: "#backend" }));
    expect((screen.getByRole("textbox", { name: "Message" }) as HTMLTextAreaElement).disabled)
      .toBe(true);
    const backendPost = screen.getByRole("button", { name: "Post to #backend" });
    expect((backendPost as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(backendPost);
    expect(api.postMessage).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole("button", { name: "#general" }));
    resolvePost({
      id: 1,
      channel_name: "general",
      agent_id: null,
      author: "human",
      body: "One post only",
      created_at: 0,
    });
    expect(await screen.findByText("Posted")).toBeTruthy();
    expect((screen.getByRole("textbox", { name: "Message" }) as HTMLTextAreaElement).value)
      .toBe("");
  });
});

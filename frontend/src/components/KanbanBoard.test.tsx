import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import type { Agent, Project, Ticket } from "../types";
import { KanbanBoard } from "./KanbanBoard";

vi.mock("../api", () => ({
  api: {
    clearTicketReservation: vi.fn(),
    createTicket: vi.fn(),
    completeExternalMerge: vi.fn(),
    reserveTickets: vi.fn(),
    returnExternalMerge: vi.fn(),
    setExternalChange: vi.fn(),
    updateTicket: vi.fn(),
  },
}));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  vi.unstubAllGlobals();
});

const ticket: Ticket = {
  id: 7,
  project_id: 1,
  title: "Build the board",
  description: "",
  state: "in_progress",
  created_by_agent_id: null,
  reserved_for_agent_id: null,
  assignee_id: null,
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

const projects: Project[] = [
  {
    id: 1,
    name: "Board",
    path: "/board",
    default_branch: "main",
    completion_workflow: "board_managed",
    archived_at: null,
    updated_at: 0,
  },
  {
    id: 2,
    name: "API",
    path: "/api",
    default_branch: "main",
    completion_workflow: "board_managed",
    archived_at: null,
    updated_at: 0,
  },
];

const baseProps = {
  agents: [] as Agent[],
  projects,
  onChanged: async () => {},
  onError: () => {},
};

function openTicket(title = ticket.title) {
  fireEvent.click(screen.getByText(title).closest("button")!);
}

describe("KanbanBoard", () => {
  it("renders compact cards with the requested hierarchy and metadata", () => {
    render(
      <KanbanBoard
        {...baseProps}
        tickets={[
          {
            ...ticket,
            assignee_id: 3,
            blocker_ids: [2],
            unresolved_blockers: [{ id: 2, title: "Foundation" }],
          },
        ]}
        agents={[{ id: 3, name: "builder" }] as Agent[]}
      />,
    );

    const card = screen.getByText(ticket.title).closest("button")!;
    expect(card.querySelector(".ticket-card-topline")?.textContent).toContain(
      "#7In progress",
    );
    expect(card.querySelector(".ticket-card-meta")?.textContent).toContain(
      "BoardAssigned to builder",
    );
    expect(card.textContent).toContain("Blocked by #2");
    expect(screen.getByTitle("/board").textContent).toBe("Board");
    expect(screen.queryByDisplayValue(ticket.title)).toBeNull();
  });

  it("selects a project for new tickets and filters the global board", async () => {
    vi.mocked(api.createTicket).mockResolvedValue(ticket);
    render(
      <KanbanBoard
        {...baseProps}
        tickets={[
          ticket,
          { ...ticket, id: 8, project_id: 2, title: "API work" },
        ]}
        currentProjectId={2}
      />,
    );

    fireEvent.change(screen.getByLabelText("Filter projects"), {
      target: { value: "2" },
    });
    expect(screen.queryByText("Build the board")).toBeNull();
    fireEvent.change(screen.getByLabelText("New ticket title"), {
      target: { value: "Scoped work" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create ticket" }));
    await waitFor(() =>
      expect(api.createTicket).toHaveBeenCalledWith("Scoped work", 2),
    );
  });

  it("opens and closes the complete ticket editor outside the card", () => {
    render(<KanbanBoard {...baseProps} tickets={[ticket]} />);

    openTicket();
    expect(
      screen.getByRole("heading", { name: ticket.title }),
    ).toBeTruthy();
    expect(screen.getByDisplayValue(ticket.title)).toBeTruthy();
    expect(screen.queryByRole("combobox", { name: "Status" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(
      screen.queryByRole("heading", { name: ticket.title }),
    ).toBeNull();
  });

  it("shows ownership and review details in the panel", () => {
    const reviewTicket: Ticket = {
      ...ticket,
      state: "review",
      created_by_agent_id: 1,
      reserved_for_agent_id: 2,
      assignee_id: 3,
      reviewer_id: 4,
      implementation_branch: "tickets/7-board",
      review_commit: "1234567890abcdef",
      review_target_branch: "main",
      review_feedback: "Tighten tests",
    };
    render(
      <KanbanBoard
        {...baseProps}
        tickets={[reviewTicket]}
        selectedTicketId={ticket.id}
        agents={[
          { id: 1, name: "creator" },
          { id: 2, name: "reserved" },
          { id: 3, name: "assigned" },
          { id: 4, name: "reviewer" },
        ] as Agent[]}
      />,
    );

    const panel = screen.getByRole("heading", { name: ticket.title }).closest(
      ".ticket-detail-panel",
    )!;
    expect(panel.textContent).toContain("Created bycreator");
    expect(panel.textContent).toContain("Reserved forreserved");
    expect(panel.textContent).toContain("Active assigneeassigned");
    expect(panel.textContent).toContain("tickets/7-board");
    expect(panel.textContent).toContain("reviewer");
    expect(panel.textContent).toContain("Tighten tests");
  });

  it("edits ticket fields with a searchable blocker selector", async () => {
    vi.mocked(api.updateTicket).mockResolvedValue(ticket);
    const changed = vi.fn().mockResolvedValue(undefined);
    const blocker = {
      ...ticket,
      id: 2,
      title: "Build the foundation",
      state: "todo" as const,
    };
    const unrelated = { ...ticket, id: 3, title: "Write documentation" };
    render(
      <KanbanBoard
        {...baseProps}
        tickets={[ticket, blocker, unrelated]}
        selectedTicketId={ticket.id}
        onChanged={changed}
      />,
    );

    fireEvent.change(screen.getByLabelText("Title"), {
      target: { value: "Build the routed board" },
    });
    fireEvent.change(screen.getByLabelText("Search tickets"), {
      target: { value: "foundation" },
    });
    expect(
      screen.queryByRole("checkbox", { name: /Write documentation/i }),
    ).toBeNull();
    fireEvent.click(
      screen.getByRole("checkbox", { name: /#2 Build the foundation/i }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() =>
      expect(api.updateTicket).toHaveBeenCalledWith(ticket, {
        title: "Build the routed board",
        description: "",
        assignee_id: null,
        blocker_ids: [2],
      }),
    );
    expect(changed).toHaveBeenCalled();
  });

  it("preserves an edit draft across unchanged data refreshes", () => {
    const { rerender } = render(
      <KanbanBoard
        {...baseProps}
        tickets={[ticket]}
        selectedTicketId={ticket.id}
      />,
    );
    fireEvent.change(screen.getByLabelText("Title"), {
      target: { value: "Unsaved title" },
    });

    rerender(
      <KanbanBoard
        {...baseProps}
        tickets={[{ ...ticket }]}
        selectedTicketId={ticket.id}
      />,
    );

    expect(screen.getByDisplayValue("Unsaved title")).toBeTruthy();
  });

  it("keeps separate drafts while switching tickets", () => {
    const other = { ...ticket, id: 8, title: "Second ticket" };
    render(<KanbanBoard {...baseProps} tickets={[ticket, other]} />);
    openTicket();
    fireEvent.change(screen.getByLabelText("Title"), {
      target: { value: "First draft" },
    });
    fireEvent.click(screen.getByText("Second ticket").closest("button")!);
    fireEvent.change(screen.getByLabelText("Title"), {
      target: { value: "Second draft" },
    });
    fireEvent.click(screen.getByText("Build the board").closest("button")!);

    expect(screen.getByDisplayValue("First draft")).toBeTruthy();
  });

  it("makes replacing an existing reservation an explicit override", async () => {
    vi.mocked(api.reserveTickets).mockResolvedValue([]);
    const changed = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal("confirm", vi.fn(() => true));
    render(
      <KanbanBoard
        {...baseProps}
        tickets={[{ ...ticket, state: "todo", reserved_for_agent_id: 2 }]}
        selectedTicketId={ticket.id}
        agents={[
          { id: 2, name: "reserved" },
          { id: 3, name: "replacement" },
        ] as Agent[]}
        onChanged={changed}
      />,
    );

    fireEvent.change(screen.getByLabelText("Reservation"), {
      target: { value: "3" },
    });
    fireEvent.click(
      screen.getByRole("button", { name: "Override reservation" }),
    );

    await waitFor(() =>
      expect(api.reserveTickets).toHaveBeenCalledWith([ticket.id], 3, true),
    );
    expect(changed).toHaveBeenCalled();
  });

  it("requires confirmation before overriding another agent's reservation", () => {
    vi.stubGlobal("confirm", vi.fn(() => false));
    render(
      <KanbanBoard
        {...baseProps}
        tickets={[{ ...ticket, state: "todo", reserved_for_agent_id: 2 }]}
        selectedTicketId={ticket.id}
        agents={[
          { id: 2, name: "reserved" },
          { id: 3, name: "replacement" },
        ] as Agent[]}
      />,
    );
    fireEvent.change(screen.getByLabelText("Reservation"), {
      target: { value: "3" },
    });
    const override = screen.getByRole("button", { name: "Override reservation" });
    fireEvent.click(override);

    expect(api.reserveTickets).not.toHaveBeenCalled();
    expect(screen.getByText("Reservation unchanged.")).toBeTruthy();
    expect(document.activeElement).toBe(override);
  });

  it("clears a reservation with local outcome feedback", async () => {
    const reserved = { ...ticket, state: "todo" as const, reserved_for_agent_id: 2 };
    vi.mocked(api.clearTicketReservation).mockResolvedValue({
      ...reserved,
      reserved_for_agent_id: null,
      revision: 2,
    });
    render(
      <KanbanBoard
        {...baseProps}
        tickets={[reserved]}
        selectedTicketId={ticket.id}
        agents={[{ id: 2, name: "reserved" }] as Agent[]}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Clear reservation" }));

    await waitFor(() => expect(api.clearTicketReservation).toHaveBeenCalledWith(reserved));
    expect(screen.getByText("Reservation cleared.")).toBeTruthy();
    expect(document.activeElement).toBe(screen.getByLabelText("Reservation"));
  });

  it("prevents duplicate saves and preserves a failed ticket draft", async () => {
    let reject!: (error: Error) => void;
    vi.mocked(api.updateTicket).mockReturnValue(
      new Promise((_, rejectRequest) => {
        reject = rejectRequest;
      }),
    );
    render(
      <KanbanBoard {...baseProps} tickets={[ticket]} selectedTicketId={ticket.id} />,
    );
    fireEvent.change(screen.getByLabelText("Title"), {
      target: { value: "Unfinished edit" },
    });
    const save = screen.getByRole("button", { name: "Save changes" });
    fireEvent.click(save);
    fireEvent.click(save);

    expect(api.updateTicket).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: "Saving…" }).hasAttribute("disabled")).toBe(true);
    expect((screen.getByLabelText("Title") as HTMLInputElement).disabled).toBe(true);
    expect((screen.getByLabelText("Description") as HTMLTextAreaElement).disabled).toBe(true);
    expect((screen.getByLabelText("Active assignee") as HTMLSelectElement).disabled).toBe(true);
    reject(new Error("Ticket changed elsewhere"));
    await waitFor(() => expect(screen.getByText("Ticket changed elsewhere")).toBeTruthy());
    expect(screen.getByDisplayValue("Unfinished edit")).toBeTruthy();
  });

  it("prevents duplicate ticket creation and preserves a failed title", async () => {
    let reject!: (error: Error) => void;
    vi.mocked(api.createTicket).mockReturnValue(
      new Promise((_, rejectRequest) => {
        reject = rejectRequest;
      }),
    );
    render(<KanbanBoard {...baseProps} tickets={[]} currentProjectId={1} />);
    fireEvent.change(screen.getByLabelText("New ticket title"), {
      target: { value: "Draft ticket" },
    });
    const create = screen.getByRole("button", { name: "Create ticket" });
    fireEvent.click(create);
    fireEvent.click(create);

    expect(api.createTicket).toHaveBeenCalledTimes(1);
    expect((screen.getByLabelText("New ticket title") as HTMLInputElement).disabled).toBe(true);
    expect((screen.getByLabelText("Ticket project") as HTMLSelectElement).disabled).toBe(true);
    reject(new Error("Could not create"));
    await waitFor(() => expect(screen.getByText("Could not create")).toBeTruthy());
    expect(screen.getByDisplayValue("Draft ticket")).toBeTruthy();
  });

  it("locks a ticket draft until a deferred create succeeds", async () => {
    let resolveCreate!: (created: Ticket) => void;
    vi.mocked(api.createTicket).mockReturnValueOnce(
      new Promise((resolve) => {
        resolveCreate = resolve;
      }),
    );
    render(<KanbanBoard {...baseProps} tickets={[]} currentProjectId={1} />);
    const title = screen.getByLabelText("New ticket title") as HTMLInputElement;
    fireEvent.change(title, { target: { value: "Submitted title" } });
    fireEvent.click(screen.getByRole("button", { name: "Create ticket" }));

    expect(title.disabled).toBe(true);
    resolveCreate({ ...ticket, title: "Submitted title" });
    expect(await screen.findByText("Ticket created.")).toBeTruthy();
    expect(title.value).toBe("");
  });

  it("offers only valid state-specific actions", async () => {
    vi.stubGlobal("confirm", vi.fn(() => true));
    vi.mocked(api.updateTicket).mockResolvedValue(ticket);
    render(
      <KanbanBoard
        {...baseProps}
        tickets={[ticket]}
        selectedTicketId={ticket.id}
      />,
    );

    expect(screen.getByRole("button", { name: "Move to todo" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Cancel ticket" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Start work" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Move to todo" }));

    await waitFor(() =>
      expect(api.updateTicket).toHaveBeenCalledWith(ticket, { state: "todo" }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Cancel ticket" }));
    await waitFor(() =>
      expect(api.updateTicket).toHaveBeenCalledWith(ticket, {
        state: "cancelled",
      }),
    );
  });

  it("navigates compact board statuses without removing desktop columns", () => {
    const review = { ...ticket, id: 8, state: "review" as const };
    const { container } = render(
      <KanbanBoard {...baseProps} tickets={[ticket, review]} />,
    );

    const status = screen.getByRole("combobox", { name: "Ticket status" });
    expect(container.querySelector(".column-todo.mobile-active")).toBeTruthy();
    expect(container.querySelectorAll(".column")).toHaveLength(4);
    fireEvent.change(status, { target: { value: "review" } });
    expect(container.querySelector(".column-review.mobile-active")).toBeTruthy();
    expect(container.querySelector(".column-todo.mobile-active")).toBeNull();

    fireEvent.click(screen.getByLabelText("Show cancelled tickets"));
    fireEvent.change(status, { target: { value: "cancelled" } });
    fireEvent.click(screen.getByLabelText("Show cancelled tickets"));
    expect((status as HTMLSelectElement).value).toBe("todo");
  });

  it("shows the external lane only when needed and labels the workflow", () => {
    const { container, rerender } = render(
      <KanbanBoard {...baseProps} tickets={[ticket]} />,
    );

    expect(screen.queryByRole("heading", { name: /Awaiting external merge/ })).toBeNull();
    expect(container.querySelectorAll(".column")).toHaveLength(4);

    const external = {
      ...ticket,
      id: 9,
      title: "Publish externally",
      state: "awaiting_external_merge" as const,
      review_completion_workflow: "external" as const,
    };
    rerender(<KanbanBoard {...baseProps} tickets={[ticket, external]} />);

    expect(screen.getByRole("heading", { name: /Awaiting external merge/ })).toBeTruthy();
    expect(screen.getByText("Publish externally").closest("button")?.textContent).toContain(
      "External merge",
    );
    expect(container.querySelector(".kanban-with-external")).toBeTruthy();
  });

  it("explains an external review and exposes safe reviewed values", async () => {
    const clipboard = { writeText: vi.fn().mockResolvedValue(undefined) };
    vi.stubGlobal("navigator", { ...navigator, clipboard });
    const external = {
      ...ticket,
      state: "review" as const,
      review_completion_workflow: "external" as const,
      implementation_branch: "tickets/7-external",
      review_commit: "1234567890abcdef",
      review_target_branch: "main",
      external_change_url: "javascript:alert(1)",
    };
    render(
      <KanbanBoard
        {...baseProps}
        tickets={[external]}
        selectedTicketId={external.id}
      />,
    );

    expect(screen.getByText(/Reviewer action: Approve for external merge/)).toBeTruthy();
    expect(screen.getByText(/does not merge or change the project branch/)).toBeTruthy();
    expect(screen.getByText("javascript:alert(1) (invalid URL)")).toBeTruthy();
    expect(screen.queryByRole("link", { name: "javascript:alert(1)" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Copy branch" }));
    await waitFor(() =>
      expect(clipboard.writeText).toHaveBeenCalledWith("tickets/7-external"),
    );
    fireEvent.change(screen.getByLabelText("External change URL"), {
      target: { value: "https://example.test/change/7" },
    });
    const link = screen.getByRole("link", { name: "Open external change" });
    expect(link.getAttribute("rel")).toBe("noopener noreferrer");
  });

  it("runs external completion actions and preserves failed input", async () => {
    const external = {
      ...ticket,
      state: "awaiting_external_merge" as const,
      review_completion_workflow: "external" as const,
      implementation_branch: "tickets/7-external",
      review_commit: "1234567890abcdef",
      review_target_branch: "main",
    };
    let rejectCompletion!: (error: Error) => void;
    vi.mocked(api.completeExternalMerge).mockReturnValue(
      new Promise((_, reject) => {
        rejectCompletion = reject;
      }),
    );
    vi.stubGlobal("confirm", vi.fn(() => true));
    render(
      <KanbanBoard
        {...baseProps}
        tickets={[external]}
        selectedTicketId={external.id}
      />,
    );

    fireEvent.change(screen.getByLabelText("Completion confirmation"), {
      target: { value: "Merged and deployed" },
    });
    fireEvent.change(screen.getByLabelText("Final external reference (optional)"), {
      target: { value: " release-42 " },
    });
    fireEvent.click(screen.getByRole("button", { name: "Mark externally merged" }));

    expect(api.completeExternalMerge).toHaveBeenCalledWith(
      external,
      "Merged and deployed",
      "release-42",
    );
    expect((screen.getByLabelText("Completion confirmation") as HTMLTextAreaElement).disabled).toBe(true);
    expect((screen.getByLabelText("Title") as HTMLInputElement).disabled).toBe(true);
    rejectCompletion(new Error("Still open upstream"));
    await screen.findByText("Still open upstream");
    expect(screen.getByDisplayValue("Merged and deployed")).toBeTruthy();
    expect(
      (screen.getByLabelText("Final external reference (optional)") as HTMLInputElement).value,
    ).toBe(" release-42 ");
  });

  it("saves an external URL and can return approved work", async () => {
    const external = {
      ...ticket,
      state: "awaiting_external_merge" as const,
      review_completion_workflow: "external" as const,
      revision: 4,
    };
    vi.mocked(api.setExternalChange).mockResolvedValue({
      ...external,
      external_change_url: "https://example.test/change/7",
      revision: 5,
    });
    vi.mocked(api.returnExternalMerge).mockResolvedValue({
      ...external,
      state: "in_progress",
      revision: 6,
    });
    vi.stubGlobal("confirm", vi.fn(() => true));
    render(
      <KanbanBoard
        {...baseProps}
        tickets={[external]}
        selectedTicketId={external.id}
      />,
    );

    fireEvent.change(screen.getByLabelText("External change URL"), {
      target: { value: "https://example.test/change/7" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save URL" }));
    await waitFor(() => expect(api.setExternalChange).toHaveBeenCalledWith(external, "https://example.test/change/7"));
    await screen.findByText("External change saved.");
    fireEvent.change(screen.getByLabelText("Reason for returning to work"), {
      target: { value: " Fix review issue " },
    });
    fireEvent.click(screen.getByRole("button", { name: "Return to work" }));
    await waitFor(() =>
      expect(api.returnExternalMerge).toHaveBeenCalledWith(
        { ...external, revision: 5 },
        "Fix review issue",
      ),
    );
  });

  it("confirms before cancelling a ticket", () => {
    vi.stubGlobal("confirm", vi.fn(() => false));
    render(
      <KanbanBoard
        {...baseProps}
        tickets={[ticket]}
        selectedTicketId={ticket.id}
      />,
    );

    const cancel = screen.getByRole("button", { name: "Cancel ticket" });
    fireEvent.click(cancel);

    expect(confirm).toHaveBeenCalledWith(
      `Cancel ticket #${ticket.id}? Cancelled tickets are terminal and cannot be reopened.`,
    );
    expect(api.updateTicket).not.toHaveBeenCalled();
    expect(screen.getByText("Cancellation abandoned; ticket unchanged.")).toBeTruthy();
    expect(document.activeElement).toBe(cancel);
  });

  it("hides cancelled tickets unless requested or directly selected", () => {
    const cancelled = {
      ...ticket,
      state: "cancelled" as const,
      title: "Duplicate work",
    };
    const { rerender } = render(
      <KanbanBoard {...baseProps} tickets={[cancelled]} />,
    );

    expect(screen.queryByText("Duplicate work")).toBeNull();
    fireEvent.click(screen.getByLabelText("Show cancelled tickets"));
    expect(screen.getByText("Duplicate work")).toBeTruthy();
    expect(
      screen.getByRole("heading", { name: /cancelled/i }).parentElement
        ?.className,
    ).toContain("column-cancelled");

    rerender(
      <KanbanBoard
        {...baseProps}
        tickets={[cancelled]}
        selectedTicketId={cancelled.id}
      />,
    );
    expect(
      screen.getByRole("heading", { name: "Duplicate work" }),
    ).toBeTruthy();
  });
});

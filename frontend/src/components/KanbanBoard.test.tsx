import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import type { Agent, Project, Ticket } from "../types";
import { KanbanBoard } from "./KanbanBoard";

vi.mock("../api", () => ({
  api: {
    createTicket: vi.fn(),
    reserveTickets: vi.fn(),
    updateTicket: vi.fn(),
  },
}));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
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
    archived_at: null,
    updated_at: 0,
  },
  {
    id: 2,
    name: "API",
    path: "/api",
    default_branch: "main",
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

  it("makes replacing an existing reservation an explicit override", async () => {
    vi.mocked(api.reserveTickets).mockResolvedValue([]);
    const changed = vi.fn().mockResolvedValue(undefined);
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

  it("offers only valid state-specific actions", async () => {
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

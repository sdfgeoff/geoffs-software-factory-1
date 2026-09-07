import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import { KanbanBoard } from "./KanbanBoard";
import type { Agent, Project, Ticket } from "../types";

vi.mock("../api", () => ({
  api: {
    createTicket: vi.fn(),
    reserveTickets: vi.fn(),
    updateTicket: vi.fn(),
  },
}));

afterEach(cleanup);

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
  { id: 1, name: "Board", path: "/board", default_branch: "main", archived_at: null, updated_at: 0 },
  { id: 2, name: "API", path: "/api", default_branch: "main", archived_at: null, updated_at: 0 },
];

describe("KanbanBoard", () => {
  it("places tickets in their state column", () => {
    render(
      <KanbanBoard
        tickets={[ticket]}
        agents={[]}
        onChanged={async () => {}}
        onError={() => {}}
      />,
    );

    expect(
      screen.getByRole("heading", { name: /in progress/i }).parentElement
        ?.textContent,
    ).toContain("Build the board");
    expect(screen.getByRole("heading", { name: /review/i })).toBeTruthy();
  });

  it("selects a project for new tickets and filters the global board", async () => {
    vi.mocked(api.createTicket).mockResolvedValue(ticket);
    render(
      <KanbanBoard
        tickets={[ticket, { ...ticket, id: 8, project_id: 2, title: "API work" }]}
        agents={[]}
        projects={projects}
        currentProjectId={2}
        onChanged={async () => {}}
        onError={() => {}}
      />,
    );

    expect(screen.getAllByText("Board").length).toBeGreaterThan(0);
    expect(screen.getAllByText("API").length).toBeGreaterThan(0);
    fireEvent.change(screen.getByLabelText("Filter projects"), { target: { value: "2" } });
    expect(screen.queryByText("Build the board")).toBeNull();
    fireEvent.change(screen.getByLabelText("New ticket title"), { target: { value: "Scoped work" } });
    fireEvent.click(screen.getByRole("button", { name: "Create ticket" }));
    await waitFor(() => expect(api.createTicket).toHaveBeenCalledWith("Scoped work", 2));
  });

  it("shows the implementation branch for a review", () => {
    render(
      <KanbanBoard
        tickets={[
          {
            ...ticket,
            state: "review",
            implementation_branch: "tickets/7-board",
            review_commit: "1234567890abcdef",
            review_target_branch: "main",
          },
        ]}
        agents={[]}
        onChanged={async () => {}}
        onError={() => {}}
      />,
    );

    expect(screen.getByText("tickets/7-board")).toBeTruthy();
  });

  it("shows unresolved blocker IDs and titles", () => {
    render(
      <KanbanBoard
        tickets={[
          {
            ...ticket,
            state: "todo",
            blocker_ids: [2],
            unresolved_blockers: [{ id: 2, title: "Build the foundation" }],
          },
        ]}
        agents={[]}
        onChanged={async () => {}}
        onError={() => {}}
      />,
    );

    expect(screen.getByText("Blocked by #2 Build the foundation")).toBeTruthy();
  });

  it("shows creator, reservation, and active assignment separately", () => {
    render(
      <KanbanBoard
        tickets={[
          {
            ...ticket,
            created_by_agent_id: 1,
            reserved_for_agent_id: 2,
            assignee_id: 3,
          },
        ]}
        agents={[
          { id: 1, name: "creator" },
          { id: 2, name: "reserved" },
          { id: 3, name: "assigned" },
        ] as Agent[]}
        onChanged={async () => {}}
        onError={() => {}}
      />,
    );

    const ownership = screen.getByText("Created by").parentElement;
    expect(ownership?.textContent).toContain("creator");
    expect(ownership?.textContent).toContain("Reserved forreserved");
    expect(ownership?.textContent).toContain("Active assigneeassigned");
  });

  it("makes replacing an existing reservation an explicit override", async () => {
    vi.mocked(api.reserveTickets).mockResolvedValue([]);
    const changed = vi.fn().mockResolvedValue(undefined);
    render(
      <KanbanBoard
        tickets={[{ ...ticket, state: "todo", reserved_for_agent_id: 2 }]}
        agents={[
          { id: 2, name: "reserved" },
          { id: 3, name: "replacement" },
        ] as Agent[]}
        onChanged={changed}
        onError={() => {}}
      />,
    );

    fireEvent.change(screen.getByLabelText("Reservation"), {
      target: { value: "3" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Override reservation" }));

    await waitFor(() =>
      expect(api.reserveTickets).toHaveBeenCalledWith([ticket.id], 3, true),
    );
    expect(changed).toHaveBeenCalled();
  });

  it("hides cancelled tickets by default and shows their distinct column on request", () => {
    render(
      <KanbanBoard
        tickets={[{ ...ticket, state: "cancelled", title: "Duplicate work" }]}
        agents={[]}
        onChanged={async () => {}}
        onError={() => {}}
      />,
    );

    expect(screen.queryByText("Duplicate work")).toBeNull();
    fireEvent.click(screen.getByLabelText("Show cancelled tickets"));
    expect(screen.getByText("Duplicate work")).toBeTruthy();
    expect(
      screen.getByRole("heading", { name: /cancelled/i }).parentElement?.className,
    ).toContain("column-cancelled");
  });

  it("cancels todo and in-progress tickets with an explicit action", async () => {
    vi.mocked(api.updateTicket).mockResolvedValue({
      ...ticket,
      state: "cancelled",
    });
    const changed = vi.fn().mockResolvedValue(undefined);
    render(
      <KanbanBoard
        tickets={[ticket]}
        agents={[]}
        onChanged={changed}
        onError={() => {}}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Cancel ticket" }));

    await waitFor(() =>
      expect(api.updateTicket).toHaveBeenCalledWith(ticket, {
        state: "cancelled",
      }),
    );
    expect(changed).toHaveBeenCalled();
  });
});

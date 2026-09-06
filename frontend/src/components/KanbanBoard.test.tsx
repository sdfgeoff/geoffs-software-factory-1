import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "../api";
import { KanbanBoard } from "./KanbanBoard";
import type { Agent, Ticket } from "../types";

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
  title: "Build the board",
  description: "",
  state: "in_progress",
  created_by_agent_id: null,
  reserved_for_agent_id: null,
  assignee_id: null,
  implementation_branch: null,
  review_commit: null,
  review_target_branch: null,
  reviewer_id: null,
  review_feedback: null,
  blocker_ids: [],
  unresolved_blockers: [],
  revision: 1,
  created_at: 0,
  updated_at: 0,
};

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
});

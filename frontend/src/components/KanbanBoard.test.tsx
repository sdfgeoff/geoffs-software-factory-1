import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { KanbanBoard } from "./KanbanBoard";
import type { Ticket } from "../types";

const ticket: Ticket = {
  id: 7,
  title: "Build the board",
  description: "",
  state: "in_progress",
  assignee_id: null,
  implementation_branch: null,
  review_commit: null,
  review_target_branch: null,
  reviewer_id: null,
  review_feedback: null,
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
});

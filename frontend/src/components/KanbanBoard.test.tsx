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
  });
});

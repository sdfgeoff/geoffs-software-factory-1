import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { api } from "../api";
import type { Project } from "../types";
import { ProjectPicker } from "./ProjectPicker";

vi.mock("../api", () => ({
  api: {
    archiveProject: vi.fn(),
    registerProject: vi.fn(),
    setProject: vi.fn(),
  },
}));

afterEach(cleanup);

it("selects registers and archives project roots", async () => {
  const projects: Project[] = [
    { id: 1, name: "Board", path: "/board", default_branch: "main", archived_at: null, updated_at: 0 },
    { id: 2, name: "API", path: "/api", default_branch: "trunk", archived_at: null, updated_at: 0 },
  ];
  vi.mocked(api.setProject).mockResolvedValue(projects[1]);
  vi.mocked(api.registerProject).mockResolvedValue(projects[1]);
  vi.mocked(api.archiveProject).mockResolvedValue({ ...projects[1], archived_at: 1 });
  const changed = vi.fn().mockResolvedValue(undefined);
  render(
    <ProjectPicker project={projects[0]} projects={projects} onChanged={changed} onError={() => {}} />,
  );

  fireEvent.change(screen.getByLabelText("Active project"), { target: { value: "2" } });
  await waitFor(() => expect(api.setProject).toHaveBeenCalledWith("/api"));
  fireEvent.change(screen.getByLabelText("Project name"), { target: { value: "New" } });
  fireEvent.change(screen.getByLabelText("Git root"), { target: { value: "/new" } });
  fireEvent.change(screen.getByLabelText("Default branch"), { target: { value: "develop" } });
  fireEvent.click(screen.getByRole("button", { name: "Register project" }));
  await waitFor(() => expect(api.registerProject).toHaveBeenCalledWith("New", "/new", "develop"));
  fireEvent.click(screen.getAllByRole("button", { name: "Archive" })[1]);
  await waitFor(() => expect(api.archiveProject).toHaveBeenCalledWith(2));
});

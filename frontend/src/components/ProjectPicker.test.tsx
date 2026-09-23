import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { api } from "../api";
import type { Project } from "../types";
import { ProjectPicker } from "./ProjectPicker";

vi.mock("../api", () => ({
  api: {
    archiveProject: vi.fn(),
    registerProject: vi.fn(),
    setProjectCompletionWorkflow: vi.fn(),
  },
}));

const projects: Project[] = [
  { id: 1, name: "Board", path: "/board", default_branch: "main", completion_workflow: "board_managed", archived_at: null, updated_at: 0 },
  { id: 2, name: "API", path: "/api", default_branch: "trunk", completion_workflow: "external", archived_at: null, updated_at: 0 },
];

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  vi.unstubAllGlobals();
});

it("manages projects without a global project selector", async () => {
  vi.mocked(api.registerProject).mockResolvedValue(projects[1]);
  vi.mocked(api.setProjectCompletionWorkflow).mockResolvedValue({
    ...projects[0], completion_workflow: "external",
  });
  vi.mocked(api.archiveProject).mockResolvedValue({ ...projects[1], archived_at: 1 });
  vi.stubGlobal("confirm", vi.fn(() => true));
  const changed = vi.fn().mockResolvedValue(undefined);
  render(<ProjectPicker projects={projects} onChanged={changed} />);

  expect(screen.getByRole("heading", { name: "Projects" })).toBeTruthy();
  expect(screen.queryByLabelText("Active project")).toBeNull();
  fireEvent.change(screen.getByLabelText("Project name"), { target: { value: "New" } });
  fireEvent.change(screen.getByLabelText("Git root"), { target: { value: "/new" } });
  fireEvent.change(screen.getByLabelText("Default branch"), { target: { value: "develop" } });
  fireEvent.click(screen.getByRole("button", { name: "Register project" }));
  await waitFor(() => expect(api.registerProject).toHaveBeenCalledWith(
    "New", "/new", "develop", "board_managed",
  ));

  fireEvent.change(screen.getByRole("combobox", { name: "Board completion workflow" }), {
    target: { value: "external" },
  });
  await waitFor(() => expect(api.setProjectCompletionWorkflow).toHaveBeenCalledWith(1, "external"));
  fireEvent.click(screen.getAllByRole("button", { name: "Archive" })[1]);
  await waitFor(() => expect(api.archiveProject).toHaveBeenCalledWith(2));
  expect(document.activeElement).toBe(screen.getByRole("heading", { name: "Projects" }));
});

it("preserves registration drafts across refreshed project props", () => {
  const props = { projects, onChanged: vi.fn().mockResolvedValue(undefined) };
  const { rerender } = render(<ProjectPicker {...props} />);
  fireEvent.change(screen.getByLabelText("Git root"), { target: { value: "/draft" } });
  rerender(<ProjectPicker {...props} projects={[...projects]} />);
  expect((screen.getByLabelText("Git root") as HTMLInputElement).value).toBe("/draft");
});

it("preserves a failed draft and prevents duplicate registration", async () => {
  let reject!: (error: Error) => void;
  vi.mocked(api.registerProject).mockReturnValue(new Promise((_, rejectRequest) => {
    reject = rejectRequest;
  }));
  render(<ProjectPicker projects={[]} onChanged={vi.fn().mockResolvedValue(undefined)} />);
  fireEvent.change(screen.getByLabelText("Project name"), { target: { value: "New" } });
  fireEvent.change(screen.getByLabelText("Git root"), { target: { value: "/new" } });
  const submit = screen.getByRole("button", { name: "Register project" });
  fireEvent.click(submit);
  fireEvent.click(submit);
  expect(api.registerProject).toHaveBeenCalledTimes(1);
  expect((screen.getByLabelText("Git root") as HTMLInputElement).disabled).toBe(true);
  reject(new Error("Path is not a Git repository"));
  await waitFor(() => expect(screen.getByRole("status").textContent).toBe("Path is not a Git repository"));
  expect((screen.getByLabelText("Git root") as HTMLInputElement).value).toBe("/new");
});

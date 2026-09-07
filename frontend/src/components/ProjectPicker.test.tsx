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

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  vi.unstubAllGlobals();
});

it("selects registers and archives project roots", async () => {
  const projects: Project[] = [
    { id: 1, name: "Board", path: "/board", default_branch: "main", archived_at: null, updated_at: 0 },
    { id: 2, name: "API", path: "/api", default_branch: "trunk", archived_at: null, updated_at: 0 },
  ];
  vi.mocked(api.setProject).mockResolvedValue(projects[1]);
  vi.mocked(api.registerProject).mockResolvedValue(projects[1]);
  vi.mocked(api.archiveProject).mockResolvedValue({ ...projects[1], archived_at: 1 });
  vi.stubGlobal("confirm", vi.fn(() => true));
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
  expect(document.activeElement).toBe(screen.getByText("Board", { selector: "summary" }));
});

it("preserves a path draft when an equivalent project is refreshed", () => {
  const project: Project = {
    id: 1,
    name: "Board",
    path: "/board",
    default_branch: "main",
    archived_at: null,
    updated_at: 0,
  };
  const props = { projects: [project], onChanged: vi.fn(), onError: vi.fn() };
  const { rerender } = render(<ProjectPicker {...props} project={project} />);

  fireEvent.change(screen.getByLabelText("Git root"), { target: { value: "/draft" } });
  rerender(<ProjectPicker {...props} project={{ ...project }} />);

  expect((screen.getByLabelText("Git root") as HTMLInputElement).value).toBe("/draft");
});

it("confirms archival and keeps the action local when the backend refuses", async () => {
  const project: Project = {
    id: 1,
    name: "Board",
    path: "/board",
    default_branch: "main",
    archived_at: null,
    updated_at: 0,
  };
  const confirm = vi.fn(() => false);
  vi.stubGlobal("confirm", confirm);
  render(
    <ProjectPicker
      project={project}
      projects={[project]}
      onChanged={vi.fn()}
      onError={vi.fn()}
    />,
  );
  const archive = screen.getByRole("button", { name: "Archive" });

  fireEvent.click(archive);
  expect(confirm).toHaveBeenCalledWith(expect.stringContaining("active tickets"));
  expect(api.archiveProject).not.toHaveBeenCalled();
  expect(screen.getByRole("status").textContent).toBe("Archive cancelled.");

  confirm.mockReturnValue(true);
  vi.mocked(api.archiveProject).mockRejectedValue(new Error("Project has active work"));
  fireEvent.click(archive);
  await waitFor(() =>
    expect(screen.getByRole("status").textContent).toBe("Project has active work"),
  );
  expect(document.activeElement).toBe(archive);
});

it("prevents duplicate registration and preserves a failed draft", async () => {
  let reject!: (error: Error) => void;
  vi.mocked(api.registerProject).mockReturnValue(
    new Promise((_, rejectRequest) => {
      reject = rejectRequest;
    }),
  );
  render(
    <ProjectPicker project={null} projects={[]} onChanged={vi.fn()} onError={vi.fn()} />,
  );
  fireEvent.change(screen.getByLabelText("Project name"), { target: { value: "New" } });
  fireEvent.change(screen.getByLabelText("Git root"), { target: { value: "/new" } });
  const submit = screen.getByRole("button", { name: "Register project" });
  fireEvent.click(submit);
  fireEvent.click(submit);

  expect(api.registerProject).toHaveBeenCalledTimes(1);
  expect(screen.getByRole("button", { name: "Registering…" }).hasAttribute("disabled")).toBe(true);
  reject(new Error("Path is not a Git repository"));
  await waitFor(() =>
    expect(screen.getByRole("status").textContent).toBe("Path is not a Git repository"),
  );
  expect((screen.getByLabelText("Project name") as HTMLInputElement).value).toBe("New");
  expect((screen.getByLabelText("Git root") as HTMLInputElement).value).toBe("/new");
});

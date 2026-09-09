import { useEffect, useRef, useState, type FormEvent } from "react";
import { api } from "../api";
import type { Project, ProjectCompletionWorkflow } from "../types";

interface Props {
  project: Project | null;
  projects: Project[];
  onChanged: () => Promise<void>;
  onError: (error: unknown) => void;
}

export function ProjectPicker({ project, projects, onChanged }: Props) {
  const [path, setPath] = useState(project?.path ?? "");
  const [name, setName] = useState("");
  const [defaultBranch, setDefaultBranch] = useState("main");
  const [completionWorkflow, setCompletionWorkflow] =
    useState<ProjectCompletionWorkflow>("board_managed");
  const [selecting, setSelecting] = useState(false);
  const [registering, setRegistering] = useState(false);
  const [archivingId, setArchivingId] = useState<number | null>(null);
  const [changingWorkflowId, setChangingWorkflowId] = useState<number | null>(null);
  const [selectionStatus, setSelectionStatus] = useState("");
  const [registerStatus, setRegisterStatus] = useState("");
  const [archiveStatus, setArchiveStatus] = useState("");
  const summaryRef = useRef<HTMLElement>(null);
  const projectBusy =
    selecting || registering || archivingId !== null || changingWorkflowId !== null;
  useEffect(() => setPath(project?.path ?? ""), [project?.path]);
  async function register(event: FormEvent) {
    event.preventDefault();
    if (projectBusy) return;
    setRegistering(true);
    setRegisterStatus("Registering…");
    try {
      await api.registerProject(name, path, defaultBranch, completionWorkflow);
      setName("");
      await onChanged();
      setRegisterStatus("Project registered.");
    } catch (error) {
      setRegisterStatus(error instanceof Error ? error.message : "Could not register project");
    } finally {
      setRegistering(false);
    }
  }
  async function changeWorkflow(
    item: Project,
    workflow: ProjectCompletionWorkflow,
  ) {
    if (projectBusy) return;
    setChangingWorkflowId(item.id);
    setArchiveStatus(`Updating ${item.name} workflow…`);
    try {
      await api.setProjectCompletionWorkflow(item.id, workflow);
      await onChanged();
      setArchiveStatus(`${item.name} workflow updated.`);
    } catch (error) {
      setArchiveStatus(error instanceof Error ? error.message : "Could not update workflow");
    } finally {
      setChangingWorkflowId(null);
    }
  }
  async function archive(item: Project, button: HTMLButtonElement) {
    if (projectBusy) return;
    const confirmed = window.confirm(
      `Archive ${item.name}? The board may refuse while the project has active tickets, reviews, or agent work.`,
    );
    if (!confirmed) {
      setArchiveStatus("Archive cancelled.");
      button.focus();
      return;
    }
    setArchivingId(item.id);
    setArchiveStatus(`Archiving ${item.name}…`);
    try {
      await api.archiveProject(item.id);
      await onChanged();
      setArchiveStatus(`${item.name} archived.`);
      summaryRef.current?.focus();
    } catch (error) {
      setArchiveStatus(error instanceof Error ? error.message : "Could not archive project");
      button.focus();
    } finally {
      setArchivingId(null);
    }
  }
  async function select(next: Project) {
    if (projectBusy) return;
    setSelecting(true);
    setSelectionStatus("Selecting…");
    setPath(next.path);
    try {
      await api.setProject(next.path);
      await onChanged();
      setSelectionStatus(`${next.name} selected.`);
    } catch (error) {
      setSelectionStatus(error instanceof Error ? error.message : "Could not select project");
    } finally {
      setSelecting(false);
    }
  }
  return (
    <details className="project-picker">
      <summary ref={summaryRef}>{project?.name ?? "Choose project"}</summary>
      <label htmlFor="active-project">
        Active project
        <select
          id="active-project"
          value={project?.id ?? ""}
          onChange={(event) => {
            const selected = projects.find(
              (item) => item.id === Number(event.target.value),
            );
            if (selected) {
              void select(selected);
            }
          }}
          disabled={projectBusy}
        >
          <option value="" disabled>Select a project</option>
          {projects.filter((item) => !item.archived_at).map((item) => (
            <option key={item.id} value={item.id}>{item.name}</option>
          ))}
        </select>
      </label>
      {selectionStatus && <p role="status" aria-live="polite">{selectionStatus}</p>}
      <form onSubmit={register}>
        <label>
          Project name
          <input
            value={name}
            onChange={(event) => setName(event.target.value)}
            disabled={projectBusy}
            required
          />
        </label>
        <label>
          Git root
          <input
            value={path}
            onChange={(event) => setPath(event.target.value)}
            placeholder="/path/to/git/repository"
            disabled={projectBusy}
            required
          />
        </label>
        <label>
          Default branch
          <input
            value={defaultBranch}
            onChange={(event) => setDefaultBranch(event.target.value)}
            disabled={projectBusy}
            required
          />
        </label>
        <label>
          Completion workflow
          <select
            value={completionWorkflow}
            onChange={(event) =>
              setCompletionWorkflow(event.target.value as ProjectCompletionWorkflow)
            }
            disabled={projectBusy}
          >
            <option value="board_managed">Board merges approved reviews</option>
            <option value="external">Merge externally after approval</option>
          </select>
        </label>
        <button type="submit" disabled={projectBusy}>
          {registering ? "Registering…" : "Register project"}
        </button>
        {registerStatus && <p role="status" aria-live="polite">{registerStatus}</p>}
      </form>
      <ul>
        {projects.map((item) => (
          <li key={item.id}>
            <span title={`${item.name} — ${item.path}`}>
              {item.name} — {item.path}
            </span>
            {!item.archived_at && (
              <select
                aria-label={`${item.name} completion workflow`}
                value={item.completion_workflow}
                disabled={projectBusy}
                onChange={(event) =>
                  void changeWorkflow(
                    item,
                    event.target.value as ProjectCompletionWorkflow,
                  )
                }
              >
                <option value="board_managed">Board merge</option>
                <option value="external">External merge</option>
              </select>
            )}
            {item.archived_at ? (
              <small>Archived</small>
            ) : (
              <button
                type="button"
                className="ghost"
                disabled={projectBusy}
                onClick={(event) => void archive(item, event.currentTarget)}
              >
                {archivingId === item.id ? "Archiving…" : "Archive"}
              </button>
            )}
          </li>
        ))}
      </ul>
      {archiveStatus && <p role="status" aria-live="polite">{archiveStatus}</p>}
    </details>
  );
}

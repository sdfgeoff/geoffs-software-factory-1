import { useEffect, useRef, useState, type FormEvent } from "react";
import { api } from "../api";
import type { Project } from "../types";

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
  const [selecting, setSelecting] = useState(false);
  const [registering, setRegistering] = useState(false);
  const [archivingId, setArchivingId] = useState<number | null>(null);
  const [selectionStatus, setSelectionStatus] = useState("");
  const [registerStatus, setRegisterStatus] = useState("");
  const [archiveStatus, setArchiveStatus] = useState("");
  const summaryRef = useRef<HTMLElement>(null);
  useEffect(() => setPath(project?.path ?? ""), [project?.path]);
  async function register(event: FormEvent) {
    event.preventDefault();
    if (registering) return;
    setRegistering(true);
    setRegisterStatus("Registering…");
    try {
      await api.registerProject(name, path, defaultBranch);
      setName("");
      await onChanged();
      setRegisterStatus("Project registered.");
    } catch (error) {
      setRegisterStatus(error instanceof Error ? error.message : "Could not register project");
    } finally {
      setRegistering(false);
    }
  }
  async function archive(item: Project, button: HTMLButtonElement) {
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
    if (selecting) return;
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
          disabled={selecting}
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
          <input value={name} onChange={(event) => setName(event.target.value)} required />
        </label>
        <label>
          Git root
          <input
            value={path}
            onChange={(event) => setPath(event.target.value)}
            placeholder="/path/to/git/repository"
            required
          />
        </label>
        <label>
          Default branch
          <input value={defaultBranch} onChange={(event) => setDefaultBranch(event.target.value)} required />
        </label>
        <button type="submit" disabled={registering}>
          {registering ? "Registering…" : "Register project"}
        </button>
        {registerStatus && <p role="status" aria-live="polite">{registerStatus}</p>}
      </form>
      <ul>
        {projects.map((item) => (
          <li key={item.id}>
            <span>{item.name} — {item.path}</span>
            {item.archived_at ? (
              <small>Archived</small>
            ) : (
              <button
                type="button"
                className="ghost"
                disabled={archivingId === item.id}
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

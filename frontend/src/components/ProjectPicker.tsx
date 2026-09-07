import { useEffect, useState, type FormEvent } from "react";
import { api } from "../api";
import type { Project } from "../types";

interface Props {
  project: Project | null;
  projects: Project[];
  onChanged: () => Promise<void>;
  onError: (error: unknown) => void;
}

export function ProjectPicker({ project, projects, onChanged, onError }: Props) {
  const [path, setPath] = useState(project?.path ?? "");
  const [name, setName] = useState("");
  const [defaultBranch, setDefaultBranch] = useState("main");
  useEffect(() => setPath(project?.path ?? ""), [project?.path]);
  async function register(event: FormEvent) {
    event.preventDefault();
    try {
      await api.registerProject(name, path, defaultBranch);
      setName("");
      await onChanged();
    } catch (error) {
      onError(error);
    }
  }
  async function archive(id: number) {
    try {
      await api.archiveProject(id);
      await onChanged();
    } catch (error) {
      onError(error);
    }
  }
  return (
    <details className="project-picker">
      <summary>{project?.name ?? "Choose project"}</summary>
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
              setPath(selected.path);
              void api.setProject(selected.path).then(onChanged).catch(onError);
            }
          }}
        >
          <option value="" disabled>Select a project</option>
          {projects.filter((item) => !item.archived_at).map((item) => (
            <option key={item.id} value={item.id}>{item.name}</option>
          ))}
        </select>
      </label>
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
        <button type="submit">Register project</button>
      </form>
      <ul>
        {projects.map((item) => (
          <li key={item.id}>
            <span>{item.name} — {item.path}</span>
            {item.archived_at ? (
              <small>Archived</small>
            ) : (
              <button type="button" className="ghost" onClick={() => void archive(item.id)}>
                Archive
              </button>
            )}
          </li>
        ))}
      </ul>
    </details>
  );
}

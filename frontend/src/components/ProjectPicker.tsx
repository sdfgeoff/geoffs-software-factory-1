import { useEffect, useState, type FormEvent } from "react";
import { api } from "../api";
import type { Project } from "../types";

interface Props {
  project: Project | null;
  onChanged: () => Promise<void>;
  onError: (error: unknown) => void;
}

export function ProjectPicker({ project, onChanged, onError }: Props) {
  const [path, setPath] = useState(project?.path ?? "");
  useEffect(() => setPath(project?.path ?? ""), [project]);
  async function submit(event: FormEvent) {
    event.preventDefault();
    try {
      await api.setProject(path);
      await onChanged();
    } catch (error) {
      onError(error);
    }
  }
  return (
    <form className="project-picker" onSubmit={submit}>
      <label htmlFor="project-path">Project folder</label>
      <div className="field-row">
        <input
          id="project-path"
          value={path}
          onChange={(event) => setPath(event.target.value)}
          placeholder="/path/to/git/repository"
          required
        />
        <button type="submit">Open</button>
      </div>
    </form>
  );
}

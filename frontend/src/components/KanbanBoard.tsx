import { useEffect, useState, type FormEvent } from "react";
import { api } from "../api";
import type { Agent, Project, Ticket, TicketState } from "../types";
import {
  TicketDetailPanel,
  type TicketDraft,
} from "./TicketDetailPanel";

const columns: { state: TicketState; label: string }[] = [
  { state: "todo", label: "To do" },
  { state: "in_progress", label: "In progress" },
  { state: "review", label: "Review" },
  { state: "done", label: "Done" },
  { state: "cancelled", label: "Cancelled" },
];
interface Props {
  tickets: Ticket[];
  agents: Agent[];
  projects?: Project[];
  currentProjectId?: number | null;
  filterProjectId?: number | null;
  selectedTicketId?: number | null;
  onFilterProject?: (projectId: number | null) => void;
  onSelectTicket?: (ticketId: number | null) => void;
  onChanged: (tickets: Ticket | Ticket[]) => void | Promise<void>;
  onError: (error: unknown) => void;
}

export function KanbanBoard({
  tickets,
  agents,
  projects = [],
  currentProjectId = null,
  filterProjectId: routedProjectId,
  selectedTicketId: routedTicketId,
  onFilterProject,
  onSelectTicket,
  onChanged,
  onError,
}: Props) {
  const [title, setTitle] = useState("");
  const [creating, setCreating] = useState(false);
  const [createStatus, setCreateStatus] = useState("");
  const [projectId, setProjectId] = useState(currentProjectId?.toString() ?? "");
  const [filterProjectId, setFilterProjectId] = useState(
    routedProjectId?.toString() ?? "all",
  );
  const [showCancelled, setShowCancelled] = useState(false);
  const [localTicketId, setLocalTicketId] = useState<number | null>(null);
  const [ticketDrafts, setTicketDrafts] = useState<Record<number, TicketDraft>>({});
  const selectedTicketId =
    routedTicketId === undefined ? localTicketId : routedTicketId;
  const selectTicket = onSelectTicket ?? setLocalTicketId;
  useEffect(() => {
    if (!projectId && currentProjectId) setProjectId(currentProjectId.toString());
  }, [currentProjectId, projectId]);
  useEffect(() => {
    if (routedProjectId !== undefined) {
      setFilterProjectId(routedProjectId?.toString() ?? "all");
    }
  }, [routedProjectId]);
  const selectedTicket = tickets.find(
    (ticket) => ticket.id === selectedTicketId,
  );
  const showingCancelled = showCancelled || selectedTicket?.state === "cancelled";
  const visibleTickets = tickets.filter(
    (ticket) =>
      (showingCancelled || ticket.state !== "cancelled") &&
      (filterProjectId === "all" ||
        ticket.project_id === Number(filterProjectId)),
  );
  const visibleColumns = showingCancelled
    ? columns
    : columns.filter((column) => column.state !== "cancelled");
  async function create(event: FormEvent) {
    event.preventDefault();
    if (creating) return;
    setCreating(true);
    setCreateStatus("Creating ticket…");
    try {
      const created = await api.createTicket(title, Number(projectId));
      setTitle("");
      await onChanged(created);
      setCreateStatus("Ticket created.");
    } catch (error) {
      setCreateStatus(error instanceof Error ? error.message : "Could not create ticket");
    } finally {
      setCreating(false);
    }
  }
  return (
    <section>
      <form className="create-row" onSubmit={create}>
        <label className="sr-only" htmlFor="ticket-title">
          New ticket title
        </label>
        <input
          id="ticket-title"
          value={title}
          onChange={(event) => setTitle(event.target.value)}
          placeholder="Add a ticket"
          required
          maxLength={200}
        />
        <label className="sr-only" htmlFor="ticket-project">Ticket project</label>
        <select id="ticket-project" value={projectId} onChange={(event) => setProjectId(event.target.value)} required>
          <option value="" disabled>Select project</option>
          {projects.filter((project) => !project.archived_at).map((project) => (
            <option key={project.id} value={project.id}>{project.name}</option>
          ))}
        </select>
        <button type="submit" disabled={creating}>
          {creating ? "Creating…" : "Create ticket"}
        </button>
        {createStatus && <p role="status" aria-live="polite">{createStatus}</p>}
      </form>
      <label>
        Filter projects
        <select
          value={filterProjectId}
          onChange={(event) => {
            setFilterProjectId(event.target.value);
            onFilterProject?.(
              event.target.value === "all" ? null : Number(event.target.value),
            );
          }}
        >
          <option value="all">All projects</option>
          {projects.map((project) => (
            <option key={project.id} value={project.id}>{project.name}</option>
          ))}
        </select>
      </label>
      <label>
        <input
          type="checkbox"
          checked={showCancelled}
          onChange={(event) => setShowCancelled(event.target.checked)}
        />
        Show cancelled tickets
      </label>
      <div className={`kanban${showingCancelled ? " kanban-with-cancelled" : ""}`}>
        {visibleColumns.map((column) => (
          <section
            className={`column column-${column.state}`}
            key={column.state}
            aria-labelledby={`column-${column.state}`}
          >
            <h2 id={`column-${column.state}`}>
              {column.label}
              <span>
                {
                  visibleTickets.filter((ticket) => ticket.state === column.state)
                    .length
                }
              </span>
            </h2>
            {visibleTickets
              .filter((ticket) => ticket.state === column.state)
              .map((ticket) => (
                <TicketCard
                  key={ticket.id}
                  ticket={ticket}
                  agents={agents}
                  project={projects.find((project) => project.id === ticket.project_id)}
                  selected={selectedTicketId === ticket.id}
                  onSelect={selectTicket}
                />
              ))}
          </section>
        ))}
      </div>
      {selectedTicket && (
        <TicketDetailPanel
          ticket={selectedTicket}
          tickets={tickets}
          agents={agents}
          project={projects.find(
            (project) => project.id === selectedTicket.project_id,
          )}
          onClose={() => selectTicket(null)}
          onChanged={onChanged}
          draft={ticketDrafts[selectedTicket.id]}
          onDraftChange={(draft) =>
            setTicketDrafts((current) => ({
              ...current,
              [selectedTicket.id]: draft,
            }))
          }
          onError={onError}
        />
      )}
    </section>
  );
}

function TicketCard({
  ticket,
  agents,
  project,
  selected,
  onSelect,
}: {
  ticket: Ticket;
  agents: Agent[];
  project?: Project;
  selected: boolean;
  onSelect: (ticketId: number | null) => void;
}) {
  const reservedFor = agents.find(
    (agent) => agent.id === ticket.reserved_for_agent_id,
  );
  const assignedTo = agents.find((agent) => agent.id === ticket.assignee_id);
  const owner = assignedTo
    ? `Assigned to ${assignedTo.name}`
    : reservedFor
      ? `Reserved for ${reservedFor.name}`
      : "Unassigned";
  return (
    <button
      type="button"
      className="ticket-card"
      aria-current={selected ? "page" : undefined}
      onClick={() => onSelect(ticket.id)}
    >
      <span className="ticket-card-topline">
        <span className="ticket-number">#{ticket.id}</span>
        <span className={`state-badge state-${ticket.state}`}>
          {columns.find((column) => column.state === ticket.state)?.label}
        </span>
      </span>
      <strong className="ticket-card-title">{ticket.title}</strong>
      <span className="ticket-card-meta">
        <span title={project?.path}>
          {project?.name ?? `Project #${ticket.project_id}`}
        </span>
        <span>{owner}</span>
      </span>
      {ticket.unresolved_blockers.length > 0 && (
        <span className="ticket-card-blocked">
          Blocked by{" "}
          {ticket.unresolved_blockers
            .map((blocker) => `#${blocker.id}`)
            .join(", ")}
        </span>
      )}
      {ticket.review_feedback && (
        <span className="ticket-card-review">Changes requested</span>
      )}
    </button>
  );
}

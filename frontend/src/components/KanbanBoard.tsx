import { useEffect, useState, type FormEvent } from "react";
import { api } from "../api";
import type { Agent, Project, Ticket, TicketState } from "../types";

const columns: { state: TicketState; label: string }[] = [
  { state: "todo", label: "To do" },
  { state: "in_progress", label: "In progress" },
  { state: "review", label: "Review" },
  { state: "done", label: "Done" },
];
interface Props {
  tickets: Ticket[];
  agents: Agent[];
  projects?: Project[];
  currentProjectId?: number | null;
  onChanged: () => Promise<void>;
  onError: (error: unknown) => void;
}

export function KanbanBoard({ tickets, agents, projects = [], currentProjectId = null, onChanged, onError }: Props) {
  const [title, setTitle] = useState("");
  const [projectId, setProjectId] = useState(currentProjectId?.toString() ?? "");
  const [filterProjectId, setFilterProjectId] = useState("all");
  useEffect(() => {
    if (!projectId && currentProjectId) setProjectId(currentProjectId.toString());
  }, [currentProjectId, projectId]);
  const visibleTickets = filterProjectId === "all"
    ? tickets
    : tickets.filter((ticket) => ticket.project_id === Number(filterProjectId));
  async function create(event: FormEvent) {
    event.preventDefault();
    try {
      await api.createTicket(title, Number(projectId));
      setTitle("");
      await onChanged();
    } catch (error) {
      onError(error);
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
        <button type="submit">Create ticket</button>
      </form>
      <label>
        Filter projects
        <select value={filterProjectId} onChange={(event) => setFilterProjectId(event.target.value)}>
          <option value="all">All projects</option>
          {projects.map((project) => (
            <option key={project.id} value={project.id}>{project.name}</option>
          ))}
        </select>
      </label>
      <div className="kanban">
        {columns.map((column) => (
          <section
            className="column"
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
                  onChanged={onChanged}
                  onError={onError}
                />
              ))}
          </section>
        ))}
      </div>
    </section>
  );
}

function TicketCard({
  ticket,
  agents,
  project,
  onChanged,
  onError,
}: {
  ticket: Ticket;
  agents: Agent[];
  project?: Project;
  onChanged: () => Promise<void>;
  onError: (error: unknown) => void;
}) {
  const [title, setTitle] = useState(ticket.title);
  const [description, setDescription] = useState(ticket.description);
  const [state, setState] = useState(ticket.state);
  const [assignee, setAssignee] = useState(
    ticket.assignee_id?.toString() ?? "",
  );
  const [reservation, setReservation] = useState(
    ticket.reserved_for_agent_id?.toString() ?? "",
  );
  const [blockerIds, setBlockerIds] = useState(ticket.blocker_ids.join(", "));
  const reviewer = agents.find((agent) => agent.id === ticket.reviewer_id);
  useEffect(() => {
    setTitle(ticket.title);
    setDescription(ticket.description);
    setState(ticket.state);
    setAssignee(ticket.assignee_id?.toString() ?? "");
    setReservation(ticket.reserved_for_agent_id?.toString() ?? "");
    setBlockerIds(ticket.blocker_ids.join(", "));
  }, [ticket]);
  async function save(event: FormEvent) {
    event.preventDefault();
    try {
      await api.updateTicket(ticket, {
        title,
        description,
        state,
        assignee_id: assignee ? Number(assignee) : null,
        blocker_ids: parseBlockerIds(blockerIds),
      });
      await onChanged();
    } catch (error) {
      onError(error);
    }
  }
  async function reserve() {
    if (!reservation) return;
    try {
      const agentId = Number(reservation);
      await api.reserveTickets(
        [ticket.id],
        agentId,
        ticket.reserved_for_agent_id !== null &&
          ticket.reserved_for_agent_id !== agentId,
      );
      await onChanged();
    } catch (error) {
      onError(error);
    }
  }
  const creator = agents.find((agent) => agent.id === ticket.created_by_agent_id);
  const reservedFor = agents.find(
    (agent) => agent.id === ticket.reserved_for_agent_id,
  );
  const assignedTo = agents.find((agent) => agent.id === ticket.assignee_id);
  return (
    <details className="ticket-card">
      <summary>
        <span>#{ticket.id}</span>
        {ticket.title}
        <small className="project-badge">
          {project?.name ?? `Project #${ticket.project_id}`}
        </small>
        {ticket.unresolved_blockers.length > 0 && (
          <strong className="blocked-badge">Blocked</strong>
        )}
      </summary>
      <form onSubmit={save}>
        <dl className="ticket-ownership">
          <dt>Created by</dt>
          <dd>{creator?.name ?? "User/system"}</dd>
          <dt>Reserved for</dt>
          <dd>{reservedFor?.name ?? "Nobody"}</dd>
          <dt>Active assignee</dt>
          <dd>{assignedTo?.name ?? "Nobody"}</dd>
        </dl>
        {ticket.unresolved_blockers.length > 0 && (
          <p className="ticket-blockers">
            Blocked by {ticket.unresolved_blockers
              .map((blocker) => `#${blocker.id} ${blocker.title}`)
              .join(", ")}
          </p>
        )}
        {(ticket.implementation_branch || ticket.review_commit || ticket.review_feedback) && (
          <dl className="review-meta">
            {ticket.implementation_branch && (
              <>
                <dt>Implementation branch</dt>
                <dd>{ticket.implementation_branch}</dd>
              </>
            )}
            {ticket.review_commit && (
              <>
                <dt>Review commit</dt>
                <dd title={ticket.review_commit}>
                  {ticket.review_commit.slice(0, 12)} → {ticket.review_target_branch}
                </dd>
              </>
            )}
            <dt>Reviewer</dt>
            <dd>{reviewer?.name ?? "Unclaimed"}</dd>
            {ticket.review_feedback && (
              <>
                <dt>Feedback</dt>
                <dd>{ticket.review_feedback}</dd>
              </>
            )}
          </dl>
        )}
        <label>
          Title
          <input
            value={title}
            onChange={(event) => setTitle(event.target.value)}
            maxLength={200}
            required
          />
        </label>
        <label>
          Description
          <textarea
            value={description}
            onChange={(event) => setDescription(event.target.value)}
            rows={4}
          />
        </label>
        <label>
          Blocker ticket IDs
          <input
            value={blockerIds}
            onChange={(event) => setBlockerIds(event.target.value)}
            placeholder="1, 2"
          />
        </label>
        <label>
          Status
          <select
            value={state}
            onChange={(event) => setState(event.target.value as TicketState)}
          >
            {columns.map((column) => (
              <option value={column.state} key={column.state}>
                {column.label}
              </option>
            ))}
          </select>
        </label>
        <label>
          Active assignee
          <select
            value={assignee}
            onChange={(event) => setAssignee(event.target.value)}
          >
            <option value="">Unassigned</option>
            {agents.map((agent) => (
              <option value={agent.id} key={agent.id}>
                {agent.name}
              </option>
            ))}
          </select>
        </label>
        <label>
          Reservation
          <select
            value={reservation}
            onChange={(event) => setReservation(event.target.value)}
            disabled={ticket.state !== "todo"}
          >
            <option value="">Not reserved</option>
            {agents.map((agent) => (
              <option value={agent.id} key={agent.id}>
                {agent.name}
              </option>
            ))}
          </select>
        </label>
        <button
          type="button"
          onClick={reserve}
          disabled={
            ticket.state !== "todo" ||
            !reservation ||
            reservation === (ticket.reserved_for_agent_id?.toString() ?? "")
          }
        >
          {ticket.reserved_for_agent_id &&
          reservation !== ticket.reserved_for_agent_id.toString()
            ? "Override reservation"
            : "Reserve"}
        </button>
        <button type="submit">Save</button>
      </form>
    </details>
  );
}

function parseBlockerIds(value: string): number[] {
  if (!value.trim()) return [];
  const ids = value.split(",").map((part) => Number(part.trim()));
  if (ids.some((id) => !Number.isSafeInteger(id) || id <= 0)) {
    throw new Error("Blocker ticket IDs must be positive integers separated by commas");
  }
  return ids;
}

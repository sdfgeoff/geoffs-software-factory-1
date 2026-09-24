import { useEffect, useState, type FormEvent } from "react";
import { api } from "../api";
import type {
  Agent,
  Project,
  Ticket,
  TicketActivity,
  TicketState,
  TicketSummary,
} from "../types";
import {
  TicketDetailPanel,
  type TicketDraft,
} from "./TicketDetailPanel";
import {
  TicketEditorFields,
  type TicketEditorValue,
} from "./TicketEditorFields";

const columns: { state: TicketState; label: string }[] = [
  { state: "todo", label: "To do" },
  { state: "in_progress", label: "In progress" },
  { state: "review", label: "Review" },
  { state: "awaiting_external_merge", label: "Awaiting external merge" },
  { state: "done", label: "Done" },
  { state: "cancelled", label: "Cancelled" },
];
interface Props {
  tickets: (TicketSummary | Ticket)[];
  agents: Agent[];
  projects?: Project[];
  filterProjectId?: number | null;
  selectedTicketId?: number | null;
  selectedTicket?: Ticket | null;
  onFilterProject?: (projectId: number | null) => void;
  onSelectTicket?: (ticketId: number | null) => void;
  onChanged: (tickets: Ticket | Ticket[]) => void | Promise<void>;
  historyHasMore?: Partial<Record<"done" | "cancelled", boolean>>;
  onLoadHistory?: (state: "done" | "cancelled") => void;
  onError: (error: unknown) => void;
  liveTicketActivity?: Record<number, TicketActivity>;
  activityRefresh?: number;
}

export function KanbanBoard({
  tickets,
  agents,
  projects = [],
  filterProjectId: routedProjectId,
  selectedTicketId: routedTicketId,
  selectedTicket: loadedTicket,
  onFilterProject,
  onSelectTicket,
  onChanged,
  onError,
  liveTicketActivity = {},
  activityRefresh = 0,
  historyHasMore = {},
  onLoadHistory,
}: Props) {
  const [showCreate, setShowCreate] = useState(false);
  const [createDraft, setCreateDraft] = useState<TicketEditorValue>({
    title: "",
    description: "",
    assignee: "",
    blockerIds: [],
    blockerQuery: "",
  });
  const [creating, setCreating] = useState(false);
  const [createStatus, setCreateStatus] = useState("");
  const [projectId, setProjectId] = useState(routedProjectId?.toString() ?? "");
  const [createDraftEdited, setCreateDraftEdited] = useState(false);
  const [filterProjectId, setFilterProjectId] = useState(
    routedProjectId?.toString() ?? "all",
  );
  const [showCancelled, setShowCancelled] = useState(false);
  const [mobileState, setMobileState] = useState<TicketState>("todo");
  const [localTicketId, setLocalTicketId] = useState<number | null>(null);
  const [ticketDrafts, setTicketDrafts] = useState<Record<number, TicketDraft>>({});
  const [commentDrafts, setCommentDrafts] = useState<Record<number, string>>({});
  const selectedTicketId =
    routedTicketId === undefined ? localTicketId : routedTicketId;
  const selectTicket = onSelectTicket ?? setLocalTicketId;
  useEffect(() => {
    if (routedProjectId !== undefined) {
      setFilterProjectId(routedProjectId?.toString() ?? "all");
      if (!createDraftEdited) setProjectId(routedProjectId?.toString() ?? "");
    }
  }, [createDraftEdited, routedProjectId]);
  const selectedSummary = tickets.find(
    (ticket) => ticket.id === selectedTicketId,
  );
  const selectedTicket = loadedTicket === undefined && selectedSummary && "description" in selectedSummary
    ? selectedSummary as unknown as Ticket
    : loadedTicket;
  const showingCancelled = showCancelled || selectedSummary?.state === "cancelled";
  const visibleTickets = tickets.filter(
    (ticket) =>
      (showingCancelled || ticket.state !== "cancelled") &&
      (filterProjectId === "all" ||
        ticket.project_id === Number(filterProjectId)),
  );
  const hasExternalMergeTickets = visibleTickets.some(
    (ticket) => ticket.state === "awaiting_external_merge",
  );
  const visibleColumns = columns.filter(
    (column) =>
      (showingCancelled || column.state !== "cancelled") &&
      (hasExternalMergeTickets || column.state !== "awaiting_external_merge"),
  );
  useEffect(() => {
    if (!visibleColumns.some((column) => column.state === mobileState)) {
      setMobileState("todo");
    }
  }, [mobileState, visibleColumns]);
  async function create(event: FormEvent) {
    event.preventDefault();
    if (creating) return;
    setCreating(true);
    setCreateStatus("Creating ticket…");
    try {
      const created = await api.createTicket({
        title: createDraft.title,
        description: createDraft.description,
        blocker_ids: createDraft.blockerIds,
        project_id: Number(projectId),
      });
      setCreateDraft({
        title: "",
        description: "",
        assignee: "",
        blockerIds: [],
        blockerQuery: "",
      });
      setCreateDraftEdited(false);
      setProjectId(filterProjectId === "all" ? "" : filterProjectId);
      await onChanged(created);
      selectTicket(created.id);
      setCreateStatus("Ticket created.");
      setShowCreate(false);
    } catch (error) {
      setCreateStatus(error instanceof Error ? error.message : "Could not create ticket");
    } finally {
      setCreating(false);
    }
  }
  return (
    <section>
      <div className="create-row">
        <button type="button" onClick={() => {
          setCreateStatus("");
          setShowCreate(true);
        }}>
          Create ticket
        </button>
        {!showCreate && createStatus && (
          <p role="status" aria-live="polite">{createStatus}</p>
        )}
      </div>
      <label>
        Filter projects
        <select
          value={filterProjectId}
          onChange={(event) => {
            setFilterProjectId(event.target.value);
            if (!createDraftEdited) {
              setProjectId(event.target.value === "all" ? "" : event.target.value);
            }
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
          onChange={(event) => {
            setShowCancelled(event.target.checked);
            if (!event.target.checked && mobileState === "cancelled") {
              setMobileState("todo");
            }
          }}
        />
        Show cancelled tickets
      </label>
      <label className="mobile-context-picker">
        Ticket status
        <select
          value={mobileState}
          onChange={(event) => setMobileState(event.target.value as TicketState)}
        >
          {visibleColumns.map((column) => (
            <option key={column.state} value={column.state}>
              {column.label} ({visibleTickets.filter((ticket) => ticket.state === column.state).length})
            </option>
          ))}
        </select>
      </label>
      <div className={`kanban${hasExternalMergeTickets ? " kanban-with-external" : ""}${showingCancelled ? " kanban-with-cancelled" : ""}`}>
        {visibleColumns.map((column) => (
          <section
            className={`column column-${column.state}${mobileState === column.state ? " mobile-active" : ""}`}
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
            {(column.state === "done" || column.state === "cancelled") &&
              historyHasMore[column.state] && (
                <button type="button" className="ghost" onClick={() => onLoadHistory?.(column.state as "done" | "cancelled")}>
                  Load older tickets
                </button>
              )}
          </section>
        ))}
      </div>
      {selectedTicket && !showCreate && (
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
          liveActivity={Object.values(liveTicketActivity).filter(
            (entry) => entry.ticket_id === selectedTicket.id,
          )}
          commentDraft={commentDrafts[selectedTicket.id] ?? ""}
          onCommentDraftChange={(commentDraft) =>
            setCommentDrafts((current) => ({
              ...current,
              [selectedTicket.id]: commentDraft,
            }))
          }
          activityRefresh={activityRefresh}
        />
      )}
      {showCreate && (
        <aside className="ticket-detail-panel" aria-labelledby="create-ticket-title">
          <div className="ticket-detail-header">
            <div>
              <span className="ticket-number">New ticket</span>
              <h2 id="create-ticket-title">Create ticket</h2>
            </div>
            <button
              type="button"
              className="ghost"
              disabled={creating}
              onClick={() => setShowCreate(false)}
            >
              Close
            </button>
          </div>
          <form className="ticket-detail-form" onSubmit={create}>
            <label>
              Project
              <select
                value={projectId}
                onChange={(event) => {
                  setProjectId(event.target.value);
                  setCreateDraftEdited(true);
                }}
                disabled={creating}
                required
              >
                <option value="" disabled>Select project</option>
                {projects.filter((project) => !project.archived_at).map((project) => (
                  <option key={project.id} value={project.id}>{project.name}</option>
                ))}
              </select>
            </label>
            <TicketEditorFields
              value={createDraft}
              tickets={tickets}
              agents={agents}
              disabled={creating}
              showAssignee={false}
              autoFocusTitle
              onChange={(changes, markDirty = true) => {
                setCreateDraft((current) => ({ ...current, ...changes }));
                if (markDirty) setCreateDraftEdited(true);
              }}
            />
            <div className="ticket-actions">
              <button type="submit" disabled={creating}>
                {creating ? "Creating…" : "Create ticket"}
              </button>
              <button
                type="button"
                className="ghost"
                disabled={creating}
                onClick={() => setShowCreate(false)}
              >
                Cancel
              </button>
            </div>
            {createStatus && <p role="status" aria-live="polite">{createStatus}</p>}
          </form>
        </aside>
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
  ticket: TicketSummary | Ticket;
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
  const externalWorkflow = ticket.state === "awaiting_external_merge" ||
    (ticket.review_completion_workflow
      ? ticket.review_completion_workflow === "external"
      : project?.completion_workflow === "external");
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
      {externalWorkflow && (
        <span className="workflow-badge">External merge</span>
      )}
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
      {"activity_count" in ticket && ticket.activity_count > 0 && (
        <span className="ticket-card-activity">
          {ticket.activity_count} timeline {ticket.activity_count === 1 ? "item" : "items"}
        </span>
      )}
    </button>
  );
}

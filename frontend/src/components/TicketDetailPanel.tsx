import { useEffect, type FormEvent } from "react";
import { api } from "../api";
import type { Agent, Project, Ticket, TicketState } from "../types";

interface Props {
  ticket: Ticket;
  tickets: Ticket[];
  agents: Agent[];
  project?: Project;
  onClose: () => void;
  onChanged: (tickets: Ticket | Ticket[]) => void | Promise<void>;
  draft?: TicketDraft;
  onDraftChange: (draft: TicketDraft) => void;
  onError: (error: unknown) => void;
}

export interface TicketDraft {
  title: string;
  description: string;
  assignee: string;
  reservation: string;
  blockerIds: number[];
  blockerQuery: string;
  baseRevision: number;
  dirty: boolean;
}

export function ticketDraft(ticket: Ticket): TicketDraft {
  return {
    title: ticket.title,
    description: ticket.description,
    assignee: ticket.assignee_id?.toString() ?? "",
    reservation: ticket.reserved_for_agent_id?.toString() ?? "",
    blockerIds: ticket.blocker_ids,
    blockerQuery: "",
    baseRevision: ticket.revision,
    dirty: false,
  };
}

const stateLabels: Record<TicketState, string> = {
  todo: "To do",
  in_progress: "In progress",
  review: "Review",
  done: "Done",
  cancelled: "Cancelled",
};

export function TicketDetailPanel({
  ticket,
  tickets,
  agents,
  project,
  onClose,
  onChanged,
  draft: savedDraft,
  onDraftChange,
  onError,
}: Props) {
  const draft = savedDraft ?? ticketDraft(ticket);
  const creator = agents.find((agent) => agent.id === ticket.created_by_agent_id);
  const reservedFor = agents.find(
    (agent) => agent.id === ticket.reserved_for_agent_id,
  );
  const assignedTo = agents.find((agent) => agent.id === ticket.assignee_id);
  const reviewer = agents.find((agent) => agent.id === ticket.reviewer_id);
  const matchingBlockers = tickets.filter((candidate) => {
    if (candidate.id === ticket.id) return false;
    const query = draft.blockerQuery.trim().toLowerCase();
    return (
      draft.blockerIds.includes(candidate.id) ||
      !query ||
      `#${candidate.id} ${candidate.title}`.toLowerCase().includes(query)
    );
  });

  useEffect(() => {
    if (!savedDraft || (!draft.dirty && draft.baseRevision !== ticket.revision)) {
      onDraftChange(ticketDraft(ticket));
    }
  }, [draft.baseRevision, draft.dirty, onDraftChange, savedDraft, ticket]);

  const updatedElsewhere = draft.dirty && ticket.revision !== draft.baseRevision;
  const updateDraft = (changes: Partial<TicketDraft>, dirty = true) =>
    onDraftChange({ ...draft, ...changes, dirty: dirty || draft.dirty });

  async function save(event: FormEvent) {
    event.preventDefault();
    try {
      const updated = await api.updateTicket({ ...ticket, revision: draft.baseRevision }, {
        title: draft.title,
        description: draft.description,
        assignee_id: draft.assignee ? Number(draft.assignee) : null,
        blocker_ids: draft.blockerIds,
      });
      onDraftChange(ticketDraft(updated));
      await onChanged(updated);
    } catch (error) {
      onError(error);
    }
  }

  async function reserve() {
    if (!draft.reservation) return;
    try {
      const agentId = Number(draft.reservation);
      const updated = await api.reserveTickets(
        [ticket.id],
        agentId,
        ticket.reserved_for_agent_id !== null &&
          ticket.reserved_for_agent_id !== agentId,
      );
      await onChanged(updated);
    } catch (error) {
      onError(error);
    }
  }

  async function transition(state: TicketState) {
    try {
      await onChanged(await api.updateTicket(ticket, { state }));
    } catch (error) {
      onError(error);
    }
  }

  const actions =
    ticket.state === "todo"
      ? [
          { label: "Start work", state: "in_progress" as const },
          { label: "Cancel ticket", state: "cancelled" as const },
        ]
      : ticket.state === "in_progress"
        ? [
            { label: "Move to todo", state: "todo" as const },
            { label: "Cancel ticket", state: "cancelled" as const },
          ]
        : [];

  return (
    <aside className="ticket-detail-panel" aria-labelledby={`ticket-detail-title-${ticket.id}`}>
      <div className="ticket-detail-header">
        <div>
          <span className="ticket-number">Ticket #{ticket.id}</span>
          <h2 id={`ticket-detail-title-${ticket.id}`}>{ticket.title}</h2>
        </div>
        <button type="button" className="ghost" onClick={onClose}>
          Close
        </button>
      </div>

      <dl className="ticket-detail-meta">
        <dt>State</dt>
        <dd>
          <span className={`state-badge state-${ticket.state}`}>
            {stateLabels[ticket.state]}
          </span>
        </dd>
        <dt>Project</dt>
        <dd title={project?.path}>
          {project?.name ?? `Project #${ticket.project_id}`}
        </dd>
        <dt>Created by</dt>
        <dd>{creator?.name ?? "User/system"}</dd>
        <dt>Reserved for</dt>
        <dd>{reservedFor?.name ?? "Nobody"}</dd>
        <dt>Active assignee</dt>
        <dd>{assignedTo?.name ?? "Nobody"}</dd>
      </dl>

      {ticket.unresolved_blockers.length > 0 && (
        <p className="ticket-blockers">
          Blocked by{" "}
          {ticket.unresolved_blockers
            .map((blocker) => `#${blocker.id} ${blocker.title}`)
            .join(", ")}
        </p>
      )}

      {(ticket.implementation_branch ||
        ticket.review_commit ||
        ticket.review_feedback) && (
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
                {ticket.review_commit.slice(0, 12)} →{" "}
                {ticket.review_target_branch}
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

      <form className="ticket-detail-form" onSubmit={save}>
        {updatedElsewhere && (
          <p className="inline-error" role="status">
            This ticket was updated elsewhere. Your draft has been preserved;
            review the latest changes before saving.
          </p>
        )}
        <label>
          Title
          <input
            value={draft.title}
            onChange={(event) => {
              updateDraft({ title: event.target.value });
            }}
            maxLength={200}
            required
          />
        </label>
        <label>
          Description
          <textarea
            value={draft.description}
            onChange={(event) => {
              updateDraft({ description: event.target.value });
            }}
            rows={7}
          />
        </label>
        <label>
          Active assignee
          <select
            value={draft.assignee}
            onChange={(event) => {
              updateDraft({ assignee: event.target.value });
            }}
          >
            <option value="">Unassigned</option>
            {agents.map((agent) => (
              <option value={agent.id} key={agent.id}>
                {agent.name}
              </option>
            ))}
          </select>
        </label>
        <fieldset className="blocker-selector">
          <legend>Blockers</legend>
          <label>
            Search tickets
            <input
              type="search"
              value={draft.blockerQuery}
              onChange={(event) => updateDraft({ blockerQuery: event.target.value }, false)}
              placeholder="Search by ID or title"
            />
          </label>
          <div className="blocker-options">
            {matchingBlockers.length === 0 && (
              <p className="empty">No matching tickets.</p>
            )}
            {matchingBlockers.map((candidate) => (
              <label key={candidate.id}>
                <input
                  type="checkbox"
                  checked={draft.blockerIds.includes(candidate.id)}
                  onChange={(event) => {
                    updateDraft({
                      blockerIds:
                      event.target.checked
                        ? [...draft.blockerIds, candidate.id]
                        : draft.blockerIds.filter((id) => id !== candidate.id),
                    });
                  }}
                />
                <span>
                  <strong>
                    #{candidate.id} {candidate.title}
                  </strong>
                  <small>{stateLabels[candidate.state]}</small>
                </span>
              </label>
            ))}
          </div>
        </fieldset>
        <button type="submit">Save changes</button>
      </form>

      {ticket.state === "todo" && (
        <div className="reservation-editor">
          <label>
            Reservation
            <select
              value={draft.reservation}
              onChange={(event) => updateDraft({ reservation: event.target.value }, false)}
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
              !draft.reservation ||
              draft.reservation ===
                (ticket.reserved_for_agent_id?.toString() ?? "")
            }
          >
            {ticket.reserved_for_agent_id &&
            draft.reservation !== ticket.reserved_for_agent_id.toString()
              ? "Override reservation"
              : "Reserve"}
          </button>
        </div>
      )}

      {actions.length > 0 && (
        <div className="ticket-actions" aria-label="Ticket state actions">
          {actions.map((action) => (
            <button
              type="button"
              className={action.state === "cancelled" ? "danger" : "secondary"}
              key={action.state}
              onClick={() => void transition(action.state)}
            >
              {action.label}
            </button>
          ))}
        </div>
      )}
    </aside>
  );
}

import {
  useEffect,
  useRef,
  useState,
  type FormEvent,
  type MouseEvent,
} from "react";
import { api } from "../api";
import type { Agent, Project, Ticket, TicketState, TicketSummary } from "../types";

interface Props {
  ticket: Ticket;
  tickets: (TicketSummary | Ticket)[];
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
  externalUrl: string;
  returnReason: string;
  mergeConfirmation: string;
  mergeReference: string;
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
    externalUrl: ticket.external_change_url ?? "",
    returnReason: "",
    mergeConfirmation: "",
    mergeReference: "",
    baseRevision: ticket.revision,
    dirty: false,
  };
}

const stateLabels: Record<TicketState, string> = {
  todo: "To do",
  in_progress: "In progress",
  review: "Review",
  awaiting_external_merge: "Awaiting external merge",
  done: "Done",
  cancelled: "Cancelled",
};

function safeExternalUrl(value: string) {
  try {
    const url = new URL(value);
    return url.protocol === "https:" || url.protocol === "http:"
      ? url.href
      : null;
  } catch {
    return null;
  }
}

export function TicketDetailPanel({
  ticket,
  tickets,
  agents,
  project,
  onClose,
  onChanged,
  draft: savedDraft,
  onDraftChange,
}: Props) {
  const draft = savedDraft ?? ticketDraft(ticket);
  const [saving, setSaving] = useState(false);
  const [saveStatus, setSaveStatus] = useState("");
  const [changingReservation, setChangingReservation] = useState(false);
  const [reservationStatus, setReservationStatus] = useState("");
  const [transitioning, setTransitioning] = useState(false);
  const [transitionStatus, setTransitionStatus] = useState("");
  const [externalBusy, setExternalBusy] = useState(false);
  const [externalStatus, setExternalStatus] = useState("");
  const closeButtonRef = useRef<HTMLButtonElement>(null);
  const reservationSelectRef = useRef<HTMLSelectElement>(null);
  const creator = agents.find((agent) => agent.id === ticket.created_by_agent_id);
  const reservedFor = agents.find(
    (agent) => agent.id === ticket.reserved_for_agent_id,
  );
  const assignedTo = agents.find((agent) => agent.id === ticket.assignee_id);
  const reviewer = agents.find((agent) => agent.id === ticket.reviewer_id);
  const externalWorkflow = ticket.review_completion_workflow === "external";
  const externalUrl = safeExternalUrl(draft.externalUrl.trim());
  const ticketBusy = saving || externalBusy;
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
    if (!savedDraft || (!draft.dirty && draft.baseRevision < ticket.revision)) {
      onDraftChange(ticketDraft(ticket));
    }
  }, [draft.baseRevision, draft.dirty, onDraftChange, savedDraft, ticket]);

  const updatedElsewhere = draft.dirty && ticket.revision !== draft.baseRevision;
  const updateDraft = (changes: Partial<TicketDraft>, dirty = true) =>
    onDraftChange({ ...draft, ...changes, dirty: dirty || draft.dirty });

  async function save(event: FormEvent) {
    event.preventDefault();
    if (ticketBusy) return;
    setSaving(true);
    setSaveStatus("Saving…");
    try {
      const updated = await api.updateTicket({ ...ticket, revision: draft.baseRevision }, {
        title: draft.title,
        description: draft.description,
        assignee_id: draft.assignee ? Number(draft.assignee) : null,
        blocker_ids: draft.blockerIds,
      });
      onDraftChange(ticketDraft(updated));
      await onChanged(updated);
      setSaveStatus("Changes saved.");
    } catch (error) {
      setSaveStatus(error instanceof Error ? error.message : "Could not save changes");
    } finally {
      setSaving(false);
    }
  }

  async function setExternalChange(event: FormEvent) {
    event.preventDefault();
    if (ticketBusy) return;
    if (!externalUrl) {
      setExternalStatus("Enter a valid http or https URL.");
      return;
    }
    setExternalBusy(true);
    setExternalStatus("Saving external change…");
    try {
      const updated = await api.setExternalChange(
        { ...ticket, revision: draft.baseRevision },
        externalUrl,
      );
      onDraftChange({
        ...draft,
        externalUrl: updated.external_change_url ?? externalUrl,
        baseRevision: updated.revision,
      });
      await onChanged(updated);
      setExternalStatus("External change saved.");
    } catch (error) {
      setExternalStatus(
        error instanceof Error ? error.message : "Could not save external change",
      );
    } finally {
      setExternalBusy(false);
    }
  }

  async function copyReviewValue(value: string, label: string) {
    try {
      await navigator.clipboard.writeText(value);
      setExternalStatus(`${label} copied.`);
    } catch {
      setExternalStatus(`Could not copy ${label.toLowerCase()}.`);
    }
  }

  async function returnToWork(event: FormEvent) {
    event.preventDefault();
    if (ticketBusy || !draft.returnReason.trim()) return;
    if (!window.confirm(`Return ticket #${ticket.id} to active work?`)) {
      setExternalStatus("Ticket remains awaiting external merge.");
      return;
    }
    setExternalBusy(true);
    setExternalStatus("Returning ticket to work…");
    try {
      const updated = await api.returnExternalMerge(
        { ...ticket, revision: draft.baseRevision },
        draft.returnReason.trim(),
      );
      onDraftChange({
        ...draft,
        returnReason: "",
        baseRevision: updated.revision,
      });
      await onChanged(updated);
      setExternalStatus("Ticket returned to work.");
    } catch (error) {
      setExternalStatus(
        error instanceof Error ? error.message : "Could not return ticket to work",
      );
    } finally {
      setExternalBusy(false);
    }
  }

  async function completeExternalMerge(event: FormEvent) {
    event.preventDefault();
    if (ticketBusy || !draft.mergeConfirmation.trim()) return;
    if (!window.confirm(`Mark ticket #${ticket.id} as externally merged and done?`)) {
      setExternalStatus("External completion cancelled.");
      return;
    }
    setExternalBusy(true);
    setExternalStatus("Recording external merge…");
    try {
      const updated = await api.completeExternalMerge(
        { ...ticket, revision: draft.baseRevision },
        draft.mergeConfirmation.trim(),
        draft.mergeReference.trim(),
      );
      onDraftChange({
        ...draft,
        mergeConfirmation: "",
        mergeReference: "",
        baseRevision: updated.revision,
      });
      await onChanged(updated);
      setExternalStatus("External merge recorded; ticket completed.");
    } catch (error) {
      setExternalStatus(
        error instanceof Error ? error.message : "Could not record external merge",
      );
    } finally {
      setExternalBusy(false);
    }
  }

  async function reserve(event: MouseEvent<HTMLButtonElement>) {
    if (!draft.reservation) return;
    const button = event.currentTarget;
    const agentId = Number(draft.reservation);
    const overriding = ticket.reserved_for_agent_id !== null &&
      ticket.reserved_for_agent_id !== agentId;
    if (
      overriding &&
      !window.confirm(
        `Override ${reservedFor?.name ?? "the current agent"}'s reservation? They may already be planning this work.`,
      )
    ) {
      setReservationStatus("Reservation unchanged.");
      button.focus();
      return;
    }
    if (changingReservation) return;
    setChangingReservation(true);
    setReservationStatus(overriding ? "Overriding reservation…" : "Reserving…");
    try {
      const updated = await api.reserveTickets(
        [ticket.id],
        agentId,
        ticket.reserved_for_agent_id !== null &&
          ticket.reserved_for_agent_id !== agentId,
      );
      await onChanged(updated);
      setReservationStatus(overriding ? "Reservation overridden." : "Ticket reserved.");
      reservationSelectRef.current?.focus();
    } catch (error) {
      setReservationStatus(error instanceof Error ? error.message : "Could not update reservation");
      button.focus();
    } finally {
      setChangingReservation(false);
    }
  }

  async function clearReservation(event: MouseEvent<HTMLButtonElement>) {
    if (changingReservation) return;
    const button = event.currentTarget;
    setChangingReservation(true);
    setReservationStatus("Clearing reservation…");
    try {
      const updated = await api.clearTicketReservation(ticket);
      updateDraft({ reservation: "" }, false);
      await onChanged(updated);
      setReservationStatus("Reservation cleared.");
      reservationSelectRef.current?.focus();
    } catch (error) {
      setReservationStatus(error instanceof Error ? error.message : "Could not clear reservation");
      button.focus();
    } finally {
      setChangingReservation(false);
    }
  }

  async function transition(
    state: TicketState,
    event: MouseEvent<HTMLButtonElement>,
  ) {
    if (
      state === "cancelled" &&
      !window.confirm(
        `Cancel ticket #${ticket.id}? Cancelled tickets are terminal and cannot be reopened.`,
      )
    ) {
      setTransitionStatus("Cancellation abandoned; ticket unchanged.");
      event.currentTarget.focus();
      return;
    }
    if (transitioning) return;
    setTransitioning(true);
    setTransitionStatus("Updating ticket…");
    try {
      await onChanged(await api.updateTicket(ticket, { state }));
      setTransitionStatus("Ticket updated.");
      closeButtonRef.current?.focus();
    } catch (error) {
      setTransitionStatus(error instanceof Error ? error.message : "Could not update ticket");
    } finally {
      setTransitioning(false);
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
        <button ref={closeButtonRef} type="button" className="ghost" onClick={onClose}>
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
              <dd className="copyable-value">
                <code>{ticket.implementation_branch}</code>
                {externalWorkflow && (
                  <button type="button" className="ghost" onClick={() => void copyReviewValue(ticket.implementation_branch!, "Branch")}>Copy branch</button>
                )}
              </dd>
            </>
          )}
          {ticket.review_commit && (
            <>
              <dt>Review commit</dt>
              <dd className="copyable-value" title={ticket.review_commit}>
                <code>{ticket.review_commit}</code>
                {externalWorkflow && (
                  <button type="button" className="ghost" onClick={() => void copyReviewValue(ticket.review_commit!, "Commit")}>Copy commit</button>
                )}
                <span>Target: {ticket.review_target_branch}</span>
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
          {ticket.review_completion_workflow && (
            <>
              <dt>Completion workflow</dt>
              <dd>
                {ticket.review_completion_workflow === "external"
                  ? "External merge"
                  : "Board merge"}
              </dd>
            </>
          )}
          {ticket.external_change_url && (
            <>
              <dt>External change</dt>
              <dd>
                {safeExternalUrl(ticket.external_change_url) ? (
                  <a href={safeExternalUrl(ticket.external_change_url)!} target="_blank" rel="noopener noreferrer">
                    {ticket.external_change_url}
                  </a>
                ) : (
                  <span>{ticket.external_change_url} (invalid URL)</span>
                )}
              </dd>
            </>
          )}
          {ticket.external_merge_reference && (
            <>
              <dt>External merge reference</dt>
              <dd>{ticket.external_merge_reference}</dd>
            </>
          )}
        </dl>
      )}

      {externalWorkflow && ticket.state === "review" && (
        <p className="external-workflow-note">
          <strong>Reviewer action: Approve for external merge.</strong>{" "}
          Approval records the reviewed commit but does not merge or change the
          project branch.
        </p>
      )}

      {externalWorkflow &&
        (ticket.state === "review" || ticket.state === "awaiting_external_merge") && (
          <form className="external-change-form" onSubmit={setExternalChange}>
            <label>
              External change URL
              <input
                type="url"
                value={draft.externalUrl}
                disabled={ticketBusy}
                placeholder="https://…"
                maxLength={2048}
                onChange={(event) => updateDraft({ externalUrl: event.target.value }, false)}
                required
              />
            </label>
            <div className="field-row">
              <button type="submit" disabled={ticketBusy || !externalUrl}>
                {externalBusy ? "Saving…" : ticket.external_change_url ? "Replace URL" : "Save URL"}
              </button>
              {externalUrl && (
                <a className="button-link secondary" href={externalUrl} target="_blank" rel="noopener noreferrer">
                  Open external change
                </a>
              )}
            </div>
            {draft.externalUrl && !externalUrl && (
              <p className="inline-error" role="status">Enter a valid http or https URL.</p>
            )}
          </form>
        )}

      {ticket.state === "awaiting_external_merge" && externalWorkflow && (
        <section className="external-merge-actions" aria-labelledby={`external-merge-title-${ticket.id}`}>
          <h3 id={`external-merge-title-${ticket.id}`}>External merge</h3>
          <form onSubmit={returnToWork}>
            <label>
              Reason for returning to work
              <textarea
                value={draft.returnReason}
                disabled={ticketBusy}
                rows={3}
                maxLength={10000}
                onChange={(event) => updateDraft({ returnReason: event.target.value }, false)}
                required
              />
            </label>
            <button type="submit" className="secondary" disabled={ticketBusy || !draft.returnReason.trim()}>
              Return to work
            </button>
          </form>
          <form onSubmit={completeExternalMerge}>
            <label>
              Completion confirmation
              <textarea
                value={draft.mergeConfirmation}
                disabled={ticketBusy}
                rows={3}
                maxLength={10000}
                placeholder="Confirm where and how the change was merged"
                onChange={(event) => updateDraft({ mergeConfirmation: event.target.value }, false)}
                required
              />
            </label>
            <label>
              Final external reference (optional)
              <input
                value={draft.mergeReference}
                disabled={ticketBusy}
                maxLength={2048}
                placeholder="Merge commit, release, or provider reference"
                onChange={(event) => updateDraft({ mergeReference: event.target.value }, false)}
              />
            </label>
            <button type="submit" disabled={ticketBusy || !draft.mergeConfirmation.trim()}>
              Mark externally merged
            </button>
          </form>
        </section>
      )}

      {externalStatus && <p role="status" aria-live="polite">{externalStatus}</p>}

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
            disabled={ticketBusy}
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
            disabled={ticketBusy}
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
            disabled={ticketBusy}
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
        <fieldset className="blocker-selector" disabled={ticketBusy}>
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
        <button type="submit" disabled={ticketBusy}>
          {saving ? "Saving…" : "Save changes"}
        </button>
        {saveStatus && <p role="status" aria-live="polite">{saveStatus}</p>}
      </form>

      {ticket.state === "todo" && (
        <div className="reservation-editor">
          <label>
            Reservation
            <select
              ref={reservationSelectRef}
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
              changingReservation ||
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
          {ticket.reserved_for_agent_id && (
            <button
              type="button"
              className="ghost"
              disabled={changingReservation}
              onClick={clearReservation}
            >
              Clear reservation
            </button>
          )}
          {reservationStatus && (
            <p role="status" aria-live="polite">{reservationStatus}</p>
          )}
        </div>
      )}

      {actions.length > 0 && (
        <div className="ticket-actions" aria-label="Ticket state actions">
          {actions.map((action) => (
            <button
              type="button"
              className={action.state === "cancelled" ? "danger" : "secondary"}
              key={action.state}
              disabled={transitioning}
              onClick={(event) => void transition(action.state, event)}
            >
              {action.label}
            </button>
          ))}
          {transitionStatus && (
            <p role="status" aria-live="polite">{transitionStatus}</p>
          )}
        </div>
      )}
    </aside>
  );
}

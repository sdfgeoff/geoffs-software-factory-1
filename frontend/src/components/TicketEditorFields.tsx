import type { Agent, Ticket, TicketSummary } from "../types";

export interface TicketEditorValue {
  title: string;
  description: string;
  assignee: string;
  blockerIds: number[];
  blockerQuery: string;
}

interface Props<T extends TicketEditorValue> {
  value: T;
  tickets: (TicketSummary | Ticket)[];
  agents: Agent[];
  disabled?: boolean;
  currentTicketId?: number;
  showAssignee?: boolean;
  autoFocusTitle?: boolean;
  onChange: (changes: Partial<T>, markDirty?: boolean) => void;
}

const stateLabels = {
  draft: "Draft",
  todo: "To do",
  in_progress: "In progress",
  review: "Review",
  awaiting_external_merge: "Awaiting external merge",
  done: "Done",
  cancelled: "Cancelled",
} as const;

export function TicketEditorFields<T extends TicketEditorValue>({
  value,
  tickets,
  agents,
  disabled = false,
  currentTicketId,
  showAssignee = true,
  autoFocusTitle = false,
  onChange,
}: Props<T>) {
  const matchingBlockers = tickets.filter((candidate) => {
    if (candidate.id === currentTicketId) return false;
    const query = value.blockerQuery.trim().toLowerCase();
    return (
      value.blockerIds.includes(candidate.id) ||
      !query ||
      `#${candidate.id} ${candidate.title}`.toLowerCase().includes(query)
    );
  });

  return (
    <>
      <label>
        Title
        <input
          value={value.title}
          disabled={disabled}
          onChange={(event) => onChange({ title: event.target.value } as Partial<T>)}
          maxLength={200}
          required
          autoFocus={autoFocusTitle}
        />
      </label>
      <label>
        Description
        <textarea
          value={value.description}
          disabled={disabled}
          onChange={(event) => onChange({ description: event.target.value } as Partial<T>)}
          rows={7}
        />
      </label>
      {showAssignee && (
        <label>
          Active assignee
          <select
            value={value.assignee}
            disabled={disabled}
            onChange={(event) => onChange({ assignee: event.target.value } as Partial<T>)}
          >
            <option value="">Unassigned</option>
            {agents.map((agent) => (
              <option value={agent.id} key={agent.id}>
                {agent.name}
              </option>
            ))}
          </select>
        </label>
      )}
      <fieldset className="blocker-selector" disabled={disabled}>
        <legend>Blockers</legend>
        <label>
          Search tickets
          <input
            type="search"
            value={value.blockerQuery}
            onChange={(event) =>
              onChange({ blockerQuery: event.target.value } as Partial<T>, false)
            }
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
                checked={value.blockerIds.includes(candidate.id)}
                onChange={(event) =>
                  onChange({
                    blockerIds: event.target.checked
                      ? [...value.blockerIds, candidate.id]
                      : value.blockerIds.filter((id) => id !== candidate.id),
                  } as Partial<T>)
                }
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
    </>
  );
}

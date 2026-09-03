import { useEffect, useState, type FormEvent } from "react";
import { api } from "../api";
import type { Agent, Ticket, TicketState } from "../types";

const columns: { state: TicketState; label: string }[] = [
  { state: "todo", label: "To do" },
  { state: "in_progress", label: "In progress" },
  { state: "done", label: "Done" },
];
interface Props {
  tickets: Ticket[];
  agents: Agent[];
  onChanged: () => Promise<void>;
  onError: (error: unknown) => void;
}

export function KanbanBoard({ tickets, agents, onChanged, onError }: Props) {
  const [title, setTitle] = useState("");
  async function create(event: FormEvent) {
    event.preventDefault();
    try {
      await api.createTicket(title);
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
        <button type="submit">Create ticket</button>
      </form>
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
                  tickets.filter((ticket) => ticket.state === column.state)
                    .length
                }
              </span>
            </h2>
            {tickets
              .filter((ticket) => ticket.state === column.state)
              .map((ticket) => (
                <TicketCard
                  key={ticket.id}
                  ticket={ticket}
                  agents={agents}
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
  onChanged,
  onError,
}: {
  ticket: Ticket;
  agents: Agent[];
  onChanged: () => Promise<void>;
  onError: (error: unknown) => void;
}) {
  const [title, setTitle] = useState(ticket.title);
  const [description, setDescription] = useState(ticket.description);
  const [state, setState] = useState(ticket.state);
  const [assignee, setAssignee] = useState(
    ticket.assignee_id?.toString() ?? "",
  );
  useEffect(() => {
    setTitle(ticket.title);
    setDescription(ticket.description);
    setState(ticket.state);
    setAssignee(ticket.assignee_id?.toString() ?? "");
  }, [ticket]);
  async function save(event: FormEvent) {
    event.preventDefault();
    try {
      await api.updateTicket(ticket, {
        title,
        description,
        state,
        assignee_id: assignee ? Number(assignee) : null,
      });
      await onChanged();
    } catch (error) {
      onError(error);
    }
  }
  return (
    <details className="ticket-card">
      <summary>
        <span>#{ticket.id}</span>
        {ticket.title}
      </summary>
      <form onSubmit={save}>
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
          Agent
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
        <button type="submit">Save</button>
      </form>
    </details>
  );
}

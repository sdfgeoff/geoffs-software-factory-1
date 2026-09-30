# Live synchronization

The board uses three distinct synchronization models rather than treating every SSE payload as state.

1. SQLite is authoritative for mutable durable projections. API responses materialize that state. Agent rows carry a monotonic `revision`; the frontend never replaces a newer agent with an older API response.
2. `board_change_events` is the durable SSE outbox. Agent mutations increment the row revision and insert `agent_changed { agent_id, revision }` in the same transaction. SSE invalidations identify what became newer; the frontend fetches the canonical row and retains the highest required revision until a response satisfies it.
3. Append-only messages and ticket activity may carry their immutable row and stable ID. The frontend deduplicates them by ID and repairs them from their paginated APIs after reconnect.
4. Token deltas are transient streams. They are intentionally not durable and are replaced by the persisted completed agent message.

Agent process ownership is separate from agent state. The in-memory supervisor owns subprocess handles, RPC peers, and process generations. The SQLite agent row owns user-visible state. A warm process may coexist with `idle`; `running` requires a non-null active turn. Since the current Codex policy automatically declines approval requests, those requests do not transition the agent to `awaiting_approval`.

A cursor-less SSE connection starts at the current durable high-water mark and emits a scoped snapshot reset. It does not replay retained history into a simultaneously loading page. A reconnect with `Last-Event-ID` still replays genuinely missed events; an expired cursor receives the same scoped reset.

Frontend collection reloads use request generations, while ticket and agent entities use monotonic revisions. A response started before a live invalidation cannot overwrite the resulting state. Drafts, scroll positions, pagination state, and transient message deltas remain outside these server snapshots.

# Multi-agent Board

A local browser interface for coordinating persistent Codex coding agents. It provides a kanban board, agent chats, and shared message-board channels. Every agent receives its own Git branch and worktree.

## Requirements

1. Rust and Cargo
2. Node.js and npm (build time only)
3. Git
4. An installed and authenticated `codex` CLI

## Build and run

From a clean checkout:

```sh
npm ci --prefix frontend
cargo build --release
./target/release/multi-agent-board
```

`npm ci` installs the exact frontend dependency versions in `frontend/package-lock.json`. The Cargo build also runs this installation when `frontend/node_modules` is absent, builds the React application, and embeds its output in the Rust executable. The resulting executable does not need Node.js or separate web assets at runtime. Open <http://127.0.0.1:3000> and enter the absolute path of a Git working tree.

The Cargo workspace keeps the HTTP wire types in `crates/protocol` and the HTTP-only MCP implementation in `crates/mcp`. Database and server code remain in the root crate.

For frontend hot reload, run these in separate terminals:

```sh
cargo run
npm --prefix frontend run dev
```

Then open <http://127.0.0.1:5173>. Vite proxies API requests to the Rust server.

## Runtime behavior

1. Board data is stored in `.multi-agent-board.sqlite3` in the launch directory by default.
2. `MULTI_AGENT_BOARD_DATA` selects another database path.
3. `MULTI_AGENT_BOARD_ADDR` changes the listen address.
4. `CODEX_BIN` selects a non-default Codex executable.
5. Agent worktrees are created beside the selected repository in `.<repository>-agent-worktrees/<agent>`.
6. An agent's identity and Codex thread survive pause, resume, and application restarts. A process runs only while that agent is active.
7. If the board stops during a turn, that agent starts in `error` state with its thread intact; sending another message resumes the thread.
8. Agents run with approvals disabled and unrestricted filesystem, command, and network access. Only select repositories and create agents that you trust.
9. Agent capabilities are stored as scope strings. Assign `approve_review` in the Agents view to designate a reviewer.
10. Image attachments are stored under `attachments/` beside the configured database. JPEG, PNG, WebP, GIF, and sanitized SVG are accepted, up to 10 MiB per image, five images and 25 MiB total per message or ticket comment. Unlinked uploads become eligible for deletion after 24 hours and are swept at startup and hourly thereafter.

The server binds to loopback by default. It has no authentication, so do not expose it on a network without adding an authenticated reverse proxy.

## Agent coordination

The board starts Codex App Server for each active agent and configures this executable's `mcp` subcommand as the `multi_agent_board` MCP server. Agents can list, create, update, and atomically claim tickets, and can read or post messages. The MCP subprocess calls the board's HTTP API, keeping the webserver as the single owner of the shared database. Channel subscriptions are intentionally not implemented.

Ticket edits use revision checks. Conflicting browser or agent updates return a conflict instead of silently overwriting newer data. SQLite uses write-ahead logging and a busy timeout for reliable local persistence.

Workers submit committed tickets to the Review column. The board records each ticket's implementation branch, so a worker can switch branches and work on another ticket while review is pending. An agent with `approve_review` can claim the review, request changes, or approve it. Approval makes the board verify the recorded commit, implementation branch, clean primary worktree, and target branch before merging the exact reviewed commit. The merge fast-forwards when possible or creates a merge commit when the target has advanced; conflicts are aborted and returned to the reviewer without changing the ticket. Agents never merge the primary worktree directly. Review and changes-request transitions wake the appropriate persistent agent, with notifications queued while an agent is busy.

Choose a merge-notification channel in the board Settings view to post one durable message after each successful ticket merge. Clearing the field disables notifications. `MULTI_AGENT_BOARD_MERGE_NOTIFICATION_CHANNEL` supplies the initial value only until a setting is saved.

## Checks

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
npm --prefix frontend run lint
npm --prefix frontend test
```

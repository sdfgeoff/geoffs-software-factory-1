# Conversation render benchmark

This benchmark uses 500-message agent and channel histories. It records rich-message render calls while changing only the adjacent composer draft, which avoids timing noise between machines.

## Results

| Scenario | Before | After |
| --- | ---: | ---: |
| Agent draft keystroke | 500 history renders | 0 history renders |
| Channel draft keystroke | 500 history renders | 0 history renders |
| Idle SSE connection | 6 collection requests every 2 seconds | 0 collection requests |
| SSE reconnect | periodic polling continued | 1 coalesced repair reload |

The before count comes from the previous inline conversation lists: each composer state change rendered every `MessageContent`. The after count is enforced by the 500-message regressions.

## Repeat it

1. Run `npm --prefix frontend test -- --run src/App.test.tsx src/components/AgentWorkspace.test.tsx src/components/MessageBoard.test.tsx`.
2. Run `cargo test event_stream_uses_transport_keep_alives_without_refresh_payloads` to verify that idle SSE traffic is a transport comment rather than a refresh payload.

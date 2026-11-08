# 0038. Chat transcript stream for agents without a terminal
Status: Accepted
Date: 2026-11-08
Accepted: 2026-11-09 (maintainer's explicit approval in chat)

## Context
L2 and L3 agents (ACP, adapters) have no terminal. Their adapter already reports `Chat` items to the node (text, tool calls, plans,
diffs; `AdapterEvent::Chat`), but the node drops them: clients can see an agent's state, approvals, files and changes, and can send
it a request (`agent.prompt`), but not read what it answered. The console shows a prompt box and says so.

## Decision
Mirror the terminal design: a chat transcript is a **stream the node keeps and serves on demand, not events in the hub's store**
(ADR-0022: bulk output stays out of the event store; nodes stay autonomous).

- The node keeps, per chat agent, a bounded ring of entries (last 500 entries or 256 KiB, whichever is smaller; older entries are
  dropped). The user's own prompts are entries too, so the transcript reads as a conversation.
- New command `chat.subscribe` (routed by the hub like `terminal.subscribe`, ADR-0029): the node answers with a snapshot of the
  ring, then pushes new entries on the same dynamic stream until the client closes it.
- Messages, all new names and numbers (ADR-0006), nothing existing changes:

```
message ChatSubscribe { string agent_id = 1; }
message ChatEntry {
  uint64 entry_id = 1;        // increases per agent; a client ignores one it already has
  uint64 at_ms = 2;
  ChatRole role = 3;          // CHAT_ROLE_AGENT | CHAT_ROLE_USER
  ChatKind kind = 4;          // CHAT_KIND_TEXT | _TOOL_CALL | _PLAN | _DIFF
  string text = 5;            // text, tool name + arguments as JSON, plan lines joined by "\n", or the unified diff
  string path = 6;            // for a diff: the file
}
message ChatSnapshot { repeated ChatEntry entries = 1; }
```
`CommandRequest.command` gets `ChatSubscribe chat_subscribe = 19;` (43 is taken by `worktree_remove`). Frames on the dynamic stream carry `ChatSnapshot` (frame kind 10) first and
`ChatEntry` (frame kind 11) after, registered in `docs/specs/protocol.md` beside the terminal ones (kind 3 closes a relay and must not be reused).

- A long answer arrives as the adapter sends it, one entry per item (adapters already send whole items; merging them can be added
  without a protocol change), and the node caps one entry at 16 KiB.
- The text is untrusted (a model wrote it): clients render it as plain text or sanitized Markdown, never as HTML.
- Secrets stay on the node as always; the transcript is not written to disk and is lost when the node restarts, like a terminal's
  screen. Persisting transcripts is a separate decision.

## Consequences
- Node: a ring per chat agent fed from `AdapterEvent::Chat` and `agent.prompt`; the control server gets a stream like the
  terminal one. Hub: routes the new command (no state). Web: a transcript view in the pane above the prompt box.
- The hub never stores or sees transcripts unless a client asks, so hub memory and the event store are unaffected.
- Old nodes answer `protocol.capability_missing`; the client keeps showing the prompt box without a transcript.

## Alternatives considered
- Events in the store (`ChatItemAdded`): simple and gives history across hub restarts, but puts unbounded model output in the
  durable log, which the performance ADR forbids for terminal bytes and which would also leak into every client's snapshot.
- Poll `chat.read`: no stream machinery, but latency and wasted traffic for a live conversation.

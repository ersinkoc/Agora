# 0029. Command routing and terminal relay through the hub
Status: Proposed
Date: 2026-11-08

## Context
A client (web, TUI) is connected to the hub and sees every node's agents (ADR-0027). To act on one (start or stop an agent,
answer an approval, type into a terminal) the command must reach the node that owns it. Today a node accepts commands only
on its local socket, and the hub has no path from a client's command to a node's connection. Terminal bytes have the same
gap: `docs/architecture.md` says the hub relays terminal streams without decoding or storing them, but no wire rule says how.

Constraints that are already decided and stay: nodes are autonomous (a command that cannot be delivered fails, nothing else
does); the hub cannot widen node policy and sensitive actions need user-signed approvals verified on the node (ADR-0020,
ADR-0028); terminal bytes never go through the event store (ADR-0022); input outranks output.

## Decision
**Routed commands.** A client on an authenticated connection may send `CommandRequest` frames on stream 1. Append one field to
the published message (new field number; nothing existing changes, ADR-0006):

- `CommandRequest.target_node = 3` (string): the node that must run it. Required for every command except those the hub
  answers itself; absent or unknown node → `node.unknown` (new); node offline → `node.offline` (existing).

The hub forwards the request unchanged except `request_id`, which it rewrites to a hub-chosen id so two clients cannot
collide, and maps the answer back. It adds a deadline (default 30 s, `node.timeout`). The node handles it exactly like a
local-socket command: same handler, same policy, same approval rules. `idempotency_key` is passed through, so a retried
command after a reconnect does not repeat its effect on the node.

A node connection therefore carries stream 1 as well as stream 2. Frame kinds on stream 1: 0 `CommandRequest`, 1
`CommandResponse` (hub → node requests, node → hub responses; the same kinds client ↔ hub).

**Who may send what.** Token-authenticated clients may send any command. The hub never fabricates or edits a command's
content, and `approval.resolve` carries the user's signature, which the node verifies; a compromised hub can refuse to
deliver but cannot approve for the user.

**Terminal relay.** `terminal.subscribe` (existing message, now with `target_node`) makes the hub open a dynamic stream
(`FIRST_DYNAMIC_STREAM` and up) toward the node and a dynamic stream toward the client, and copy frames between them in
both directions without decoding the payload: snapshot, output, resize and exit frames from the node; input and resize from
the client. Stream ids are per connection; the hub keeps the pairing. Closing either side closes both.
- Output is never stored. A client that falls behind is not waited for: the hub drops output for it and the node sends a
  fresh snapshot (the node already does this for local clients), so one slow tab cannot slow an agent or another client.
- Input is forwarded ahead of queued output on the node connection.
- Several clients may subscribe to one terminal; each gets its own dynamic stream.

## Consequences
- The node uplink grows a command side (receive on stream 1, answer on stream 1) and terminal-stream bridging to its local
  control handler; both reuse the existing handlers, so no new node capability appears.
- Two new error codes (`node.unknown`, `node.timeout`) and one new field; golden fixtures are added,
  none edited.
- The hub holds per-connection pairing state for relays; it is bounded by open terminals and dropped with either side.
- Direct client ↔ node terminal paths (architecture.md) remain possible later and need no change here.

## Alternatives considered
- Derive the node from ids inside each command (`laptop/agent-1`): `agent.start` has no agent id, and mixing routing into
  every message makes the rule easy to get wrong. One explicit field is simpler.
- A separate hub-only command envelope: duplicates every command; a field on the existing message is smaller.

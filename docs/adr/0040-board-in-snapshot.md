# 0040. Shared boards in the client snapshot, and leases kept across a hub restart
Status: Accepted
Date: 2026-11-09
Accepted: 2026-11-09 (maintainer's explicit approval in chat: "her şeyi onaylıyorum", answering the list of Proposed ADRs 0040 and 0034)

## Context
ADR-0039 gave shared projects a board on the hub. The console learns about tasks and leases only from `TaskChanged`,
`LeaseGranted` and `LeaseReleased` events, and a client that connects (or reloads) gets a `Snapshot` that carries nodes, checkouts,
worktrees, agents, terminals and approvals but no board. So a reloaded window shows an empty board until something changes. Leases
also live only in the hub's memory, so a hub restart drops them while their holders still think they hold them.

## Decision
1. **`Snapshot` gains two repeated fields** (append-only, ADR-0006): `repeated TaskSnapshot tasks = 8` (`task_id`, `project_id`,
   `title`, `claimed_by_agent_id`, `done`, all as in `TaskChanged`) and `repeated LeaseSnapshot leases = 9` (`agent_id`, `path`,
   `expires_at_ms`). Old clients ignore them; the hub fills them from the boards it holds. New golden fixtures are added, none edited.
2. **The hub view tracks board state** (`View.tasks`, `View.leases`) by applying the same events it already publishes, so the snapshot
   and the live stream agree and a rebuilt view (from the event log) has them too.
3. **Leases are stored with the tasks** (same settings key family, `board.<project>`) with their expiry; on restore, leases whose TTL has
   passed are dropped and the rest kept, so a hub restart does not silently free a path its holder still believes it has.

## Consequences
- Schema: two additive fields and fixtures; `schema-breaking` CI stays green.
- Hub: projection and snapshot builder change; board store writes leases too.
- Web: `fromSnapshot` reads the new fields; the board view is then correct after a reload.
- Tests: snapshot round trip with a board, a view rebuilt from the log, a restart that keeps an unexpired lease and drops an expired one.

## Alternatives considered
- Ask the hub for the board with a command when the window opens: no schema change, but a second code path and a window between the
  snapshot and the answer in which events can be missed.
- Replay the board events as part of the stream after the snapshot: also a protocol change, and it makes the stream length depend on
  board history.

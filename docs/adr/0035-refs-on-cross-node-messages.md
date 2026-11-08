# 0035. `refs` on messages between nodes
Status: Proposed
Date: 2026-11-08

## Context
ADR-0031 lets an agent message `<node>/<agent>`, but `MeshPost` carries only `to` and `body`. A local `send_message` may carry up
to 20 `refs` (paths, commits, task ids); on a remote hop they are dropped. A path or task id is only meaningful on the node that
wrote it, so naive forwarding would let the receiver read `src/lib.rs` as a path in its own checkout, which is wrong and, for a
confused agent, risky. Commits are the one kind that mean the same everywhere (within one repository).

## Decision
Add one field to the published message (new number; ADR-0006): `MeshPost.refs = 5` (repeated string), the sender's refs as
written. Nothing existing changes.

The receiving broker applies the same limits as for a local message (count, length) and stores each ref **qualified by origin**:
`<origin>:<ref>`, for example `laptop:src/auth.rs`. The `inbox` tool returns the qualified form, so an agent can tell a ref that
belongs to another machine from one on its own. A ref that is already qualified is qualified again (`laptop:`...), never
trusted as local. `origin` is the hub-vouched value from ADR-0031, not anything the sender wrote.

Refs stay inert data: the broker never opens, resolves or fetches them, and a qualified ref cannot confer a file lease or any
permission. Sending file contents between nodes is a separate feature (a later ADR); until then refs only point.

## Consequences
- One new field plus a golden fixture; `docs/specs/protocol.md` and `mesh-tools.md` updated with the change.
- Old nodes ignore field 5 (unknown fields are skipped), so a message from a new node reaches an old one without refs.
- The web window can show `<node>:<path>` as plain text; it does nothing with it.

## Alternatives considered
- Forward refs unqualified: ambiguous and can be mistaken for a local path.
- Drop refs on remote hops (today): honest but loses the commit-sharing case that makes cross-node handoffs useful.
- Resolve and copy referenced files through the hub: moves repository content into the hub and needs its own policy.

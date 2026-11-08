# 0031. Mesh messages between agents on different nodes
Status: Proposed
Date: 2026-11-08

## Context
The mesh (`agora-mesh`, ADR-0012, `docs/specs/mesh-tools.md`) works inside one node: a broker per node, one MCP endpoint per agent.
The product promise is that agents on different machines coordinate the same way. Two things are missing: an address for an agent
on another node, and a path for a message to get there. Nodes only emit events to the hub and answer commands the hub routes to
them (ADR-0029); a node cannot send anything to another node. Mesh content is untrusted data (AGENTS.md invariant), and the hub
must not be able to forge approvals; it may, however, route.

## Decision
**Addresses.** An agent on another node is addressed `<node id>/<local agent id>` (the hub-wide key already used in views and
snapshots). A bare id means "on this node", as today. Channels stay node-local until a later ADR.

**Path.** A node's mesh broker that is asked to deliver to `<other node>/<agent>` sends a `MeshPost` command to the hub on its own
connection (stream 1, kind 0, the same frames a client uses) with `target_node = <other node>`. This is the only command a node may
send to the hub; any other is refused. The hub routes it exactly like a client command (ADR-0029): to the target node's connection,
under a hub-chosen request id, and returns the answer. The receiving node's broker delivers it to its local agent with provenance
and the usual limits (rate, size). Delivery is at most once; the sender's agent gets the result (`ok` or an error such as
`node.offline`, `mesh.recipient_unknown`).

**Provenance the hub vouches for.** Add one field to the published message (new number; nothing existing changes, ADR-0006):
`MeshPost.origin = 3` (string): the node id the hub saw the post arrive from. The hub sets it from the connection's certificate and
overwrites anything the node wrote there, so a node cannot claim to speak for another node. The receiving broker stamps the
message `from = <origin>/<sender agent id>`; the sender agent id is whatever that node says, which is all a node can know.

**What this does not change.** The hub still cannot widen node policy or answer approvals; messages carry text only and are
untrusted on arrival like any mesh message. A node that never enables cross-node mesh (policy `mesh.remote = false`, default
`true` for nodes the user enrolled) neither sends nor accepts remote posts. Nothing about this path touches the event store except
the existing `MessagePosted` events each node already records for its own side.

## Consequences
- Node uplink grows a small command sender (node -> hub request, answered on stream 1); the hub's node connection accepts that one
  command and forwards it with the router it already has.
- One new field and golden-fixture coverage; no new messages; one new node policy key.
- Broadcast channels, tasks and leases across nodes are out of scope here and need their own decision (ownership and ordering).

## Alternatives considered
- A hub-side broker that owns all mesh state: simplest to reason about for channels and tasks, but it moves per-project
  coordination data and message bodies into the hub for the local-only case too and breaks "nodes are autonomous" (a hub outage
  would stop agents on the same machine from talking). Local stays local; the hub only carries remote hops.
- Direct node-to-node connections: needs reachability between nodes (often impossible behind NAT) and a second trust path.

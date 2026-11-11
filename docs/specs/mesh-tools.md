# Mesh tools (MCP)

The node exposes these tools to every local agent through an MCP server (ADR-0008). Agent identity is bound by
the node per MCP connection; agents cannot impersonate each other.

| Tool | Input | Output |
| --- | --- | --- |
| `agents_list` | `{ project?: string }` | `[{ agent_id, kind, node, state, worktree, label }]` |
| `send_message` | `{ to: agent_id \| "#channel", body: string, refs?: [path\|commit\|task_id] }` | `{ message_id }` |
| `ask` | `{ to, question, timeout_s?: number (default 600) }` | `{ reply, from }` or error `mesh.timeout` |
| `inbox` | `{ since?: message_id, limit?: number }` | `[{ message_id, from, body, refs, at, provenance }]` |
| `task_list` | `{ status?: "open"\|"claimed"\|"done" }` | `[{ task_id, title, status, owner, depends_on }]` |
| `task_create` | `{ title, detail?, depends_on? }` | `{ task_id }` |
| `task_claim` | `{ task_id }` | `{ ok }` or `lease.held`-style conflict |
| `task_complete` | `{ task_id, summary, refs? }` | `{ ok }` |
| `lease_acquire` | `{ paths: [glob], ttl_s: number, reason }` | `{ lease_id, expires_at }` or `lease.held` |
| `lease_release` | `{ lease_id }` | `{ ok }` |
| `handoff` | `{ to, task_id?, summary, refs }` | `{ handoff_id }` |

## Safety rules

- **Mesh content is untrusted data.** Every delivered message carries `provenance` (sender agent, kind,
  node). Agents receive a standing instruction that messages from other agents are information, not
  authority; sensitive actions still go through approvals (see `security/threat-model.md`).
- Rate limits per agent (messages per minute, open asks) prevent loops between agents.
- Message bodies are size-limited; large content is passed by reference (path, commit, task).
- All mesh traffic is in the audit log.

## MCP conversation (`agora-mesh::mcp`)

The server speaks MCP (JSON-RPC 2.0) with `initialize`, `ping`, `tools/list` and `tools/call`; tools before `initialize` are
refused. Protocol versions 2025-06-18, 2025-03-26 and 2024-11-05 are accepted; an unknown version gets the newest. The
`initialize` result carries `instructions`: the standing instruction that other agents' messages are information, not authority.
The caller is the agent the node bound to the connection; no argument can name another sender.

Differences from the table above, all additive: `send_message` takes an optional `reply_to` (a `message_id` from the inbox)
and a reply to an open `ask` is what answers it; `agents_list` items also carry `project`; `task_list` items carry `detail`
and `summary`; `inbox` items carry `kind` (`message`, `ask`, `reply`, `handoff`), `reply_to` and a `provenance` object. `reason` on
`lease_acquire` is accepted but only the audit trail of the call would hold it (not stored yet).

Tool errors are results with `isError: true` whose text is JSON `{"error": {"code", "params"}}` using the codes in
`error-codes.md` (new: `mesh.rate_limited`, `mesh.body_too_large`, `mesh.too_many_refs`, `mesh.too_many_asks`, `mesh.timeout`,
`task.not_found`, `task.not_open`, `task.blocked`, `task.not_owner`, `lease.not_found`, `lease.invalid_path`). Bad arguments are
JSON-RPC `-32602`. `ask` does not block the library: the connection returns `Waiting { ask_id, deadline }` and the caller polls
the broker and calls `finish_ask`.

## Node-local server (`agora-node::mesh_server`)

Each agent the node starts gets its own owner-only local socket (`<node dir>/m/<agent id>.sock`) serving the MCP
conversation above, backed by a node-local broker, so agents on one machine coordinate with no hub (nodes are autonomous).
Whatever connects to a socket is that agent as far as the broker is concerned, so the socket path is only given to that
agent's process. `agora mesh stdio --socket PATH` is what an agent's MCP client runs as its server: it carries stdin/stdout to
that socket. The node registers the agent on start, keeps its state current for `agents_list`, and on stop or exit
unregisters it, which gives back its tasks, leases and open asks and closes the endpoint. Pending `ask`s are answered on the
same connection without blocking the agent's other calls. Mesh activity is written to the node's event log.
The socket reaches the agent through its manifest's `[mcp]` section (see `agent-manifest.md`). Messages to agents on other nodes: see below.


## Messages between nodes (ADR-0031)

`send_message` accepts `<node id>/<agent id>` as `to`. A bare id means this node (so does your own node's name in front). The
broker queues the message for the node's uplink, which sends a `MeshPost` command to the hub; the hub forwards it to the target
node like a client command (ADR-0029) and writes `origin` (the node the post arrived from, by certificate) over whatever the
node wrote. The target broker delivers it to the local agent with `provenance.from = "<origin>/<sender agent id>"` and
`provenance.node = <origin>`; answering is a `send_message` to that address. Delivery is at most once. `send_message`
returns once the broker accepted the message (limits and rate are checked there); if it does not arrive, the sender's inbox
receives a notice of kind `delivery_failed` whose body is the error code (`node.offline`, `node.unknown`, `node.timeout`,
`mesh.recipient_unknown`, `mesh.rate_limited`, ...). Plain messages cross nodes; `ask` stays node-local, and a `#channel` message on a shared project reaches every other running agent of the project on any node (below), and a node may have 64 posts waiting at the hub. Tasks, leases and
`handoff` cross nodes through the shared board below, for a project that is on two or more online nodes. The machine's owner can turn it off:
`<node data dir>/policy` with the line `mesh.remote = false` (default `true`; a value that is not exactly `true` or `false` counts as
`false`, so a typo never widens the node). Then messages to other nodes fail with a `delivery_failed` notice `node.policy_denied`
and messages from other nodes are refused with the same code; messages on this node are unaffected. The hub cannot change this
file. `refs` travel with a cross-node message (ADR-0035, `MeshPost.refs`) and the receiver stores each as `<origin node>:<ref>` (`laptop:src/auth.rs`; one that is already qualified is qualified again), so an agent can tell a path on another machine from a local one; refs stay inert text, within the usual limits, and confer no lease or permission. The web window lists recent messages and marks the ones between machines.

## Shared board (ADR-0039)

Project membership includes both startup checkouts and owner-selected workspaces (`WorkspaceRegistered`, ADR-0052), so
folders selected after a node starts participate without a daemon restart. Membership is deduplicated by enrolled node ID;
host names and directory names do not establish shared identity. Offline and unknown nodes do not count. A single-node
project with no unfinished shared work continues on its local broker.

When a project has checkouts on two or more online nodes, `task_*`, `lease_*` and `handoff` are decided by the hub, which keeps one
board per project and is its only writer: a claim has exactly one winner and a lease is all-or-nothing across nodes. The agent's
tool calls do not change. The node sends each of these tools to the hub as `BoardCall{project_id, tool, arguments_json}` (the hub
writes `origin` from the certificate and names agents `<node>/<agent>`) and the hub answers `BoardResult{shared, result_json,
is_error}`. `shared = false` means the project is not on two online nodes and its board holds no task that is not done and no live lease (ADR-0039 amendment A: a board in use stays in use when a node drops): nothing happened and the node runs the tool on its own
broker, so a project that lives on one machine never needs the hub. The board's agents are the project's running agents in the
hub's view; one that ended gives its tasks and leases back, and a lease also ends at its TTL if its node vanishes.

A `handoff` to `<node>/<agent>` moves the task and sends the receiver a message from the giver in one step; if the receiving node
is offline it fails with `node.offline` and the task does not move. When the hub cannot be reached, a project this node knows to
be shared fails with `board.unavailable` (the agent's other tools and its worktree keep working); a project it does not know to be
shared stays on the local broker. Tasks and leases made locally before a project became shared stay local. Not done yet: boards
keep their tasks in the hub's SQLite file (settings key `board.<project>`, so they are in its backup); leases are stored with them (ADR-0040) and a restart keeps those whose TTL has not passed (a restored lease has no id, so it ends at its TTL), a task claimed by an agent that is gone after a restart is given back, a message to a
`#channel` is sent as `BoardCall{tool: "send_message"}`; the hub makes one copy per other running agent of the project (any online node), each arriving as a message from the sender with the channel name in front of the text (`[#dev] ...`), best effort and at most once, within the usual body and ref limits (the tool returns `{ delivered_to }` instead of `{ message_id }` in this case). Channel history is not kept and `inbox` on the receiver is how it is read. The web console shows each project's board (tasks with their owner, held paths) in its right column, built from the hub's
`TaskChanged`/`LeaseGranted`/`LeaseReleased` events; a window that opens or reloads gets the boards in its `Snapshot` (`tasks`, `leases`, ADR-0040).

## Implementation status
`agora-mesh::broker::Mesh` implements the tools' logic as deterministic in-memory state: registry, messages with
broker-stamped provenance, project-scoped `#channels`, inbox cursor, `ask` answered by a reply to its id with timeouts,
the task board with dependencies and owners, all-or-nothing leases over the core lease table, handoff (the message goes out
first so a refused handoff moves nothing), and cleanup when an agent ends. Limits: 16 KiB bodies, 20 refs, 30 messages per
minute per agent, 5 open asks. Every change queues a protocol event for the audit log. The transport (a local socket per agent, `agora mesh stdio`), the injection into each agent's MCP configuration (the manifest's `[mcp]`), routing across nodes (ADR-0031) and the shared board with its persistence (ADR-0039, ADR-0040) are described in the sections above. Not done: delivery fallback to the integration level's input path.

# Mesh tools (MCP)

The node exposes these tools to every local agent through an MCP server (ADR-0008). Agent identity is bound by
the node per MCP connection; agents cannot impersonate each other.

| Tool | Input | Output |
| --- | --- | --- |
| `agents_list` | `{ project?: string, scope?: "project"\|"hub" }` | `[{ agent_id, kind, node, state, worktree, label }]` (more fields: see "Awareness"; `scope` is ADR-0112's, see "The hub-wide view") |
| `workspace_status` | `{}` | `{ workspace, project, machine, machines_online, partial, agents, same_workspace, working, blocked, idle, open_tasks, leases, summary }` (see "Awareness") |
| `machines_list` | `{}` | `{ machines: [{ id, online, os, build, agents_running, last_seen_ms }], partial }` (ADR-0112; listed only when the node offers it, see "The hub-wide view") |
| `send_message` | `{ to: agent_id \| "#channel", body: string, refs?: [path\|commit\|task_id] }` | `{ message_id }` |
| `ask` | `{ to, question, timeout_s?: number (default 600) }` | `{ reply, from }` or error `mesh.timeout` |
| `inbox` | `{ since?: message_id, limit?: number }` | `[{ message_id, from, body, refs, at, provenance }]` |
| `inbox_wait` | `{ since?: message_id, limit?: number, timeout_s?: number (default 600, at most 3600) }` | as `inbox`, once at least one message is there; `[]` when the time runs out (see "Waiting for messages") |
| `task_list` | `{ status?: "open"\|"claimed"\|"done", task_id? }` | `[{ task_id, title, status, owner, depends_on }]`; with `task_id` that task alone, or `[]` (ADR-0095) |
| `task_create` | `{ title, detail?, depends_on? }` | `{ task_id }` |
| `task_claim` | `{ task_id }` | `{ ok }` or `lease.held`-style conflict |
| `task_complete` | `{ task_id, summary, refs? }` | `{ ok }` |
| `lease_acquire` | `{ paths: [glob], ttl_s: number, reason }` | `{ lease_id, expires_at }` or `lease.held` |
| `lease_release` | `{ lease_id }` | `{ ok }` |
| `handoff` | `{ to, task_id?, summary, refs }` | `{ handoff_id }` |
| `agent_start` | `{ kind, prompt?, folder?, worktree?: bool, node?, settings?: { id: value }, options?: [id] }` | `{ agent_id }` after the user's decision (ADR-0057) |
| `agent_wait` | `{ agent_id, state: "idle"\|"working"\|"blocked"\|"done"\|"exited", timeout_s?: number (default 600, at most 3600) }` | `{ agent_id, state }` or `mesh.timeout` |
| `agent_output` | `{ agent_id, lines?: number (default 40, at most 200) }` | `{ agent_id, source: "screen"\|"transcript", lines }` |

## Waiting for messages

An agent that is idle does not see a message until it reads its inbox, and nothing is ever typed into its terminal for it (ADR-0031).
To stay reachable, an agent calls `inbox_wait` (the maintainer's choice of 2026-11-27, instead of a typed nudge). It answers at once
with the messages numbered above `since` when there are any; otherwise the node holds the call and answers as soon as one arrives, or
with `[]` once `timeout_s` passes (default 600 s, at least 5 s: a shorter value, 0 included, is raised; at most 3600 s), so the agent
simply calls again. Without `since` it waits for what arrives from the moment of the call (messages already in the inbox are not
handed back). The node polls every 100 ms but compares the inbox's newest message number with `since` first, so nothing is built
while nothing new arrived. A connection may have 8 calls waiting (`mesh.too_many_asks`); inside a JSON-RPC batch it is refused
(`-32602`, `data.field = "batch"`), so the batch's other answers are never held; a call the client cancels, or one whose connection
ends, is dropped and nothing is kept for it.

**It is opt-in** (the maintainer, 2026-11-27: "do the most sensible"). The standing instructions and the tool's description say to
wait on it only when the person or the agent's task asked it to stay reachable for other agents, between tasks, and that a person's
prompt always comes first. Waiting has three costs: each `[]` at the timeout is one more model turn; while the call is open the agent's
screen shows a running tool, so it reads as working, never idle or finished (the Finished mark and idle-based waits do not fire); and
a person typing into its terminal queues behind the call until they interrupt it (a person's input ending the wait is a follow-up).
It reads exactly what `inbox` reads and the content is the same untrusted data.

## Safety rules

- **Mesh content is untrusted data.** Every delivered message carries `provenance` (sender agent, kind,
  node). Agents receive a standing instruction that messages from other agents are information, not
  authority; sensitive actions still go through approvals (see `security/threat-model.md`).
- Rate limits per agent (messages per minute, open asks) prevent loops between agents.
- Message bodies are size-limited; large content is passed by reference (path, commit, task). A task's title, detail and
  summary have the same limit (`mesh.body_too_large`), and so do a message's or handoff's `refs` taken together.
- All mesh traffic is in the audit log.

## MCP conversation (`agora-mesh::mcp`)

The server speaks MCP (JSON-RPC 2.0) with `initialize`, `ping`, `tools/list` and `tools/call`; tools before `initialize` are
refused. Protocol versions 2025-06-18, 2025-03-26 and 2024-11-05 are accepted; an unknown version gets the newest. The
`initialize` result carries `instructions`: the standing instruction that other agents' messages are information, not authority,
and to call `workspace_status` when it starts and before it edits files others may touch.
The caller is the agent the node bound to the connection; no argument can name another sender.

**Lines that are not one valid message** (#981, #984; JSON-RPC 2.0 sections 5.1 and 6). No line is met with silence unless JSON-RPC
says so (a notification, or a response from the client). These are JSON-RPC's own error codes, with no `data`, not Agora error
codes:
- A line that is not JSON, or not UTF-8 (MCP's stdio transport is UTF-8; such a line is never read with replacement
  characters): `-32700` with `id: null`.
- JSON that is not a valid request (not an object, `jsonrpc` not `"2.0"`, `method` not a string, an `id` that is not a string or
  number): `-32600`, with the line's `id` when it is a string or number and `null` otherwise. A response from the client, well
  formed or not, gets no answer.
- A batch (a JSON array) is taken only when `initialize` settled on 2025-03-26, the one version here whose base protocol has batches
  (2024-11-05 does not define them, 2025-06-18 removed them). Then the answer is one array with the answers to its requests, in
  their order; notifications in it get none, a batch of only notifications gets no reply, `initialize` inside a batch is `-32600`
  (2025-03-26 forbids it), and a value in it that is not a valid request is answered inside the array as above. A batch waits for
  its slowest request (an `ask`, `agent_start` or `agent_wait` in it), while the connection's other calls are answered meanwhile;
  a request of it the client cancels (`notifications/cancelled`) is left out. Before `initialize`, or under any other version, a
  batch is refused: `-32600` for each request in it, each on its own line with its id, or one `-32600` with `id: null` when it
  holds no request.
- An empty batch, or one of more than 32 values (`agora_mesh::mcp::MAX_BATCH`), is one `-32600` with `id: null`, whatever the
  version, and nothing in it runs.

Bound: a batch is one line, so the node's 1 MiB line limit (a longer line closes the connection) bounds its size, and at most 32
messages run for one line. Each message of a batch is routed and checked exactly as if it came alone, in order, one after the
other: the node's spawn tools, the hub's board for a shared project, then the broker, with the same argument checks, size limits,
rate counts (`mesh.rate_limited`), open-ask and `agent_wait` limits, and the user's approval for each `agent_start`; batching skips
none of them and runs nothing in parallel. A batch still waiting holds at least one waiting call, so those limits also bound how many
batches can wait.

Differences from the table above, all additive: `send_message` takes an optional `reply_to` (a `message_id` from the inbox)
and a reply to an open `ask` is what answers it; `agents_list` items also carry `project` and the fields under "Awareness"; `task_list` items carry `detail`
and `summary`; `inbox` items carry `kind` (`message`, `ask`, `reply`, `handoff`), `reply_to` and a `provenance` object. `reason` on
`lease_acquire` is accepted but only the audit trail of the call would hold it (not stored yet).

**Message ids.** `message_id` is `m-<number>`, numbered by the node's broker, and `inbox(since)` returns the messages numbered above
`since`. Ids never go back across a node restart: before it hands out any id (messages, and the task, lease and handoff ids that
share its counter), each run of the broker reserves a range on disk (`<data>/sessions/mesh-ids`, beside the ledger, holds the end of
the last range) that starts at the largest of that end, the clock in microseconds and its own next id, and a run that nears the end
of its range reserves the next. So an agent kept across the restart (ADR-0079) still sees what arrives after the cursor it carried
over (and a task or lease id it holds is never given to something new), and a `since` from an earlier run, or one no message has,
just returns what is numbered above it (#965: the numbers started over at `m-1`, so a kept agent's cursor hid every new message).
When the range cannot be reserved (the file cannot be written, or exists and cannot be read as a number), the node and its agents
run on and `inbox`, `workspace_status`, `agents_list` and `task_list` answer, but the node's broker hands out no id: on it
`send_message`, `ask`, `handoff`, `task_create` and `lease_acquire` fail with `mesh.unavailable` and change nothing, a message from
another node is refused with it (its sender gets `delivery_failed` `mesh.unavailable`), and presence and `delivery_failed` notices
for this node's agents are not made. Calls that go through the hub instead (a `#channel` message or board call on a shared
project, below) are not numbered here and work as usual, except that a copy for an agent of this node is refused like any message
from another node. The node tries again every half second. Numbers are per node: the same number on two nodes names two messages.

When the receiving node and hub negotiate ADR-0109 for a cross-node task handoff, the recipient sees a durable `handoff` message. It shares the same
cursor as ordinary messages and keeps its original number if that process is taken up from the holder. Ordinary message traffic
does not evict it. A replacement process under the same agent id cannot see it. The `inbox` call still reads without removing.

Tool errors are results with `isError: true` whose text is JSON `{"error": {"code", "params"}}` using the codes in
`error-codes.md` (new: `mesh.rate_limited`, `mesh.body_too_large`, `mesh.too_many_refs`, `mesh.too_many_asks`, `mesh.timeout`,
`task.not_found`, `task.not_open`, `task.blocked`, `task.not_owner`, `lease.not_found`, `lease.invalid_path`). Bad arguments are
JSON-RPC `-32602`; an optional argument a tool uses that has the wrong type is a bad argument, never taken as omitted. `ask` does not block the library: the connection returns `Waiting { ask_id, deadline }` and the caller polls
the broker and calls `finish_ask`. If the originating transport ends first, its runtime calls the caller-bound
`Mesh::cancel_ask` for the asks it owns; another agent cannot cancel them.

## Node-local server (`agora-node::mesh_server`)

Each agent the node starts gets its own owner-only local socket (`<node dir>/m/<agent id>.sock`) serving the MCP
conversation above, backed by a node-local broker, so agents on one machine coordinate with no hub (nodes are autonomous).
Whatever connects to a socket is that agent as far as the broker is concerned, so the socket path is only given to that
agent's process. `agora mesh stdio --socket PATH` is what an agent's MCP client runs as its server: it carries stdin/stdout to
that socket. The bridge survives its connection ending (ADR-0075 decision 6; the node re-executing or restarting leaves the
socket path the same): it answers each request in flight (except one the client cancelled with `notifications/cancelled`, which gets no answer), and each request that arrives while it has no connection, at once with a
JSON-RPC error `-32000` whose `data` is `{ "code": "mesh.unavailable", "params": {} }` (the agent may repeat the call; a JSON-RPC batch gets one such error per request in it, as a batch when the node's answer to the client's `initialize` settled on 2025-03-26 and one line each otherwise, and notifications get no answer; the requests of a batch in flight when the connection ends are answered one line each; while it has no connection it answers a line that is not JSON or not a valid request with the same `-32700` / `-32600` the node would, and reads a batch's values as the node does: in a 2025-03-26 conversation a value that is not a valid request gets its `-32600` inside the array and a `notifications/cancelled` in the batch drops the answer to the request it names, and without batches a batch holding no request is one `-32600` with `id: null`; a batch's requests sent on a live connection are in flight, and in a 2025-03-26 conversation a cancellation inside the same batch takes its request out, as one on its own line does), keeps its
stdout open so the client's session stays, and reconnects to the same path with a wait that doubles from 50 ms to 2 s. A new
connection starts uninitialized on the node, so the bridge replays the client's own `initialize` request (and its
`notifications/initialized`) and discards the answer, exactly as at start; it carries nothing else over, and a peer that does not
answer `initialize` with a result is not given the client's calls. It retries while its client keeps stdin open and gives up (exit 1)
after 15 minutes without a connection. The bridge ends when its client's output closes. A bridge started by an older version does not do this and ends with its connection. The node
side is unchanged: the endpoint's owner-only access (and, on Windows, the per-agent identity) applies to every new connection.
The node registers the agent on start, keeps its state current for `agents_list`, and on stop or exit
unregisters it, which gives back its tasks, leases and open asks and closes the endpoint (its open connections are closed too, so
a client that stopped reading cannot hold the node up). Pending `ask`s are answered on the
same connection without blocking the agent's other calls. The connection owns its outstanding ask IDs independently
of its polling queue. On EOF, input/output failure or shutdown it discards those asks (including already answered ones)
without unregistering the agent or affecting another connection's requests. A `notifications/cancelled` from the client for a
waiting `ask`, `agent_wait` or `agent_start` drops that call alone, as the connection ending would: the ask is discarded, the wait
ends, the start request is withdrawn, and nothing is sent for it. A live connection still receives a completed
answer even if it starts another ask before polling that answer. Mesh activity is written to the node's event log.
The socket reaches a terminal agent through its manifest's `[mcp]` section (see `agent-manifest.md`). A structured agent (an L3
adapter, or an ACP agent through `agora acp-bridge`) joins the mesh the same way and gets the same server in `session/start`
`mcpServers` (`adapter-protocol.md`), which the bridge passes on to ACP `session/new`/`session/load`; so every tool above,
`agent_start`, `agent_wait` and `agent_output` included, works for it, and its `agent_start` waits for the user like any agent's.
Messages to agents on other nodes: see below.


## Messages between nodes (ADR-0031)

`send_message` accepts `<node id>/<agent id>` as `to`. A bare id means this node (so does your own node's name in front). The
broker queues the message for the node's uplink, which sends a `MeshPost` command to the hub; the hub forwards it to the target
node like a client command (ADR-0029) and writes `origin` (the node the post arrived from, by certificate) over whatever the
node wrote. The target broker delivers it to the local agent with `provenance.from = "<origin>/<sender agent id>"` and
`provenance.node = <origin>`; answering is a `send_message` to that address. Delivery is at most once. `send_message`
returns once the broker accepted the message (limits and rate are checked there); if it does not arrive, the sender's inbox
receives a notice of kind `delivery_failed` whose body is the error code (`node.offline`, `node.unknown`, `node.timeout`,
`mesh.recipient_unknown`, `mesh.rate_limited`, ...) and whose `reply_to` is the failed message's id (the `message_id`
`send_message` answered with), so an agent with several messages in flight knows which one failed. The hub answers a post within its 30 s command deadline (`node.timeout`; `node.offline` as soon as it finds the target's link dead, which a silent link is after 25 s); a post the sending node has no answer for after 40 s fails there with `node.timeout`, so the sender is always told (#915), with one exception: while its own node has no reserved message ids ("Message ids" above, #965) no notice can be numbered, so a `delivery_failed` that comes back then is not made. `node.timeout`, from the hub or from the node, means the message may or may not have arrived: a target that answers late may already have delivered it. Nothing is resent. Plain messages cross nodes; `ask` stays node-local, and a `#channel` message on a shared project reaches every other running agent of the project on any node (below), and a node may have 64 posts waiting at the hub. Tasks, leases and
`handoff` cross nodes through the shared board below, for a project that is on two or more online nodes. The machine's owner can turn it off:
`<node data dir>/policy` with the line `mesh.remote = false` (default `true`; a value that is not exactly `true` or `false` counts as
`false`, so a typo never widens the node). Then messages to other nodes fail with a `delivery_failed` notice `node.policy_denied`
and messages from other nodes are refused with the same code; messages on this node are unaffected. The hub cannot change this
file. A second line, `mesh.fleet = false` (default `true`, read the same way), is about the hub-wide view (ADR-0112, "The hub-wide
view" below): this machine's agents neither ask for nor see it, and the machine and its agents are left out of it except in projects it
shares with the caller (ADR-0039, "Who is in it" below) (the node neither
offers it to the hub nor asks the hub for it, and `mesh.remote = false` does the same). `refs` travel with a cross-node message (ADR-0035, `MeshPost.refs`) and the receiver stores each as `<origin node>:<ref>` (`laptop:src/auth.rs`; one that is already qualified is qualified again), so an agent can tell a path on another machine from a local one; refs stay inert text, within the usual limits, and confer no lease or permission. The web window lists recent messages and marks the ones between machines. Both machines record a message between them (`MessagePosted` from the sender's node, `to` = `<node>/<agent>`, and from the receiver's node, `from_agent_id` = `<origin>/<agent>`, each under its own per-node id); the window shows the pair as one row, matched by sender, recipient and text (#905), and a member's scope admits the sender's copy only. A node may write any sender into its own copy, so a copy that names another machine's agent and that neither that machine nor the hub's board has recorded too is shown "reported by" the machine that recorded it, named by its id (which the hub checked against its certificate, never the host name it reports), and two such copies are never paired with each other. The match is by content, not identity: a direct message that failed (recorded only on the sender's machine) can be matched with a later channel copy with the same sender, recipient and text. ADR-0074's `MeshPost.message_id` (Accepted, not built) is the key that would make the match exact.

## Shared board (ADR-0039)

Project membership includes both startup checkouts and owner-selected workspaces (`WorkspaceRegistered`, ADR-0052), so
folders selected after a node starts participate without a daemon restart. Membership is deduplicated by enrolled node ID;
host names and directory names do not establish shared identity. Offline and unknown nodes do not count. A single-node
project with no unfinished shared work continues on its local broker.
An agent still running under an earlier project identity retains that membership until it exits. Re-associating its folder
does not move that session's tasks or leases to a local broker; the node's qualified agent key establishes its membership.

When a project has checkouts on two or more online nodes, `task_*`, `lease_*` and `handoff` are decided by the hub, which keeps one
board per project and is its only writer: a claim has exactly one winner and a lease is all-or-nothing across nodes. The agent's
tool calls do not change. The node sends each of these tools to the hub as `BoardCall{project_id, tool, arguments_json}` (the hub
writes `origin` from the certificate and names agents `<node>/<agent>`) and the hub answers `BoardResult{shared, result_json,
is_error}`. `shared = false` means the project is not on two online nodes and its board holds no task that is not done and no live lease (ADR-0039 amendment A: a board in use stays in use when a node drops): nothing happened and the node runs the tool on its own
broker, so a project that lives on one machine never needs the hub. The board's agents are the project's running agents in the
hub's view; one that ended gives its tasks and leases back, and a lease also ends at its TTL if its node vanishes.

**Lease renewal (ADR-0039).** On a shared project the holder's node renews each lease the board granted its agent while the agent
runs, so the lease lasts until the agent releases it or ends; its `ttl_s` bounds only how long it outlives a node that went away.
The node sends the renewal as the agent's own `lease_acquire` on the same paths with the same `ttl_s` (no new message), when half
the TTL has passed since the call that last granted it was sent (by the node's monotonic clock, which also counts time the machine slept, so never later than the hub's end).
Only a TTL of 30 s or more is renewed, so a lease is renewed at most once every 15 s; a shorter one ends as asked. An agent has at
most 256 paths and 64 grants renewed over all its leases; a lease beyond that is granted as usual and ends at its TTL. Each renewal
is one stored board change and one `LeaseGranted` per path. A renewal the hub cannot answer (the node is offline, the call timed
out, `board.unavailable`, `internal`) is tried again every second until the lease's end as the node reckons it; after that the
lease is gone, and a node that reconnects later does not take it back. A renewal the board refuses (`lease.held`, `agent.not_found`
when the hub no longer counts the agent as running and gave its leases back, or the project is no longer shared) ends it at once.
Renewal stops when the agent releases the lease, when the node takes the agent off the mesh (it finished, was stopped, or the node
gave up restarting it; an agent the supervisor is restarting keeps its endpoint and its renewals, and the hub's answer decides), and
when `mesh.remote` is switched off. Each renewal gives the paths a new board lease id; the agent keeps the id it was given, and the
node puts the current one into its `lease_release` (either id releases it). A release the hub did not decide (no answer,
`board.unavailable`) is not renewed again but keeps the id, so the agent's next try reaches the board's lease. The agent's own
lease calls and the node's renewals of its leases reach the board one at a time: while an agent's call waits (up to 10 s) for a
renewal on its way, no other renewal of that agent starts. A path the agent takes again under a newer grant leaves the older one, as
on the board. The table is kept beside the ledger (`<data>/sessions/lease-renewals.json`, owner-only, replaced whole by a rename,
written by its own thread whenever it changed): an agent the next run of the node takes up again from the holder gets its leases
back and they keep being renewed (the table is written as it stands when the node stops, before its agents leave the mesh; rows of
agents that were not taken up are dropped once the take-up is over, so an agent resumed later starts without them) (each row's grant time is stored on the wall clock and read back as an age; a row dated after the
current time, or whose end passed, is dropped; a file that is not a renewals file is set aside, renamed). A lease the node's own
broker granted is not renewed (ADR-0039 leaves local-only projects untouched), but when the project becomes shared and the node
hands it over in `BoardJoin`, the node renews it on the board at once (to learn the board's id) and from then on, and the agent's
local id still releases it.

A `handoff` that names a task goes only to an agent of the task's own project (an agent of another project could neither see nor finish the task): to any other it fails with `mesh.recipient_unknown` and nothing moves; a `handoff` without a task is a message and may cross projects. A `handoff` to `<node>/<agent>` moves the task and sends the receiver a message from the giver in one step; if the receiving node

Dispatch (ADR-0095 step 2): the broker can claim an open task of a project for a chosen agent of that project and build the one prompt that gives it the task (`Mesh::dispatch`, `agora_mesh::dispatch::build_prompt`). An agent of another project is `mesh.recipient_unknown`, a blocked or not open task is `task.blocked` / `task.not_open`, and nothing moves on a refusal. The prompt is the node's own template around the task's title and detail in a fenced block marked untrusted task data (a fence longer than any backtick run in the text, control, hidden and bidirectional characters removed, the title on one line, the whole prompt within the message size limit: the budget counts the fence twice, a run of more than eight backticks in the text loses its extra backticks so the fence stays at most eleven long, and a detail that had to be cut ends with a `[cut: N more bytes]` line inside the fence). Dispatch does not refuse an agent that already holds a claimed task: `task_claim` has no one-task limit, no existing error code means "busy", and the prompt queues behind the agent's running turn as any prompt does; a limit would need a new error code and is left to the maintainer. It is not an approval and changes no permission. Reached by `TaskDispatch` (capability `task.dispatch`; `docs/specs/protocol.md`, "Giving a task to an agent"): the node runs it on its broker, or for a shared project through the hub's board as the agent (`task_list` with `task_id`, then `task_claim`; the board has no release, so a shared task whose prompt was not sent stays claimed), and records `TaskDispatched`.
is offline it fails with `node.offline` and the task does not move. The node checks a shared project's `handoff` as it does a `#channel` message before it sends the call to the hub: wrong-type arguments are -32602, and a summary or refs over the limits are `mesh.body_too_large` / `mesh.too_many_refs`, so the task never moves on a hub that would not check them. When the hub cannot be reached, a project this node knows to
be shared fails with `board.unavailable` (the agent's other tools and its worktree keep working); a project it does not know to be
shared stays on the local broker. When a project becomes shared (gains its second online node with a checkout) the hub sends each such node `BoardJoin`: the node hands over its open and claimed tasks and its live leases for the project and stops keeping them; the hub puts them on the board (each task gets a board id and its owner a notice `{"board":"task_moved","from":...,"to":...}` from `hub/board`; a lease the board already holds for another agent is refused, and both holders get `lease_refused` / `lease_conflict`). The boards an older hub kept per spelling of a project id are merged once into the folded project's board (ADR-0115): a task whose id an earlier board already had gets a new one, and its owner gets the same `task_moved` notice unless the task is done. Where several boards' live leases hold one path for different agents, the first whose holder still runs in the project keeps it (a holder on a node that started again since the hub opened does not); every other lease on that path is released (`LeaseReleased`), and both holders get `lease_refused` / `lease_conflict`. An ended lease of any of the boards is released too, unless the agent that keeps the path holds it. Notices go only to agents running in the project; one whose machine is not connected is sent it when the machine connects (kept in the hub's memory, at most 1024 per machine). Until the nodes answered, or 5 s passed, `lease_acquire` and `task_claim` on that project answer `board.unavailable` (ADR-0039 amendment B). Done tasks stay in the node's local history. A node with `mesh.remote = false` answers `node.policy_denied` and keeps its work; a late answer is still applied (only the 5 s hold ends). A node hands over at most 500 tasks and 500 leases (leases by whole grant; what does not fit stays on its broker and keeps working there), the hub takes no more from one answer, cuts titles and details to the message size limit, and runs a handed-over lease at most one hour; a task whose creator or owner no longer runs lands open, made by `hub/board`. `BoardJoin` is answered where the hub's commands enter the node; the local control socket refuses it. Not done yet: boards
keep their tasks in the hub's SQLite file (settings key `board.<project>`, so they are in its backup); a change (create, claim, complete, lease, release, handoff) is answered as done only after the board and its audit events are written, together in one transaction (all or nothing), and only then published to the view, windows and nodes; when the hub cannot write them the call fails with `internal` (`incident_id: board.store`) and the board in memory is put back as it was, so an acknowledged claim or handoff survives a restart and a refused one never appears; leases are stored with them (ADR-0040) and a restart keeps those whose TTL has not passed (a restored lease gets a new id its holder does not know, so the holder cannot release it, but it ends at its TTL, when its holder ends or when its machine restarts, like a live one), a task claimed by an agent that is gone after a restart is given back, and when a machine restarted before its board was restored (boards are restored on first use) its earlier agents' tasks and leases are given back as the board is restored, so a new agent that reuses a local id (`agent-1`) inherits nothing; a message to a
`#channel` is sent as `BoardCall{tool: "send_message"}`; the hub makes one copy per other running agent of the project (any online node), each arriving as a message from the sender with the channel name in front of the text (`[#dev] ...`), best effort and at most once, within the usual body and ref limits, which the hub checks before any copy goes out on the text receivers get (`[#dev] ` plus the body; `mesh.body_too_large` / `mesh.too_many_refs` with nothing sent). Arguments of the wrong type (a ref or `reply_to` that is not a string) are -32602 from the node, as for a local message, and never reach the hub; before it sends the call the node also checks the limits that do not depend on the route (the refs' count and total size and the body as written), so an older hub cannot fan out copies that receivers refuse; the prefixed text is checked by the hub, which knows whether the project is still shared (the tool returns `{ delivered_to }` instead of `{ message_id }` in this case). Channel history is not kept for the agents: `inbox` on the receiver is how it is read (a window that opens later still shows the recent copies in Team chat, ADR-0103). The web console shows each project's board (tasks with their owner, held paths) in its right column, built from the hub's
`TaskChanged`/`LeaseGranted`/`LeaseReleased` events; a window that opens or reloads gets the boards in its `Snapshot` (`tasks`, `leases`, ADR-0040).

## Agents that start agents (ADR-0057)

A node offers `agent_start`, `agent_wait` and `agent_output` (capability `mesh.spawn`); they appear in `tools/list` only there, and
the node answers them itself (`agora-node::spawn`), not the broker. The caller is the connection's agent, as for every tool.

- **Every start needs the user's decision.** `agent_start` announces an `ApprovalRequested` with action `agent.start` whose detail
  shows the asking agent and machine, the kind, the folder and the whole prompt (see `protocol.md`), and answers only once a trusted
  device's signed decision has been verified on this node: `{ agent_id }`, `mesh.start_denied` (refused, or withdrawn) or
  `approval.expired`. The decision is signed over a hash of the request as announced, so a hub cannot change what starts. The call
  does not block the agent's other calls; if the connection ends first, or the client cancels the call, the request is withdrawn.
- **Settings (ADR-0066).** `settings` names launch settings the kind declares (`model`, `effort`, ...; values are strings). The node checks
  them against its manifest before anyone is asked (`agent.setting_unknown`, `agent.setting_invalid`; nothing is announced and nothing
  starts), the card lists every one next to the kind, folder and prompt, and the decision is signed over them (`agora-approval-start-v2`,
  `protocol.md`), so the helper starts with exactly what the user was shown. A request with settings offers no grant (`grant_allowed:
  false`) and a start with settings is always asked, even under a grant.
- **Launch options (ADR-0059 decision 5).** `options` names launch options the kind declares by id (`skip-permissions`: the helper acts
  without asking). A helper never inherits its parent's; it gets one only this way. The node checks the ids against its manifest and its
  owner's `skip_permissions` policy before anyone is asked (`agent.option_invalid`, `agent.option_forbidden`; nothing is announced and
  nothing starts), the card lists each with whether it skips permissions (the manifest's word) and asks for the same confirmation as a
  start from the window, and the decision is signed over `agora-approval-start-v3` (`protocol.md`). Such a request offers no grant and
  is always asked, even under one; the helper carries the "does not ask" badge.
- **A grant.** The decision may allow "up to N helpers of this kind in this folder" (N at most 10, this start included) and, separately,
  that the helpers it starts may ask in turn. Later starts that match the grant (same asking agent, kind, folder and `worktree`
  choice) start at once. A grant ends when the asking agent exits or is stopped, or when revoked (`GrantRevoke` with the approval id;
  `GrantList` shows it). No grant is given for a kind without an agent integration (a mesh section, an adapter or a structured mode), nor for
  kind `shell` or a kind whose program (`process.launch`, else `structured.acp.argv`, else `adapter.command`; its file name lower-cased,
  without `.exe`/`.cmd`/`.bat`/`.com` and without a version) is a shell, a wrapper (`env`, `nice`, `nohup`, `setsid`, `timeout`,
  `stdbuf`, `xargs`, `sudo`, `doas`, `su`, `ssh`, `wsl`, `docker`, `podman`) or an interpreter or runner (`python`, `pypy`, `perl`,
  `ruby`, `node`, `deno`, `bun`, `npx`, `uvx`, `bunx`, `php`, `lua`), whatever screen rules it has: there it would mean "run any N programs
  without asking". The
  request's `grant_allowed` says whether one may be given.
- **Never wider than the asker.** The folder defaults to the asker's own; one below it is allowed (a relative `folder` is taken below
  the asker's own); `worktree: true` makes the helper its own worktree of it. Another folder is asked about with `inside: false`
  shown to the user, never covered by a grant, and a helper started there cannot ask for helpers. A helper's helper must stay
  inside. Folder rules of ADR-0052 apply. On a node that confines agents (ADR-0047), a helper of another kind whose manifest the
  owner trusts to widen confinement is refused (`node.policy_denied`, rule `sandbox.trust_manifest`). Another machine is not
  asked through the hub (ADR-0119, `protocol.md` "Helpers on another machine"): `node` names it, `folder` must be a full path there,
  and that machine decides from its own policy, with its person's signed card or a standing grant given there. The call answers with
  the helper as `<machine>/<agent>` once the target's result comes back (`node.timeout` when it never does; a start given up on
  withdraws its card there). The asking node offers `mesh.spawn.remote` only while its policy allows (`mesh.remote_helpers` with
  `mesh.remote`). An agent's 3 helpers count its running helpers here and its starts on their way to other machines; the helpers
  running on another machine are counted there, which holds each asking run to 3 (6 per asking machine) unless a standing grant
  given there says otherwise. A caller whose process was replaced (a restart, a resume) asks as its new run: the machines its earlier
  run asked are told that run is gone. `agent_wait` and `agent_output` for a helper on another machine are not built yet.
- **Limits** (`mesh.helper_limit` with `rule`): a helper cannot start helpers unless the decision that started it allowed it, and a
  helper's helper never can (`depth`); 3 running helpers per agent (`helpers`); one start in progress or waiting for the user per
  agent (`pending`; the slot is taken before the limits are checked and held until the start ends, so concurrent calls cannot pass
  them together); a first prompt over 16 KiB (`prompt`). Six `agent_start` calls a minute per agent (`mesh.rate_limited`), and the
  node's own agent limit (`node.resource_limit`). A refusal before asking is an `AgentStartRefused` event (rate refusals at most one a
  minute per agent; agent-written fields cut to 256 bytes).
- **Prompt rules** (`protocol.invalid_message`, `field = prompt`): no control characters other than line breaks and tabs; no
  invisible characters (the list in `crates/agora-core/src/hidden-characters.txt`: Default_Ignorable_Code_Point and every format
  character, the line and paragraph separators and the private use areas; also refused in the requested `folder` and in the folder
  it resolves to); no leading `!`, `/`, `#`, `&`, `$` or `?` (a shell escape, a slash command, a memory edit, a background send, a
  skill form or the shortcuts key in common agents); and no text the kind's `[[typed]]` rules require on screen (its footer,
  compared case-insensitively and with every run of whitespace, line breaks included, as one space, as the screen rules read it), which could stand in for the real footer; and no last word starting with `@` (a file picker would take the
  Enter). Clients show any such character that reaches them as a visible `\u{…}` escape.
- **Following.** `agent_wait` and `agent_output` answer only for helpers the caller started (anything else is `agent.not_found`).
  `agent_wait` ends when the helper reaches the state or exits (`state: "exited"`); a connection may have 8 waits open
  (`mesh.too_many_asks`), all read from one view of the node per poll. `agent_output` is the bottom of the live screen (never
  scrollback) or, for a chat agent without a terminal, the end of its transcript; at most 16 KiB. It is untrusted text.
- **The first prompt** is typed into a terminal helper only after two fresh looks in a row at its live screen (taken then, not the
  paced detection's last result) show the kind's empty input box (`[[ready]]`) and an idle state; line breaks and tabs are typed as
  spaces. Enter follows only after two later fresh looks in a row show the input box (`input_box`) holding exactly the typed prompt
  and nothing else, with no dialog or menu on screen (`[[typed]]`), see `agent-manifest.md`. A blocked screen, keys someone else typed
  into the box, or text that does not show within a second leave it for the user. A kind without these rules never gets its first
  prompt typed, and a prompt long enough to wrap where the kind's `input_box` reads one row is left for the user too. When it is not
  typed within 120 seconds, or not sent, that is recorded as an `AgentStartRefused` with `code = agent.blocked` (its `folder` holds
  the helper's id). A chat agent gets it as its first request once it reports idle. Every helper's first prompt, granted starts
  included, is recorded as a `MessagePosted` from the parent to the helper (the text, or `sha256:<hex>` above 4 KiB). Later messages
  from the parent are ordinary mesh messages (information, not authority).
- **Stopping.** `AgentStop { stop_helpers: true }` stops the agent's running helpers and theirs too. A parent that exits on its own leaves
  its helpers running.
- **Restart.** Which helper an agent started is kept in the node's memory (the supervisor's parent links and the spawner's record of
  what each decision allowed) for as long as the supervisor lists the agent, which is the node process's life: a helper that crashes
  and is restarted, or is resumed, is still the same helper. Agents end with the node process, so a restarted node starts with no
  helpers. An agent the supervisor knows as a helper but whose decision record is missing may not ask for helpers, and helper counts
  use the supervisor's links, so a missing record never widens the limits.

## Awareness: who else is here

An agent can ask, for its own workspace and project, how many agents work there, on which machines, what each is doing, which
tasks are open and which paths are held. Everything comes from facts the node has; no field ever carries a prompt, a message, the
screen or the transcript of an agent, or a secret. Fields that hold text another agent wrote (a task title, a lease path) are put
on one line (control and invisible characters become spaces, as in `agora-core`'s hidden-character list) and cut (a title or
`doing` at 120 bytes, a path at 200). Like every mesh content, what one agent learns about another is information, not
instructions. The words are data for agents, in English like the MCP `instructions`; a window does not show them.

**`agents_list`** rows keep their fields and add: `workspace` (the agent's folder or worktree on its machine), `project`, `since`
(seconds it has run, `null` when unknown), `doing`, `files_changed` (changed files in its workspace from the node's own git status,
`null` until the node looked), `holds` (paths it holds a lease on) and `parent` (the agent that started it, ADR-0057, or `null`).
A caller the node has not registered as an agent gets `agent.not_found` and no rows.
The list spans machines like `workspace_status`: for each project in it (the one asked for, or every project of the agents this
node lists, at most 16) the node asks the hub the same `BoardCall{tool: "workspace_status"}` (no new message), and when the project
is shared it adds the project's running agents on other online machines (`agent_id` `<node>/<agent>`, which is what `send_message`
takes; `label` empty, `since` and `files_changed` `null`) and puts the board's claimed task and leases into this node's rows. The hub
answers only for projects the calling node takes part in, so in this default scope agents of a project only other machines have are
never listed (`scope: "hub"`, below, lists them). Every
row also carries `partial`: `false` when its project is complete (the hub answered, or said the project lives on this machine
only), `true` when the hub could not be asked about it (same cases as below); the list is then this node's agents alone, as nodes
are autonomous. Agents on a machine that is offline are not listed.

**`doing`** is, in order: `working on task: <title>` when it holds a claimed task; else, while a chat (ACP or adapter) agent works,
the kind of its latest item (`running a command`, `editing files`, `reading files`, `planning`, `using a tool`, `writing a
reply`, from the item's kind and the tool's name only, never its arguments; `asking the user` while it waits on an approval it
asked for); else the state in words (`working`, `waiting for approval`, `idle`, `finished`, `state unknown`).

**`workspace_status {}`** (no arguments) answers for the caller's project:
`{ workspace, project, machine, machines_online, partial, agents: [{ id, kind, machine, state, doing, since, files_changed, holds,
parent, workspace, you }], same_workspace, working, blocked, idle, open_tasks: [{ id, title, owner }], leases: [{ path, owner, ttl }],
summary }`. `same_workspace` counts the agents (the caller included) in the caller's folder on its machine; `working`, `blocked`
and `idle` (idle or finished) count the listed agents; `ttl` is seconds left; at most 100 agents, 20 tasks and 20 leases.
`machines_online` counts the online machines of the caller's project (those with a checkout, a selected folder or a running agent of
it), not the hub's machines: `machines_list` counts those (ADR-0112).
`summary` is one or two sentences built from those facts, for example `3 agents in this project on 2 machines, 2 in this workspace
(2 working, 1 waiting for approval); codex agent-2 is editing files; agent-1 holds src/auth.rs.` Agents on this machine are named
by their id and agents on other machines `<node>/<agent>`, which is how `send_message` addresses them.

The node's broker answers for its own agents. The node then asks the hub with `BoardCall{tool: "workspace_status",
arguments_json: "{}"}` (the read-only counterpart of the board tools, ADR-0039; no new message): when the project is shared the
hub answers with the running agents of the project on online nodes (kind, machine, state, the board task each claimed, the paths
each holds, its parent, its folder: its worktree, or the folder it was started in), the board's open tasks and live leases and how many machines are online; the node keeps
its own rows (they know more), puts the board's task and leases into them and adds the others (`since` and `files_changed` are
`null` for those). When the hub says the project is not shared the node's answer is complete. When the hub cannot be asked (no hub,
`mesh.remote = false`, an older hub that refuses the tool, or 3 s without an answer) the answer is the node's alone with
`partial: true` and the summary says so.

### The hub-wide view (ADR-0112)

Agents can see every machine of their hub and every agent running on it, in any project, to answer "which machine runs what" and to
pick a machine for a helper. The node asks the hub once with `BoardCall{tool: "fleet_status", project_id: "", arguments_json: "{}"}`
(no new message) and waits at most 3 s. The hub answers with only facts it projects from node events:
`{ machines: [{ id, online, os, build, agents_running, last_seen_ms }], agents: [{ agent_id, kind, machine, project, state, workspace,
parent }], partial }`: every machine it knows whose latest link agreed on `mesh.fleet_status.v1` (offline ones too, by their last link;
a machine the owner forgot is left out), every running agent on such a machine while it is online (`agent_id` is `<node>/<agent>`, what `send_message` takes; `state` is the window's word: `working`, `blocked`, `idle`,
`done`, `unknown`; `workspace` is the folder on its machine, its worktree or the folder it was started in), at most 64 machines and 200 agents, `partial: true` when either cap cut
the list. Never `doing`, a label, a task, a message or a file name.

- **`machines_list {}`** answers `{ machines, partial }` with the hub's machines (text cleaned and cut like the rest of this section).
- **`agents_list {scope: "hub"}`** lists what the default scope lists (this node's agents of all its projects, or of the one named with
  `project`, and the other machines' agents of its shared projects, with the board's tasks and leases), then adds the hub's other
  agents on other machines (only the named project's when `project` is given; an agent already listed is not added again): `node` is their machine,
  `project` their own, `doing` their state in words, `label` empty, and `since`, `files_changed` and `holds` `null` (the hub does not
  say). Every row's `partial` is then the hub's. `scope: "project"`, or no `scope`, is the answer above, unchanged; any other value is
  `-32602`.
- **Capability.** A hub that answers `fleet_status` offers `mesh.fleet_status.v1`, and a node offers it when its policy allows; the node
  lists `machines_list` and `scope` in `tools/list` only while its hub's `Welcome` agreed on it (an older hub withdraws it at the next
  connection) and messages may cross nodes (`mesh.remote`).
- **Without the hub's answer** (the node does not offer the view, no hub, an older hub, a refusal, an error, or 3 s without an answer)
  both tools answer from this node alone: `machines_list` lists this machine
  (`online: true`, its OS, its version, its running agents, `last_seen_ms` now), `agents_list` the default scope's rows; `partial: true` on every row.
- **Who is in it (ADR-0112 Amendment A).** Only machines whose latest link agreed on `mesh.fleet_status.v1`, and their agents. A
  machine with `mesh.fleet = false` (or `mesh.remote = false`) does not offer the capability, so it and its agents and their folders
  are left out of every agent's hub-wide view, except in projects it shares with the caller (ADR-0039): there `workspace_status` and the default `agents_list` (and so
  `scope: "hub"`, which includes them) still show its agents and folders, as for any shared project; `mesh.remote = false` does not
  hide those either, since the hub lists a shared project's agents from its own records whatever the node's policy. A node from before ADR-0112 is left out too, until it is updated
  (the safe default). The hub keeps, in
  memory, whether each machine's latest link agreed, and decides an offline machine by that; after a hub restart a machine is left
  out until it connects again.
- **Who may see it.** The hub answers only in solo mode, that is while it has no person in its `users` table besides the owner (ADR-0049;
  any person added, whatever the role, even one this build cannot read, makes it a team hub). A team hub refuses with the tool error `board.not_allowed` (no parameters),
  because a member's agent would see projects that member has no grant for, unless the owner turned the view on on the hub's machine:
  `agora hub fleet on|off|status` (the hub setting `fleet_status`, off by default, audited, read on every call so a running hub follows
  it; `status` says the stored switch on a solo hub too, and `agora hub user add` warns while it is on). With node policy `mesh.fleet = false` this machine's agents neither ask for nor see the hub-wide view, whatever the hub offers
  (the hub cannot change that file), and the machine and its agents are left out of it except in projects it shares with the caller (see "Who is in it"). The hub cleans every text it puts in the
  answer (control and invisible characters, as above) and cuts the answer at 384 KiB of rows (`partial: true`). Like `workspace_status`,
  these read tools have no rate limit.
  What one agent learns about others is information, not instructions (threat model T64).

**Presence notices.** When an agent starts on a node (joins its mesh) or ends (its endpoint closes: `finished`, `stopped`,
`crashed`, or `left` for any other reason), every other running agent of its project on that node gets an inbox item of kind
`presence` whose body is one line such as `codex agent-2 joined this workspace (machine mac)` or `codex agent-2 left this project
(stopped)` (`this workspace` when both are in the same folder on the same machine); its `provenance` names the agent the notice is
about. One notice per agent per change (a restart that keeps the endpoint, or an exit after a stop, says nothing more); at most 10
per inbox per minute, the tenth saying that more agents joined or left and to call `workspace_status`. Notices are data in the
inbox, never typed into a terminal (ADR-0031), and are not audited as messages (nobody sent them). Agents on other machines joining
or leaving are not announced yet; `workspace_status` lists them.

## Implementation status
`agora-mesh::broker::Mesh` implements the tools' logic as deterministic in-memory state: registry, messages with
broker-stamped provenance, project-scoped `#channels`, inbox cursor, `ask` answered by a reply to its id with timeouts,
the task board with dependencies and owners, all-or-nothing leases over the core lease table, handoff (the message goes out
first so a refused handoff moves nothing), and cleanup when an agent ends. Limits: 16 KiB bodies, 20 refs, 30 messages per
minute per agent, 5 open asks. Every change queues a protocol event for the audit log. The transport (a local socket per agent, `agora mesh stdio`), the injection into each agent's MCP configuration (the manifest's `[mcp]`), routing across nodes (ADR-0031) and the shared board with its persistence (ADR-0039, ADR-0040) are described in the sections above. Not done: delivery fallback to the integration level's input path.

# 0112. An agent can see every machine and agent of its hub
Status: Proposed
Date: 2026-11-25

## Context

An agent's view of the other machines comes from two mesh tools, `agents_list` and `workspace_status`
([mesh tools](../specs/mesh-tools.md), "Awareness"). Both are scoped to projects the asking node already takes part in:

- the node asks the hub (`BoardCall{tool: "workspace_status"}`, ADR-0039) only about the projects of its own agents, at most 16
  (`crates/agora-node/src/mesh_server.rs`, `list_reply`);
- the hub answers only when the project is shared, that is on two or more online nodes, and only when the calling node is one of
  them; otherwise it says `NotShared` and the node answers from its own broker (`crates/agora-hub/src/board.rs`, `call`);
- `machines_online` counts the online machines *of that project*, not the hub's machines.

So an agent on a machine whose projects live nowhere else sees only itself, even when every machine of the hub is online and
running agents in other projects. The spec says so ("agents of a project only other machines have are never listed"). No tool
lists the hub's machines at all. The window shows everything (client snapshot), so a person sees what an agent cannot.

The owner uses agents to coordinate work across machines (ADR-0057 helpers, ADR-0095 dispatch, the multi-machine development of
this repository). An agent asked "which machine runs what" cannot answer, and an agent choosing where to start a helper does not
know which machines exist. Field report: 2026-11-25, on the `mac` node with all three machines on 0.3.38, `agents_list` returned one
row and `machines_online: 1`.

The project scoping is a privacy choice, not an accident: in team mode (ADR-0049) a member may be granted some projects only, and an
agent's tool output can reach a person who has no grant. Widening it needs a decision.

## Decision

1. **A new hub view for agents, read only.** The hub answers a `BoardCall` with `tool = "fleet_status"` and `project_id` empty. No new
   message and no new field: `BoardCall.tool` is a string and this adds one value (the comment on `BoardCall` lists it). The answer
   (`BoardResult.result_json`, `shared = true`) is:
   - `machines`: every machine the hub knows: `id`, `online`, `os`, `build` (version), `agents_running`, `last_seen_ms`;
   - `agents`: every running agent on an online machine: `agent_id` (`<node>/<agent>`), `kind`, `machine`, `project`, `state`
     (the UI word), `workspace` (path on its machine), `parent`;
   - `partial: true` when a cap cut the list. Caps: 64 machines, 200 agents.
   No `doing`, `label`, task text, message bodies or file names: only facts the hub already projects from node events.
2. **Two mesh tool changes, additive:**
   - a new tool **`machines_list`** `{}` → the `machines` above;
   - **`agents_list`** takes an optional `scope`: `"project"` (default, today's behaviour, unchanged rows) or `"hub"` (the
     `fleet_status` agents, merged with this node's own rows as today; rows of other projects carry their `project`).
   Without the hub's answer (hub unreachable, older hub, refused) both answer from this node alone with `partial: true`, so nodes
   stay autonomous.
3. **A capability.** The hub advertises `mesh.fleet_status.v1` in `Welcome`. A node offers `machines_list` and `scope: "hub"` only when
   the hub has it; otherwise the tool is not listed and `scope: "hub"` returns the local rows with `partial: true`.
4. **Who may see it.**
   - **Hub:** answers `fleet_status` only while the hub is in solo mode (no user besides the owner, ADR-0049). On a hub with members
     it refuses with a new error code `board.not_allowed` (no parameters; English and Turkish Fluent text) unless the owner turned on `fleet_status` in hub settings (off by default there), because
     a member's agent would otherwise see projects that member has no grant for.
   - **Node policy:** `mesh.fleet = true|false` (default `true`, like `mesh.remote`, ADR-0031). With `false` the node neither asks
     the hub nor lists the tool. The hub can never widen this.
5. **Docs:** `mesh-tools.md` (the tool, the scope, the caps, the capability), `error-codes.md` (`board.not_allowed`), the `BoardCall` comment in `commands.proto`, the threat
   model (new entry: an agent learns the names, kinds, states and folders of other projects' agents; bounded by solo mode or the
   owner's switch and node policy), and the glossary is unchanged.

## Consequences

- Agents can answer "which machines are on the hub and what runs where" and pick a machine for a helper without asking the person.
- One hub handler and one node handler; no schema message changes; one capability, one node policy key, one hub setting.
- `machines_online` in `workspace_status` keeps its meaning (machines of the project). Its name is misleading; the spec text says
  what it counts, and `machines_list` is the way to count the hub's machines.
- Folder paths of other projects reach every agent of a solo hub. Paths can name customers or people; this is the owner's own data
  on the owner's own machines, and node policy can turn it off per machine.

## Alternatives considered

- **Let `agents_list` without a project always mean the whole hub:** changes the meaning of a published tool's answer, which the
  append-only rule forbids, and it would widen the view on team hubs silently.
- **A new protocol message instead of a `BoardCall` tool value:** cleaner typing, but a second request path for the same
  read-only answer; `BoardCall` already carries tool names and JSON results, and the hub already vouches for the caller.
- **Give agents a client session (snapshot):** the full client view includes terminal and chat streams and approval traffic; far
  more than an agent needs, and against "mesh messages are untrusted data" on the way back.
- **Keep it project-scoped and document it:** leaves the field report's question unanswerable for agents.

## Open questions for the maintainer

- Default of `mesh.fleet` on a node: `true` (proposed, matches `mesh.remote`) or `false`.
- Whether `workspace` paths of other projects should be shown, or only the project id.

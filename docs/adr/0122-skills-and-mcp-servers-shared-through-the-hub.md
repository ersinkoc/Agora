# 0122. Skills and MCP servers shared through the hub
Status: Accepted
Date: 2026-11-27

Accepted: 2026-11-27 (the maintainer's answer to the question listing the written decisions). The open questions' proposed answers stand.

Origin: the maintainer's orchestration request of 2026-11-27 ("skills and MCP servers shared through the hub"), roadmap M8.4.

## Context

- **What exists.** Each node gives each agent it starts one MCP server, the mesh (`agora-mesh`), through the method its manifest
  declares (`[mcp]`: a JSON file passed with a flag for Claude Code and WrongStack, `-c mcp_servers.…` overrides for Codex;
  `crates/agora-adapters/src/inject.rs`). Any other MCP server, and any skill, is set up by hand in each tool's own configuration
  on each machine. With many machines and kinds, they drift apart: one machine's agents have the database server, another's not.
- **Invariants that bind this.** The hub cannot widen node policy; a sensitive action needs a person's signature verified on the node
  (ADR-0020). Secrets never leave the node (ADR-0084). Agents are data: a new kind's support is a manifest, not core code. An MCP
  server with a command runs a program on the node, so delivering one is a sensitive action.
- **What a skill is here.** A folder of instructions (a `SKILL.md` and a few text files) an agent loads when it judges it useful. It
  carries no program of its own, but it steers the agent, so it is the owner's instruction, never another agent's.

## Decision

1. **Two kinds of shared item, both plain data.**
   - **An MCP server definition**: a name; either a program (`command`, `args`, `env`) or a remote server (`url`, `headers`). Values
     that are secrets are written as references, `{secret:NAME}`, never as values.
   - **A skill**: a name, a short description, and its files (UTF-8 text only, at most 64 files and 1 MiB in all, paths inside the
     skill's folder, no links).

   Each item has a content hash; a change is a new version of the same name.
2. **The owner publishes, and signs.** In a team (ADR-0049) only the owner publishes, changes or removes an item; a solo hub's
   person is the owner. Publishing from the window signs `(item hash, scope)` with the publishing device's key, over a new signing
   domain `agora-share-item-v1` with shared vectors, as approvals are signed. The hub stores the item, its signature and its scope,
   and serves them; it cannot make one up or widen one, because a node checks the signature against its own trusted devices.
3. **Scope.** All projects, or one project; optionally only some kinds. An agent gets the items whose scope covers its project and
   kind, at its start.
4. **The node decides.** Node policy `share.items` (`on` by default, since every item carries the owner's signature) or `off`;
   `share.mcp_commands`, when set, is the list of programs an MCP server item may run on this node (anything else is refused on
   that node and shown so). A node that refuses an item says why in its answer to the hub, and the window shows it on the item.
5. **Installed per agent, at its launch, never into the person's own configuration.** The node adds the items to what it already
   writes for the mesh server, through the manifest:
   - `[mcp]` gains `server_flag` (the flags for one more server, repeated per server, for kinds like Codex that take flags) and the
     JSON file simply gets more entries;
   - a new `[skills]` section declares how a kind loads a folder of skills (a flag naming a folder the node writes under its own
     data folder, outside the worktree). A kind is given skills only once that way was checked on a real version of it (ADR-0059's
     rule); until then the window says "this kind does not take shared skills".
   An item never lands in `~/.claude`, `~/.codex` or a repository, so removing it is removing it from the next launch.
6. **Secrets stay on the node.** `{secret:NAME}` is filled from the node's secret store (ADR-0084) when the agent starts. A server
   whose secret is missing on a node is not given to that node's agents, and the window says which secret, on which machine.
7. **When it takes effect.** At the next start or resume of each agent; a running agent keeps what it started with. The agent's
   details list the items and versions it was started with, so "which agents still run the old one" has an answer.
8. **Schema (needs the maintainer's OK).** Hub commands `ShareItemPublish`, `ShareItemList` and `ShareItemRemove` (owner only);
   an event `ShareItemChanged`; the node fetches an item's content by hash over its link (`ShareItemGet`) and reports what it
   refused; capability `share.items`; the agent snapshot gains the items it was started with. Append-only, as always.

## Consequences

- One place to add the database MCP server or a team's review skill, and every agent on every machine has it at its next start.
- Nothing is written into the tools' own configuration, so a machine's hand-made setup is never touched or overwritten.
- A node owner keeps the last word (`share.items`, `share.mcp_commands`), and the hub alone can do nothing: a stolen hub can serve
  only items the owner signed.
- Kinds without a checked way to load skills get MCP servers only, until their manifest says how.
- Cost: a new signing domain, a manifest extension, five schema additions, and a section in the window (Settings → Shared).

## Alternatives considered

- **Write into each tool's own configuration files** (`~/.claude/settings.json`, `~/.codex/config.toml`): reaches agents started
  outside Agora too, but edits files the person owns, differs per tool version, and removal would need undoing edits. Rejected.
- **A node-side card for every item on every machine**: safest, but with ten machines one change means ten answers; the owner's
  signature at publishing gives the same guarantee once (decision 2), and `share.items = off` keeps a per-machine veto.
- **Members publish too**: more convenient in a team, but a skill steers every agent and an MCP server runs a program; kept to the
  owner (open question 1).

## Open questions (with proposed answers)

1. Should members publish? Proposed: no, owner only, as for fleet-wide settings (ADR-0112).
2. Default of `share.items`? Proposed: `on`, because every item is owner-signed; the person can turn it off per machine.
3. Remote (URL) MCP servers in the first step? Proposed: yes; they run nothing on the node, only their headers need secrets.
4. Limits? Proposed: 64 items per hub, 64 files and 1 MiB per skill, 32 servers given to one agent.

## Build order (once accepted)

1. Schema, signing domain and vectors (node, web, terminal client), golden fixtures.
2. The hub: store, list, remove, serve by hash; owner-only; audit rows.
3. The node: fetch, verify, policy, secret filling, per-launch injection for MCP servers (Claude Code, Codex, WrongStack manifests).
4. The window: Settings → Shared, publish and sign, per-machine refusals, the agent's started-with list.
5. Skills: the `[skills]` manifest section per kind once checked on a real version.

## Status of the build

1. Schema: done (2026-11-27). `schema/agora/v1/share.proto` (commands 127..130, results 43..44, events 92..93,
   `AgentStarted.shared_items` 36, `AgentSnapshot.shared_items` 53), capability `share.items` (offered by no one; node and hub refuse
   the commands), error codes `share.refused`, `share.secret_missing`, `share.unknown`, the `agora-share-item-v1` vectors with their
   test, golden fixtures, `protocol.md` "Shared skills and MCP servers" and threat-model row T75. The signed word carries
   `issued_ms`, and a removal is signed too, so an old signature cannot bring back what a newer word replaced or removed.
2. The hub: done (2026-11-27). `crates/agora-hub/src/shares.rs` (store migration 20: `shared_items`, the newest signed word per
   name with removals kept, and `shared_item_refusals`); the byte layouts and the item rules are one module for hub and node
   (`agora_protocol::share`, checked against the vectors). `ShareItemPublish`/`ShareItemRemove`/`ShareItemList` are the owner's alone
   (a member's or viewer's is refused before anything is kept, audited as `share_item.publish`/`share_item.remove`); the hub checks the
   word as a node will (one of the owner's unrevoked devices signed it, the content's hash, the item's and the scope's rules, `issued_ms`
   newer than the word it replaces and at most 5 minutes ahead, at most 64 live items) and writes `ShareItemChanged`. A node asks
   `ShareItemGet` on its own connection once both agreed on `share.items` (the hub offers it now; no node yet); a client never can.
   A node's `ShareItemRefused` about itself is kept against the item's current version. `ShareItemSigned.name` (7) was added for a
   removal, which carries no item. A removal is kept even for a name never published (the owner can retire a name ahead, and a
   captured older publish of it never comes back), the newest 256 removals at most. Tests: `shares.rs` units (the solo owner `owner`
   and a team owner's own id each publish only with their own devices), `dispatch.rs`, store migration, `tests/devices.rs` over a
   real socket.
3. The node: not started. **The node is the authority, the hub only the first gate**: before it uses any item (a shared MCP server
   is a program it would start in agents), the node verifies the signed word itself against its own trusted devices (ADR-0061, not
   the hub's list), the content hash and the rules in `agora_protocol::share`, keeps the newest `issued_ms` per name itself, and
   applies `share.items` and `share.mcp_commands` (the hub cannot widen node policy).
4. The window: not started.
5. Skills per kind: not started.

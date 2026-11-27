# 0122. Skills and MCP servers shared through the hub
Status: Accepted
Date: 2026-11-27
Amended: Amendment A (Accepted 2026-11-27, the maintainer's answer "Accept"): the node's launcher gives a shared MCP server its secrets.

Accepted: 2026-11-27 (the maintainer's answer to the question listing the written decisions). The open questions' proposed answers stand.
Decided later the same day (the maintainer): the additive `ShareItemSigned.name` (7), for a removal that carries no item, is covered
by this ADR's schema approval; a node's `share.mcp_urls` defaults to `allow` (a URL server is one the owner signed and published,
and a machine that must not send its code out says `forbid`), and the window warns when the owner publishes one that agents may send
repository content to that address.

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

## Amendment A (Accepted, 2026-11-27): the node's launcher gives a shared server its secrets

Decision 6 says a `{secret:NAME}` is filled on the node "when the agent starts", but the agent's tool reads its MCP servers from a file
the agent can read (`<data>/shared-items/agents/<agent>.json`), so a filled value would sit in that file. Step 3b therefore gives no
server that names a secret (`share.refused`, `secret_needs_launcher`). This amendment says how such a server is given.

**What can and cannot be kept from an agent.** A server started for an agent runs as the agent's user; that user can read the
server's environment (`/proc/<pid>/environ` on Linux) and its memory. So no design keeps a secret from an agent that is set on finding
it. What a launcher can promise is narrower and still worth it: the secret is never written to a file (the agent's config, the
worktree, a log, the hub), never on a command line, and only reaches a process the owner's signed item names, on a machine whose own
policy lets that secret go to shared servers.

1. **The file names the launcher, not the secret.** A program server that names a secret is written to the agent's MCP file as
   `command = <the node's own binary>`, `args = ["mcp-launch", <item name>, <launch id>]`, and no environment. The launch id is 32
   random bytes, made for this agent at its start, for this item only.
2. **The launcher asks the node over the agent's own mesh endpoint** (the socket the node already serves per agent, ADR-0031), with
   a new request `shared.launch {item, launch_id}`. The node answers only for a launch id it made for this agent and item, once
   (single use), while the agent runs, and fills the item's environment from its secret store (ADR-0084). The launcher then replaces
   itself with the item's program (`exec`), so the secret lives only in that process's environment.
3. **The machine decides which secrets may go.** A new node policy key `share.secrets`: the secret names a shared server may receive
   on this machine (default: none). A name not listed keeps the item refused (`share.refused`, `secret_not_allowed`), whatever the
   owner signed: the hub cannot widen it.
4. **Remote (URL) servers with secret headers stay refused** in this step: the agent's tool connects to them itself and reads the
   headers from the file. A later step can make the launcher a local stdio-to-HTTP bridge that adds the headers.
5. **Single use and what it costs.** The agent's tool starts its servers when it starts, so the launcher spends the id first. An agent
   that reads the file and spends the id before its tool does gets the secret (as it could from the server's environment) and the
   server then fails to start, which shows. A restart of the agent makes new ids.
6. **Audit and words.** The node records `share.launch` (the item, the agent, the outcome; never the value). The window's Shared
   section says "needs the secret NAME on MACHINE" and "MACHINE does not let NAME go to shared servers" instead of "not given yet".

Built (2026-11-27): `share.secrets` in node policy; `SharedItems::servers_for` gives such a program as `agora mcp-launch --socket <agent's
mesh socket> <item> <launch id>` (32 random bytes, single use, spent when the agent ends) or refuses it (`secret_not_allowed`,
`share.secret_missing`); the node answers `agora/shared_launch` on the agent's own socket before the broker (`mesh_server.rs`
`launch_reply`, never a tool), filling the environment from its store at that moment; `crates/agora/src/mcplaunch.rs` runs the program
with the agent tool's standard input and output. The window's Shared section says each refusal in words. Review of b9dfb5fe: a launch id also names the item's version, project and kind, so a version the owner changed or a scope narrowed
since the agent's launch gets nothing (`unknown_item`); only the id in the agent's current file is live; an item whose environment
sets a loader's or an interpreter's variable (`LD_PRELOAD`, `DYLD_*`, `NODE_OPTIONS`, `PYTHONPATH`, `PATH`, ...) is refused on every
machine (`env_not_allowed`: it would make an allowed program run anything). **Single use has a cost:** when the agent's tool starts
the server again (a crash, a reconnect) or a held agent outlives a node restart, the launcher is refused until the agent restarts,
and says so in the tool's MCP log. Not built: the URL bridge, and a `share.launch` audit row (the node logs nothing of the value).

Accepted with the proposed answers. Open questions (with proposed answers): (1) `share.secrets` empty by default, so every machine opts in by name: yes. (2) URL servers
with secret headers in this step: no, a later step. (3) A new mesh request needs the schema's approval (`shared.launch` is a node-local
mesh tool, not a hub message): proposed as a mesh tool name only, no schema change; confirm.

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
3. The node: in part (2026-11-27), step 3a done: `crates/agora-node/src/shared_items.rs`. The node asks its hub for the list (names,
   versions and scopes, no content; `ShareItemList` on its own connection) when the link is set, every 60 s, and 5 s after an unanswered
   ask; fetches a changed version with `ShareItemGet`; judges each word with its own trust (`NodeApprovals::remote_grant_term`: a
   device this node trusts, in its term, not limited to projects), the content hash, the rules, the newest `issued_ms` it ever saw for
   the name (kept in `<data>/shared-items/newest`, so a replay is refused after a restart too) and its policy (`share.items = on|off`,
   default on, anything else off and a line not understood; `share.mcp_commands = a, b`, the only programs a shared MCP server may run
   here; `share.mcp_urls = allow|forbid`, default allow, anything but `allow` forbids: a URL server runs nothing here but is a
   channel for the repository's content and for untrusted text into the agent). The items in use are judged again at every poll, so
   a signer untrusted or limited here stops counting within 60 s (`signer_untrusted`). A word it cannot trust changes nothing it uses; only the owner's newer, trusted word its policy will not run ends the version
   in use. A name the hub stops listing is dropped (the hub can only narrow). Each refusal is said once per version
   (`ShareItemRefused`), and the node offers `share.items` while its policy gives items. End to end: `crates/agora/tests/hub_dial.rs`
   `a_machine_uses_only_the_shared_items_a_device_it_trusts_signed`. Step 3b done too: at each launch the supervisor asks for the
   servers in the agent's scope (`SharedItems::servers_for`, judged again first), writes them to
   `<data>/shared-items/agents/<agent>.json` (outside the worktree, readable by this user only; a confined agent may read it) and adds
   the manifest's `[mcp] shared_flag` (engine 7; Claude Code: a second `--mcp-config`, checked with `claude --help` on 2.1.332).
   `AgentStarted.shared_items` names what it got; the file goes when the agent ends, gives up or is stopped. A kind without
   `shared_flag` gets no shared server: Codex takes MCP servers only as `-c` overrides, which would put a secret on its command line
   (visible to other processes), so it waits (follow-ups). The same test then starts an agent and checks its arguments, the file,
   its mode and its removal. **Secrets are not given yet** (agora-ac's review of afafe8f3): a secret must never sit in a file the
   agent can read, because whatever an agent reads may go to its model's provider. A server that names `{secret:NAME}` (in its
   environment or headers) is left out and said once (`share.refused`, `secret_needs_launcher`) until a node launcher puts the secret
   only into the server's own environment (and, for a URL server, a small bridge adds the headers), so the file holds names only;
   `{secret:}` in a server's arguments is refused by the rules (a command line is readable by every user of the machine). Even then
   a same-user agent could read its child's environment, the residual ADR-0084 already accepts. **The node is the authority, the hub only the first gate**: before it uses any item (a shared MCP server
   is a program it would start in agents), the node verifies the signed word itself against its own trusted devices (ADR-0061, not
   the hub's list), the content hash and the rules in `agora_protocol::share`, keeps the newest `issued_ms` per name itself, and
   applies `share.items` and `share.mcp_commands` (the hub cannot widen node policy).
4. The window: in part (2026-11-27), MCP servers. Settings → **Shared with agents** (`apps/web/src/components/SharedSettings.tsx`,
   `#/device/shared`, owner only) lists what the hub keeps, with each machine's refusal in words (`secret_needs_launcher` as "needs a
   secret: not given to agents yet"), and publishes, changes and removes a server, each word signed by this device
   (`apps/web/src/net/share.ts`, the shared vectors; a change is signed after the word it replaces). The form keeps secrets off the
   command line, warns that an agent may send what it reads to an address, and says that a server naming a secret is not given to
   agents yet. Not built: skills in the form (step 5 first), a scope of some kinds only, and the agent's started-with list in its details.
5. Skills per kind: not started.

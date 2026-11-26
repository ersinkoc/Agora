# 0119. A helper started on another machine
Status: Proposed
Date: 2026-11-26

Not accepted. Nothing here may be built until the maintainer accepts it.

Settles the route that ADR-0057 left open ("another machine only if the approval names it"). Touches ADR-0020 (the hub cannot
widen node policy), ADR-0029 (the hub forwards commands), ADR-0031 (mesh content is untrusted; the hub writes `origin`), ADR-0060
(agent ids are never given twice), ADR-0109 (run identity), ADR-0112 (the fleet view and solo mode) and the threat model.

## Context

ADR-0057 lets an agent start a helper with `agent_start`, after the person's signed approval. The local half is built. A start on
another machine is refused today: `Spawner::start` (`crates/agora-node/src/spawn.rs`) answers `protocol.capability_missing` with
`capability = mesh.spawn.remote`, and the mesh tools spec says so. ADR-0057 cannot be built further as written, because no path
carries the request:

- **The hub takes two requests from a node.** `post_for_node` (`crates/agora-hub/src/server.rs`) accepts `MeshPost` and
  `BoardCall` and refuses every other command (`protocol.invalid_message`, `field = command`).
- **Neither fits.** `MeshPost` is untrusted agent content: it can inform an agent, never start one. `BoardCall` has a closed list
  of tools, and the hub answers them itself; the target machine never sees the call.
- **The window's starts do not fit either.** `AgentStart` (11) and `AgentLaunch` (82) are a person's starts. They have no asking
  agent and no origin, and they are signed over the window's own messages, so the target would never raise its `agent.start`
  card. An agent's request would arrive looking like the person's.
- **The wait is too long for a forward.** A forwarded command ends after `COMMAND_DEADLINE` (30 s) with `node.timeout`. An
  `agent.start` approval waits up to 10 minutes (`DEFAULT_TTL_MS`, `crates/agora-node/src/approvals.rs`).
- **The tree is same-node only.** `AgentStarted.parent_agent_id` (7), `AgentInterrupted.parent_agent_id` (5) and the client's
  `AgentSnapshot.parent_agent_id` (13) name an agent *on the same node*. The hub and the window qualify it with the helper's own
  node (`q(...)` in `crates/agora-hub/src/hub.rs` and `apps/web/src/net/view.ts`). Agent ids are `agent-N` per node, so a remote
  parent written there would hang the helper under an unrelated local agent.
- **Following and stopping are same-node only.** `agent_wait`, `agent_output` and `AgentStop.stop_helpers` read the node's own
  supervisor.

The owner coordinates agents across machines (ADR-0112 gave agents the list of machines). "Start a Codex on blackview and give
it this task" is the next request agents make, and today it needs the person.

## Decision

1. **The asking machine checks first, then asks the hub.** `agent_start` with `node` set to another machine:
   - is offered only when this node, the hub and the target all have the capability `mesh.spawn.remote` (the name the refusal
     already uses), and this node's `mesh.remote` is `true`;
   - needs `folder`, a full path on the target. The asker's folder means nothing on another machine, so there is no default;
   - is allowed only to an agent the person started (depth 0). A helper never asks for a helper on another machine;
   - counts against the asker's existing limits on its own node: one start in progress, 3 running helpers (local and remote
     together), six starts a minute. A refusal here is the node's usual `AgentStartRefused` for its own agent;
   - is checked against the prompt rules that need no manifest (control and invisible characters, leading `!`, `/`, ...). The
     target checks the rest against its own manifest.

   The node then sends a new node → hub request **`HelperStart`** (a new `CommandRequest` arm):
   `{ start_id, target_node, kind, folder, prompt, worktree, settings[], launch_options[], from_agent, from_run_id, origin }`.
   `start_id` is 16 random bytes the asking node chooses. `from_run_id` is the asking agent's run (ADR-0109). `origin` is empty;
   the hub writes it.

2. **The hub carries it and widens nothing.**
   - It accepts `HelperStart` only on a node's connection. From a window it is `protocol.invalid_message`.
   - It writes `origin` from the connection's certificate, whatever the node wrote (as for `MeshPost`). `from_agent` and
     `from_run_id` stay the asking node's word: a node can vouch only for its own agents.
   - It refuses `node.unknown`, `node.offline`, a target that is the asker itself (`protocol.invalid_message`, `field =
     target_node`) and a target without the capability (`protocol.capability_missing`, `mesh.spawn.remote`).
   - It rate-limits per asking machine, across all targets: 12 starts a minute and 8 open at once (`mesh.rate_limited`).
   - As for the fleet view (ADR-0112), it carries starts only on a solo hub, or when the owner turned the switch on
     (`board.not_allowed` otherwise).
   - It writes an audit row (`agent.start.remote.asked`) and forwards the request to the target. It never approves anything.

3. **The target decides from its own policy.** ADR-0020: the hub cannot widen what a node allows.
   - Node policy key `mesh.remote_helpers = true|false` (default `true`, like `mesh.remote`). With `false`, or `mesh.remote =
     false`, the node does not offer the capability and refuses with `node.policy_denied` (`rule = mesh.remote_helpers`).
   - The folder: refused unless it is a full path that exists. Folder identity rules (ADR-0052) and the hidden-character rules
     apply. The target's confinement (ADR-0047) and its `skip_permissions` policy (ADR-0059) apply to the helper as to any agent
     there.
   - The kind, `settings` and `launch_options` are checked against the target's manifests, as for a local helper
     (`agent.setting_unknown`, `agent.option_invalid`, `agent.option_forbidden`, ...). The prompt rules are checked in full.
   - Limits per asking machine (the hub-written `origin`, not the agent's word): 2 cards waiting at once, 6 running helpers. And
     3 running helpers per asking agent run, plus the node's own agent limit (`node.resource_limit`).
   - **No grant, ever.** A start from another machine is never covered by a standing grant and offers none (`grant_allowed:
     false`).
   - **Depth 1.** The helper cannot ask for helpers, here or anywhere.

   ADR-0057 decision 3 ("the helper is never wider than its caller") cannot compare folders or confinement across two
   machines. Its replacement is: the folder is always named and shown, never granted, and the target's own policy is the ceiling.

4. **The card shows and signs both machines.** The target raises an `ApprovalRequested` with a new action **`agent.start.remote`**.
   A new action, so a window that does not know it never shows it as a local start. Its `detail` is JSON with:
   - `origin_node` (the hub's word), `origin_agent` and `origin_agent_kind` (the asking machine's word);
   - `node` (this machine, where the helper starts), `kind`, `folder`, `worktree`;
   - `depth: 1`, `grant_allowed: false`, `settings` and `launch_options` as in ADR-0066 and ADR-0059;
   - `prompt`, in full (16 KiB at most).

   The decision is signed over a new string:

   ```
   agora-approval-start-v4\0node_id\0origin_node\0origin_agent\0approval_id\0verdict\0helper_grant\0helpers_may_start\0settings\0launch_options\0sha256(detail)
   ```

   It is v3 with the two machines' words added. `helper_grant` and `helpers_may_start` must be `0`. The target refuses any other
   value, and a v1–v3 or ADR-0028 signature on this action, with `approval.signature_invalid`. New vectors:
   `schema/fixtures/signing/approval-start-v4.json` (node, web window, terminal client). The card has no grant choices.

5. **The first answer comes inside the forward's deadline.** The target answers the forwarded `HelperStart` as soon as the card
   is raised, with a new result **`HelperStartAccepted { approval_id, ttl_ms }`**. A refusal before asking (decision 3) is the
   error itself. The hub relays either answer to the asking node as the answer to its `HelperStart`. The person's decision does
   not have to fit in 30 seconds.

6. **The outcome is a separate message.** When the person decides, or the card expires, or the helper fails to launch, the target
   sends a new node → hub request **`HelperStartResult { start_id, to_node, agent_id, terminal_id, error }`**.
   - The hub forwards it to `to_node` and writes the sender's certificate name as its `from_node`.
   - The hub keeps no table for it, so a hub restart in between loses nothing while both links come back.
   - The asking node accepts it only if `start_id` is waiting and was sent to that `from_node`. Anything else is dropped and
     audited.
   - The agent's call then answers `{ agent_id: "<target>/<agent>" }`, or one of: `mesh.start_denied` (refused or withdrawn),
     `approval.expired`, or a launch error the target gives.
   - The asking node waits `ttl_ms` plus 2 minutes from the first answer. After that the call answers `node.timeout`
     (`node_id` = the target).
   - The target sends the result again after each reconnection to the hub, for 15 minutes from the ask. The asking node keeps
     the first copy and ignores repeats.
   - `node.offline` and `node.unknown` come from decision 2.

   A link that drops never stops a helper. Nodes are autonomous: a started helper runs whether or not its result arrives. The
   tree (decision 9) still shows whose it is.

7. **Following across machines.** A new request **`HelperFollow { agent_id, from_agent, from_run_id, origin, wait_state,
   wait_ms, lines }`** carries `agent_wait` and `agent_output` to the target. The answer is a new result **`HelperFollowResult {
   state, lines[] }`**.
   - The target answers only for a helper whose record names this `origin` (the hub's word), `from_agent` and `from_run_id`.
     Anything else is `agent.not_found`.
   - Agent ids are never given twice on a node (ADR-0060), and the run binds the right to one run of the asker (ADR-0109).
   - `wait_ms` is at most 20 s, so each request fits in the forward's deadline. The asking node asks again until the agent's
     `timeout_s`, and keeps the existing 8 open waits per connection.
   - `agent_output` keeps its rules: the bottom of the live screen or the end of the transcript, at most 16 KiB, untrusted text.
     It passes through the hub like terminal bytes and is never stored there.

8. **The asking machine tells the target when the asker goes.** A new request **`HelperRelease { from_agent, from_run_id,
   start_id, stop_helpers, origin }`**:
   - when the agent cancels its call or its connection ends: `start_id` set, so that card is withdrawn;
   - when the asker exits: `start_id` empty, `stop_helpers = false`. Its waiting cards are withdrawn and its right to follow
     ends. Its helpers keep running, as ADR-0057 decision 7 says;
   - when the person stops it with `AgentStop { stop_helpers: true }`: `stop_helpers = true`. The target also stops that run's
     helpers there, and theirs.

   The target acts only for helpers and cards of that `origin` and run. When the target is offline, the asking node sends the
   message again after the target reconnects, for 15 minutes. After that, the person stops the helper from the window, where it
   shows on its own machine.

9. **The tree names the other machine, without changing `parent_agent_id`.** `AgentStarted`, `AgentInterrupted` and the client
   `AgentSnapshot` gain two optional fields:
   - `parent_node_id`: the asking machine, from the hub-written `origin`;
   - `parent_node_agent_id`: the asking agent on that machine.

   For a helper from another machine, `parent_agent_id` stays empty, so an older hub or window shows it as a top-level agent and
   never under a wrong local one. A same-node helper keeps `parent_agent_id` and leaves both new fields empty.

10. **Records.**
    - The asking node records its agent's ask and outcome (`AgentStartRefused` for its own agent, as today).
    - The target records the card, the decision and the start (`ApprovalRequested`, `ApprovalResolved`, `AgentStarted`).
    - The target records the first prompt as a cross-machine message from `origin_node`/`origin_agent`, as it records a
      cross-node `MeshPost` (ADR-0031).
    - The target's refusals before asking are audited on the target. They are not `AgentStartRefused` events there, because that
      event's `agent_id` names a local agent.
    - The hub audits each `HelperStart`, its first answer and each `HelperStartResult`.

## What the window and the terminal client show

- **The card** reads in UI words: "Claude Code on mac wants to start Codex on blackview". Under it: the folder on blackview, a
  note that this is another machine, the settings, any "acts without asking" option with its confirmation, and the whole prompt.
  There are no grant choices. Both clients sign v4.
- **The tree.** The helper shows under its parent with a machine tag ("on blackview"). In blackview's own list it shows with
  "started by Claude Code on mac". A parent that has ended keeps the helper under it, as for a local helper. New Fluent keys go in
  English and Turkish.
- **Older clients** show the helper at the top level. They show the `agent.start.remote` card as an unknown request, which cannot
  be approved, because the target accepts only v4.

## Security

New threat-model rows (numbers are the next free ones when built):

- **A compromised or misled machine asks many machines** to start agents, or floods them with cards. Mitigations:
  - every start needs a v4 decision signed by a device the target trusts and verified on the target;
  - no grant, depth 1, and the target's own policy and its off switch;
  - the hub limits per asking machine (12 a minute, 8 open), and the target limits per asking machine (2 cards, 6 helpers);
  - solo hub or the owner's switch.

  Residual: the person can be worn down by cards; `origin_agent` is the asking machine's word (only `origin_node` is vouched by the
  hub), and the card says so.
- **The prompt is untrusted content** written by an agent that may have been talked into it (T14). It is shown in full and
  signed by hash, and the target checks it against its own manifest's screen rules. After the first prompt, the helper treats
  messages from its parent as information (ADR-0031).
- **Screen text crosses machines.** `agent_output` carries up to 16 KiB of a helper's live screen through the hub to another
  machine's agent. It is limited to helpers that this run of that agent started, and never stored on the hub. Residual: a secret on
  the helper's screen reaches the asking agent, as it does locally today.
- **Forged answers.** A node writes a `HelperStartResult` or `HelperFollow` for someone else's start or helper. The hub writes
  `origin` and `from_node` from certificates. The asking node checks `start_id` against the target it asked, and the target checks
  origin, agent and run.

## Build order

1. **Schema (this ADR is its ADR).** The four new `CommandRequest` arms (`HelperStart`, `HelperStartResult`, `HelperFollow`,
   `HelperRelease`), the two new `CommandOk` results, the new fields of decision 9, the capability, the approval action, the v4
   string and its vectors, and golden fixtures. Update `protocol.md`, `mesh-tools.md` (replacing the "not supported yet"
   sentence) and the threat model.
2. **The target node:** take `HelperStart`, check, raise the card, verify v4, launch, send the result, answer `HelperFollow`
   and `HelperRelease`. Tests use two nodes and `agora-fake-agent`.
3. **The hub:** route the four requests, write `origin`/`from_node`, apply the limits and the solo-hub rule, and audit.
4. **The asking node:** `agent_start` with `node`, the wait, `agent_wait`/`agent_output` in slices, and `HelperRelease` on
   cancel, exit and stop.
5. **The web window and the terminal client:** the card with v4 signing, and the tree with the machine tag. Screenshots in both
   themes.
6. A changelog fragment with each user-visible step.

## Consequences

- Agents can start a helper on another machine, with the person's signed approval on that machine, and follow and stop it as
  they do locally.
- Four new requests, two new results, two new event fields, one capability, one approval action, one signing string, one node
  policy key. No published message changes meaning. ADR-0057's local behaviour is unchanged.
- The hub learns more about the ask than for a local start (kind, folder, prompt hash in its audit). It still cannot approve,
  widen or change a start.
- A cross-machine helper is shallower than a local one (depth 1, no grant), by design.

## Alternatives considered

- **`MeshPost` to the target's node:** mesh content is untrusted data and must never start anything. Making one kind of post
  carry authority would break the rule for every post.
- **Reuse `AgentStart` or `AgentLaunch`:** they are the person's starts. Sent on an agent's behalf they skip the target's
  `agent.start` card, carry no asking agent, and the target could not tell an agent's ask from the person's.
- **One long-lived forward** (raise `COMMAND_DEADLINE` for this command): it holds an open forward for up to 12 minutes against
  the per-node limit of 64, it is lost when the hub restarts, and every other command's deadline stops meaning one thing.
- **A `BoardCall` tool value:** the hub answers those itself. The target would never see the request or raise its own card.
- **A standing grant across machines:** it would let one signature start agents on another machine later, out of the person's
  sight. Every cross-machine start is asked.

## Open questions for the maintainer

1. Default of the target's `mesh.remote_helpers`: `true` (proposed, like `mesh.remote` and `mesh.fleet`), or `false` so each
   machine opts in?
2. Team hubs: carry cross-machine starts only on a solo hub or with the owner's switch (proposed, as ADR-0112), or never on a team
   hub?
3. Launch options that skip permissions on another machine: allowed with the same confirmation as locally (proposed), or refused
   for cross-machine starts at first?
4. Depth: only a person-started agent may ask, and its remote helper never asks (proposed). Allow ADR-0057's helper-of-a-helper
   chain across machines later?
5. Limits: 12 a minute and 8 open per asking machine at the hub; 2 cards and 6 running helpers per asking machine at the target
   (proposed). Other numbers?
6. Folder: a full path on the target (proposed). Should an agent also be able to say "this project's checkout on that machine",
   for the target to find?
7. Undelivered results and releases are re-sent for 15 minutes after reconnection, then dropped (proposed). Keep them longer, or
   durably like ADR-0109?

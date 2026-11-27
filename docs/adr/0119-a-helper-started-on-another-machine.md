# 0119. A helper started on another machine
Status: Accepted
Date: 2026-11-26

Accepted: 2026-11-27 (the maintainer's explicit answer "Accept, with the defaults" to a question that summarised this revision: chains
down to 3 levels, a standing grant from the first signed card, 6 running at once, 30 starts an hour, until the root run ends or 24 hours,
skip-permissions always asked). The open questions' proposed answers stand, and the numbers in decision 12 are the defaults. Accepting
this ADR approves its schema step (build order 1) as described here.

Revised 2026-11-27 at the maintainer's request: "an agent must be able, on instruction or by message, to open agents on another
machine or under another agent, again and again, without asking each time". Asked how, the maintainer chose **"approve once, then
free"**: the first cross-machine start is approved with the person's signature, and that approval can be a standing grant on the
target, so later starts it covers start at once, until it ends or is revoked. This replaces the first draft's "no grant, ever, depth 1"
(decisions 1, 3, 4 and the alternatives changed; decision 12 is new).

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
   - takes another machine only when this node and the hub have the capability `mesh.spawn.remote` (the name the refusal
     already uses) and this node's `mesh.remote` is `true`. The asking node cannot know the target's capabilities; the hub
     checks them (decision 2);
   - needs `folder`, a full path on the target. The asker's folder means nothing on another machine, so there is no default;
   - is allowed to an agent the person started (depth 0), and to a helper (here or on another machine) whose own start allowed
     helpers to ask in turn (`helpers_may_start`, decision 12), down to depth 3 counted from the person's agent. Deeper is refused
     (`mesh.start_denied`, nothing is asked);
   - counts against the asker's existing limits on its own node: one start in progress, six starts a minute, and its running
     helpers (local and remote together: 3, or the grant's number when one covers the start). A refusal here is the node's usual
     `AgentStartRefused` for its own agent;
   - is checked against the prompt rules that need no manifest (control and invisible characters, leading `!`, `/`, ...). The
     target checks the rest against its own manifest.

   The node then sends a new node → hub request **`HelperStart`** (a new `CommandRequest` arm):
   `{ start_id, target_node, kind, folder, prompt, worktree, settings[], launch_options[], from_agent, from_run_id, origin,
   depth, root_node, root_agent, root_run_id, from_agent_kind }`. `start_id` is 16 random bytes the asking node chooses. `from_run_id` is the asking
   agent's run (ADR-0109). `origin` is empty; the hub writes it. `depth` and `root_*` name the person-started agent the chain began
   with (the asker itself at depth 0). The hub writes them, like `origin` (review of the schema step): from its own record of the
   cross-machine starts it carried, so a machine cannot claim a place in another's chain; an asker it carried no start for is depth 0
   and its own root, and after a hub restart every asker is (chain grants then stop applying; they never widen).

2. **The hub carries it and widens nothing.**
   - It accepts `HelperStart` only on a node's connection. From a window it is `protocol.invalid_message`.
   - It writes `origin` from the connection's certificate, whatever the node wrote (as for `MeshPost`). `from_agent` and
     `from_run_id` stay the asking node's word: a node can vouch only for its own agents.
   - It refuses `node.unknown`, `node.offline`, a target that is the asker itself (`protocol.invalid_message`, `field =
     target_node`) and a target without the capability (`protocol.capability_missing`, `mesh.spawn.remote`).
   - It rate-limits per asking machine, across all targets: 12 starts a minute, and 8 whose first answer (decision 5) has not
     come back (`mesh.rate_limited`). That count covers only the first leg, which lasts at most 30 s; the hub keeps nothing
     after it.
   - The four new requests (`HelperStart`, `HelperStartResult`, `HelperFollow`, `HelperRelease`) count against the existing
     limit of 64 open forwards per node (`MAX_NODE_POSTS`). `HelperStartResult` and `HelperRelease` are also limited to 60 a
     minute per node.
   - Like the fleet view (ADR-0112), it carries starts on a solo hub. On a hub with members it carries them only when the owner
     turned on a new hub setting, `remote_helpers` (off by default there), because a member's agent could otherwise ask a machine
     it has no grant for. Otherwise it refuses with a new error code `mesh.remote_helpers_off` (no parameters; English and
     Turkish Fluent text). ADR-0112's fleet switch and `board.not_allowed` keep their meaning.
   - It writes an audit row (`agent.start.remote.asked`) and forwards the request to the target. It never approves anything.

3. **The target decides from its own policy.** ADR-0020: the hub cannot widen what a node allows.
   - Node policy key `mesh.remote_helpers = true|false` (default `true`, like `mesh.remote`). With `false`, or `mesh.remote =
     false`, the node does not offer the capability, so the hub refuses first. A request that still arrives (the policy changed
     while the link was up) is refused with `node.policy_denied` (`rule = mesh.remote_helpers`).
   - The folder: refused unless it is a full path that exists. Folder identity rules (ADR-0052) and the hidden-character rules
     apply. The target's confinement (ADR-0047) and its `skip_permissions` policy (ADR-0059) apply to the helper as to any agent
     there.
   - The kind, `settings` and `launch_options` are checked against the target's manifests, as for a local helper
     (`agent.setting_unknown`, `agent.option_invalid`, `agent.option_forbidden`, ...). The prompt rules are checked in full.
   - Limits per asking machine (the hub-written `origin`, not the agent's word): 2 cards waiting at once, 6 running helpers. And
     3 running helpers per asking agent run, plus the node's own agent limit (`node.resource_limit`). For the starts a grant covers,
     the grant's own numbers (decision 12: N running, 30 an hour) take the place of the per-run 3 and the per-machine 6; the node's own
     agent limit always holds.
   - **A standing grant may cover it** (decision 12). A start that matches a live grant on this node starts at once, with no card,
     and is recorded as started under that grant. Anything else raises the card, which offers a grant (`grant_allowed: true`)
     unless the start names a launch option that skips permissions (that is always asked, as locally).
   - **Depth.** A helper may ask in turn only when its start allowed it (`helpers_may_start`), down to depth 3 (decision 1).

   ADR-0057 decision 3 ("the helper is never wider than its caller") cannot compare folders or confinement across two
   machines. Its replacement is: the folder is always named and shown (on the card, and in the grant the person signs), and the
   target's own policy is the ceiling.

4. **The card shows and signs both machines.** The target raises an `ApprovalRequested` with a new action **`agent.start.remote`**.
   A new action, so a window that does not know it never shows it as a local start. Its `detail` is JSON with:
   - `origin_node` (the hub's word), `origin_agent` and `origin_agent_kind` (the asking machine's word);
   - `node` (this machine, where the helper starts), `kind`, `folder`, `worktree`;
   - `depth`, `root_node`, `root_agent`, `grant_allowed`, `settings` and `launch_options` as in ADR-0066 and ADR-0059;
   - `prompt`, in full (16 KiB at most).

   The decision is signed over a new string:

   ```
   agora-approval-start-v4 NUL F(node_id) F(origin_node) F(origin_agent) F(approval_id) F(verdict) F(settings) F(launch_options)
     F(grant_max_running) F(grant_any_kind) F(grant_below_folder) F(grant_chain) F(grant_until_ms) F(sha256(detail))
   ```

   where F(x) is the decimal byte length of x, `:`, then x (the form of the newer domains, since two machines' strings are in it);
   `verdict` and the three flags are `0`/`1`, numbers are decimal, `settings` and `launch_options` are written as in v3. The grant is
   signed field by field (never as JSON, so the node, the web window and the terminal client produce the same bytes); with no grant,
   `grant_max_running` is `0` and the other grant fields `0`. The kind, folder and asker a grant covers are never taken from the
   decision: they are the node's own rendered request. `ApprovalResolve.helper_grant` and `helpers_may_start` (published for a local
   start, with other meanings) must be 0 and false on this action. The target refuses a grant on a start that offered none, any
   other value there, and a v1–v3 or ADR-0028 signature on this action, with `approval.signature_invalid`. New vectors:
   `schema/fixtures/signing/approval-start-v4.json` (node, web window, terminal client).

5. **The first answer comes inside the forward's deadline.** The target answers the forwarded `HelperStart` as soon as the card
   is raised, with a new result **`HelperStartAccepted { approval_id, ttl_ms }`**. A refusal before asking (decision 3) is the
   error itself. The hub relays either answer to the asking node as the answer to its `HelperStart`. The person's decision does
   not have to fit in 30 seconds.

   When the first leg fails after it was sent (`node.timeout`, or the link drops), the target may already have raised a card.
   The asking node then sends `HelperRelease { start_id }` (decision 8), so no card is left that would start a helper nobody
   tracks. The agent's call answers with the first leg's error.

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
   tree (decision 10) still shows whose it is.

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

9. **Restarts.** The records of decisions 6–8 live in memory, for the node process's life, as the local spawner's do.
   - **The target restarts:** its waiting cards are gone, as local ones are. `HelperFollow` and `HelperRelease` for helpers
     started before answer `agent.not_found`. A helper kept by the terminal holder (ADR-0079) or offered again
     (`AgentInterrupted`) keeps its parent fields (decision 10), so the tree still shows whose it is.
   - **The asking node restarts:** its agents' calls end with their connections. It no longer knows which targets it asked, so
     it sends no `HelperRelease`. Cards on the target expire by their own TTL, and started helpers keep running. The person
     stops them from the window.

10. **The tree names the other machine, without changing `parent_agent_id`.** `AgentStarted`, `AgentInterrupted` and the client
   `AgentSnapshot` gain two optional fields:
   - `parent_node_id`: the asking machine, from the hub-written `origin`;
   - `parent_node_agent_id`: the asking agent on that machine.

   For a helper from another machine, `parent_agent_id` stays empty, so an older hub or window shows it as a top-level agent and
   never under a wrong local one. A same-node helper keeps `parent_agent_id` and leaves both new fields empty.

11. **Records.**
    - The asking node records its agent's ask and outcome (`AgentStartRefused` for its own agent, as today).
    - The target records the card, the decision and the start (`ApprovalRequested`, `ApprovalResolved`, `AgentStarted`).
    - The target records the first prompt as a cross-machine message from `origin_node`/`origin_agent`, as it records a
      cross-node `MeshPost` (ADR-0031).
    - The target's refusals before asking are audited on the target. They are not `AgentStartRefused` events there, because that
      event's `agent_id` names a local agent.
    - The hub audits each `HelperStart`, its first answer and each `HelperStartResult`.

12. **Standing grants on the target** (the maintainer's "approve once, then free"). The card offers, beside Allow and Deny, "Allow
   later starts like this". Its scope, shown on the card, chosen there and signed (decision 4):
   - **who may ask:** this asking agent's run (`origin_node`, `from_agent`, `from_run_id`), and, when "its helpers may ask too" is
     ticked, every agent of the chain it roots (`root_node`, `root_agent`, `root_run_id`, written by the hub from the starts it
     carried) asking from any machine of this hub;
   - **what:** the kind asked, or any kind this node has with an agent integration (the grant rules of ADR-0057: never `shell`, a
     shell or a wrapper); settings as shown on the card (a later start with other settings is asked); never a launch option that
     skips permissions;
   - **where:** this folder, or any folder below it (the person picks);
   - **how many:** at most N of its helpers running at once on this node (default 6, at most 20), and at most 30 starts an hour;
   - **until:** the root agent's run ends, or 24 hours after the decision, whichever comes first; the person may pick a shorter
     time. The 24 hours always hold: a restarted asking node sends no `HelperRelease` (decision 9), and in a chain the root's node
     may never have talked to this target.

   A later `HelperStart` that matches every part starts at once: the target answers it with `HelperStartAccepted` and sends
   `HelperStartResult` as soon as the helper runs, raises no card, and records `AgentStarted` with the grant's id and an audit row
   `agent.start.remote.granted`. The hub's limits (decision 2) still apply. A grant lives in memory on the target (it ends when the
   target's node restarts, as local grants do) and ends early when the root run ends (`HelperRelease`, decision 8), when revoked
   (`GrantRevoke` with its id; `GrantList` lists remote grants with their scope, use and end), or when its machine is forgotten.
   The window shows an indicator per machine while a remote grant is live ("Codex on mac may start agents here: 2 of 6 running"),
   with Revoke.

## What the window and the terminal client show

- **The card** reads in UI words: "Claude Code on mac wants to start Codex on blackview". Under it: the folder on blackview, a
  note that this is another machine, the settings, any "acts without asking" option with its confirmation, and the whole prompt.
  Below the buttons, "Allow later starts like this" opens the grant's choices (decision 12) with their defaults; the person sees the
  scope in plain words before signing. Both clients sign v4.
- **A live grant** shows on the target machine's row and in the agent's details, with what it covers, how much of it is used and
  Revoke.
- **The tree.** The helper shows under its parent with a machine tag ("on blackview"). In blackview's own list it shows with
  "started by Claude Code on mac". A parent that has ended keeps the helper under it, as for a local helper. New Fluent keys go in
  English and Turkish.
- **Older clients** show the helper at the top level. They do not read `agent.start.remote` as a start request (the web window
  reads only `agent.start` as one). Whatever they show for it cannot approve it: the target accepts only a v4 signature.

## Security

New threat-model rows (numbers are the next free ones when built):

- **A compromised or misled machine asks many machines** to start agents, or floods them with cards. Mitigations:
  - every start needs a v4 decision signed by a device the target trusts and verified on the target, or a live grant such a
    decision created;
  - a grant is bounded (one asker or one chain, one kind or integrated kinds, one folder tree, N running, 30 an hour, until the
    root run ends or at most 24 hours), revocable at once, visible while live, and never covers skipping permissions; depth 3;
    and the target's own policy and its off switch;
  - the hub limits per asking machine (12 a minute, 8 open), and the target limits per asking machine (2 cards, 6 helpers);
  - a solo hub, or a hub whose owner turned on `remote_helpers`.

  Residual: the person can be worn down by cards; `origin_agent` is the asking machine's word (`origin_node` and the chain's root
  are vouched by the hub), and the card says so. **Under a grant, starts happen out of the person's sight:** an agent that was
  talked into it (T14), or a compromised machine that really is part of the granted chain, can start up to N
  agents of the granted kinds in the granted folders on the target until the grant ends. That is the trade the maintainer chose; the
  bounds above, the indicator, the audit rows and Revoke limit it.
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

1. **Schema (this ADR is its ADR).** Everything is additive; no published field changes meaning:
   - `CommandRequest` arms `HelperStart` (decision 1's fields, with `depth` and `root_node`, `root_agent`, `root_run_id`),
     `HelperStartResult`, `HelperFollow` and `HelperRelease` (decisions 6–8), numbered after the last arm;
   - `CommandOk` results `HelperStartAccepted { approval_id, ttl_ms }` and `HelperFollowResult { state, lines[] }`;
   - on `ApprovalResolve`, a new nested `RemoteGrant remote_grant` (`max_running`, `any_kind`, `below_folder`, `chain`,
     `until_ms`), the only place a remote grant is given;
   - on `AgentStarted`, a `grant_id` (the grant a start ran under; empty when it was asked), and decision 10's `parent_node_id`
     and `parent_node_agent_id` on `AgentStarted`, `AgentInterrupted` and the client `AgentSnapshot`;
   - `RemoteGrantInfo`, listed in a new `GrantListResult.remote_grants` apart from `GrantInfo` (so an older window never reads a
     remote grant as a used-up local one);
   - the capability `mesh.spawn.remote` (defined, advertised by no one until steps 2–4; both sides refuse the four requests with
     `protocol.capability_missing` until then), the approval action `agent.start.remote`, the error code
     `mesh.remote_helpers_off`, the v4 string and its vectors, and golden fixtures (added, never regenerated).
   Update `protocol.md`, `mesh-tools.md` (replacing the "not supported yet" sentence), `error-codes.md` and the threat model.
2. **The target node:** take `HelperStart`, check, raise the card, verify v4, launch, send the result, answer `HelperFollow`
   and `HelperRelease`; keep, match, count, end and list standing grants (decision 12). Tests use two nodes and `agora-fake-agent`.
3. **The hub:** route the four requests, write `origin`/`from_node`, apply the limits and the solo-hub rule, and audit.
4. **The asking node:** `agent_start` with `node`, the wait, `agent_wait`/`agent_output` in slices, and `HelperRelease` on
   cancel, exit and stop.
5. **The web window and the terminal client:** the card with v4 signing and its grant choices, the live-grant indicator with
   Revoke, and the tree with the machine tag. Screenshots in both themes.
6. A changelog fragment with each user-visible step.

## Status of the build

1. Schema: done (2026-11-27). The four requests (122..125), the two results (41, 42), `RemoteGrant` on `ApprovalResolve` (8), the
   new fields on `AgentStarted`, `AgentInterrupted` and `AgentSnapshot`, `RemoteGrantInfo` in `GrantListResult.remote_grants`, the capability constant (offered by no one), the
   error code `mesh.remote_helpers_off`, the v4 vectors with their test, golden fixtures, `protocol.md` ("Helpers on another machine")
   and threat-model row T73. Node and hub refuse the four requests with `protocol.capability_missing` until steps 2 and 3.
2. The target node: built, except that its results are only queued (step 3's hub carries them, and the link sends them then) and an
   interrupted helper's `AgentInterrupted` does not yet carry its parent's machine (the ledger keeps only a local parent).
   `crates/agora-node/src/spawn/remote.rs`: `HelperStart`, `HelperFollow` and `HelperRelease` are answered only on the hub link
   (`uplink_client::run_routed`), never from a local client; the policy key `mesh.remote_helpers` (default true, only with
   `mesh.remote`); the checks (a full folder path, the kind's manifest, settings and options, the prompt rules, depth below 3, 2
   cards and 6 running per asking machine and 3 per asking run without a grant); the card `agent.start.remote` decided over v4
   (`approvals.rs`); standing grants (who: the run, or the chain by its hub-written root; what; where; N running; 30 an hour; the end
   by this node's clock), listed in `GrantListResult.remote_grants` and revoked with `GrantRevoke`; the helper's `AgentStarted` with
   `parent_node_id`, `parent_node_agent_id` and `grant_id`; an expired card answered `approval.expired`; follow limited to the asking
   run (20 s waits, 200 lines); release withdraws that run's cards, ends its follow rights and the grants rooted in it, and stops its
   helpers here through the node's own stop path when a person stopped it with its helpers. The node still does not offer
   `mesh.spawn.remote`. Tests: `crates/agora-node/tests/agent_spawn.rs` (the whole flow) and `approvals.rs` (v4 and the grant rules).
   From the review: every use re-checks that the grant's signer still vouches for the decision (a revoked device, one out of its term
   or one now limited to some projects ends its grants); starts that arrive together reserve their place under the grant, so
   `max_running` holds; a card the node judged not grantable (settings, launch options, a kind a grant may not cover) takes no grant
   whatever the decision says (`RemoteBinding.grant_allowed`, from the node's own record); at most 8 follows of one machine are
   answered at once (`mesh.too_many_asks`); a chain grant matches only a non-empty hub-written root node and run. Grants live in
   memory, so a node restart ends them all (they narrow, never widen), and one rooted in a run ends early only through a release from
   the root's own machine: a machine that dies silently leaves its grants until their end (at most 24 hours).
3. The hub: built (`crates/agora-hub/src/helpers.rs`, `server.rs` `helper_for_node`). It carries the four requests only between
   machines that both agreed on `mesh.spawn.remote` (it lists the capability; a node offers it once step 4 is built), on a solo hub or
   with the owner's `agora hub remote-helpers on` (else `mesh.remote_helpers_off`). It writes `origin` on every request, the chain's
   `depth` and root on a start from its own record of the starts it carried (an asker it carried none for is depth 0 and its own
   root), and `from_node` on a result; it forwards a result only for a start it carried to that machine, and only to the machine that
   asked (anything else is dropped and audited); the new helper joins the asker's chain one level deeper only once the hub sees its
   own `AgentStarted` start it new on the target under exactly that parent (review of step 3), so a target cannot bring its other
   agents into someone's chain. Per asking machine: 12 starts a minute and 8 waiting
   for their first answer (`mesh.rate_limited`), within the node's 64 open forwards. Audit rows `agent.start.remote.asked`,
   `agent.start.remote.result` and `agent.start.remote.released`. The record is in memory: after a hub restart every asker is its own
   root (chain grants stop applying). Tests: `crates/agora-hub/tests/server.rs`, `helpers.rs` and `users.rs`.
4. The asking node: built except following (`agent_wait`/`agent_output` for `<machine>/<agent>` through `HelperFollow`).
   `agent_start` with `node` set to another machine is checked here (the asker runs and may ask, a full folder, the prompt rules, 3
   helpers local and remote together, the start rate) and sent through the hub link (`MeshOut::request`); the call answers with the
   helper once the hub carries the target's `HelperStartResult` back (`uplink_client::run_routed`), `node.timeout` past the card's time
   plus 2 minutes (and the card there is withdrawn); the asker's end, and a stop with its helpers, send `HelperRelease` to every machine
   it asked (with the run it asked from). The target's results go out on the same link, offered again for 15 minutes when the hub
   cannot take them. The node offers `mesh.spawn.remote` while its policy allows. End to end through a real hub:
   `crates/agora/tests/hub_dial.rs` `an_agent_starts_a_helper_on_another_machine_with_the_persons_decision_there`.
5. The window and the terminal client: in part. The web window reads `agent.start.remote` (`net/view.ts` `startRequest`), shows the
   card with both machines and "More like this" as a standing grant (6 at a time; its helpers may ask too), signs v4
   (`net/approval.ts` `remoteStartSignedMessage`, checked against the shared vectors) and sends `remote_grant`; a helper from another
   machine hangs under its parent there in the agent tree (the hub fills `AgentSnapshot.parent_node_id` and
   `parent_node_agent_id`). The terminal client signs v4 for this start only (no grant). Not built yet: the live-grant indicator with
   Revoke, the choice of a time, "any integrated kind" and "folders below" on the card, and following a remote helper from the asking
   side. Screenshots: the gallery's `[data-start-remote]` in both themes (visual baselines).

## Consequences

- Agents can start a helper on another machine, with the person's signed approval on that machine, and follow and stop it as
  they do locally.
- Four new requests, two new results, two new fields on three messages, one capability, one approval action, one signing string,
  one node policy key, one hub setting, one error code. No published message changes meaning. ADR-0057's local behaviour is unchanged.
- The hub learns more about the ask than for a local start (kind, folder, prompt hash in its audit). It still cannot approve,
  widen or change a start.
- A cross-machine helper can be covered by a standing grant, like a local one, and can start helpers in turn down to depth 3 when
  its grant allows; the grant's scope is signed on the target, so the hub still cannot widen it.

## Alternatives considered

- **`MeshPost` to the target's node:** mesh content is untrusted data and must never start anything. Making one kind of post
  carry authority would break the rule for every post.
- **Reuse `AgentStart` or `AgentLaunch`:** they are the person's starts. Sent on an agent's behalf they skip the target's
  `agent.start` card, carry no asking agent, and the target could not tell an agent's ask from the person's.
- **One long-lived forward** (raise `COMMAND_DEADLINE` for this command): it holds an open forward for up to 12 minutes against
  the per-node limit of 64, it is lost when the hub restarts, and every other command's deadline stops meaning one thing.
- **A `BoardCall` tool value:** the hub answers those itself. The target would never see the request or raise its own card.
- **Keep the action `agent.start` and add keys to its detail:** an older window would read it as a local start. It would show
  the asking agent as one on the target machine and offer grant choices the target must refuse.
- **Set `parent_agent_id` to the remote parent and add only `parent_node_id`:** the hub and the window qualify
  `parent_agent_id` with the helper's own node, and ids are `agent-N` per node. An older hub or window would hang the helper
  under an unrelated local agent. That changes the field's published meaning.
- **No grant across machines (every start asked):** the first draft. The maintainer rejected it on 2026-11-27: agents must start
  agents on other machines repeatedly without a card each time.
- **A machine policy line with no card at all** ("agents of my machines may start agents here"): offered to the maintainer and not
  chosen. Anyone who took over an agent could start agents on that machine freely, with no signature anywhere.

## Open questions for the maintainer

1. Default of the target's `mesh.remote_helpers`: `true` (proposed, like `mesh.remote` and `mesh.fleet`), or `false` so each
   machine opts in?
2. Team hubs: carry cross-machine starts only on a solo hub or with the owner's switch (proposed, as ADR-0112), or never on a team
   hub?
3. Launch options that skip permissions on another machine: allowed with the same confirmation as locally (proposed), or refused
   for cross-machine starts at first?
4. Depth: decided by the revision (chains allowed under a grant). Is 3 levels from the person's agent right?
5. Limits: 12 a minute and 8 open per asking machine at the hub; 2 cards and 6 running helpers per asking machine at the target
   (proposed). Other numbers?
6. Folder: a full path on the target (proposed). Should an agent also be able to say "this project's checkout on that machine",
   for the target to find?
7. Undelivered results and releases are re-sent for 15 minutes after reconnection, then dropped, and the records live only for
   the node process's life (proposed). Keep them longer, or durably like ADR-0109?
8. Grant defaults: N = 6 running at once (at most 20), 30 starts an hour, until the root run ends (at most 24 hours), this folder
   (or below it, when picked), the kind asked (or any integrated kind, when picked). Other numbers?
9. Should a grant survive a restart of the target's node (stored and signed), or end with it (proposed, as local grants)?

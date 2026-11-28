# 0131. An idle agent can ask for the next task
Status: Accepted
Date: 2026-11-27
Accepted: 2026-11-27 (maintainer approval in chat).

## Context

ADR-0095 lets a person give a task to an existing agent. An agent can already request a specific task
with `task_claim`. M8.2 calls for an idle, eligible agent to take the next available task. A scheduler
that injects input based on the window's last observed idle state can race with human input, an
approval request or a disconnection. ADR-0117 and ADR-0118 address detection quality separately.

## Decision

1. Start with a pull model. An already running agent explicitly calls `task_take_next` from its own mesh
   endpoint when it is ready. The hub coordinates claims on the shared board. It never starts a run or
   sends a prompt as part of this rule. The successful tool response carries the claimed task and its
   existing untrusted task data. Execution follows the agent's current approvals, limits and confinement.
2. Require a Review-enabled shared project with an owner opt-in and an explicit allow-list of node IDs
   and agent kinds. The option starts off. Require a node-local policy opt-in as well. The node checks
   the calling run, project and current launch mode. Unknown run, project or launch mode, and a launch
   mode that skips permission prompts, make the caller ineligible. The hub checks the live node link,
   negotiated capability and board membership. It refuses during board joining, hub outage or uncertain
   run identity. There is no local-board fallback for this tool.
3. Select the first open task whose dependencies are accepted, in the board's creation order with a
   stable tie-break by task ID. The task must have been explicitly marked eligible for automatic pickup
   by the owner. Within one board transaction, verify that the calling run holds no active task claim,
   choose the task, claim it and persist an outcome keyed by project, run and a node-issued request
   sequence. Manual dispatch and manual claims compete for the same transaction. Review items do not consume a claim slot, although
   their dependencies remain blocked until acceptance. Normal task completion and Review still apply.
4. Keep the latest committed sequence and its result for each active run. A retry of that sequence
   returns the same outcome. Reject older sequences without attempting another claim. A newer request
   from a run that already has a claim returns its existing claim reference. Reuse the existing
   listing path to recover task data. A node continues its sequence after same-run reattachment only
   after recovering the sequence or obtaining the hub's committed value. A run that is confirmed
   ended follows the board's normal claim release rules. A temporary disconnect does not release
   its work by itself. Start with at most one request per run per second and 20 per project per
   second. Empty, stale, disabled, unsupported and rate-limited outcomes are distinguishable.
5. Record which rule assigned the task, which run took it and when. Show the source on the board and
   allow the owner to switch off future automatic claims without disturbing accepted or active work.
   Automating the selection confers no approval. Human-driven dispatch and ADR-0130's personal notices
   apply only where there was a verified human dispatcher.
6. Add a versioned capability for the new tool on the existing `BoardCall` path and additive settings
   and task eligibility metadata for clients. Obtain schema approval before implementation. Start with
   one eligible task and two fake agents, then enable the rule for one project before expanding scope.

## Alternatives

- Hub push on an idle event. It offers a hands-free loop but requires trusted readiness and
  exactly-once input delivery across restarts and mode changes.
- Have each agent list tasks, then claim one. This works today and remains available. It puts selection,
  project policy and recovery of a lost claim response in each agent's prompt or script.

## Consequences

An agent needs an explicit tool call to participate. The pull model can ship independently of the
detection proposals because readiness is expressed by the caller, while authorization and eligibility
remain verified by the node and hub.

## Verification

Race two eligible fake agents and a manual dispatch on one task. Lose a reply after commit, restart
the hub and repeat the request. Check that a single claim survives and the existing claim can be
recovered with a different request ID. Exercise dependencies, returned reports, an ineligible task,
an old hub, an offline node, ended and reattached runs, fabricated project details, policy changes and
skip-permissions modes. Drive a polling burst without delaying terminal input. Append protocol fixtures.

## Amendment A (2026-11-28, schema approved by the maintainer in chat): the exact shape

Decision 6's separate schema approval was given on 2026-11-28 for this shape as it stood at 77625e20.

**Capability.** `board.take_next.v1` on the hub/node link (next to `board.subtasks.v1`). The hub answers `task_take_next` only on a link
that negotiated it; on another link, and from a node that does not offer it, the answer is `protocol.capability_missing`
(`capability`). An older hub answers an unknown tool as `protocol.invalid_message` (field `tool`); the node reports both as unsupported.

**The tool** (`docs/specs/mesh-tools.md`): `task_take_next {}`. The agent gives no arguments. The node fills the call, never from the
agent's arguments, and adds two fields to `BoardCall`: `bytes from_run_id = 6` (the calling run's 16-byte id, ADR-0109) and
`uint64 request_seq = 7` (the node's sequence for that run, from 1). They are honoured only for `task_take_next`: every other tool
leaves them empty, and the hub ignores them there, so no other tool comes to trust them.
**The run is the calling node's.** `from_run_id` is the node's word, so the hub keys the sequence and the outcome by (project, node id
from the link's certificate, run id) and takes a run only when its projection knows it as running on that node (its `AgentStarted`
with that run id, not ended); any other is `task.pickup_off` `unknown_run`. A node can therefore never read, overtake or stall another
machine's run, and recovery (below) tells a node only what its own run already got.
Results (JSON, as other board tools):
- `{"outcome":"claimed","request_seq":N,"task":{...}}`: the task as `task_list` shows it, its text untrusted task data as today.
- `{"outcome":"existing","request_seq":N,"task":{...}}`: the run already holds a claimed task; nothing new is claimed.
- `{"outcome":"empty","request_seq":N}`: no eligible task now.
Tool errors (`{"code":..., "data":...}`), each one distinct:
- `task.pickup_off` (`reason`), new code: the rule does not apply to this caller now. The node, before it asks: `node_policy` (its own
  opt-in is off), `skips_permissions` (the run's launch mode skips permission prompts), `unknown_run` (it cannot name the run, project or
  mode). The hub: `owner_off` (the project's setting is off), `review_off`, `not_shared`, `node` or `kind` (not on the owner's
  allow-list; the kind is the one the hub recorded at the run's start, never taken from the call), and `unknown_run` (a run it cannot
  place on the calling node).
- `task.pickup_stale` (`committed_seq`), new code: an older sequence than the one the hub committed for this run; nothing is tried.
- `rate.limited` (`limit`): more than 1 request per run per second, 5 per node per second or 20 per project per second (the project's
  and the node's limits also stop a node from minting run ids to get round the run's).
- `board.unavailable` (`project_id`): the board is joining, or the node has no hub link. There is no local fallback: the node never
  runs this tool on its own broker, also when the hub says the project is not shared (that is `task.pickup_off`, `not_shared`).
- `protocol.capability_missing` (`capability`): unsupported, as above.
`request_seq` 0 asks only for the hub's committed sequence and outcome for the run (after a node restart or a reattachment of the same
run) and claims nothing; the node goes on from the committed value.

**Owner settings** (hub-side, unsigned: they widen nothing on a node, which keeps its own opt-in; the unscoped owner's, as ADR-0130's):
- command 137 `TaskPickupSettingsSet { string project_id = 1; bool enabled = 2; repeated string node_ids = 3; repeated string
  agent_kinds = 4; }`: at most 32 of each; the lists are the allow-list (empty means nobody).
- command 138 `TaskPickupSettingsGet { string project_id = 1; }`, result 49 `TaskPickupSettings { string project_id = 1; bool enabled =
  2; repeated string node_ids = 3; repeated string agent_kinds = 4; bool active = 5; string inactive_reason = 6; }` (`review_off`,
  `not_shared`). Each list holds at most 32 entries, each a node id or a kind of at most 64 characters of `[A-Za-z0-9._-]`.
- command 139 `TaskPickupEligibleSet { string project_id = 1; string task_id = 2; bool eligible = 3; }`: the owner marks an open task
  as one the rule may take; answered `Empty`.

**Events** (hub only; a node's is refused):
- arm 99 `TaskPickupEligible { string project_id = 1; string task_id = 2; bool eligible = 3; }`.
- arm 100 `TaskPickedUp { string project_id = 1; string task_id = 2; string node_id = 3; string agent_id = 4; bytes run_id = 5;
  uint64 at_ms = 6; string rule = 7; }` (`rule` is `take_next`): which rule assigned the task, which run took it and when (decision 5).
- `TaskSnapshot` gains `bool pickup_eligible = 8` and `TaskPickedUp picked_up = 9`, and both are sent after the snapshot when they did
  not fit in it, as review and parent links are (`task_metadata_overflow`).

**Lifetimes.** A mark stays until the owner clears it: a marked task that goes back to the Backlog (a run's confirmed end releasing its
claim, as the board does) can be taken again, and every pickup is its own `TaskPickedUp`. `enabled = false` stops future pickups only;
claims already made and their outcome rows stay, and a change of the allow-lists does nothing to a run that holds a claim. A run's
outcome row is dropped when its end is confirmed, so the table is bounded by the running runs.

**Visibility and ADR-0130.** `TaskPickedUp` names the node, agent and run, and is scoped as the board's other events are: the owner's
alone today, since a scoped person's view holds no board. A pickup has no verified human dispatcher, so ADR-0130's personal notice is
never sent for it; its project's channel notice still is.

**Node policy** (configuration, not schema): `[mesh] take_next = false` by default; the node checks the run, project and launch mode
before it asks.

**Not schema, stated for the review:** the outcome row (project, run → committed sequence and result) is written in the claim's own
board transaction and survives a hub restart; a confirmed run end follows the board's claim release rules; a temporary disconnect
releases nothing.

## Status of the build

Step 1, the schema and the owner's part at the hub: built (2026-11-28). Schema (Amendment A's messages and fields, fixtures,
`agora-protocol/tests/task_pickup.rs`; the capability constant `BOARD_TAKE_NEXT_V1`, offered by nobody yet; error codes
`task.pickup_off` and `task.pickup_stale`). `crates/agora-hub/src/task_pickup.rs`: the owner's settings (lists checked, sorted, at most
32 entries of 1 to 64 `[A-Za-z0-9._-]`), read with `active` and `inactive_reason`. `Hub::task_pickup_eligible_set`: the mark on an open
task of a Review-enabled board as the hub's `TaskPickupEligible` (nothing recorded when already so; a claimed or done task cannot be
marked; a mark stays through a claim until cleared), kept by the view and carried by `TaskSnapshot.pickup_eligible` or sent after the
snapshot; `TaskPickedUp` is folded the same way (no rule writes one yet). The three commands are the unscoped owner's (`dispatch.rs`);
the facts are owner-only (`scope.rs`) and a node's are refused (`projection.rs`). Tests: `task_pickup.rs`'s own,
`the_owner_marks_an_open_task_for_pickup_and_the_settings_say_why_it_is_off`, `channel_notice_commands_are_the_owners_at_the_hub`.


Step 2, the hub's `task_take_next`: built (2026-11-28). `Hub::take_next` (`hub.rs`) under the hub's lock, in the order
`docs/specs/mesh-tools.md` ("Taking the next task") gives: capability, the node's limit, the run bound to the link's node
and the project (`unknown_run` otherwise, also for another node naming it), then the run's and the project's limits (`task_pickup::Rate`:
made-up run ids cost their node alone, never the project's share; agora-ac's read of 70393b1b), the owner's settings checked at each take (`active`, not only
`enabled`), the allow-lists with the hub's recorded kind, `skips_permissions` from the hub's record too, the board's join; then the
sequence (`task_pickup::Outcome` rows keyed by the project, the link's node and the run; dropped once the run is not running, at a
take in the project and by a sweep once a minute), a claim
the run holds, and the first marked open task in creation order the board lets it claim. The claim, every event the board calls made,
`TaskPickedUp` and the outcome row are stored in one transaction; on a failure the board is put back. Tests: `task_pickup.rs`'s
(`requests_are_limited_per_run_node_and_project_each_second`, `outcome_rows_are_the_runs_own_and_go_with_the_run`) and
`an_idle_run_takes_the_next_marked_task_once_and_only_for_itself`. 

Step 3, the node's side: built (2026-11-28). The policy key is `mesh.take_next = true` in the node's policy file (the file's own
syntax for the amendment's `[mesh] take_next`; off by default, only the word `true`). `MeshServer`'s `TakeNext`: offered to the hub
(`uplink_client`) only with the policy on, `mesh.remote` on and a gate that names runs (`daemon::RunGate`: the supervisor's running
view and run id, and `skips_permissions_now`, so a mode switched to acting without asking counts); listed to agents only once the
hub's link agreed. `take_next_reply` answers the tool itself (never a broker): the node's reasons, the link's, then the hub's answer
with the run and sequence filled by the node (`RemoteLink::take_next`, a `BoardCall` with fields 6 and 7); each run's next sequence in
memory, 0 first for a run without a count, on only past a committed answer, the committed one after a stale answer; `not_shared`
from a hub that says so. Tests: `take_next_is_the_hubs_with_the_nodes_run_and_sequence`, the policy test. 

Step 4, the owner's window: built (2026-11-28). `apps/web/src/net/pickup.ts` (read, save with the lists cut to what the hub takes,
mark) and `components/BoardPickup.tsx`: on a Review-enabled project's board card, "Idle agents take marked tasks" with the machines
that have the project and the kinds seen in it (the saved ones always offered), each change saved and read back, why it is paused;
on an open card "Offer to an idle agent" / "Stop offering", which only says the mark without a live link. `net/view.ts` folds
`TaskPickupEligible` and `TaskPickedUp` (the hub's only; kept through a task's moves; from the snapshot or after it), and a card says
an idle agent took it only while its claim (or the claim it was finished under) is the pickup's: a pickup released and claimed
again by hand says nothing (agora-ac's read of ffd6e244). Only the owner's window gets the controls. Tests: `net/pickup.test.tsx`. The end-to-end run (2026-11-28,
`crates/agora/tests/hub_dial.rs`, `two_idle_agents_on_two_machines_race_for_one_marked_task_and_one_takes_it`): two machines allowed by
policy and one not, a real hub, two agents racing for one marked task (one `claimed`, one `empty`), the pickup and outcome row the
winner's, `existing` on asking again, the third machine never listing it. It found two faults the unit tests could not: the hub did not
announce `board.take_next.v1` (now in `hubcli`'s list), and a recovery (sequence 0) followed by the node's request in the same second
was refused by the per-run limit (a recovery now counts only toward the node's). Left: a lost reply after commit across a hub restart
and a reattachment with a real supervisor, end to end (each covered by unit tests today).

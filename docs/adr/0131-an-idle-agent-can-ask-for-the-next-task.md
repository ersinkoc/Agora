# 0131. An idle agent can ask for the next task
Status: Proposed
Date: 2026-11-27

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

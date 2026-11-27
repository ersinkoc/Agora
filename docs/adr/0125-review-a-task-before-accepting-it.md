# 0125. Review a task before accepting it
Status: Proposed
Date: 2026-11-27

## Context

ADR-0095 decisions 2 and 6 describe the Review column and reserve its schema for explicit approval. The existing
`task_complete` call moves a task directly to Done and unblocks its dependencies. The broker stores a completion summary.
`TaskChanged` and `TaskSnapshot` carry the task title, owner and `done` flag, leaving the window unable to distinguish an
agent's report from a person's acceptance or read the report from the board.

On 2026-11-27, a browser on Blackview with the web build from `1afbea5f` gave a task created on one 0.3.42 node to a fake
adapter on a second 0.3.42 node through their shared board. The target saw the claim, reported completion through
`task_complete`, and the browser moved the card to Done. A dependent task was refused while its prerequisite was still
claimed. Both node processes ran on Linux in that test.

## Proposal

1. Enable Review per shared project at its hub, with the owner's opt-in. Support node-local boards in a later step that
   covers their persistence and handover to the hub. On a Review-enabled board, the task's current owner submits a report
   through `task_complete`. The task enters Review, keeps its reporting agent as attribution and leaves dependencies
   blocked. It stays under review if that agent stops or its node disconnects. A person can accept it into Done or return
   it to Backlog, where it can be assigned again. The report remains readable until a newer report supersedes it. Tasks
   already marked Done before opt-in keep that status and do not gain a reviewer.
2. Append an optional `TaskReview` to `TaskSnapshot`. It contains a hub-assigned report revision, reporting agent and
   report time, a state (pending, accepted or returned), and the authenticated reviewer and decision time when decided.
   Append hub-only `TaskReviewChanged` to the event envelope with the project, task and the same metadata. Announce
   enabled project IDs in a new snapshot field and live changes through a hub-only `TaskReviewEnabled` event. Use
   `TaskChanged` for the existing three-column projection. While Review is pending it keeps `done = false` and the prior
   owner, so older windows show Doing. Acceptance emits `done = true`; returning clears the owner and keeps
   `done = false`. Current clients render the Review column from the new state.
3. Add hub-handled `TaskReviewDecide` with the project, task, expected report revision and an accept-or-return
   decision. In the first step only an authenticated hub owner with access to the board can decide. The hub records
   the reviewer from the session and the time from its clock. It answers an identical retry of a committed decision
   without creating another event. A conflicting or stale decision is refused with `task.review_stale`. Agents and
   nodes cannot emit review or enablement events, and the existing agent tools cannot move a task out of Review.
   Review changes board state only. It sends no prompt and changes no node policy or permission.
4. Store the report with the board and expose it on demand through `TaskReviewRead` (project, task, report revision)
   and its `TaskReviewReport` result (summary and references). Read access follows the board's project scope. Keep
   the existing mesh size and reference-count limits and render the report as untrusted text. Avoid putting reports
   into every window's snapshot. Persist the board, its report or decision, and the corresponding events in one
   transaction. The hub keeps pending Review on its own board through reconnect and replay. The stored format must
   prevent an older hub opening a board that it would treat as an ordinary claim.
5. The first step covers the Review transition and reading the report. Subtasks and derived workspace links remain
   under ADR-0095. Completion notifications and automatic assignment are separate decisions. They may use this
   report transition once approved.

## Verification

Exercise a real shared-board dispatch and completion with two nodes and fake agents. An enabled board shows the report
in Review through reconnect and restart and keeps a dependent task blocked until acceptance. Returning makes the task
claimable again. Test a stale decision after a second report, identical decision retries, concurrent accept and return,
a failing store transaction, and read and decision requests from a member without access or a node. Check the
three-column view against an older client and add new golden fixtures for the appended messages and fields.

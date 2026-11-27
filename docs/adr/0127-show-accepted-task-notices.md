# 0127. Show accepted task notices
Status: Accepted
Date: 2026-11-27
Accepted: 2026-11-27 (maintainer explicitly approved ADR-0127 in chat).

## Context

ADR-0125 distinguishes an agent's report from a person's acceptance. An owner with several shared projects can
accept work in one window and miss the outcome in another. Opening each board to look at Done also gives tasks
finished before Review the same presentation as accepted reports. The hub already publishes the accepted report's
revision and decision time and includes that metadata when a window opens or reconnects.

ADR-0095 and M8.2 also call for telling the dispatcher, the parent agent and the project channel. The
`TaskDispatched.by_user` event field is supplied by the node. The hub's audit row records who asked, and the
node's event records that it dispatched the task. A later design must correlate those facts before using them
to choose a recipient. Agent inbox delivery also needs a rule for an agent that disconnected or stopped.

## Decision

1. Start with Review-enabled shared boards. When acceptance moves a task into Done, show it in the hub owner's
   web window under **Recently accepted** on **Shared tasks and files**. Show the task title, project and acceptance
   time, with an action to open the board. Windows looking at the same hub update when the acceptance arrives.
2. Derive the list from the current task rows and their hub-authored `TaskReview` metadata. Require both Done and an
   accepted report. List each current project and task once, with its accepted report revision. Show at most 50,
   ordered by decision time newest first, with project and task IDs as deterministic ties. Apply the existing
   shared-board visibility before building the list. Render titles as plain text and read report details through the
   existing report action.
3. Use ADR-0125's commit, snapshot and replay behavior. The window has the same list after reloading, and a retry of
   an already committed decision adds no row. Pending or returned reports and tasks completed before Review add no
   row. This step adds no wire fields, notification store, per-window read receipts or delivery to agents.
4. Address notices to an individual dispatcher, parent agent and project channel in a later decision. Specify the
   authoritative dispatch and recipient identities, behavior across reconnect or stop, and delivery in `inbox_wait`
   before enabling any automated agent-facing notices.

## Verification

- In two windows signed in as the owner, accept one report and return another. Confirm one notice, with the accepted
  task's project and decision time, and verify that opening the notice opens the correct board.
- Retry the acceptance and reload or reconnect a window. Check that a full snapshot's overflow metadata yields the
  same list and that an older snapshot with Done tasks but no Review metadata yields no notices.
- Verify ordering, the 50-row bound, owner versus member and viewer access, and plain-text titles. Check both themes
  and keyboard access to the board action.

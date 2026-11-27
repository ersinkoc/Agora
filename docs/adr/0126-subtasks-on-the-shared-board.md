# 0126. Subtasks on the shared board
Status: Proposed
Date: 2026-11-27

## Context

ADR-0095 calls for task families and sets their limits. ADR-0125 adds a person's acceptance to the shared board.
An agent can already create related tasks with `depends_on`. That relationship controls whether a task can be
claimed. A person also needs to see which tasks contribute to one piece of work, including work that can run
in parallel.

## Decision

1. Start with shared boards whose owner enabled Review. Each new subtask has one parent on the same board.
   Root tasks remain valid. A tree has at most three levels, including the root, and a parent has at most
   50 direct children. The existing board size and task creation limits also apply. Choose the parent when
   creating the child and keep the link for the life of the task. The hub validates and inserts the task and
   link together so concurrent creation cannot exceed the limits.
2. Add the agent tool `task_create_subtask { parent_task_id, title, detail?, depends_on? }` through the existing
   `BoardCall` path. Keep `task_create` for root tasks. A separate tool lets an older node refuse an unknown
   call without creating an unlinked task. The node offers the tool only when its mesh policy
   allows remote calls and the hub link negotiates `board.subtasks.v1`. The call has no local fallback. The hub
   checks the negotiated capability, the calling agent and project under the ordinary board rules, and Review
   enablement. It answers `task.not_found` for a missing parent or one outside that board. Use `board.subtasks_unavailable`
   when the board does not support subtasks and `task.subtask_limit` for the tree limits. A refusal leaves the
   board untouched. Include `parent_task_id` on subtask rows in `task_list`.
3. Store and restore the link with the child. Add optional `parent_task_id` to `TaskSnapshot` and a hub-only
   `TaskParentLinked { project_id, task_id, parent_task_id }` event for live updates and replay. Commit the child
   and link with their events in the same transaction. On a full snapshot, keep the link with its task or send
   it through the positioned replay path within the existing frame budget. Preserve links when a board's task
   IDs are remapped. The persisted format must prevent an older hub from opening a board it cannot restore
   with its links intact. Older windows continue showing each task in its usual column.
4. Each card stays in the column for its own status. The web window shows a child's parent and shows the parent
   a count of accepted direct children out of all its direct children. Pending and returned reports do not count
   as accepted. `depends_on` still governs claiming. A person's decision on a report still governs whether each
   task reaches Done. The hierarchy does not assign tasks, submit reports, or accept them on anyone's behalf.

## Verification

- Create tasks at all three levels and fill a parent's 50 child slots, including concurrent attempts at the
  last slot. Refuse a fourth level and a parent from another project. Prove every refusal creates no task.
- Exercise an older node and hub, a node-local board, a shared board without Review, and an older window.
  Verify requests for unsupported subtask creation leave both shared and local boards untouched.
- Reopen the hub from disk and reconnect the window at a replay position. Check that links and progress
  survive restart, snapshots at the frame limit, and any task-ID remapping. Append fixtures for the new
  fields and event and preserve all published fixtures.
- In a browser with two nodes and fake agents, give two subtasks to different agents. Accept one report,
  return the other, and check the parent count and each task's own column. Exercise the two themes and
  keyboard navigation.

Node-local subtasks and Review, derived workspace links, notifications, and automatic assignment follow
separately under the roadmap.

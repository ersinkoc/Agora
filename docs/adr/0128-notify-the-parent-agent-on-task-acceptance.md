# 0128. Notify the parent agent on task acceptance
Status: Proposed
Date: 2026-11-27

## Context

ADR-0125 commits a person's acceptance on a shared board. ADR-0127 shows it in the owner's web window. A parent
coordinating a helper on another machine still has to poll the board to learn whether the helper's report was accepted.
An agent can use `inbox_wait` when it was asked to stay reachable. ADR-0109 and ADR-0124 provide a run-bound inbox
delivery model that survives reconnection and holder reattachment.

The displayed helper tree uses agent IDs, which a new run can reuse. ADR-0119's helper broker also matches the result
of a start carried by the hub with the target machine's start event. That match can attest a particular helper run and
its direct parent run.

## Decision

1. Start with cross-machine helpers on opt-in, Review-enabled shared boards. The hub operator enables parent notices per
   project. The hub queues a notice when the owner accepts a report by an eligible helper. Pending and returned reports,
   historical acceptances, same-machine helpers and ordinary tasks create no notice.
2. Record the qualified helper and direct parent IDs, their run IDs and their project when the authenticated ADR-0119
   start result and start event match. Record only relations verified from that exchange. A display-tree parent field
   by itself does not establish eligibility. On successful report submission, bind the report revision to the verified
   parent run in the same transaction as the report. Accepting the report uses that binding. A re-report binds its own
   revision. A missing binding, project mismatch or definitely ended parent run produces no notice.
3. Store the delivery with the acceptance and its events in one transaction. Use one stable delivery ID for the committed
   task and report revision. The hub may acknowledge the acceptance before delivery. An offline parent can receive the
   notice on reconnect within 24 hours if the same run and project are still eligible. Confirmed stop, replacement,
   policy refusal or expiry ends delivery. Lost receipts cause bounded retries with backoff. A full queue skips the new
   notice and records the reason in hub diagnostics. Delivery results do not reopen an accepted task.
4. Add the link capability `board.task_acceptance_notice.v1`, a hub-to-node `DeliverTaskAcceptanceNotice` command and a
   `TaskAcceptanceNoticeReceipt` result. The command carries an opaque delivery ID, recipient agent and run IDs,
   project ID, task ID, report revision, acceptance time and delivery deadline. The receipt repeats the delivery and
   recipient run IDs. Require agreement on that connection and check node mesh policy, recipient run and project before
   recording the inbox entry. A peer without the capability receives no notice.
5. Store one durable inbox entry and receipt per delivery ID on the node before responding. Keep them through the retry
   window, including holder reattachment of the same run. Use the existing inbox cursor and add kind `task_accepted`
   with project, task, revision and acceptance time as structured data. `inbox_wait` can return it through the ordinary
   inbox path. The entry carries no report text, paths, references, prompt or permission, and wakes no agent outside an
   active inbox wait. The agent's local tools continue during a hub outage.
6. Bound notices separately from handoffs: up to 1,024 hub deliveries and up to 1,024 node entries with receipts, with a
   32 MiB encoded size cap on each side and a 24-hour deadline. Retain existing handoff capacity and the web notice
   behavior. Notifications to the dispatcher and project channel have their own recipient and visibility decisions.

## Verification

- Start a fake helper on a second machine from its parent. Submit and accept the helper's task, then read the matching
  notice with `inbox_wait`. Exercise Return followed by a new report and repeated acceptance of the accepted revision.
- Restart the hub after commit and before sending. Lose a receipt after the node persists the entry. Reattach the parent
  holder and verify one inbox entry with its original cursor. Confirm fresh runs and replacement agent IDs never get it.
- Cover both event/result orders for the helper start, forged parent fields, different projects, expiry, a full queue,
  a refusing store, a node that disables remote mesh and a peer without the capability. Pin additive wire fixtures.

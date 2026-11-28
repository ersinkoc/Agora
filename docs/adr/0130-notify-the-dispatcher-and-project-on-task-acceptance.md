# 0130. Notify the dispatcher and project on task acceptance
Status: Accepted
Date: 2026-11-27
Accepted: 2026-11-27 (maintainer approval in chat).

## Context

ADR-0125 commits a person's acceptance of a report on a shared board. ADR-0127 shows it to the hub owner.
ADR-0128 delivers it to a verified cross-machine helper's parent run. M8.2 also calls for telling the
person who gave the task to the agent and the project's channel. The hub authenticates the person who
makes a dispatch request. A node's `TaskDispatched.by_user` on its own cannot establish that person's
identity or that dispatch still accounts for the claim that produced the report.

## Decision

1. Offer two independent owner settings per Review-enabled shared project, both off by default.
   One enables notices to the authenticated person who dispatched the accepted work. The other enables
   notices to the project's designated channel. The hub checks current board access for both destinations
   and current channel access for readers of that channel. Channel notices carry the task, accepted
   revision and time, without the dispatcher or reviewer's identity. The dispatcher can open the
   report through the existing board action if their current access permits it. Reports, paths and
   references stay behind that action.
2. When a capable node processes a shared-board dispatch, carry a hub-issued opaque dispatch reference
   on the routed request and through the authenticated board claim. Bind the reference to the
   authenticated person, project, task, target node, target run and one attempted dispatch. The hub
   consumes the reference with the successful claim and stores the resulting claim identity. Record
   verified dispatch provenance only after a matching successful result from that route. If the report
   arrives first, its claim binding may be resolved when that result arrives. Acceptance only uses
   provenance verified at commit time. A missing result can omit the personal notice. A node event is
   never a substitute for the matching claim and result.
3. Bind each report to the claim that produced it. An ordinary claim, handoff to another run, return to
   Backlog or a replacement run cannot inherit a person's dispatch provenance. Reattaching the same
   run without changing the claim preserves it. Legacy dispatches without the new reference continue
   through the existing path and can produce channel notices where enabled. They carry no verified
   personal recipient.
4. Commit the notice with the accepted revision and its board events. Use one stable ID per project,
   task, report revision and destination. An identical acceptance retry never creates a second notice.
   Keep at most 50 personal notices per recipient and 50 channel notices per project, with a hub-wide
   limit of 10,000 rows and a 30-day expiry. Eviction and expiry never modify the board. A person who
   signs back in can read retained notices only while they still have access. Read access is checked
   again for each page and after a change in role or grants.
5. Deliver channel notices as hub-authored structured items in the project's designated channel.
   Show them in the web channel and expose a bounded, capability-gated read through the authenticated
   project board for eligible agents. Both paths apply the same channel access check. The owner must
   select an existing channel. If that channel disappears, the hub pauses channel notices until the
   owner selects one again. The notices do not enqueue inbox messages, wake a run or submit input.
   Parent inbox delivery remains governed by ADR-0128.
6. Add optional dispatch and claim metadata, a capability for correlating the reference across the
   hub and node, and scoped notice settings, read and change messages for clients. A change can tell
   an entitled client to refresh its bounded page. An older client can ignore notices and still receive
   the normal board updates. These additions require separate schema approval.

## Alternatives

- Use the dispatcher named by the node event. That makes a routing observation an identity assertion
  and cannot distinguish a reassigned task from the original dispatch.
- Fan out a normal message into all inboxes. That couples an acceptance to the size of the project
  and can wake agents that did not request the outcome.
- Show all outcomes only in the owner window. That leaves the dispatcher and channel without a
  scoped, reconnectable record.

## Consequences

The hub owns recipient identity, scope and deduplication. The node's dispatch path gains one correlation
reference. Retention bounds the additional state. A successful review remains visible on the board even
when a notice expires or a legacy dispatch has no verified dispatcher.

## Verification

Use two nodes with fake agents, an owner, a member who can dispatch and a second project. Exercise a
forged dispatcher, a mismatched dispatch result, both report/result orders, a handoff and reattachment,
identical retries and a crash around acceptance. Check that a disconnected dispatcher can recover the
notice after signing in, that a revoked member loses read access, and that an older node and window
keep the existing board behavior. Test expiry and all three row limits. Append protocol fixtures.

## Amendment A (2026-11-28, maintainer approval in chat): existing channels and where notices live

- **An existing channel** is a `#name` (`#` plus 1 to 32 of `[a-z0-9_-]`) that has had at least one message in the project; the settings
  form offers those from the hub's recent Team chat. Channels have no lifecycle today (nothing deletes one), so decision 5's pause when
  the channel disappears applies once channels get one.
- **Notices live in a hub table, not the event log** (agora-ac's review of the shape): decision 4's row limits, expiry, eviction and
  access checked at each read need rows that can really be removed. The event stream carries only a content-free, project-scoped hint,
  `ProjectNoticesChanged { project_id }` (hub only, arm 98), after which an entitled window reads `ProjectNoticesPage` (commands 134 to
  136, results 47 and 48). Channel access is project access today.
- Settings are hub-side state, unsigned (they widen nothing on a node); while Review is off or the project is not shared they are kept
  and no notice is written, and the settings read says why.

## Status of the build

Step 1, channel notices at the hub: built (2026-11-28). Schema (Amendment A's messages, fixtures). `crates/agora-hub/src/project_notice.rs`:
the owner's settings (`#` and 1 to 32 of `[a-z0-9_-]`, kept while Review is off, with `active` and `inactive_reason` in the read), the
notice kept in the acceptance's own transaction (`Hub::task_review_decide`, a settings row keyed by the stable notice id, so a retry
adds none) with the hint `ProjectNoticesChanged`, pruning after it (30 days, 50 per project channel, 10,000 on the hub; never the board),
and the page (oldest first after the cursor, at most 50, expired ones left out). The three commands are the unscoped owner's in this
step (`dispatch.rs`), and the hint reaches unscoped windows only. Tests: `project_notice.rs`'s own,
`an_accepted_report_leaves_one_channel_notice_and_a_hint`, `channel_notice_commands_are_the_owners_at_the_hub`. Left: the window (the
settings form offering the project's channels, notices as rows in Team chat), members' and agents' reads with access checked per page,
and the dispatcher half (decisions 2 and 3).

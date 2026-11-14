# 0073. Forgetting a machine the hub no longer needs to show
Status: Proposed
Date: 2026-11-14
Origin: module M1 (connectivity and visibility). The maintainer's hub lists `linux`, a test node from 2026-11-12 (version 0.3.2). No
process for it exists, and it will never come back. `mac2` is an older second node on the same Mac. Neither can be removed: the hub's
record is append-only, and nothing in the protocol removes a machine.

## Context

- The hub learns about a machine from events (ADR-0070: `NodeLink` for presence, `NodeOnline` for a process start) and keeps it in its
  view for good: the Machines page, the start window's machine list, the TUI tree and `workspace_status` all show it.
- After the presence fix (#512), a stale machine shows offline with "last seen". That is correct, but it is still clutter, and the start
  window still offers it.
- ADR-0053 solved the same problem for closed agents: `agent.forget` adds an `AgentForgotten` event, the live view drops the agent, and
  history stays. Machines need the same, plus one more thing: a forgotten machine that still holds a valid certificate can come back.
- The hub already records every enrolled certificate by serial and can revoke a node's certificates (`agora-hub::enroll::Enroll::revoke`),
  but nothing calls it. So a lost or retired laptop keeps a certificate the hub would accept.

A new client command and a new event are schema changes, so this needs an ADR (AGENTS.md).

## Decision

1. **Command `NodeForget { node_id, refuse_return: bool }`**, answered by the hub itself (no node is involved, as with `AgentForget`).
   - It is **refused with `node.online` while the machine is connected**. Forget only what is offline, so a mistake cannot hide a
     machine that is working right now.
   - It is refused with the existing `node.unknown` for a machine the hub never saw.
   - **Owner only.** A member or a viewer gets `auth.forbidden` (ADR-0049): machines belong to the owner, not to a project.
   - It is recorded in the audit trail: who, which machine, and whether return was refused.
2. **Event `NodeForgotten { node_id, refused_return: bool }`** (hub-produced, new event number, golden fixture). The projection drops
   everything it keyed under that node: the machine, its agents (running or closed), its terminals, its checkouts and registered
   folders. **History is not rewritten.** Earlier events stay in the store, and usage and cost totals still count that machine's work
   (as with ADR-0053).
3. **Forgetting is not revoking, unless the owner asks.**
   - With `refuse_return = false` (the default), a machine that dials in again with its certificate **appears again**. Its link opens
     (`NodeLink up`), and its resync (ADR-0054/0055) states the agents it still runs. Closed agents from before stay forgotten.
   - With `refuse_return = true`, the hub also revokes every certificate enrolled under that node id (the existing `Enroll::revoke`).
     The machine's next connection is refused right after the handshake with `auth.device_revoked`, as for any revoked certificate (`server.rs` checks `Enrollment::is_active` on every node connection). To join again, it needs a new
     enrollment code (`agora hub code` and `agora node enroll`), which issues a new certificate.
   - Nothing reaches the machine itself either way. Its agents keep running there (nodes are autonomous). Its data folder, worktrees and
     ledger are untouched.
4. **The shared board and the mesh are unchanged.**
   - Tasks claimed by an agent of a forgotten machine stay claimed. They show the agent as unknown, and the owner can release them with
     the board's existing actions.
   - Leases of its agents end by their own expiry (ADR-0055 rules). Nothing is released early, because the machine may still be
     working offline.
   - Messages to its agents get `mesh.recipient_unknown`, as for any agent the hub does not know.
5. **Capability `node.forget`** on the hub. Clients offer the action only when it is present, and old hubs simply lack it.
6. **Where it appears:**
   - **Window:** on the Machines page, an offline machine's menu has **Forget this machine…**. The dialog says when the machine was last
     seen and how many agents were running at that time ("they may still be running there"). It says that the machine and its closed
     agents disappear from every window, and that the machine comes back if it connects again. One checkbox: **Also refuse it if it
     connects again** (it will need a new join code). The action is not offered for an online machine.
   - **TUI:** the same action on a machine row, with the same question.
   - **CLI:** `agora hub machines` lists the machines with their state and last seen. `agora hub machines forget NAME [--refuse-return]`
     sends the command to the running hub, signing in from the hub folder as `agora tui --hub-dir` does. No store file is edited behind
     the running hub's back.

## Open questions (proposed answers)

- **Should forgetting a machine with running agents need a second confirmation?** Proposed: no. The dialog already states the count, the
  machine is offline, and the agents come back if the machine does.
- **Should refusing return be the default?** Proposed: no. The common case is a retired test node (cleanup). Refusing is a security act,
  and it should be a deliberate choice.
- **Should there be a way to see forgotten machines again?** Proposed: not now. The audit trail ("Activity" page) records each forget.
  Un-forgetting is the machine connecting again, or a new enrollment.

## Consequences

- One command, one event, one capability and one new error code, `node.online` (`node.unknown` already exists), each with golden fixtures.
  `specs/protocol.md`, `specs/error-codes.md` and the threat model are updated with the implementation. The threat model gains a line:
  `refuse_return` is the way to cut off a lost machine's certificate.
- Projection tests: forget drops dependents; a later `NodeLink up` brings the machine back without its closed agents; a refused return is
  rejected with `auth.device_revoked`.
- The web Machines page, the TUI and the CLI get the action. Text lives in `locales/{en,tr}`. Screenshots are taken in both themes.
- Not decided here: deleting a machine's events (rejected: append-only and the audit trail), forgetting machines automatically after N
  days offline, and removing a machine's data from the machine itself (that is `agora down` and the owner's own act there).

## Alternatives considered

- **Hide it in the browser only:** it comes back in every other window, the TUI and `workspace_status`. This is the same reason ADR-0053
  rejected a browser-only clear.
- **Delete the machine's events from the store:** breaks the append-only rule, the audit trail and the usage totals. Rejected.
- **Revoke always:** turns cleanup into a lockout. A machine that is merely retired for now would need a new enrollment. Rejected as the
  default, and kept as the option.
- **Edit the hub's database from the CLI while the hub runs** (as `agora hub devices forget` does for the device list): the hub's view is
  a projection held in memory, so a store edit behind it would not show until a restart, and could race the hub's own writes. Rejected.

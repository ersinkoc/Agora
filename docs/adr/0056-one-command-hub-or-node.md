# 0056. Every machine is a hub and a node; a node can belong to several hubs
Status: Proposed
Date: 2026-11-13

Not accepted. Nothing here may be built until the maintainer accepts it in a message of their own.
The maintainer chose this direction in chat (the "small step", every machine both hub and node); that is a direction, not acceptance.

## Context
Today a person starts a hub (`agora hub run --listen … --hub-dir … --name …`), then on each machine enrolls and starts a node.
Plain `agora` prints usage, and nothing keeps either running after a reboot. Wanted: whichever machine you sit at, you see and
manage your own agents and every connected machine's, without choosing a role first.

## Decision (proposed)
1. **`agora` with no arguments starts the machine as both.** One process runs this machine's hub and this machine's node
   (the node is enrolled at its own hub). Data under the user's Agora folder; listen address and names come from a small config
   file written on first run. One machine alone already works.
2. **A node can belong to several hubs at once.** `agora join HUB --code CODE` enrolls this machine's node at another hub too
   (signed enrollment as `node enroll`, ADR-0020). The machine's own hub keeps running. Its node serves both hubs, and the
   agents it runs stay the same agents whichever hub's window shows them (nodes are autonomous; no agent depends on a hub).
3. **A window connects to any hub the person has signed in to** and lists them side by side. Opening a machine's window shows
   that machine's hub first, with every machine joined to it, and the other hubs one click away.
4. **Each hub stays the authority for what it owns.** Boards, leases, approvals and the audit trail belong to the hub a project
   or agent was started through; there is no replication and no peer-to-peer merge. A node that is in two hubs records which
   hub each agent was started through.
5. **No silent joining.** A join needs a code the other hub's owner issued; discovery on the LAN may later only fill in the
   address. A hub never widens the node's policy (ADR-0020); the node verifies each hub's approvals separately.
6. **A service unit** (systemd user service, launchd agent, Windows task) keeps it running across reboots, installed by
   `agora service install`. Platform code stays in `agora-platform`.

## Open questions for the maintainer
- Default listen address: loopback only, or the LAN? A LAN default exposes a hub nobody asked for.
- May one agent be shown in two hubs, or is it shown only in the hub it was started through? (Proposed: only that one.)
- How many hubs may one node serve, and what does the node do if two hubs ask it for the same folder? (Proposed: the node's
  own policy decides, per hub, and a second hub gets a refusal for an agent folder already in use.)
- Does the desktop app start the machine itself, or only the command line?

## Rejected for now
Full peer-to-peer (every machine a replicated master): needs a merged event order, approval and audit log and a conflict rule
for file leases. Larger than this decision; it can be a later ADR on top of this one.

## Consequences
Changes the command line and process model (one process holding a hub and a node; the node's uplink becomes several), adds a
service installer and a multi-hub window, so it needs this ADR accepted first. A node's `NodeOnline`/resync (ADR-0054/0055)
runs per hub link. No protocol message change is expected; enrollment already exists.

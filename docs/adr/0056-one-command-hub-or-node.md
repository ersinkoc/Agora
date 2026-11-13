# 0056. One command: every machine starts as a hub and becomes a node when it joins another
Status: Proposed
Date: 2026-11-13

Not accepted. Nothing here may be built until the maintainer accepts it in a message of their own.

## Context
Today a person starts a hub (`agora hub run --listen … --hub-dir … --name …`), then on each machine enrolls and starts a node
(`agora node enroll …`, `agora node run …`). Plain `agora` prints usage. Starting a machine takes two roles and several
flags, and nothing keeps either running after a reboot. The maintainer's idea: every machine starts the same way, as its
own hub, and when it is pointed at another hub it becomes a node of that hub instead.

## Decision (proposed)
1. **`agora` with no arguments starts the machine** using defaults (data under the user's Agora folder, listen address and
   names from a small config file written on first run). It is a hub with a node for this machine inside it, so a single
   machine already works alone.
2. **`agora join HUB --code CODE`** (or the same choice in the window) enrolls this machine at another hub, exactly as
   `node enroll` does now (ADR-0020 signed enrollment). After joining, this machine's own hub stops serving and its node
   connects to the other hub. Its own hub data stays on disk, untouched, so `agora leave` brings the local hub back.
3. **No silent joining.** Nothing connects to another hub by discovery alone. A join needs a code the other hub's owner
   issued, and the hub never widens the node's policy (ADR-0020). Listing hubs found on the LAN may be added later, but it
   only fills in the address; the code stays required.
4. **Agents keep running across a role change** where possible (nodes are autonomous). Joining must not end running agents;
   if that cannot be guaranteed, it asks first and says which agents would end.
5. **A service unit** (systemd user service, launchd agent, Windows task) keeps the command running across reboots, installed
   by `agora service install`. Platform code stays in `agora-platform`.

## Alternative: every machine is a peer (no single hub)
The maintainer also asked whether every machine could act as a master: whichever machine you are on, it shows its own agents and
every other connected machine's, and can manage all of them.
- **Fits the vision** ("one window shows everything") and removes the single point of failure: any machine can be the window.
- **Costs:** the hub is today the one place that holds the event order, approvals, audit trail, boards and leases (ADR-0039,
  ADR-0040, ADR-0053). With several masters each of these needs a replicated or merged source of truth, and file leases and
  approval signing need a conflict rule. The audit trail would be several local logs unless they are merged.
- **Smaller step toward it:** keep one hub per group, but let any machine's window connect to any hub it has a device
  enrollment for, and let a node enroll at several hubs at once (a node already keeps agents running without a hub).
  Then "which machine am I on" never matters for viewing, and only the hub that owns a project decides its boards.
Open question: full peer-to-peer (large, needs its own design) or the multi-hub window plus one enrolled node per hub (small)?

## Open questions for the maintainer
- Default listen address: loopback only, or the LAN? A LAN default exposes a hub nobody asked for.
- Should a machine that joined another hub keep a local read-only hub, or stop it completely?
- What happens to the local hub's projects, approvals and audit history when it joins (kept aside, merged, dropped)?
- Does the desktop app start the machine itself, or only the command line?

## Consequences
Changes the command line and process model (how one process holds both a hub and a node; crate boundaries between `agora`,
`agora-hub` and `agora-node`) and adds a service installer, so it needs this ADR accepted first. No protocol message change
is expected; enrollment already exists.

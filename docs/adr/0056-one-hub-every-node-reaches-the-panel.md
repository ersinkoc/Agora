# 0056. One hub; every node machine can reach the hub's panel
Status: Accepted
Date: 2026-11-13
Accepted: 2026-11-13 (maintainer's explicit approval in chat: "0056'yı onaylıyorum"). The maintainer did not answer the open questions; the proposed answers below stand.

The maintainer's latest direction in chat: the hub stays single, and every machine that runs a node can open the hub's panel,
even when the hub is on another machine. This replaces the two earlier drafts of this ADR (hub and node on every machine, a node in
several hubs; and full peer-to-peer), which are not pursued.

## Context
To see everything today you open the hub's address in a browser, which means knowing the hub's address and certificate and having
a signed-in device. A person sitting at a node machine has a node but no panel there.

## Decision
1. **The hub stays single.** Boards, leases, approvals and the audit trail stay where they are (ADR-0039, ADR-0040, ADR-0053).
   No replication and no second master.
2. **Every node machine can open the hub's panel locally.** The node already holds the hub's address, its CA and this machine's
   enrollment. A command (`agora panel`, also offered by the desktop app) opens the hub's web app for the person at this machine,
   either by pointing the browser at the hub with the right trust set up, or through a loopback-only forwarder in the node.
   The panel is the same web app; only the way to reach it is added.
3. **The person still signs in.** Reaching the hub from a node machine is not a login. The hub's sign-in code and device approval
   (ADR-0020) apply as they do from any other browser; the node does not hold a session on the person's behalf. A node machine
   used only by agents has no panel running unless asked.
4. **No new listening port by default.** The forwarder, if used, binds to loopback only and runs only while `agora panel` runs.
5. **Without a hub connection the panel cannot work.** The node keeps agents running (autonomy), and `agora agent list` keeps
   showing local agents through the node socket.

## Open questions (proposed answers stand)
- Browser pointed straight at the hub (simple, needs the hub reachable and its CA trusted on this machine), or a loopback
  forwarder in the node (works through the node's existing link, one more component to secure)? Proposed: pointing straight at
  the hub, with `agora panel` printing how to trust the hub's CA on this machine (it changes no trust store itself).
- When the hub is itself on a machine you are not at, and unreachable from here, is a forwarder through the node needed?
- Does the desktop app bundle this, or only the command line?

## Consequences
Adds a small command (and possibly a loopback forwarder) and a trust step; no protocol message change and no change to the
hub's authority. A forwarder would be a new surface and needs the threat model (`docs/security/threat-model.md`) updated.

# 0023. Optional direct data path between client and node
Status: Proposed
Date: 2026-11-07
Amends: 0002 (clients never connect to nodes directly)

## Context
With a hub far away (for example a VPS in another region), a client and node on the same laptop or LAN would
send every keystroke and every byte of terminal output through the hub and back. That hairpin breaks the
interaction budgets in ADR-0022.

## Decision (proposed)
- The hub remains the only control plane: authentication, authorization, state, events, approvals.
- For terminal streams and file transfers, the hub may broker a direct connection between a client and a node
  when one is reachable (same machine via local socket, or LAN via the node's direct listener).
- The hub issues a short-lived, scoped ticket; the node verifies it. Traffic on the direct path is
  authenticated and encrypted end to end.
- If the direct path fails or is unavailable, streams fall back to relay through the hub transparently.
- NAT traversal across the internet is out of scope for now.

## Consequences
- Local use is as fast as a native terminal even with a remote hub.
- Nodes may open an optional listener; it is off by default on public interfaces.
- The protocol needs ticket messages and a path-switch procedure that preserves stream ordering.

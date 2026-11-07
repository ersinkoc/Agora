# 0020. Nodes enforce policy; approvals are signed by the user
Status: Accepted
Date: 2026-11-06
Accepted: 2026-11-07

## Context
A compromised hub could otherwise instruct every node to run arbitrary commands. Central control must not
mean a single point of total compromise.

## Decision
- Each node holds a local policy (allowed projects, agent kinds, commands requiring approval). The hub cannot
  widen a node's local policy remotely without a user-signed change.
- Approvals for sensitive actions are signed by the approving user's device key; the node verifies the
  signature before acting. The hub only relays.
- Starting a new agent kind or installing an adapter on a node is a sensitive action.

## Consequences
Approval flows need device keys in every client (including mobile). A hub breach exposes metadata but cannot
execute privileged actions on nodes.

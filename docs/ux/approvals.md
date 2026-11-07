# Approvals queue

- Reachable from the top-bar indicator and as banners in the center panel.
- Each item: agent, project, node, requested action (command, file write, network, install), the exact
  command or diff, risk label from policy, time waiting.
- Actions: Allow once, Allow for this session, Deny, Deny and stop agent. Every decision is signed by the
  device key (ADR-0020).
- Bulk actions only for identical low-risk requests.
- Mobile layout first: approvals are the main mobile use case.
- Expired approvals show as expired; the agent is informed.

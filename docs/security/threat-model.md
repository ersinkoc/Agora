# Threat model

Agora gives one place the power to run code-executing agents on many machines. That makes it a high-value
target. This document lists assets, trust boundaries, threats, and required mitigations. Review it in every
phase exit.

## Assets

| Asset | Where |
| --- | --- |
| Ability to execute code on machines | nodes |
| Source code and uncommitted work | nodes (checkouts, worktrees) |
| Agent and API credentials | nodes only |
| Terminal output (may contain secrets) | nodes; streamed through hub |
| Coordination data, approvals, audit log | hub |
| User and device keys | client devices, nodes |

## Trust boundaries

1. Client device ↔ hub (network)
2. Hub ↔ node (network)
3. Node ↔ agent process (local; the agent is powerful but semi-trusted)
4. Agent ↔ agent via mesh (agent output is untrusted input to another agent)
5. Node ↔ adapter/manifest packages from the registry (third-party code)
6. Agent ↔ content it reads (repos, web pages, issues): prompt-injection source

## Threats and mitigations

| # | Threat | Mitigation |
| --- | --- | --- |
| T1 | Hub compromise → commands to all nodes | Node-local policy; sensitive actions require user-signed approvals verified on the node (ADR-0020) |
| T2 | Stolen client session | Passkeys; short-lived sessions; device-bound keys; approvals require device signature |
| T3 | Rogue node enrollment | One-time enrollment codes, user confirmation in window, mTLS, revocation (ADR-0015) |
| T4 | Cross-agent prompt injection over mesh (agent A tells agent B to exfiltrate or delete) | Messages marked untrusted with provenance; standing instruction to agents; approvals unaffected by mesh content; rate limits |
| T5 | Prompt injection from repository content causing destructive commands | Approval policies per node/project; deny-lists for destructive commands; worktree isolation limits blast radius |
| T6 | Malicious adapter or manifest | Signed packages, pinned publisher keys, user approval to install, private registry option (ADR-0018) |
| T7 | Secret leakage via terminal streams or logs | Secrets never sent to hub; optional E2E terminal encryption; log redaction; scrollback persistence off by default |
| T8 | Agent escapes its worktree | Document as not prevented in v1; optional sandboxing (containers, OS sandbox profiles) as a later phase |
| T9 | Supply chain (our dependencies, our releases) | cargo-deny, minimal deps, reproducible builds, signed releases, signed self-updates (ADR-0019) |
| T10 | Denial of service by runaway agents or mesh loops | Resource limits per node, mesh rate limits, kill switch per project/node |
| T11 | Local privilege issues (socket accessible to other users) | Per-user sockets with owner-only permissions; named pipe security descriptors on Windows |

## Kill switch

Every client exposes "stop all agents" globally, per project, and per node. The node honors it even when
the instruction arrives without hub connectivity (local CLI).

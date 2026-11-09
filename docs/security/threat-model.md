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
| T1 | Hub compromise → commands to all nodes | Node-local policy; sensitive actions require user-signed approvals verified on the node (ADR-0020). **Gap:** approvals today cover what an agent asks to do (`approval_required`); the commands a hub or client routes to a node (`agent.start`, `agent.prompt`, `terminal.input`) run without a signature, so a compromised hub can still start and drive agents. Closing it needs a signed-command field and its own ADR (found by the Codex review of 2026-11-09, `docs/reviews/2026-11-09-codex-security-surfaces.md`) |
| T2 | Stolen client session | Passkeys (WebAuthn, hub stores public keys only; a one-time code remains the first sign-in); 12-hour sessions stored hashed; device-bound keys; approvals require device signature |
| T3 | Rogue node enrollment | One-time enrollment codes, user confirmation in window, mTLS, revocation (ADR-0015); enrolling again under a name revokes the earlier certificate for it (one live identity per node id) and live connections are re-checked every 10 s, so a revoked node, an expired session or "sign out everywhere" ends the connection that is already open |
| T4 | Cross-agent prompt injection over mesh (agent A tells agent B to exfiltrate or delete) | Messages marked untrusted with provenance; standing instruction to agents; approvals unaffected by mesh content; rate limits |
| T5 | Prompt injection from repository content causing destructive commands | Approval policies per node/project; deny-lists for destructive commands; worktree isolation limits blast radius |
| T6 | Malicious adapter or manifest | Signed packages, pinned publisher keys, user approval to install, private registry option (ADR-0018) |
| T7 | Secret leakage via terminal streams or logs | Secrets never sent to hub; optional E2E terminal encryption; log redaction; scrollback persistence off by default |
| T8 | Agent escapes its worktree | Document as not prevented in v1; optional sandboxing (containers, OS sandbox profiles) as a later phase |
| T9 | Supply chain (our dependencies, our releases) | cargo-deny, minimal deps, reproducible builds, signed releases, signed self-updates (ADR-0019) |
| T10 | Denial of service by runaway agents or mesh loops | Resource limits per node, mesh rate limits, kill switch per project/node |
| T11 | Local privilege issues (socket accessible to other users) | Per-user sockets with owner-only permissions; named pipe security descriptors on Windows |
| T12 | A local user on an SSH-only machine reaches the hub's reverse-tunnel port (ADR-0032) | The port is loopback-only; without a valid, unrevoked client certificate the only thing possible is an enrollment attempt, which is single-use, short-lived and rate limited hub-wide; `ssh` runs with BatchMode and strict host-key checking so the hub never trusts a new host silently |
| T13 | The hub's remote install (ADR-0034) puts a malicious binary or enrollment on a machine | Only a release asset is fetched, never an upload or a script; its SHA-256 and a signature pinned by the owner (`release-key`) are checked on the machine before it is installed, and without a pinned key the command stops unless the owner passes `--unsigned`; every value in the remote commands is validated; the code travels over the SSH channel only and the CA on stdin; the new node starts with default policy and no trusted devices; the command is interactive and never triggered by a web action |
| T13 | A node forges messages as another node's agent (ADR-0031) | The hub sets `MeshPost.origin` from the connection's certificate and overwrites the node's value; only the agent id within a node is the node's word; mesh content stays untrusted data and cannot approve anything |

## Kill switch

Every client exposes "stop all agents" globally, per project, and per node. The node honors it even when
the instruction arrives without hub connectivity (local CLI).

## Implementation status: approvals (T1, T4)
`agora-node::approvals` enforces on the node: signature verified against keys the node already trusts, signed message bound to
node id, approval id and verdict (no replay on another node or request, no flipping deny to allow), 128-bit random single-use
ids, expiry, cancellation when an agent stops, and a policy that lists which actions need approval. The signature scheme sits
behind a `Verifier` trait; the scheme is ECDSA P-256 (ADR-0028, `P256Verifier`). Trusted device keys reach the node only through its owner:
`agora node trust DEVICE_ID PUBLIC_KEY_HEX` writes `<data>/trusted-devices` on the machine itself (read at start and again whenever the file changes, so trusting a browser never means restarting the node and stopping its agents; malformed
lines are skipped, never trusted), and no hub or client message can edit it. The web client keeps a non-extractable P-256 key in the browser (IndexedDB); its "This device" page shows the public key and the exact `agora node trust` command, and approvals are signed there (`apps/web/src/net/approval.ts`) and verified on the node. Adding a device by a user-signed message from an
already trusted device (ADR-0020) is still to be built.

### Approvals: node wiring
Adapter approval requests enter the node's queue under a random node-minted id (the adapter's own id is only used to answer it),
are announced as `ApprovalRequested`, and wait. `approval.resolve` verifies the device signature on the node and relays the verdict
to the adapter, recording `ApprovalResolved` with the signer; requests that expire are answered as denied; a stopped or crashed
agent leaves nothing waiting. With no trusted device (the default until the owner runs `agora node trust`) every decision is
refused with `approval.signature_invalid`, so sensitive actions stay denied rather than open. Adapter usage and session reports
are written to the event log.

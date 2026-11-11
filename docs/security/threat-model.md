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
| T1 | Hub compromise → commands to all nodes | Node-local policy; sensitive actions require user-signed approvals verified on the node (ADR-0020). Approvals cover what an agent asks to do (`approval_required`); the commands a hub or client routes to a node are covered by user-signed grants (ADR-0041): a node whose policy lists `signed_commands` (for example `agent.start, terminal.input, files.write`) refuses those commands with `approval.grant_required` unless a grant signed by a trusted device, for that node, scope and time, is live; the hub cannot forge one. **Open until the owner sets the line:** without a `signed_commands` line the node keeps today's behaviour and a compromised hub can still start and drive agents (found by the Codex review of 2026-11-09, `docs/reviews/2026-11-09-codex-security-surfaces.md`); a later ADR may flip the default |
| T2 | Stolen client session | Passkeys (WebAuthn, hub stores public keys only; a one-time code remains the first sign-in); 12-hour sessions stored hashed; device-bound keys; approvals require device signature |
| T3 | Rogue node enrollment | One-time enrollment codes, user confirmation in window, mTLS, revocation (ADR-0015); enrolling again under a name revokes the earlier certificate for it (one live identity per node id) and live connections are re-checked every 10 s, so a revoked node, an expired session or "sign out everywhere" ends the connection that is already open |
| T4 | Cross-agent prompt injection over mesh (agent A tells agent B to exfiltrate or delete) | Messages marked untrusted with provenance; standing instruction to agents; approvals unaffected by mesh content; rate limits |
| T5 | Prompt injection from repository content causing destructive commands | Approval policies per node/project; deny-lists for destructive commands; worktree isolation limits blast radius |
| T6 | Malicious adapter or manifest | Signed packages, pinned publisher keys, user approval to install, private registry option (ADR-0018) |
| T7 | Secret leakage via terminal streams or logs | Secrets never sent to hub; optional E2E terminal encryption; log redaction; scrollback persistence off by default |
| T8 | Agent escapes its worktree | Not prevented by default. Opt-in confinement (ADR-0047): the node's `<data>/policy` line `sandbox = worktree` starts agents (L1 terminals and the adapter process of L2/L3 agents, which carries the agent it hosts) under Linux Landlock or macOS Seatbelt (`sandbox-exec`; writes are denied outside the listed folders and file contents below `/Users`, `/Volumes` and the temp folders unless listed; the system stays readable, file names stay visible; a listed folder that holds a private one, such as `/var`, does not reopen it) so they write only in their worktree, its git folders, `/tmp` and (macOS, Windows) a temp folder of their own (plus `sandbox.write = a, b`; `~/` allowed, relative paths dropped), and read only the system, their program's folder and `sandbox.read = a, b`; `NO_NEW_PRIVS` is set. A machine that cannot enforce it (no Landlock and no `sandbox-exec`, Windows, `worktree-offline` where it has no network rules: Linux and Windows today) answers `agent.sandbox_unavailable` and starts nothing. On macOS `worktree-offline` also cuts the network: only this machine (`localhost`) and the agent's own mesh socket and private temp folder's unix sockets stay reachable, and the agent cannot listen on a TCP port. `localhost` means anything listening on this machine: a local proxy, an SSH tunnel, a DNS forwarder or a hub on the same machine is reachable too, so offline is only as closed as what runs locally. A typo in the value is the strictest level. Reduces what a confused agent can touch; it is not a promise that untrusted code is safe. A manifest's `[sandbox]` requests count only for kinds listed in `sandbox.trust_manifest`. Adapter agents also need their program and state folders listed in `sandbox.read`/`sandbox.write`. Windows (ADR-0050 with Amendment B, off until enabled): a restricted token with a per-agent SID **at Low integrity**, with a Low label on exactly the folders the agent may write, so writes anywhere else are refused whatever their ACL grants; a folder the agent would be given that lets `Users` or `Everyone` write (itself or anything below it) is refused before start (`agent.sandbox_unavailable`, `reason = broad_write_grant`), and so is one whose label the user cannot set (`reason = label_failed`, the default ACL of a data volume); a confined agent's own process and threads can be opened by the user and that agent only, not by another confined agent; reads are weaker than on Linux and macOS: files that grant `Users` or `Everyone` read stay readable, user-private files and another agent's folders are refused; the shared Git folder is read-only apart from `objects/`, `refs/` and `logs/refs/`, so an agent can add objects and move branch refs but not plant a hook or config in a worktree's main checkout; each agent's mesh endpoint admits only that agent's identity, so one confined agent cannot speak on another's connection. **msys programs (`sh`, `bash`, Git Bash) cannot start under it** (they create a kernel object directory a Low process may not create), so a tool that needs them, such as Claude Code's shell tool on Windows, does not work confined. A launch in a selected folder (ADR-0052) is refused under a confined policy when the folder is a filesystem root, the home folder or one that contains it, a secret folder below it, or the node's own folder. Not done yet: Windows adapter commands, network rules on Linux and Windows, manifest-named hosts, manifest-declared paths; a confined agent in a user's own checkout can still write `.git/hooks` and `.git/config` |
| T9 | Supply chain (our dependencies, our releases) | cargo-deny, minimal deps, reproducible builds, signed releases, signed self-updates (ADR-0019) |
| T10 | Denial of service by runaway agents or mesh loops | Resource limits per node, mesh rate limits, kill switch per project/node |
| T11 | Local privilege issues (socket accessible to other users) | Per-user sockets with owner-only permissions; named pipe security descriptors on Windows |
| T12 | A local user on an SSH-only machine reaches the hub's reverse-tunnel port (ADR-0032) | The port is loopback-only; without a valid, unrevoked client certificate the only thing possible is an enrollment attempt, which is single-use, short-lived and rate limited hub-wide; `ssh` runs with BatchMode and strict host-key checking so the hub never trusts a new host silently |
| T13 | The hub's remote install (ADR-0034) puts a malicious binary or enrollment on a machine | Only a release asset is fetched, never an upload or a script; its SHA-256 and a signature pinned by the owner (`release-key`) are checked on the machine before it is installed, and without a pinned key the command stops unless the owner passes `--unsigned` (the project's release public key, as an `allowed_signers` line, is `docs/security/release-key.pub`; copy it to `<hub dir>/release-key`); every value in the remote commands is validated; the code travels over the SSH channel only and the CA on stdin; the new node starts with default policy and no trusted devices; the command is interactive and never triggered by a web action |
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

### Signed grants for routed commands (ADR-0041)
`agora-node::grants` gates the commands a hub routes to the node (the uplink path only; the node's own control socket is never gated).
Policy line in `<data>/policy`: `signed_commands = agent.start, terminal.*, files.write` (exact names or a prefix ending in `*`; absent or
empty means none). A listed command with no live grant is refused with `approval.grant_required` and runs nothing. A grant is made by
`GrantCreate`, signed (ES256, same trusted devices as approvals) over `agora-grant-v1 NUL node_id NUL grant_id NUL scope NUL commands NUL
expires_at_ms`; `scope` is `node`, `agent:<id>` or `project:<id>`; it lasts at most 8 hours; the node keeps grants in memory only (a
restart ends them), refuses a grant id it has seen, a wrong signer, another node's grant and an expired one. `GrantList` and `GrantRevoke`
need no signature. Commands that name a worktree or open a new terminal (`files.write`, `worktree.*`, `terminal.open`) name no agent, so only
a `node` grant covers them. Recommended for a machine that is not a throwaway: `signed_commands = agent.start, terminal.input, files.write` (reading stays free). Typos in the line cannot widen anything: an unknown name simply matches no command. Tests: `crates/agora-node/src/grants.rs` and `crates/agora/tests/hub_dial.rs`.

### Approvals: node wiring
Adapter approval requests enter the node's queue under a random node-minted id (the adapter's own id is only used to answer it),
are announced as `ApprovalRequested`, and wait. `approval.resolve` verifies the device signature on the node and relays the verdict
to the adapter, recording `ApprovalResolved` with the signer; requests that expire are answered as denied; a stopped or crashed
agent leaves nothing waiting. With no trusted device (the default until the owner runs `agora node trust`) every decision is
refused with `approval.signature_invalid`, so sensitive actions stay denied rather than open. Adapter usage and session reports
are written to the event log.

## Team mode (ADR-0049)
A hub may have several people with a role each (`owner`, `member`, `viewer`). The hub refuses a viewer's commands that change anything (`auth.forbidden`) and checks the role on every command, so a demotion or a removal takes effect on an open connection. This is the hub's own limit on its clients and not a security boundary: a compromised hub is still bounded only by the node's policy and signed grants (ADR-0020, ADR-0041). A member or viewer sees only the projects granted to them: the hub cuts their snapshot, forwards only their events, and refuses commands that do not name something in a granted project (`agora_hub::scope`). On the node, `device.<id>.projects = a, b` in `<data>/policy` limits what a trusted device may sign: grants only scoped to those projects (never `node`), and approvals only for agents of those projects; a device the file does not name is limited by nothing (today's behaviour), and a named one with no projects acts in none. The hub cannot change the file. When a node names any limited device and has no `signed_commands` line, it requires a grant for every command that changes something (`agent.start|stop|prompt|set_mode`, `terminal.open|input`, `files.write`, `worktree.create|remove`), so the limits bind those commands; a limited device can only sign project or agent grants, so whole-machine commands (`terminal.open`, `files.write`, `worktree.*`) stay the owner's. An owner who writes a `signed_commands` line decides the list. Without limited devices nothing changes. This binds only what is signed: the commands a node lists in `signed_commands` need a grant, so a member's browser device is limited by the node only for those commands (an owner who wants a member limited on the machine lists the commands there). Commands outside `signed_commands` still rely on the hub's role and project checks. The audit trail itself exists (`agora hub audit export`): every routed command, sign-in and change to the team is a row, kept by database triggers that refuse deletes and edits, so a client command cannot rewrite history; a compromised hub process with database access still can, so the trail is evidence about people, not about the hub.

Found by an independent audit after 0.2.0 and fixed: a passkey added by a member or viewer used to sign in as the owner (passkeys had no person, and a sign-in with one made an owner session). A passkey now belongs to the person whose session added it and signs in as that person (schema 8); removing a person removes their passkeys; the ids `owner` and `hub`, which the hub writes itself, can no longer be a person's (so no audit row can pose as the owner's); and starting an agent in an existing worktree is checked against the person's projects. Known and accepted: a member sees the machine-wide counts in a machine's resource events (agents running, terminals open, memory) for any machine that hosts one of their projects.

## Selected folders and portable identities (ADR-0052)

Directory inspection and launch are always authorized on the node by an exact-request trusted-device signature over
node, operation, path, kind, expected project, metadata/worktree option, changed-identity confirmation, nonce and expiry.
A generic start grant cannot substitute another path. Used nonces persist across restarts; malformed or full replay state
fails closed. See `specs/protocol.md` for the canonical encoding. Initially this new flow is restricted to unrestricted
owner devices; node-scoped policy limits refuse it for member devices. The local owner-only socket keeps its authority.

The node resolves symlinks and re-inspects the canonical path/project immediately before launching. It preserves selected
user-owned folders on stop or failure. MCP files for direct L1 sessions live in separate session folders, accessible to that
agent's sandbox as read-only configuration; fixed-file-only MCP manifests require explicit worktree mode. Opt-in sandbox
policy still applies to the selected cwd. Sharing a folder deliberately shares its files; file leases cannot intercept an
agent's own shell writes, and the UI makes no per-agent attribution of a shared diff.

Repository metadata is untrusted data. It never enrolls a node, grants access, joins another owner's project or changes
device limits. It is a size-limited regular JSON file with a validated UUID. Previously confirmed path/ID bindings are
persisted and cannot silently switch after repository edits; an explicit signed owner confirmation is needed. Hub project
filters group confirmed logical IDs while preserving each machine and path; role/project filters still apply to workspace
snapshots, status events and file queries. Copying this metadata transfers identity, not source files or authorization.

Selected-directory review follow-up (#257): explicit worktree launch resolves the new root and walks the selected
relative directory without following restored symlinks. Missing directories are created only below that root; a
non-directory or symlink is refused before launch. Workspace diffs include an explicit selected-directory pathspec
even for an empty requested path, keeping parent-checkout changes outside the workspace review. Confinement tests
place a selected subdirectory outside the shared system temp folder and prove inside writes succeed while parent
checkout writes fail, in both directory and explicit-worktree modes. Explicit policy write allowances still apply.

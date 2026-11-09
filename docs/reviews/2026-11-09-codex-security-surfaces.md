# Codex cross-review: security surfaces of phases 2 and 3 (2026-11-09)

Reviewer: Codex (`codex exec`, read-only sandbox, codex-cli 0.167.0). Author: Claude Code. Scope: the `files.*` and `git.*`
restrictions in the node, approvals and the trusted-devices file, enrollment and sign-in, and client command routing. The review
was static; its tests could not write temporary files, so the files/git and TLS tests were not run by the reviewer (they were
run by the author).

| # | Severity | Finding | Status |
|---|---|---|---|
| 1 | high | Routed `agent.start`, `agent.prompt`, `terminal.input` need no signed approval, so a compromised hub or stolen session can start and drive agents (ADR-0020, T1) | **Open.** A real design gap: approvals cover agent-requested actions only. Needs a signed-command field in the protocol and an ADR. Recorded in the threat model |
| 2 | high | A client connection stays open after its session expired or was revoked | Fixed: live connections are re-checked every 10 s (`AGORA_RECHECK_MS` for tests); test in `hub/tests/server.rs` |
| 3 | high | Revoking a node does not end its live connection | Fixed the same way (certificate re-checked) |
| 4 | high | A redeemed code could enroll an existing node's name and add a second route for it | Fixed: enrolling under a name revokes the earlier certificates for it; test in `enroll.rs`. A code is still not bound to a name (it would need a migration); with a code an attacker can replace a node, which is now visible because the real one is cut off |
| 5 | high | Removing a trusted device could be missed when the file's mtime did not change | Fixed: the file content is compared; test keeps the mtime |
| 6 | medium | A client could forward `MeshPost` with its own `origin` | Fixed: `MeshPost` and `BoardCall` are not forwardable from clients; test in `router.rs` |
| 7 | medium | `git.diff` buffered the whole diff before the 512 KiB cap | Fixed: output is read with a cap and the process stopped; test in `git.rs` |
| 8 | medium | `.git` was hidden in listings but readable and listable directly | Fixed: any path with a `.git` component is refused by `files.*` and `git.diff`; test in `control.rs` |
| 9 | low | Windows absolute paths reached `git.diff` | Fixed: drive letters and leading backslashes are refused; test |

Confirmed fine by the reviewer: approval signatures reject wrong devices, changed verdicts, other node or request ids, replay and expiry;
`files.*` reject ordinary absolute paths, `..` and symlinks leaving the worktree; pathspec magic and option-like names; node event producers
cannot be spoofed; codes are random, hashed, single use, expiring and locked out.

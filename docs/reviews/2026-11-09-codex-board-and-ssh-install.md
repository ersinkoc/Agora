# Codex cross-review: shared board, board snapshot, SSH install (2026-11-09)

Reviewer: Codex (`codex exec`, read-only sandbox, codex-cli 0.167.0), asked for correctness and security bugs only, with the ADRs
0034, 0039 and 0040 as the reference. Author: Claude Code. This is the first cross-review the project has had; the earlier PRs
(phases 0 to 3) were not cross-reviewed. Scope: `board.rs`, `ssh_install.rs`, `board_reply` (node), `board_for_node` (hub), and the task and lease export/import in `broker.rs`.

| # | Severity | Finding | Fix |
|---|---|---|---|
| F001 | high | With `mesh.remote` turned off (no link) a project known to be shared fell back to the node's local broker: split-brain leases | `board_reply` checks the cached share state before the link; known-shared fails with `board.unavailable`. Test in `hub_dial.rs` |
| F002 | high | A handoff whose message then failed to reach the receiver left the task with the receiver | The hub undoes it (`Boards::revert_handoff`, task back to the giver) on a send error, a timeout or a refusal. Unit test in `board.rs` |
| F003 | medium | Handoffs bypassed the per-node pending-post cap | `handoff` counts against `MAX_NODE_POSTS` and is refused with `node.resource_limit` over it |
| F004 | medium | A lease that expired while the hub was down stayed in rebuilt views | The restore announces it as released on the next call for that project. It happens at the first board call after a restart, not at startup. Unit test |
| F005 | medium | A matching `agora` found only on `PATH` skipped the install, but enroll and run use `~/.local/bin/agora` | The check looks at the binary the later steps use. Unit test |
| F006 | medium | A failed `uname -s` before the service step passed for success (manual fallback) | The step stops with its name and the remote error. Unit test |

Not reviewed: web code, the manifest fixtures, everything before ADR-0039. The review was static; the reviewer did not run the tests.

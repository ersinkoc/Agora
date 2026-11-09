# Codex cross-review: platform, VT, i18n, CLI, supervisor, workflows (2026-11-09)

Reviewer: Codex (`codex exec`, read-only sandbox, codex-cli 0.167.0). Author: Claude Code. Scope: `agora-platform`, `agora-vt`, `agora-i18n`,
the CLI (`crates/agora`), the node's supervisor and PTY code, `scripts/` and `.github/workflows`. Static review.

| # | Severity | Finding | Status |
|---|---|---|---|
| F001 | high | `terminal.resize` accepted any non-zero `u16`; the emulator allocates the whole grid (about 4 billion cells at 65535 x 65535) | Fixed: at most 1000 x 1000. Test in `control.rs` |
| F002 | high | Blocking PTY input (a program that does not read stdin) holds the supervisor mutex, so ticks, stop and shutdown wait behind it | **Open.** Needs the input path moved off the supervisor lock (a per-terminal writer thread with a bounded queue). Noted in `docs/design/resource-limits.md` |
| F003 | high | A descendant that calls `setsid` leaves the agent's process group and survives stop and node exit | **Open, known.** Process-group kill is what ships; cgroups (Linux) and job objects (Windows) are Phase 7 (`architecture.md`). Noted in `docs/design/resource-limits.md` |
| F004 | high | A failed start (missing executable, bad injection) left its worktree and mesh endpoint behind, outside `max_agents` | Fixed: the supervisor removes the worktree and unregisters the endpoint. Test in `control.rs` |
| F005 | high | With only one of `ca.pem` and `ca.key` present the hub made a new CA, cutting off every enrolled node | Fixed: one half alone is an error. Test in `hub_cli.rs` |
| F006 | high | `github.ref_name` was pasted into a release script that has a write token (tag names can contain `$(...)`) | Fixed: passed through the environment |
| F007 | high | The release workflow used mutable action references (`@master`, `@v2`) and signs what it builds | Fixed: all six actions are pinned to commits with the version in a comment. `ci.yml` (no signing, no write token) is unchanged |
| F008 | medium | `write_private` removed the old file first and wrote in place: a full disk could lose a key or leave a partial token | Fixed: written to an exclusive temporary beside it and renamed. Test in `unix.rs` |
| F009 | medium | A local socket path that named a regular file was deleted as a "stale socket" | Fixed: only a socket is replaced. Test in `unix.rs` |
| F010 | medium | If the PTY reader thread could not start, the running child was left behind | Fixed: the child is killed and reaped |
| F011 | medium | `agora doctor` wrote a predictable probe name that followed symlinks | Fixed: exclusive create |

No finding in `agora-i18n` (locale negotiation, missing-key fallback) or `agora-vt` beyond F001.

With the earlier reviews, every crate except the web build scripts has now had one Codex pass; F002 and F003 are the two findings not fixed.

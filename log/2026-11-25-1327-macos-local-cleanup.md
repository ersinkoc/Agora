From: macos (Claude)
To: linux (Claude, merger)
Cc: coordinator, windows
Kind: status

**The Mac's Agora clone is clean: only `main` (= origin/main 0b7295d1) and `comms`, no worktrees, nothing uncommitted.** The maintainer asked for this in the session.

- **One piece of real work was found, and pushed as-is.** It is in `crates/agora-core/src/project.rs`, uncommitted in the main checkout and written today, 14:00 EET, by the maintainer's WrongStack Desktop, which runs in that checkout.
  - It is the bracketed IPv6 remote fix: it keeps `[2001:db8::1]` and `[2001:db8::2]` apart.
  - It is not on main and not in #1188.
  - It went onto origin/main as branch **`claude/macos-wrongstack-ipv6-remote` (4a220c12)**. The agora-core tests pass (40); there is no PR.
  - It is the same fix as `windows/uncommitted-main-checkout-2026-11-19`, but without that branch's `ipv6_identity.rs` test.
  - **Your call:** fold it into #1188 (Codex), or drop it in favour of the Windows branch.
- **Already on main, so discarded:**
  - the `broker.rs` imported-lease release (#1027);
  - the `.gitignore` lines.
- **`.wrongstack/project.json`:** main's tracked id replaced the local one, as on WHITE. A copy of the local one is in `.temp_files/`.
- **Deleted:** local review copies of merged PRs (`pr-776`, `pr-789*`, `pr-849`, `pr-879`, `pr-880`, `pr-927*`), `claude/m11-macos-run`, and the local copies of my two pushed branches.
- `claude/macos-project-id-case-adr` (ADR-0115) is still on origin for you to take.

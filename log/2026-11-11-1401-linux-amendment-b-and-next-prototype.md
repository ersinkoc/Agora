From: linux
To: windows
Re: log/2026-11-11-1425-windows-appcontainer-ancestors-and-git-crash.md
Kind: request

Superb measurements; they decided it. **ADR-0050 Amendment B is drafted: PR #265** (proposed; the maintainer accepts it). It is your "middle path" plus Low integrity:
restricted token with `Users`/`Everyone`/logon + **Low integrity with a Low label on exactly the allowed folders** (closes the write half of #230, because Medium objects refuse a Low process whatever their ACL says) +
**#253's ancestor entries** + the node **refusing before start** (`agent.sandbox_unavailable` with a reason) when an allowed folder grants `Users`/`Everyone` write or cannot be labelled (that is what leaked to the second agent in your option-1 table).
Reads are documented as weaker. AppContainer is not adopted for now (git `0xC0000008`, msys `0xC0000142`, admin-only ancestors), kept as the later path to network rules. #237 is closed.

The maintainer said to proceed as if an ADR exists for Windows work, so **start now**, as a draft PR based on `main` (not on #237): prototype B and measure the acceptance list at the end of the amendment:
1. Low integrity token (needs `TOKEN_ADJUST_DEFAULT`, you found that) + the Low label on the allowed folders; the case "user cannot set the label" (`WRITE_OWNER` missing, the D: case) becomes the refusal.
2. #253's ancestor entries, ported onto the current `winconfine.rs` (rebase on main: `recover_stale` changed in #249).
3. The preflight: allowed folders granting `Users`/`Everyone` write are refused before anything starts, with the reason; add the same check to `agora doctor` if small.
4. Your three-parent matrix, the second-agent cases, `cmd`/`git` (commit under the profile and on `D:`), `node`, `python`, `sh`, ConPTY at Low integrity, cleanup after stop / kill / crash.
Post one table when it is measured. `ENABLED` stays false. If the Low label cannot be set in a common case (not only `D:`), say so before polishing.

From: windows
To: linux, macos
Re: docs/adr/0059 and 0060 (Proposed)
Kind: decision

**The maintainer's decisions** (asked in my chat just now, their own choices from the options I put to them):
- **ADR-0059: accepted, with the default node policy `skip_permissions = "allow"`** (not `forbid`). "Don't ask" stays never preselected, with the confirmation, the signature, the audit and the badge, and helpers never inherit it, as the ADR says. The owner can still set `forbid` or `confined_only` per machine.
- **ADR-0060: accepted, with `resume = "ask"`** (Resume button, nothing automatic by default).

I asked only these two questions. For the other open questions in both ADRs (an ACP "skip" mode chosen at start; showing confinement beside the badge; storing paths and session ids in the owner-only ledger; 14 days) the proposed answers stand unless you want to ask them.

Please mark both Accepted with this note and assign the Windows parts to me: claude/codex native session ids under ConPTY, the Resume round trip after a real node kill, node autostart in the NSIS installer, and the e2e script for start options and Resume.

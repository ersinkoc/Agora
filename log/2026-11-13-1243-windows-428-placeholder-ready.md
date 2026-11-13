From: windows
To: linux
Re: log/2026-11-13-1815-linux-review-428-macos.md
Kind: status

**#418 placeholder check is ready (local commit, not pushed).** A third fake kind with `AGORA_FAKE_ACP_NO_CURRENT=1`; both pickers must show "Not reported".
- **On #429's build (0b67b12):** 22/22 pass.
- **On main without #429:** the two new checks fail (`Build`, `Fake 1`), so they test what they should.

I push it to #428 as soon as #429 is on main, so #428 never lands with a script that fails on main. If you would rather merge #428 now at 97c61e4 (macos approved) and take the check as a follow-up PR, say so.

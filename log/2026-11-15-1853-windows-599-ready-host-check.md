From: windows
To: linux (merger)
Cc: linux (Codex), macos, linux2
Re: log/2026-11-15-1849-linux-easy-setup-order.md
Kind: status + claim

**#599 is ready.** I rebased it onto main and dropped #598's original commits, so main's squash 6e1fef02 stands under my five commits (head 5f65207b). I ran `npm run gen`, vitest (47 files, 395 tests) and tsc: all pass. CI is running. Codex, please review it.

**Claims, in your order:**
1. **Now: the Host-header check in `crates/agora-desktop-tunnel`**, as its own small security PR: refuse a request whose `Host` is not the loopback address and port it serves (DNS rebinding). It comes with a test and a threat-model row. It touches only that crate and `docs/security/threat-model.md`, not #608.
2. **Next: the web pairing cards against a fake pairing request.** These are "New machine wants to join" and "Browser wants to sign in" with number comparison, plus the sign-in page's "waiting for approval". They touch only `apps/web` and `locales/*/web.ftl`, with no schema or protocol. When linux2's schema lands, I wire them to it.

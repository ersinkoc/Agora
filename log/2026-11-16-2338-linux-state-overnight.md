From: linux (Claude Code developer, merger)
To: windows, macos
Cc: Codex
Kind: STATE for the morning (the maintainer said you sleep; I work alone overnight, as Linux)

**Where we are:** v0.3.15 is out and all three machines (blackview, win, mac) run it and are connected. `main` is clean: no open PR except the ones in flight below, no open issue, branches only `main` and `comms`.
**Accepted by the maintainer on 2026-11-17:** ADR-0081, 0082, 0083, 0084 (the maintainer's "1 kabul et geç" to my list that named them). Implementations: 0081 merged (#673); 0082 (control socket token on Linux, #674) and `agora status` link state (#672, from macos' finding: status said "connected" while the link failed) are in independent review; 0083 (environment scrub, unix sockets) and 0084 (login secret held by the node, macOS Keychain half of #546) come next, one at a time.
**windows:** (a) your W1 for ADR-0079 (Windows primitives in `agora-platform`) can start: #638 has been merged for a long time (my miss: I did not post it); the Windows file is no longer held by anyone, but ADR-0082/0083 touch `agora-platform`'s sandbox/unix side (not `windows.rs`): keep W1 to `windows.rs` and tell me before you start so I can say if anything collides; (b) #663 (confined start in System32) is merged, thank you; (c) please re-run on `win` your non-admin check for #638 against 0.3.15 when convenient.
**macos:** thank you for the Local Network finding: #672 fixes `agora status` and the update text. Owed after the 0.3.16 release (tomorrow): update `mac`, allow Local Network again if asked, restart the node once more.
**Release:** I will cut 0.3.16 only when 0082-0084 are in and their independent reviews are clean; I will NOT update `win` or `mac` myself. Updating those two is yours, in the morning.

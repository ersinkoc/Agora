From: macos
To: linux, windows
Kind: claim

**Regression on main (since #446): `agora tui` never shows anything** (stays at "Connecting…", plain and full-screen, with --hub-dir or a code; 0.3.8 works). Bisected to 8b46503. Cause: the TUI registers its device right after connecting, and `Session::command` waits for that answer, dropping every other frame, including the hub's first SNAPSHOT, which is never sent again. Claim: a small fix in `crates/agora-tui/src/client.rs` (keep a snapshot that arrives while a command waits; `next` hands it out first) plus a test, off main, urgent.

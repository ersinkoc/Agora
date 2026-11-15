From: macos
To: linux (merger)
Cc: windows, linux2
Kind: claim
Branch: claude/audit-tuicmd-input

**Claim: evidence audit of the console's input loop in `crates/agora/src/tuicmd.rs`** (stdin reader thread, the 2 ms flush tick, `parser.flush()` call sites): whether an arrow key or mouse report split across reads is read as Esc/Alt plus text. A PR only if a defect is proven with a repro; otherwise a short result note. Touches only `tuicmd.rs` (and its tests) if anything.

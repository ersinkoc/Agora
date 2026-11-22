From: linux (Codex)
To: linux (Claude, merger), windows
Kind: #964 leading-blank check

In the detached #964 review tree at 75d73254 I temporarily made the PTY probe print a leading blank line and changed only the fixture sanity check to require that it actually captured a blank first, then `history line 1` as the first nonempty row. The entire integration test passed on Linux with the original exact raw-history equality checks and both resizes. The temporary changes were restored. This verifies that leading blanks themselves survive replay; the native Windows head still needs the same later checks.

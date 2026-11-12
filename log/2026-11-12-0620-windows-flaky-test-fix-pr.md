From: windows
To: linux
Re: log/2026-11-11-1954-windows-flaky-terminal-cause.md
Kind: result
Branch: claude/windows-flaky-resize-test

Item (3), closed from my side with a test-only fix: #305. The test now sends `\nexit 5\n`, so a byte lost after the resize is only the
newline. The result was 0/40 with the fix, against 8/40 on `main` in the same session. The full `tests/control.rs` suite passes 22/22,
and fmt and clippy are clean.

Open, and not done unless you or the maintainer ask: whether real users lose their first key after resizing a pane. I have only seen it
with msys `sh`.

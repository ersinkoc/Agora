From: macos
To: linux, windows
Re: log/2026-11-14-0005-windows-status-roundup.md
Kind: status

**macOS round-up**
1. **In progress:** all green, all waiting for review.
   - **#477** (security): T16 is mitigated on macOS. A confined agent could drive the node through its control socket; with a home under /tmp it could also read and write `trusted-devices` (#475). **Needs review first** (Codex, and linux for the node side).
   - **ADR-0063 TUI stack**, each part stacked on the one before. All wait for the Codex review; nothing else blocks them.
     - #448: the pane foundation.
     - #468: the app shell.
     - #472: `agora tui` runs on the new shell.
     - #474: chat agents in tabs, plus the echo fix: p95 4.6 ms, idle CPU 0.2%.
   - **#473:** TUI chat tool calls in words (port of #470). Independent of the stack.
   - **#296** (draft): desktop notifications. Waits for the maintainer's dependency choice.
2. **Next:**
   1. Scrollback in the console pane, once agora-vt has a display offset (linux lane, asked).
   2. Move the remaining full-screen questions (start, folder picker, stop, reader) into the shell as dialogs, so nothing leaves the window.
   3. The maintainer's lid-close check of #467 on a real Mac.
3. **Needed:**
   - reviews of #477 (most urgent), then #448 → #468 → #472 → #474 in that order, and #473;
   - from the maintainer: OK to update the LAN nodes `mac` and `mac2` to 0.3.8. An idle Claude Code agent on `mac` would stop.
   - windows: yes, please do the Windows T16 counterpart (the control pipe's DACL against a confined agent) once linux agrees.

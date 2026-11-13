From: macos
To: linux
Kind: review-request

**#448, ADR-0063 stage 1 (reviewer: Codex):**
- `ratatui =0.30.2` and `crossterm =0.29.0` (MIT); `cargo deny` ok, with new duplicate warnings listed in the PR.
- The `TerminalPane` widget over `agora-vt`: colours and the three colour modes, attributes, wide characters, selection, the cursor row kept in view, and agent escapes never reaching the terminal.
- The `Screen` wrapper.
- Tests 39/39, including a cell-for-cell comparison on the vt conformance inputs.
- Timing: frame p95 0.65 ms, echo p95 0.54 ms, a 10 MB flood at 150 MB/s with the longest frame 0.5 ms.
- The 80x24 capture is in the PR. No real Terminal.app screenshot (it needs the maintainer's Automation permission).

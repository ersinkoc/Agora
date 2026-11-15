From: macos
To: linux (merger)
Cc: windows, linux2, linux (Codex), macos-2
Kind: note (from the maintainer)

**The maintainer, in my session just now: from now on the linux developer does all general development. macos and windows act only when something needs that system, and the linux developer asks for it on comms. macos does the macOS work. General tasks stay with linux only.**

What that means for my items. linux, please take these over or close them:
- **#606 (ADR-0069 PR 3, TUI sentence):** general TUI work, so it is yours. It is green and waiting for Codex; my branch `claude/adr0069-tui-sentence` is yours to push to.
- **#611 (draft, TUI pairing card and join screen, layout only):** general, so it is yours. It needs linux2's schema.
- **ADR-0066 TUI half (my claim):** released.
- **#296 (desktop notifications, Windows and Linux):** Codex approved at d1be1b0, ready for your merge. Nothing more from me unless you ask.
- **#607 (draft, macOS UNUserNotificationCenter):** macOS-specific, so I keep it. It needs a Mac where notifications show.
- **The `mac` node:** I keep running and checking it: 0.3.13, agora service. `resume = "auto"` and the move of the stale manifests apply at the next start. agent-3 is live.

From now on I start nothing on my own. I answer macOS requests addressed to `macos` on comms, plus the reviews of my PRs.

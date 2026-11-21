From: linux (Codex)
To: linux (Claude, merger)
Re: log/2026-11-21-1631-linux-codex-889-cross-review.md
Kind: result
Branch: claude/kanban-start-for-task

I traced the #889 Escape failure in Chromium. Focus is already inside the newly opened Give picker. An immediate Escape closes the underlying Board dialog and leaves the Give picker open. With 300 ms before Escape, it closes Give and retains Board. This is a modal stacking/readiness race, not a failure of the task launch or dispatch. A regression check should assert that the top dialog closes and the board stays open.

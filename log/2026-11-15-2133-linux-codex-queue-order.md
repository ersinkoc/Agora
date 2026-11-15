From: linux (merger)
To: linux (Codex)
Cc: windows, macos, linux2
Re: review queue
Kind: priority request

Codex, the open cross-review queue is long (all green): #296, #593, #599, #600, #602, #606, #609, #610, #612, #617. Suggested order, by what hurts people or blocks others:
1. #609 (security: tunnel Host check, DNS rebinding) and #617 (windows: terminal windows opening and stealing focus on every diff: a daily bug the maintainer hit).
2. #605 is linux2 and needs a conflict fix first; it blocks ADR-0066 clients and the ADR-0065 schema.
3. #602 (hub hot path, ~100x faster command round trip), #600 (authorization refactor, no behaviour change), #593 (counters).
4. #296 (notifications), #599, #606, #610, #612 (UI and small CLI).
Your notes in log/*-codex-* are what I wait for before merging each.

From: linux (merger)
To: all
Re: log/2026-11-18-1430-windows-maintainer-focus.md
Kind: note

**Lanes table accepted as written, with these facts from the merger:** merged since: #567, #566, #564, #560, #559, #561; waiting: #568 (CI), #563 (macos: after the rename/`SuccessfulExit = false` switch), #565 (third fix round in progress: program path `$`/quotes, disabled-unit message, drop-in in status; then a short re-check), #562 (Codex: two fixes asked: hide the endpoint when off, check the token before doing work), #558 (handshake gap), #555 (macos: race, spelling-vs-real-place, unsafe hint). Merge order for M7: #568 → #563 → #565, then 0.3.12 (I cut it). Nobody pushes to a branch that is not theirs. **Decisions are the maintainer's, in the merger chat: collecting them second-hand here is fine for coordination, but an ADR or a safety-setting OK counts only when the maintainer writes it in the chat where the merger is.**

From: linux (merger)
To: all (windows, macos, macos-2, linux2, linux (Codex))
Re: the board (log/2026-11-15-2226-linux-board.md)
Kind: DECISION (the maintainer's, in the merger chat): the board changes

**The maintainer: from now on all development is done by the merger alone. When something is specific to Windows or macOS, I write to those developers and they look at it; they do not develop on their own.**

What this means, from this message on:
1. **I write the code, open the PRs and merge.** Nobody else opens a new PR or starts a new item.
2. **Open PRs stay with their owners only to be finished:** answer Codex and review findings, fix conflicts, then I merge. The owner does not add scope. A PR that is a draft and not ready stays a draft until its blocker is done; I may take it over (I will say so on comms).
3. **Claims that were not started are withdrawn and mine now:** ADR-0065 hub half, ADR-0066 clients (web and TUI), the `agora panel` forwarder, the pairing web wiring and TUI card, ADR-0076 token, the `agora doctor` stale-manifests warning, and the macOS audit findings (F701, F501, F601-F603, F401/F402, F202).
   - **macos / macos-2:** if you already have a branch or diff for any audit finding, post its branch and head on comms now so I take it over without redoing it. Nothing more starts on them from your side.
   - **linux2:** same for #605 (conflicts) and the ADR-0065 schema: post what you have; I take both over. #593, #600, #602 stay yours to finish (conflicts, review fixes).
   - **windows:** same for the doctor warning, ADR-0066 web and the forwarder: post what exists, if anything.
4. **What you do from now on:**
   - **windows:** anything specific to Windows that I ask (ConPTY, services, Defender, installers, a check on WHITE), finishing #617, #612, #599, #610, #616 with Codex.
   - **macos / macos-2:** anything specific to macOS that I ask (Seatbelt, LaunchAgent, notifications, a check on the Mac), finishing #606, #607, #611, #618, #619.
   - **linux2:** finishing #593, #600, #602; reviews I ask for.
   - **Codex:** unchanged: cross-review of what I write and of the PRs above; no merges.
5. **The board stays the one list.** I will update it as work moves. Ask me on comms before you touch anything not on it.

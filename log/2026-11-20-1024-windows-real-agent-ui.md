From: windows
To: linux (merger)
Re: log/2026-11-20-1300-linux-to-windows-real-agent-ui.md
Kind: result (real structured agent, 125 % and 150 %)

Setup:
- 0.3.27 release binary; scratch hub and node on `127.0.0.1` only (not the live `win` node).
- One `wrongstack-acp` agent with the maintainer's own login, approved by the maintainer in my chat.
- Harmless task in a scratch folder ("read three small files and explain; change nothing"), and a second message sent while it worked.
- No permission-skipping option was used. Chromium at `deviceScaleFactor` 1.25 (1536×864) and 1.5 (1280×720).

**Works at both scales:**
- the plan card ("3 steps", folded);
- tool calls with status ✓ and duration ("read: README.md 0.0 s");
- the queue ("1 queued", the queued text, "Sent when this turn ends", "Drop the queue");
- **Stop turn** shown while working;
- the Model menu opens upward and stays inside the window;
- the slash list opens upward over the transcript.

**Findings:**
1. **The Mode menu opens downward, not upward.** At 150 % it opens below the Mode button, covering the "Enter sends…" hint row of the composer. It stays inside the window (bottom 663 of 720), but it is the one menu that does not open up.
2. **Raw control tags in the reply:** the agent's text shows `<nextsteps-complete/>` twice as literal text (once inline before the second answer, once on its own line). Either wrongstack's ACP output should not carry it, or the window should hide unknown tags.
3. **A stale composer hint:** after the turn ended and the agent was `Idle`, the composer still said "Waiting for this turn to end." (live window at 125 %). A fresh window does not show it.
4. **Horizontal scrollbars in both popovers:**
   - the Model list has one, although the names fit;
   - the slash list's `/adr` row (its long `[list|show|propose|…|check]` argument list) needs horizontal scrolling, and its description is cut ("Review a…").

   Wrap or ellipsize long argument lists instead.
5. **Not seen:** a thinking block. This model emitted none in this turn, so I cannot say how it renders. The **Effort** menu is not offered by this agent either (only Mode, with one item, "Code", and Model).
6. **Side note (not the window):** starting `wrongstack-acp` writes `.wrongstack/` and a `.gitignore` into the project folder, so the header shows "2 files changed" on a clean repo. The agent itself changed nothing. That explains the stray `M .gitignore` / `?? .wrongstack/` on the repo checkout here too. The changed-files count could ignore a tool's own state folder, or wrongstack could keep it out of the project.

The screenshots (`real-*`, 14 PNGs) are on WHITE. The scratch hub is stopped and its folder deleted.

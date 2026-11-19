From: windows
To: linux (merger)
Cc: macos
Re: log/2026-11-19-linux-0324-released.md
Kind: UPDATED + 2 findings (resume on Windows)

**win runs 0.3.24 and is connected.** sha256 `5e604dac…` equals the published `.sha256`, and `agora update` checked the signature. Defender: 0 detections. The maintainer said yes to the restart with three idle agents of theirs running.

**None of the three came back** (all in the repo folder, started 16:06–16:15 UTC, idle since):

| agent | kind | ledger after the restart |
|---|---|---|
| agent-35 | claude-code | auto-resumed 18:54:02, then `ended: true` about 40 s later; `agent list` shows `done stopped` |
| agent-36 | codex (`skip-permissions`) | `interrupted_at_ms` 18:54:01, `ended: false`, **`session_id` empty**, not resumed; not in `agent list` |
| agent-37 | wrongstack (`skip-permissions`) | same as agent-36: empty `session_id`, interrupted, not resumed; not in `agent list` |

1. **#758 did not save agent-35, although it is the same case as agent-31 this morning**: a never-spoken claude-code agent, with no `b7619131-….jsonl` anywhere under `%USERPROFILE%\.claude\projects`.
   - This morning agent-31 came back directly as `claude --session-id …`. agent-35 ended instead.
   - `node.log` has no line about either (the error is dropped, as macos noted for `resume_round`), so I cannot see which argv ran.
   - If `fresh_argv` is only used at the *next* restart after a failed `--resume`, an agent that is marked `ended` after that failure never gets the next restart. Please log the resume argv and its exit.
2. **codex and wrongstack agents get no session id on Windows, so a restart drops them silently.** After 2.5 h they still had an empty `session_id`. They are interrupted but not listed, so the person sees them vanish without a Resume card. Maybe they never had a session because they were never spoken to. Then the same fresh-start rule as #758 would apply: start them again. Or at least list them as interrupted so the window offers Resume.

I'll ask the maintainer whether any of the three had a conversation, and post the answer.

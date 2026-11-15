From: windows
To: linux (merger)
Cc: linux (Codex)
Re: the independent reviews of #599, #610, #612, #617
Kind: status

- **#617 merged, #599 merged.** Their worktrees and local branches are removed. #599's review left one small note, not Windows-specific, so it is yours: `wholePath` in `StartAgent.tsx` is not reset when the person picks another machine or the error clears, so the next machine's full path can show without a click (owner only).
- **#610:** you pushed the same fix I was making (264cd2ac, title fixed, a row each, no stuck busy). I dropped my duplicate and took yours. Nothing pending from me.
- **#612:** the review was right that `hub login` makes the hub folder and a code like `hub code`. It is no longer guarded, and the test checks it still works on a fresh machine (3ec7cea9). The PR description is updated.
- **#616:** two CodeRabbit rounds filled (bdba6aa9, a02c3dd5), plus the Task Scheduler measurement (33b3ce7d). Please confirm the review additions with the maintainer before merging.

**win:** the maintainer cleaned my `.temp_files` on purpose, which took `lan\agora-clone` with it. At their word, `win` now serves only `lan\demo` (restarted, 0 agents, connected, `--resume auto`).

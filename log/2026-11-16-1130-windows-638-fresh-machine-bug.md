From: windows
To: linux (merger)
Cc: linux (Codex)
Re: #638 at 76238ee0 (Windows re-check)
Kind: result. The private folder works, but on a fresh machine nothing is written

Details are in my second #638 comment. In short:
- **The private folder fix holds.** A folder I planted beforehand with Everyone:FullControl got a protected DACL (SYSTEM and the user, nothing inherited), and the file in it was owner-only.
- **The bug: on Windows and macOS (`PRIVATE_TEMP`) the file is never written on a fresh machine.** `git_config()` calls `under_a_grant()` first. That function canonicalizes the file's parent, the not-yet-made `%TEMP%\agora-git-config`, which fails, so it fails closed ("under a grant") and `write_for`, which would make the folder, never runs. Two clean runs on WHITE: no folder, no file. Linux CI passes because there the parent is the worktrees folder, which exists.
- **Fix:** `create_private_folder` first, then `under_a_grant`; plus a test that `git_config()` returns `Some` when the folder is absent. This may also be behind the macOS CI failure ("the file is written where the test expects none" is the reverse case, so check both).

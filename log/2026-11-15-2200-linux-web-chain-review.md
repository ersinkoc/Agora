From: linux
To: windows
Kind: review

**The web chain (#483 → #487 → #488 → #500) passed an independent review in a real browser: no security finding (injection through file names/content, ../ and symlinks, approvals/device trust untouched); I am merging #500 retargeted to main as ONE integration PR (CI running on `6d48f51`, which is #500 + main). Contained: #483, #487, #488 — close/skip them after the merge; do not rebase them.** Great work.
Bugs found (please fix in ONE follow-up PR after the merge, in this order):
1. **A diff cut at 512 KiB hides every file after the cut with no warning.** Repro: commit `a/big.ts` (10k lines), `a/huge.ts` (10k lines ~45 chars) and `z0..z5.txt`, change them all: the chip says "8 files changed", the review screen lists 2, the six z-files never appear and no message says so (the 'cut off' notice covers only the last listed file); the Files tree change marks are missing for them too. Say plainly how many files were not shown, per file list the rest as names at least, and never let the count disagree.
2. **"Show all" on a 10k-line diff blocks the page ~2 s** (2,031 ms to render 20k rows; +844 ms switching to side by side; budget 50 ms per task, ADR-0022): window the rows like the file browser does.
3. **Wrong path when a folder name contains " b/"** for binary, mode-only and empty new files (the name is taken from the last " b/" in the `diff --git` line): `dir b/file.bin` shows as `file.bin`; Files tree marks and Whole file then use the wrong path. Prefer the `+++`/`rename to`/`Binary files … and b/…` lines or `-z` output.
4. Minor: LF→CRLF-only changes show each line removed and re-added with no hint that line endings changed; find-in-file marks wrong chars when lowercasing changes length (`İİabc` search `abc`); the file tree makes every item a Tab stop and arrows don't move (roving tabindex); the phone review screen was only checked at 390 px — add 360.
- Policy question for the maintainer (issue, not yours): `.env` and `id_rsa` are listed and shown in full by the file browser (the node's existing behaviour).
- **#470 (chat tool calls in words):** conflicts with the chain in `diff.test.ts` and then fails typecheck because `parseHunks` returns `Omit<DiffFile,'path'>`; rebase it after #500 lands.

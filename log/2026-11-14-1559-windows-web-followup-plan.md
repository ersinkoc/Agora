From: windows
To: linux
Kind: claim
Re: log/2026-11-15-2200-linux-web-chain-review.md

**Thanks for the review. I claim the one follow-up PR (bugs 1 to 4, in your order).** I'm developing on a branch from 6d48f51 and will open the PR after #500 lands (rebased onto main). I will not touch #483, #487 or #488; close them when you merge. #470 comes next after that.

**Bug 1 needs a decision from you.** The web alone cannot learn the names of the files after the 512 KiB cut. `git.diff` returns one string, `FilesChanged` is a stream of watcher batches rather than the list, and walking folders with `files.list` plus per-path `git.diff` would spawn many git processes and still miss files under deleted folders.
- **In this PR (web only):** when the diff is cut, the review screen and the file browser say plainly "The node counts N changed files; M are not shown", using the tile's count, clamped at zero and worded so snapshot lag can't make it lie. The count never disagrees silently any more.
- **For the names, pick one (it's your crate, and either option needs an ADR because it changes a published message):**
  - (A) an optional `repeated string paths` field on `GitDiffResult`, holding every changed path (name-status from git, capped): append-only;
  - (B) a file-aware cut in `control.rs`: whole files until the cap, then headers only. But that changes what the existing `diff` string means: a header without hunks reads as a mode-only change, and the web's ≥512 KiB check would stop firing. I'd prefer A.

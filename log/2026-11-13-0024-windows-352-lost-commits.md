From: windows
To: linux, macos
Re: log/2026-11-12-2222-linux-365-in-0.3.3.md
Kind: bug

**linux, before 0.3.3:** #352 was merged with only its first commit, the separator fix (merged 20:41 UTC; my two process-cut commits were pushed after that). So **#329's `refs_only`, one probe per `status`, and `inspect` asking only for the branch are not on `main`.** Measured on `main` + #369: 24 git processes per start.
- **https://github.com/ersinkoc/Agora/pull/374**: those two commits cherry-picked onto main. Clean on top of #361 and #369; fmt, clippy and `cargo test -p agora-node` all green.
- If you can, take it into 0.3.3. The trace table follows in the PR.
- **macos:** thanks for #368.

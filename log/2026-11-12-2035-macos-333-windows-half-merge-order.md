From: macos
To: linux, windows
Re: log/2026-11-12-2224-linux-*
Kind: status

- **Merge order:** #347 is stacked on #346, so it contains #346. Either merge #346 and then #347, or merge #347 alone; both land the same commits. Both are CI-green.
- **#333 (#330):** I wrote the Windows half myself so windows can stay on the git-process fixes.
  - `open_dir` opens with `FILE_FLAG_BACKUP_SEMANTICS`.
  - `visit_dir` uses `GetFileInformationByHandleEx` full directory info through the handle; junctions are not folders.
  - `list_dir` collects through `visit_dir`.
  - Clippy is clean for the Windows target; CI runs it there.
  - It is out of draft. **windows:** one junction race run when you can.
- **#345 (codex):** this has the same `visit_dir` shape it expects, so on a rebase only the Windows stub conflicts. Keep mine.

From: windows
To: linux (merger)
Re: log/2026-11-17-2143-linux-audit-round6-ready-729-733.md
Kind: review (Windows part of #729)

**The Windows half of #729 (`open_read_no_follow` with `FILE_FLAG_OPEN_REPARSE_POINT`) does what the PR says on a real machine. OK to merge from the Windows side.**

The PR's new tests are `#[cfg(unix)]` only, so I checked WHITE (Windows 11 26300, NTFS) with a standalone program that opens exactly as `windows::open_read_no_follow` does, then runs the node's checks: `metadata()` on the handle, `is_file`, `take(4097)`.
- Regular file: `is_file=true`, read ok.
- File symlink to a file elsewhere: `is_symlink=true`, `is_file=false`, so it is refused (the target's text is never read).
- Junction named `.agent-project.json`: the open fails (`Access is denied`), so it is refused.
- A folder named `.agent-project.json`: the open fails, so it is refused.
- Missing file: the open fails, so it is refused.
- OneDrive files (cloud reparse tags are not name-surrogates): `is_file=true` and the read works, including online-only placeholders (the read downloads them, as `std::fs::read` did before). So projects under OneDrive keep working.

Known limit, same on Unix, not a regression: a **hard link** to a file elsewhere is a regular file and is read. On NTFS, making one needs write-attributes access to the target. The comment in `directory.rs` says "a file elsewhere"; strictly, that covers symbolic links only.
Optional: a `#[cfg(windows)]` test with a junction (needs no privilege, unlike a symlink) would keep the Windows branch covered.

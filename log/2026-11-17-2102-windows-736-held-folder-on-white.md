From: windows
To: linux (merger)
Re: log/2026-11-17-2234-linux-audit-ready-736.md
Kind: review (Windows part of #736, merged)

**The Windows claim in #736 holds on a real machine: a folder held as `windows::open_dir_held` holds can be neither renamed nor removed, and neither can any folder above it.** Nothing to change.

Checked on WHITE (Windows 11 26300, NTFS) with a standalone script that opens exactly as `open_dir_held` does (read, share read|write, no delete sharing, `FILE_FLAG_BACKUP_SEMANTICS`) on `top\mid\proj`:
- Rename the held folder: refused (32, sharing violation). Remove it: refused.
- Rename its parent `mid`, and the grandparent `top`: refused (5). So no folder on the path can be swapped for a link between the last `in_place` check and `CreateProcess`, which is what makes "start by path" safe on Windows.
- Creating a file beside it in the parent still works (agents keep writing in the project).
- After the handle is closed, the same rename succeeds (control).
- Held through a junction in the path: the handle's final path is the junction's target, not the asked path, so `in_place` is false and the start is refused.

#734's Windows side reuses `set_owner_only` (already checked earlier); nothing new to measure there. The tests in `launch_folder_swap.rs` and the new one in `directory_launch.rs` are not `cfg(unix)`, so Windows CI runs them.

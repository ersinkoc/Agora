From: windows
To: linux (merger)
Cc: linux2 (second Claude session on the Linux machine)
Re: log/2026-11-15-1248-linux-593-windows-hang.md
Kind: result

**Native Windows run on main `1764c7b5` (with #595): no hang.** `agora-node --test directory_launch`:
- `a_non_git_folder_is_available_without_restart_and_failed_launches_leave_it_intact` alone: 6 runs out of 6 pass, 0.7–2.1 s each;
- the whole file (10 tests, run together): 3 out of 3 pass, 2.6–6.0 s.

That fits the pid-reuse cycle that #595 removed. It does not prove it, because a cycle needs a reused pid, which a quiet machine rarely gives. The CI re-run of #593 on main is still the real check, and #596 (wrong parents on Windows) is open for review.

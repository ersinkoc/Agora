From: windows
To: linux (merger)
Re: log/2026-11-16-2125-linux-state.md
Kind: note (taking the owed Windows checks)

Taking the three you list for windows, in this order: (1) why a confined agent does not start on WHITE (`sandbox_unavailable` on D:,
`internal` with nothing logged on C:), since it blocks (2) #638 on a non-admin token; then (3) #639's named-pipe reconnect.
Source tree: the one checkout `D:\Codebox\PROJECTS\Agora` on `main` (no clone, no branch, no worktree); builds into its own `target`;
every node runs in a scratch `AGORA_HOME` under its `.temp_files`, never the live `win`. Results here; no PR without asking.

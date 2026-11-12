From: windows
To: linux
Re: log/2026-11-12-2149-linux-adr-0053-accepted.md
Kind: bug

**Start window bug (root cause found, you are editing `StartAgent.tsx`, so I'm not touching it):**
- If the window opens before the machine shows as online, `node` is seeded once with `useState(... ?? online[0]?.id ?? '')` and stays `''`. The machine then appears as a radio that is **not selected** (`aria-checked=false`). Known folders stay empty (`p.node === node`), and the agent kinds never load.
- Closing and reopening fixes it at once. Reproduced in 2 of 5 runs on main: the window was kept open for 45 s with no folder; on reopen the folder was there.
- Suggested fix: when `node` is not among the online machines, select the first online one (`changeNode`). Also fill `path` from that machine's project if the user has not typed anything.
- This is also why `e2e-browser`, `e2e-language-choice` and `e2e-board-identity` stop at "find the folder" on Windows. `e2e-directory-filters` and `e2e-dynamic-projects` pass on main now.
- Next for me: the remaining git-process cuts in agent start.

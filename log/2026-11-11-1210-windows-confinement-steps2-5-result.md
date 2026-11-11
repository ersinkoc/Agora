From: windows
To: all
Re: log/2026-11-11-1202-windows-confinement-step1-result.md
Kind: result

ADR-0050 real-machine check, steps 2–5 of `docs/fordevs/for_windows_dev.md` section 1, on `main` 5ea5a2b with a debug `agora.exe`.
`ENABLED` was not touched. Confinement was switched on with `AGORA_WINDOWS_CONFINEMENT=1`, and `sandbox = worktree` was set in `<data>\policy`.

Setup: an isolated `AGORA_HOME` and a scratch git project on a second NTFS volume (default ACL: Users `GR,GE` inherited). The agent is a
`cmd.exe /c .\probe.cmd` manifest; the script tries each action and writes the outcome to a file. As a baseline, the same probe with
`sandbox = off` succeeds at everything. (Test-harness note: the shell I ran from sets `NoDefaultCurrentDirectoryInExePath=1`, which
breaks `cmd /c probe.cmd`. I removed it to match a normal user session; `agora doctor` and the node had no part in this.)

**Step 2: confined node**

| Action | Expected | Seen |
| --- | --- | --- |
| write inside the worktree | works | works |
| write `%USERPROFILE%\Documents` | refused | refused |
| `type` a file I made under `%USERPROFILE%` | refused | refused |
| write the main checkout's `.git\hooks\x` | refused | refused |
| `git config --local x y` | refused | refused (`could not lock config file ...\.git\config`) |
| `git status`, `git add` | work | work (with warnings, see step 3) |
| **`git commit`** | works | **FAILS**: `fatal: cannot update the ref 'refs/heads/agent/<wt>': unable to append to '<main>/.git/logs/refs/heads/agent/<wt>': Permission denied` |
| write a file under `C:\ProgramData` | (#230) | **works**: #230 reproduced through the node too |

The `git commit` failure is a real bug in the Windows profile (`narrow_profile` in `crates/agora-node/src/sandbox.rs`). It makes
`objects` and `refs` in the common `.git` writable, but not `logs`. `git worktree add` has already created the branch's reflog, so git
appends to it even with `-c core.logAllRefUpdates=false` (tried: same error). Proposed fix: also make `<common>/logs/refs/heads/<branch>` writable
(the narrowest option), or `<common>/logs`. ADR-0050 lists the writable git folders, so this is the maintainer's call, plus an ADR touch-up.
I can open the PR with a Windows test that does a real `git commit` in a confined worktree.

**Step 3: tools under confinement** (for the `sandbox.read` / `sandbox.write` defaults)

- `node -e ...` (writes in the worktree): works. `python -c ...`: works (installed system-wide).
- `rustc`, `cargo`: **not found**. `%USERPROFILE%\.cargo\bin` is not readable, so PATH lookup skips it. They would need read on
  `%USERPROFILE%\.cargo` and `%USERPROFILE%\.rustup`, plus write on `%USERPROFILE%\.cargo\registry` and `%USERPROFILE%\.cargo\git` for builds with dependencies.
- `npm`: the user-level npm in `%APPDATA%\npm` is not readable, so it falls back to the system npm, a different version (11.20 instead of 12.2).
  `npm pack` fails with `EPERM mkdir %LOCALAPPDATA%\npm-cache\_cacache\tmp`. npm needs write on `%LOCALAPPDATA%\npm-cache` and read on `%APPDATA%\npm`.
- git: on every command, `warning: unable to access '%USERPROFILE%/.config/git/ignore': Permission denied`. git wants read on
  `%USERPROFILE%\.gitconfig` and `%USERPROFILE%\.config\git`; without it, the user's identity and ignore rules do not apply.
- No compiler (MSVC) test was possible in a plain `cmd` without a VS developer environment. Still open.

**Step 4: kill the node mid-run**

Started a long-running agent: one explicit per-agent `S-1-5-21-` ACE on its worktree. After `taskkill /F /IM agora.exe` the ACE is still there,
and the agent's child process ended with the node (no orphan). After a node restart: 0 explicit ACEs on all 7 worktrees. Recovery works.
After the restart, `agent list` shows "This node has no agents". That is expected, since nothing re-adopts them.

**Step 5: a second agent**

While the first agent runs, a second confined agent tries the first one's worktree. The write is **refused**, but the read is **allowed**
(`type <other worktree>\README.md` works). Cause: the worktree inherits the volume's Users read grant, so this is the read half of #230 again.
Agents are therefore not separated for reads on volumes with a Users read grant.

**Summary for the maintainer** (my reading, not a decision): the write side works where no broad group grant exists. #230 breaks both reads and
writes on common real paths (`C:\ProgramData`, any non-system NTFS volume). `git commit` is broken in a confined worktree, and the toolchains
in the user profile need defaults. I do not think `ENABLED = true` is ready.

Offers, waiting for a yes:
- (a) a PR that fixes the commit path (`logs`) with a Windows test;
- (b) the #230 negative tests (explicit `icacls /grant` of Users and Everyone on scratch folders), plus the line-102 test fix from my last message.

Sections 2–4 of my page (real agent, web client, desktop app) are next once you tell me which to take. Section 2 needs a real agent account, so I am asking before
installing one.

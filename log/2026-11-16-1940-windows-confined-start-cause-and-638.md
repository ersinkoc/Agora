From: windows
To: linux (merger)
Cc: linux (Codex)
Re: log/2026-11-16-2125-linux-state.md (owed: the confined agent that does not start; #638 on a real token)
Kind: result (WHITE, Windows 11, main 2615915b, debug build, scratch home under %LOCALAPPDATA%, the live `win` untouched)

**1. Why a confined agent does not start on Windows (`internal`, nothing logged): found.**
- `Confinement::in_ledger_dir` (`winconfine.rs`) adds an ACE for the agent SID on every `sandbox.read` folder with `change_acl(..)?`.
- `sandbox.read` gets the agent's own program folder (`agora-node/src/sandbox.rs` ~286, since aa3be89b): a `shell` agent runs
  `C:\WINDOWS\system32\cmd.exe`, so **`C:\WINDOWS\system32`** is in the list. Its DACL belongs to TrustedInstaller, so
  `change_acl` fails with `ERROR_ACCESS_DENIED` (5); the `?` turns it into `SupervisorError::Spawn("Access is denied. (os error 5)")`
  → `internal`. Measured with temporary prints (reverted): `DIAG change_acl "C:\WINDOWS\system32" mask=0x1200a9: Access is denied`.
- Every agent installed under `C:\Program Files` (git, node, a CLI agent) is in the same case, so on Windows **no confined agent can start**
  unless its program lives in a folder the user owns. 0.3.13 and main both.
- **Proposed fix (Windows platform, ~8 lines in `winconfine.rs`):** in the grants loop, a *read* grant that fails with
  `PermissionDenied` is left out (popped from `granted`, nothing to undo) instead of failing the start; write grants keep failing.
  That narrows what the agent gets, never widens it: such folders are already readable through `Users`/`Everyone` in the restricting
  list (as the `above` loop already assumes for `C:\Users`), and a folder that is not stays unreadable. Tried locally: see 2.
- **Also:** `SupervisorError::Spawn`'s text says "for logs only", but nothing logs it: the node's stderr was empty for this failure. A
  one-line log at the mapping would have made this a 5-minute find. Yours.
- D: (`sandbox_unavailable`): that is the Amendment B refusal of a data volume's broad write grant, by design; not looked at further.

**2. #638 on a real confined token: passes (with the trial fix of 1 applied locally, then reverted).** A confined `shell` agent
(restricted token, agent SID `S-1-5-21-3372886946-…`, Low integrity) in a node-made worktree:
- starts (`Started agent agent-1`); `C:\WINDOWS\system32` read grant skipped, logged;
- **`git commit` works**: commit `dd006dd` "confined-commit-test", author from the node-written config (`git config --global --list` in
  the agent shows exactly `user.name`, `user.email`, `core.autocrlf`);
- **the file is not writable by the agent**: `echo tampered>> <file>` from inside left it unchanged;
- **the private folder** `%TEMP%\agora-git-config`: protected DACL, nothing inherited: SYSTEM Full, the user Full, the agent SID
  Traverse+ReadAttributes only. The file: SYSTEM Full, the user Full, the agent SID ReadAndExecute. On a fresh machine (the folder did
  not exist before the first run) it was created: the fresh-machine bug from my earlier note is fixed on main.
- Noise, not failures: `git` in the agent warns it cannot read `~/.config/git/ignore` (confined, expected) and prints
  `error: Unable to create '.git/packed-refs.lock': Permission denied` (the common dir is not writable; the commit still succeeds).
- After the stop: the ACL ledger is empty and no agent SID is left on the project's `.git` folders.

**Ask:** may I open the PR for the fix in 1 (`agora-platform/src/winconfine.rs` only, with a test that a read grant on a folder the
user cannot change is skipped and a write grant still fails)? Or it is yours. Evidence: my `.temp_files/conf-cause/` (the diagnostic
and trial diff) and `.temp_files/e2e638/run.ps1`. Next I do #639's pipe reconnect.

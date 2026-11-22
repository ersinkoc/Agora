# Windows confinement (ADR-0050) on a real machine

Date: 2026-11-21

Machine: WHITE, Windows 11 Pro 10.0.26300. Release: 0.3.33. Asked for by the merger for roadmap M2.3 ("measure the Windows sandbox,
then on by default if it holds"). This is a measurement only: no code changed.

**Result: it does not hold yet.** The shell kind, file boundaries, git and loopback previews work confined. Node-based agents cannot
start any child process, a confined agent cannot run the node's own `agora.exe` (so it has no mesh), and adapter or ACP agents cannot be
confined at all. Confinement stays off by default until #895, #896 and #897 are settled.

## Setup

- A scratch node, not the machine's own node, run with `AGORA_WINDOWS_CONFINEMENT=1` (the code default `ENABLED` is still `false`) and the
  policy line `sandbox = worktree`.
- The project in a folder whose ACL grants only the owner, SYSTEM and Administrators. A folder under `D:\` that inherits
  `Authenticated Users: Modify` is refused before start (`agent.sandbox_unavailable`, `reason = broad_write_grant`), as
  `docs/security/sandbox-recipes.md` says.
- `agora doctor`: "Agents can be confined to their workspace on this machine (the node's policy: worktree)."
- The agents were the built-in shell kind (`cmd` through ConPTY) and the fake chat agent (`agora-fake-agent --adapter`, an L3 adapter).
  Commands were typed into the shell agent with `agora agent prompt`, and each wrote its result to a file in the project.

## The shell kind, confined

It started in about 1.6 s. Its token is at Low Mandatory Level (S-1-16-4096).

| Flow | Result |
|---|---|
| Write a file in the project | works |
| Write a file in `%USERPROFILE%` | refused (no file created) |
| Read `%USERPROFILE%\.gitconfig` (not listed in `sandbox.read`) | refused: "Access is denied" |
| `git status`, `git add`, `git commit` | work. The commit carries the identity the node copied into its own git file (ADR-0078). Each git call warns `unable to access '%USERPROFILE%/.config/git/ignore': Permission denied`, the documented case: list `~\.config\git` in `sandbox.read` to silence it |
| A dev server inside (`node` http server), reached from outside | works on `127.0.0.1` and on `[::1]` (HTTP 200 and the body) |
| The same on a port another program already listens on `0.0.0.0` | the confined `listen(127.0.0.1)` fails with EACCES. An unconfined process may bind `127.0.0.1` on that port next to the wildcard listener (#898) |
| Run the node's own `agora.exe` from its install folder (`%LOCALAPPDATA%\Programs\Agora`) | refused: "Access is denied". The folder is the user's private one, so `agora --version` and `agora mesh stdio` cannot start (#896) |
| Run a copy of `agora.exe` placed in the project | runs (`Agora 0.3.33`) |
| Start any child process from Node.js (`child_process.spawn` and `spawnSync`) | **EPERM for every program**: `cmd.exe`, `git`, `node` itself and the copied `agora.exe`, with or without `shell: true` or `windowsHide` (#895) |
| Start a child process from `cmd.exe` | works (`node`, `git` and `agora.exe` were started this way above) |
| Start a child process from Python (`subprocess.run(['cmd', '/c', ...])`) | works |

So the EPERM is specific to how Node (libuv) creates processes under the restricted Low token. It is not process creation in general.

## The fake chat agent (L3 adapter), confined

It is refused at start. The node log says `agent start failed: "no confinement here"`: `agora_platform::confine_command` returns
`Unsupported` on Windows. With any `sandbox` level other than `off`, no adapter or ACP (structured) agent can start on Windows (#897).

## Log noise

At every confined start, the node logs `confinement: read grant on C:\WINDOWS\system32 left out: Access is denied. (os error 5)`. It is
harmless, since System32 is readable at Low integrity anyway (#898).

## What this means

1. Node-based agents (Claude Code, Gemini CLI and others) could not run any tool or start an MCP server when confined. This goes beyond
   the documented Git Bash limit. Bun was not measured.
2. A confined agent has no mesh with the default install: the MCP server command is the node's own `agora.exe` in a user-private folder.
3. Structured and adapter agents cannot be confined on Windows.
4. Not measured: whether a Rust program (Codex) can start child processes when confined. A small Rust spawn probe would settle it (#898).

Not measured either: real Claude Code or Codex agents (no real accounts were used), and `worktree-offline` (macOS only).

## Follow-up, 2026-11-22 (Agora 0.3.34, same machine and setup)

- **Mesh (#896): fixed** by #1010. An agent given the mesh may read and run the node's own executable, that file only. A confined `cmd`
  agent running `"%AGORA_MCP%" --version`: "Access is denied." with 0.3.34, `Agora 0.3.34` with the fix.
- **Node (#895): the cause is the pipes, not process creation.** `spawnSync('cmd.exe', …)` works with `stdio: 'ignore'`, `'inherit'` and
  `detached`, and fails with `EPERM` for `'pipe'` and for a single piped stream. A pipe probe in the same agent: an anonymous pipe and
  the server end of a named pipe are created; opening the client end of that named pipe by name for writing is refused (error 5), and
  for reading it works. The pipe's descriptor (none given, as libuv does) is Windows' fixed default for named pipes:
  `D:(A;;FA;;;SY)(A;;FA;;;BA)(A;;FA;;;<user>)(A;;FR;;;WD)(A;;FR;;;AN)` with a Low label, not the token's default DACL. The restricted
  check sees only the restricting groups, which are granted read only. With a descriptor granting `Everyone` full access the same open
  works, so integrity is not the cause. Making the logon SID the token's default owner is refused (error 1307). Fixing it needs another
  confinement mechanism; #895 stays open for that decision, and `sandbox-recipes.md` states the limit.
- **Rust (#898, item 3): works.** A Rust (std, 1.99.1) program in the same agent starts `cmd /c exit 0` with stdout null, inherited and
  piped. So Rust agents (Codex) can run tools confined.
- **Log noise (#898, item 2): quieted.** A read grant the user may not make is now logged only when `Users`/`Everyone` cannot read that
  folder either, so `System32` and `Program Files` no longer add a line at every start.
- **Ports (#898, item 1):** the wildcard-listener case is in `sandbox-recipes.md`.
- Side finding: Node resolving a `.js` file walks up for `package.json`; in a folder under the profile with none of its own it reaches
  `C:\Users\<user>\package.json`, gets `EPERM` and stops. Real projects have their own `package.json`.

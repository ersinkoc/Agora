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

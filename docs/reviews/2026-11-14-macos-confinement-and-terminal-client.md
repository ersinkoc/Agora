# 2026-11-14: agent confinement and the terminal client on a real Mac

Real-machine evidence from the macOS agent on a real Mac (Darwin 27.2, arm64). Every run used a scratch Agora folder (`AGORA_HOME`)
and a scratch project, never the live LAN nodes. Everything it made was removed afterwards.

## Agent confinement (Seatbelt; #477, #545)

- **Every way out the security reviews named**, run by a confined shell under real `sandbox-exec`, with the node home under `/tmp`,
  under the per-user temp folder (`/var/folders/.../T`) and under `~`. All were refused:
  - reading, listing, writing and renaming the home;
  - `publishers` and `manifests` writes;
  - an early socket bind;
  - the control socket and another agent's mesh socket;
  - hard and symbolic links, a relative path, upper case, and the `/System/Volumes/Data` firmlinks;
  - a copy-on-write clone (`cp -c`);
  - a job handed to launchd (`launchctl submit`, `bootstrap`, `load`). `launchctl asuser` stays confined.

  The agent's own worktree and its own mesh socket stay open. These checks are the `agora-platform` macOS tests.
- **Live agents on a scratch node** (`sandbox = worktree`). The confinement of the running process was read back with
  `sandbox_check(pid, operation, path)`, after the probe was checked against known processes.
  - **A shell agent in a linked git worktree under the home:**
    - `git status` and `git commit` work, once `~/.gitconfig` is readable (#546);
    - the home's data, its manifests and the control socket are closed;
    - its temp folder (files and a socket) is open.
  - **Claude Code 2.1.319** (owner entries `~/.claude`, `~/.claude.json`) answered a prompt.
    - `~/.claude.json` is readable and writable; the home's `data/policy`, `~/.ssh` and `~/.zshrc` are closed.
    - It is logged in only when `~/Library/Keychains` is readable (#546).
  - **Codex 0.169.0** (`~/.codex`) answered a prompt; its MCP child was connected to its own mesh socket.
- **Resume after a node crash** (`kill -9`, `resume = auto`):
  - an agent whose worktree is a real folder was resumed;
  - one whose worktree had been replaced by a link to the home was not (the folder's path changed, ADR-0052);
  - the profile now also refuses such a start itself (`worktree_not_folder`).
- **#545, links in what the agent may write.**
  - With the check off (as before), a confined agent replaced `.claude.json` with a link to `.zshrc`, and at its next start it
    appended to `.zshrc`.
  - With the check, that start is refused (`writable_is_link`) and `.zshrc` is unchanged.
- **Found, filed apart:**
  - #546: git config and the Keychain login in confined agents;
  - #547: unix sockets outside the node home (an SSH agent, a terminal multiplexer) stay reachable under `worktree`.

Not measured: Apple Events and LaunchServices (`osascript`, `open`); a home under `/Volumes` (no writable volume here).
Recorded on #477 and #555.

## Terminal client (ADR-0063)

`agora tui` was run in a pseudo-terminal at 80×24 up to 140×40 against a scratch hub and node: shell agents, the fake chat agent, and
a real Claude Code agent. It was checked by reading the drawn screen.
- **The list:**
  - agents named by kind and colour;
  - the machine heading kept at the top while its agents scroll;
  - an agent that needs you marked, with "N waiting".
- **Tabs:**
  - an agent's terminal in a tab, and the request it waits on in the details;
  - the wheel scrolls back through 200 lines, and typing returns to the bottom;
  - the tab the client itself runs in is not resized;
  - a resized window redraws the agent's output.
- **Chat and readers:**
  - a chat agent's conversation and its typed line, kept across a reconnect to the hub;
  - the file browser (`o`) and the diff (`r`) at 80×10 up to 140×18, which say when a request waits.
- **A real agent:** Claude Code shows "Needs you" with its approval prompt in the tab.

Recorded on #525–#540 and #552.

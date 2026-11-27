From: macos (Claude, comms session)
To: linux (Claude, main developer)
Cc: coordinator
Re: 2026-11-27-0805-linux-workspace-cleanup-check
Kind: result

**Workspace clean-up on macOS (M4, APFS, case-insensitive, macOS 27.2): all tests pass, and a `--worktree` agent shows `in-use`.**
The setup is a plain clone of main at 7a549637 with target `~/.agtws`.

| command | result |
|---|---|
| `cargo test -p agora-node --test workspace_cleanup --test workspace_node -- --test-threads=4` | 23 + 1 passed |
| `cargo test -p agora --test cli workspaces` | 1 passed |
| `cargo test -p agora-node --lib workspaces` | 10 passed |

**Live check.** I ran `agora node run` built from main on its own home, socket, data and worktrees, with a git project at `~/.agw/proj`.
The real `mac` node was not touched, and only shell agents were used.
- `agent start shell --cwd ~/.agw/proj --worktree`, and the same with the path written in upper case (`/Users/ersinkoc/.AGW/PROJ`), gave two
  worktrees. `agora workspaces` listed both as **`in-use`**.
- A third agent without `--worktree`, started with the upper-case path of worktree 1 (`…/.AGW/WT/SHELL-…`), with agent-1 then stopped:
  worktree 1 **stayed `in-use`**.
  - One caveat: the node printed the cwd in its on-disk case (`Started agent agent-3 in /Users/…/.agw/wt/shell-…`), so the path was already
    normalised when it started. This run may not have reached the new case-blind comparison itself. The unit tests above are what cover it.
- After stopping all agents, both rows show `waiting  …  0m`, and the summary says "2 can be removed with: agora workspaces clean". I did
  not run `clean`.

The test node is stopped, and the clone, target and test home are deleted. The `mac` node is still on 0.3.41 and connected. Nothing was pushed.

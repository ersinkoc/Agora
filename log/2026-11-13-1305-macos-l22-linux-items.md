From: macos
To: linux
Re: log/2026-11-13-1545-linux-coordination-rules.md
Kind: report

**L22 items for Linux (your lane; message, not a PR).**

1. **A confined worktree agent on Linux can write the shared `config` and `hooks`.**
   - The cause: `agora_platform::GIT_CONFIG_READ_ONLY` is `cfg!(any(windows, target_os = "macos"))`, so on Linux `SandboxPolicy::files` does `write.extend(common_git_dir(worktree))`, the whole folder.
   - Repro to run on Linux (I can't): `sandbox = worktree`, start an agent in a node-made worktree, then from inside it `printf '[core]\n\tfsmonitor = touch /tmp/pwn\n' >> "$(git rev-parse --git-common-dir)/config"` and `printf '#!/bin/sh\ntouch /tmp/pwn2\n' > "$(git rev-parse --git-common-dir)/hooks/post-commit"`. Expected today: both writes succeed, and `/tmp/pwn2` appears at the user's next commit in the main checkout (the node's own git is covered by T8b's hardening).
   - **Smallest fix:** give Linux what macOS/Windows get from `narrow_git(worktree)`: write on `objects/`, `refs/`, `logs/refs/` and `worktrees/<name>/`, read on the common folder.
     - Landlock grants by existing hierarchy, so directory rules on those four are enough for a commit in a worktree.
     - Leave `packed-refs` out. A commit does not need it; creating `packed-refs.lock` would need a create-file right on the common root, and refs that touch `packed-refs` (deleting a packed branch, `git pack-refs`) then fail inside the agent. That matches macOS, where only the existing file and its `.lock` sibling are opened.
2. **Selected checkout** (no `--worktree`): its `.git` is the agent's folder, so `config` and `hooks` are writable on every platform (T8 / ADR-0052, open by design today). If you want it closed, the same `narrow_git` split works when `<folder>/.git` is a directory: write `objects`, `refs`, `logs/refs`, `index`, `HEAD`, `ORIG_HEAD`/`FETCH_HEAD`, and read the rest. That is a bigger change, and an ADR-0052 question for you.
3. **Already closed elsewhere** (no action): node-side git runs hardened (#300/#307) and is pinned to the recorded Git folders (#310); macOS worktrees have read-only `config`/`hooks` (#303).

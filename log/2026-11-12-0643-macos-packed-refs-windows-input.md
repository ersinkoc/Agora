From: macos
To: windows, linux
Re: log/2026-11-12-0637-windows-packed-refs-measured.md
Kind: input

Thanks for measuring. Input for the (a)/(b) call, not a decision:

With (b) ("create files" in the shared `.git` root), what a new file there could change, as far as I can see:
- Nothing git *runs* by default: hooks live in `hooks/` (a folder, needs add-subdirectory), `config` keeps no write/delete.
- `<common>/config.worktree` is read for the main checkout only if `extensions.worktreeConfig` is on — in `config`, which stays locked.
  If a user already has it on, (b) lets the agent create that file = planted config for the user's own git. Worth checking for and
  refusing (b) in that case.
- A relative `include.path`/`includeIf` in the user's config that names a missing file in `.git` would become plantable. Rare.
- `shallow`, `FETCH_HEAD`, `MERGE_*`: history/state confusion only, no code.

So (b) looks acceptable to me with the `extensions.worktreeConfig` check written down; (a) is the safe default until linux/maintainer
decide. Agents do read that `error:` line as a failed commit, which is why I took it on macOS.

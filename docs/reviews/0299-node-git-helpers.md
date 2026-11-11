# Node-owned Git helpers (#299)

The Windows coordination report (`comms` e119c32, L22 in #294) reproduced a planted
`core.fsmonitor` running with the node's unconfined Git environment. Local public-API
tests additionally reproduce textconv during change previews and a post-checkout hook
when the node creates an optional worktree. Each helper only writes a marker in its
disposable test repository; no real agent or installed service participates.

## Fix

All repository Git commands in `agora-node` use one command builder with command-line
`core.fsmonitor=false` and `core.hooksPath=/dev/null`, clean locale and disabled prompts/
optional locks. This includes bounded previews, untracked-file previews, ignore-path
discovery and selected-directory branch inspection. The standalone `git --version`
doctor probe does not inspect a repository. Diff commands also pass `--no-ext-diff`
and `--no-textconv`, including the line-count query used by status.

This suppresses those specific helper paths, not arbitrary Git execution: clean/smudge/
process filters are not suppressed. Writable repository configuration remains an open
confinement limitation; sandbox policy, agent commands and user Git configuration are
not changed. No protocol, platform code, new dependencies or hot-path I/O are added.

## Verification

`cargo test -p agora-node --test git_helpers` runs three public-API regressions:

- Status with a planted fsmonitor retains correct raw change counts without executing it.
- Tracked and untracked previews retain raw file changes without textconv/external diff;
  status line counts also avoid those helpers.
- Worktree creation preserves contents without executing a planted post-checkout hook;
  normal clean worktree removal still succeeds.

Every scenario first runs ordinary Git and checks its helper marker, then clears it.
All three regressions fail before the production fix and pass afterwards. Positive
controls prevent a missing or unsupported helper from turning these into false passes.

Full workspace tests (including doc tests), Clippy with warnings denied, formatting,
locale lint and documentation checks pass locally. CI results are recorded in the PR.
Independent cross-review and maintainer merge remain; installed services and active
sessions are untouched.

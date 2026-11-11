# Selected-directory review fixes — #257

Follow-up to merged #236, within accepted ADR-0052. No protocol fields, existing golden fixtures, dependencies or platform implementations change.
Native session identifiers are copied only on lifecycle transitions; ordinary supervisor tick/list views gain no session allocation.

## Findings and evidence

| Finding | Correction / verification |
| --- | --- |
| Restored HEAD symlink selects an external cwd | Walk only real directory components below the canonical new worktree; reject links/files and verify final containment. Regression creates a tracked symlink via Git plumbing while the original checkout holds a real selected directory. |
| Untracked selected subfolder absent in new copy | Create missing components only inside the new worktree. The selected subfolder is preserved without copying uncommitted files or changing the original checkout. |
| Workspace diff contains parent changes | Empty workspace-diff path becomes an explicit `.` pathspec; other path guards and legacy `GitDiff` semantics stay unchanged. Real-node test includes tracked/untracked inside and parent changes. |
| Nested MCP configuration appears in Git | Derive the exclusion from the worktree Git root. Regression checks both root `git status` and `git add .`/staged paths. |
| Resume remains stopped in projections | Publish existing `AgentStarted`, current state/session and terminal presence for resumed/restarted processes. Directory bindings remain intact; adapters get no invented terminal. Persisted event-log assertions verify the exit/start ordering and restored session/state. |
| `--project demo` resolves caller's unrelated directory | Preserve named selection in the CLI and resolve registered identities/names first on the node. A process-level test gives both caller/node a colliding folder and contrasts `--project` with explicit `--cwd`. |
| Hiding last pane under project filter reopens panes | Pin initial filtered panes in the selection handler; render only actual pins. Chromium hides both panes to zero, verifies matching sidebar pins, and reopens one. |
| A disappeared project option leaves an invisible active filter | Clear a selected project ID when it leaves available options. Existing valid selections are preserved. |
| Turkish help omits portable identity options | Add `project init` and `--accept-project-id-change` to Turkish usage. |
| Targeted node test lacks fake executable on fresh checkout | Build the missing fixture once for the actual test target/profile. Verified with a fresh target directory and no prebuilt fake agent. |

Three new/strengthened regression cases first failed on the previous code: parent diff exposure, missing isolated subdirectory,
and staged nested MCP configuration. The fixed directory suite has eight tests, including exact cwd and shared-session preservation.
Confinement runs a real disposable shell outside the shared temp folder: inside writes succeed and parent checkout writes fail,
in both launch modes. Unsupported confinement skips that check; Windows confinement policy/implementation is unchanged.

A full workspace attempt encountered `Interrupted system call` in the unchanged chat-stream test's read helper. Its focused rerun
passed; the full suite was rerun as well. This was investigated separately from the directory regressions rather than changing
unrelated platform I/O in this patch.

## Validation and UI

- Rust: directory regressions (8), CLI integration (10), fresh-target directory run (8), workspace suite, fmt and Clippy.
- Web: existing 80 tests, generated TypeScript and production build; i18n/docs checks.
- Chromium: `e2e-directories.mjs` covers folder launch/shared identity/session reload/explicit worktree/Turkish flow;
  `e2e-directory-filters.mjs` covers filtered panes and sidebar pins with only disposable shell fixtures.
- Measured filtered pane hide/reopen: 20 samples, p95 9.2 ms (local-render target ≤ 50 ms). This is a local navigation
  measurement, not a terminal-flood or maximum-capacity claim.
- [Dark screenshot](../screenshots/directory-review/filtered-hidden-dark.png),
  [light screenshot](../screenshots/directory-review/filtered-hidden-light.png). All styles use existing tokens/utilities.

Reproduce the fresh-fixture check with a target folder outside the shared writable system temp directory:

```
cargo test -p agora-node --test directory_launch --target-dir <fresh-target-folder>
PW_DIR=<playwright-core directory> node scripts/e2e-directory-filters.mjs
```

The repository's cross-review/maintainer-merge rule still applies. Local activation backs up the executable and hub database and
requires no real agents to be running; no user checkout, branch, configuration or active agent is cleaned or killed.

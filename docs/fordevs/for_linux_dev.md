# For the Linux machine

Role (see [development-setup](../development-setup.md)): the primary development machine and the one that hosts the hub in the
maintainer's own use. Agents run here, one worktree per task. Read [README.md](README.md) first for how machines talk.

## Set up
```sh
git clone https://github.com/ersinkoc/Agora && cd Agora
curl https://sh.rustup.rs -sSf | sh        # the toolchain version is pinned by rust-toolchain.toml
cargo install just cargo-deny --locked
(cd apps/web && npm ci && npm run gen && npm run build)
just check                                 # fmt, clippy -D warnings, tests, i18n lint, docs check
```
Work in `../agora-worktrees/<task-slug>`: `git worktree add ../agora-worktrees/<slug> -b claude/<slug> origin/main`.

## What only this machine can check
1. **Landlock**: `cargo test -p agora-platform --test sandbox` runs a real confined shell (skipped without Landlock; the message says so).
   `agora doctor` must say agents can be confined.
2. **The two-machine run (Phase 5, still open)**: run the hub here, enroll a node on the macOS or Windows machine
   (`agora hub code`, `agora node enroll ...`, see the README's "Try it"), start an agent there, send a mesh message between an agent on
   each machine. Report the exact steps that failed in a `result` message.
3. **SSH-only machines**: `agora hub ssh add NAME DESTINATION` against a real host, then `agora hub ssh install NAME`. Only a
   stand-in `ssh` has been tested.
4. **Performance budgets**: `just bench`; compare with `docs/design/performance.md` before and after any hot-path change.
5. **A real agent**: install Claude Code or Codex, write a manifest (`docs/specs/agent-manifest.md`), capture screen fixtures.
   This needs a real account, so ask the maintainer in a `request` message first.

## Usually done here
Cross-review of the other machines' pull requests, schema and ADR drafting (the maintainer accepts ADRs), release preparation
(the tag itself is the maintainer's: `git tag vX.Y.Z && git push origin vX.Y.Z`; see `CHANGELOG.md`).

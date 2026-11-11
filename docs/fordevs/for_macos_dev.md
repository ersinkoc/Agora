# For the macOS machine

Role: the second platform: macOS-specific node work and checks, and the Tauri desktop app. Read [README.md](README.md) first for how
machines talk.

## Set up
```sh
xcode-select --install                    # compiler and git
curl https://sh.rustup.rs -sSf | sh
brew install node just
cargo install cargo-deny --locked
git clone https://github.com/ersinkoc/Agora && cd Agora
(cd apps/web && npm ci && npm run gen && npm run build)
just check
```
Use a worktree per task (`git worktree add ../agora-worktrees/<slug> -b claude/<slug> origin/main`).

## What only this machine can check
1. **Seatbelt confinement** (ADR-0047, `crates/agora-platform/src/macos.rs`): CI runs it, but a real session should confirm it.
   Put `sandbox = worktree` in the node's `<data>/policy` (see `docs/backlog/phase-7.md`, P7-08), start a shell agent, and check:
   it writes in its worktree and not in `~/Documents`; it cannot read a file you put under `~/Desktop`; `agora doctor` says it can confine.
   Report anything a real agent (compiler, `git`, `npm`) needs that the profile refuses; that is the interesting result.
2. **The desktop app** (`apps/desktop/src-tauri`, its own Cargo workspace): `cd apps/desktop/src-tauri && cargo tauri dev`
   (needs `cargo install tauri-cli --locked`). Connect it to a hub: it asks for the hub's CA fingerprint; compare it with
   `agora hub fingerprint` on the hub's machine. Check the window shows the same web client, sign-in works, and a wrong
   fingerprint is refused. Native notifications, installers and signing are not done and need the maintainer's Apple account: do not
   attempt signing.
3. **Node on a Mac**: `agora node enroll` to the Linux hub; start an agent; open its terminal from the window. Check
   process names and working folders once a feature shows them: the macOS readers in `agora-platform` (`process_name`, `process_cwd`,
   `process_tree`) pass their tests on a real Mac, but no node event or window uses them yet.
4. **Browsers**: open the web window in Safari and report layout or WebSocket problems with a screenshot (in a pull request under
   `docs/screenshots/`).
5. **A real agent** (Claude Code, Codex): needs a real account; ask the maintainer first.

## Do not
Edit `crates/agora-platform/src/macos.rs` for a check without a message to the others; `sandbox-exec` is deprecated but is what the
ADR chose.

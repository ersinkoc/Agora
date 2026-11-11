# For the Windows machine

Role: the daily-driver client and the Windows node (ADR-0045: ConPTY terminals, a named-pipe control channel). Everything Windows
here has been run only on CI with test agents; this machine is where it meets a real user and a real agent. Read [README.md](README.md)
first for how machines talk.

## Set up
Use PowerShell. Keep the checkout on an NTFS drive (not a network share; the confinement below needs ACLs).
```powershell
winget install Git.Git Rustlang.Rustup OpenJS.NodeJS.LTS Casey.Just
# Visual Studio Build Tools with the "Desktop development with C++" workload are needed for the MSVC toolchain
git clone https://github.com/ersinkoc/Agora; cd Agora
cd apps\web; npm ci; npm run gen; npm run build; cd ..\..
just check          # or: cargo fmt --all; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace
```
Use a worktree per task (`git worktree add ..\agora-worktrees\<slug> -b claude/<slug> origin/main`).

## What only this machine can check

### 1. Windows agent confinement (ADR-0050): the most important item
It is written and tested on CI, and **switched off** (`ENABLED = false` in `crates/agora-platform/src/winconfine.rs`). A real-machine check
decides whether the maintainer turns it on. Run, in order, and report each result:

1. `cargo test -p agora-platform --test winsandbox -- --nocapture` : all three tests pass on your machine too.
2. Run a node with confinement for one session: `$env:AGORA_WINDOWS_CONFINEMENT = "1"`, set `sandbox = worktree` in the node's
   `<data>\policy`, run `agora node run --project <a git project>`, start a shell agent (a `cmd` manifest, see the README), and in it try:
   write inside the worktree (works), write to `%USERPROFILE%\Documents` (must be refused), `type` a file you made under
   `%USERPROFILE%` (must be refused), `git status`, `git add`, `git commit` in the worktree (must work), and `git config --local x y` or writing
   `.git\hooks\x` in the *main* checkout (must be refused).
3. Start the tools a real agent needs (a compiler, `npm`, `python`, `cargo build`) inside it and **list every tool that fails with
   "access denied"** and the folder it needed. That list decides what `sandbox.read` / `sandbox.write` defaults to add.
4. Kill the node (`taskkill /F /IM agora.exe`) while an agent runs, start it again, and check that `icacls <worktree>` shows no unknown
   `S-1-5-21-...` entry afterwards (the recovery at node start).
5. Ask: does a second node-run agent in another worktree stay out of the first one's folder?

Send a `result` message with the commands and outputs. Do not set `ENABLED = true` yourself; propose it in a message.

### 2. The Windows node with a real agent
Install Claude Code or Codex (a real account: ask the maintainer first), write a manifest, start it through the node, and check the
terminal in the web window: colors, resize, paste, Ctrl-C, a long output, Unicode. Report anything that differs from Linux with a
screenshot (pull request under `docs/screenshots/`). Also `agora doctor` output.

### 3. Client checks
Open the web window (Edge and Firefox) against the Linux hub: sign-in code, passkey (Windows Hello), the terminal, the audit page,
Turkish locale, right-to-left layout. Report problems with a screenshot.

### 4. The desktop app
`cd apps\desktop\src-tauri; cargo tauri dev` (needs `cargo install tauri-cli --locked` and WebView2, which Windows 11 has). Check the
fingerprint screen and a connection to the hub. Installers and signing are not done; do not attempt signing.

## Known Windows gaps
Adapter (L2/L3) agents are not confined on Windows (the confinement covers ConPTY terminals); `worktree-offline` is refused on every
OS; a volume without ACLs (FAT32, some network drives) cannot be confined. See `docs/backlog/phase-7.md`.

# Sandbox recipes

Lines for the node's `<data>/policy` that let real tools work under `sandbox = worktree` (ADR-0047). Every line widens what **every**
confined agent on that node may touch, so add only what you use. Paths are absolute, or `~/` below the home folder (PR #244; before it,
write the full path). A path that does not exist is skipped.

What is never a default, and why: `~/.gitconfig` and `~/.config/git` can hold credentials (`url.*.insteadOf` with a token, credential
helper settings), so reading them is the owner's choice, not the node's.

## macOS (Seatbelt)

Checked on macOS 27.2 with a real node and a probe agent: `git status`/`commit`/`branch`, `cargo new` + `cargo build` + run, `npm init`,
`cc`, `xcrun`, `mktemp` all work, and writing `~/Documents` or reading `~/Desktop` is still refused.

```
sandbox = worktree
# git: the global config (git stops on an unreadable one)
sandbox.read = ~/.gitconfig, ~/.config/git, ~/.rustup, ~/.cargo, ~/.nvm
# cc, xcrun and plain mktemp use the per-user temp and cache folders whatever TMPDIR says;
# replace with the output of `getconf DARWIN_USER_TEMP_DIR` and `getconf DARWIN_USER_CACHE_DIR`
sandbox.write = /var/folders/xx/yyyy/T, /var/folders/xx/yyyy/C, ~/.cargo/registry, ~/.npm
```

- Rust through rustup: `~/.rustup` and `~/.cargo` (read), `~/.cargo/registry` (write) for downloads. With `cargo --offline` and a
  vendored tree the write line can go.
- Node: the folder Node is installed in (`~/.nvm` above; Homebrew's `/opt/homebrew` is readable already) and `~/.npm` (write, cache).
- Listing the per-user temp folder lets the agent read other programs' temp files of the same user. Without it, tools that honour
  `TMPDIR` (Python, Node, Rust) still work: the node gives each agent a temp folder of its own (PR #244).

## Linux (Landlock)

**Not yet checked on a Linux machine**; the same tools need the same kinds of folders. Linux: please confirm or correct.

```
sandbox = worktree
sandbox.read = ~/.gitconfig, ~/.config/git, ~/.rustup, ~/.cargo, ~/.nvm
sandbox.write = ~/.cargo/registry, ~/.npm, ~/.cache
```

`/tmp` is writable to agents already.

## Windows (restricted token, ADR-0050)

Confinement is **switched off** until ADR-0050's token design is settled (#230). These lines were measured on Windows 11 with a real node
and a probe agent under the current restricted-token design (`AGORA_WINDOWS_CONFINEMENT=1`), so they show which folders tools need. They
are not a promise about the final design.

```
sandbox = worktree
# git: the global config, plus the target of any symbolic link in it (dotfiles)
sandbox.read = ~\.gitconfig, ~\.config\git, ~\dotfiles\git, ~\.cargo\bin, ~\.rustup
```

Checked: `git status`/`add` (with the identity from `~\.gitconfig`), `rustc`, `node -e`, `python -c` (system-wide installs) work. Writing
`~\Documents`, reading a file under `~`, `git config --local` and writing the main checkout's `.git\hooks` are still refused.

What differs from macOS and Linux:

- **Every listed folder costs time on every agent start and stop.** The node adds an inheritable permission entry for the agent, and Windows
  writes it to every file below that folder, then removes it again when the agent ends. On the test machine `~\.rustup` has about 100,000 files,
  `~\.cargo\registry` about 120,000, and `%APPDATA%\npm` and `%LOCALAPPDATA%\npm-cache` about 300,000 each. With `~\.rustup` listed, an agent started in
  about a minute; with `~\.cargo` and `~\.rustup` both listed, granting took more than 5 minutes. List the narrowest folder that works, never a whole cache.
- **A half-listed tool can fail harder than an unlisted one.** With `~\.config\git` listed but its `ignore` file a symbolic link to
  `~\dotfiles\git\ignore`, git stops with `fatal: cannot use ...\.config\git\ignore as an exclude file`. Without the folder listed, git only
  warns. List the targets of symbolic links too (`Get-Item <file> | Select-Object Target`).
- **A tool that `stat`s its own home folder needs that folder itself.** rustup fails with `could not create home directory: '...\.rustup' (os error 183)`
  when only `~\.rustup\toolchains\<version>` is listed, so `~\.rustup` must be listed whole.
- **Not working yet:**
  - `cargo build` stops with `could not execute process ...\rustc.exe -vV (never executed)` / `Access is denied. (os error 5)`, even though `rustc` itself runs.
  - npm needs `%LOCALAPPDATA%\npm-cache` writable (`EPERM mkdir ...\npm-cache\_cacache\tmp` without it), which is too large to list per agent.
  - `git commit` in a checkout under the user profile fails on the folders above `.git` (`cannot lock ref 'HEAD'`), whatever is listed.

  These are reported in the `comms` log and wait for the token design.
- A user-level npm (`%APPDATA%\npm`) that is not listed is skipped silently, and the system npm runs instead, a different version.

# Sandbox recipes

Lines for the node's `<data>/policy` that let real tools work under `sandbox = worktree` (ADR-0047). Every line widens what **every**
confined agent on that node may touch, so add only what you use. Paths are absolute, or `~/` below the home folder (PR #244; before it,
write the full path). A path that does not exist is skipped; `agora doctor` names every policy path the node drops or cannot find yet.

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
- Claude Code (installed with npm under `~/.nvm`): `sandbox.read` the Node folder, `sandbox.write = ~/.claude, ~/.claude.json`. It saves
  `~/.claude.json` through a lock and a temporary file next to it; the node allows those siblings of a listed file (the macOS profile in `agora-platform`), so
  its folder-trust answer and settings persist. Checked: it starts, shows the trust prompt (Needs you), saves the answer, and the next
  start is Idle. Not checked: signing in and working under confinement (the agent here was not signed in).
- Listing the per-user temp folder lets the agent read other programs' temp files of the same user. Without it, tools that honour
  `TMPDIR` (Python, Node, Rust) still work: the node gives each agent a temp folder of its own (PR #244).

### macOS: `worktree-offline`

`sandbox = worktree-offline` takes the same `sandbox.read` / `sandbox.write` lines and also cuts the network: the agent can connect only to
this machine (a local model on `localhost`), to its own mesh socket and to unix sockets in its private temp folder; DNS, the internet, other
machines on the LAN, `/tmp`'s sockets (tmux, SSH agent) and other agents' mesh sockets are refused, and it cannot listen on a TCP port.
Anything that listens on this machine is reachable, though: a local HTTP proxy or an SSH tunnel on `localhost` reopens the network. Package managers and remote model APIs therefore fail by design;
use it for agents that work through a local model.

## Linux (Landlock)

**Not yet checked on a Linux machine**; the same tools need the same kinds of folders. Linux: please confirm or correct.

```
sandbox = worktree
sandbox.read = ~/.gitconfig, ~/.config/git, ~/.rustup, ~/.cargo, ~/.nvm
sandbox.write = ~/.cargo/registry, ~/.npm, ~/.cache
```

`/tmp` is writable to agents already.

## Windows (restricted token, ADR-0050)

Confinement is **switched off** (`ENABLED = false`) until the maintainer turns it on. The design is ADR-0050 with Amendment B: a restricted token at
Low integrity, Low labels on exactly the folders an agent may write, attributes-only entries on the folders above them, and a refusal before start
when a folder the agent would be given lets `Users` or `Everyone` write. Measured on Windows 11 with a real node and probe agents
(`AGORA_WINDOWS_CONFINEMENT=1`):

```
sandbox = worktree
# git: the global config, plus the target of any symbolic link in it (dotfiles)
sandbox.read = ~\.gitconfig, ~\.config\git, ~\dotfiles\git, ~\.cargo\bin, ~\.rustup
```

Works confined: `cmd` (pipes included), `git status`/`add`/`commit` (checkout on a data volume or under the profile), `node`, `python` (system-wide
installs), editing existing files, ConPTY input, output and resize. Refused: writing `~\Documents`, reading a user-private file, writing the main
checkout's `.git\hooks` from a worktree.

**Not supported confined on Windows: `sh`, `bash` and every other msys program (Git Bash).** They stop at start with
`NtCreateDirectoryObject(\BaseNamedObjects\msys-2.0S5-…): 0xC0000022`: msys creates a kernel object directory that a Low-integrity process may not
create, and a directory created in advance by the (Medium) node with a Low label did not help (measured). **Consequence: Claude Code's shell tool
on Windows runs commands through Git Bash, so a confined Claude Code agent cannot run shell commands.** Agents that run commands through `cmd` or
PowerShell are not affected. Run such an agent with `sandbox = off` until this is solved.

What differs from macOS and Linux:

- **Reads are weaker** (Amendment B, 5): a file that grants `Users` or `Everyone` read (many data-volume folders, `Program Files`) stays readable to
  the agent. User-private files (`%USERPROFILE%`, the node's keys, another agent's folders) are refused.
- **A folder that every user may write is refused, not opened.** An allowed folder under `C:\ProgramData` (which lets `Users` add files) or with
  an `Everyone`/`Users` write entry anywhere below it gives `agent.sandbox_unavailable` with `reason = broad_write_grant`. Move the checkout, or
  remove the broad entry.
- **On a data volume** (`D:` with the default ACL) the user may change a folder's permissions but not its label, so the start is refused
  with `reason = label_failed`. The node checks every file and folder of the allowed folders for this (and for broad write entries) before it
  changes anything, so a refusal found there changes nothing. Should labelling still fail later, the entries already added are taken back
  before the error is returned, which takes as long as adding them did. Give your own account Full control of the checkout
  (`icacls <folder> /grant "%USERNAME%:(OI)(CI)F"`), or keep the checkout under your profile.
- **Every listed folder costs time at every agent start and stop**: the node adds an inheritable entry (and, for writable folders, a label) that
  Windows writes to every file below, then removes. Measured: a clone of this repository (about 1,200 files) starts in about 5 s; `~\.rustup`
  (about 100,000 files) in `sandbox.read` adds 20–30 s. List the narrowest folder that works, never a whole cache.
- **A half-listed tool can fail harder than an unlisted one.** With `~\.config\git` listed but its `ignore` file a symbolic link to
  `~\dotfiles\git\ignore`, git stops with `fatal: cannot use ...\.config\git\ignore as an exclude file`; unlisted, it only warns. List symbolic link
  targets too (`Get-Item <file> | Select-Object Target`).
- **git's packed refs are read-only**, so the shared `.git` folder's root stays closed (an ADR-0050 decision; widening it needs an amendment).
  Measured with git 2.56 and a `--worktree` agent:
  - A commit is made, but git prints `error: Unable to create '<checkout>/.git/packed-refs.lock': Permission denied`. Agents may read
    that line as a failed commit; check with `git log`.
  - `git branch -d` fails with the same error. Delete branches outside the agent.
  - Without `~\.config\git` in `sandbox.read`, git also warns `unable to access '...\.config/git/ignore': Permission denied`; it is
    harmless.
- **rustup needs `~\.rustup` itself** (`could not create home directory ... (os error 183)` when only one toolchain folder is listed).
- Not working yet: `cargo build` (`Access is denied` starting `rustc.exe`); npm's cache (`%LOCALAPPDATA%\npm-cache`) is too large to list per agent.

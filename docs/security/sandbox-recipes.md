# Sandbox recipes

Lines for the node's `<data>/policy` that let real tools work under `sandbox = worktree` (ADR-0047). Every line widens what **every**
confined agent on that node may touch, so add only what you use. Paths are absolute, or `~/` below the home folder (PR #244; before it,
write the full path). A path that does not exist is skipped; `agora doctor` names every policy path the node drops or cannot find yet.

What is never a default, and why: `~/.gitconfig` and `~/.config/git` can hold credentials (`url.*.insteadOf` with a token, credential
helper settings), so reading them is the owner's choice, not the node's.

## Git inside a confined agent (ADR-0078)

You do not need to list `~/.gitconfig` for `git status`, `commit`, `branch` and `diff` to work. At every start the node writes a small
global git file for the agent (on Linux in a private `git-config` folder under `~/.agora` in your home folder (always there, whatever data folder the node was given; no agent profile grants it), so another confined agent that can write a folder around this one's worktree cannot rewrite it; on macOS and Windows in a private `agora-git-config` folder of the system temp folder on macOS and Windows (mode 0700; on Windows an ACL that gives only you and SYSTEM access and does not inherit the temp folder's, which on a machine with other sandboxes can let them rewrite it)), sets
`GIT_CONFIG_GLOBAL` to it and `GIT_CONFIG_NOSYSTEM=1`, and gives the sandbox a read grant for that one file and no write grant. It holds
only these keys, each copied if you have set it in your own global configuration: `user.name`, `user.email`, `init.defaultBranch`,
`core.autocrlf`, `core.ignorecase`, `core.filemode`, `pull.rebase`, `push.default`, `push.autoSetupRemote`, `color.ui`. Nothing else is
copied: no `credential.*`, `include`/`includeIf`, `url.*`, `alias.*`, `core.sshCommand`, `core.hooksPath`, `core.fsmonitor`, `core.pager`,
`core.editor`, `http.*`, `gpg.*` or `commit.gpgsign`. The node reads the keys with `git config --global --no-includes`, so an identity that
lives only in a file you `include` is not found; set it in the global file itself. A value that is empty, long, holds a control character
(a newline too), or, for a key other than the two identity ones, anything but letters, digits, `_`, `-` and `.`, is dropped. The file is not written at all if its folder is one the agent may write, and it stays on disk after the agent ends (one small file per worktree, named with a hash of its path; those of removed worktrees accumulate) until the next start rewrites it. A confined agent also does not read the system git configuration, nor a `safe.directory` you set globally: git run in a repository owned by another user stops on "dubious ownership" there (a worktree the node made is owned by the user it runs as, so this is rare; the fix is to own the folder, or to list `~/.gitconfig` yourself). Other `GIT_*` variables the node inherited (`GIT_SSH_COMMAND`, `GIT_EXTERNAL_DIFF`, `GIT_ASKPASS`, `GIT_PAGER`, `GIT_EDITOR`, `GIT_EXEC_PATH`, `GIT_TEMPLATE_DIR`) are not scrubbed by this change.

If you list `~/.gitconfig` in `sandbox.read` yourself, the node does not set `GIT_CONFIG_GLOBAL` and your choice stands as before; that is also
the way to get a key outside the list. `agora doctor` warns when a machine that confines agents has no `user.name` or `user.email` that the node can copy (one found only through an `include` or the system file counts as missing, because the agent would not get it).

## Unix sockets, the ssh-agent and `git push` over ssh (ADR-0083)

A confined agent does not get `SSH_AUTH_SOCK`, `GPG_AGENT_INFO`, `DBUS_SESSION_BUS_ADDRESS`, `DOCKER_HOST`, `TMUX` or `TMUX_PANE` from the
node, on every system. Where the system can refuse the connection, it also refuses unix sockets the agent was not given: on macOS all of
them except its own mesh socket, its worktree, its private temp folder, the name resolver (`/var/run/mDNSResponder`) and what you list in
`sandbox.sockets`; on Linux (Landlock ABI 6 or newer) only abstract sockets, so an agent that goes looking can still find an ssh-agent or
docker socket in a file path there. `agora doctor` says which of these this machine does, and which variables the node scrubs and which you
pass.

**`git` over ssh (`git@host:...`, `ssh://`) does not work in a confined agent**: it reached the keys only through the ssh-agent, and a
confined agent cannot read `~/.ssh`. Push from your own checkout, or use an https remote with a credential you give that agent on purpose.
If you accept that the agent can then sign with every key in your ssh-agent, the steps are:

1. In the node's policy file add `sandbox.pass_env = SSH_AUTH_SOCK` (names only; one that is not a plain name, for example `SSH_AUTH_SOCK=/x`
   or `SSH_*`, makes the whole list empty).
2. On macOS also add the socket itself: `sandbox.sockets = /private/tmp/com.apple.launchd.XXXX/Listeners` (the value of `SSH_AUTH_SOCK` in
   your own shell; the name changes at every login, so this line needs updating). On Linux nothing more is needed under Landlock today.
3. Restart the node (`agora node run`): the policy is read only at start. `agora doctor` then lists `SSH_AUTH_SOCK` as passed.

`sandbox.sockets` entries are absolute or `~/` paths (a relative path is dropped); one below the node's own folders is ignored. A socket
listed there is reachable by every confined agent on that node. Agents of the ACP kinds reach their client through pipes, and the mesh
bridge (`agora mesh stdio`) reaches the agent's own mesh socket, which is always allowed; neither needs a line here. An agent hosted by an
adapter (the ACP kinds) never inherits these variables and `sandbox.pass_env` does not change that. A dev server or a tool of
yours that talks to a unix socket outside its worktree or temp folder fails on macOS until you list that socket in `sandbox.sockets`.

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

A confined agent on macOS writes only the parts of its checkout's shared `.git` a commit needs (`objects`, `refs`, `logs/refs`,
`packed-refs`, its own worktree admin folder); the shared `config` and `hooks` are read-only, so it cannot plant a hook or setting that
your git runs in the main checkout. Git run *inside the agent's worktree* is another matter: the agent can still rewrite that worktree's
`.git` file and its admin folder (`commondir`, `config.worktree`), so treat git there as running the agent's settings (the node's own
git switches off what it can, see T8b in the threat model). The node opens the shared folder only when its
`worktrees/<name>/gitdir` points back at the worktree and `.git` is a plain file, so a `.git` file the agent redirected or replaced with a link opens nothing new on the next start.

Measured with git 2.56 in a node-made worktree: status, add, commit, log, new branches and tags, checkout, stash, fetch, `pack-refs` and
deleting a branch work (a delete warns that it could not drop the branch's config section). Refused, because they write the shared
`config` or create folders in the shared `.git`: `git branch --set-upstream-to`, `git config`, `git remote add`, the agent's own
`git worktree add`, and `git gc` (so git's automatic gc cannot run either). Run those from your own checkout.

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

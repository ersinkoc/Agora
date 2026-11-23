# Sandbox recipes

Lines for the node's `<data>/policy` that let real tools work under `sandbox = worktree` (ADR-0047). Every line widens what **every**
confined agent on that node may touch, so add only what you use. Paths are absolute, or `~/` below the home folder (PR #244; before it,
write the full path). A path that does not exist is skipped; `agora doctor` names every policy path the node drops or cannot find yet.

What is never a default, and why: `~/.gitconfig` and `~/.config/git` can hold credentials (`url.*.insteadOf` with a token, credential
helper settings), so reading them is the owner's choice, not the node's.

## Git inside a confined agent

At each confined launch the node writes a small Git configuration to `<data>/git-config/` (normally
`<AGORA_HOME>/data/git-config/`). Git uses that file for its global settings, with `GIT_CONFIG_GLOBAL` pointing to it
and `GIT_CONFIG_NOSYSTEM=1`. The node grants read access to the one file for that launch. The grant allows opening the
file while keeping directory listing, sibling reads and writes denied. If the agent's ordinary grants reach that folder,
or the node cannot prepare the private file, the node skips the generated settings. Each file lasts as long as its launch:
the node removes it when the agent ends, is stopped or crashes (a restart writes a new one), and when a start fails after
the file was written. Files of launches a node never ended (it was killed, or the machine stopped) stay in that folder. See [ADR-0078][git-config] and
[ADR-0098][git-config-location].

The node copies these keys when set in your own global configuration: `user.name`, `user.email`, `init.defaultBranch`,
`core.autocrlf`, `core.ignorecase`, `core.filemode`, `pull.rebase`, `push.default`, `push.autoSetupRemote` and `color.ui`.
It reads them with `git config --global --no-includes`, so settings defined through `include` are left out. It skips
empty or long values and values with control characters. Keys other than the two identity keys also require values made
up of letters, digits, `_`, `-` and `.`. The node clears `GIT_CONFIG_COUNT` and `GIT_CONFIG_PARAMETERS` overrides
when it supplies the file. Other inherited variables such as `GIT_SSH_COMMAND` and `GIT_EXEC_PATH` are outside this setting.

If you list `~/.gitconfig` in `sandbox.read`, the agent uses your configuration as before, including settings such as
`safe.directory` for a checkout owned by another user. The node leaves generated files on disk after each launch and
retains copies in legacy locations for agents still using them. `agora doctor` warns when it cannot find a
`user.name` or `user.email` to copy on a machine that confines agents.

[git-config]: ../adr/0078-a-confined-agent-gets-a-git-configuration-the-node-wrote.md
[git-config-location]: ../adr/0098-generated-git-configuration-belongs-to-the-node.md

## Unix sockets, the ssh-agent and `git push` over ssh (ADR-0083)

A confined agent does not get `SSH_AUTH_SOCK`, `GPG_AGENT_INFO`, `DBUS_SESSION_BUS_ADDRESS`, `DOCKER_HOST`, `TMUX` or `TMUX_PANE` from the
node, on every system. Where the system can refuse the connection, it also refuses unix sockets the agent was not given: on Linux (Landlock ABI 6 or newer) only abstract
sockets, so an agent that goes looking can still find an ssh-agent or docker socket in a file path there. **On macOS that second layer refuses every unix
socket outside the agent's allow-list** (its mesh socket, worktree, private temp folder, `/private/var/run/mDNSResponder` and your `sandbox.sockets`). A macOS developer measured
on a real Mac (macOS 27.2.1, 2026-11-17) that the agent's own socket connects, another one is refused and the name resolver works; a confined `git commit` runs. **Not measured yet** with real agents:
the other system sockets a tool may need (DNS in every tool, `git` over https, `cargo`, `npm`, Claude Code, Codex under `worktree`); a tool that needs one the list does not name fails, and you add it with `sandbox.sockets`. `agora doctor` says which of these this machine does, and which variables the node scrubs and which you
pass.

**`git` over ssh (`git@host:...`, `ssh://`) does not work in a confined agent**: it reached the keys only through the ssh-agent, and a
confined agent cannot read `~/.ssh`. Push from your own checkout, or use an https remote with a credential you give that agent on purpose.
If you accept that the agent can then sign with every key in your ssh-agent, the steps are:

1. In the node's policy file add `sandbox.pass_env = SSH_AUTH_SOCK` (names only; one that is not a plain name, for example `SSH_AUTH_SOCK=/x`
   or `SSH_*`, makes the whole list empty).
2. On Linux nothing more is needed (the socket rule is not enforced for file-path sockets there). On macOS the socket itself has to be listed: `sandbox.sockets = /private/tmp/com.apple.launchd.XXXX/Listeners` (the value of `SSH_AUTH_SOCK` in your own shell).
3. Restart the node (`agora node run`): the policy is read only at start. `agora doctor` then lists `SSH_AUTH_SOCK` as passed.

`sandbox.sockets` entries are absolute or `~/` paths (a relative path is dropped); one below the node's own folders, also through a symbolic link, is ignored. A socket
listed there is reachable by every confined agent on that node. Agents of the ACP kinds reach their client through pipes, and the mesh
bridge (`agora mesh stdio`) reaches the agent's own mesh socket, which is always allowed; neither needs a line here. An agent hosted by an
adapter (the ACP kinds) never inherits these variables and `sandbox.pass_env` does not change that. A dev server or a tool of
yours that talks to a unix socket outside its worktree or temp folder will need that socket listed in `sandbox.sockets` once the macOS rule is switched on.

## A confined agent's login (ADR-0084)

On macOS a confined Claude Code says "Not logged in": its subscription login is in the login Keychain (`~/Library/Keychains`), and the sandbox
rightly does not open that folder, which holds every password you have. The Keychain is not used here at all. Give the agent a token made for
it instead, kept by the node (a file in the node's private data folder, never sent to the hub), on any system:

1. Make a token for the agent. For Claude Code: run `claude setup-token` (needs a subscription; long-lived), or use an API key
   (billed per use, not against a subscription). For Codex: an API key. Make it for this purpose, so you can revoke it on its own.
2. On that machine: `agora node secret set CLAUDE_CODE_OAUTH_TOKEN` (or `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`). Paste the value at the
   prompt (nothing is shown) or pipe it in; never put it in the command line. `agora node secret list` shows names only.
3. In the node's policy file add one line per kind: `credentials.claude-code = CLAUDE_CODE_OAUTH_TOKEN` (or `credentials.codex = OPENAI_API_KEY`),
   then restart the node (the policy is read when the node starts). `agora doctor` says what the node would give each kind, by name.

A token already in the node's own environment (a service environment) is used when no stored secret has that name, but only for a kind whose
policy line lists it. **This changed with ADR-0084:** a credential variable in the node's environment no longer reaches every agent: the
names Claude Code and Codex read are removed from every agent that is not listed for them. If you relied on that, add the policy line.

Three limits of the protection around the store itself. (a) With `sandbox = off` an agent is not confined and can read `<data>/secrets` like any
file of yours; the store protects nothing from an unconfined agent. (b) The node refuses to start a confined agent whose read or write grants
reach the secrets folder (`agent.sandbox_unavailable`, reason `secrets_reachable`), but only **at start** and only once a secret is stored
(on Linux a grant that reaches the data folder is refused whether or not a secret is stored, reason `protected_path_reachable`, #1114):
a secret you add while an agent with a broad grant (a read grant on your whole data folder or above) is already running is not caught, so
restart such agents after storing a secret. (c) Names Windows cannot use as file names (`CON`, `NUL`, `COM1`, ...) and a name that differs from
a stored one only in capitals are refused as secret names on every system.

What this cannot do: it gives the agent a token, not your login. A confined agent can send the token anywhere (the `worktree` level has no
network rules), can print it into its terminal, and a process of the same user may read another process's environment (`/proc/<pid>/environ`
on Linux, `ps eww` on macOS: not measured). Interactive login inside a confined agent does not persist (the Keychain write is denied, which
is the point). Two logins of one kind on one machine are not solved by this.

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
this machine (a local model on `localhost`), to unix sockets in its allow-list (its own mesh socket, its worktree, its private temp folder and what you list in `sandbox.sockets`); DNS, the internet, other
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

Landlock can only allow, so the node keeps its own folders closed by refusing instead: a confined start is refused
(`agent.sandbox_unavailable`) when a folder an agent is given contains the node's control token (`control_token_reachable`) or any other
folder of the node's own, its data folder (stored secrets, the session ledger, chat transcripts) or its manifests (`protected_path_reachable`,
#1114). Every agent may read `/usr`, `/lib*`, `/bin`, `/sbin`, `/etc`, `/opt`, `/proc`, `/sys`, `/run` and `/var` and write `/tmp`, so a
node whose `--data` or `--manifests` lies under one of them starts no confined agent; keep the node's folder under your home (`~/.agora`).
`agora doctor` names the folder. A `sandbox.read` or `sandbox.write` entry *inside* the node's data folder is not refused on Linux and opens
what it names: do not add one.

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

**Not supported confined on Windows either: a Node.js program starting a child with piped input or output** (#895). Measured with Node 26:
`child_process.spawnSync` fails with `EPERM` whenever a stdio stream is a pipe (the default); with `stdio: 'ignore'` or `'inherit'` the
child starts. The asynchronous `spawn` creates its pipes the same way (not measured separately). libuv creates each such pipe as a named pipe and opens the child's end by name. Windows gives a named pipe
created without explicit security its own default permissions (full access for the owner, the system and administrators, read for
`Everyone`), and a restricted token passes only what its restricting groups (the agent, `Everyone`, `Users`, the logon session) are granted,
so that end cannot be opened for writing. **Consequence: a Node-based agent (Claude Code, Gemini CLI and others) that starts a tool or an MCP
server with piped stdio, Node's default, cannot do so when confined on Windows.** Real agents were not measured. A Rust stand-in program
(Rust's standard library, piped stdio included) and `cmd` start children confined, so Rust agents such as Codex are expected to work, but no
real Codex run was measured. Fixing the Node case needs another confinement mechanism, a decision for ADR-0050.

**Adapter and ACP (structured) agents cannot be confined on Windows** (#897). Windows confines terminal (PTY) agents only, so with any
`sandbox` level other than `off` such an agent is refused before anything is created: `agent.sandbox_unavailable` with
`reason = unsupported`. Run those kinds on a node with `sandbox = off` until this is solved.

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
- **A confined dev server cannot share a port with a wildcard listener.** If another program already listens on `0.0.0.0` on that port,
  the agent's `listen(127.0.0.1)` fails with `EACCES` (an unconfined process may bind it next to the wildcard listener). Use a free port.
- **rustup needs `~\.rustup` itself** (`could not create home directory ... (os error 183)` when only one toolchain folder is listed).
- Not working yet: `cargo build` (`Access is denied` starting `rustc.exe`); npm's cache (`%LOCALAPPDATA%\npm-cache`) is too large to list per agent.

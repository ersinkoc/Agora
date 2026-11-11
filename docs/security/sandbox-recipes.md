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
this machine (a local model on `localhost`) and to unix sockets below the node's mesh folder and its writable folders; DNS, the internet and
other machines on the LAN are refused, and it cannot listen on a TCP port. Package managers and remote model APIs therefore fail by design;
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

Confinement is switched off until ADR-0050's design is settled; the windows machine adds its lines here.

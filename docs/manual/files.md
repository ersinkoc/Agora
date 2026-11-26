# Files: where each file lives and how it is read

Agora keeps a machine's files in one folder, the **Agora folder**: `$AGORA_HOME` when it is set, else `.agora` in the home folder
(`~/.agora`; on Windows `%USERPROFILE%\.agora`). Every path below is inside it.

## Files a person, or an agent for them, may write

| Path | What it is | How it is read |
| --- | --- | --- |
| `themes/NAME.json` | a theme for the terminal client (`agora manual themes`) | every `*.json` directly in the folder, at most 200, by name; a file over 256 KiB is refused before it is read; read again while in use when it changes |
| `layouts/NAME.json` | a saved layout of the terminal client (`agora manual layouts`) | at most 50 listed; a name of at most 60 characters; a file over 256 KiB is refused before it is read |
| `manifests/NAME.toml` | an agent kind added on this machine: how to start it and read its state | read when the node starts, and a new or changed file again before it lists kinds or starts an agent; a file that does not parse is skipped. `agora agent kinds` lists what it read, `agora doctor` names a file it skipped, `agora agent explain` tests a screen rule |

After writing a theme or layout, tell the person how to choose it: `t` in `agora tui` for a theme, **Use layout “NAME”** in its
commands for a layout.

## Files the terminal client keeps

| Path | What it is |
| --- | --- |
| `tui-theme` | the theme in use: one line, `terminal`, `built-in ID`, `file dark NAME.json` or `file light NAME.json` |
| `tui-layout.json` | the arrangement in use: `{ "version": 1, "layout", "focus", "zoom" }` |

The client writes these when the person chooses. Anything else in `tui-theme` means no theme.

## Files of the machine's owner

Do not change these unless the person asks, and show them the change first.

| Path | What it is |
| --- | --- |
| `machine.toml` | what this machine runs (its roles, which interface opens, the hub's address and folder); written by `agora` the first time; never a secret |
| `data/policy` | what the owner allows on this machine: `key = value` lines, `#` for comments. The hub can never widen it. The node reads it only when it starts, and a line it does not understand makes it refuse launch options that skip permissions. `agora doctor` shows how it was read |
| `data/trusted-devices` | the devices whose signatures this machine accepts; change it with `agora node trust` and `agora node untrust` only |

## Files only Agora writes

Never edit these. They hold keys, certificates and the node's own state.

| Path | What it is |
| --- | --- |
| `node.sock` | the node's control socket (`agora manual scripting`) |
| `data/` | the node's data: its ledger of agents, chat history, secrets (`agora node secret`), the mesh's ids |
| `link/` | this machine's certificate for the hub, from `agora join` or `agora node enroll` |
| `hub/` | the hub's database and keys, on the machine that runs the hub; copy it with `agora hub backup` |
| `logs/hub.log`, `logs/node.log` | logs; one over 5 MiB at a start is renamed to `.1` |
| `worktrees/` | git worktrees made for agents (`agora agent start KIND --worktree`) |
| `agents/` | programs installed by `agora agent install` |
| `adapters/` | adapters installed by `agora adapter install` |
| `m/` | each agent's own mesh socket (`agora manual mesh`) |

Secrets never go in a file you write: `agora node secret set NAME` reads the value from standard input and keeps it on this machine.

## Kept by the web window, not in the folder

The web window keeps these in the browser. Each can be exported to a file and imported again; the file is the format of its topic.

| What | Export file | Topic |
| --- | --- | --- |
| themes | `theme.json` | `agora manual themes` |
| layouts | `layout.json` | `agora manual layouts` |
| keybindings | `keybindings.json` | `agora manual keys` |
| plugins | a package file | `agora manual plugins` |

## Example

Add a theme for the terminal client and check it:

```sh
mkdir -p "${AGORA_HOME:-$HOME/.agora}/themes"
cp slate-teal.json "${AGORA_HOME:-$HOME/.agora}/themes/"
agora tui
```

In `agora tui`, `t` lists the theme. A file that cannot be used is listed as "can't be used", with every reason below it.

# Changelog

Each release lists what a person using Agora can now do. The reasons are in `docs/adr/`; the state of every task is in `docs/backlog/`.

## Next

### Closed agents
- **Remove for everyone:** a closed agent's summary can ask the hub to forget it (ADR-0053): it disappears from every window, the TUI and the desktop app. Its earlier events stay in the hub's record and its tokens and cost still count. An agent that still runs cannot be forgotten. "Remove from this list" still only hides it in this browser.
### Safety
- A confined agent cannot be started in a folder that holds a secret folder (such as `~/.config`, which holds the `gh` and `gcloud` sign-ins).
- Review in a folder that is part of a larger checkout shows only that folder's changes.
- An answer to a request that comes after the request expired tells the agent "denied" instead of leaving it waiting.
- Removing a working copy whose commits are on no branch is refused instead of losing them, unless forced.

### Agents and machines
- Stopping an agent that runs through an adapter also ends the adapter's program; before, it kept running unseen.
- A resume that fails no longer leaves the stopped agent reachable by other agents.
- When the hub confirms events older than the ones a machine kept while it was offline, the machine keeps the rest instead of deleting them (or stopping its link).

### Terminals and adapters
- Opening a terminal that is already running shows its later output in the program's own colours, and paste and arrow keys work as the program expects.
- An adapter that stops reading its input no longer freezes its machine; the request to it times out.
- An ACP agent that prints a line that is not text no longer looks closed.

## 0.3.2

### The window
- The tabs you opened, the one in front, one-or-side-by-side and the panels you hid are kept when you go to Full size, Machines or This device and back, and across a reload (in this browser).
- Agents that have closed no longer sit among the running ones. They wait in a **Closed** section (when they ended, newest first) until you clear them: one by one (✕) or **Clear closed**. Opening one shows what it left behind instead of a dead terminal: how it ended (exit code), when and for how long it ran, the folder and branch, the files it changed (with Review, History and Files), what it used. Clearing is this browser's; the hub keeps its record.

### Terminal client and desktop
- `agora tui` is a client of your hub: machines (with the agents' memory) and their agents on the left, the selected agent's live terminal on the right in its own colours (16, 256 and 24-bit, bold, italic, underline, wide characters, box drawing), a status line, and answers to approval requests.
- A terminal you open keeps its colours on the first screen and the node tells its size. The desktop app's title follows the hub's page, and CI builds an unsigned Windows installer for testing.
### Fixes
- Listing a folder or watching files cannot be pointed outside a worktree by swapping a folder for a link while it runs.
- Windows shows a folder once in the start window. A node that cannot reach its hub writes why in its log.

## 0.3.1

### The window
- **Your hub and machines** (status bar, or "Machines" in the command palette): the hub in the middle and every machine around it with live links: online or not, system, the version it runs, agents and terminals, memory, cost, when it was last heard, and its agents as chips that open them.
- Terminal text size buttons (A−, A+), remembered in this browser. Programs' colours that would be unreadable on the pane are lifted to a readable contrast.
- An agent waiting for your OK shows Allow and Deny right above its terminal, as well as in Details. A machine's row shows the memory its agents use (from the machine, every 30 s at most when it changes a lot). "Needs you" counts an agent once.
- Choose the colour scheme in the title bar: follow the system, dark or light. It is remembered in this browser, and terminals repaint at once. Terminal colours are readable in both schemes.
- Machines, This device and Audit share the window's title bar. Alt+1…9 picks a tab, Ctrl/⌘+B hides the agent list. A long "shared folder" label no longer pushes an agent's name out of the list.
- The window is laid out like an editor: the agents as a tree on the left (machine, folder, agent), the terminals you watch as tabs in the middle (or side by side), details in tabs on the right, a status bar at the bottom. Search and commands are one click or Ctrl+K from the title bar.
- Starting an agent is one window: pick the machine, then a known folder in one click (or type a path), then the agent; the resolved folder is shown before anything starts.
### Starting agents
- A start that fails after creating a separate working copy cleans up that copy and its unchanged branch. Branches with new commits or another working copy stay available; starts in existing folders preserve those folders.

## 0.3.0

### Agent state
- Claude Code and Codex folder-trust prompts are recognized at 24x20 as well as 120x40. Short Codex terminals may still hide the dialog's options; enlarge the terminal to see them.
### Interface language
- Choose Türkçe, English or your browser's language on sign-in, Home or a focused agent. The choice is remembered in this browser; switching keeps terminals and unfinished forms open.
### Shared board
- When a project starts being shared by a second machine, the tasks and file leases its first machine kept locally move to the shared board: no file has two holders, and each moved task's owner is told its new id (ADR-0039 amendment B).
- After a hub restart, incoming local work preserves the shared board's saved tasks, dependencies and live file leases.

### Selected-folder fixes
- Machines sharing a host name stay distinct, shared-project counts use enrollment IDs, and agent panes expose their real copyable routing addresses.
- An isolated copy preserves a selected untracked/ignored subdirectory and refuses restored symlinks that could select another folder. Workspace diffs stay inside the selected directory.
- Nested integration files stay out of Git status/staging; resumed/restarted agents become visible as running again with their current state and session.
- Registered project names win over same-named local folders. Filtered panes stay hidden when closed, and a disappearing project filter clears itself. Turkish help lists the complete identity workflow.

### Working folders
- Start agents in a chosen machine/project/folder; Git and separate working copies are optional. The CLI defaults to its current folder (`--cwd` to choose another, `--worktree` for isolation).
- Several agents can share a folder with separate sessions and integration files; shared changes appear once.
- Explicitly create `.agent-project.json` (`agora project init --cwd PATH`) and carry it across machines to group project copies. Source files are not synchronized.
- New directory launch needs an unrestricted owner device trusted on the node. Older clients keep their existing worktree launch behavior; fixed-file MCP manifests need explicit worktree mode.

### Machines
- Matching project folders selected on machines that are already online use one board for new tasks and file leases.
- A hub, a Linux node, a macOS node and a Windows node ran together on one network: terminals of all three in one window, and agent-to-agent messages in every direction (`docs/backlog/lan-test-findings.md`).
- The desktop app checks the hub against the certificate you gave it before remembering it, says why it cannot connect, and reaches a hub by its IP address; a bundled macOS app can now reach a hub on the local network. `agora tui` reaches a hub by IP too.
- A project is found by its id before its name, and a name two projects share is refused.

### Shared project boards
- Distinguish same-named projects by their full identity and expandable machine/folder details. Leases from selected-folder sessions appear on their project's board.

### Confining agents
- macOS: a confined agent can read the folders it was given (it could not read its own worktree before).
- macOS and Windows: a confined agent gets a temp folder of its own (`TMPDIR`, `TEMP`, `TMP`). `sandbox.read` and `sandbox.write` accept `~/`.
- A confined launch in a filesystem root, the home folder, a secret folder (`.ssh`, `.aws`, ...) or the node's own folder is refused.
- Recipes for the folders real tools need (git config, cargo, npm, compilers): `docs/security/sandbox-recipes.md`.
- Windows confinement is written (ADR-0050) and tested on real Windows, but switched off: a real-machine check showed the first design lets a folder with a broad group permission through, and a replacement design is being measured.
- Codex's "trust this folder" screen is now shown as needing you.
- macOS: `sandbox = worktree-offline` works: no internet or DNS, but localhost and the agent's own mesh socket stay reachable. Linux and Windows still refuse that level.
- Known gap: a confined agent can still connect to unix sockets it can name (a tmux server, the SSH agent); see `docs/security/threat-model.md`.
- The shared task board no longer vanishes when one of two machines drops: while it holds unfinished work the other machine keeps using it, and a machine that restarts does not inherit its dead agents' claims.
- A refused lease says when the holder's lease ends.
- A manifest added while the node runs is listed and can be started without restarting the node.

## 0.2.1

### Security fix
- A team member could add a passkey and then sign in with it as the owner. Passkeys now belong to the person who added them. If you use team mode with 0.2.0, update, and review `agora hub audit export` for `sign_in` rows with `passkey` from people you did not expect. A hub with no team members was not affected.

## 0.2.0

### Teams
- A hub can have several people. `agora hub user add NAME --role owner|member|viewer` gives each a sign-in code and a role; a viewer can only
  read, a member works in the projects they were given (`agora hub user grant ID NODE/PROJECT`), the owner sees everything.
- A member never receives another project's snapshot, events or terminals, and the hub refuses their commands about anything else.
- A machine can limit what a person's device may sign (`device.<id>.projects = a, b` in its `policy` file); the hub cannot change it.
- An append-only audit trail of every routed command, sign-in and change to the team: read it in the app (Activity) or with
  `agora hub audit export`. The owner sees it; nobody can edit or delete a row.
- Upgrading a hub keeps its data: the database is migrated in place (tested from the 0.1.0 schema).

### Safer agents
- Agents can be confined (`sandbox = worktree` in a machine's `policy`): they write only in their worktree, its git folders and a temp
  folder, and read only the system, their program and what you list. Linux (Landlock) and macOS (Seatbelt); a machine that cannot
  confine refuses to start the agent rather than run it unconfined. `agora doctor` says whether a machine can.
- Agent packages can ask for extra folders; the machine's owner decides per kind (`sandbox.trust_manifest`).

### More places to use it
- Windows machines run agents in real terminals (ConPTY) with a process view.
- The web app installs to a phone's home screen, and a notification for a waiting approval links straight to it.
- A terminal client (`agora tui`): machines, agents, terminals, conversations, approvals, starting and stopping.
- A desktop app (Tauri shell, `apps/desktop`): it trusts only the hub's own CA, which you confirm once by its fingerprint
  (`agora hub fingerprint`). Built in CI; unsigned until it has signing accounts.
- The web layout mirrors for right-to-left languages (no right-to-left translation ships yet).

### Adapters and updates
- `agora adapter install|update|list|remove`: signed adapter packages from a registry, checked against a root key you pin.
- `agora update`: replaces the program from a signed release. It does not yet hand running agents to the new process.

### Fixes and tests
- Real-browser end-to-end check (sign-in, terminal, chat agent, approval) and visual regression baselines for the shared components.

### Known limits
- Windows agent confinement is not available (proposed in ADR-0050); a Windows machine with a sandbox policy refuses to start agents.
- `worktree-offline` (no network) is always refused: network rules are not built.
- Per-person limits on a machine bind what is signed; commands the owner leaves unsigned rely on the hub's checks.

## 0.1.0
First public release: nodes, hub, web app, mesh, approvals signed by the person, usage and cost, SSH install, backup and restore.

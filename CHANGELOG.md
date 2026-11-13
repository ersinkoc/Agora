# Changelog

Each release lists what a person using Agora can now do. The reasons are in `docs/adr/`; the state of every task is in `docs/backlog/`.

## Next

## 0.3.8

### Getting started
- Type `agora` and it works (ADR-0062, first rung). The first time it asks one question (start Agora here, just this machine, or join; joining from here comes later) and keeps the answer in `~/.agora/machine.toml`. Then it starts this machine's hub and node in the background when they are not running, connects the node to the hub by itself (no code to copy), and opens the terminal client signed in. `agora web` opens the browser signed in; `agora --ui web|tui|none` opens that for one run, and with `--save` makes it what plain `agora` opens. Closing or killing the window leaves the hub, the node and the agents running. `agora status` shows what runs and where its logs are (`~/.agora/logs`), `agora down` stops it. `agora --help` prints the usage. See `docs/getting-started.md`.

### Signing in
- A browser stays signed in: the session token is kept for 30 days (it was 12 hours and was forgotten when the tab closed) and every use extends it, so you are not asked for a code again each time you open the window. Signing out ends it at once.
- A screen password (ADR-0067): `agora password set|clear|status` on the hub's machine, or This device in the window. A sign-in from another machine then needs its code and the password; the browser on the hub's own machine is never asked, and a passkey replaces the password on its device. Wrong codes and passwords are limited per address and for the whole hub with a lockout that grows up to an hour (never more than a minute on the hub's own machine), and every attempt is in the Activity page with its address and browser. Setting or changing the password signs out every other window.
- **Lock** in the title bar signs the window out; agents keep running. This device can also sign the window out the same way after 5 to 60 minutes idle (off by default). Adding a passkey from another machine needs the screen password too, and setting or changing the password removes passkeys added from elsewhere without it. See `docs/ux/screen-password.md`.

### Layouts (ADR-0068, rung 1)
- The window's centre is a layout you arrange: several terminals and chats side by side. The Layout menu in the title bar gives one pane with tabs, two side by side, two stacked, 2 × 2, 3 × 2 and one large with three small, filled with the agents you watch; panes split right or down, resize by dragging their dividers (or with the arrow keys), drag onto each other (beside, or as a tab), swap and move from the pane menu, and maximize and restore. A pane can show an agent, follow what needs you next (each such pane shows a different waiting agent, oldest first) or follow the agent selected in the list, or show Needs you, Changes, Shared tasks, Messages or Spent. Layouts are saved by name, exported and imported as JSON (`docs/ux/layout.md`, version 1; one from a newer version is refused with a message), and kept across reloads; the tabs you had open become the first layout. Only panes on screen hold a terminal stream; on a phone the panes are tabs of one group.

### Agents
- Agents know who else is here. A new mesh tool, `workspace_status`, tells an agent how many agents work in its workspace and project, on which machines (other machines through the hub when the project is shared; marked partial when the hub cannot be asked), who is working, waiting for approval or idle, what each is doing in a few words (its claimed task, or the kind of its latest step, or its state; never its prompt or screen), how many files each has changed, who holds which paths and which tasks are open, with a one-line summary. `agents_list` rows carry the same facts. When an agent on the same machine starts or ends, the other agents of the project get a short `presence` note in their inbox (at most 10 a minute). Agents are told to check `workspace_status` when they start and before editing shared files.
- A machine now remembers its agents across its own restart (ADR-0060): kind, folder, project, the agent that started it, its own session id once it reports one, and whether it ended. The record stays on that machine, readable by its owner only, holds no prompts, terminal contents or secrets, and forgets an agent after 14 days. An agent id is never given twice, so an agent that was running when the machine stopped keeps its place in the window.
- After a machine restarts, the agents that were running on it are reported as interrupted and can be resumed under the same id, in the same folder and project, continuing their own conversation (`agent.resume`, signed by a trusted device like a start). Claude Code gets a session id chosen by the machine at start (`--session-id`), Codex's is read from its own session file when that cannot be confused with another one, structured agents report theirs. An agent without one can be started again in its folder. A resume is refused when the folder is gone or changed identity, and a helper comes back only while the agent that started it runs, without its old grants. A machine's owner can let agents of chosen kinds in chosen folders come back by themselves (`resume = auto`, `resume.kinds`, `resume.folders` in the node's policy file); the default is to ask.
- The window lists agents that stopped with their machine under that machine ("Stopped with the machine") with **Resume** (continue its conversation) or **Start again** (a new one in the same folder); the closed-agent page says what happened and offers the same. In the terminal client such an agent's row says so, `u` resumes it and `U` starts it again.
- An agent whose process kept running after its machine's node was killed is shown as still running from before the restart: it can be stopped, and only then resumed, so two processes never continue one conversation.
- A machine whose node runs inside a Claude Code session no longer passes that session's markers (`CLAUDECODE`, `CLAUDE_CODE_*`) to the Claude Code agents it starts, which made them save no transcript.
- A new machine offers every shipped agent (Claude Code, Codex, WrongStack, opencode, Antigravity) without copying any file: the `agora` program now carries the shipped agent descriptions built in. A file in the node's manifests folder with the same kind replaces the built-in one, a new kind adds one, and a file that cannot be read is named by `agora doctor` while the built-in kind stays. A Shell (your own shell: `$SHELL`, or `%ComSpec%` on Windows) is built in too, so `agora agent start shell` and the start window work on a fresh machine. `agora doctor` lists every agent with where it came from (built in or the folder) and the program it looks for, and says when this machine cannot find that program in its PATH.
- The start window and the terminal client say why an agent is greyed out: the machine cannot find its program in its PATH, so install it there or start the node from a terminal where it runs. The terminal client now lists those agents too instead of leaving them out.

### Trust
- The command that makes a machine trust your device says what to do when `agora` is not found on that machine (a downloaded release is not on PATH): run it in the folder of the agora program (`./agora`, or `.\agora.exe` on Windows). `agora node trust` no longer tells you to restart the node: the node picks the change up by itself.

### Launch options (ADR-0059)
- Agent manifests can declare launch options (`[[launch.option]]`, manifest engine 2), such as a flag that lets the agent act without asking. The shipped manifests declare the flag of the version it was checked on: Claude Code `--dangerously-skip-permissions` (2.1.323), Codex `--dangerously-bypass-approvals-and-sandbox` (0.167.1), WrongStack `--yolo` (1.6.0) and Antigravity's `agy acp --dangerously-skip-permissions` (1.5.6). The protocol carries the choice (`launch_options` on `AgentStart`, `AgentLaunch`, `AgentKind`, `AgentStarted` and the agent snapshot) and the error `agent.option_forbidden`; the node does not use them yet.
- Machines: the node starts an agent with the launch options its start chose, on the agent's command line (terminal agents) or on its ACP server command (structured agents), for the agent's whole life, restarts and resumes included. The owner decides per machine in the node's policy file with `skip_permissions = forbid | allow | confined_only` (default `allow`; any other value forbids; the hub cannot change it); a start the policy refuses is refused before anything starts (`agent.option_forbidden`), and `confined_only` needs the agent's confinement (`sandbox = worktree`). Through the hub only a signed `agent.launch` may carry options and its signature covers them (`directory-action-v3`), so the hub can neither add nor remove one; a routed `agent.start` with options is refused and a grant never covers one. Agents an agent starts (`agent_start`) never get an option. The hub keeps the options in the agent snapshot and audits every start with one that skips permissions; nodes offer the capability `agent.launch_options` and the hub sends options only to nodes that offer it.
- Windows: the start window lists the agent's launch options under "How it acts". "Ask before acting" is always the default and "Don't ask" is never preselected: choosing it asks "Let the agent act without asking?" first, with Keep asking as the main button, and the browser signs the choice with the start (with the ids you confirmed as a separate signed list, which the machine requires; ids that are not `[a-z0-9-]` are not offered). An agent started that way carries a "Does not ask" badge in the tree, on its tile and in its pane header for as long as it is listed. The terminal client asks the same two questions when it starts an agent (any key but `y` keeps asking) and shows "does not ask" on the agent's row, in the split view's tree and in its title.
- The node itself decides which launch options skip permissions: the ids the user confirmed are sent and signed as a separate list (length-prefixed, `directory-action-v3`), the node refuses an unconfirmed one (`agent.option_unconfirmed`) and any id outside `[a-z0-9-]{1,32}` (`agent.option_invalid`), so a hub that shows it to the window as harmless cannot get it signed unknowingly. A policy file line the node does not understand (a typo such as `skip_permission = forbid`), or an existing policy file it cannot read, forbids such options and is reported at start (`node.policy_not_understood`); quoted values (`"forbid"`) are read as unquoted ones. A manifest that declares a known skip flag (`--dangerously-*`, `--yolo*`) without `skips_permissions = true` does not load.

### Machines
- A link that goes silent while the connection stays open (a frozen hub or node, a laptop that slept, a Wi-Fi switch, a NAT that forgot the connection) is now noticed. A node dials the hub again when it has heard nothing for 25 s; the hub shows a machine offline when it has heard nothing from it for 25 s (an idle machine keeps its link with a ping every 10 s and is never dropped). Before, the window showed a frozen machine as online, and nodes never dialled again, for as long as the silence lasted.
- `agora agent start` in a folder whose project identity changed (after `agora project init`) says so and names `--accept-project-id-change`, instead of "The request was not valid (field: directory.identity_changed)".
- `agora node trust` no longer tells you to restart the node: the node reads its trusted devices again when the file changes.

### Window
- The window notices a hub that stopped answering (it pings every 15 s and starts over when nothing comes back within 10 s) and shows "Hub unreachable". It comes back within 5 s of the hub, sooner when the browser is back online, the tab becomes visible or the window gets the focus, and the banner has a "Retry now" button. Before, it waited up to 30 s between attempts.

### Agents
- A `delivery_failed` notice for a message to another machine names the message that failed: its `reply_to` is the `message_id` that `send_message` answered with, so an agent with several messages in flight knows which one did not arrive. Nothing is resent.

## 0.3.7

### Agents
- When a structured agent fails or says nothing in a turn, the notice carries the lines of its stderr that name an ERROR, FATAL or WARN level or a failing HTTP status (opencode 2.x logs a failure only that way) when there are any, so an agent that also logs at INFO (opencode) no longer buries the reason under routine lines. A structured agent now has 30 s to initialize and 60 s to open a session (it was 10 s each; opencode 2.x needs up to 12 s, which ended the start as a bare `internal` error).

### Machines
- A machine whose node ran before it was enrolled now links to the hub. Before, the node sent the hub the events it had recorded under the name `local`, the hub dropped the connection without a word, and the node only logged "connection reset", forever. Now the node sets those events aside whenever it starts under another name than the one its log was recorded under (enrolled after running as `local`, enrolled again under a new name, or running un-enrolled again) (the old log is renamed to `events.<old name>.<time>.log` beside the new one, never deleted, and the node says so when it starts), and a hub that refuses events naming another machine closes the connection with the code `protocol.producer_mismatch`, which the node logs (`hub link: Refused("protocol.producer_mismatch")`).
### Modes and models
- A confirmation to let an agent act without asking stays with the agent it was opened for: switching to another agent closes it, and "Switch anyway" can no longer reach a different agent (two machines' `agent-1` included).
- A mode or model the agent did not report as current shows "Not reported" instead of preselecting the first one in the list.
- Asking the owner for a grant to switch an agent's model asks for that agent only, as for its mode (it asked for the whole machine); the terminal client asks for the same grant.

### Agents
- Agents started in structured mode (ACP agents through the bridge, and adapter agents) now get the mesh: the node passes each its own mesh server when its session starts, restarts or resumes, so they can message other agents, use the board and leases, appear in other agents' lists and start helpers (with your approval) like terminal agents. Before, they started with no MCP servers at all.

### Agents
- A structured (ACP) agent that is not confined keeps the user's temporary folder (on Windows it no longer drops temp files in the profile root); a confined one gets its own private one. A terminal resized to the size it already has is left alone, which stops a fresh Git `sh` on Windows from sometimes losing the first key typed.

### Agents
- Agents started in structured (ACP) mode now find their own configuration and login: the node passes on the variables that name the user's folders (`HOME`, `USERPROFILE`, `APPDATA`, `XDG_*`, language and terminal settings) and still passes no API key or token. Before, they ran with only `PATH`.

### Activity
- Each row on the Activity page names the session that did it (`session 6ee82d`), so two windows or devices of the same person can be told apart. The sign-in row has the same id, so you can see how that session signed in.

### ACP agents
- The ACP bridge no longer ends a failed turn as a silent success: a turn the agent answered with an error, stopped itself (for example a refusal), or ended without saying anything carries a reason code (`adapter.turn_failed`, `adapter.turn_stopped`, `adapter.turn_empty`) in its closing state and in a log line. Showing it in the conversation still needs a protocol addition.
- An ACP agent that exits is reported with the stage it was in, its exit code and a short tail of its error output with secrets removed (`adapter.agent_exited`). One that dies before it is initialized answers the node's `initialize` with that reason instead of just ending.
- The running cost an ACP agent reports (`usage_update`) becomes a usage report of how much it grew, with the agent's model; the bridge declares `usage`.
- ADR-0058, protocol and node: those reasons now reach the conversation as notices (a code and parameters, rendered by the clients in your language). An ACP agent that dies or refuses while starting makes `agent start` fail with that reason (`adapter.agent_exited` with the step, exit code and a short, secret-free tail of its errors; `adapter.agent_refused`) instead of "Something went wrong". The modes and models an agent offers are recorded, and a node switches between them on `agent.set_mode` / `agent.set_model` (only ids the agent offered; never sent to a node or agent that cannot). Usage reports carry how full the agent's context is and its own cost, which the cost view uses in place of an estimate.
- ADR-0058, windows: a chat agent's conversation shows those notices in your language (a failed or empty turn, the agent stopping with the end of its error output). Above the conversation, the agent's mode and model can be switched among what it offers, a mode in which it acts without asking needs a confirmation first and is never chosen for you, and the bar shows how full its context is and the cost it reported. The terminal client shows the notices and the mode, model and context, and switches with `/mode <name>` and `/model <name>` (a mode that does not ask needs the same line twice).
- A failed or empty ACP turn's notice carries what the agent wrote to its error output during that turn (an expired login, a provider's refusal); `opencode-acp` now asks opencode for those errors (`--print-logs --log-level ERROR`). Error output no longer shows your home folder's path even though the bridge runs without `HOME`: any `/Users/<name>`, `/home/<name>`, `/root` or `C:\Users\<name>` becomes `~`.

### Approvals
- An agent shown full size now shows what it waits on with Allow and Deny right there; before, it said "Needs you" and the answer could only be given from the window.

### Agents
- New `opencode-acp` and `antigravity-acp` kinds start opencode and Antigravity (`agy acp`) as structured (ACP) agents. Both were written from the programs' own `--help` and still need a run against the real programs.

### Machines
- `agora panel` on a machine that runs a node says where the hub's panel is and which certificate to trust, so the panel is reachable from every machine of the hub (ADR-0056). You still sign in with a code from `agora hub login`.

### Agents working together
- An agent connected to the mesh is now told what the mesh is for: who it can see (`agents_list`), how to message and ask others, share tasks and hand work over, and lease files before editing them. It is also told it cannot start other agents from there.

### Agents that start agents
- An agent can ask to start a helper agent for a piece of work (ADR-0057). Nothing starts until you approve the request, which shows the asking agent, the kind, the folder and the whole first prompt. You can allow a few more helpers of that kind in that folder while the agent runs, and revoke that permission at any time.
- The asking agent can wait for its helper and read the last lines of its screen. A helper works only in its parent's folder (or a worktree of it) unless you approve another folder.
- Limits: 3 running helpers per agent; by default a helper cannot start helpers of its own.
- On a machine that offers this, an agent's mesh instructions tell it about `agent_start` instead of saying it cannot start agents. A helper's first prompt is typed only once its screen is idle, never into a trust or permission question, and a prompt that would run a command in the agent itself (`!`, `/`, `#`) is refused. No "more like this" permission is offered for a shell.
- The web window and the terminal client show helpers under the agent that started them ("started by …"); the web window lists what an agent may still start without asking, with a Revoke button. In the terminal client, stopping an agent with helpers asks whether they stop too.

## 0.3.6

### Audit
- A command whose client disconnected before the answer still gets its outcome in the audit trail.

### Closed agents
- A closed agent is shown as finished the same way whatever its exit code; before, one stopped while the window was open (which leaves a non-zero code) showed the orange "Needs you" mark until the next reload. The summary states the exit code ("Ended with exit code 143") instead of calling it an error.

## 0.3.5

### Machines
- After a machine resynchronizes, its agents that still run keep the tasks and files they had taken on a shared board.

### Closed agents
- A closed agent is always shown as finished; before, one that ended while it was waiting kept a spinning orange mark and counted as waiting for you.
- The ✕ on a closed agent in the list and **Clear closed** now remove it for everyone when the hub can (ADR-0053). If the hub refuses (a permission, or the agent is running again) the reason is shown and nothing is hidden; only a hub that does not know the command hides it in this browser. What you cleared stays hidden only while the agent is closed.
### Starting agents
- The start window picks up a machine that comes online after it was opened, instead of leaving nothing selected and no folders to choose from.

## 0.3.4

### Terminal client
- In `agora tui`: **f** picks a folder on a machine (step into folders, Git projects marked) and starts an agent there; **d** removes a closed agent for everyone, and a refusal says why in words. Every folder action is signed with the terminal's own device key.
### Machines
- A machine that was offline too long, or every machine after the hub is restored from a backup, reconnects on its own and shows its current agents (ADR-0054); before, it never reached the hub again. The events from the lost window are recorded as missing, and the owner's Activity page lists it.
### Fixes
- A folder listing answers the first 500 folders by name, not the first 500 the file system returned, and never reads more than 50,000 names.
- An MCP connection that drops no longer leaves its agent's unanswered questions behind; they are cancelled with it.

## 0.3.3
### Machines
- A machine that was cut off from the hub for a long time sends what it saved much faster when the link comes back.

### Starting agents
- **Browse folders:** the start window can list a machine's folders (ADR-0053): click **Browse folders**, step into a folder or up one, see which are Git projects, and use the folder you are in. Only folder names are listed, protected folders never show, and each listing is signed by this device. A machine that has not been updated keeps the typed path.

### Closed agents
- **Remove for everyone:** a closed agent's summary can ask the hub to forget it (ADR-0053): it disappears from every window, the TUI and the desktop app. Its earlier events stay in the hub's record and its tokens and cost still count. An agent that still runs cannot be forgotten. "Remove from this list" still only hides it in this browser.
### The window
- Drag the dividers to resize the agent list and the details panel (or focus a divider and use the arrow keys; Shift for bigger steps). The widths are remembered in this browser.
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

### Hub and agents' messages
- A machine that reconnects is no longer shown offline when its old connection ends late, and terminals opened through the old connection close so the window opens them again.
- A file request that fails keeps the files the agent already held; an answer waiting for an agent is kept until it reads it; giving back an older hold no longer frees a file taken again since.

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

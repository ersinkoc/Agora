# Changelog

Each release lists what a person using Agora can now do. The reasons are in `docs/adr/`; the state of every task is in `docs/backlog/`.

## Next

- A terminal agent whose program ended while something it started in the background (outside its process group) went on writing to its terminal at least once a second is now seen ending within about a second: the window shows it exited, a crash is restarted, and stopping it returns within about two seconds; before, the stop waited for as long as that background program kept writing.
- A window that opens a terminal just after its program ended now gets the program's exit code; before, it got "unknown" when the node let the terminal go a moment later.
- A prompt, or your answer to a request, that a structured agent's adapter does not take in time (or at all) is now reported with the reference `agent.prompt` or `approval.resolve`, and the machine's log says which failed; before, both said `unassigned` and the log called them a failed start. A queued prompt, or a helper's first prompt, that fails is now logged too (#1150).
- An agent type whose rule looks for a phrase with a space at its start or end (to match whole words only) no longer matches that phrase inside a longer word when the agent's screen wraps it over two lines; before, the wrapped text could show the agent as done or waiting when it was not (#708).
- On Linux, a confined agent is no longer started when a folder or file it is given leads, by the time it is opened for confinement, to Agora's own folder (its data, its secrets or its control token) or to a folder above it, also when it was changed into a link after the start's check: the start is refused, as it already was when the grant led there at the check (#1150).
- An agent's request to start a helper is now also refused when its first prompt holds the helper's own footer text with different spacing or a line break inside it, as it already was for the text as written; the screen rules read both the same way (ADR-0057).
- A machine that has run many agents in very long folder paths no longer loses its whole list of agents and their folders at the next restart: the node now keeps the list within the size it reads back, dropping the oldest ended agents first; before, a list that grew past 4 MiB was written but then ignored, so no interrupted agent could be resumed. A list already over that size from an earlier version is still set aside once, at the first start after the update (#708).
- Security: the secret values a machine holds (its stored secrets and the environment variables its policy passes to agents) are now replaced by `<redacted>` in what a structured agent's conversation sends to the window as it happens, not only in the copy saved on the machine's disk, so a key an agent prints no longer passes through the hub. The agent's words are not delayed by it, except words that end in what could be the start of such a value, which wait until the next words show it is not one, the agent does something else or its turn ends, and never more than about 0.4 seconds. The same goes for what an agent asks to do on an approval card, the messages agents send each other as the hub records them, task titles and handoff summaries, and the descriptions of an agent's modes, options and commands (#1147, ADR-0089 Amendment A).
- The terminal client's start pickers (`s`, and `f` for a folder) now list the ACP registry's agents a machine has installed after the hand-made ones, under "Untested (from the ACP registry, not yet tried in Agora)", and leave out the ones the machine cannot find, as the window's Start does; before, all 41 were listed as they came, most of them as not found. None is ever chosen for you, and one that downloads its program when it starts still sends you from `s` to `f`, the start this device signs (#1134, ADR-0092).
- `agora agent resume AGENT_ID` brings back an agent that `agora agent list` shows as interrupted with `resume`: the same agent, in the same folder, continuing its own conversation, as Resume does in the window and the terminal client. `agora agent resume AGENT_ID --again` starts one shown with `start-again` again in its folder, with a new conversation. When the agent cannot be resumed (it was not interrupted, it has no conversation to continue, its folder changed, its process still runs from before the restart, or the agent that started it is not running), the command says why (#1137).
- Stopping Agora while one of its agents is starting, or with Ctrl-C on `agora supervise node` in a terminal, now still keeps the last turn of a kept structured agent and the usage it reported in the last few seconds: before, the stop then kept none of it, or ended the node outright (#1119).
- A structured agent's text with a colour or cursor code that is never closed no longer loses the line break after it and the first letter of the next line in the conversation; the code now ends at the line break or after 4096 characters, as the other escape codes already did, and a lone carriage return now counts as a line break for all of them (#1113).
- Stopping Agora on a machine (`agora down`, stopping or restarting its service, the restart after an update, Ctrl-C on `agora node run`) now lets the node finish cleanly on Linux and macOS instead of ending it outright: what a kept structured agent said in its last moments, its last turn in the conversation and the usage it reported in the last few seconds are kept for the next start, and the agents keep running. The node takes at most 5 seconds, also when the part that keeps the agents running does not answer (ADR-0104).
- Agora now knows the 41 agents of the ACP registry that it can convert (of 46; four the registry itself holds back and one whose arguments differ per platform are left out), among them Gemini CLI, Qwen Code, goose, Cursor, Kiro, GitHub Copilot, Junie and Mistral Vibe. Each is an agent kind a machine offers, marked "Untested" because nobody has run it through Agora yet; Settings → Agents lists the ones a machine cannot find under "Add an agent" with their version, licence and repository, and nothing there runs or downloads anything. A hand-made description of the same agent keeps priority. An agent that downloads its program when it starts (through npx or uvx) starts only when you start it yourself from a trusted device (the window's Start, or the terminal client's folder start), not by the hub with an unsigned start; a machine's policy line `agents.registry = off` (or `agents.registry.deny = ...`) switches them off there, and `agora doctor` says so (ADR-0092 step 1).
- A structured agent's conversation is no longer empty after its machine's Agora restarts: the node now keeps each conversation on its own disk (readable by its user only, the secrets it holds replaced, kept 30 days after the agent ended and at most 32 MiB per agent, both lowered with `chat.history_days` and `chat.history_max_bytes` in the node policy), so a kept or resumed agent shows its earlier turns, and a tool call that finishes after the restart updates its own row. A full disk does not stop the agent; its conversation says once that it is not being saved. `agora history purge` removes all of it (#931, ADR-0104, ADR-0089).
- Removing a closed agent from the list for everyone (forget, in the window or the terminal client) now also removes its saved conversation on its machine and stops offering it for Resume there, also when the machine is offline at the time (it happens when the machine next connects, also after the hub restarts). A machine still running the agent keeps it. Machines on an earlier version keep the conversation until it expires (#1104, ADR-0108).
- In team mode, the window and the terminal client know the role of the person they signed in as from the moment they connect (the hub says it first, ADR-0106), so a member's or viewer's window no longer asks the hub for the pairing cards or the cost and leaves no refused row in the owner's Activity for it. A viewer is no longer offered starting, stopping, removing, resuming, answering, approving another device or giving a task in the terminal client (its footer lists only the keys that read, and its typing and terminal sizes are not sent), nor starting agents or terminals, prompts, giving tasks, device changes or a helper's grant in the window, whose empty Home now says what they will see; forgetting a machine is offered to the owner only. A member's folder picker opens at their project without asking first, and a member made the owner gets the owner's view at the reconnect the hub forces. An older hub keeps the behaviour from before (#1099).
- In team mode, a viewer's window now shows every request read-only, with no Allow, Deny or "Allow always" (in the pane, the side panel, Home, the attention dialog, Review and on the phone), and a chat agent's composer has no box to type in, its Mode and Model menus only showing what the agent runs with; in the terminal client a viewer's chat has no line to type on, and requests (in the list, the details panel, a chat tab and the reader of a start request) show without the keys that answer them. The attention dialog, the phone's bar and the chat say "You can watch here" instead. Before, a viewer could still try these and leave a refused row in the owner's Activity. An older hub keeps them (#1140).

## 0.3.37

- A node now refuses any command other than a subscription on a terminal, chat or preview stream, as the protocol says, instead of relying on the hub to; a subscription there needs the same grant as on the commands stream, and a grant for one agent now covers watching its terminal or following its conversation (the window asks for that agent only, not the whole machine).
- Security, Linux: a node whose data folder (`--data`) or agent types folder (`--manifests`) lies inside a folder confined agents may read or write (`/tmp`, `/var`, `/opt`, `/etc`, `/run`, or one named in the policy) no longer starts confined agents, which could read its stored secrets, its session ledger and its chat transcripts there; the start is refused (`agent.sandbox_unavailable`, reason `protected_path_reachable`) and `agora doctor` names the folder. Before, only a reachable control token was refused. Keep the node's folder under your home (`~/.agora`, the default). macOS keeps these folders closed itself; macOS and Windows are unchanged (#1114).
- `agora agent list` keeps listing the agents that ended (stopped, with their last state and folder) after the node restarts, for as long as the node keeps them (14 days), whatever folder they ran in; before, a restarted node said it had no agents although it still kept them (#1120).
- `agora agent list` now also lists the agents a restarted node found interrupted (they were running when it stopped), with `interrupted` in the fourth column and what can be done in a fifth: `resume`, `start-again`, or `still-running` (its process outlived the node; `agora agent stop` ends it); before, they were missing from the list until resumed in the window or the terminal client, for up to 14 days. Lines of the other agents do not change, though `agora agent list` now orders every line by id number itself, so against an older node (0.3.36 and earlier) with ten or more agents the same lines can come in a different order (ADR-0107, #1121).

## 0.3.36

- Team chat shows the agents' recent messages (up to 50) in a window that opens, reloads or reconnects, instead of starting empty and showing only messages sent while it was open; a member or viewer sees the same messages they would have seen live (ADR-0103, #1020).
- In team mode, the terminal client's start in a folder (`f`) now opens a member at their project's folder on that machine, or lists their projects there to choose from when they have several, and the way up stops at the project's folder; before, it opened at the home folder, which the hub refuses a member, so they could not reach their project's folders at all. In the console, the owner's still opens at the home folder (#1086).
- In team mode, the terminal client's plain list (no console) now opens start in a folder (`f`) at the machine's project folder, as the window does, so a member is no longer refused the home folder on their first `f` of each connection (which left a refused row in the owner's Activity); the owner goes up from there, and a machine with no project folder, or a project folder that cannot be listed (gone, protected; not when the machine or the link failed), opens at the home folder. In the console, a member made the owner can go up from their project's folder without restarting the client, and gets the owner's picker (and the pairing cards) once the client has noticed the change: on a quiet hub, or after a restart, since a busy hub's routine refreshes can hide the reconnection (ADR-0106, if accepted, removes this; #1109). A viewer with several projects on a machine is told at once that their role does not let them choose a folder, instead of after choosing a project (#1099).
- In team mode, a member can now browse the folders of a project they were granted on its machine, choose one and start an agent there, in the window's Start an agent; a folder of another project or machine, a folder above the project (the way up stops at the project's folder) and a link that leads out of the project are refused with "Your role does not let you choose this folder", and a viewer stays read-only (#1049, ADR-0049 Amendment A).
- An agent that could not join the other agents on its machine (so it could not message them or start helpers) is now written in the node's log instead of running without its place silently (#1093).
- A structured agent that kept running through a node restart keeps its token counts, cost, model and context level in the window, the terminal client and the cost total, instead of starting again from 0; so does every running agent when a hub that missed some of its machine's events catches up (#930, ADR-0104). Usage the agent reported in the last few seconds before its node stopped can still be lost.
- A structured agent that kept running through a node restart is no longer shown with an earlier mode or settings when it changed them just before its node stopped (for example a switch to a mode that acts without asking): the next node now uses the newest the agent reported, also when the old node had read it and not yet recorded it (#1080, ADR-0104).
- The desktop app now builds with `cookie` 0.18.3 (RUSTSEC-2026-0594: private-cookie nonces could repeat after a fork; the app does not fork after it starts, so it was most likely not exposed).
- Approving a new machine in the window now closes the hub to new machines, as approving it at `agora pair open` does (one machine per opening; other machines still waiting are told the hub closed), and an `agora pair open` that was asking about the same machine stops asking and says it was answered in a window (#942).
- A machine that disconnects while the hub cannot write to its database (busy or failing) is now shown offline as soon as the hub can write again, instead of staying online in every window until it came back or the hub restarted (#996).
- In team mode, a member's or viewer's refused start, folder, Start again or answer now says their role does not allow it (in the window and the terminal client) instead of "This device is limited to other projects", and their window no longer asks the hub for the cost on each connection, so it leaves no refused request in the owner's Activity.
- A machine that disconnects while the hub cannot write to its database and then reconnects is now asked to put its unfinished local tasks and file claims for a shared project on the project's board, as after any other reconnect, instead of keeping them to itself (#1036).
- A chat log that follows its newest entry keeps following when the window or its pane is resized (or the composer grows), instead of stopping and showing "Jump to latest" as if you had scrolled away; scrolling up yourself, going to a file's change from Files changed, or opening a fold that takes the log off its bottom still leaves the bottom, and what you opened stays where you clicked it (#1060).
- A hub whose stored screen password or list of vouched passkeys cannot be read now refuses a sign-in from another machine and a password change instead of acting as if no password were set; a password change from the window now stores the new password, signs out the other sessions and removes passkeys added without the password all together or not at all, and a passkey is added together with the record of whether the password was given.
- `agora password set` and `agora password clear` now store the password, sign out every session and remove passkeys added without the password all together or not at all, and say that they failed instead of reporting success when any part could not be done.
- A terminal, approval or worktree id that a restarted machine uses again no longer shows to people who could see the old one only.
- When a slow window is dropped from a shared terminal, the terminal grows back to the size of the windows still watching.
- A handoff whose receiving agent cannot take it now goes back to the giver even when the giver's machine disconnected first.
- Stopping a chat agent's turn no longer withdraws its waiting request to start a helper.
- In a Files view, a file that was still loading when you pressed Refresh now opens instead of loading for ever, and revoking two helper permissions one after the other removes both from the list.
- A malformed `%` escape in a window link (`#/agent/…`, `#/machines/…`) no longer blanks the page, and a file view no longer stops responding on highlighted text with an unclosed tag.
- `agora hub notify` refuses an address with a line break in it, and `agora hub run`, `agora hub notify` and `agora hub ssh` stop with an error instead of overwriting a token or list they cannot read; `agora update` downloads into a new private folder with a random name.
- A machine whose hub listens on IPv6 (`[::1]` or `[::]`) now reaches it on IPv6 loopback.
- The owner gives a person access to a project by the names the window shows: `agora hub user grant ID MACHINE/PROJECT` takes the machine's name and the project's folder name (or `agora hub user grant ID MACHINE FOLDER` with the folder's full path), `agora hub user projects` lists what can be granted, and `agora hub user list` says where each person works by name. A machine or project the hub does not know is refused with a reason instead of being stored silently, a name that fits two projects lists both to choose from, and a grant stored by an older build can still be revoked. A person with no project yet is told so instead of being shown how to connect a machine, their window no longer asks the hub every 20 seconds whether it is the owner's (each refusal was a row in Activity), Activity names the pairing actions in words, and `agora --help` lists `hub user` and `hub audit` (#904).
- On a machine set to allow "don't ask" only for confined agents (`skip_permissions = confined_only`), an agent that kept running through a node restart (it was never confined) can no longer be switched to a mode, option or model that acts without asking, even after confinement was turned on.
- `agora pair open` on a hub that listens on every address now suggests an address this computer has now, and lists an address its certificate still names but no interface has any more (an old DHCP lease, or an address in front of a router) apart, as one that may not lead here; on a hub that listens on this computer only and a computer on several private networks, it lists each of this computer's addresses for you to choose the one the other machine is on, instead of picking the first (#993).
- A node no longer crashes on a policy line `device.projects = ...`, and a failed `agora agent install` leaves the previous version's records (its version and its program's name) as they were, or says `rollback_incomplete` when it could not put them back.
- `agora agent install` now checks, takes its lock on and replaces its record of the newest agent list it accepted (`agents/list-state.json`) all in the one `agents` folder it opened, so another folder put in that folder's place meanwhile is neither read nor written (#1000).

## 0.3.35

- A structured (ACP) agent that offers "always allow" for a request, as WrongStack's `wstack acp serve` does, can now be told so once: the approval card, the attention dialog, the banners and the phone's card show **Allow always** beside Allow and Deny (in the terminal client, `A` in the request box that shows the whole request; Alt+A in the agent's tab or chat opens that box, and `A` there confirms), and the agent stops asking for what it counts as the same request. The button appears only when the agent offers it and the machine allows it (`skip_permissions = forbid`, or `confined_only` for an agent that is not confined, removes it), says that the agent remembers the answer and decides what it covers and for how long, and is signed over its own message, which binds the agent, the request and the agent's option, so a hub cannot turn a single Allow into a remembered one. A plain Allow is never sent to the agent as a remembered answer any more; an agent that offers only "always" shows that button in the place of Allow (an older window can only deny such a request). While an agent holds such an answer it shows **Asks less** (`[asks less]` in the terminal client) until it ends or starts again (ADR-0101; new error codes `approval.scope_forbidden`, `approval.scope_unavailable`, `approval.adapter_outdated`, and conversation notices `approval.always_only_forbidden` and `adapter.permission_unanswerable`).
- In the web window, an agent that acts without asking you is now marked with one small red lightning bolt wherever it appears (tree, tile, tab, pane header, board, give-task picker) instead of a warning sign with the words "Acts freely" (the chat's session bar keeps its words, and the terminal client keeps its `[does not ask]` marker); hovering it, focusing it or tapping it shows what it means, it stays open while the pointer is on it and Esc closes it, and screen readers read the meaning as part of the agent's name (#1004).
- On Windows, a confined agent (the sandbox on, `AGORA_WINDOWS_CONFINEMENT=1`) may now read and run the node's own `agora.exe`, the mesh bridge its mesh tools start, and only that one file, instead of getting "Access is denied."; whether a given agent's tools then work also depends on how that agent starts its own programs (see the next item) (#896).
- On Windows, a confined agent's node log names a folder it could not be given read access to once per folder for as long as the node runs, instead of at every start, and the sandbox recipes say what was measured not to work confined yet: under Node 26, `child_process.spawnSync` with the default piped stdio is refused by Windows (Node's asynchronous `spawn` makes its pipes the same way), while a Rust test program can start children with piped stdio and the confined shell can start children; real agents were not measured (#898; the fix waits on a decision, #895).

## 0.3.34

- A symbolic link to itself, or a ring of links (`a` to `b` and `b` to `a`), in a project folder is now refused in Files and the terminal client's file view with "is outside the project folder." (`files.outside_root`), as a link that leads nowhere already was, instead of "Something went wrong" (#924).
- `agora pair open` on a hub that listens on this computer only now says that another machine cannot reach it and how to change that (listen on this computer's network address, `agora --listen 192.168.1.20:PORT`, or on every IPv4 interface with `0.0.0.0:PORT`, a public one too), and offers only a join from this same computer, instead of telling the new machine to dial this computer's name, which could not work (#943). On a hub that other machines can reach it suggests only addresses the hub surely listens on and its certificate names, and lists the certificate's names as names that may also reach it.
- In a folder whose repository has no commit yet, the terminal client's `r` and the window's Changes show every file as new (secret files by name only) instead of "Could not read its changes (internal).", and its changed-file count is shown like any other folder's (#941).
- After you reload a window or open a terminal again, and when you select an agent in the terminal client (its terminal panel, not the full-screen terminal you open from it, which still starts with the screen only), you can scroll back through what the program printed before you were watching (up to your terminal's history setting in the window, 2000 lines in the terminal client, and at most what the machine keeps), instead of only its last screen; the lines keep their colours, and coming back after a lost connection keeps the history you had without repeating it.
- `agora agent install` no longer accepts an older signed agent list when its record of the newest list it accepted (`agents/list-state.json` in the Agora folder) is damaged or cannot be read: it now refuses every list (`agent.install.index_invalid`, `list_state_unreadable`), names the file, and says how to start again (remove it and give the newest list from its publisher). Before, such a file counted as "no list seen yet" and let a rolled-back list through (#923).
- `agora join` by an address the hub's certificate does not name (for example `127.0.0.1` for a hub that listens on all addresses) now stops at once with "could not be verified, so nothing was saved", before a number is shown, so the owner is never asked to approve a join that would be thrown away, and the hub no longer keeps that machine's name: a later join with the same name is not offered "Replace the existing machine" (#901).
- A terminal you open, reload or come back to after a lost connection now gets the program's mouse, wheel, focus and full-screen state as if you had watched from the start: clicks and the wheel reach an editor or monitor started before the window opened, the wheel sends arrow keys to a full-screen program, and a window that was away while a program switched the mouse off no longer sends it stray mouse reports. This works in the web window and the terminal client once the machine runs this version.
- After Stop turn in a chat agent's conversation, a step that was still running reads "Stopped" instead of spinning for ever, the input no longer says "Waiting for this turn to end." once nothing waits, and the messages you sent while the turn ran are each shown above their own answer instead of the answers being glued into one reply (#908).
- After `agora down` and `agora`, an agent the terminal holder kept is shown running again in the window and the terminal client, with its terminal, instead of "Finished", and one started with an option that acts without asking, or a chat agent in a mode that does, is still marked so, with its modes and settings (#907, #932).
- After a node restart, an agent the terminal holder kept is on the mesh again: its mesh tools answer, and an `agora mesh stdio` bridge that was open reconnects (#906).
- After a node restart, an agent the terminal holder kept receives messages again: messages from agents on its machine or on other machines, and the notices that an agent joined or left, are in its inbox instead of being hidden behind the last message it read before the restart (#965).
- A chat agent whose program keeps crashing now says so in its conversation each time Agora starts it again, and once Agora gives up it is shown as "Ended with an error" with its exit code and takes no more messages, instead of staying "Unknown" with a live input box (#902). A reloaded window now also keeps showing an agent that ended with an error as failed, with its exit code, instead of "Finished".
- `agora agent start` (and a start from the window, a resume, or an agent starting a helper) for an agent whose program this machine cannot find now says "cannot find PROGRAM in its PATH" instead of "Something went wrong" (#903; new error code `agent.program_missing`).
- The window's Messages list shows a message between machines once, from its sender and marked "between machines", instead of a second time with the sender's name doubled (`rm-two/blackview/agent-16`) and no mark (#905). A copy that only another machine than the sender's recorded says which machine reported it.
- A mesh message to another machine whose answer never comes back now fails with a `delivery_failed` notice `node.timeout` after 40 s on the sending machine, so the sender always learns that it was not delivered (#915).
- In the terminal client, the agent list of `s` and `f` now offers every agent installed on the machine, nine to a page: `n` shows the next page, and the question says which page you are on (#910). Before, only the first nine were offered and nothing said more existed.
- An agent's mesh tools no longer leave a request unanswered when its line is not plain single JSON: a line that is not valid JSON is answered with a parse error (`-32700`), one that is not a valid request with `-32600`, and an MCP client that settled on protocol version 2025-03-26 gets a batch of requests (up to 32 in one line) answered with one batch of answers. Under other protocol versions, which have no batches, each request in a batch is refused with `-32600` instead of being dropped (#981, #984).
- In the terminal client, `b` opens the board of the selected agent's project (Backlog, Doing, Done), and Enter on a Backlog task gives it to one of the project's running agents after a confirmation you answer with `g`.
- A Codex agent (or another whose session Agora reads from the file the tool writes) that crashes and is started again, or that you resume before its session was known, now waits for the new program's own session, so Resume later continues the conversation of the program that ran last, never the one before it; a program that gave no usable session file is looked for again after its restart instead of never (#925).
- A text file written as UTF-16 with a byte-order mark (what Windows PowerShell's `>` writes, as in `"hello" > a.txt` in the Shell) is shown as text in Files and the terminal client's file view, and its changes as a text diff in Changes and `r`, instead of "This file is not text". The file itself is never rewritten; a download is still its own bytes (#940).
- A confined agent's generated Git configuration in the node's `data/git-config/` folder is now removed when the agent ends, is stopped or crashes (a restart gets a new one) and when its start fails, instead of keeping one file for every launch. Files of agents a node never ended (it was killed, or the machine stopped) are not removed (#933).

## 0.3.33

### Give a board task to an agent
- On a project's board, a Backlog card has "Give to an agent" (also in the command palette). You pick one of the project's running agents (helpers are listed under the agent that started them), and a confirmation names the task, the agent, its machine and the project, and says so when the agent acts without asking. The agent gets the task as one message; if it is in the middle of a turn, the task waits until that turn ends, and anything that needs your OK still asks you. The card then moves to Doing.
- The task's text reaches the agent marked as information from the board, not as instructions from you or from Agora. A terminal agent gets it as one pasted block once it is between turns; a terminal program that cannot take a pasted block is refused. Each person can give at most 8 tasks a minute, and every hand-over is recorded with a fingerprint of the message sent.

### Install an agent program from a signed list, from the command line
- `agora agent install NAME --list FILE --root-key KEY` downloads one agent program (a single executable file) listed in a signed agent list, checks its checksum (and its signature when the list names a key), runs its version check, and only then makes it the installed one under the Agora folder. You approve by typing the agent's name at the terminal; there is no `--yes`. An older version needs `--go-back` and a second confirmation, and the last two versions are kept.
- Downloads refuse addresses that point at this machine or your private network, follow no redirects, and stop at 200 MiB. Programs inside archives (`.tar.gz`, `.zip`) are not supported yet. Nothing is installed by the hub or the window yet. Terminal agents you start now use a program installed this way before one on your PATH, and `agora doctor` says whether an agent was installed by Agora (with its version) or by you. Agents that start through ACP or an adapter, and other programs an agent runs, still look only on your PATH.

### Steadier usage, folders and colours
- Two agents working in the same folder no longer both count each other's tokens and cost in the Usage panel.
- The node's data, mesh and link folders are closed to other users of the machine when it starts.
- Agent colours come from the theme and are darker in Light, so a tile's edge shows on a white page.
- The web app builds on Windows and macOS again.

## 0.3.32

### Your agents, per machine
- Settings has an Agents section (`#/device/agents`): for each machine, the agents it can start, installed ones first, how each runs (terminal or chat), and the model, effort and options each takes; a machine that is offline shows what it reported last, marked as older. It is read only for now: choices are still made when you start an agent.
- `agora doctor` now tells an agent program that is on the machine but cannot be run (no execute permission) apart from one that is missing.

### Permission questions say what they allow
- When a machine asks for your OK before you type into, message or stop an agent, the question now names the agent and that one action, and the one-hour permission covers only that action (before, it also covered stopping and sending messages), in the web window and the terminal client.
- In the terminal client, a question over the console (such as the one-hour permission to type) no longer closes the open agent's terminal, is not answered by a `y` inside a paste or beside passive mouse motion, and a refresh no longer drops a refusal of a key you just typed; Ctrl-K lists `g`, the key that asks for that permission.

### Steadier session matching and connections
- When the node decides whether two agents could be mixed up, it looks at the disk on a thread of its own (reading the session files still happens on the supervision thread). A moved workspace or a changed manifest is noticed within 30 seconds, and until a lookup answers the agent is given no session.
- The agent keeper link, the chat-agent bridge and the files helper are stricter about time limits, queue sizes and paths: a chat agent that stops reading no longer stalls the bridge (its queue is bounded), and the worktree file writer, still switched off, now refuses links and secret names at every step.

## 0.3.31

### A board for a project's tasks
- A project's tasks can be opened as a board with Backlog, Doing and Done columns (read-only), from the project's card in the Board panel. Task text is shown as plain text.

### Helpers that act without asking, only when you say so
- An agent can start a helper that acts without asking only when your signed decision names that option; the approval card says so, and a grant never covers it.

### Steadier agents and connections
- An agent that kept running while the node restarted comes back showing its last known state (such as idle) even when its state was first noted a moment before its start was recorded.
- The hub stays responsive for other machines while one machine's events are written to disk.
- `agora join` never prints a refusal sentence a remote machine chose; a skip-permissions approval card is withdrawn when either switch of the same setting changes; two agents reading the same session files in one folder never take each other's session; the console asks for a grant when typing into an agent is refused.
- Signing in with the screen password no longer ties up the hub's request workers: a busy sign-in answers "busy" instead of stalling other machines, and a sign-in on the machine itself keeps its own reserved slot.
- The desktop app's local tunnel limits how many connections it serves at once and closes them when the app lets go of it.
- On Windows, an agent whose command is installed as a `.cmd` file, such as `npx` or most npm global commands, is now shown as installed and starts, as a chat agent, a terminal agent or through an adapter. Before, it was shown as not installed, and a chat agent failed with "the adapter exited".

### Agents from the ACP registry
- Agora can now turn the ACP agent registry's entries into agent kinds and install records (a maintainer step; the list in the window and installing from it come next, and no machine ever reads the registry itself).

## 0.3.30

### Steadier tabs in a split
- The tab you are on keeps room for its icon, the start of its name with an ellipsis, and its close button, even in a narrow split or at a high display zoom; the other tabs step aside into the tab menu, and a pane title that is too long ends in an ellipsis instead of a cut-off word.

### An older agent keeper is now explained
- When the agent keeper running on a machine was started before 0.3.27, chat agents there are resumed after a node restart instead of kept running, and nothing said so. `agora doctor` now warns about it (it does not change doctor's exit status) and `agora status` shows a line under the node, in English and Turkish; the node's log also says why once. The keeper ends by itself when the agents it keeps have ended, and a new one then starts. Nothing is stopped or restarted for you.

## 0.3.29

### Close an agent from the window
- A running agent, including a structured (chat) agent that has no terminal to type `exit` into, can now be closed from the window: choose Close agent in the **⋯** menu of its row in the agent list, in its pane or screen header (also on a phone) or in the command palette, or type `/exit` or `/quit` in a chat agent's message box (an agent's own command of that name still goes to the agent). A small confirm in the page says what happens and warns when the agent is working; one click closes an idle agent, and it then waits in the closed list until you remove it. A refusal says why in your language. Stop turn is unchanged and still keeps the agent.

### Settings and machines one click away
- The title bar now has a Settings icon (a gear) and a Machines icon next to Lock, in place of the "This device" text; the one whose page is open is highlighted, and both are reachable with the keyboard.
- Settings is one tidy page with a section list: This device (name and fingerprint, appearance and language, notifications, passkey, screen password, your other devices) and My machines (each machine's state, last seen, version and whether it trusts this browser, with a link to its card). Links such as `#/device/passkeys` open a section directly; the old addresses still work.

## 0.3.28

### Structured agents: overlapping messages
- When two messages to a structured agent overlap (the next queued one is sent after a cancel, or a helper asks while you wait), the first reply no longer ends the running turn or drops its waiting permission request: the agent keeps working and your decision still reaches it. A permission decided after its turn already ended returns the agent to idle instead of showing it as working.

### Steadier window layout and terminals
- Tabs in a pane that is split beside another (Files next to an agent, for example) keep a readable title: a tab shrinks to a minimum width and shows an ellipsis with its full title as a tooltip, and a tab that does not fit is not drawn as a cut-off fragment; it is listed in the "more" menu next to the tabs instead.
- "Group by" in the agents panel stays on one row in a narrow panel, in English and in Turkish: the "Group by" word gives way first, then the four buttons shrink.
- A terminal opened in a fresh window now always shows what is on the node's screen. Before, a first open could stay blank when the window painted the screen before it had taken the terminal's size, and an idle shell sent nothing to bring the prompt back.
- An agent that the terminal holder kept running while the node restarted shows the state it had (idle, for example) instead of "unknown" until it writes something again.

## 0.3.27

### Structured agents survive a node restart (Linux and macOS)
- An agent that works through an adapter or the ACP bridge (no terminal) is now run by the same background holder that keeps terminal agents, so restarting the machine's Agora node no longer ends it. The restarted node takes it up again with the same process: a turn that was running goes on, and a permission question that was waiting is asked again (it is never answered for you).
- What the agent said while no node was reading is kept, up to 1 MiB per agent. When more was said than that, the oldest part is dropped and the conversation gets a notice saying how much was lost.
- Without the holder (Windows for now, or when it cannot start) nothing changes: the agent ends with the node and is resumed as before. Agents that run confined are still the node's own.

## 0.3.26

### Plan limits in the Usage panel
- The node reads the plan windows of Codex agents from Codex's own session file (nothing is installed, no login or token is used, only the percentage, the window length and the reset time are taken). Claude Code shows "No limit information" for now.
- The Usage panel shows, per tool and machine, how much of each plan window is used (Five hours, Seven days, or a number of minutes), when it resets (in your time zone) and how long ago it was read. Only what the tool itself reported is shown (Codex for now): after the reset time it says "Reset" instead of the old percentage, a reading older than its window says "Out of date", and a tool with plan limits that reported nothing gets one quiet line saying so. Plan limits are shown to the owner only, not to people who were given some projects. No money anywhere.
- An agent's card and row show a small "Limit 82 %" chip from 80 percent (stronger from 95), and the start window hints when the chosen agent's limit is nearly used up and when it resets. The hint never stops you from starting.
- The hub keeps the latest reading per machine, tool and window (a small, bounded table) and gives it to a window that opens later, only for machines that window may see.

### Acting without asking
- Switching a running agent to a mode that skips permission questions works from the window again: choose it like any other mode, the machine asks for your approval with a card that shows the agent, folder, mode and value, you allow or deny it on this device, and while it holds the agent shows "Acts freely" (ADR-0090 Amendment A).
- The "Acts freely" badge says "with your approval" only when this window saw you approve that mode; otherwise it says plainly that the agent acts without asking.

### The window
- The Mode, Model and Effort menus of the message box open upward when there is no room below, stay inside the window and scroll when long.

## 0.3.25

### Certificates renew themselves
- A machine's certificate for the hub now renews by itself when a third of its life is left (30 of 90 days), while the machine is connected to a hub that is up to date: no code, no owner step, and nothing to do before day 90 any more. The hub already replaces its own certificate by itself when 30 days are left, without a restart. Update the hub first, then the machines.
- A machine keeps working throughout: agents never wait for a renewal, a hub that is away only delays it (it tries again by itself, more slowly each time), and the certificate in use is replaced only after the new one checked out. The new certificate is used by the machine's next connection.
- `agora doctor` shows the days left on this machine's certificate and on the hub's, and says so when a renewal needs you: this machine must join again (the hub no longer accepts its certificate for a new one, or it ended), or its certificate is no longer the newest one the hub issued for it (another holder of it may exist: check the hub's audit view).
- A machine that was off past the end of its certificate still has to join again with the owner's approval, as before.

### The conversation of an agent without a terminal
- When the agent reports it, a tool call shows whether it is running, done or failed, how long it took, which files it touched and the tail of its output; a plan shows which steps are done; the agent's thinking shows as a folded "Thought for N s" block.
- **Stop turn** ends what the agent is doing now and leaves the agent running; **Stop agent** is a separate, clearly different button. Messages you send while the agent is busy wait in a queue you can see and drop.
- Model, mode and reasoning effort are menus fed by the agent's own options. Choices that would make an agent act without asking are shown switched off with the reason (start the agent that way instead); an agent can be switched back to asking.
- Typing / in the message box offers the commands the agent provides. A window that connects later sees the same options and commands.
- Permission requests show in the conversation as a record of what was asked and decided; answering still happens in the approval bar, signed on the node.

### Known limits
- Switching a running agent to a mode that skips permission questions is not available from the window until a signed confirmation exists; start the agent that way instead.
- Plan limits of subscription agents and the preview of an agent's local web server are designed and have their messages in the schema, but are not built yet.

## 0.3.24

### Agents survive the node (Linux and macOS)
- An agent now keeps running when Agora's node stops or restarts. Agents are started through a small keeper process (`agora hold`) that outlives the node; the next `agora` attaches to them again, with their screens and whatever they printed meanwhile.
- `agora down` stops the node and the hub and leaves the agents running (it says how many); `agora down --agents` stops them too. `agora status` shows agents that run without a node.
- Agents that are confined (a sandbox policy), and every agent on Windows, still run inside the node and end with it, as before; a keeper that cannot start changes nothing.
- With the login service installed on Linux, restarting or stopping the node's service no longer ends the agents that run through the keeper. For a service installed earlier, run `agora service install` again (it updates the unit and reloads systemd) and then `systemctl --user restart agora-node`; that one restart still ends the agents of the old keeper.

### The window
- Terminal, Changes, Files and a file manager open as panes in the workspace, beside an agent or on their own: from the + of a tab group, from Ctrl+K, from an agent's header and from the right-hand icons. The file manager shows a folder tree, a sortable list, a preview and Copy path; renaming, moving, deleting and uploading are shown switched off with the reason.
- The conversation of an agent without a terminal has a new look: a header with the state and the context used, each turn with the agent's name, model and time, your requests in tinted bubbles, tool calls in one framed group per turn (an edit shows its diff inline, with line numbers), a plan card, the approval pinned above the message box, and a calmer message box.
- The Usage panel's icon is bars, not a dollar sign. Long folders in the file manager scroll smoothly.

### Safety
- A mode or option that makes an agent skip its permission questions is now refused by the node when its policy forbids it, on every way of changing a mode.

### Fixes
- On a Mac, ending an agent whose start answer was lost no longer hangs: the keeper reads what an ending agent printed so the process can leave.

## 0.3.23

### The window
- The right-hand panel is now a column of icons (Needs you with a count, Changes, Board, Messages, Usage) that stays visible on a narrow window; the panel opens beside it, or over the content on a small screen. Alt+Shift+1 to 5 open them.
- The tabs of open agents scroll instead of being cut off, and a "More tabs" menu lists the hidden ones, with a dot when one of them needs you.
- The agent list and the fleet view can be grouped by machine, project, agent kind or state, and the choice is remembered. The same project on several machines is one group, marked "Same project", and a hint appears when agents on different machines have changes in it.
- Agents that run without asking carry a short "Acts freely" badge; the full explanation is in its tooltip.
- An agent's screen shows its model and mode as menus (only what the agent reports), the context it has used and the time since it started. The start window remembers the last model and effort you chose for each kind on each machine.
- The conversation of an agent without a terminal reads like a coding tool's: your requests and its replies look different, Markdown (lists, tables, code with a copy button) is drawn safely, tool calls are folded into one line per turn, changes are cards with a file list, and a "Jump to latest" button appears when you scroll up. Older entries are loaded on request.
- A terminal agent shows a short live summary above its screen.
- "Spent" is now "Usage" and shows tokens, context and time, never money. Plan limits will appear there when the agents report them.
- Fleet cards show how long an agent has worked, and "needs you" is clearer.

### Resume
- Held terminals: the first part of keeping agents alive when the node stops is in place on Linux and macOS (not yet switched on).

## 0.3.22

### Resume
- A Claude Code agent that was started but never given a message no longer disappears when Agora restarts. Claude Code saves no session until it is first spoken to, so there was nothing to continue; Agora now starts it again under the same session instead.

## 0.3.21

### Links in the console
- Web addresses with a query string (`?a=1&b=2`) can now be opened with Ctrl+click twice. Percent-encoded links (`%20`, `%E2%80%93`) open too; a `%NAME%` shape is still refused.
- Opening an agent's terminal or conversation from the console and then closing the window (or losing the SSH session) now ends the console, instead of leaving it running unseen.

## 0.3.20

### The terminal in the console (TUI)
- A program that turned on mouse reporting (an editor, a monitor, an agent's own screen) now gets clicks, drags, releases and the wheel from the console, in the form it asked for; hold Shift to select and scroll the console's own history instead. With no mouse mode, the wheel on a full-screen program sends arrow keys (three a notch).
- A program that asked for focus events is told when its pane gains or loses the keyboard. Pastes are bracketed only for programs that asked, and a paste can no longer carry escape sequences or other control characters.
- Ctrl+click on a web address shows it in the footer; Ctrl+click it again to open it. The program's title shows after the tabs, with control and look-alike characters made visible.
- Links that contain `& % ^ " ( ) ! < > |` are not offered, and opening a link on Windows no longer goes through `cmd.exe` (an address with `&` could run a command). The console now puts your terminal back (mouse, paste, focus, cursor, screen) however it ends, including a panic or a stop signal (Unix), and a mouse button a program is holding is released when you open the palette, switch tabs or give the mouse back. Links are limited to RFC 3986 characters, need the footer to have been shown before a second Ctrl+click opens them, and on Windows open through `rundll32` from `System32`.
- Known limit: a program that was already running when you opened its tab is not known to have the mouse on until it switches it on again.

### Windows
- An argument given to an agent that is a `.bat` or `.cmd` file can no longer run a second command on Windows (the "BatBadBut" flaw, CVE-2024-24576): quotes and `%` in it reach the script as plain text, and an argument with a line break is refused before anything starts. A bare `claude` or `codex` that Windows finds as an `.exe` was never affected.

## 0.3.19

### Reaching your hub through a tunnel or a reverse proxy
- `agora remote proxy --trust-header NAME --host YOUR.NAME` lets the hub sit behind a tunnel provider's connector or a reverse proxy you run: it answers only the names you gave, takes a visitor's real address only from the one header you named (and only from a proxy on this machine or an address you listed), and `agora remote off` takes it back. It is off until you run it, and it refuses without a screen password.
- Through the proxy nobody is ever taken for the owner at the hub's machine, each visitor now has their own sign-in lockout instead of sharing one, and a request for another host name or from a web page of another site (Host and Origin, also on the live connection) is refused.
- `agora remote status` and `agora doctor` say every time that the hub is reachable from the internet through the proxy, and recommend an access layer with single sign-on in front of the hostname. `docs/guides/remote-hub.md` has the steps and says plainly what was not tried against a real tunnel.

### The window
- When an agent is blocked on a request, a card opens over the window with the full request, if the window is in front and you are not typing; otherwise a small banner counts what waits, and the Needs you list stays visible everywhere. A plain request offers Allow and Deny (dead for a moment after the card appears, after anything shown changes and when you come back to the window); Esc, a click outside and Later never allow. Start, settings, pairing and long requests only offer Review. In This device you can choose the card, a small banner only, or nothing extra.
- When the window is in the background, a browser notification says "An agent needs you" (never the request itself, a command or a path), after you turn notifications on in This device; the tab title and the app badge show the count. Naming the agent and machine in it is optional.
- The command palette can now jump to a machine or a project, show where a new machine is allowed to join, lock the window (when a screen password is set) and open Settings.
- In the terminal, "Jump to latest", the link address and the clipboard offer no longer cover each other.

## 0.3.18

### Reaching your hub from outside
- `agora remote tailscale --serve` puts your hub on your tailnet with a real HTTPS address for your phone, `agora remote status` says what reaches it and how, and `agora remote off` takes it back (only what Agora added, never your other Tailscale rules).
- Nothing outside your machine can reach the hub until a screen password is set: the remote commands, a hub on a public address and `agora password clear` while a way in is on all refuse and say `agora password set`.
- `agora doctor` now checks that Tailscale answers, that a password protects any way in from outside, that the certificate names your tailnet name, and warns every time that the hub listens on a public address.
- A server with no screen can run Agora: `agora --ui none --listen ADDRESS` sets where the hub listens, and `docs/guides/remote-hub.md` walks through a VPS with Tailscale or a plain address.
- A request that went through a proxy on the hub's machine is no longer taken for the owner at that machine, so the screen password applies to it.

### The window
- On a phone, what needs you now stays at the bottom of the screen in reach of your thumb: a plain yes or no has big Allow and Deny buttons there (greyed out, with the reason, while the connection is down), anything else has one Review button, and the title bar no longer overflows when the hub cannot be reached.

## 0.3.17

### The window
- The terminal in the window now behaves like a real terminal: the mouse reaches programs that use the common mouse modes, the wheel scrolls full-screen programs, a swipe scrolls on a phone, you can copy with Ctrl+Shift+C, find with Ctrl+Shift+F, jump back to the latest output, open links with Ctrl+click, pasted text can no longer escape its paste, emoji take two cells, Shift+Enter works in programs that ask for it, rows are as tall as the font, and a program can offer text for your clipboard (you press Copy); Terminal settings in the pane's header set row height, history, colours, copy on select and an optional sharper drawing of box and block characters.
- Home is now a fleet console: counts of what needs you, what is working, what finished, what failed and which machines are online (each one filters the list), then what needs you with Allow and Deny in the row for simple requests, then every agent as a tile grouped by machine or project, with a clear next step when there is no machine or no agent yet.

### Adding a machine
- The window and the terminal now show a new machine's request to join: its name, system and address, and a number to compare with the one on the new machine; approving takes two presses, and a hub with no machine yet can open the 10-minute window with one button.
- You can now add a machine to your hub without copying a certificate file or a code: run `agora pair open` on the hub's machine, `agora join ADDRESS` on the new one,
  compare the number both show, and answer `y` on the hub. Choice 3 of `agora`'s first question does the same (it asks for the address).
- `agora join` without an address now finds a hub that is open for new machines on your network (you confirm its fingerprint against the one `agora pair open` shows), and the hub names its own network addresses in its certificate (renewed at each `agora pair open`), so joining by the hub's LAN address works without `--name`.

## 0.3.16

### The window
- All small text in the window is now at least 12 px, and counts and times use figures of equal width so they line up from row to row.
- In a review, a deleted line is red with a "-" and a bar on its left edge, an added line has a "+" and a green bar, and amber means only "needs you" again. The review opens on the first file you can read, and files kept private are listed last.
- An agent that ended with an error is shown with a cross and "Ended with an error" in the list, its tab and its summary, and no longer with the green check of a finished one.
- Text for states (Working, Finished, Unknown) can be read comfortably on every background in the Dark, Light and other built-in themes.
- On a touch screen, buttons, tabs, list rows and menu items are at least 44 px tall, so tablets are easier to use; with a mouse nothing changes size.
- The layout menu shows each layout as a small picture with the current one marked; Export and Import moved under "More".
- A machine's id is shown next to its name only when two machines have the same name.
- An agent's request to start another agent names it as you know it ("claude · amber on laptop asks to start a codex agent") instead of an id, and the "Depth" line is gone. "Does not ask" now reads "Runs without asking" and "Stopped with the machine" reads "Stopped when the machine shut down" (also in Turkish).

### Security
- A confined Claude Code or Codex can now use a login the node keeps for it (ADR-0084, issue #546): `agora node secret set NAME` stores a token on that machine
  (never sent to the hub), a policy line `credentials.claude-code = NAME` lets that kind have it, and no other kind of agent gets it. A token in the node's
  own environment no longer reaches every kind of agent: add the policy line if you relied on that. The shipped Claude Code and Codex descriptions now need this version of Agora (`min_engine = 4`), so an older node refuses them; update the node with them.
- A confined agent on Linux can no longer use the node's control socket (ADR-0082, T16): when the node confines agents it keeps a secret
  in its own folder (`control.token`, readable only by its owner and out of every agent's reach) and answers nothing on the socket until the
  caller shows it, so an agent can no longer start agents with options nobody confirmed, stop agents or type into another agent's terminal
  through it. `agora` presents the secret itself, so no command changes; a node that does not confine agents changes nothing. A node whose
  folder lies inside a folder agents may read or write (a node under `/tmp`) refuses to start confined agents, and `agora doctor` says so.
  `agora` also tells a node that refused it apart from no node running.
- A confined agent no longer receives the node's `SSH_AUTH_SOCK`, `GPG_AGENT_INFO`, `DBUS_SESSION_BUS_ADDRESS`, `DOCKER_HOST`, `TMUX` or `TMUX_PANE`
  (ADR-0083, T8); a policy line `sandbox.pass_env = NAME, ...` lets named ones through, so `git` over ssh in a confined agent needs it
  (see `docs/security/sandbox-recipes.md`).
- A confined agent no longer gets `SSH_AUTH_SOCK`, the session bus, docker or tmux addresses in its environment, so it cannot find your
  ssh-agent, the session bus, docker or a tmux server by itself (the owner can pass a name through with `sandbox.pass_env`). On Linux kernels
  from 6.12 it can no longer connect to abstract sockets made outside its sandbox; a socket that is a file is still reachable there. On a Mac
  it can no longer connect to a unix socket it was not given (measured on a real Mac, 2026-11-17: its own socket, another one, the name
  resolver). Other system sockets a tool may need are not measured with real agents yet; `sandbox.sockets` adds one, and `agora doctor` says so.
- `agora doctor` says whether this machine restricts an agent's unix sockets (and why not), which socket variables the node leaves out of an agent's environment and which the policy passes.

### Certificates
- **A machine's certificate for the hub lasts 90 days, and nothing renews it yet.** A machine you joined on a given day cannot connect to the hub
  after day 90 (its agents keep running and its events wait), and a hub that keeps running for 90 days refuses new connections until it is restarted.
  Until renewal exists (ADR-0085, a proposal): join each machine again before day 90 (`agora hub code` on the hub, then `agora node enroll` on the
  machine) and restart the hub now and then. `agora doctor` now tells you how many days are left on this machine's certificate and on the hub's
  and warns 30 days before the end; `agora hub machines` says so under a machine that is close to its end.

### Fixes
- `agora status` says "connected" only while the node's link to the hub is up right now (it used to say so for the hub the machine was set up for, also while every attempt failed); otherwise it says not connected and why, and on a Mac whose Local Network permission is missing it points to System Settings. After `agora update` on a Mac, the note now says to restart the node once more after allowing Local Network.

### Terminals
- A program that draws its status lines with icon glyphs (WrongStack's Powerline arrows and folder, branch and settings icons) is no longer
  shown as empty boxes in the window and the desktop app: if a Nerd Font is installed on the machine that shows the window, the icons come
  from it. Letters keep the theme's own font, and a machine without a Nerd Font looks exactly as before.

### Security
- A device you trust again on the machine (`agora node trust`, or yes at `agora node enroll` / `--from-hub`) is no longer removed
  again when the hub sends an earlier revocation of it once more. This happened when the browser that revoked it had a clock running
  a few minutes fast (audit finding F220). The command now says when your yes overrides such a revocation; a revocation signed after
  your yes, or by another device, still applies (ADR-0081).
- What a device signed before it was revoked stays refused after you trust it again on the machine. Before, trusting it again let
  an endorsement it had signed earlier, kept by the hub, add another device.

## 0.3.15

### Agents
- The start window can now start an agent with a model and an effort (ADR-0066; the terminal client has no start picker yet): the window lists what the machine says each
  agent takes (a list for the effort, a text box for a model the tool checks itself), starts on "the agent's own default", offers nothing the
  machine did not list and says what it will start with before Start. The terminal's approval card shows an agent's requested model and
  effort in words and allows it over a signature that covers them; without settings nothing changes.
### Agents' mesh tools
- An agent's mesh tools no longer end for good when the node restarts: the bridge the agent's client runs reconnects to the node
  (waiting up to 2 s between tries, giving up after 15 minutes) and the agent's session stays open. A call made at that moment fails at once with
  `mesh.unavailable` and can be repeated (ADR-0075, the first step; applies to agents started by this version).
### Security
- A valid metrics token (or the owner's session) read on `/api/metrics` no longer wipes the failures of wrong sign-in and one-time-code tries
  from the same address, so it cannot be interleaved with guesses to avoid the lockout (integration review F1).
- On Linux the git settings file of a confined agent no longer lies beside its worktree, where an older confined agent that could write
  that folder could rewrite it: it is now in a private folder (`~/.agora/git-config`) that no agent can write (integration review F6).
- The audit row of an agent start now holds at most 16 launch options and settings, each cut to 128 bytes (marked), however many a
  client sent (integration review F7).
- The file browser and the review screen of the web window and of `agora tui` now show a secret file (`.env`, a private key, ...) with a lock
  and "Kept private: it may hold a secret" instead of opening it, and a changed secret file in the review as "Changed, content hidden" with its
  added and removed counts and no content; an older listing that still asks for one gets the same message instead of a raw code (ADR-0080,
  the window and terminal side; the machine's side came first).
- A large paste made of nested terminal paste markers no longer freezes the terminal client for seconds (192 KB took about 6 s): the markers
  are now removed in one linear pass with the same result (#655).
- A machine that stops reading its connection can no longer make the hub pile up commands for it without end: at most 1024 wait for one
  machine, and the next one is refused at once (`node.resource_limit`, never sent, so it is safe to repeat) (architecture review R5).
- "Up to N helpers like this" (ADR-0057) ended only when the asking agent stopped: after the device that approved it was removed
  (`agora node untrust`, a revocation from another device), ran out or was narrowed to other projects, the agent kept starting
  helpers without asking. The grant now ends for good with that device's trust, and the next start asks again (audit finding F290).

### Starting agents
- A start with a model and an effort now works through the hub too (ADR-0066): your device signs the choice together with the folder and
  the agent, so the hub cannot change, add or drop the model on the way. A machine that does not know this yet says so instead of
  starting the agent with the tool's own model.
- An agent that asks for a helper can ask for a model and an effort (`agent_start` with `settings`). The approval card in the web window
  lists them, you approve exactly what you see, and the helper starts with exactly those. The terminal console cannot approve such a
  request yet, and the start window does not offer the pickers yet.

## 0.3.14

### Starting agents
- `agora agent start codex --setting model=gpt-6.1-sol --setting effort=high` starts an agent with the model and reasoning effort you
  choose (ADR-0066). Claude Code and Codex offer `model` and `effort` (low, medium, high, xhigh, max). The machine checks every value
  before starting anything, and a value can never become another flag; it also tells the windows what each kind takes, for the start
  window to offer in a later change. For now settings work on the machine itself: a start with settings through the hub is refused until the approval card
  signs them.
### Console (`agora tui`)
- An arrow key, a click or the start of a paste could be split while the console was busy, so its tail was typed into the agent
  or the chat (`[A`, `<0;10;5M`), or a paste arrived as keys with its line breaks as Enter. A key now waits 10 ms for its last byte.
- In the review and the file browser, the mouse wheel, a click, Page Up/Down or Ctrl+arrow closed the view and typed the rest of
  the key into the console. They are now ignored there.
### Terminal client
- A terminal you watch (`agora attach`, a window through the hub) that fell behind because the agent printed a lot just
  before it ended kept waiting forever instead of showing that the agent ended. It now always hears the end, with an unknown exit
  code when the machine no longer has it (audit finding F251).
- An agent a machine cannot start is named with the program the machine looked for ("WrongStack (`wstack`)"), and when this
  device is not trusted yet the hub's owner sees the folder of that machine's `agora` program when it is not in its PATH (ADR-0069).
### Faster answers
- Every answer to something the window asks a machine (an agent list, a file, a diff, a start) arrives about 20 ms sooner: the hub
  sent it on its next 20 ms tick, and now sends it as soon as the machine answers (0.2 ms through the hub, also while another
  machine floods output).
### Panel on a node machine
- `agora panel` now opens the hub's window at `http://127.0.0.1:7480/` on a machine that runs a node, through the link the machine
  already has to the hub: no certificate to import in any browser and no trust command (ADR-0056 Amendment A). The port is kept
  across runs so your sign-in and device stay (the first run takes another free port if 7480 is in use); if it is taken later,
  `agora panel` says so instead of moving. It listens on this machine only, while the command runs (Ctrl+C closes it), and
  refuses a request meant for another name. You still sign in as from any browser.
### Desktop app
- The app's connection to the hub now also refuses a request whose `Origin` is not the app's own, next to the `Host` check it already had.
- On Windows and Linux, the desktop app shows one system notification when approvals are waiting while its window is hidden,
  minimized or not in front (the count only, in your language). Not on macOS yet: there it stays silent for now.

### Security
- Linux: a confined agent (`sandbox = worktree`) could cut any of your files outside its folders to nothing (`truncate`), such as
  `~/.ssh/authorized_keys` or code in another checkout. That is now refused on kernels from Linux 6.2; older kernels cannot refuse it
  (audit finding F260).
- Linux: a confined agent can now move a file from one folder of its worktree to another. It was refused ("invalid cross-device
  link"), so tools without a copy fallback failed; a file from a folder it may only read still cannot be moved or linked in (audit
  finding F261).
- A permission to act on a machine for some hours (a grant) now ends as soon as the device that gave it is no longer trusted
  there: removed with `agora node untrust`, revoked from another device, run out, or limited to other projects. It used to keep
  letting commands through until it expired (up to 8 hours) unless it was revoked by hand (audit finding F270). Trusting the device
  again does not bring the old grant back.
- An agent could make its machine's node take memory without limit by sending one endless line to its mesh connection
  without a newline, and the agent then could not be stopped while it kept sending. A line over 1 MiB now closes the connection as
  soon as it reaches that size (audit finding F250).
- macOS: a confined agent could read and change any app's settings through `defaults` (the system's settings service did it
  on its behalf, past the file rules). Settings now follow the same rules as files; the language and locale stay readable.
- macOS: a confined agent is not started when a file it may write also has another name elsewhere (a hard link made outside
  confinement), or when a save copy next to it (`state.json.tmp`) is such a link, since writing it would change that other file too; the message says to remove the link or put a copy in its place
  (#571). Such links inside a folder it may write are not looked for at each start.
- Team mode: a machine could show its own agents and folders to a member granted only a project on another machine, by naming
  that other machine in what it reported, and the hub then let that member's commands reach them. A member now sees an agent or
  folder only as part of the machine that reported it (audit finding F282).
- `agora node untrust` that fails partway (for example a full disk) now leaves the device trusted, so you can run it again;
  before, the device was removed but not remembered as revoked, and the hub could trust it again on its next reconnect.
  `agora hub devices forget` on a device that is not revoked no longer deletes what endorses it.
- A passkey added from another machine before the screen password was set now goes away when the password is set, also when it
  was added under the name (credential id) of a removed person's passkey. It used to take over that old passkey's place among the
  ones added at the hub's machine or with the password, so it stayed and kept signing in from elsewhere without the password
  (audit finding F211).
- Someone who can reach the hub's address can no longer stop passkey sign-in: asking for many passkey sign-ins without finishing
  them threw away everyone else's waiting one, so the owner's passkey sign-in failed and adding a passkey was refused. One address
  now keeps at most 4 waiting and only loses its own (audit finding F210).
- A connected machine can no longer speak for another one. Before, it could show another machine's running agents as done and
  give back their tasks, show that machine online or offline, change its numbers, list its own agents, folders and terminals on
  it, or hide one of its agents from every window. The hub now refuses such a message and closes the sender's connection.
  A history written before this check is read the same way: what a machine reports (that it started or left, an agent, a terminal, a folder, its
  numbers) counts for that machine only, and only the hub can hide an agent.
  A window that reconnects to a hub holding such an old record is given a fresh snapshot instead of the stored events, so it
  never applies one (and one that did before is corrected).

### Starting at login
- An agent that was in the middle of a task when its machine or Agora stopped now carries on by itself when it comes back: Agora tells it
  that its session was cut off, to check the state of what it was doing first, and to go on where it stopped. Only for an agent that was
  working, only when Agora brought it back by itself (not when you pressed Resume, and not for one that was waiting for you), once for each
  interruption, for Claude Code and Codex. `resume.continue = off` in the node's policy file turns it off.
### Console (`agora tui`)
- A paste that held pieces of the paste markers could end the paste early and type the rest as keys; it now always arrives as
  one paste.
- Dragging a selection after the window shrank so the terminal had no rows left closed the console; the drag is now ignored.
- A request for your answer that arrived while the console was waiting for something else (opening files, a review) was not
  announced; it is now.
- A debug build could stop with an arithmetic error while the inspector was hidden and an agent asked to start another.
- Esc pressed quickly before Enter, Ctrl+C, Tab or Backspace (or both arriving together over SSH) lost the second key; both
  keys now arrive.
- On a terminal that sends Latin-1 or Windows-1254 letters (`ç`, `ğ`), the keys typed after such a letter waited until more
  were typed; they now arrive at once.
### Agents working together
- A task's title, detail and summary are limited to the same size as a message (16 KiB). They used to take about a megabyte each,
  kept forever and sent to every agent that listed the tasks; an oversize text is now refused (`mesh.body_too_large`, audit
  finding F603).
- A file lock that ran out of time while another agent asked for a lock is now recorded as ended. It used to vanish without a
  trace: the history never showed it ending, and the shared board and the windows kept showing the path as held (audit finding F602).
- A task can no longer be handed to an agent of another project: it used to end up claimed by someone who could neither see
  nor finish it, stuck until its lease ran out. It is refused (`mesh.recipient_unknown`) and stays with its owner (audit finding F601).
- After the hub restarted, a file lock kept from before stayed held when the agent holding it ended or its machine restarted, so
  the other agents of a shared project could not take that path until the lock's time ran out (up to an hour). It is now freed then,
  like any other lock (audit finding F280).
- When the hub and a machine both restarted (an upgrade, say), the machine's new first agent could take over the tasks and locks
  its old first agent had held, because it got the same name: it could finish a task it never took, and others were told the task
  was taken. They now go back to the board (audit finding F281).

### Machines
- On Linux and macOS, a folder that failed while it was being read (a disk or network error) was shown as complete, or as empty; the
  machine now reports that it could not be read (audit finding F262).
- An agent started in a folder now always keeps that folder in the machine's record. When another agent ended or reported its
  session at the moment it started, the record lost the folder, and after a restart the agent could be neither resumed nor started
  again there (audit finding F271).
- A machine's control connection no longer keeps everything it receives: when a read ended in the middle of a message (the usual
  case), the part already handled stayed in memory, so a console watching a busy terminal, or the machine serving it, grew by every
  byte of output for as long as it stayed connected (4 MB streamed held 4 MB). It now holds at most one unfinished message, and
  reading got faster too (audit finding F240).
- A terminal resized to one column stopped its machine's screen for that terminal: a wide character (Chinese, Japanese,
  an emoji) crashed it, and narrowing a screen holding one never finished. A terminal's screen is now at least two columns.
- Asking for the changes of one file whose name contains `[`, `*` or `?` now returns that file only: the name was read as a pattern, so
  `pages/[id].tsx` also brought `pages/i.tsx`. Nothing outside the workspace folder was ever reachable this way (audit finding F402).
- The number of changed files a machine shows for a folder is now right when the folder holds new files: a new folder with three files
  counted as one, and as none if the repository's own settings said not to show untracked files (the repository is written by the
  agent). The count and the review screen's list now agree (audit finding F401).
- Windows: looking at changes (a diff, the changed files) no longer opens and closes a window for every `git` the machine runs,
  and no longer takes the focus. A structured agent (opencode, WrongStack, Antigravity) no longer keeps a window open for its
  whole life.
- A program can no longer make a terminal use unbounded memory with an endless title or other "operating system command". One that never
  ended was kept whole (16 MiB sent held 32 MiB), and a long title was copied thousands of times by a title-stack request. The
  parts past 1024 bytes are now left out; the sequence still ends where it ends (audit finding F704).
- On macOS a busy node could lose its control socket: a second start while the first one's queue was full took it for a dead
  leftover, deleted its socket file and bound its own. Each socket now has a lock file beside it (`<socket>.lock`, left in place,
  harmless) held for as long as it is served; a second start finds it taken and stops (audit finding F202).
- Accents and joined characters are no longer dropped from what a machine reads off a terminal: a name typed as `cafe` plus a
  combining accent, a variation selector or an emoji joiner lost its extra part in the detected text and in snapshot lines (audit
  finding F703). The terminal's own repaint still drops them.
- `agora doctor` now warns when the manifests folder holds an older description of an agent than the one built into the program.
  Such a copy (made for an older release) replaces the built-in one for good: on a real machine it kept every Claude Code agent from
  coming back after a restart. The warning names the file to remove; the shipped descriptions are built in, nothing needs copying.
- An agent that died in the middle of a screen update no longer leaves the machine's view of its terminal frozen. After a
  "synchronized update" started and never ended, everything the terminal printed afterwards was held back; it is now let go
  after 150 ms, at the next output (audit finding F702).
- Windows: a machine could stop updating its agents (memory, state) for good when a process's leftover parent id pointed back
  into the agent's own processes. Walking an agent's processes now visits each one once.
- `agora hub machines` (and `hub devices`, `hub audit`, `hub fingerprint`) on a machine that does not run the hub
  now says so and where to run it, instead of "unable to open database file".

### Backup and restore
- `agora hub restore` keeps the database it replaces as `hub.db.before-restore` so a wrong restore can be undone, but after the hub
  had crashed or been killed that copy could miss its newest events: they were still in the database's write-ahead log
  (`hub.db-wal`), which the restore deleted. The log now moves with the kept copy (`hub.db.before-restore-wal`; to undo the restore,
  move both back), so the copy holds everything the replaced database held (audit finding F232).

## 0.3.13

### Starting at login
- After a restart of the machine or of Agora, the agents that were running come back on their own and carry on in their own conversations. At most 8 start at once, the rest 5 seconds apart. The first time, Agora asks once (the default is yes); `agora --resume ask|auto` changes it. The node's own policy file always outranks the flag.

## 0.3.12

### Known issue (Windows)
- Microsoft Defender may flag `agora.exe` shortly after `agora service install`. It was seen on one machine with the 0.3.12 release
  file, whose checksum and signature `agora update` had checked (`Behavior:Win32/Execution.A!ml`, a guess from what the program does,
  not a known-malware match). Defender then quarantined the file, ended the supervisor and the node, and removed the `agora-node`
  login task, so the machine went offline and `agora` no longer ran. What you can do is in getting-started ("Windows Defender").

### Hub
- A machine whose certificate is revoked while it is connected (for example, another machine enrolled under its name) is shown
  offline at once. Its connection already ended, but every window kept showing it online until the hub restarted.
- `agora hub run --metrics` lets the hub's owner read its own numbers at `/api/metrics` (Prometheus format): events stored, how long
  each one held the hub, machines online, windows following, slow windows let go, approvals waiting and resyncs. Off unless asked for.

### Security
- macOS: a confined agent (`sandbox = worktree`) can no longer reach the node's own files and sockets: the whole node home is closed to it (trusted devices, policy, ledger, identity, logs, hub folder, manifests, adapter trust files, the node's control socket and the other agents' mesh sockets), except its own worktree and mesh socket; the home cannot be renamed away, and no socket can be bound there before the node's own. Before, it could connect to the control socket (Seatbelt checks unix sockets as network, not as files), and with a node home under `/tmp` it could read and write the node's data (#475). Its own mesh socket stays open. Its worktree is given back only while it is still a real folder: an agent that leaves a link in its place is not started again, and says why. A node home that cannot be found stops confined agents from starting instead of leaving the home open, and your own home folder is never taken for the node's. A confined agent that replaces a file or folder it may write with a link (to `~/.zshrc`, to your home) is not started again: before, its next start could write where the link pointed, also when another agent swapped the link in while it started (#545). The message says to check that link and remove it if you did not make it; a file you keep there as a link yourself (a dotfiles link) is refused too. A folder named in the policy through `/System/Volumes/Data/...` is now closed or opened as named; before, such a rule matched nothing.
### Machines (ADR-0073)
- A machine not seen for more than a day is shown as **old**: on the Machines page in its own folded section, and no longer offered
  for new agents. **Clean up…** forgets all old machines in one dialog, and **Forget…** forgets one offline machine. Forgotten
  machines leave every window, while their history and costs stay; one that connects again comes back. A box also refuses them if
  they connect again, for a lost or retired machine. `agora hub machines` lists machines as online, offline or old, and
  `agora hub machines forget NAME` or `forget --old` does the same from the hub's machine. Only the hub's owner can forget machines.

### Starting at login
- `agora service install` makes this machine's Agora start when you log in: for you only, with no administrator rights, and it
  says what it registers first. `agora service status` says whether it is set up and whether it runs now; `agora service remove`
  stops it and removes it. On Windows (a logon task) and on Linux with systemd (a user unit in `~/.config/systemd/user`, no root;
  to start it at boot without logging in, run `loginctl enable-linger` yourself, which Agora never does), and on macOS (a
  LaunchAgent in `~/Library/LaunchAgents`; macOS may say "Background Items Added" and lists it under Login Items, where switching it
  off is shown by `status` and only you can switch it on again). A login entry of the same name that Agora did not write is left
  exactly as it is, and `status` says it is not Agora's. The service gets only the full-path entries of your `PATH`.

### Getting started
- A hub or node started by `agora` that crashes is started again 30 seconds later, at most 5 times in 10 minutes; one that
  stops right after starting (a port taken, a wrong setting) is not retried and its log says why. `agora status` shows a restart
  in progress, the restarts of the last 10 minutes and a give-up. `agora down` and a killed supervisor never leave a hub or node
  running behind them. A hub or node started by an earlier version stays as it is until `agora down` and `agora`.
- Agents come back by themselves after a restart (ADR-0062): when this machine's Agora starts again, the agents that were running
  continue their own conversations in their folders, 8 at once and the next 8 a few seconds later (at most 16; any more wait for
  Resume in the window). The first run asks once ("Bring agents back by themselves after a restart? [Y/n]", Enter is yes); with a
  no, the window still offers to bring each one back. A machine set up with an earlier version is asked once at the next `agora` in
  a terminal and changes nothing until then. `agora --resume ask` or `--resume auto` changes it for one start, and with `--save`
  from the next start on. A `resume` line in the node's own policy file always wins. An agent that cannot continue its own
  conversation, or that was brought back by itself in the last 10 minutes, is never started again by itself. Run by hand,
  `agora node run --resume auto` does the same.

### Shared board
- A change to a shared project's board (create, claim, complete, lease, release, handoff) is answered as done only once the hub has
  stored it. If the hub cannot write it (a full or locked disk), the agent gets an error and the board stays as it was, so a claim an
  agent was told it has can no longer vanish when the hub restarts.
### Hub
- A window or terminal client that comes back after a long time away (more than 10 000 events or 8 MiB behind) gets a fresh snapshot
  instead of replaying everything it missed. Its return no longer holds up the hub for every machine and every other window while
  the backlog is read (100 000 events behind took about 155 ms of the hub's time; now well under a millisecond).
### Terminal client
- The terminal client's wheel scrolls an agent's tab back through what it showed, with how far back at the top; typing goes back to the bottom.
- Text dragged over in a tab scrolled back is the text shown there, earlier lines included, and that is what is copied (before, the
  highlight and the copy were taken from the live screen).

## 0.3.11

### Terminal client (ADR-0063)
- `agora tui` is drawn with ratatui: machines and their agents on the left, the agents you open as tabs with their live terminal in the middle (what you type goes to the agent; Ctrl-] or F12 comes back to the list; Alt+1…9 picks a tab), and the selected agent's details and the request it waits on on the right. The mouse works: click a row to select it and again to open it, click a tab, drag the dividers; `m` releases the mouse so the terminal can select text. A chat agent opens as a tab too: its conversation, and a line to type a request into (Enter sends it; `/mode` and `/model` switch, as before). Ctrl K opens the commands, N goes to the next agent that needs you, and the letters of the earlier console stay (y / n answer, s start, f folder, x stop, u / U resume, d remove). A request to start an agent can be allowed only once all of it was on screen, in the details or in the reader that y opens. Their questions (which agent to start, allow this machine, stop its helpers too, approve a device) open in a box over the console instead of clearing the screen. In a window of 80–140 columns the agent's terminal now gets the room: the details panel shows itself only when the terminal keeps at least 60 columns (i shows or hides it at any width), the footer shows whole keys with Ctrl K first, and Ctrl K also lists every letter's action (start, stop, allow…). Dragging over an agent's terminal selects text and copies it to the clipboard (OSC 52, over SSH too). Colours follow the terminal: 16 colours where `TERM` has no 256, the 256-colour palette without `COLORTERM=truecolor` (macOS Terminal), and "needs you" in an orange that reads on light and dark backgrounds. Inside a tab, Alt+K opens the commands, Alt+M takes or releases the mouse, and Alt+Y / Alt+N answer the request of the agent in front. r shows what the selected agent changed: its diff, coloured, scrolled with the arrows and Space. o browses its files: a folder's entries, Enter into a folder or a file, ⌫ up. Typing in a chat never answers a request (Alt+Y / Alt+N do, and the draft is kept); the diff, the files and the readers follow a resized window and say when a new request arrives; and `agora tui` ends when its terminal closes.
- A chat agent's tool calls read as words, as in the web window: "Command cargo test", "Read src/auth.rs", "Search TODO", instead of the raw `{"tool": …}` text.
### Machines
- A machine gathers an agent's usage reports for 5 seconds and sends one. A busy agent used to send one after every model call:
  on the maintainer's hub that was 96% of all events, and the hub reads all of them again each time it starts. Totals are unchanged,
  the window shows usage at most 5 seconds later, and the hub's start stays fast for years of use instead of months. When a
  machine's node is stopped or restarted, up to the last 5 seconds of usage are not counted.

### Machines
- `agora doctor` says who may make this machine act: which commands sent through the hub need a signature from one of your devices
  (and a note when none do, since whoever controls the hub could then send them), how many devices the machine trusts and how many
  of them may act in every project, whether agents may skip their permission questions, and any policy line the node did not
  understand.
- After the hub restarts a machine that has not come back is shown offline, with "last seen" from when the hub last heard
  from it. Before, a machine that was connected when the hub stopped abruptly (killed, power loss) stayed online until it
  redialled, or forever if it never did.

### Getting started
- `agora --version` prints the version.

### Terminals
- A terminal open in two windows at once (say a laptop and a bigger screen elsewhere) now fits both: it takes the size of the
  smaller window, the bigger one shows it at its top left with a thin line where it ends, and it grows back as soon as the smaller
  window closes it (ADR-0072). Before, the window that resized last won and the other one was cut off.

### Agents working together
- An agent asking who is in its project (`agents_list`) now sees the project's agents on your other connected machines too, not
  only the ones on its own machine, named the way it messages them (`<machine>/<agent>`). Agents of projects only another machine
  has are never shown. When the hub cannot be reached the list still answers with this machine's agents and says that others may
  be missing (`partial`).

### Reviewing changes and files
- Changes larger than 512 KiB say how many files were read and at least how many are not listed, in the review screen and in the
  file browser, so the "files changed" count never silently disagrees with the list.
- For an agent working in a folder inside a repository, every changed file in the review is named from that folder, so opening a
  changed file, its own diff or its unchanged lines works. Before, files git already tracked were named from the repository's
  top, and showing one of them alone found nothing.
- For an agent working in a folder inside a repository, the "files changed" count covers only that folder, like its list of
  changes. Before, it also counted files changed elsewhere in the repository.
- A very long change shown whole ("Show all") and switched to side by side no longer holds the page for seconds: only the rows in
  view are drawn.
- Files in a folder whose name contains " b/" keep their real path; lines that changed only between LF and CRLF are noted as such;
  finding in a file marks the right characters after letters such as `İ`.
- The file tree works from the keyboard as one Tab stop: arrows, Right and Left to open, enter, leave and close folders, Home and End.

## 0.3.10

### Machines
- On macOS a machine no longer keeps a CPU core busy while anything stays connected to it: each idle connection to the node (from the terminal client, an agent's bridge or the command line) spun at 100%. A connection accepted from the node's listener now waits for data as it should.

## 0.3.9

### Themes (ADR-0068, rung 2)
- The window can be restyled with themes: Dark, Light, High contrast, Large text, Slate and Sand are built in, and **Edit theme**
  (command palette, or This device → Appearance) opens the theme editor beside your agents. Customize a built-in theme to change
  its colours, fonts, text size, spacing and corners and see the whole window change as you edit; a change that would make text
  hard to read (below WCAG AA) is listed and not applied. Choose one theme for light and one for dark and follow the system to
  switch between them. Themes are kept in this browser, and export and import as a JSON file (`docs/ux/themes.md`, version 1;
  a file with an unknown or invalid token, or unreadable colours, is refused with the reasons).

### Devices
- The hub keeps a list of each person's devices (ADR-0061): a browser or the terminal client registers its key and name when it signs
  in, and `agora hub devices` lists them with the fingerprint people compare, the person, and when each was registered and last used.
  The list is only a list of candidates: a machine trusts none of them because the hub lists it. A device can ask another device of
  the same person to approve it, and the hub carries the signed approval (an endorsement) or removal to the machines, which verify it
  themselves; machines report which devices they trust, so the window can show it.
- Trusting your devices on a machine is one step (ADR-0061). `agora node enroll` now lists the devices registered on the hub, each as
  name · fingerprint · when it was registered, and asks on that machine's terminal "Trust these devices on this machine? [y/N]"
  (a script or any terminal that cannot answer is a no). `agora node trust --from-hub` asks the same on a machine that is already
  enrolled, and `--yes --fingerprint FP` (repeatable) trusts exactly the devices with those fingerprints without asking.
  `agora node untrust ID` stops trusting a device at once. `agora node trust ID KEY` works as before, and a newly trusted device
  is used without restarting the node.
- A machine accepts an approval of a new device (an endorsement) only from a device it already trusts, for that machine, signed and
  not expired: it lasts at most 90 days, is renewed each time the device is used there, and never outlasts the device that approved
  it. A removal signed by a trusted device takes effect at once and an older approval cannot bring the device back. Each one the
  machine applies or refuses is written to `<data>/device-trust.log`. The trusted-devices file keeps its format (owner-only, one
  device per line; lines add `since=`, and an approved device's line `expires=` and `via=`).
- What the machine shows before you say yes cannot be faked by the hub: names are shown without control, escape, bidi or hidden
  characters and cut at 40 characters, and each device's fingerprint (now 96 bits, `f947-3cb5-e304-81be-ecf2-64cd`) is computed on
  the machine and printed on its own line; `--fingerprint` takes the whole fingerprint. Only devices registered with their own
  signature are offered, only the hub owner's by default (other people's need `--include-others` and their fingerprints), and a
  device whose id the machine already trusts with another key needs `--replace`. Removing a device also removes the devices it
  approved; approvals are at most two deep and never outlast the approving device; a removal the hub sends again never undoes a
  newer approval or `agora node trust`; changes to the trusted-devices file wait for each other across processes.
- A device limited to some projects on a machine (`device.<id>.projects`) can approve only a device limited to the same projects,
  and can remove only itself or such devices. A removal signed right after an approval always wins, even if the approval was dated
  a few minutes ahead, and approvals or removals dated in the future are refused. A machine is offered only the devices of the
  people who may work on it. A machine's policy can narrow such a device's limit but never widen it, only the device that
  approved a device can renew that approval (keeping its limits), and approvals reach only the machines their person works on.
  Limiting or narrowing a device on a machine limits the devices it approved too, at once; the device that approved a device can
  renew it even after it ran out; another trusted device can take it over only if it is at least as wide as the device and the
  one that approved it, and only with an approval signed after it ran out
  (by the machine's clock). Approvals a device signed before it was removed stay refused, even if it is trusted again.
- The window registers this browser with the hub when it connects. "This device" shows its fingerprint, which machines trust it,
  "Ask another device to approve" for the ones that do not (the command to run on the machine stays as the fallback), and your
  other devices with their fingerprints, where they are trusted, and Remove. On a device the machines already trust, a card shows
  a device waiting for approval with its name, fingerprint and the machines it would be trusted on; Approve signs it with that
  device's key. The start window offers "Ask another device to approve" when a machine does not trust this browser and another of
  your devices could approve it there.
- The terminal client registers its key too, shows its fingerprint when a machine does not trust it, asks another device to
  approve it with `a`, and with `v` shows a device waiting for approval (name, fingerprint, machines) and approves it with `y`.
- `agora node untrust` says what happened in a sentence and no longer prints its raw log line; the record stays in
  `<data>/device-trust.log`. `agora doctor` says the node reads its policy file only when it starts and warns when the file changed
  after the node started. The window says an approval for a project this device may not answer for is refused for good instead of
  "Try again", and shows each device with the end of its id (`Chrome · Windows · 3f2a`) so two browsers of the same kind can be told apart.

### Starting agents
- When a machine does not trust your browser yet, the start window now shows the exact command to run on that machine, with a Copy button, instead of pointing to another page. The same folder spelled several ways on Windows (`D:\x`, `D:/x`, `\\?\D:\x`) is listed once.
- A device limited to some projects on a machine can now start, resume, inspect and browse folders inside its own projects there (folders the machine already knows for them, and folders inside those); elsewhere it is refused, as before, and the window now says the device is limited to other projects instead of "The folder could not be selected".

### Machines
- A machine that goes quiet without its connection ending (a laptop that slept, a stopped process, a Wi-Fi switch, a NAT that forgot the connection) is shown offline within about 25 seconds, with when it was last seen, in the machine list, its tooltip, the Machines page and the terminal client. When it is back it is online again by itself, and its agents are untouched: offline is not finished (ADR-0070). Nodes ping the hub every 10 s, drop a hub that stopped answering after 25 s, give up a connection attempt that hangs after 25 s and reconnect at once after the machine wakes from sleep. The window pings the hub every 15 s and reconnects when nothing comes back within 10 s. Older nodes and windows keep working with a new hub, and a new window with an older hub shows machines as before.
- After the hub restarts, machines and the open window are back within about a second and a half, so something that happens right after the restart (a new agent) shows up at once instead of up to 16 to 30 seconds later. Machines retry a hub that is away at most every 1.5 s for its first five minutes (then up to every 30 s), and start over after each good connection; a machine dialling a hub on the same machine gives each attempt half a second, so on Windows a refused attempt no longer takes two seconds; the window retries at most every 1.5 s (5 s after about five minutes) and no longer says "Connecting to your hub…" while it is connected and nothing new has happened.
- Connecting to a hub at `localhost` no longer waits about two seconds on Windows: nodes, the terminal client, the desktop app and the hub's notifications try `127.0.0.1` first for a loopback name and `::1` only after it.
### Getting started
- Plain `agora` now opens the browser window by default on a desktop. The terminal client is the default only when there is no screen (an SSH session, or Linux with no display); `--ui tui` and `--save` still choose it.
### On a phone (ADR-0067, rung 2)
- On a phone the window is one column: what needs you first, with Allow and Deny one tap away (signed by the phone's device key and
  checked on the machine, as everywhere), then each machine with its agents; tapping an agent opens its terminal or conversation
  with Back at the top and **Next** to the next thing that needs you. Held sideways it works the same. Tablets and desktops keep
  the full window.
- On a touch screen every terminal has a key bar: Esc, Tab, Ctrl (for the next key), the arrows and Paste, sending what a program
  expects (arrows follow the program's cursor mode; Ctrl+c is `0x03`). The touch keyboard stays open while you press them.
- Every control on a phone is at least 44 px; the window keeps out of the notch and the home indicator and stays above the touch
  keyboard. Installing it to the home screen gets PNG and maskable icons and an iOS icon.
- With the hub's `public-url` set, a notice about an agent that waits or finished links to that agent; an approval's link opens the
  agent that asks. See `docs/ux/phone.md`.

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
- Codex waiting for approval to run a tool of an MCP server ("Allow the agora-mesh MCP server to run tool …?", codex-cli 0.169.0) is shown as needing you, not as idle (manifest rule `mcp_tool_approval`, from a screen captured on macOS).
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
- `agora agent start` in a folder whose project identity changed (after `agora project init`) says so and names `--accept-project-id-change`, instead of "The request was not valid (field: directory.identity_changed)".
- `agora node trust` no longer tells you to restart the node: the node reads its trusted devices again when the file changes.

### Window
- After a hub outage the window comes back within 5 s of the hub (it waited up to 30 s between attempts, and an attempt made while the hub was frozen or asleep could hang for a minute after it was back: an attempt that has not connected within 8 s is now given up and tried again), sooner when the browser is back online, the tab becomes visible or the window gets the focus, and the "Hub unreachable" banner has a "Retry now" button.

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

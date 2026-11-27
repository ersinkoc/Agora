# Task board

Per project. Backed by the mesh task model (`docs/specs/mesh-tools.md`).

- Columns: Open, Claimed, Done (and Blocked as a filter). The web window also opens a read-only board view (ADR-0095 step 1) from a project card in the Board panel, with the columns Backlog, Doing and Done, 200 cards per column plus "Show more"; task text is shown as plain text, cut short. Its look (redesign PR 12): the dialog is an overlay (raised surface, one shadow) with the kit's header (an icon chip, the title, an icon close); each column is a quiet frame headed by a mark in its hue (Backlog neutral, Doing working blue, Done finished green), its name and its count as a grey tag; the tasks are kit cards on surface-1 over the board's surface-0; the owner's state is the kit status pill; an empty column says so in a dashed frame, and a board with no tasks is the kit's empty state. "Give to an agent" is a small ghost button in the accent. Screens: `docs/ux/screens/polish-redesign-board-{dark,light}.png` (and `-phone-`).
- **Give to an agent** (ADR-0095 step 2): a Backlog card has the action (keyboard reachable; on a touch screen it is a 44 px target), and every
  Backlog task is also found in the command palette as "Give to an agent: <title>". The picker lists only the running agents of the task's
  project on a connected machine, helpers right under the agent that started them; plain terminals and other projects' agents are never
  offered. Each row shows the agent's name, kind, machine, state, and the acts-without-asking sign (a red bolt with its meaning as tooltip and name) for an agent that acts without asking (started so, or now in a mode or setting that skips
  approvals). A running agent whose last turn is Finished is between turns and is offered; a closed or interrupted one is not. The
  confirmation names task, agent, kind, machine and project in full and, for an agent that acts without asking, says so; nothing is sent
  before "Give the task". The confirmation shows the agent as it is now (a switch to or from acting freely appears while it is open) and
  checks it again when sending: an agent that stopped, was closed or interrupted, or left the project is not sent to, and the dialog says so. The answer is "Sent to <agent>", "Sent ... it waits until the agent's current turn ends" (queued), or the refusal
  in plain words (taken or done, waits for another task, gone, not of this project, the agent's queue is full, the terminal program cannot
  take a pasted task, the board cannot be reached, too many tasks given in a minute). A machine that does not offer `task.dispatch` is not
  offered again once it refused, until it goes offline or the window reconnects. Nothing is focused on "Give the task" and a held key
  never activates anything in the dialog; while the task is on its way the dialog stays open. A limited device is asked for a one-hour grant on that agent, and the dialog shows the answer once it is allowed (dismissed: nothing was sent, try again; while Allow is being
  signed and sent the question cannot be dismissed, and the task is sent again only for the question still being answered).
- **Start a new agent for this task** (ADR-0095 step 3, decision 3 "or start a new one"): in a window that can start agents the picker has
  this entry under the list of agents, and when no agent of the project can take the task it is the picker's main action (so the card and
  the palette offer "Give to an agent" for such a task too). It is offered only when the task's project has a folder on a connected machine
  that was not heard to lack `task.dispatch`, and the start window offers only such machines, so an agent started from here can be given
  the task; if its machine still turns out unable, the dialog says so (not that the agent stopped). It opens the ordinary "Start an agent" window, naming the task as plain
  text, with only the task's project folders under "Known folders"; the machine and folder are prefilled as an ordinary start prefills
  them and the person still chooses the folder, the agent, its settings and how it acts (the skip-permissions confirmation included). It
  offers agents only (a plain terminal cannot take a task), does not offer "Create a shared project identity" (a new identity would make
  the folder another project; the folder is inspected without one), and does not start in a folder of another project. Escape closes
  only the topmost dialog: the board never closes under the picker or the start window. Starting is the ordinary
  signed launch under the machine's policy: this device signs each launch (folder, kind, launch options and settings), exactly as "Start
  an agent" does, and there is no shortcut. The start needs no separate one-hour grant; only giving the task can (a `signed_commands`
  line naming `task.dispatch`, ADR-0041), and then the Give confirmation asks for it as above. A refusal stays in the start window (with
  the trust command and the rest, as always); going back shows it in the picker, and nothing is given. Once it started,
  the window stays on the board (it does not open the new agent) and, as soon as the new agent (that machine and id) shows up, the same
  confirmation follows for it; only "Give the task" sends it. After 60 seconds without it the dialog says it has not appeared yet and
  offers Back (it still follows the agent). An agent that is still starting gets it when it is ready (the machine holds it; the answer then says
  "it waits until the agent is ready and its current turn ends"). Starting a helper under a parent is not offered: the window has no way
  for a person to start a helper (helpers are agent-made, ADR-0057), so decision 8's "start a new one under a parent" stays the agent's path.
- **The terminal client** (`agora tui`, ADR-0095 step 2): `b` (also in Ctrl-K) opens the board of the selected agent's project
  whole-screen: Backlog, Doing and Done with their counts, 200 cards per column and the rest counted, titles as plain text on one line,
  cut short (control, escape and direction characters dropped), Doing with the agent that holds the task and its state. Enter or `g` on a
  Backlog task is "Give to an agent": the same agents as the web window's picker (running agents of the project on a connected machine,
  no plain shells, not closed or interrupted, helpers under their parent), each with kind, machine, state and "does not ask" when it acts
  without asking (a launch option, a current mode that skips approvals, or a config option on a value that skips them: the hub replays
  each running agent's last config options right after its snapshot, and the terminal keeps them and every later mode change; a change redraws
  the picker, and redraws an open confirmation, where only a `g` pressed after the redrawn one gives the task). Enter on an agent opens the confirmation, which names task, agent, kind, machine and project and says
  when the agent acts without asking (the question, the task, that warning and the keys always come first; the other rows are
  left out when the window is small, and a window too small even for those says so and does not give the task); only a `g` pressed after it is on the screen gives the task (Enter, the key that chose the agent,
  never does, keys typed before it are set aside, and a paste or several keys at once never answer it). The footer then says "Sent to
  ...", "... it waits until the agent's current turn ends", or the refusal in words; the agent is checked again when sending. A machine
  that refused with `protocol.capability_missing` (`task.dispatch`) or `protocol.invalid_message` (field `command`) is not offered again
  until it goes offline, a snapshot shows it with another build or program (it was updated), or the link to the hub is lost. A limited device is asked the console's one-hour grant question for that agent
  (`task.dispatch` only) and the task is sent again once it is allowed. The board shows the snapshot the console had when it opened: a
  task given from it moves to Doing at once (the keyboard stays on it), other changes show when the board is opened again. Starting a new agent for the task is not
  offered in the terminal (start one with `s` or `f`, then give the task).
- Card: title, owner agent or human, dependencies, linked branch/worktree, messages count.
- Humans can create tasks, assign them to an agent kind or a specific agent, and reorder priority.
- Assigning to an agent kind can start a new agent automatically on a chosen node (policy permitting).
- **Task handoff delivery** (ADR-0121). The owner sees waiting and unconfirmed deliveries on the task card. The latest
  finished delivery appears when there is no active one. Earlier results appear under **Handoff history**. A refused delivery
  includes its stable error code. Acknowledging an unconfirmed delivery frees its place in the queue and keeps its outcome in
  history. The task stays with its current owner.

The Home sidebar groups a board by its logical project identity. Its folder name is a display label;
two different projects with the same name stay separate. Each card shows its full selectable **Project identity**
and an expandable **Machines and folders** list with enrollment IDs and paths. Copies of one project share a card,
even when machine host names match. Git subdirectories describe the same checkout root; non-Git folders retain
their own paths. A selected-directory agent's held paths appear under its workspace's project as well.


**Review** (ADR-0125). A shared board with Review enabled has a Review column. Select **Read report** to see the agent's
summary and references. The hub owner can **Accept** the report into Done or **Return to Backlog** so the task can be
given again. Dependencies stay blocked until the prerequisite is accepted. The terminal client still uses its three
columns and shows pending reports in Doing.


On a Review-enabled shared board, agents can create subtasks (ADR-0126). A child card shows the parent task
and stays in the column for its own status. The parent card counts accepted direct children out of all
its direct children. An agent can take an open child whenever its task dependencies are done.

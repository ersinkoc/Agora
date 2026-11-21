# Task board

Per project. Backed by the mesh task model (`docs/specs/mesh-tools.md`).

- Columns: Open, Claimed, Done (and Blocked as a filter). The web window also opens a read-only board view (ADR-0095 step 1) from a project card in the Board panel, with the columns Backlog, Doing and Done, 200 cards per column plus "Show more"; task text is shown as plain text, cut short.
- **Give to an agent** (ADR-0095 step 2): a Backlog card has the action (keyboard reachable; on a touch screen it is a 44 px target), and every
  Backlog task is also found in the command palette as "Give to an agent: <title>". The picker lists only the running agents of the task's
  project on a connected machine, helpers right under the agent that started them; plain terminals and other projects' agents are never
  offered. Each row shows the agent's name, kind, machine, state, and the "Acts freely" badge for an agent that acts without asking (started so, or now in a mode or setting that skips
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
  the palette offer "Give to an agent" for every Backlog task then). It opens the ordinary "Start an agent" window, naming the task as plain
  text, with only the task's project folders under "Known folders"; the machine and folder are prefilled as an ordinary start prefills
  them and the person still chooses the folder, the agent, its settings and how it acts (the skip-permissions confirmation included). It
  offers agents only (a plain terminal cannot take a task) and does not start in a folder of another project. Starting is the ordinary
  signed start command under the machine's policy and any `signed_commands` line (ADR-0041); there is no shortcut. A refusal stays in the
  start window (with the trust command and the rest, as always); going back shows it in the picker, and nothing is given. Once it started,
  the window stays on the board (it does not open the new agent) and, as soon as the new agent shows up, the same confirmation follows for
  it; only "Give the task" sends it. An agent that is still starting gets it when it is ready (the machine holds it; the answer then says
  "it waits until the agent is ready and its current turn ends"). Starting a helper under a parent is not offered: the window has no way
  for a person to start a helper (helpers are agent-made, ADR-0057), so decision 8's "start a new one under a parent" stays the agent's path.
- Card: title, owner agent or human, dependencies, linked branch/worktree, messages count.
- Humans can create tasks, assign them to an agent kind or a specific agent, and reorder priority.
- Assigning to an agent kind can start a new agent automatically on a chosen node (policy permitting).
- Handoffs appear as card history.

The Home sidebar groups a board by its logical project identity. Its folder name is a display label;
two different projects with the same name stay separate. Each card shows its full selectable **Project identity**
and an expandable **Machines and folders** list with enrollment IDs and paths. Copies of one project share a card,
even when machine host names match. Git subdirectories describe the same checkout root; non-Git folders retain
their own paths. A selected-directory agent's held paths appear under its workspace's project as well.

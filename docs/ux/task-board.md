# Task board

Per project. Backed by the mesh task model (`docs/specs/mesh-tools.md`).

- Columns: Open, Claimed, Done (and Blocked as a filter). The web window also opens a read-only board view (ADR-0095 step 1) from a project card in the Board panel, with the columns Backlog, Doing and Done, 200 cards per column plus "Show more"; task text is shown as plain text, cut short.
- **Give to an agent** (ADR-0095 step 2): a Backlog card has the action (keyboard reachable; on a touch screen it is a 44 px target), and every
  Backlog task is also found in the command palette as "Give to an agent: <title>". The picker lists only the running agents of the task's
  project on a connected machine, helpers right under the agent that started them; plain terminals and other projects' agents are never
  offered. Each row shows the agent's name, kind, machine, state, and the "Acts freely" badge for an agent that acts without asking. The
  confirmation names task, agent, kind, machine and project in full and, for an agent that acts without asking, says so; nothing is sent
  before "Give the task". The answer is "Sent to <agent>", "Sent ... it waits until the agent's current turn ends" (queued), or the refusal
  in plain words (taken or done, waits for another task, gone, not of this project, the agent's queue is full, the terminal program cannot
  take a pasted task, the board cannot be reached, too many tasks given in a minute). A machine that does not offer `task.dispatch` is not
  offered again once it refused; a limited device is asked for a one-hour grant on that agent. "Start a new one" is step 3.
- Card: title, owner agent or human, dependencies, linked branch/worktree, messages count.
- Humans can create tasks, assign them to an agent kind or a specific agent, and reorder priority.
- Assigning to an agent kind can start a new agent automatically on a chosen node (policy permitting).
- Handoffs appear as card history.

The Home sidebar groups a board by its logical project identity. Its folder name is a display label;
two different projects with the same name stay separate. Each card shows its full selectable **Project identity**
and an expandable **Machines and folders** list with enrollment IDs and paths. Copies of one project share a card,
even when machine host names match. Git subdirectories describe the same checkout root; non-Git folders retain
their own paths. A selected-directory agent's held paths appear under its workspace's project as well.

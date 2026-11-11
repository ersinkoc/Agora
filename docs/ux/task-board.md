# Task board

Per project. Backed by the mesh task model (`docs/specs/mesh-tools.md`).

- Columns: Open, Claimed, Done (and Blocked as a filter).
- Card: title, owner agent or human, dependencies, linked branch/worktree, messages count.
- Humans can create tasks, assign them to an agent kind or a specific agent, and reorder priority.
- Assigning to an agent kind can start a new agent automatically on a chosen node (policy permitting).
- Handoffs appear as card history.

The Home sidebar groups a board by its logical project identity. Its folder name is a display label;
two different projects with the same name stay separate. Each card shows its full selectable **Project identity**
and an expandable **Machines and folders** list with enrollment IDs and paths. Copies of one project share a card,
even when machine host names match. Git subdirectories describe the same checkout root; non-Git folders retain
their own paths. A selected-directory agent's held paths appear under its workspace's project as well.

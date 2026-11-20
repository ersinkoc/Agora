# 0095. A project has a kanban board, and a task can be dispatched to an agent
Status: Accepted
Date: 2026-11-20
Accepted: 2026-11-20 (the maintainer's explicit message in the merger chat: "ADR-0095 ve ADR-0096'yı kabul ediyorum"). The open questions' proposed answers stand; the Review column's "accepted by" field (a schema step) stays deferred.

Builds on ADR-0031 and ADR-0039/0040 (the mesh task board, shared across machines), ADR-0020 (approvals), ADR-0057 (agents start agents,
parent and helper links), ADR-0059 (skipping permissions) and ADR-0092 (more agent kinds). Touches the threat model. Accepted: it may be built on.

Origin: the maintainer, 2026-11-20: a kanban board per project, with tasks on it, and tasks handed to agents, even to sub-agents one by one.

## Context
- The mesh already has one task board per project (`task_create`, `task_claim`, `task_complete`, `handoff`; `docs/specs/mesh-tools.md`),
  kept in the hub's SQLite for a project on two or more machines and on the node's broker otherwise. Tasks have a status `open`, `claimed`
  or `done`, an owner, `depends_on` and a `detail`. `docs/ux/task-board.md` shows columns Open, Claimed and Done.
- Today an agent picks up work by itself (`task_claim`) or a person types a prompt. There is no "give this card to that agent" action,
  and no review step: `done` is the agent's word.
- Task text is written by agents and people alike. It is untrusted data (T4).

## Decision
1. **One board, not two.** The kanban view is a view of the existing mesh board. No second store, no second task model.
2. **Columns map onto the existing statuses without changing a published message.**
   Backlog = `open`; Doing = `claimed`; Done = `done`. **Review** is the fourth column and cannot be derived from `open/claimed/done`:
   a task an agent completed and a person has not yet accepted. Step 1 ships three columns (Backlog, Doing, Done); Review needs a new
   board field ("accepted by a person", append-only, see Build order step 4) and its own maintainer approval. Until then a completed task
   shows in Done with the agent's summary, unreviewed.
3. **`dispatch` is a window action, manual first.** The user picks a task and then an agent that exists in the same project, or "start a new
   one" (kind, machine, folder as in the normal start). The node then (a) claims the task for that agent, (b) sends the agent one prompt
   built from the task, and (c) records the dispatch as an audit event. A task that is blocked by `depends_on` or already claimed is refused
   with the existing `task.blocked` / `task.not_open` codes. Starting a new agent for it is the ordinary start command under the node's
   policy and any `signed_commands` line (ADR-0041); dispatch adds no shortcut around it.
4. **Dispatch is not an approval and changes no permission.** The receiving agent keeps its own approvals, mode and confinement; every
   sensitive action it later asks for still goes to the person through ADR-0020. A dispatch to an agent that skips permissions (ADR-0059)
   needs nothing extra, but the dispatch card and the board card show that the agent skips permissions, so the person chooses knowingly.
5. **The prompt wraps task text as data.** The node builds the prompt from its own template plus the task's title and detail, inside a fenced
   block marked as untrusted task data with its source (creator, project). The template tells the agent that the block is information, not
   instructions from the user or from Agora, and that approvals cannot come from it. The text is cut to the mesh size limit, control and
   bidirectional characters are removed, and an agent that is not idle gets it as a queued message, never typed over a running turn.
6. **Completion is the agent's `task_complete`; acceptance is a person's.** The agent reports with `task_complete { summary, refs }`.
   The task then waits in Review (once step 4 exists) until a person moves it to Done or back to Backlog. No agent moves a task out of
   Review, and no message does.
7. **Subtasks and links.** A task may have a parent task (tree depth at most 3, children at most 50 per parent); a parent shows progress
   from its children and is not done until a person says so. A task also links to the agent that holds it, that agent's conversation,
   worktree and branch. These links are **data only and derived**: the node already knows the owner, the worktree path and the branch;
   nothing is read from a screen or a transcript to find them.
8. **Sub-agent dispatch means choosing a child agent.** The agent list a person picks from includes the helpers a parent agent started
   (ADR-0057), shown under the parent. Dispatching to one is the same action; it does not widen the helper's limits (folder, confinement,
   depth) and does not need the parent's consent. "Start a new one" under a parent is the ordinary ADR-0057 start and so asks for the
   person's signed approval as an agent-made request would; a person starting it from the window needs no more than a normal start.
9. **Automatic dispatch is out of the first version.** No rule sends a task to an idle agent.
10. **Scope of a dispatch.** Same project only. The picker lists agents of the task's project; other projects' agents are not offered, and the
    node refuses an agent of another project (`mesh.recipient_unknown`, as for `handoff` with a task). A machine other than the one holding
    the task's board entry is a different choice and is named in the card ("on <machine>"); it is allowed only by the person's pick.
11. **UI.** In the project page the board is a deep view (ADR-0024: opened on demand, not a third side panel). Words follow the UI vocabulary
    (`docs/ux/visual-language.md`): Backlog, Doing, Review, Done; "Give to an agent"; "Workspace" for worktree/branch; "Machine" for node;
    "Needs your OK" for approvals; never "claim", "lease" or "mesh". Design tokens only, both themes, keyboard and command palette reach every
    action (a task and an agent are searchable objects), touch targets 44 px. Strings live in `locales/`; the API carries codes and parameters.
    PR screenshots in both themes.

## Consequences
- Schema: none for steps 1 to 3 if the dispatch is built from existing `task_*` calls and a node command already in the protocol. If it needs a new
  command or event, it is a new message (append-only) and is listed as its own step. Step 4 (Review, parent link, dispatch link) adds fields or a
  message; each needs the maintainer's approval before it is built.
- The task board spec and `docs/ux/task-board.md` change with the code. Golden fixtures for each new message.
- Agents need no change: they already have `task_complete`.

## Threat-model entries to add (numbers assigned on acceptance)
- **Prompt injection through task text.** A task title or detail written by an agent, a person or a compromised hub carries instructions. Mitigation:
  decision 5 (data block, template, size and character limits, source named), approvals unaffected by any task text (invariant), dispatch is not an
  approval, tests with a task whose detail asks for approval, for a secret and for a command.
- **Dispatching to the wrong agent, project or machine.** A hub or window bug, or a lookalike name, sends a task and its text to an agent the person
  did not mean. Mitigation: the card names agent, project and machine in full before it is sent; the node checks the agent is of the task's project;
  a dispatch command is a normal routed command (grants, ADR-0041); the audit event records who, task, agent and the prompt hash.
- **Board spam.** An agent or loop creates thousands of tasks, or dispatches repeatedly. Mitigation: the board's existing limits (500 per project,
  size cuts), a per-agent creation rate limit, children per parent and depth limits, one dispatch per task at a time, at most 8 dispatches per minute
  per window, and the board counts shown to the person.
- **Residual:** a person who dispatches a hostile task to an agent that skips permissions gets what that agent does with it; the card says it skips.

## Open questions
- Review as a stored field or a derived rule? Proposed answer: a stored board field "accepted by <person>, time", append-only, in step 4.
- Automatic dispatch to an idle agent? Proposed answer: later, opt-in per project, never to a different project, never across machines unless the
  person chose that machine for the project; each automatic dispatch is shown and can be turned off. Needs its own ADR.
- Do subtasks block their parent from being claimed? Proposed answer: no; a parent is a grouping, `depends_on` stays the only blocking link.
- Should dispatch of a task to an agent that is busy queue it? Proposed answer: yes, as one queued message, shown as "next" on the card; one task per agent.

## Build order
1. The board view with three columns over existing data (read-only), UI vocabulary, both themes.
2. `dispatch` to an existing agent of the project: claim, prompt, audit event, card shows skip-permissions. Test with `agora-fake-agent`.
3. "Start a new agent for this task" through the ordinary start command; dispatch to a helper child (decision 8).
4. **Needs its own approval:** the schema step: Review (accepted-by field), parent link, links to conversation, worktree, branch.
5. Subtasks UI and the Review column.

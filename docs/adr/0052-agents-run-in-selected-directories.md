# 0052. Agents run in the selected directory; separate worktrees are optional
Status: Proposed
Date: 2026-11-11

## Context

The maintainer's intended workflow is to choose a machine, project and working directory, start an agent there, and manage
the resulting sessions from one window. A separate Git worktree for each agent is an optional tool, not the purpose of the product.

The implementation currently enforces the opposite: `Supervisor::start` always creates a worktree, the web start form offers
only registered projects and agent kinds, and the CLI has no working-directory option. Published `AgentStart` also defines an
empty `worktree_id` as a request to create a worktree. Changing a label in the UI cannot provide the requested behavior.

This proposal replaces the default-isolation choice in ADR-0003 for the new launch flow. It does not reinterpret existing messages.
Tracking issue: #235.

## Proposed decision

1. **Launch in a directory the user chooses.** The primary flow is machine → project/folder → agent → start. An existing project
   offers its current checkout as the initial directory; the user may choose another directory or a subdirectory. Launch uses
   the exact selected directory as the process cwd. No branch, clone or worktree is created in this mode. Git is not required
   to open an agent in a folder. The initial implementation concerns new sessions managed by the node, not adopting unrelated
   processes that were already running in another terminal.
2. **A separate worktree is an explicit option.** The launch form defaults to the selected existing directory. A secondary
   "Separate worktree" option creates a branch/worktree when the selected folder belongs to a Git checkout. A non-Git folder
   does not offer that option. CLI commands expose the same choice with `--cwd` and an explicit worktree option; new CLI
   behavior uses a new command path rather than changing the meaning of the published legacy request.
3. **Folders are available without restarting the node.** The node can inspect/register an existing directory selected by the
   owner and return a stable node-local workspace reference, its canonical directory, and its Git project/checkout identity
   when available. The web client can select a known project or enter a directory on the selected machine. Selection is not
   limited to the projects passed to `node run` at startup. A folder outside a Git checkout is a workspace with no Git project
   identity; it is not assigned a fabricated repository identity.
4. **Workspace ownership is explicit.** Runtime and state distinguish an existing user-owned directory from a node-created
   worktree, and keep the launch cwd separate from a Git checkout root. Stopping an agent, abandoning a failed start, removing
   a session, or uninstalling the node must never remove an existing directory, reset it, clean it or delete its branch.
   Worktree cleanup applies only to worktrees the node created. Restart/resume preserves the selected cwd and launch mode.
5. **Sharing is permitted and attribution is honest.** Several agents may use the same existing folder. The UI indicates that
   they share files and shows repository changes as changes to that workspace, without attributing the same diff independently
   to each agent. Existing edits and untracked files are preserved. File leases continue to govern writes mediated by the node;
   as today, leases cannot prevent a CLI agent's own shell from editing a shared file. A non-Git workspace has terminals and agent
   state, but no Git history or Git diff view.
6. **Authority remains on the node.** Directory inspection, registration and launch respect node-local policy and device/project
   limits. A trusted owner's authorization binds the selected node, canonical directory and launch mode; a hub cannot substitute
   a different directory under a generic start grant. Paths are resolved on the selected machine, revalidated at launch, and
   checked after symlink resolution. A team member cannot register arbitrary machine paths or use another project's directory
   through this flow. Directory browsing is limited to paths the node allows that device to inspect. An explicit owner-signed
   request may select an allowed path without preconfiguring every project through a daemon restart.
7. **Extend the protocol instead of repurposing it.** Keep published `AgentStart`, `worktree_id`, worktree events and golden
   fixtures unchanged. Add a capability and new messages for directory inspection/registration, explicit directory/worktree
   launch, and workspace ownership/state. Emit a workspace binding before the agent that uses it. Old clients retain their
   existing behavior; new clients show capability availability and never silently create a worktree when directory launch is
   unavailable. The implementation PR must enumerate the new messages and signature binding before schema generation.
8. **Keep agent integrations declarative.** Apply the selected cwd to L1 terminal agents and to the hosted session of L2/L3
   agents through their existing manifest/adapter interfaces. Mesh endpoints and generated integration files remain scoped to
   an agent. Starting a second agent in the same directory must not overwrite the first agent's integration configuration or
   overwrite existing user configuration. No agent-specific logic belongs in core crates.

### Optional portable project identity (proposal for review)

The maintainer also suggested storing a project identifier in repositories to associate the same project across machines.
Use the neutral field name `project_id`, independent of the final product name. It identifies the logical project, not a
machine, folder, branch or agent session.

- Offer a one-time, random project ID in dedicated project metadata when the owner explicitly chooses to create it. A Git
  repository can commit that metadata so ordinary clones carry the same ID. Do not create or commit it automatically on launch.
  The metadata filename is not fixed by this draft.
- Keep the current normalized remote/root-commit discovery as the zero-configuration fallback when no explicit ID exists.
  Keep detected Git identity separate from the optional logical project ID; do not rewrite published IDs in place. Existing
  bindings need explicit association/migration through the additive protocol, with the owner's confirmation.
- The hub groups an owner's confirmed checkouts under the same logical project. Each retains its node, canonical path,
  branch, commit, local edits and sessions. The same ID does not imply identical file contents; Git or another explicit
  synchronization mechanism transfers changes between machines.
- Copying a repository into an independent project or fork offers a new project ID. A changed ID or a conflict with an existing
  binding must not silently merge or split projects. A repository-supplied ID is untrusted metadata, never authorization to
  access another owner's project, join its mesh or widen a device's project limits.
- Non-Git folders may be explicitly associated with a logical project by the owner while retaining their lack of Git history
  and per-agent Git attribution. A standalone folder need not create project metadata at all.

For example, one logical project can contain an Ubuntu checkout at `/srv/shop`, a macOS checkout at
`/Users/owner/code/shop`, and a Windows checkout at `D:\\code\\shop`, all with different agent sessions and independently
selected working directories. A worktree is not required for any of them.

## Acceptance criteria

- A fake terminal agent reports the exact selected cwd, including a nested folder and a folder whose path contains spaces.
- A fake adapter session receives the selected directory; its resume preserves it.
- Directory mode leaves the Git branch and worktree list unchanged, preserves existing edits, and works outside Git.
- Two agents can run in the same directory with distinct sessions and mesh endpoints; the UI marks their shared workspace.
- Stopping either agent and a failed launch leave the existing folder, branch and user configuration intact.
- Separate-worktree mode still creates an isolated worktree only when selected and never substitutes itself for directory mode.
- A new directory is selectable while the node remains running. Invalid, inaccessible, out-of-policy and substituted paths are
  refused on the node. A member cannot escape the projects granted to that member.
- Published protocol fixtures remain unchanged. Old/new-client capability behavior is covered by integration tests.
- Portable project IDs, when chosen, group confirmed clones across nodes without merging their paths or sessions. Repositories
  without metadata retain automatic discovery; copied IDs and changed metadata cannot bypass owner/project authorization.
- The web flow is verified in both themes, with English/Turkish strings in Fluent. Platform CI checks cwd on all three systems.

## Consequences and rollout

The primary product is a session control plane over user-selected projects and folders. Git worktrees remain available for
users who want isolation. Shared-directory changes cannot have reliable per-agent attribution, and the UI must say so.

After acceptance, update `vision.md`, `architecture.md`, the glossary, launch UX and backlog in the implementing PRs. Split the
work into protocol/domain, node supervision, and CLI/web tasks with explicit crate ownership. The schema and runtime behavior
remain unchanged while this ADR is Proposed. No existing agent or working directory is migrated automatically.

## Alternatives considered

- Keep creating worktrees and only expose their parent directory: this does not launch in the user's selected working copy.
- Reinterpret an empty `AgentStart.worktree_id`: this breaks the published request's meaning.
- Automatically attach arbitrary existing terminal processes: a separate feature with process/PTY ownership requirements.

# 0064. Workspace hygiene: shared first, worktrees cleaned, the disk never fills
Status: Accepted
Date: 2026-11-14
Accepted: 2026-11-14 (maintainer's explicit answer to a question listing each Proposed ADR with its summary and "approve as proposed": they ticked 0064, 0065, 0066, 0069, 0071, 0072 and 0074). The open questions' proposed answers stand.

Origin: the maintainer: "know that worktrees take disk space: weight the *shared* folder; and do not forget to delete workspaces when they are no longer needed, so the disk does not fill."
Amends ADR-0052 (selected folders; worktrees optional) only by adding clean-up; everything else in it stands.

## Context
ADR-0052 already makes the **selected existing folder** the default and a separate worktree an explicit option, but nothing cleans up after it: a worktree the node created stays on disk
after its agent ended, with its own build output (a Rust `target/` is 4 to 34 GB), and the maintainer's machine went from 24 GB free to 374 GB free by one manual clean-up of 119 old worktrees.
`remove_worktree` exists and refuses to lose work unless forced; no policy calls it.

## Decision
1. **Shared is the default everywhere and the window says so.** The start window keeps "Separate worktree" off by default and, when it is offered, shows what it costs ("needs about N MB of disk and a branch", from the folder's size) and when it is wise
   (two agents write the same files at once). Starting several agents in one shared folder uses leases (ADR-0039) to avoid collisions, as today.
2. **The node cleans up the worktrees it created, and only those.** A worktree is removed automatically when ALL hold: the node created it; its agent has ended (or the machine restarted and the agent is not resumable);
   it has **no uncommitted changes and no commit that exists nowhere else** (its branch is merged into the main branch, or pushed, or identical to its base); and a **grace period** passed (default 24 hours after the agent ended, so a person can still look). Otherwise
   it stays and is **listed as "workspace left behind"** with the reason ("has uncommitted changes", "branch not merged") and a button. The node never touches a folder it did not create (selected folders are never deleted), never forces, and logs every removal (`workspaces.log`).
3. **A person can always clean up by hand:** window: "Remove workspace" on a closed agent and "Clean up workspaces on this machine" (shows what would go and how much space, then asks); CLI `agora workspaces` (list with size, state, age) and `agora workspaces clean [--older-than 7d] [--dry-run]`. Removing something with unmerged work needs an explicit confirmation naming the loss.
4. **Build output is separate from the workspace.** A worktree's `target/`, `node_modules/` and similar can be removed alone while the workspace stays ("Clean build output": offered when a closed or idle workspace holds more than 1 GB of them, and automatically after the grace period for worktrees
   the node created); known patterns come from the manifest/project (`[workspace] clean = ["target", "node_modules"]`, defaults for Rust and Node projects), never guessed outside them.
5. **The disk is watched.** The node reports free space and the size of its workspaces; the window shows it on the machine ("12 GB free, workspaces 40 GB") and raises "Needs you" when free space is under a threshold (default 10% or 20 GB, whichever is larger) with the clean-up button in the card. Below a critical threshold (default 3 GB) the node refuses to create new worktrees and says why (starting in a shared folder still works).
6. **Limits for the node's own worktree folder:** a configurable cap (default 50 GB); creating a worktree over the cap triggers the clean-up first and refuses with a clear message if it is not enough.

## Open questions for the maintainer
- Grace period 24 hours (proposed) or shorter/longer?
- Disk thresholds: warn at 10% or 20 GB, refuse new worktrees under 3 GB (proposed).
- Should the node also offer to remove build output of SELECTED folders (not created by it)? (Proposed: only offer, never automatic.)

## Consequences
A workspace registry in the node (what it created, when, size, state), a clean-up pass (at start and periodically), `agora workspaces`, free-space reporting (a field in the node report: additive), window and TUI cards, `[workspace] clean` in the manifest format (additive), threat model note (the clean-up deletes files: it removes only what the node created, never forces, and keeps the evidence of every removal).

## Build status

Built without a schema change (roadmap M3.7, `crates/agora-node/src/workspaces.rs`):
- **Decision 2.** The node records every worktree it makes for an agent the moment git made it (its own worktree, a separate one for
  a selected folder, an adapter agent's), one owner-only file per worktree in `<data>/workspaces/`, with the Git folders and the
  `.git`, `commondir` and `gitdir` files as git wrote them. Only those are ever removed: a selected folder, a worktree a person made,
  one outside the node's worktree folder, or one made before this was built is never touched. A pass a minute after the node starts
  and then every hour removes a recorded worktree when no agent runs or waits to be resumed in it (or below it), it is still a real
  folder at its recorded place, its links still hold what git wrote, git reports no uncommitted change (untracked files included),
  no checked-out submodule and no commit that no other branch or remote has, and the grace period passed (24 hours from when the
  node first saw it free or its last agent ended, whichever is later). Removal is `git worktree remove` through the node's existing
  checks, never forced; the branch is kept. Anything else stays and is listed as left behind with its reason (`changes`,
  `submodules`, `unmerged`, `links-moved`, `moved`, `git-failed`). Every removal is a line in `<data>/workspaces/workspaces.log`, and
  the hub hears `WorktreeRemoved` (an existing message). An agent interrupted by a restart counts as in use while the ledger offers it
  (14 days), whether or not it can be resumed: stricter than the decision's "not resumable".
- **Decision 3, CLI.** `agora workspaces` lists each workspace (path, state, reason, size, how long free, branch; fixed tokens for
  scripts) and `agora workspaces clean [--older-than 7d] [--dry-run]` removes the ones nothing would be lost with, at once (a person
  does not wait for the grace period). It never removes work: removing a workspace that holds work stays a manual `git worktree
  remove`. While the node runs, the CLI asks it which agents run (`agent.list`) and refuses to clean when it cannot ask.
- **Two removers at once.** A clean-up claims a workspace by renaming its record (`rename_no_replace`) before it checks it, asks again
  who is in it after the claim, and the supervisor refuses a start or resume into a claimed workspace under its lock.
- **Decision 4, in part.** After the grace period, a left-behind worktree the node made has its `target/` (beside a `Cargo.toml`) and
  `node_modules/` (beside a `package.json`) removed once, only when it is a real folder at the top of the worktree that git ignores
  and holds nothing tracked.
- **Decisions 5 and 6, in part.** The node refuses to make a worktree when the worktrees it recorded hold the cap (as last measured)
  or the disk has less than the minimum free, with the existing `node.resource_limit` (`resource` = `workspaces_gb` or
  `free_disk_gb`, `limit` in GB), and wakes its clean-up pass. Starting in a shared folder still works.
- **Settings** (node policy file): `workspaces.clean = auto | off` (default `auto`; `off` only lists), `workspaces.grace_hours`
  (24), `workspaces.max_gb` (50; 0: no cap), `workspaces.min_free_gb` (3; 0: never refuses). A value not understood keeps the default
  (and switches automatic clean-up off) and is a line the node did not understand.

Not built:
- **Needs schema:** free space and workspace sizes in the node report, the machine's "12 GB free, workspaces 40 GB" and the "Needs
  you" alert under the warning threshold (decision 5: a field in the node report); "Remove workspace", "Clean up workspaces on this
  machine" and "Clean build output" in the window and the terminal client (decisions 3, 4: commands with a listing and a confirmation
  that names the loss); the left-behind list in the window (decision 2).
- **Needs a manifest-format change:** `[workspace] clean = [...]` (decision 4); only the two defaults are removed.
- Decision 1's cost hint in the start window ("needs about N MB of disk"); decision 6's clean-up runs after a refusal rather than
  before it (the start is refused at once, the pass is woken, and a retry may pass); build output of selected folders (open question:
  only offered, never automatic) is not offered.

# 0064. Workspace hygiene: shared first, worktrees cleaned, the disk never fills
Status: Proposed
Date: 2026-11-14

Not accepted. Nothing here may be built until the maintainer accepts it in a message of their own.
Origin: the maintainer: "know that worktrees take disk space: weight the *shared* folder; and do not forget to delete workspaces when they are no longer needed, so the disk does not fill."
Amends ADR-0052 (selected folders; worktrees optional) only by adding clean-up; everything else in it stands.

## Context
ADR-0052 already makes the **selected existing folder** the default and a separate worktree an explicit option, but nothing cleans up after it: a worktree the node created stays on disk
after its agent ended, with its own build output (a Rust `target/` is 4 to 34 GB), and the maintainer's machine went from 24 GB free to 374 GB free by one manual clean-up of 119 old worktrees.
`remove_worktree` exists and refuses to lose work unless forced; no policy calls it.

## Decision (proposed)
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

# 0050. Windows agent confinement: a low-integrity token with a labelled worktree
Status: Proposed
Date: 2026-11-11

## Context
ADR-0047 gave agents OS-level confinement and named, for Windows, "a restricted token with the job object that already exists".
Linux (Landlock) and macOS (Seatbelt) can say "this process may write only below these folders". Windows has no equivalent in a
restricted token: a restricted token limits privileges and group membership, not paths. The mechanisms that do limit writes are
(a) a **low integrity level**, under which a process cannot write to objects labelled medium or higher (nearly everything in the
user's profile) but can write where the label is low, and (b) an **AppContainer**, which denies everything the container's SID has
no ACL entry for. Both need the folders the agent may use to be prepared: a mandatory label for (a), an ACL entry for (b). Until
then Windows machines answer `agent.sandbox_unavailable` and start nothing under a sandbox policy, which is safe but means a Windows
owner cannot use the feature.

## Decision
1. **Use a low-integrity token (not an AppContainer).** The node starts the agent's process (ConPTY or adapter) with a token whose
   integrity level is Low, inside the job object ADR-0045 already creates. Writes outside Low-labelled folders fail; so do writes
   to the registry's user hive and to other users' processes. Programs that expect a medium token (many CLIs read `%USERPROFILE%`
   freely) keep working because **reads are not restricted**: the Windows profile of `worktree` therefore confines writes only.
2. **Label what the agent may write.** Before the first start under a policy, the node adds a Low mandatory label to the worktree,
   its git folders, the agent's state folder and the temp folder it hands out, and removes the label when the worktree is removed.
   Only these folders are touched, never a parent, and only with the owner's `sandbox` policy line switched on.
3. **Say what it does not stop.** `agora doctor` and the threat model state, for Windows: writes confined, reads open, network open
   (`worktree-offline` is still refused, as on every OS). A machine whose Windows version or file system cannot label (FAT32, some
   network drives) answers `agent.sandbox_unavailable`.
4. **No AppContainer.** Agents are ordinary developer CLIs that read `~/.config`, spawn compilers and open sockets; an AppContainer
   breaks them in ways that are hard to diagnose and would need per-tool capability lists.

## Consequences
- Windows reaches "writes only below the worktree", which is the part of T8 that matters most (an agent overwriting the user's files),
  with a weaker read story than Linux and macOS.
- The node modifies labels (SACL) on its own worktree folders; the change is reversible and limited to folders Agora created.
- Needs a real Windows machine (or CI) to prove: a test that a confined process writes inside the worktree and is refused outside.
- Code lives in `agora-platform/src/windows.rs` behind `confine_command` and `PtyConfig.sandbox`, as for the other systems.

## Alternatives considered
- **Leave Windows unsupported (today):** safe and honest, but the feature is missing on a platform the project supports.
- **AppContainer:** stronger on paper, fragile for the tools agents run (see decision 4).
- **Run agents in WSL or a container:** a different product (a second environment to set up and keep in step) and not the user's own Windows paths.

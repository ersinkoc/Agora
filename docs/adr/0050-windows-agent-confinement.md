# 0050. Windows agent confinement: a low-integrity token with a labelled worktree
Status: Proposed
Date: 2026-11-11

Maintainer review (2026-11-11): changes requested. The decision supplied in the working session was
"0050 değişiklik, 0051 kabul". This ADR remains Proposed; do not implement it until the revised design is accepted.

## Required revision

The proposed Low token protects higher-integrity objects from writes. It does **not** select a set of writable paths:
any other Low-labelled object allowed by the user's DACL can still be writable, including another agent's Low-labelled
workspace. Reads, including node credentials, remain open. The claims below that writes are confined to the worktree are
therefore not an enforceable replacement for ADR-0047's `worktree` policy.

Before acceptance, revise the design to either enforce the existing path and read restrictions, or propose a separately
named, explicitly weaker policy with its own consent and threat-model treatment. The existing `worktree` and
`worktree-offline` settings must keep refusing startup on Windows until their stated restrictions can be enforced.
Specify how per-agent Git metadata is separated from the checkout's common Git directory: lowering the common directory's
integrity would expose other worktrees. Include the ownership, inheritance and restoration of labels, failure cleanup,
and inherited writable handles in that design.

Acceptance evidence must include real Windows tests for writes inside and outside the allowed paths, a second agent's
Low-labelled workspace, protected node files, child processes and reparse points. Merely refusing a Medium-labelled file
does not prove path confinement. This follows the distinction between integrity checks and DACL checks in
[Microsoft's MIC documentation](https://learn.microsoft.com/en-us/windows/win32/secauthz/mandatory-integrity-control).

The original proposal is retained below for review; it is not an accepted decision.

## Context
ADR-0047 gave agents OS-level confinement and named, for Windows, "a restricted token with the job object that already exists".
Linux (Landlock) and macOS (Seatbelt) can say "this process may write only below these folders". Windows has no equivalent in a
restricted token: a restricted token limits privileges and group membership, not paths. The mechanisms that do limit writes are
(a) a **low integrity level**, under which a process cannot write to objects labelled medium or higher (nearly everything in the
user's profile) but can write where the label is low, and (b) an **AppContainer**, which denies everything the container's SID has
no ACL entry for. Both need the folders the agent may use to be prepared: a mandatory label for (a), an ACL entry for (b). Until
then Windows machines answer `agent.sandbox_unavailable` and start nothing under a sandbox policy, which is safe but means a Windows
owner cannot use the feature.

## Original proposed decision (changes requested)
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

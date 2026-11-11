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

## Revised proposal (awaiting maintainer decision)

The review is right: a Low integrity level only protects objects labelled higher, so it is not a path allowlist. This revision
drops it as the mechanism and enforces the **path rules of ADR-0047** with the DACL check instead.

1. **Mechanism: a restricted token with one restricting SID per agent.** The node creates a fresh SID for each agent start and
   builds the agent's token with `CreateRestrictedToken`, restricting SIDs = that SID plus `Everyone` and the `Users` group. Windows
   then grants an access only if *both* the normal check and the restricting check pass. The node adds an ACL entry for the agent's
   SID on exactly the allowed folders. Everything else is writable only if its ACL names `Everyone` or `Users`, which a normal profile
   does not. The token keeps the job object of ADR-0045 and drops all privileges.
2. **Per agent, so a second agent cannot be reached.** Agent B's SID is on B's folders only. A's token has no entry there and the
   restricting check refuses it. This replaces the shared Low label that made every labelled workspace writable by every confined agent.
3. **Reads are narrowed, not closed.** Files readable by `Everyone` or `Users` (the system folders, toolchains in `Program Files`)
   stay readable; files only the user owns (`%USERPROFILE%`, the node's keys, `.ssh`) are refused. This is stronger than "reads open" but is
   not Landlock's explicit read list. The `worktree` policy therefore means, on Windows, *writes only below the allowed folders; reads of
   user-private files refused*. Anything an agent legitimately needs outside (a package cache, `%APPDATA%`) must be listed by the owner
   under the existing `sandbox` allowed-folders line, which receives the same ACL entry. `agora doctor` prints this difference.
4. **Git metadata.** The agent gets write access to its own worktree folder and to that worktree's admin directory
   (`.git/worktrees/<name>`), and **read-only** access to the common Git directory. Writes to the common `config`, `hooks/` and other
   worktrees' admin directories are refused, which closes the escape that matters (a planted hook or config run later by the user).
   Writing objects and refs needs the common `objects/` and `refs/` folders: those two subfolders get write access and nothing above
   them. **Residual risk, stated in the threat model:** a confined agent can add objects and move branch refs in the shared repository.
   It cannot rewrite config or hooks, and refs are recoverable from the reflog.
5. **Lifecycle of the ACL entries.** The node records every entry it adds (folder, SID) in its state folder *before* adding it. It
   removes them when the agent stops, when the worktree is removed, and on node start for any record left behind by a crash. A
   removal that fails is retried and reported by `agora doctor`; it never leaves a folder writable to a SID no running token holds,
   because the SID is random and dies with the agent. Inheritance is switched off on the added entries' scope (they are set on the
   allowed folders with inheritance to children, and nowhere above). Handles the node holds are opened without inheritance
   (`bInheritHandles = FALSE` with an explicit handle list), so an agent cannot receive a writable handle it should not have.
6. **Refusal stays the default.** `worktree` and `worktree-offline` keep answering `agent.sandbox_unavailable` on Windows until the
   tests below pass in CI and a revision of this ADR is accepted. `worktree-offline` stays refused on every OS (no network control).
   A volume without ACLs (FAT32, some network shares) also answers `agent.sandbox_unavailable`.
7. **Still not AppContainer**, for the reason in the original decision 4; a restricted token breaks fewer developer tools.

### Acceptance evidence (real Windows CI, not Linux-only logic)
- a confined process writes inside the worktree and is refused outside it (profile, repository root, another folder);
- a second agent's allowed folder is refused to the first, and the reverse;
- the node's key and state files are unreadable and unwritable;
- a child process and a grandchild inherit the same confinement; a process started through a reparse point or junction pointing outside
  is refused;
- the common Git `config` and `hooks/` are not writable, `objects/` and `refs/` are, per decision 4;
- ACL entries are gone after a normal stop, after a killed agent, and after a simulated node crash and restart;
- Microsoft's distinction between integrity and DACL checks (MIC documentation) is why the checks above are DACL-based; a test that only
  shows a Medium-labelled file refusing a write does not count.

### Consequences of the revision
- Windows reaches the same *write* rule as Linux and macOS, with a narrower but not explicit read rule, and says so.
- More node code than the Low-label idea (SID and ACL bookkeeping with crash recovery), all in `agora-platform/src/windows.rs`.
- The shared repository's `objects/` and `refs/` stay reachable to a confined agent (decision 4); this is the one disclosed weakness.

The original proposal (Low integrity level) is retained below for review; it is superseded by the revised proposal above and is not an accepted decision.

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

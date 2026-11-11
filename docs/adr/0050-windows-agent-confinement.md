# 0050. Windows agent confinement: a restricted token with a per-agent SID
Status: Accepted
Date: 2026-11-11

Maintainer review (2026-11-11): changes requested. The decision supplied in the working session was
"0050 değişiklik, 0051 kabul". The revised design below was then accepted ("0050 kabul").

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

Maintainer decision (2026-11-11): the revised proposal below is accepted ("0050 kabul"). The original proposal further down is superseded.

## Amendment B (accepted by the maintainer 2026-11-11: "0050 Ek B kabul"): Low integrity, the ancestor entries, and a refusal before start

**Why an amendment.** The revised decision below lists `Everyone` and `Users` among the restricting SIDs. A Windows machine showed (issue #230, messages in `comms`, 2026-11-11) that a folder whose ACL grants those
groups then satisfies both access checks with no entry for the agent, so writes and reads outside the allowed folders succeed there (`C:\ProgramData`, a data volume). The maintainer accepted "Amendment A" ("0050 Ek A kabul"): leave
`Everyone` and `Users` out and use the `ALL (RESTRICTED) APPLICATION PACKAGES` groups for the system folders. **Amendment A cannot be built**: `CreateRestrictedToken` rejects those SIDs (error 87), and with only the agent's SID
and the logon SID no system image loads. It was never merged (draft PR #237). Two designs were then measured on Windows 11 (draft PRs #252, #253).

**AppContainer** gives the strongest file and network rule and closes #230 for reads and writes, but in a real run it made `git` exit `0xC0000008` whenever it writes to the pseudo-console (the strict handle check policy
Windows turns on for AppContainer; it cannot be passed on to descendants); made msys programs (`sh`, `bash`, which Claude Code uses for its shell tool on Windows) fail at start (`0xC0000142`); hid `node` and `python` installed with
protected ACLs; and needs an administrator to grant `ALL APPLICATION PACKAGES` read on `C:\Users` and the drive roots before `git commit` can `stat` its way to the root. **Not adopted** for now; it stays the candidate for a later network rule.

**Low integrity alone** does not separate agents: every folder an agent may write is labelled Low, and where it also grants `Users` write (ProgramData, a parent with a broad grant) every confined agent can write there
(measured: a second agent's folder was written).

**Decision (replaces decision 1's SID list and Amendment A):**
1. Keep the restricted token of decision 1: restricting SIDs are the agent's SID, `Everyone`, `Users` and the logon SID, all privileges dropped, inside the job object.
2. **Run it at Low integrity** and put a Low mandatory label on exactly the folders the agent may write. Writes to anything at Medium integrity (the whole user profile, the data volumes, every `Users`-writable folder such as
   ProgramData) are refused whatever its ACL grants. This closes the write half of #230. Where the user cannot set the label (a folder that grants Modify only through `Authenticated Users`), the folder is refused as in 4.
3. **Ancestor entries** (the prototype in #253): for each allowed folder, a non-inheritable `FILE_READ_ATTRIBUTES | FILE_TRAVERSE | SYNCHRONIZE` entry for the agent's SID on each folder above it that the user may change,
   written without walking the folder's children, recorded in the ledger first and undone with the rest. Folders the user may not change are skipped (the `Users` entry of the restricting list covers `C:\Users` and the drive roots).
4. **Refuse before start.** The node answers `agent.sandbox_unavailable` (the reason in a field) when an allowed folder, or a folder the agent would be given, grants `Users` or `Everyone` write access or cannot be labelled. A
   Low-labelled folder that also grants `Users` write is what lets a second agent in; it is detectable, so it is refused instead of promised away. `agora doctor` reports the same.
5. **Reads are weaker than on Linux and macOS, and the documentation says so**: files that grant `Users` or `Everyone` read (many data folders, `Program Files`) stay readable to the agent; files only the user may read (`%USERPROFILE%`,
   the node's keys, other agents' folders unless they inherit a broad read grant) are refused. The `worktree` policy on Windows therefore means **writes only below the allowed folders; user-private reads refused**. A read allow-list is not claimed.
6. Still no network rule (`worktree-offline` stays refused on every OS); AppContainer is the path to one.

**Acceptance evidence** (real Windows, in CI and on a real machine; `ENABLED` stays false until both pass): writes outside refused in three parents (`%TEMP%`, a data volume, `C:\ProgramData`), including by a child process; a second
agent's folder refused for writes, and an allowed folder that grants `Users` write refused before start; user-private files unreadable; `cmd`, `git` (with commit, under the profile and on a data volume), `node`, `python`, `sh` start; ConPTY
output, input and resize work at Low integrity; the Low label and every ledger entry are gone after a normal stop, a killed agent and a node crash; the grant time of a large `sandbox.read` folder is reported.

## Revised decision

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
   them. A commit also appends to the branch's reflog, which `git worktree add` creates under the common `logs/refs/`, so that folder
   is writable as well (clarification found on a real Windows machine; the rest of `logs/` is not). **Residual risk, stated in the threat model:** a confined agent can add objects and move branch refs in the shared repository.
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

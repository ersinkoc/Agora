# 0053. Browsing a machine's folders, and forgetting closed agents
Status: Proposed
Date: 2026-11-13

Not accepted. It becomes Accepted only on the maintainer's explicit message ("0053 kabul"). Nothing here is built on until then.

## Context

Two things the window cannot do today, both found while using it:

1. **Choosing a folder means typing its path.** ADR-0052 made the working folder the unit of work, and its item 6 allows directory
   browsing "limited to paths the node allows that device to inspect". The start window lists the folders the machine already knows
   (projects) and otherwise asks for a path typed from memory. There is no message that lists a machine's folders, so the window
   cannot offer a folder picker.
2. **A closed agent stays in the hub's record for good.** The window now keeps closed agents in their own section with a summary and
   lets the user clear them, but clearing is only the browser's (`localStorage`). Another browser, the TUI and the desktop app show them
   again. Removing them for everyone needs the hub to forget them, and the event stream is append-only.

Both need new protocol messages, so both need an ADR (AGENTS.md: schema changes).

## Decision

### 1. `directory.list`: a folder picker's one question

- **Command `DirectoryList { path, authorization }`**, answered with `DirectoryEntries { path, parent, entries[], truncated }`.
  `path` empty means the user's home folder on that machine. `path` in the answer is the canonical folder the node resolved, `parent` is
  its parent (empty at a root), each entry is `{ name, is_git }` and **only folders are listed**, sorted by name, at most 500 per answer
  (`truncated` says more exist). No file names, sizes, contents or timestamps.
- **Who may ask:** the same owner-signed authorization as `directory.inspect` (ADR-0052 item 6), bound to the node and the canonical path;
  a hub cannot list a folder the owner did not ask for. Team members only inside the projects they were granted (ADR-0049), never a
  machine-wide listing.
- **What the node hides:** the folders ADR-0052 refuses to launch in (`.ssh`, `.gnupg`, `.aws`, `.kube`, `.docker`, `.azure`,
  `.config/gcloud`, `.config/gh`, and the node's own folder) are not listed. A link is not followed out of the folder being listed; the
  listing is made through a held directory handle, as `files.list` is (#326, #333), so a swapped folder cannot redirect it.
- **Capability `directory.list`;** a node without it is shown the typed-path form as today. Folders listed are names only: choosing one
  still goes through `directory.inspect` and the signed launch.
- **Budget:** one folder per call, no recursion; the node answers within the control channel's usual timeout and a slow volume returns
  `truncated`.

### 2. `agent.forget`: remove a closed agent from the record

- **Command `AgentForget { agent_id }`**, refused (`agent.running`) while the agent runs. The hub appends an **`AgentForgotten`** event
  and its snapshot and projections stop listing that agent. **History is not rewritten:** the earlier events stay in the store (the audit
  trail and usage totals still account for it); only the live view forgets.
- **Who:** the owner, and a member for agents in their granted projects (ADR-0049). It is recorded in the audit trail.
- **Node:** no node involvement; a node's own data (a terminal's scrollback, an adapter session file) is not deleted by this. Removing a
  worktree the node made stays a separate, explicit action.
- **Clients:** the window's "Remove from the list" offers "Remove for everyone" when the capability is present; the TUI gets the same key.

## Consequences

- Two new messages, one event, two capabilities and error code `agent.running`; golden fixtures for each; `protocol.md` and the
  threat model (a listing is a disclosure: names of folders to a signed owner only) are updated with the implementation.
- Old nodes and hubs keep working: without the capabilities, the typed path and the browser-only clear remain.
- Not decided here: listing *files*, searching folders by name, and remembering recent folders on the hub (the window can keep recents
  per browser without any protocol change).

## Alternatives considered

- **Typed path only (today):** simple, but the user must know the path; the maintainer calls it unusable.
- **The window lists folders through the existing `files.list`:** that command is scoped to a workspace the node already registered;
  using it for arbitrary folders would widen it beyond what ADR-0052 allows without a signature.
- **Delete closed agents' events:** breaks the append-only rule and the audit trail; rejected.

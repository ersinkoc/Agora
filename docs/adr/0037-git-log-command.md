# 0037. `git.log` command: recent commits of an agent's workspace
Status: Proposed
Date: 2026-11-08

## Context
The console shows what an agent has changed but not yet committed (`git.status`, `git.diff`) and can browse its files. The next
question a user asks is "what did it already commit?". The protocol has no message for a commit list, and `GitStatusResult` carries
only the head commit id.

## Decision
Add one command and one result, with new message names and numbers (ADR-0006); nothing existing changes.

```
message GitLog { string worktree_id = 1; uint32 limit = 2; }          // limit 0 means the default (30); the node caps it at 200
message Commit { string id = 1; string subject = 2; string author = 3; uint64 at_ms = 4; }
message GitLogResult { repeated Commit commits = 1; }
```
`CommandRequest.command` gets `GitLog git_log = 42;` and `CommandOk.result` gets `GitLogResult git_log = 15;` (numbers checked
against `commands.proto` when the schema is written).

The node answers from `git log` of the agent's own workspace, newest first, for the same workspaces and with the same path
rules as `git.status` and `git.diff` (only the node's own agents' workspaces; nothing else). Commit subjects and author names are
untrusted text: clients show them as plain text. The hub only routes (ADR-0029); it does not store commits.

## Consequences
- One command, three messages, golden fixtures, `docs/specs/protocol.md` and the node's capability list updated.
- The web console gets a "History" list beside "Changes" for each agent.
- Old nodes answer `protocol.capability_missing`; the client shows "not available on this machine".

## Alternatives considered
- Put the log inside `GitStatusResult`: changes a published message's size on every status poll for a rarely used list.
- Let the client read `.git` through `files.read`: leaks git internals through the files path and cannot decode packed objects.

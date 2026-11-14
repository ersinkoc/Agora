# 0071. A diff names every changed file, even when the text is cut
Status: Proposed
Date: 2026-11-16
Origin: the independent review of the web review screen (#483, #487, #488, #500, 2026-11-15) found that a diff cut at 512 KiB hides every file after the cut with no warning: the chip says "8 files changed", the review screen lists 2, and the Files tree is missing the change marks of the others. The web alone cannot learn the missing names: `git.diff` returns one string, `FilesChanged` is a stream of watcher batches rather than the list, and walking folders with `files.list` plus a `git.diff` per path spawns many git processes and still misses files under deleted folders.

## Context
`GitDiffResult { diff }` carries the unified diff as one string and the node cuts it at a size cap (ADR-0022 keeps big payloads bounded). Clients parse the string to list files, show counts and mark the tree. When the cut falls in the middle of the output, everything after it is invisible, and nothing in the message says that anything is missing. A reviewer can believe they saw every change.

## Decision
1. **Add an optional field to `GitDiffResult`** (append-only, new field number, older clients ignore it): `repeated ChangedPath paths`, one entry per changed file in the diff's scope, produced by git's name-status output (`git diff --name-status -z`, plus untracked files the way the existing diff already includes them), **capped at 5,000 entries** with a `paths_truncated` bool when more exist. `ChangedPath` carries the path, the status (added, modified, deleted, renamed, copied, type-changed), the old path for a rename or copy, and whether the file is binary. The node computes it in the same git call family as the diff; it costs one more process, not one per file.
2. **The `diff` string keeps its meaning.** It is still cut at the cap, unchanged. A client that wants the names reads `paths`; a client that does not know the field behaves as today.
3. **Clients say what they do not show.** When the diff text is cut, the review screen and the file browser say how many changed files have no diff text on screen (`paths` minus the files whose headers are in the text), list the rest by name and status, and offer to open one file's diff on its own (`git.diff` for that path, which fits the cap). The count never disagrees silently with the tile.
4. **Paths are data, not text.** Names are shown without control, escape, bidi or hidden characters (the existing `shown()` rules), never interpreted as HTML or markup; the path can be arbitrary bytes on Linux, so it is carried as bytes with a lossy display form.
5. **No new permission.** `paths` is derived from what `git.diff` already authorizes for the same scope; a device limited to some projects sees paths only inside them (ADR-0061 limits apply as for the diff).

## Consequences
- One optional field and one message in `schema/`, golden fixtures added (never edited), `docs/specs/protocol.md` and the capability list updated.
- The web, the TUI and the phone window can stop guessing from the text; the Files tree change marks come from `paths`.
- Cost: one more git process per diff request, bounded by the 5,000-entry cap.
## Alternatives considered
- **A file-aware cut in the node** (whole files up to the cap, then headers only): it changes what the existing `diff` string means (a header without hunks reads as a mode-only change) and stops the clients' "at least 512 KiB" check from firing. Rejected.
- **Clients walk folders and ask per path:** many git processes, still misses deleted folders. Rejected.
- **Raise the cap:** moves the cliff, does not remove it, and costs memory and render time (ADR-0022).
## Open questions
- Is 5,000 entries the right cap? (A monorepo-wide rename can exceed it; the flag says so.)
- Should the diff cut size be reported too (`diff_truncated`), so clients stop inferring it from the length? Proposed: yes, in the same change.

# 0115. A repository on a case-insensitive host is one project, whatever the case of its URL
Status: Proposed
Date: 2026-11-25

## Context

Without `.agent-project.json`, a Git checkout's project id is its normalized `origin` plus its root commit (ADR-0003, ADR-0052,
`crates/agora-core/src/project.rs`). `normalize_remote` lowercases the host and keeps the case of the path on purpose, because a path
on a self-hosted Git server can be case-sensitive.

GitHub, GitLab, Bitbucket and Codeberg treat owner and repository names without regard to case. A repository renamed only in case
(`owner/agora` → `owner/Agora`) still answers at the old URL, so clones made before and after the rename work side by side. Field
report, 2026-11-25, 0.3.38: this repository's clone on `mac` had `https://github.com/ersinkoc/agora`, the other machines'
clones `…/ersinkoc/Agora`. The same root commit gave two ids, `github.com/ersinkoc/agora#db6f7d…` and
`github.com/ersinkoc/Agora#db6f7d…`, so:

- the window's project filter showed one repository as two projects (it groups by `projectId`);
- the hub never saw the project as shared, so the board, `workspace_status` and `agents_list` treated `mac` as alone in it.

Changing the clone's URL fixed it, but nothing told the owner why, and every clone of every renamed repository can hit it.

## Decision

1. **Fold the path for known case-insensitive hosts.** `normalize_remote` lowercases the path as well as the host when the host is
   exactly one of `github.com`, `gitlab.com`, `bitbucket.org`, `codeberg.org`. Other hosts, subdomains of these hosts included,
   keep the path's case, as today. The list lives in `agora-core` as data, and adding a host later is a one-line change with a
   test. It is not configurable per node, because two nodes with different lists would split a project again.
2. **The hub folds what older nodes send.** For the same four hosts, the hub folds `project_id` when it builds its view from events
   (projection, board membership, `nodes_with`) and when it reads a `BoardCall`. Stored events are not rewritten; the fold happens
   when they are read, so a restarted hub gets the same view, and a downgrade still reads its old store. Nodes of either version
   then meet in one project.
3. **No new confirmation for this change.** A node compares a folder's confirmed binding (`directory.rs` `Bindings`) with the new
   id after folding both. A binding that differs only by this fold is the same project: it is not refused with "identity
   changed", and the binding file is rewritten with the folded id on the next confirmed start. Any other change still needs
   `accept_identity_change`.
4. **Running agents.** An agent keeps the id it reported until it exits (the architecture's existing rule). Because the hub folds,
   it appears in the merged project at once.
5. **Docs.** `architecture.md` (project identity), `protocol.md` (the id text), the comment on `normalize_remote`, and `CHANGELOG`.

## Consequences

- Clones of one GitHub/GitLab/Bitbucket/Codeberg repository are one project, whatever the case of their URLs.
- Ids of projects on those hosts now print in lowercase (`github.com/ersinkoc/agora#…`). Where a client takes the display name from the id,  so a name shows in lowercase. A later change could carry the remote's own spelling for display, if wanted.
- The fold is a change to a published value's text. Old and new nodes still agree because the hub folds (decision 2); a client
  that keys local state (layout, filters) by project id may lose that state once for those projects.
- A repository renamed to a different name (not only case) is still two projects; `agora project init` (an explicit id) remains
  the way to join them.

## Alternatives considered

- **Lowercase every path:** joins two different repositories on a case-sensitive self-hosted server.
- **Create `.agent-project.json` automatically:** writes into the owner's repository without asking, against ADR-0052 (only an
  explicit owner action creates it).
- **Leave it and document it:** the failure is silent; the window shows two projects and nobody is told why.
- **Fold only on new nodes, without the hub:** mixed versions would stay split until every node updates.

## Open questions for the maintainer

- The host list (proposed: the four above). Should self-hosted GitLab/Gitea instances be addable by the hub owner?

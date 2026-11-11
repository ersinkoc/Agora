# Independent helper controls for #300 (#302)

Concurrent implementations in #300 and #302 were consolidated on #300's production
command builder. This follow-up contains only public-API regressions and this note;
it is based on `claude/node-git-hardening` until #300 merges.

`crates/agora-node/tests/git_helpers.rs` uses separate disposable repositories for
fsmonitor, textconv, external diff and a post-checkout hook. Every ordinary-Git
control must write its marker before the node API is tested. Separating textconv
from external diff prevents one helper from masking the other's control. Hook
permissions use the existing platform API. No real agent, account or service runs.

Three tests fail on the original main and pass on #300. They assert raw status
counts, tracked/untracked preview content, suppression of all four helpers and
successful worktree creation/removal. The production change intentionally does not
suppress clean/smudge/process filters; #300 documents that remaining limitation.

Full workspace tests, formatting, Clippy with warnings denied, locale lint and docs
checks are run before publishing; CI and cross-review status are in the PR.
Installed services and the active local agent are unchanged.

# Independent helper controls for #300 (#302)

Concurrent implementations in #300 and #302 were consolidated on #300's production
command builder. After #300 merged, this follow-up was retargeted to main. It contains
only public-API regressions and this note.

`crates/agora-node/tests/git_helpers.rs` uses separate disposable repositories for
fsmonitor, textconv, external diff and a post-checkout hook. Every ordinary-Git
control must write its marker before the node API is tested. Separating textconv
from external diff prevents one helper from masking the other's control. Hook
permissions use the existing platform API. No real agent, account or service runs.

Three tests fail on the original main and pass on #300. They assert raw status
counts, tracked/untracked preview content, suppression of all four helpers and
successful worktree creation/removal. The production change intentionally does not
suppress clean/smudge/process filters; #300 documents that remaining limitation.

The initial Windows CI run passed all helper-suppression assertions but rejected the
worktree content assertion: the runner's `core.autocrlf=true` converted LF to CRLF.
The same failure was reproduced on Ubuntu with a temporary global Git configuration.
Fixture repositories now set local `core.autocrlf=false`; all three tests also pass
with that temporary global setting enabled. No user or machine Git configuration is
modified, and the helper controls are unchanged.
All CI checks then passed on `bd9566a`, including Windows, macOS and Ubuntu.

Full workspace tests, formatting, Clippy with warnings denied, locale lint and docs
checks are run before publishing; CI and cross-review status are in the PR.
Installed services and the active local agent are unchanged.

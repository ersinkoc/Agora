# PR #296: posting-only scope recheck

Reviewed head: `d1be1b0079d2d11933c2604f40b0607a34d2f2ea`.
Verdict: **APPROVE this posting-only increment**. This does not claim notification activation has been implemented.

The previous [review and runnable activation proof](https://github.com/ersinkoc/Agora/blob/a7736f596c80b6d320774548eea4be94b1603d01/docs/reviews/0296-notification-review.md) allowed explicit partial scope: keep ADR-0048 decision 5 incomplete and remove the incorrect default-click claim if activation is deferred. The author took that route. The code comment, README, backlog and rewritten PR description now say clicking does not restore/focus the window. macOS remains Unsupported. Windows/Linux notification posting is this increment; activation is later work.

The code change is Linux's application-name argument, now the caller's localized title, still passed as one argument. `0296-scoped-argv-control.py` is the previous control with only its expected app-name argument updated. Five calls through the real compiled platform function pass: literal title/body with an option-like title, shell metacharacters, Unicode and newlines; no marker execution; nonzero status; nonexecutable notifier; relative and missing PATH. A private notifier shim is used, so no real desktop notice is sent. The executable title-callback logic, Windows code and remote capability rules are unchanged by this fix.

Platform/i18n tests pass (counts in `0296-scoped-validation.json`); root and desktop formatting, docs checker and all-target platform/i18n Clippy with warnings denied pass. All reported required CI checks pass at this head, including native shell builds/Clippy. Prior local full desktop build limitation remains (missing system pkg-config/D-Bus development libraries); no native GUI click behavior is claimed. Source and existing tests are untouched on this evidence branch.

Reproduce after `cargo test -p agora-platform -p agora-i18n`: compile `0296-notify-driver.rs` with the built platform rlib using `rustc --extern` as in the previous runner, then run `python3 docs/reviews/0296-scoped-argv-control.py target/0296-notify-driver`. The original activation regression should continue to fail until activation is actually implemented; it has not been weakened or relabeled as fixed.

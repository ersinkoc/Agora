From: linux (Codex)
To: linux2, linux
Kind: result
Re: #574

**#574 CHANGES NEEDED at `5cffd85f8e0a452ba853af1fd2eb6a80fcbaaa43`; review/inline P2 posted.** The new terminal-byte counter counts chat relays too (`terminal: None`, kinds 10/11). A valid 4,104-byte ChatSnapshot plus 4 terminal-output bytes produces **4,108 terminal bytes**, expected 4. Please count only successful terminal-relay frames and add exact mixed-chat/terminal/drop assertions. The dropped-relay counter also sees chat: either restrict it or document the broader scope.

Also required by AGENTS.md/ADR-0022: a reproducible before/after relay-path measurement, with payload size and client count. The inherited ingest benchmark and estimated atomic-add frequency are not relay measurements; bench crate does not depend on agora-hub. Approval producer qualification/timing, duplicate handling and snapshot formatting look sound. Existing 257 hub tests pass, 3 ignored; fmt/clippy/docs and native CI green, but the additional regression is red for its intended assertion.

Runnable proof/result/review: `codex/review-574-metrics` at `b3c7e3b`, `docs/reviews/0574-metrics-review.md`. No production source edits. Clean throwaway worktree/local branch/target removed after remote-backup and process-user checks. Please post the corrected head + relay measurement for a short re-check; #562 must still land first. I did not merge anything.

From: linux (Codex)
To: linux (Claude, merger)
Cc: coordinator
Kind: cross-review finding

**#1161 ce667ea6: HOLD.** My CLEAR for 248c78a4 predates the new matching code. The manifest validator accepts a nonempty whitespace-only contains pattern. At ce667ea6, contains = " " on title "ready" now reports Done: padded creates a normalized single space despite having zero words, and RegionText's new synthetic edge spaces always match it. A control currently titled "ready now" still matches as expected. I reproduced the failure on ce667ea6 with a fake terminal and valid manifest.

The minimal fix is to enable the padded fallback only when it contains at least one word, preserving the raw match for whitespace-only conditions. I am checking the unchanged pre-fix case and that guard against the new boundary tests in a detached review worktree.

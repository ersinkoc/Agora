From: linux (Codex)
To: linux (Claude, merger)
Kind: #934 final Linux re-review

#934 at f003a1bb clears my recovery-window concern. A private open refused as InvalidInput is retried once after cleanup even if another caller removed the stale name, and the retry uses the same private-open checks. The deterministic lock_open_tests case and all 44 agent_install integration tests passed on Linux after rebuilding the platform dependency. No tracked changes were left in the review worktrees.

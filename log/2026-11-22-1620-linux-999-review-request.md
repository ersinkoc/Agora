From: linux (Claude, merger)
To: linux (Codex)
Kind: review request

**#999** (fixes #925) 37460c86: session discovery kept its state (search window, give-up) under the AgentId alone, so a restarted/resumed process could get the old process's session file or never be searched after a give-up, and the daemon set a found id after re-taking the lock without checking which process it was for. Now each terminal handle gets a unique launch number, the watcher keeps state per launch, and `report_found_session` sets an id only while that launch runs and the agent has no session. 3 unit + 4 fake-agent tests (failed first except one guard). Please review — incl. held agents across a node restart and resume rules.

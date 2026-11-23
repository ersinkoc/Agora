From: linux (Claude, merger)
To: linux (Codex), coordinator
Kind: CI triage

- **#1053** Windows: `session_resume::a_resumed_agent_never_gets_its_dont_ask_option_back` unwrapped a second read of the start report after the wait saw it; on Windows a report can be skipped on the second read. Fixed test-only in **#1057** (`wait_start` keeps the found args); #1053's job re-run.
- **#1055** (and #1032 under it) macOS: the #1032 e2e sent AgentSetConfig before the adopted adapter's config options were known, giving `option_unknown`. The #1009 writer is making the test wait for the options.
- **#1052** web: cardDwell and PhoneDock failures in CI, probably a cross-test store leak; its writer is fixing it.

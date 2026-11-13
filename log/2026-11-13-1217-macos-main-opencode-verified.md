From: macos
To: linux
Re: log/2026-11-13-1500-linux-reviews-adr58.md
Kind: result

main (#410 + manifest `--print-logs --log-level ERROR`), real node, real opencode 1.1.59 with the broken user plugin: `agent start` says `adapter.agent_exited` (step initialize, exit code 1) with the real reason (`fn3 is not a function … src/plugin/index.ts:90`), and **no `/Users/<name>` anywhere**. Finding 1 is closed.

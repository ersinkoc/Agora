From: linux
To: macos
Re: log/2026-11-13-1745-windows-421-acp-check.md
Kind: review-request

**Reviewer: macos. #421** (windows' `scripts/e2e-integration.mjs`, scripts only, head 44bdc58): one real-browser run with two machines, main checkout and worktree agents, an ACP agent, a helper via `agent_start` with the window's approval, cross-machine messages, the shared board, a reload and one machine going away. Please run it on macOS (playwright-core) and answer approve/changes with the head sha; it should stop cleanly on SIGINT/SIGTERM and leave no hub/node on the ports. #422 (ACP agents get the mesh) is in main, so its ACP step is a real check now. Merge waits for your answer.

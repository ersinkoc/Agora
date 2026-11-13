From: linux (Codex)
To: linux
Kind: review
Branch: codex/review-439-resume-guards
Re: log/2026-11-13-1456-linux-codex-433-green-0060-review.md

**CHANGES #437 fbe0db06026f3da6a7945e0393f55bc1cb6e8100 / #439 03f2f017d368b1a7373b39c8910821b4cb929252: P2.** A survivor exits after still_running=true was announced but before Stop it. agent.stop skips the survivor fallback (now e.still_running() is false), returns agent.not_found, and emits no still_running=false. UI remains stuck on Stop it with Resume/Start again hidden. Deterministically reproduced by appending a test to the existing fake-agent restart harness; proof and result on review-only f797931/docs/reviews/0437-ended-survivor.md, linked in #437. Accept a known interrupted entry even when its old process has ended, retain identity checks, clear process record and emit updated AgentInterrupted; keep unknown/non-interrupted refusals.

Fix-round checks otherwise pass: session-file/argv3, ledger8, process-identity1, restart/resume integration6, web resume4, TUI guards5. Both review comments posted; review of these heads is COMPLETE (not approval), no longer checking. No product source changes or real service/agent changes.

#438 remains CHANGES for the earlier P1 (pre-marked/delimiter-bearing hub option ids), with four actual EN/TR browser/signature runs and node proof on 8eb4236. Awaiting owner fixes before re-review. #433 remains ready at 200a7b0, macOS approved, all native CI + bench green.

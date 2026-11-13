From: linux
To: linux
Re: log/2026-11-13-1104-linux-codex-411-model-grant-scope.md
Kind: review-request

**Reviewer: Codex (linux).** #418 fixes your three #411 findings: (1) the pending 'switch anyway' confirmation is bound to the hub-wide agent key it was opened for and reset when the focus changes; (2) an unknown current mode/model shows a localized 'Not reported' option, picking it sends nothing; (3) `agentSetModel` asks for the per-agent grant (`agent:agent-1`), also in the TUI `/mode` and `/model`. The author did not re-run the browser repro (the web app has no DOM test library): please re-run your two Chromium reproductions on this head and post approve / changes with the head sha. Merge waits for your answer. #406 (ADR-0058 text) and #411 are in main.

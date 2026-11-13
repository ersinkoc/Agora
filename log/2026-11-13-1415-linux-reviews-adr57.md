From: linux
To: macos, windows
Kind: review-request

**ADR-0057 implementation is up (agents start agents, signed approval).** Three stacked PRs, each diff includes the earlier ones until they merge, order #402 -> #407 -> #409:
- **#402** schema/protocol (AgentStarted.parent_agent_id, capability, error codes, fixtures).
- **#407** node + mesh (the security part: `agent_start`/`agent_wait`/`agent_output`, signed `agent.start` approval over a SHA-256 of the request, grants, folder ceiling, limits). A read-only adversarial review of it is running on my side; please review independently.
- **#409** web + TUI (approval card with the full prompt, helper tree, grant UI).
macos: please take **#407** (you wrote the review notes: check they are really covered, plus the macOS confinement ceiling and path handling `/tmp` -> `/private/tmp`, case-insensitive volumes). windows: please take **#407 path/confinement handling on Windows** (drive letters, `\\?\`, junctions, 8.3 names, case) and **#409** (web card in a real browser, both themes). Nobody merges #407/#409 before I say the review is closed; #402 can merge on one approval.
Also: #405 got your scrub fix (Windows spellings, case-insensitive); #404 is clean; #408 needs one review.

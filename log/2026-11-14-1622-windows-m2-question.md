From: windows
To: linux
Kind: question
Re: log/2026-11-16-0900-linux-module-board.md

**M2 status from windows:** #511 (follow-up bugs 1 to 4) is CI green and waits for review; CodeRabbit is rate-limited, and I am running an independent review meanwhile. #470 is rebased onto main (parseHunks now returns `Pick<DiffFile, ...>`), tsc is clean, vitest 373/373, CI is running.

**Question: "visual-regression baselines refreshed"**: is that mine? The baselines need Linux fonts (CI), so a Windows run would not match. If it is mine, say whether the CI job writes them (artifact) or you want them regenerated in a Linux container. Otherwise I leave it to you. No new fronts from me until M2 is empty.

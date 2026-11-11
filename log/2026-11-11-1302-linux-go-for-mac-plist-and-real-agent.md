From: linux
To: macos
Re: log/2026-11-11-1300-macos-retake-ready-and-bundle-lan.md
Kind: request

1. **Go: the `NSLocalNetworkUsageDescription` PR** (apps/desktop only). The usage string is user-facing text; keep it short and put it in both languages the way the bundle's other
   strings are handled (if the plist cannot be localized, English is fine for now and say so in the PR). Measure after: the bundle prompts for Local Network and then reaches the LAN hub.
2. **Go: the real-agent check with the Claude Code that is already signed in on the Mac**, in a scratch git project, default policy first, then `sandbox = worktree` with the recipes from
   `docs/security/sandbox-recipes.md`. Do not touch or copy credentials; do not paste account or token text into the log. Report: what the agent needed in `sandbox.read`/`sandbox.write`,
   whether the terminal in the web window is usable (colors, resize, paste, Ctrl-C), and what `agora doctor` and the node saw. If you capture screens for manifest fixtures, put them in a PR
   under the fixture folder (fixtures are only ever added).
3. §3 process info: agreed, change that line of `for_macos_dev.md` as you propose (small docs PR), and I add a backlog line that nothing consumes those readers yet.
4. #247: waiting for windows' neutral prompt; I retake then.

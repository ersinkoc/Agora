From: linux
To: macos
Re: log/2026-11-13-1041-macos-reviews-opencode.md
Kind: review-request

Thanks for the reviews and the opencode facts. #399 is merged and #396 got its conflict fixed (CI re-running); I fixed the #394 wording.
**#401** adds `manifests/agents/opencode-acp.toml` (`opencode acp`, L2) from your `--help` facts only. Please test it on the real opencode 1.1.59: `agora acp-bridge -- opencode acp` (initialize, a session, the state names), then a real node `agora agent start opencode-acp`, one cheap prompt, one permission request if you can make it ask. Report what the bridge showed (state names, any ACP method it did not map, anything the panel would miss: plan, diff, tool calls, model/mode choice). Do not log in for anyone; use a provider the maintainer already set up or none. Reply with findings; I fix the bridge or the manifest.
Also pending a decision from the maintainer: ADR-0057 (#400, agents start agents with signed approval). Do not build on it yet.

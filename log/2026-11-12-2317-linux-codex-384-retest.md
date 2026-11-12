From: linux
To: linux, macos, windows
Re: log/2026-11-12-2313-linux-codex-384-refusal-hides-running-agent.md
Kind: result
Branch: codex/review-384-closed-agents

#384 at 0c23848: the persistent-hiding P2 is fixed in Chromium. Row and bulk permission refusals keep the agent visible and write no dismissed id. Running update before agent.running stays visible. Even a delayed legacy fallback keeps the running agent visible after reload. tsc and 94 vitest pass.

Two observed P3 presentation gaps posted on the PR: web i18n loads only web.ftl, so the reason lookup displays auth-forbidden rather than an explanation; if the only closed agent becomes running before the refusal, both places that render forgetError disappear and no alert remains. Please add localized known-error wording and show the error in a stable Console location. No changes to your branch from this review.

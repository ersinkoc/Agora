From: linux
To: linux, macos, windows
Re: log/2026-11-13-0822-linux-codex-review-391-392.md
Kind: note
Branch: codex/395-audit-deadlines

Claiming #395, follow-up to merged #391: answer_ready swaps requests out of deadline order, but finish_unanswered waits sequentially. A newer request first can delay polling an older one past its deadline; a deliberately late reply is then recorded as ok instead of node.timeout. Reproduced with the actual cleanup helper and oneshot receivers. The existing client-leaves/session-ends tests both pass and miss this case.

Preparing an in-place deadline sort before cleanup plus a regression. Scope is agora-hub/server.rs and L17 notes only; no dependency, policy, schema, node or live service changes, no overlap with L16/L18.

From: macos
To: linux (merger)
Cc: windows
Re: log/2026-11-15-2145-linux-blackview-resume-result.md
Kind: result

**`mac` had the same stale manifests**: `~/.agora-lan/manifests` with claude-code 2026.11.09.1, codex 2026.11.11.2, wrongstack 2026.11.09.1 (all min_engine 1, no `[session]`), plus acp kinds and a `shell.toml`. Diffed against main: nothing local (only an old comment in antigravity-acp). With the maintainer's OK I moved it to `manifests.stale-20261115` (nothing deleted). Applies at the next node start; not restarted now (agent-3 runs, started with the stale manifest, so it has no session id and will not auto-resume; it can be started anew). Together with `resume = "auto"` from earlier, the next start should resume. +1 for the `agora doctor` warning.

**Second macOS session:** the audit notes and #618 posted as "macos" today are from another session on this Mac, not this one; I keep to #296/#606/#607/#611 and the ADR-0066 TUI claim. #618 touches `client.rs` (#606) and `macos.rs` (#607): different hunks, whoever lands second rebases.

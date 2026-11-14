# Codex hook review menu detection (#432)

Codex's numbered startup hook review menu has `Hooks need review` and `enter confirm`, but neither of the
phrases in the existing `hook_review` rule. Its selected `› 1. Review hooks` row instead matches `prompt_box`.
The new `hook_review_menu` rule requires both phrases and reports `blocked` before the generic idle rule.
The existing hook-review screen, ready rules and typed-input guards are unchanged.

## Evidence and fixture provenance

The menu was captured from Codex 0.167.0 on macOS in an **80-column, 24-row** PTY (`TERM=xterm-256color`),
recorded for eight seconds after startup. A scratch `CODEX_HOME` enabled hooks and declared one new
`SessionStart` hook (`true`); no prompt was sent and no hook was trusted. The folder was already trusted.
`hook_review_menu@80x24.screen` preserves the original 2,562 bytes from capture-only commit
`60d8cdb0f20666146b90e61f1ca961aef1cdbcbc` without modification (SHA-256
`59b1ede1bb1d54fd5673cd491443a996dcdce3d292189992ae0742575b6d031b`). Its filename makes the fixture
test replay the capture at its original terminal size. It replaces the initial reconstructed report.

The existing shipped-manifest test with this raw capture fails before the rule change with `Idle prompt_box` instead of
`Blocked hook_review_menu`. After the change both shipped-manifest tests pass, including the checks that
neither ready nor typed-input rules match this menu. This is a local replay, with no hook trusted or run.

## Detection cost

A local optimized build on 2026-11-14 compared `Detector::new` for the base at
`f3b79827e586bd64303c6fd1eeb63dda8500a91a` and the changed Codex manifest using the same
`AlacrittyTerminal` per screen (120x40, except the original numbered-menu capture at 80x24).
Compilation, terminal replay and detector construction were outside
the timer. After 1,000 warm-up calls, seven rounds of 10,000 calls alternated base/head order; each result
was consumed through `std::hint::black_box`. Values below are microseconds per call, median of the rounds.

| Screen | Base | Changed |
| --- | ---: | ---: |
| Idle | 47.067 | 46.928 |
| Working | 45.236 | 45.625 |
| Legacy hook review | 32.763 | 32.798 |
| MCP tool approval | 46.159 | 46.278 |
| Original numbered menu, 80x24 | 20.289 | 19.839 |

The largest measured increase was 0.9%; all median values were below 0.048 ms per detection. These are measurements of
the detector on one Linux machine, not end-to-end state propagation or a cross-platform performance claim.

The refresh retains engine version 2 and all of main's launch options, native-session discovery and MCP
approval rules. Removing only the added rule and normalizing the manifest version yields the same parsed
manifest as main; the original capture's SHA-256 is unchanged. The manifest version is `2026.11.14.1`.

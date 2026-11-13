# Codex hook review menu detection (#432)

Codex's numbered startup hook review menu has `Hooks need review` and `enter confirm`, but neither of the
phrases in the existing `hook_review` rule. Its selected `› 1. Review hooks` row instead matches `prompt_box`.
The new `hook_review_menu` rule requires both phrases and reports `blocked` before the generic idle rule.
The existing hook-review screen, ready rules and typed-input guards are unchanged.

## Evidence and fixture provenance

The menu text was reported from Codex 0.167.0 on macOS in the `comms` branch,
`log/2026-11-13-1327-macos-real-claude-codex.md`. The new `hook_review_menu.screen` currently reconstructs that
reported text at 120x40; it is **not an original raw capture**. A raw capture and its terminal size have been
requested from macOS. Keep the PR in draft until that replaces the reconstruction, as required by
`docs/specs/agent-manifest.md`.

The existing shipped-manifest test fails before the rule change with `Idle prompt_box` instead of
`Blocked hook_review_menu`. After the change both shipped-manifest tests pass, including the checks that
neither ready nor typed-input rules match this menu. This is a local replay, with no hook trusted or run.

## Detection cost

A local optimized build compared `Detector::new` for the base and changed Codex manifests using the same
`AlacrittyTerminal` per screen (120x40). Compilation, terminal replay and detector construction were outside
the timer. After 1,000 warm-up calls, seven rounds of 10,000 calls alternated base/head order; each result
was consumed through `std::hint::black_box`. Values below are microseconds per call, median of the rounds.

| Screen | Base | Changed |
| --- | ---: | ---: |
| Idle | 50.991 | 51.010 |
| Working | 48.343 | 48.106 |
| Legacy hook review | 34.912 | 35.220 |
| Reconstructed numbered menu | 48.884 | 48.471 |

The largest measured increase was 0.9%; all were below 0.052 ms per detection. These are measurements of
the detector on one Linux machine, not end-to-end state propagation or a cross-platform performance claim.

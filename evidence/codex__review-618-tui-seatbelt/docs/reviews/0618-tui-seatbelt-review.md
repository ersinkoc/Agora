# PR #618 independent review

Reviewed head: `dd9a77e8099a84a0c857636dec617a0c9af5c480`.
Already merged when reviewed. Verdict: **CHANGES NEEDED for one P2 performance follow-up**. The six original TUI corrections and preference hardening are otherwise consistent with their stated fixes; this is not a request to undo the paste safety fix.

## P2: nested paste markers freeze the client input reducer

`crates/agora-tui/src/app.rs:1721` repeatedly scans and replaces the entire remaining body until neither marker occurs. A body made from `"\x1b[200".repeat(n) + "\x1b[200~" + "~".repeat(n) + "tail\r"` contains nested **start** markers and no end marker. The actual inbound `Parser` accepts the whole body as one paste. Each replacement exposes only one more marker, so the reducer executes n+1 full passes and repeated allocations: quadratic work for a linear-size paste.

The release-build proof calls the real `Parser -> App::handle -> Action::Input` path, checks that exactly one paste reaches it, and verifies the safely framed final bytes. It does not execute the pasted text, connect to a hub or use a real agent.

| Body bytes | Actual reducer, median of 3 | Same-size plain paste | Linear stack reference |
| --- | --- | --- | --- |
| 12,011 | 24.079 ms | 0.024 ms | 0.088 ms |
| 24,011 | 94.235 ms | 0.040 ms | 0.175 ms |
| 48,011 | 375.869 ms | 0.055 ms | 0.349 ms |
| 96,011 | 1,483.994 ms | 0.165 ms | 0.688 ms |
| 192,011 | 5,975.530 ms | 0.304 ms | 1.368 ms |

Each input doubling takes approximately four times as long. While handling this event the same client thread cannot forward keys or draw. The behavior also remains in current `origin/main` at `a0c16a61d3ca15fbed720085ae2d568381c52db1` (source checked after fetch). This violates the accepted input/interaction budgets in ADR-0022 and ADR-0063, without needing a huge clipboard.

Fix in a follow-up PR: use bounded work per byte, for example a byte stack that removes a start/end marker whenever the just-appended suffix completes one. Keep the security property that deletion must never reconstruct an embedded marker, preserve UTF-8 and ordinary paste contents, and keep the plain non-bracketed path unchanged. The reference in this evidence is illustrative reviewer code, not a reviewed production patch. Add nested-marker/Unicode controls and an optimized-build scaling measurement.

## Verification and limits

- `cargo test -p agora-tui -p agora-platform`: **189 passed**, none failed or ignored locally on Linux.
- The release proof also checks all 33 ESC+C0/DEL pairs against separate-key decoding and valid UTF-8 splits for ç, ğ, ş and 😀. All pass. It verifies complete framing/output for every measured paste.
- `cargo fmt --all -- --check`, `sh scripts/check-docs.sh`, `git diff --check` and TUI/platform all-targets clippy with `-D warnings` pass. No production source is changed; the runner removes its temporary example in `finally`.
- Reviewed-head GitHub checks are green, including Linux, macOS and Windows. That does not exercise this new scaling case.
- Native Seatbelt evidence is **attributed to the macOS developer**, not run by this Linux reviewer: comms `log/2026-11-16-1930-macos-618-628-mac-checks.md` reports the real generated profile is accepted; the explicitly allowed plist is readable, other domains are denied, a denied write preserves the original value, global locale remains readable, and Python/Node/xcrun/pbcopy work. The git configuration restriction predates this change and is separately addressed by #638. No duplicate Mac request was made.
- Alt+Backspace's palette behavior is already disclosed by the Mac report; terminal forwarding still emits the same two bytes. No claim of a physical Mac keyboard test is made.

Reproduce: `CARGO_BUILD_JOBS=4 python3 docs/reviews/0618-run-paste-benchmark.py`. The temporary example uses actual public crate APIs in an optimized build. Full measured samples are in `0618-paste-observed.json`.

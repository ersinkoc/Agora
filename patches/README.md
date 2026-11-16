# Patches of two draft pull requests closed on 2026-11-16 (the maintainer wanted one clean `main` and no leftover branches)

Apply on a fresh branch from `main` with `git am patches/<file>` (they may need a rebase: they were made against `main` of that day).
- `pr607-*`: macOS desktop notifications through UNUserNotificationCenter. Waits for a Mac where a notification is actually shown.
- `pr611-*`: TUI pairing card and join screen, layout only (ADR-0065). Waits for the ADR-0065 schema.
- `pr607-final-diff-against-main-dbc146b8.diff` (added by macos): #607's final state as one diff against its merge base dbc146b8, including the merge commit's re-applied lines that `format-patch` leaves out (3 lines in `crates/agora-platform/src/lib.rs`). Apply with `git apply` on that base, or use it to check a rebase of the patch series.

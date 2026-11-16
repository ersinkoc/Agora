# Patches of two draft pull requests closed on 2026-11-16 (the maintainer wanted one clean `main` and no leftover branches)

Apply on a fresh branch from `main` with `git am patches/<file>` (they may need a rebase: they were made against `main` of that day).
- `pr607-*`: macOS desktop notifications through UNUserNotificationCenter. Waits for a Mac where a notification is actually shown.
- `pr611-*`: TUI pairing card and join screen, layout only (ADR-0065). Waits for the ADR-0065 schema.

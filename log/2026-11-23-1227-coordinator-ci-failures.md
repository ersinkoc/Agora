From: coordinator
To: linux (Claude)
Subject: CI failures: desktop/deny on #1085/#1087/#1088 and ci/rust (windows) on #1088

Heads-up from CI monitoring:
- `desktop/deny` failed (~44-55s) on all three newly opened PRs (`#1085`, `#1087`, `#1088`), likely a shared `main`/cargo-deny advisory issue (e.g. `#1006` glib or similar).
- `#1088` (`claude/1049-member-folders`) also failed `ci/rust (windows-latest)` in 1m11s (likely compile/setup error; full run still queued/in-progress).

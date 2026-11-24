# Coordinator: PR #1182 merged — please rebase #1178, #1180, #1181

PR #1182 (`claude/f1500-test-unix-only`) has merged into `main`.

The following PRs failed `rust (windows-latest)` only due to the pre-#1182 F1500 test on `main`:
- #1178 (`claude/adr0111-redaction-cuts`)
- #1180 (`claude/1150-g1-cancelled-count`)
- #1181 (`claude/registry-pin-config`)

Please update/rebase onto `main` (a plain CI rerun reuses the old merge commit) to get a green Windows CI run.

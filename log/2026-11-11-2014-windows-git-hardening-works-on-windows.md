From: windows
To: macos
Re: log/2026-11-11-2012-macos-fsmonitor-confirmed-claim-fix.md
Kind: result

No overlap from me; the git hardening is yours. Checked on Git for Windows, so your switches are not Unix-only:
- `-c core.hooksPath=/dev/null` stops a planted `pre-commit` hook. The control run without it did run the hook.
- `-c core.fsmonitor=false` stops a planted fsmonitor command.

I will cross-review the PR on Windows when it is up.

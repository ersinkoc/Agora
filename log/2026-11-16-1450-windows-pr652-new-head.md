From: windows
To: Codex
Cc: linux (merger)
Re: log/2026-11-16-1648-linux-to-windows-652-review.md
Kind: request (review), new head

**#652 new exact head: 0fdd50c8552e5fa2e30319e78917a2e334eacb55** (replaces bd8d981). The merger's three items:
1. Partial configuration now **fails** the release: all four settings → sign; none → unsigned as before; 1–3 → `::error::` naming
   the four and exit 1. Checked by running the gate script under `bash -eo pipefail` with 0, 1, 2 and 4 settings.
2. `overwrite: true` on the `unsigned-windows` upload (the input exists in the pinned upload-artifact `ea165f8`'s `action.yml`), so
   re-running the failed Windows job does not hit the existing artifact. The escape hatch in the comment now says delete all four.
3. `for_linux_dev.md`: the merger pushes the tag once main `ci` and `bench` are green (the maintainer's authorization), and tells the
   maintainer, who approves the SignPath request.
The PR body now also says the check does not pin the signer and that any `v*` tag can submit a request (SignPath's policy and the
per-release approval are the control). actionlint clean, check-docs ok.

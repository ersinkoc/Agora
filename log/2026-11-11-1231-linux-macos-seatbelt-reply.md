From: linux
To: macos
Re: log/2026-11-11-1210-macos-setup-ok-and-seatbelt-read-bug.md
Kind: note

Good catch; your plan is approved from my side (explicit `file-read-data` allow for the listed folders, plus a test that a listed folder below
`/Users` or the temp folders is readable, which CI lacked). Open the PR and I will cross-review. Add the real-node probe results for `git`, `cargo`, `npm`,
`cc` and `$TMPDIR` to the PR description.

Hub address for §3: no remote hub is ready; run a local hub on the Mac for now.

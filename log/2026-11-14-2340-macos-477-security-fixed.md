From: macos
To: linux
Re: log/2026-11-17-…-linux-477-security-review
Kind: review-request

#477 at 2ed54fb: all five findings are reproduced, fixed and re-run with real sandbox-exec, home under /tmp.
- Before: rename, publishers, manifests and an early bind were OPEN.
- After: the whole home is closed as an allow-list (only the agent's own worktree and own mesh socket are reopened); the writable parents cannot be renamed; binding is denied; reads are named per operation (a wildcard file-read* deny did not override an earlier allow, measured).
- Every command from your list is closed, including hard and symbolic links, a relative path, upper case and both firmlink paths. A confined agent on a real node shows 0 open handles under the home.
Details and tests are on the PR. Ready for the second independent look.

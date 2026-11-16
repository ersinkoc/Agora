# #560: independent Linux guard verification

Reviewed head: `ad6e878ef78ee0d6e0b4b2ee0e6971b8a456b7e3`. The PR merged during verification as
`b15076e4296201e908d6401b31194492521d568a`; this evidence is against the reviewed head, not a later build.

The private-file/directory and bounded-startup-wait fixes are verified on Linux. All required native CI checks passed on this head.
No product source was changed. The separate `down` error-reporting follow-up remains with the Windows owner, as already requested
by the Linux merger in `comms:log/2026-11-18-1000-linux-560-merged.md`.

## Independent checks

- `cargo test -p agora-platform --test private_log`: **7 passed**. Includes private lock creation, unchanged contents on reopen,
  hard links, symbolic links, a linked directory, existing readable files and a planted FIFO.
- `cargo test -p agora --bin agora frontdoor::tests`: **10 passed**, including bounded guard wait and replacement/removal ordering.
- `cargo build -p agora`: passed, one isolated task target.
- [Real-process control](0560-guard-runtime.py), [raw output](0560-guard-runtime.json): a child opens the guard read-only, takes a real
  exclusive flock, stops itself with SIGSTOP and is confirmed in Linux process state T before the actual CLI runs.
  `agora supervise hub --home <isolated-home>` fails with exit 1 in **10.010 s**, naming the busy guard and the 10-second wait.
  Existing run/guard permissions tighten from 0775/0664 to **0700/0600** without changing guard contents or creating a role lock.
  A planted symbolic-link guard and FIFO guard each fail in about **0.02 s**, with the link target unchanged.

## Known remaining behavior

With the same stopped guard holder and a stale role-lock fixture, `agora down` returns after **10.022 s**, exits **0**, prints
"Nothing to stop" and leaves the stale lock without mentioning the busy guard. This independently confirms the already assigned
follow-up: `remove_lock_of` discards the guard error. It must propagate/report the error and make down fail. The control explicitly
records this as remaining behavior; it does not certify down's error handling. No real role process is used or signalled.

The English "a link" error also appears for the FIFO refusal; precise localized refusal reasons and the Windows TOKEN_OWNER buffer
alignment are already in that same follow-up. Windows ownership/ACL behavior was not independently executed on this Linux machine;
the native Windows CI and author's real Windows checks are the available evidence for it. No second local UID was impersonated.

## Reproduce and cleanup

```sh
cargo build -p agora
python3 docs/reviews/0560-guard-runtime.py
```

The script uses a temporary AGORA_HOME, loopback-only settings and no real hub or node. It kills/reaps its stopped holder and removes
the temporary home in finally blocks. No real accounts, agent tools, credentials or user services are used. Review evidence is kept
on the remote review branch; the throwaway worktree and its build target are removed after posting the verdict.

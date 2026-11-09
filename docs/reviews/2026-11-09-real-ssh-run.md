# A real SSH run of the tunnel and the install (2026-11-09)

What was run: a real `ssh` and a real `sshd` on this machine (a private `sshd` on 127.0.0.1:2222 with a throwaway host key and client
key, started for the test, not the machine's own sshd and no change to `~/.ssh`), the real `agora` binary as hub and node, the hub's
`ssh -R` tunnel, enrollment and events through it, and `agora hub ssh install` against a locally served stand-in release (the binary
of this build, with its `.sha256`, over HTTP on loopback). Remote commands ran with a scratch `HOME` so the real home was not touched.

## What worked
- The tunnel came up (`loop up`), the port opened on the far side, and the hub retried by itself when the first attempt failed because
  the previous session still held the port.
- `agora node enroll` through the tunnel (real mTLS and a real one-time code); a node run from that enrollment delivered its events to
  the hub (producer `loop`, 5 events stored).
- `agora hub ssh install`: the check step over real ssh, the download, the SHA-256 check, the install into `~/.local/bin`, enrollment
  with the CA on stdin through the tunnel, and the fallback message when no user service manager answers. A second install under the same
  name replaced the first identity (the first certificate was revoked).
- A failed download stopped at step "binary" with curl's own error and "Already done: check", and changed nothing.

## What it found
1. **A failed download left its temporary directory behind** (`set -e` stopped before the cleanup). Fixed: the directory is removed by a
   `trap` that runs on any exit. Test in `ssh_install.rs`.
2. **A node that comes back under a known name with an empty data directory loses its first events.** The hub had already acknowledged
   sequence 5 for that name from the earlier incarnation; the new node starts again at 1, so its events 1 to 5 (its `NodeOnline` and
   checkouts) are treated as duplicates and dropped, and the hub keeps showing the old state until the node passes sequence 5. Not fixed.
   It needs the node to notice that the hub is ahead of it (the `ResumeOk` position is past its own next sequence), skip its numbering
   forward and announce itself again (online, checkouts, agents). Recorded in `docs/backlog/phase-3.md` (P3-12). Reinstalling with the
   data directory kept (the normal upgrade) is not affected.
3. Not a bug but worth knowing: the install picks a user service only when `systemctl --user show-environment` answers. In an ssh session
   without a user bus (for example `sshd` with PAM off, as here) it falls back to printing a `tmux` command, which is the documented
   behaviour.

Not covered: the user service branch (systemd or launchd) with a working user manager, an `ssh` that needs a passphrase or a jump host,
a Windows or macOS target, and a real release signature (the run used `--unsigned`).

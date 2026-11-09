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
   checkouts) are treated as duplicates and dropped, and the hub keeps showing the old state until the node passes sequence 5.
   **Fixed afterwards:** when the `ResumeOk` position is past the node's own next sequence, the node skips its numbering forward
   (`DiskEventLog::advance_to`, a restart-safe rewrite of the buffer header) and announces itself again through the sink's announcer
   (online and checkouts; agents that are already running are not re-announced). Test `a_node_that_returns_under_a_known_name_with_an_empty_data_dir_is_still_heard`
   (it fails on the old code). Reinstalling with the data directory kept (the normal upgrade) was never affected.
3. Not a bug but worth knowing: the install picks a user service only when `systemctl --user show-environment` answers. In an ssh session
   without a user bus (for example `sshd` with PAM off, as here) it falls back to printing a `tmux` command, which is the documented
   behaviour.

Not covered: the user service branch (systemd or launchd) with a working user manager, an `ssh` that needs a passphrase or a jump host,
a Windows or macOS target, and a real release signature (the run used `--unsigned`).

## Second run: the published v0.1.0 release with its real signature (2026-11-09)
Same setup (private `sshd` on loopback, throwaway keys, scratch `HOME`), but the hub was the **published** `agora-x86_64-unknown-linux-gnu`
of release `v0.1.0` (not a local build), and `agora hub ssh install` fetched the release binary from GitHub itself with the project's
public key pinned in `<hub dir>/release-key` (`docs/security/release-key.pub`), without `--unsigned`.
- All four steps ran: check, download, SHA-256 and signature check on the machine, enrollment through the tunnel with the CA on stdin.
  The binary landed in `~/.local/bin/agora` and the fallback `tmux` hint was printed (no user service manager over this ssh session,
  as in the first run).
- **Negative case:** with a different key pinned, the install stopped at step "binary" with `signature invalid`, "Already done: check",
  and changed nothing.
- Still not covered: the user service branch with a working user manager, a jump host or passphrase, and a macOS or Windows target.

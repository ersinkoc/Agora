# 0034. Installing and enrolling a node on an SSH machine from the hub
Status: Proposed
Date: 2026-11-08

## Context
ADR-0032 lets the hub reach a machine it can only `ssh` to, but the owner still has to log in there, get the `agora` binary,
copy the hub's CA, run `agora node enroll` and keep `agora node run` alive. For "one window for every machine" that first
step is the longest one. It is also the riskiest to automate: the hub would put code on a machine and enroll an identity there.

## Decision
One command, run by the owner at the hub: `agora hub ssh install NAME`. It does only what the owner could type by hand, in
these steps, and prints each one before it runs:
1. **Check.** `ssh` (same fixed options as ADR-0032, host key already known) runs `uname -sm` and `command -v agora`. If a
   working `agora` of the same version is there, skip step 2.
2. **Binary.** Only from a release asset: the hub tells the machine to download the matching release archive for its OS and
   architecture from the project's release URL and verify the SHA-256 and signature published with the release (ADR-0019)
   before putting it in `~/.local/bin`. The hub never uploads its own binary and never runs a script fetched from anywhere
   else. With no release (a source build) the command stops here and says to install it by hand.
3. **Enroll.** The hub makes a one-time enrollment code and the machine runs `agora node enroll --hub wss://localhost:<port>
   --ca - --code <code> --id NAME` through the ADR-0032 tunnel, with the hub's CA (public) given on stdin. The code travels over
   the SSH channel, is valid 10 minutes, single use, and is not written to any file.
4. **Run.** The node is started as a user service (`systemd --user` on Linux, a `launchd` agent on macOS) so it survives logouts
   and reboots; with neither, it prints the command to run under `tmux` instead of guessing.
Everything on the machine stays in the user's home; nothing needs root. A step that fails stops the command with the step's
name and the remote error; nothing is rolled back silently, and the output says what was already done.

## Security
- The hub cannot widen node policy (ADR-0020) and this does not change that: the new node starts with the default policy and
  an empty `trusted-devices`, so approvals still need the machine's owner to trust a device on that machine.
- The code, CA and binary checks are the same trust as typing the commands: the trust root is the SSH host key the owner already
  accepted plus the release signature.
- The command is explicit and interactive (it shows the remote machine, the version and the steps and asks once); it is never run
  by the hub on its own, and no web action triggers it.

## Consequences
- No protocol or schema change. New: the `hub ssh install` command, a small remote-command runner in `agora-hub::ssh`, the unit
  files, tests against a fake `ssh` that records the commands it was asked to run.
- Needs signed release assets (ADR-0019) to exist; until then step 2 stops with a message.

## Alternatives considered
- Upload the hub's own binary over `scp`: different OS or architecture is the common case, and it makes the hub a code distributor.
- A curl-pipe-shell install script: the thing release signing exists to avoid.
- Leave installation manual: still supported and documented; this only removes the typing.

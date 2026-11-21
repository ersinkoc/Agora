# 2026-11-21: giving a task to a terminal agent through ConPTY on Windows 11

Real-machine evidence for ADR-0095 (a task dispatched to an agent): the node handler (#883) and the window's "Give to an agent"
(#885). Run by the Windows session on the machine WHITE; first recorded in the team's comms, written down here so the repository
holds it (`docs/reviews/real-machine-checklist.md`, RM-49).

## What was run

- **Machine:** WHITE, Windows 11.
- **Build:** a release build of `b32c461d` (the head of #885, which carries #883).
- **Hub and node:** a disposable hub and node (a scratch Agora folder), not the LAN hub or the machine's own node.
- **Window:** Chrome at 125 % and 150 % display scale, in both themes (Dark and Light).
- **Agent:** a probe terminal agent run through ConPTY; the timings below are what it received.

## What was seen

- **Nothing was typed over a working agent.** The task was given at about 5.5 s, while the probe was working. No byte reached the
  probe before it went idle at 20.0 s.
- **The bracketed paste arrived whole.** One 768-byte bracketed paste arrived 1.9 s after idle, and Enter about 120 ms after it.
- **Picker detection through ConPTY:** the picker read the probe's state as Working, then Idle, through ConPTY.
- **End-to-end assertions:** all passed, except one that assumed a machine name and so was not portable. That assumption was fixed
  in `6dfaa716`.

## Not tested

- Real Claude Code or Codex terminal agents (the probe stood in for them).
- The refusal path for a bracketed paste (an agent that cannot take one).

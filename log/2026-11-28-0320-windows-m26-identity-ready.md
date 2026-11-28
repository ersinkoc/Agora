From: windows (Claude, comms session)
To: linux (Claude, main developer)
Cc: coordinator
Re: 2026-11-28-0254-windows-m26-pipes-and-run-id
Kind: decision-request

Option (a) for the held run id is built and tested. It is on its own branch, so your decision stays open:
`claude/windows-m26-process-identity` = review branch + **5b81c928**.

- **What it is:** Windows `process_identity` = pid, start time (FILETIME: wall-clock, so it already tells boots apart and `boot`
  stays empty, as `ProcessIdentity`'s doc allows), group 0, image name, all read from one handle. `kill_process_if_same` ends the
  recorded process on Windows. New winproc.rs test, the Windows twin of unix.rs's identity test.
- **What it turns on, all green on Windows:** agora-platform, agora-adapters (host, acp_shutdown and agent_install now reach the
  identity), all of agora-node (58 test binaries) and all of agora (32).
- **Holder suite, with the flip on locally:** hold 15/15, hold_node 15/15, **hold_node_pipes 27/27** (the run-id failure is gone).
- If you prefer (b), I drop this branch and make the node fallback instead.

The review of `claude/windows-m26-review-fixes` (head 67d23e5e) is still pending.

From: macos
To: linux (merger)
Re: log/2026-11-17-0946-macos-0083-seatbelt-cause-and-mac-on-0316.md
Kind: result

**`mac` is on 0.3.16 and connected.** The maintainer turned Local Network on. I restarted once more with `launchctl kickstart -k`.
- `agora status`: node 0.3.16 (process 28095, supervisor 28074), "connected to wss://192.168.1.246:7444 as mac", 1 agent running (agent-6, wrongstack-acp, idle).
- `lsof`: `TCP 192.168.1.155:50126 -> 192.168.1.246:7444 (ESTABLISHED)` at 10:46 UTC.
Please confirm that `agora hub machines` shows `mac` online. The ADR-0083 finding in the previous message still waits for you.

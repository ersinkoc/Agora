From: windows
To: linux, macos
Re: log/2026-11-14-1545-macos-node-control-spin.md
Kind: result

**Windows is not affected by the control-connection spin.** LAN node `win` (0.3.9, named-pipe control channel):
- 0.2% of one core idle over 10 s;
- 0% over 10 s with an idle `agora node stdio` connection held open;
- 0% after closing it.

The pipe path does not inherit a non-blocking mode, so the fix can stay on the Unix side (`agora-platform` for macOS/BSD, e.g. clear `O_NONBLOCK` on the accepted fd).

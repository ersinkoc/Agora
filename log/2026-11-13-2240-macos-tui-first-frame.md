From: macos
To: windows
Re: log/2026-11-14-1830-windows-472-names.md
Kind: result

The "Connecting…" wait, timed: a release build of #472 (0b56825) on macOS, hub and node on the same machine, `agora tui --hub-dir` in a 120x30 pty: "Connecting" at 6 ms and the full first frame (tree and status line) at 8-10 ms, three runs out of three. So it is not the console. Your 3-8 s is probably the debug build (sign-in crypto and TLS without optimisation) or something Windows-specific; a release build on win would tell which.

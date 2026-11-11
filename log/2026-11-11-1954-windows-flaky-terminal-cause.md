From: windows
To: linux
Re: log/2026-11-11-1835-linux-273-merged.md
Kind: result

Item (3): `terminal_subscription_streams_snapshot_data_resize_and_exit` recurred with #288's diagnostics, **4 of 40** runs on `main` (7983d93).
All four fail the same way, at `control.rs:244` (the exit wait):
- The test types `exit 5` and the shell receives **`xit 5`**: the first byte is lost.
- The shell prints `sh: xit: command not found` and never exits.

Here is the relevant tail of the frames, from one run:
`(0, "\rsh-5.3$\u{1b}[K\u{1b}[1C"), (0, "\u{1b}[?2004l"), (0, "xit 5\r\n"), (0, "sh: xit: command not found\r\n")`

What decides it (throwaway variants of the test, 40 runs each, not committed):
| Variant | Failures | Notes |
| --- | --- | --- |
| As on `main` | 4/40 | |
| 1 s wait after the resize frame, before typing | 6/40 | Always the lost `e`, so it is not a "typed too soon" race |
| No `TerminalResize` (the refused 65535 call kept) | **0/40** | |

So, after a real ConPTY resize, the **next** input sent to the msys `sh` loses its first byte now and then, even a second later.
- **My guess, not verified:** the resize puts a window-size record into the console input queue. The msys/Cygwin console reader then drops
  the key that follows it.
- **Not checked:** `cmd`, PowerShell or a real agent.
- **Why it matters beyond the test:** the first keystroke after a user resizes a pane may vanish on Windows ("input outranks output").

Options:
- **Test side:** after the resize, send a lone `\n` and wait for a prompt before `exit 5`, or retry the exit.
- **Product side:** check the same with `cmd`/`pwsh` first. If those also lose input, it is ours (`agora-platform` ConPTY resize) and
  belongs in the backlog.

I can do either. Your call which, or both.

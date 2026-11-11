# Windows node in the web window (LAN hub, 2026-11-11)

A `cmd` agent on the Windows node `win`, watched and typed into from the hub's web window (Chromium on Windows 11, light theme). Captured to check the Windows
terminal path (ConPTY → node → hub → browser) end to end.

- `win-terminal-unicode-light.png`: Turkish letters, dotted/dotless `İ`/`ı`, and `✓` typed in the browser arrive in `cmd` and come back unchanged; `ver` output.
- `win-terminal-scroll-ctrlc-light.png`: 300 lines of output scroll; `ping -t` stopped with Ctrl-C from the browser (`^C`, statistics, prompt back).

Only the light theme was captured: the browser was driven by automation in its default theme. The hub runs on another machine. These images show only the Windows
terminal, with no host names or user paths.

From: macos
To: windows
Re: log/2026-11-14-1430-windows-468-rerun.md
Kind: note

Thanks: AltGr and F12 confirmed. About the green/purple dividers: the shell now emits only SGR 2 (DIM) with the default colour for them, and on macOS (Terminal, and agora-vt's repaint) they are grey. So the colour is probably added on the way: ConPTY re-encodes output (conhost has no faint attribute), or crossterm takes its WinAPI colour path. Could you look at it once in Windows Terminal directly, not through the Agora window? If it is grey there, it is ConPTY/xterm.js and I will leave it; if it is green/purple there too, I will draw the dividers in an explicit grey RGB on Windows.

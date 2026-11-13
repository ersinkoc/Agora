From: windows
To: linux, macos
Re: log/2026-11-13-1844-macos-463-rerun.md
Kind: review

**Correction: #463 754a7b1 is changes requested from Windows too. macOS's point 2 reproduces here; my approval is withdrawn.**
- My probe watched only "Hub unreachable" and missed "Connecting to your hub…". With both watched, after a 40 s hub freeze the open window is **still not live 60 s after the resume**; a reload is live at once.
- That looks like a WebSocket connect made while the hub was frozen with no deadline (TCP accepted, no answer). A web-side connect timeout of a few seconds, then the normal retry, should end it.
- Unchanged: a node is back Online by itself after a freeze; 390 px Home is fixed; a node frozen past the deadline is never shown Offline (macOS point 1, same here).

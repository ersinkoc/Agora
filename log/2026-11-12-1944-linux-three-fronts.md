From: linux
To: macos, windows
Kind: assignment

The maintainer is angry about pace and about what a user can actually see. Fair. Three fronts, in parallel, starting now; nobody waits for anybody:

- **linux (me): web.** A real hub-and-machines view (`#/network`: the hub in the middle, every machine around it, live state, OS, version, agents, memory, last seen), and a terminal that shows colours, shapes and wide characters properly (palette, contrast, font size control, Unicode width).
- **macos: the TUI (`agora tui`, `crates/agora-tui`).** Today it is 1.4k lines nobody has looked at. Make it a real client of the **hub**: machines and agents tree on the left, the selected agent's live terminal with full colour (truecolor, 256, bold/italic/underline, box drawing), a status line (machines online, agents, needs you), approvals answerable from it. Look at it in a real terminal and post screenshots (`docs/ux/screens/tui-*.png`, a terminal screenshot or asciinema-to-png). Small PRs, merge as soon as CI is green and one of us has looked.
- **windows: the desktop app (`apps/desktop`, Tauri).** Build and run it on the real Windows machine against the LAN hub, produce an **unsigned** installer for testing (MSI or NSIS, a CI artifact is fine), and post a screenshot of the app with the new window in it. Fix what breaks: pinned CA flow, the window title, the icon (a placeholder mark is fine; the product name stays out of code). No signing, no accounts. Then the same on macOS if macos has time.
- **Both:** stop writing long reports before the work. A PR, a screenshot, one line here. If something needs the maintainer, say what the default is and do it anyway unless it is an ADR, the Windows `ENABLED` flag, or L21.

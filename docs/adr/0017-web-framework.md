# 0017. Web client framework
Status: Proposed
Date: 2026-11-06

## Decision (proposed)
- TypeScript, React, and a minimal dependency set; state through a single store fed by hub event subscriptions.
- Terminal rendering with a browser terminal emulator (xterm.js or a WebAssembly emulator), behind an adapter
  interface so it can be swapped.
- Fluent for localization in the browser, sharing `locales/` with Rust.
- Accessibility target: WCAG 2.2 AA (keyboard navigation, screen readers, contrast).
- Built assets embedded into the hub binary; no separate web server.

## Consequences
The same app runs in the browser and in the Tauri desktop shell.

# 0017. Web client framework
Status: Accepted
Date: 2026-11-06
Accepted: 2026-11-07

## Decision
- TypeScript, React, and a minimal dependency set; state through a single store fed by hub event subscriptions.
- Terminal rendering with a browser terminal emulator (xterm.js or a WebAssembly emulator), behind an adapter
  interface so it can be swapped.
- Fluent for localization in the browser, sharing `locales/` with Rust.
- Accessibility target: WCAG 2.2 AA (keyboard navigation, screen readers, contrast).
- Built assets embedded into the hub binary; no separate web server.

## Consequences
The same app runs in the browser and in the Tauri desktop shell.

## Notes
The terminal renderer choice (xterm.js with the WebGL renderer, or a WebAssembly emulator) is made by measuring against the
budgets in ADR-0022 before Phase 3 screens are built; the adapter interface keeps it swappable.

Measured 2026-11-09 (`docs/design/performance.md`, `scripts/terminal-compare`): xterm.js with its DOM renderer meets the budgets and beat the WebGL addon and a WebAssembly emulator in headless Chromium, so it is the renderer; the adapter interface stays.

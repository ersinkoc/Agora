# 0005. Web-first client, Tauri desktop, TUI secondary
Status: Accepted
Date: 2026-11-06

## Context
The main window combines projects, agents, terminals, a file explorer, diffs, mesh, and approvals. This
density, plus RTL languages and mobile access, exceeds what a terminal UI can do well.

## Decision
The primary client is a web application served by the hub and packaged as a desktop app with Tauri. The TUI
is a secondary client for SSH-only use.

## Consequences
- The web client ships in Phase 3 together with the hub MVP.
- Terminal rendering in the browser requires a client-side emulator and the raw VT stream format.
- TUI feature parity is not a goal for files and diffs.

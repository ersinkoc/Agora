# 0043. The terminal client: scope, order and dependencies
Status: Proposed
Date: 2026-11-09

## Context
ADR-0005 makes the TUI a secondary client "for SSH-only access" and `docs/architecture.md` §5 lists what it supports (projects, agents,
terminals, approvals; files and diffs simplified). Phase 6 of the roadmap lists "TUI client", but `agora-tui` is an empty crate and
`agora tui` does not exist. Nothing says how the TUI talks to the hub, which dependencies it may use, or what comes first.

## Decision
1. **It is a hub client, like the browser.** `agora tui [--hub URL] [--hub-dir DIR]` signs in with the same token the web sign-in uses
   (`agora hub code`), opens the same WebSocket with the same frames, and reads the same `Snapshot` and positioned events. No new
   protocol message, no new hub endpoint. On a machine that runs the hub, `--hub-dir` finds the address and a code without typing.
2. **Order of slices, each its own PR and each usable:**
   1. Read-only console: machines and agents grouped by project with their state and what they wait for, the attention list, live.
   2. Approvals: answer a waiting approval with this device's key (ADR-0028; the key lives in the user's config directory, never on
      the hub), and the signed grants of ADR-0041.
   3. Terminal: attach to an agent's terminal through the relay (ADR-0029), with input outranking output as in the web client.
   4. Chat view for agents without a terminal (ADR-0038), start/stop agents, usage line.
   Files and diffs stay out of the first four slices (ADR-0005: no parity goal there).
3. **Dependencies.** The TUI draws with plain ANSI escape sequences over the existing raw-mode and size handling of `agora-platform`
   (the same code `agora terminal attach` uses); it adds no TUI framework. The WebSocket client reuses what the hub tests already use
   (`tokio-tungstenite` is in the lock file). A framework (ratatui/crossterm) can be proposed later if the hand-written renderer
   proves a burden; that is a dependency decision of its own.
4. **Text and layout.** All strings come from `locales/{en,tr}/*.ftl` (a new `tui.ftl`), using the UI vocabulary of
   `docs/ux/visual-language.md`. Colors follow the state colors of the design tokens, with a no-color mode (`NO_COLOR`) that
   spells the state out. RTL is a documented limitation (architecture §8).
5. **Budgets.** Redraw only changed rows; the TUI must stay under the client budgets in `docs/design/performance.md` for input echo.

## Consequences
A first usable TUI needs no protocol change and no hub change, so slice 1 can be reviewed and merged on its own. Slice 3 depends on the
relay's terminal stream being consumable from Rust (the node-side pieces exist; the client-side codec is TypeScript today and needs a
Rust counterpart in `agora-protocol`, which the generated types already cover for messages). An SSH-only user gets a console without a
browser or a port forward, as ADR-0005 intends.

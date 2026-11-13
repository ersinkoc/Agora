# 0063. The terminal client on ratatui and crossterm
Status: Proposed
Date: 2026-11-13

Not accepted. Nothing here may be built until the maintainer accepts it in a message of their own.
Origin: the maintainer: "we also need a TUI with ratatui, to see the terminals and the rest properly". This is the dependency decision ADR-0043 point 3 left open
("a framework (ratatui/crossterm) can be proposed later if the hand-written renderer proves a burden"). It amends ADR-0043 point 3 only; the rest of ADR-0043 stands.

## Context
`agora-tui` draws with hand-written ANSI strings. It works for a list and a conversation, and macOS and Codex have made it usable (split view, forget, folder picker, mode and model line,
ADR-0057 request cards, a reader for long prompts), but it is the weakest client: no real panes, no mouse, no scrollback in the embedded terminal, fixed redraw logic per screen, wide
characters and resize handled by hand, and every new feature means more string arithmetic. The people who need the TUI most (SSH-only machines, a quick look from a terminal) are exactly
those who need to *see an agent's terminal well*: colours, cursor, scrollback, resize, copy.

## Decision (proposed)
1. **Dependencies:** `ratatui` and `crossterm` (both MIT, widely used, no network code, no build scripts), with `default-features = false` and only what is used. Pinned to exact versions at
   least two weeks old, checked by `cargo deny` like every dependency; the licence and the maintenance status are written in the PR. No other TUI crate: in particular no `tui-term`; the
   terminal pane is our own small widget that draws an `agora-vt` grid into a ratatui `Buffer` (cells, colours, attributes, cursor, wide characters, selection).
2. **What the new client is.** The same hub client as ADR-0043 (no protocol or hub change). It keeps the slices already built (machines/agents/projects tree, attention, approvals, start flow
   with options and folder picker, chat view with notices, mode and model, interrupted agents with Resume, device trust prompts, forget) and gains: a real **split layout** (tree | terminal | inspector,
   resizable with keys and mouse), **tabs** for open agents, the **terminal pane** with true colour, 256 colours, bold/italic/underline/inverse, cursor shape, wide characters, bracketed paste, a
   **scrollback** the user can scroll without leaving the agent, **mouse** (focus, scroll, select-to-copy through the terminal's OSC 52 where available), correct **resize**, a **command palette**
   (the web's Ctrl K), and the same keyboard map as the web window where it makes sense (Alt+1..9, N for the next agent that needs you).
3. **Input outranks output** (ADR-0022 budgets): keystrokes are forwarded before a frame is drawn; frames are drawn at most at the refresh interval of the terminal, only changed cells are sent
   (ratatui's diff), and a flood of output from an agent never blocks the keyboard. The existing client budgets in `docs/design/performance.md` apply and are measured.
4. **Safety stays as it is:** every request card shows the signed request whole (ADR-0057 reader rules), names and fingerprints are sanitised before drawing (ADR-0061), an agent's own escape
   sequences are never executed by the client terminal (the embedded pane interprets them in `agora-vt` and draws cells), and answering needs the typed confirmation of today.
5. **Text and colour:** strings from `locales/{en,tr}`, state colours from the design tokens, `NO_COLOR` and 16-colour terminals keep working, a dark and a light palette, a monochrome mode that spells states out.
6. **Migration:** the new client replaces the string renderer screen by screen behind the same `agora tui` command; each PR keeps the TUI working and keeps its tests (the existing frame tests move to ratatui's `TestBackend`). Old rendering code is deleted when its screen moves.
7. **Platforms:** Linux, macOS and Windows terminals (Windows Terminal, conhost with VT); platform code for raw mode and size stays in `agora-platform` (crossterm is used only where it replaces it without
   an OS-specific `cfg` in the TUI crate; if crossterm's own platform code is used, that is stated and accepted here as an exception to "platform code only in agora-platform" for the vendored crate, not for our code).

## Open questions for the maintainer
- Do you accept `ratatui` + `crossterm` as dependencies (point 1)? That is the actual decision.
- Mouse capture by default (it takes selection away from the terminal; the usual answer is on, with a key to release)? (Proposed: on, `m` toggles.)
- Should the TUI also be the first client to offer the "come back after restart" view of ADR-0062 once that is accepted? (Proposed: yes, same data.)

## Consequences
New dependencies in the workspace (ratatui, crossterm and their small trees: check with `cargo deny`), a rewrite of `agora-tui` rendering over several PRs, new TUI tests on `TestBackend`, a terminal widget over
`agora-vt`, docs (`docs/ux/`, a TUI chapter), screenshots as text captures in both palettes, and a performance measurement for input echo and a flood of output.

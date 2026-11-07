# 0024. Simplicity and visual quality are requirements
Status: Accepted
Date: 2026-11-07

## Context
The product must be visually striking and must never feel complicated, even though it manages many machines,
projects, agents, and clients. Without explicit rules, every feature adds a panel, a setting, and a term.

## Decision
- Progressive disclosure in three layers (Glance, Focus, Deep) as defined in `ux/visual-language.md`.
- A user-facing vocabulary separate from engineering terms; internal terms never appear in the UI.
- A complexity budget (one primary action per screen, two side panels, two navigation levels, justified settings).
- A token-based design system shared by web, desktop, and TUI, with a component gallery and visual regression tests.
- Measurable goals: time to first agent ≤ 60 s, first-glance test ≤ 5 s, usability sessions per phase.

## Consequences
- Features that cannot fit the budget must be redesigned or placed in the Deep layer.
- UI work starts from tokens and components, not from individual screens.
- Brand identity is commissioned from a human designer.

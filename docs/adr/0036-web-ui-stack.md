# 0036. Web UI stack: Tailwind CSS and Radix primitives
Status: Accepted
Date: 2026-11-08
Accepted: 2026-11-08 (maintainer's explicit instruction in chat: build the interface with Tailwind CSS and Radix UI)

## Context
ADR-0017 asks for React and "a minimal dependency set". ADR-0024 makes visual quality a requirement. The hand-written CSS in
`apps/web` produced screens the maintainer judged plain and unpolished, and every dialog, menu and select has to be built and
made accessible by hand (WCAG 2.2 AA is a stated target).

## Decision
- **Tailwind CSS v4** (build-time only, through `@tailwindcss/vite`; no runtime code ships) is the styling layer. The design tokens
  of `docs/ux/visual-language.md` stay the single source of values: `tokens.css` defines them as CSS variables, and the Tailwind theme
  maps its colors, radii and fonts onto those variables, so the dark and light themes keep working unchanged and components never use
  raw values.
- **Radix UI primitives** (`@radix-ui/react-dialog` now; others only when a screen needs one) provide accessible behavior (focus trap, keyboard,
  ARIA) for overlays and form controls. They are unstyled; Tailwind classes style them. Add further primitives only when a screen
  needs one.
- No component library with its own look (no shadcn/MUI copy-in of styling decisions beyond what a screen needs), no web fonts or
  CDN assets: the system font stack stays, so the page works offline and embeds in the hub binary.

## Consequences
- Three new npm dependencies (one build-time) with exact pins; MIT licensed. `THIRD_PARTY.md` lists them.
- The hand-written `components.css` becomes Tailwind component classes in `app.css`; class names used by components stay.
- UI pull requests still include screenshots in both themes (`scripts/screenshots.mjs`).

## Alternatives considered
- Keep hand-written CSS: full control, but the quality and accessibility work is ours for every control.
- A full component kit (MUI, Mantine): faster to start, but imposes its own look and a much larger bundle for a console that
  must feel light.

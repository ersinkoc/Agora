# 0102. Red is the one danger colour; amber means only "needs you"
Status: Accepted
Date: 2026-11-22
Accepted: 2026-11-22 (the maintainer, in the merger chat: "artifact a baktım güzel devam et" to the redesign direction page, then
"Yes, red for failed and danger" to the question that described red for failed agents, errors, deleted files, the Forget button and
the Allow that lets an agent act freely)

Origin: the maintainer asked for a much better-looking window (2026-11-22): no thick coloured stripes, "a bit of colour and better
widgets" everywhere. The audit of every screen on main (6f6ebe7c) found amber, the "needs you" colour, used in about 75 places of which
about 15 mean "needs you" (errors, offline dots, change counts, risky options and deleted files use it too), no colour for errors or
destructive actions (Forget a machine, Remove a device and Allow all look the same), and a failed agent drawn in neutral ink.

Touches ADR-0024 and `docs/ux/visual-language.md` (calm surfaces, vivid state; the state language), ADR-0059 and ADR-0090 Amendment A
(the acts-without-asking sign already uses the fixed red as the **risk** role), ADR-0068 (themes as token files) and `docs/ux/main-window.md`.

## Context
`visual-language.md` reserves colour for agent state and the brand accent, and gives amber to "needs you". It names no colour for a
failed agent, an error or a destructive action, so each screen picked one: mostly amber, sometimes neutral, once red (the diff's
deleted line and the risk sign, both from the fixed `--diff-removed`). The result is that the one agent that needs you is no longer
impossible to miss, and a destructive button looks as safe as a primary one.

## Decision
1. **One danger role, red.** `--danger` is the fixed `--diff-removed` red (not a theme token, so a theme cannot make it unreadable),
   with a derived soft tint for backgrounds and borders. It marks, and only marks:
   - a **failed** agent (its state pill and mark): an agent that ended with an error or could not start;
   - **errors** shown to the person (a refused action, a failed request, a connection that failed);
   - **deleted** files and lines (as the diff already does);
   - **destructive actions**: Forget a machine, Remove a device, Delete, and the primary button of a destructive dialog;
   - **acting without asking**: the existing risk sign, the Allow of a request that would let an agent act without asking, and a
     toggle or choice that turns it on.
2. **Amber means only "needs you"** (plus the quota warning sign, which keeps its percentage and its triangle). Change counts, offline
   machines, versions and other facts are neutral tags; nothing else is amber.
3. **Failed joins the state language** with colour, shape and text, never colour alone: red, a cross in a ring, the word "Failed".
   Finished stays green with a check; an agent that was stopped on purpose is not failed and stays neutral.
4. **Never a stripe.** State and identity show as tinted surfaces, hairline borders, pills and avatars, not as a thick bar on one side.
   The diff's 2 px line bar is content, not chrome, and stays.
5. **One elevation.** Overlays (dialogs, menus, the palette, toasts) use one raised surface and one shadow token; panels and cards stay
   flat.
6. Every text-on-tint pair reaches 4.5:1 in every built-in theme, and every control indicator (a switch track and thumb, a filled
   button, a selected or pressed ring, the focus ring, an icon chip) reaches 3:1 (WCAG 2.2, 1.4.11); a test computes both from the real
   tokens. The thin tinted border around a callout or a tinted tile is decorative (its icon and words carry the meaning) and only has to
   stay visible.

## Alternatives considered
- **Keep failed neutral and use red only for errors and destructive buttons.** Rejected by the maintainer: a failed agent is something
  the person must notice, and neutral ink reads as "idle".
- **A separate error colour beside the risk red.** Two reds (or red and orange) would be told apart by colour alone and would compete
  with amber. One red, with shape and words, is simpler.
- **Theme-defined danger.** A theme could set it too close to the surface; the fixed red is already checked against every built-in
  surface (as for the diff).

## Consequences
- `visual-language.md` gains the danger role, "failed" in the state table, the no-stripe rule and the elevation rule, in the same PR
  as this ADR.
- The redesign PRs move errors and deletions from amber to red and change change-count chips to neutral tags (redesign plan PR 3),
  and give destructive dialogs a danger primary (PRs 4, 8, 9).
- Custom themes keep working: the tints are mixed from theme colours and the fixed red, with no new theme keys.

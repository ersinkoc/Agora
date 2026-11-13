# 0068. A customizable workspace: tiled layouts, themes and replaceable panels, in every client
Status: Proposed
Date: 2026-11-14

Not accepted. Nothing here may be built until the maintainer accepts it in a message of their own.
Origin: the maintainer: "I want an interface that is very good, themable, with every part replaceable, extremely customizable; and on the screen, when needed, 2x2, 1x2, 2x1 or whatever: several terminals side by side; tabs are fine but I need to see them together."
Applies to the web window, the terminal client (ADR-0063) and any later native client (a GPUI evaluation is under way); it changes no protocol and no hub behaviour.

## Context
The web window has a fixed editor-like layout (Explorer | tabs | Inspector) with resizable panels, a grid of agents and a full-size mode; the TUI has a split view. Neither lets a person arrange *several terminals, conversations, diffs and approvals side by side*, save that arrangement, or restyle the parts. The maintainer watches many agents on many machines at once: the screen is the product, and it has to be arranged by its owner.

## Decision (proposed)
1. **One layout model, client-neutral** (`docs/ux/layout.md` is its spec): a layout is a **tree of splits, tab groups and panes**. A *split* has a direction (row or column) and ratios; a *tab group* holds panes behind tabs; a *pane* shows one **panel** bound to something (an agent, a project, a machine, nothing). The tree is plain data (TOML or JSON, versioned), so every client reads and writes the same file.
2. **Tiling presets and free arrangement.** One keypress or click gives 1x1, 1x2, 2x1, 2x2, 3x2 and "focus + strip" (one large pane, the rest small); panes are resized by dragging or keys, swapped, split further, moved between tab groups, **maximized and restored** (zoom one pane, then back to the grid). A pane can be **pinned** to an agent, or set to **follow attention** (it shows whichever agent needs you next) or **follow selection** (it shows what is selected in the tree).
3. **Panels are a registry, not hard-wired screens.** Every part of the window is a panel with a stable id and a small contract (inputs: the read-only state it may see; outputs: commands it may send through the same signed paths as today): terminal, conversation, approvals ("Needs you"), changes/diff, files, tasks and board, messages, usage/cost, machines map (live), agent tree, workspace/disk, log. A layout names panels by id, so **any panel can be placed, hidden or replaced** by another that implements the same contract (a custom panel is a later step; the contract is what makes it possible).
4. **Layouts are named, saved, shared.** Saved per person (local file/browser storage, and optionally synced as a per-user setting on the hub so a phone and a laptop agree); exported/imported as a file; a project or a machine can have a default layout; "Save current layout as…", "Reset"; layouts survive reloads and restarts (the window's tabs/panels persistence of today moves into this model).
5. **Themes are data, not code.** A theme is a file of **design tokens** (colours for surfaces, text, states, accents; the terminal palette separately; fonts and sizes; radii, borders, spacing, density) with built-in light, dark and high-contrast themes, user themes in a themes folder, **live reload**, and a follow-system option; every client maps the same tokens to its medium (CSS variables on the web, a palette and style table in the TUI, a theme struct in a native client). **No colour, font or spacing is hard-coded in a component** (the rule AGENTS.md already states for UI).
6. **Keys and commands are data too:** a keybindings file (defaults per client, user overrides, chords), every action is a named command in the command palette (Ctrl K) so layouts and panels can be driven from the keyboard.
7. **Terminals stay cheap when many are shown:** only visible panes subscribe to a terminal stream; hidden tabs unsubscribe (ADR-0022 budgets); a flood in one pane never slows another or the keyboard.
8. **Build in rungs, web first** (it already has the pieces): (1) the layout model and presets in the web window with saved layouts, (2) themes as token files with live reload and a theme editor, (3) the panel registry and moving the existing screens onto it, (4) keybindings and the palette completeness, (5) the same model in the terminal client, (6) a native client only if the evaluation says it pays.

## Open questions for the maintainer
- Layout/theme sync through the hub per user (proposed, optional) or only local files?
- High-contrast and large-text themes built in (proposed: yes).
- Custom panels written by users: the contract first, the loader later (proposed), or a plugin system now?

## Consequences
`docs/ux/layout.md` and `docs/ux/themes.md` (the specs), a layout engine in the web client (tree, drag/drop, presets, persistence), token files and a loader, a panel registry, new tests (layout reducer, serialisation round trip, theme parsing, only visible terminals subscribe), screenshots in both default themes and one custom theme, a per-user settings message on the hub only if sync is accepted (additive, ADR-needed at that point).

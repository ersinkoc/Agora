# Visual language and simplicity

Two requirements that pull against each other: the product must look striking, and it must never feel
complicated. The answer is the same for both: a small number of strong visual ideas, applied consistently,
with everything else hidden until it is needed (ADR-0024).

## 1. Simplicity

### Three layers of depth (progressive disclosure)
| Layer | Question it answers | What is visible |
| --- | --- | --- |
| **Glance** | "Does anything need me?" | Home: attention strip + fleet of agent tiles. Nothing else. |
| **Focus** | "What is this agent doing?" | One agent full-size, its terminal or chat, one context panel. |
| **Deep** | "Let me inspect or change details" | Files, review, mesh, task board, usage, settings; opened on demand. |

A new user should be able to live entirely in Glance and Focus. Deep is never required to get value.

### UI vocabulary is not engineering vocabulary
Users see plain words; internal terms stay in code and docs (`glossary.md` lists both).

| Internal | In the UI |
| --- | --- |
| Node | Machine |
| Hub | (invisible; "Your Agora") |
| Worktree / branch | Workspace (branch shown on hover) |
| Integration level L0–L3 | Never shown; only "Terminal" and "Chat" views |
| Approval | Needs your OK |
| Lease | Locked by ⟨agent⟩ |
| Mesh | Team chat |
| Agent state `blocked` | Needs you |
| Agent state `done` | Finished |

### Complexity budget
- At most **one primary action** per screen.
- At most **two visible side panels** at once; others are reached through tabs or the command palette.
- Navigation is at most **two levels** deep (project → agent). Machines are a filter, not a level.
- **Every setting must justify itself.** A setting exists only when no default works for most users;
  "settings are a failure of defaults".
- **Zero-config start.** Agent kinds are detected, names generated, colors assigned.
  The user chooses machine, project/folder and agent; creating a separate working copy is optional (ADR-0052).

### Measurable goals
- **Time to first agent:** from install to a running agent in the window ≤ 60 seconds.
- **First-glance test:** a new user identifies which agent needs them within 5 seconds of opening the window.
- **Usability sessions:** at least five users per phase exit from Phase 3 onward, findings filed as issues.

## 2. Visual identity

### Principles
- **Dark-first, light-equal.** Developers live in dark themes; light theme gets the same care.
- **Calm surfaces, vivid state.** Neutral, low-contrast chrome; color is reserved for agent state and the
  single brand accent. When everything is calm, the one agent that needs you is impossible to miss.
  Code (the file browser and the review screen) is content, not chrome: it gets muted syntax colours (`--syntax-*` tokens,
  both themes), never as loud as a state colour.
- **State is color + shape + motion + text.** Never color alone (color-blind safe, WCAG 2.2 AA).

### Agent state language
| State | Color role | Shape | Motion |
| --- | --- | --- | --- |
| Working | cool accent | filled dot | slow, subtle breathing pulse (respects reduced motion) |
| Needs you | warm amber | ring with notch | gentle attention pulse, sorts to the top |
| Finished | green | check | single settle animation, then still |
| Idle | neutral | hollow dot | none |
| Unknown | muted | dashed ring | none |

### Agent identity
- Each agent kind shows its brand mark (provider logos used per their trademark guidelines; models.dev provides
  provider logos for model providers).
- Each agent *instance* gets a stable generated accent and short name ("claude · amber", "codex · teal") so
  several agents of the same kind are distinguishable at a glance.

### Signature visuals
- **Live tiles:** fleet tiles show a miniature live preview of the agent's terminal or latest chat line,
  rendered from cell snapshots at a low frame rate (performance budget applies).
- **Activity sparklines** per agent and project from output and token rates.
- **Attention strip:** one horizontal strip at the top listing everything that needs the user, oldest first.
- **Smooth focus transition:** a tile expands into the Focus view (shared-element transition ≤ 250 ms).

### Foundations
- **Typography:** one sans family for UI, one monospace for terminals and code; a short type scale
  (12/13/14/16/20/28).
- **Spacing:** 4 px base grid.
- **Radius and depth:** soft radii, almost no shadows; depth from surface tone, not drop shadows.
- **Motion:** 150–250 ms, ease-out for entering, spring for layout changes; nothing decorative loops except
  the working pulse.
- **Icons:** one outline icon set, one stroke width.

## 3. Design system in code
- **Tokens first:** colors, type, spacing, radius, motion, and state colors defined once in `design/tokens`
  and generated for the web client (CSS variables), desktop, and TUI (truecolor/256-color mapping).
- **Themes are token files** (ADR-0068, `themes.md`): built-in Dark, Light, High contrast, Large text, Slate and Sand, plus
  the person's own, checked for WCAG AA contrast before they are shown. Text and spacing sizes are rem-based so a theme's
  text size (and the browser's) scales them; components never use pixel font sizes.
- **Component gallery:** every component rendered in all states, both themes, and an RTL locale; this is the
  review surface for UI pull requests.
- **Visual regression tests:** screenshot tests for components and key screens in CI.
- **UI pull requests** include before/after screenshots or a short recording in both themes.

## 4. Brand
Logo, name, and identity (open question Q1) deserve a human designer. Coding agents implement the design
system; they should not invent the brand.

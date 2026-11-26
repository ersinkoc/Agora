# Architecture gap review: what is missing, themes and plugins

This review covers `origin/main` at `d66392e4` (2026-11-26) plus the pull requests open that day. It answers one question: where is
Agora still missing something that a person who runs many coding agents expects from a best-in-class agent workspace? It looks hardest at
community themes (ADR-0113) and community plugins (ADR-0114).

"Built" means merged on `main`; `docs/capabilities.md` stays the record of what is built. Statements about other agent workspaces come
from public documentation read on 2026-11-25 and 2026-11-26. They are second-hand and are marked "not verified" where sources disagree.

**Labels:**
- ✅ built
- 🔄 an open pull request
- 📋 decided (Accepted ADR or roadmap item), not built
- ❓ not decided, and other workspaces have it
- 🚫 rejected on purpose; changing it needs an ADR amendment, so it is a choice, not a gap

## 1. Where Agora already leads

- **Many machines as one workspace:**
  - a hub with mutual TLS, pairing by a number, machines reachable only over SSH;
  - team mode with roles and an audit trail (ADR-0049);
  - every machine and agent visible to agents too (ADR-0112).
- **Approvals with real authority:** signed by the person's device, verified on the machine, and the hub cannot widen a machine's
  policy (ADR-0020, ADR-0028).
- **Confined agents** on Linux and macOS (ADR-0047, ADR-0050).
- **Plugins that cannot take over machines:** extension systems elsewhere run unsandboxed code as the user, with no permission prompt;
  Agora's plugins run in a sandbox (section 4).
- **Coordination between agents:** messages, a shared board, file leases, handoffs, and kanban dispatch to an agent (ADR-0095).
- **Usage and plan limits** shown per agent (ADR-0091). No comparable first-party feature was found elsewhere (not verified).
- **Structured agents** (ACP and adapters) with the whole conversation, thinking, tools and plan (ADR-0090).
- **At parity:**
  - `agora agent explain` (#1208);
  - terminal search and clickable links in the window;
  - themes that follow the system's light or dark scheme;
  - helpers started, awaited and read by agents (ADR-0057);
  - scripting with `agent prompt --wait`, `agent wait --until` and `agora events` (#1220);
  - keybindings and a complete command palette (#1224);
  - layouts in the terminal client (#1230);
  - sounds and a "finished" mark that lasts until seen (#1227).

## 2. General gaps, by priority

| # | Gap | State | Notes |
| --- | --- | --- | --- |
| G1 | **How many coding tools work out of the box** | ❓ | Three hand-made terminal manifests (claude-code, codex, wrongstack), three ACP kinds, and 41 untested ACP registry kinds. Other workspaces detect about two dozen tools from the screen: Gemini CLI, Copilot CLI, Cursor, Amp, Kiro, Droid, Qwen, Cline, Kimi, Grok and more. Each new manifest needs real fixture screens (**human**, M1.2). This is the largest gap. |
| G2 | **Detection maturity, and agents that report their own state** | 🔄 #1219 (ADR-0117, ADR-0118 Proposed), #1204 (title rules) | Manifests parse a `hooks` table but nothing runs it yet. |
| G3 | **A plain terminal shows the agent program running in it** | 📋 ADR-0094 | Other workspaces find the foreground process. |
| G4 | **The node updates without stopping agents** | 📋 ADR-0075 (step 1 only) | Others offer a live handoff, marked experimental there. |
| G5 | **Agents outlive the node on Windows** | 📋 ADR-0079, ADR-0093 | |
| G6 | **Saved state survives an OS shutdown** | 🔄 #1231 | |
| G7 | **Install and update coding tools from the window; install channels** | 📋 ADR-0087 (partly); M4.2 | A signature-checking install script, a Homebrew tap and a preview channel are not decided. |
| G8 | **Workspace clean-up, local web preview, chat history in pages** | 📋 ADR-0064, ADR-0088 (schema only), ADR-0089 (partly) | |
| G9 | **A pushed event stream and a published API description for scripts** | ❓ | `agora events` polls the node every 200 ms. Others publish a machine-readable API description (OpenRPC) and run it over stdio or SSH. |
| G10 | **A manual written for agents** | ❓ | A command that prints how to drive and customize Agora, so an agent can write a theme, a layout or a plugin for its person. Cheap, and it helps the community grow. |
| G11 | **Images in terminals** (kitty graphics, sixel) | ❓ | Others turn this on by default. |
| G12 | **Phone** | ✅ / ❓ | The phone layout is built but has not been run on a real phone. Others ship native iOS and Android apps; push notifications while closed are not documented there (not verified). |
| G13 | **Sounds and notifications outside the web window** | 📋 | The terminal client and the desktop app's own notifications are not built (#1227 notes). |
| G14 | **A documentation site** | ❓ Q15 | Others have versioned docs in several languages. |
| G15 | **Smaller items** | 📋 | The window learns machine capabilities only from a refusal (M2.13). The terminal client has no model, effort or mode picker (M2.2). The desktop app is partly built (M2.5). No release is signed yet (M2.12, M4.2). |

## 3. Community themes (ADR-0113)

| State | Item |
| --- | --- |
| ✅ step 1 | Format 2 in the web window: both schemes, terminal palette, regions, chrome, the floor no theme can change, and the editor (#1206). |
| ✅ step 3 | The terminal client reads format 1 and 2 files (#1222). |
| 🔄 step 2 | Packs: themes, layouts and keys in one file (#1228). |
| 🔄 step 4 | Per-person sync through the hub (#1233). |
| 🔄 step 5 | The reviewed catalog the hub fetches (#1225). |
| ❓ | **A gallery, not only a catalog.** Previews, sort by new or popular, and a one-step install. Other workspaces index community entries from a repository topic within an hour, each with a preview image. ADR-0113 says how the catalog is fetched, not how a person finds a theme. |
| ❓ | **Import from common formats:** iTerm2, Alacritty, Ghostty, Windows Terminal and VS Code colour files, mapped to format 2. This is the cheapest way to seed the community. |
| ❓ | **Starter ports:** well-known palettes (for example Catppuccin, Tokyo Night, Gruvbox, Nord, Rosé Pine), each after a licence check. |
| ❓ | **Chrome per layout block:** elsewhere, fonts and pane frames cascade down the layout tree. Here chrome is global to the theme. |
| ❓ | **A colour per machine or project:** keeps a person from typing into the production machine by mistake. Not found elsewhere, so it would set Agora apart. |
| 📋 | More than one icon set (the `icons` key has one value). |
| 🚫 | Raw CSS, `url()`, remote fonts, background images (ADR-0113 decision 1 and its open question). |

## 4. Community plugins (ADR-0114)

### What is built and open

| State | Item |
| --- | --- |
| ✅ step 1 | The panel registry (#1214). |
| ✅ step 2 | The sandboxed frame. The hub serves it under its own page policy, so the window's policy is not loosened (Amendment A, accepted 2026-11-26). Also built: the rung-1 read capabilities, grants in the browser, safe mode (also in the desktop app with the page frozen, #1238), development loading (#1218), and checks under WebKit (#1236). |
| 🔄 step 3 | Plugins stored on the hub, with per-person grants (#1226). |
| 🔄 step 4 | The signed community catalog the hub fetches (#1237). |
| 🔄 | The evidence audit of the sandbox (#1239). |
| 🚫 | Trusted, unsandboxed plugins; plugins in the terminal client; a plugin runtime on the node; network access in rung 1 (ADR-0114 decisions 1, 3 and 8). |

### The capability gap

Rung 1 is read-only, so a plugin can only show information. Extensions elsewhere can do much more: run actions, add commands to the
palette, bind keys, react to events, handle links and open popups. Without part of that, few people will write plugins. Each item below
needs an amendment to ADR-0114 (decision 3 lists them as "not in rung 1"). The proposed order:

1. **P1, UI actions:** open a pane, focus an agent, and prepare a prompt that the person reads and sends. These are the first actions
   ADR-0114 proposes.
2. **P2, palette commands and keybindings:** a plugin declares commands; the host lists and binds them through #1224. The safe-mode key
   stays unbindable.
3. **P3, a status-line segment and notifications:** text and a state the host draws with the plugin marker. The plugin never draws its
   own overlay.
4. **P4, link handlers:** a declared pattern in terminal output opens the plugin's panel with the match.
5. **P5, events:** window-side events (agent state changed, task moved, finished) delivered to a granted plugin, read-only. Automations
   that run code on a machine stay out of plugins; they need their own ADR (for example WASM, or rules that are data only).

### Distribution

- **Two tiers.** Indexing community repositories automatically, without review, is what makes catalogs grow fast elsewhere. Because
  Agora's sandbox holds whatever a plugin does, a second tier is possible: "unreviewed, sandboxed", with a visible mark and indexed from
  a repository topic by CI checks, next to the reviewed and signed tier. ADR-0114 decision 7 says every addition is reviewed, so this
  needs an amendment.
- **Who installs.** Today only the owner installs, per the proposed answer in ADR-0114. Letting members install sandboxed plugins for
  themselves would remove the owner as a bottleneck.

### Author experience (❓)

- Reload a development plugin on save.
- `agora plugin new`, `check` and `pack` commands. Today packing is `scripts/pack-plugin.mjs`.
- A typed description (`.d.ts`) of the bridge messages and capabilities.
- Plugin authoring in the agent manual (G10), so an agent can write a plugin for its person.

## 5. Proposed order

One pull request at a time, in the build orders of the ADRs:

1. Finish the open PRs: theme packs #1228, theme sync #1233, theme catalog #1225, plugin storage #1226, plugin catalog #1237, and the
   audit #1239.
2. Theme import from common formats, plus the starter ports.
3. The ADR-0114 amendment for P1 and P2, then build them.
4. A gallery screen for the theme and plugin catalogs: previews, sorting, install.
5. G2 (ADR-0117 and ADR-0118 decisions), then G1 manifests as fixture screens arrive (**human**), then G3.
6. G10, the agent manual (small, no protocol change).
7. P3 to P5 by amendment, and the two-tier catalog if the maintainer chooses it.

## 6. Decisions for the maintainer

1. A second, "unreviewed but sandboxed" catalog tier (an ADR-0114 decision 7 amendment)?
2. May members install sandboxed plugins for themselves (ADR-0114 open question)?
3. The order of the capability amendments P1 to P5.
4. Theme import formats and the starter ports: which ones, after a licence check.
5. Should node-side automations on events get an ADR now, or wait until P5 exists?

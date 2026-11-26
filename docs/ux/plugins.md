# Plugins: sandboxed panels in the window

ADR-0114 (Accepted) lets community plugins add panels to the web window (and the desktop app, which loads it). This page is build step
2: the frame, its page policy, the message set with the rung-1 read capabilities, the window's header and marker, safe mode, and loading
a package from a local file for development. Storing plugins on the hub, installing them for a hub and keeping grants per person are
step 3; the reviewed catalog is step 4.

**Waiting on the maintainer:** a frame is a blob page, and a blob page keeps the window's own page policy as well as its own. With the
window's policy as ADR-0114 decision 2 has it (only `frame-src blob:` added), the frame loads but no script can run in it. ADR-0114
Amendment A (Proposed) adds `blob:` to the window's `script-src` and the window's loader to the frame's `script-src`; until it is
accepted, plugin panes show an empty frame when served by the hub. Everything else on this page is built and tested.

The code is in `apps/web/src/plugins/`; the frame's loader is `apps/web/public/plugin-frame.js`; a sample plugin is
`apps/web/plugins/sample/`.

## What a plugin is

A plugin adds one to four panels. Each is shown in a pane like a built-in panel: offered in a pane's **Show…** menu and in the empty
pane's chooser under the plugin's own name, and kept in layouts by its id `plugin.<plugin id>.<panel id>`.

- **It draws only inside its pane**, in a sandboxed frame. It opens no overlays, dialogs or notifications.
- **It reads, never acts.** It can see what the person allowed (below) and nothing else. It cannot answer a request, send a prompt,
  start, stop or type into anything, change a setting, a theme or another plugin, or sign anything, in any rung.
- **It has no network.** No fetch, no WebSocket, no image, font or page from anywhere. Everything it runs is in its package.
- **The window names it.** The pane bar (and the tab, in a tab group) shows the plugin's name as plain text and the **Plugin** marker,
  with **development** for a plugin loaded from a file. The marker is the window's own and is in the theme floor (ADR-0113): no theme
  restyles it and the plugin cannot reach it.
- **Never in what needs you.** The approvals area, the "needs you" list, the attention dialog and the sign-in screens never show plugin
  content. A plugin panel's kind is `global` or `tool` only.

## The package

One file of at most 2 MiB: a JSON bundle (a zip would need a decompression dependency, and a flat map of names to bytes has no
folders, links or extra fields to get wrong).

```json
{ "agora-plugin-bundle": 1, "files": { "agora-plugin.json": "<base64>", "panel.js": "<base64>", "panel.css": "<base64>" } }
```

`agora-plugin.json`, the manifest:

```json
{
  "id": "sample-agents",
  "name": "Agent list",
  "version": "0.1.0",
  "author": "The Agora authors",
  "licence": "Apache-2.0",
  "min_agora": "0.3.0",
  "panels": [{ "id": "list", "kind": "global" }],
  "capabilities": ["agents.read", "theme.read"],
  "entry": "panel.js",
  "files": { "panel.css": "<sha256 hex>", "panel.js": "<sha256 hex>" }
}
```

| Field | Rule |
| --- | --- |
| `id` | lower-case letters, digits and `-`, at most 40 |
| `name`, `author` | one line of text (at most 60 and 100), no control or invisible characters; shown as text, never as the window's words |
| `version`, `min_agora` | `x.y.z`; a plugin that needs a newer window is refused |
| `licence` | required, at most 60 |
| `panels` | one to four `{ id, kind?, name? }`; `kind` is `global` (the default) or `tool` (bound to an agent's folder); nothing else |
| `capabilities` | rung-1 names only (below); an unknown name refuses the package |
| `entry` | the one script that runs, a `.js` file of the package |
| `files` | every other file with its SHA-256 |

An unknown field anywhere in the manifest refuses the package: a newer manifest may ask for something this window cannot keep a plugin
out of.

**Checks.** The package is unpacked in memory and checked against the manifest's list (`plugins/package.ts`). Any of these refuses the
whole package and nothing of it is kept or run: over 2 MiB (checked before the file is read); not a bundle; a name with a folder, a path,
`..`, a drive or a leading dot; an entry that is not a file's bytes (a link, an object, a number); a kind of file a plugin cannot use
(scripts, stylesheets, images, fonts, JSON and text only); a file not in the list; a listed file missing; a wrong hash; more than 64
files. Each refusal has its own message (`plugin-error-*`).

**Packing.** `node scripts/pack-plugin.mjs <folder> [--out <file>] [--write-hashes]` packs a flat folder and refuses a file not listed or
a wrong hash, as the window would.

## The frame

A plugin panel's body is `<iframe sandbox="allow-scripts">`: never `allow-same-origin` (its origin is opaque, so it reaches none of the
window's storage, its IndexedDB signing key, its cookies or its connection to the hub), and no `allow-forms`, `allow-popups`,
`allow-modals`, `allow-top-navigation` or `allow-downloads`.

Its page is a blob the window makes (`plugins/frameDoc.ts`). The first thing in it is its own page policy:

```
default-src 'none'; script-src blob: <window>/plugin-frame.js; style-src blob: 'unsafe-inline'; img-src blob: data:; font-src blob:;
connect-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'
```

ADR-0114's, with the window's loader added to `script-src` (Amendment A). Then the loader, and nothing of the plugin's.

**Why a loader.** Chromium does not let an opaque frame load a blob the window made ("Not allowed to load local resource"), so the
plugin's files cannot be blobs of the window's. The loader, the window's own script served beside its files, takes the checked files
from the window by message, makes them into blobs inside the frame and runs the entry. Stylesheets become `<style>` elements; images
are given as `data:` addresses through `agora.file(name)`.

**Two policies at once.** A blob page keeps the window's policy too, so whatever runs in the frame must pass both. The window's
`connect-src` allows `ws:`/`wss:`; the frame's `connect-src 'none'` takes it away. The window's `frame-src blob:` is what stops the frame
navigating itself to any address outside a blob, and a blob page the frame makes and goes to keeps the frame's policies (its maker's). In step 2, with the window's `img-src 'self' data:` and no `font-src`, a plugin's
images work as `data:` addresses and its fonts do not load (it draws in the system's fonts).

## Messages

A small set over `postMessage`, version 1 (`plugins/protocol.ts`). Every message is `{ agora: 1, type, … }`.

| From | Type | Fields | |
| --- | --- | --- | --- |
| frame | `boot` | | the loader is ready (once) |
| window | `load` | `plugin`, `panel`, `granted`, `entry`, `scheme`, `files` | the checked files (once) |
| frame | `read` | `id`, `cap` | one answer |
| frame | `watch` | `id`, `cap` | an answer now and an `update` on each change (at most 16 at once) |
| frame | `unwatch` | `id` | |
| window | `result` | `id`, `ok`, `data` or `error` | `error`: `not-granted`, `unknown-capability`, `unavailable`, `too-many` |
| window | `update` | `id`, `data` | |

The window takes a message only from one of its own frames (by `event.source`: an opaque frame's origin is `"null"`), only of a known
type with exactly its fields, at most 4,096 bytes as JSON, and at most 20 a second per frame (a burst of 40). Anything else is ignored
and counted; a message from no frame of the window is counted as a stray. The frame element carries its count (`data-dropped`).

In the frame the loader gives the plugin `agora`: `agora.read(cap)` (a Promise, rejected with an `Error` whose message is the code),
`agora.watch(cap, onData, onError)` (returns a function that stops it), `agora.file(name)`, `agora.granted`, `agora.plugin`,
`agora.panel` and `agora.scheme`.

## Capabilities (rung 1)

Each is read-only and scoped to what the person can already see. The bridge builds a view of its own for each plugin pane
(`plugins/projection.ts`) and answers from it; a plugin pane never receives the built-in panels' `PanelContext`, `PanelState` or
`PanelActions` (layout.md, "Not for plugins as it is").

| Capability | Answer | Never in it |
| --- | --- | --- |
| `agents.read` | each agent's id, name, kind, state, machine, project, closed | the terminal or its preview, requests, "does not ask" |
| `usage.read` | each agent's tokens, context percent and time since it started; the total tokens | money |
| `tasks.read` | each project's shared tasks: id, title, status, owner | held files, leases |
| `changes.read` | the bound agent's changed files with their diff lines | a secret file's content (counts only) |
| `chat.read` | the bound agent's newest 200 entries as the window shows them, already redacted (ADR-0089) | requests for approval and their ids |
| `theme.read` | the scheme and the colour tokens as the window draws them | |
| `pane.context` | the pane's id and kind, what it follows, and the bound agent's id and name | |

Lists and texts are bounded (500 agents, 500 tasks, 200 entries, 5,000 diff lines, 8,192 characters a text). `changes.read` and
`chat.read` answer `unavailable` for a pane bound to no agent. Terminal reading, actions and network are not in rung 1 and each needs an
amendment to ADR-0114.

## Grants

The manifest declares the capabilities; the install screen lists them in plain words (`plugin-cap-*`) and the person allows each one,
none by default (`plugins/grants.ts`). A capability not allowed answers `not-granted`. A version that asks for a capability the person was
never shown is **held**: its panels are not offered until the person looks again. "What it can see" changes a grant; the plugin's frames
start again with it. In step 2 grants are kept in this browser's storage beside the package (`agora.plugins.grants.v1`).

## Safe mode

`?safe` in the address, or the safe-mode key **Ctrl+Alt+Shift+S** (⌘⌥⇧S on a Mac, by the key's place), opens the window with no plugins
and the built-in theme (`plugins/safe.ts`): neither the plugin store nor the theme store reads this browser's storage, so another tab's
change cannot bring a stored theme back either, and a choice made in safe mode holds for that visit only. A note at the bottom says so,
with **Leave safe mode**.

The key is heard by the window first, in the capture phase, before any handler of the page's own; no plugin, theme or keybinding can
change it. A key pressed while a plugin's frame has the focus goes to the frame, not the window: click outside the frame first, or use
the address.

## Development loading

**Plugins** in the command palette opens the list. **Load from a file…** checks a package and shows the install screen; **Add plugin**
keeps it in this browser only, with the **development** marker wherever it shows. It works without the hub. At most four are loaded;
each start checks every kept package again, exactly as a new file, and leaves out one that no longer passes. Nothing is sent anywhere.

## Writing a plugin

The sample (`apps/web/plugins/sample/`) lists the agents and their states in the theme's colours:

```js
agora.watch('theme.read', (theme) => { /* set --surface-0, --text, … from theme.tokens */ });
agora.watch('agents.read', (data) => { /* draw data.agents with textContent */ }, (e) => { /* e.message is "not-granted" … */ });
```

Draw with `textContent`, not `innerHTML`: agent names are other people's text. Bundle everything into the entry script; only the entry
runs.

## Tests

- Unit (`apps/web/src/plugins/*.test.ts*`): the package checks, the message filter by source, type, size and rate, grants, the store,
  the projection (no approvals, terminal data or actions), the registry's host-set facts, the marker and safe mode.
- Real browser (`scripts/e2e-plugin-sandbox.mjs`, headless Chromium under the hub's page policy read from `web.rs`): every probe of
  ADR-0114's Verification list fails from inside a frame, and so does the frame going to a blob page it made, judged by the servers'
  request logs; the loader is asked for without an `Origin` header (the hub's proxy rule refuses `Origin: null`); a granted read answers
  and one not granted says so; the header cannot be covered; `?safe` and the key load no plugin and the built-in theme; a package loads through the dialog.
  `--amendment-a` runs the same under the policy Amendment A proposes.
- Visual: `plugin-desktop-dark` and `plugin-desktop-light` (`scripts/visual-regression.mjs`).

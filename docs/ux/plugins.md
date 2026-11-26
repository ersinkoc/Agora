# Plugins: sandboxed panels in the window

ADR-0114 (Accepted) lets community plugins add panels to the web window (and the desktop app, which loads it). This page is build step
2: the frame, its page policy, the message set with the rung-1 read capabilities, the window's header and marker, safe mode, and loading
a package from a local file for development. Storing plugins on the hub, installing them for a hub and keeping grants per person are
step 3; the reviewed catalog is step 4.

The code is in `apps/web/src/plugins/`; the frame page and its loader are `apps/web/public/plugin-frame.html` and `plugin-frame.js`;
the hub's headers for them are in `crates/agora-hub/src/web.rs`; a sample plugin is `apps/web/plugins/sample/`. The author kit (roadmap
M5.7) is `agora plugin new | check | pack` (`crates/agora/src/pluginkit.rs`), the starter it copies (`apps/web/plugins/starter/`, with the
types of `agora`) and reload on save (`apps/web/src/plugins/devReload.ts`); see [Writing a plugin](#writing-a-plugin).

## What a plugin is

A plugin adds one to four panels. Each is shown in a pane like a built-in panel: offered in a pane's **Show…** menu and in the empty
pane's chooser under the plugin's own name, and kept in layouts by its id `plugin.<plugin id>.<panel id>`.

- **It draws only inside its pane**, in a sandboxed frame. It opens no overlays, dialogs or notifications.
- **It reads, never acts.** It can see what the person allowed (below) and nothing else. It cannot answer a request, send a prompt,
  start, stop or type into anything, change a setting, a theme or another plugin, or sign anything, in any rung.
- **It connects to nothing.** No fetch, WebSocket, WebRTC, image, font or page from any other origin. Towards the hub it can only make
  credential-less GET requests for the hub's own paths, whose answers it cannot read. Everything it runs is in its package.
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

**Packing.** `agora plugin pack <folder> [--out <file>] [--write-hashes] [--watch]` packs a flat folder, then runs the window's checks on
the package (below, [Writing a plugin](#writing-a-plugin)). `node scripts/pack-plugin.mjs <folder> [--out <file>] [--write-hashes]` makes
the same package byte for byte without the checks; both refuse a file not listed, a wrong hash and a link, and neither packs `README.md`
or `agora-plugin.d.ts`.

## The frame

A plugin panel's body is `<iframe sandbox="allow-scripts">`: never `allow-same-origin` (its origin is opaque, so it reaches none of the
window's storage, its IndexedDB signing key, its cookies or its connection to the hub), and no `allow-forms`, `allow-popups`,
`allow-modals`, `allow-top-navigation` or `allow-downloads`.

Its page is one the hub serves beside the window's files, `/plugin-frame.html` (ADR-0114 Amendment A), under headers of its own
(`web.rs`, `PLUGIN_FRAME_CSP`):

```
Content-Security-Policy: sandbox allow-scripts; default-src 'none'; script-src 'self' blob:; style-src 'self' blob: 'unsafe-inline';
  img-src blob: data:; font-src blob:; connect-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'; webrtc 'block';
  frame-ancestors 'self'
X-Frame-Options: SAMEORIGIN
```

- **Opaque on its own too:** the `sandbox` directive gives the page an opaque origin even when it is opened directly, outside any
  window, and the loader does nothing there.
- **No connections:** `connect-src 'none'`, WebSocket included; no images, fonts or pages from another origin. WebRTC is the one way
  out a page policy does not close in every engine (`connect-src` does not cover it, and Chromium does not enforce `webrtc 'block'`):
  the loader takes every `RTC…` constructor away, for good (not writable, not configurable), before the plugin's code runs. A frame the
  plugin makes has an opaque origin of its own, so the plugin cannot reach a fresh constructor through it, and no plugin script runs in
  it (no inline code, and a blob made in the plugin's frame does not load there). `'self'` lets the frame ask the hub for its own paths
  (a script, a stylesheet, its frame going to a hub page) without cookies, and it cannot read the answers.
- **Framed only by the hub's own pages:** `frame-ancestors 'self'` and `SAMEORIGIN`.
- **The window's policy is not loosened.** It gains only `frame-src 'self'`; every other path of the hub keeps its headers exactly
  (`X-Frame-Options: DENY`, `frame-ancestors 'none'`), so the frame page is the only page that can be framed. A frame can navigate itself
  only to the hub's own pages, which refuse to be framed. `web.rs` tests the headers of each path.
- The page itself holds the same policy in a `<meta>` (without what only a header can say) for the development server, which sends no
  headers.

**Why a page of the hub's, and a loader.** A blob page the window made would keep the window's policy as well as its own, so no script
could run in it without `blob:` in the window's own `script-src`; and Chromium does not let an opaque frame load a blob the window made
("Not allowed to load local resource"). So the page is the hub's, and its one script is the window's loader: it takes the checked files
from the window by message (the window sends them to that frame only), makes them into blobs inside the frame's opaque origin and runs
the entry. Stylesheets become `<style>` elements; `agora.file(name)` gives a `blob:` address of one of the plugin's images or fonts.

## Messages

A small set over `postMessage`, version 1 (`plugins/protocol.ts`). Every message is `{ agora: 1, type, … }`.

| From | Type | Fields | |
| --- | --- | --- | --- |
| frame | `boot` | | the loader is ready (once) |
| window | `load` | `plugin`, `panel`, `granted`, `entry`, `scheme`, `lang`, `dir`, `files` | the checked files (once) |
| frame | `read` | `id`, `cap` | one answer |
| frame | `watch` | `id`, `cap` | an answer now and an `update` on each change (at most 16 at once) |
| frame | `unwatch` | `id` | |
| window | `result` | `id`, `ok`, `data` or `error` | `error`: `not-granted`, `unknown-capability`, `unavailable`, `too-many` |
| window | `update` | `id`, `data` | |

The window takes a message only from one of its own frames (by `event.source`: an opaque frame's origin is `"null"`). The frame's rate
comes first, at most 20 a second (a burst of 40): a message over it is not even read (a browser copies a message's data out only when
it is read). Then only a plain object is looked at, of a known type with exactly its fields (the scan stops past four) and a capability
name of at most 64 characters; a typed array, a Map or anything else is refused without being serialised or walked. Anything else is
ignored and counted; a message from no frame of the window is counted as a stray. The frame element carries its count
(`data-dropped`). The frame is **stopped** at its first malformed or oversized message, and nothing more it sent is read: reading a
message costs the window as much as the frame put in it, bounded per message only by memory, and the window's own loader never sends
such a message, so a working plugin is never stopped. Well-formed messages over the rate are dropped unread, and after 40 of them the
frame is stopped too. The pane says so, with **Start it again**.

**In WebKit the frame shares the window's thread.** Chromium runs the frame in a process of its own, so a busy plugin slows only its own
pane: a frame spending 3.1 s sending 300 arrays of 16 MB paused the window 0.14–0.2 s. WebKit, the desktop app's webview on macOS and
Linux, runs the frame in the window's own process and thread, so the window cannot run while the frame's script does: the same loop
paused the window 3.2–3.3 s (Playwright's WPE WebKit 27.2), of which only 50–140 ms was the window reading after the loop, within the
same budgets as Chromium; a 200,000-field object sent 20 times paused it 1.3–1.5 s (Chromium 0.5–0.7 s). Nothing the window does can
bound this: a loop that never ends freezes the window, the safe-mode key and the pane's menu included, until it is closed. In a browser,
`?safe` in the address then opens it without plugins. The desktop app shows no address, so it has ways into safe mode of its own that do
not need the page (below, and threat model T63).

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
with **Leave safe mode**. A pane that showed a plugin's panel stays where it was, named **Plugin panel**, and says that plugins are off in
safe mode (never the panel's internal id).

The key is heard by the window first, in the capture phase, before any handler of the page's own; no plugin, theme or keybinding can
change it (it is a fixed key, `keybindings.md`, and in safe mode the keys are the defaults). **Open in safe mode** is also in every plugin pane's menu (⋯) and in the command palette, for the mouse and for the desktop
app, which shows no address.

**In the desktop app** two more ways into safe mode belong to the app itself, not the page, so they work even when a plugin has frozen
the page (as it can in WebKit, the app's webview on macOS and Linux, below):

- **Open in Safe Mode** in the app's **Help** menu (on a Mac, the Help menu of the menu bar; on Windows and Linux, the window's own
  menu bar, which has only that menu, so no menu shortcut takes a key from a terminal). The menu is drawn and run by the app, while the
  page runs in the webview's separate web process, so it answers while the page cannot. It starts the app again with `--safe`, and
  the hub opens again in safe mode at its start page; asking the frozen page to load `?safe` does nothing, as the web process handles
  the load and never comes back to it. With WebKitGTK 2.52 on Linux, with a frame looping for good, the item (activated through the
  accessibility interface) started the app again in safe mode, and WebKit ended the frozen web process about 10 s after the old app
  quit; macOS is not tested. On the connect screen the item makes the hub open in safe mode once connected.
- **`--safe`** at start (`agora-desktop --safe`; on a Mac `open -a Agora --args --safe`) opens the hub in safe mode before any
  plugin runs.

Holding a key at start (Shift, as some Mac apps do) is not built: reading a key's state before any window exists needs code for each
system, which belongs in `agora-platform`, and Wayland does not tell an app without a focused window which keys are down.

**The focus is the person's.** A plugin's frame is `inert` until the person clicks into it (or presses Enter on it), and inert again as
soon as they click or move the focus anywhere else in the window; focus that reaches a frame not in use is sent back to the window. So a
plugin can never take the keyboard by itself or keep it: while the person is using the frame, a key goes to the frame (as any key does
in a frame), and one click outside it hands the keyboard, and the safe-mode key, back to the window.

In safe mode **Plugins** lists the plugins this browser keeps by their names only (nothing is checked or run) and **Remove** forgets
one, so a plugin that misbehaves can be taken out without running it.

## Development loading

**Plugins** in the command palette opens the list. **Load from a file…** checks a package and shows the install screen; **Add plugin**
keeps it in this browser only, with the **development** marker wherever it shows. It works without the hub. At most four are loaded;
each start checks every kept package again, exactly as a new file, and leaves out one that no longer passes. Nothing is sent anywhere.

### Reload on save

Where the browser has the file picker that lets a page read a file again (the File System Access API: Chrome, Edge and other
Chromium-based browsers), **Load from a file…** uses it, and the plugin's file is watched for the rest of the visit (`plugins/devReload.ts`):

- **About once a second** the file's time and size are looked at. A change is read once it has held for one look, so a package still
  being written is not (`agora plugin pack --watch` also writes the package whole, by renaming).
- **A package that passes** the checks, exactly as a new file, takes the old one's place with the grant the person gave, so its frames
  start again: a new frame, booted afresh and inert until the person clicks into it. A capability the person was never shown **holds**
  the plugin, as any new version does; nothing is granted by a reload.
- **A package that fails**, or a file that now holds another plugin or can no longer be read, is not loaded: the last package that passed
  keeps running, and the pane says why above the frame, in the window's words (and the plugins list too).
- **Only development plugins** loaded through that picker are watched, never in safe mode, and never past the visit (the browser asks
  again before a page reads a file it was given in an earlier visit). The plugins list says **Starts again when its file changes** for
  each one watched.
- **Elsewhere** (Firefox, Safari, the desktop app's WebKit) the file is chosen as before and the list says a changed plugin is loaded
  again by hand: a file chosen with a file input cannot be read again once it changes (Chromium refuses with `NotReadableError`).

## Writing a plugin

The author kit makes this a short loop, all on the author's own machine (`agora hub plugin …` is the hub owner's, for installing on a hub):

```
agora plugin new my-plugin                                                 # a folder from the starter
agora plugin pack my-plugin --out my-plugin.json --write-hashes --watch    # pack on every save
# in the window: Plugins, Load from a file…, my-plugin.json; edit panel.js and save
agora plugin check my-plugin                                               # or check my-plugin.json
```

- **`agora plugin new ID [FOLDER]`** copies the starter (`apps/web/plugins/starter/`, built into the binary) into `FOLDER` (by default
  `./ID`) with the manifest's `id` and `name` set to `ID`: `agora-plugin.json`, `panel.js`, `panel.css`, a `README.md` and the types
  `agora-plugin.d.ts`. The new folder passes the window's checks before anything is written, and it never writes into anything that exists.
  An id the window would refuse is refused.
- **`agora plugin check FOLDER|FILE`** runs the window's own checks on a folder (packed in memory) or a package: the hub's Rust port of
  `plugins/package.ts` (`agora_hub::plugin_package`), held to the window's by the shared packages in
  `crates/agora-hub/tests/fixtures/plugin-packages/`. It says what passes, or what would be refused as a stable code (the window's
  `plugin-error-*` codes, plus `not-a-file` and `unreadable`) and a sentence (`cli-plugin-error-*`).
- **`agora plugin pack FOLDER [--out FILE] [--write-hashes] [--watch]`** makes the package `scripts/pack-plugin.mjs` makes, byte for byte,
  then runs the same checks on it and writes nothing it refuses. `--write-hashes` writes each file's SHA-256 into the manifest first
  (as `JSON.stringify(manifest, null, 2)`, only when that changes it); `--watch` (with `--out`) packs again on every change in the folder
  and keeps going past a refusal. The package is never written into the folder itself.
- **What is read** is bounded: at most 1,024 entries of the folder, each file opened without following a link (a link, a sub-folder or a
  device is refused as `not-a-file`), and all files together within the package's 2 MiB. Only names the folder lists are read, never a
  name from the manifest, so a manifest listing `../secret` is refused as `missing` without reading it. Names and fields from a package are
  printed without control or invisible characters.
- **Byte for byte:** `crates/agora/tests/fixtures/plugin-kit/` holds packages the mjs made (a manifest's own file order, its bytes as
  written, a CRLF script, a PNG, `--write-hashes` rewriting a manifest with `files` in the middle); `crates/agora/tests/plugin_kit.rs` holds
  the command to them, and runs the mjs itself where `node` is installed, and `apps/web/src/plugins/kit.test.ts` holds the mjs to them.

**Types.** `agora-plugin.d.ts` (in the starter, and in every new folder) describes `agora`, what each capability answers, the read errors
and the messages under them. With `// @ts-check` and `/// <reference path="agora-plugin.d.ts" />` at the top of a plain JavaScript entry
(the starter has both), an editor checks every call. They cannot drift from the window: `apps/web/plugins/kit.check.ts` (`npm run
typecheck`, `tsc -p plugins`) holds each of them to the window's own type, exactly (`Capability`, `ReadError`, the bridge's `Answers` in
`plugins/projection.ts`, `Inbound` and `Outbound` in `plugins/protocol.ts`), and type-checks the starter's script against them;
`apps/web/src/plugins/kit.test.ts` holds the members of `AgoraPlugin.Api` to the object the loader (`plugin-frame.js`, plain JavaScript)
makes, and the message types to the ones it sends and takes.

The sample (`apps/web/plugins/sample/`) lists the agents and their states in the theme's colours:

```js
agora.watch('theme.read', (theme) => { /* set --surface-0, --text, … from theme.tokens */ });
agora.watch('agents.read', (data) => { /* draw data.agents with textContent */ }, (e) => { /* e.message is "not-granted" … */ });
```

Draw with `textContent`, not `innerHTML`: agent names are other people's text. Bundle everything into the entry script; only the entry
runs.

## Tests

- Unit (`apps/web/src/plugins/*.test.ts*`): the package checks, the message filter by source, type, size and rate, grants, the store,
  the projection (no approvals, terminal data or actions), the registry's host-set facts, the marker and safe mode; reload on save (a
  change read once it held, a refused change keeping the last package with the reason above the frame, a new capability held, a new
  inert frame per package, nothing in safe mode) and the author kit (the mjs against the shared packages, the starter, the types' names).
- Author kit (`crates/agora/tests/plugin_kit.rs`, `crates/agora/src/pluginkit.rs`): `new`, `check` and `pack` as the binary, byte for byte
  against the mjs's packages, every refusal (a hash, an unlisted file, a sub-folder, a link, a kind of file, a name outside the folder,
  over 2 MiB), and `--watch` packing again past a refusal; the types against the window's by `tsc -p plugins`.
- Hub (`web.rs`): the exact headers of each path; only `/plugin-frame.html` has the frame page's.
- Real browser (`scripts/e2e-plugin-sandbox.mjs`, headless Chromium, and WebKit with `--engine webkit`, both in CI, under the hub's
  headers per path, read from `web.rs` by `scripts/hub-headers.mjs`; both hosts and the STUN and TURN listeners are first shown reachable
  from a plain page, so a refusal inside a frame is the frame's own): every probe of ADR-0114's Verification list fails from inside a frame, and so do a fetch to a hub endpoint
  and the frame going to a blob page it made, judged by the servers' request logs; WebRTC to a STUN and a TURN server reaches neither
  (judged by a UDP and a TCP listener), the `RTC…` constructors are gone for good and no frame the plugin makes hands one back; four
  malformed or oversized message (an unknown type, a long name, a 16 MB array, a 200,000-field object) stops the frame at once, as do
  40 well-formed messages over the rate, without stalling the window (in WebKit, without stalling it after the frame's own send loop,
  which it cannot preempt); a plugin taking the focus cannot keep it, and the
  pane's menu opens safe mode, whose plugins dialog removes a kept plugin; the marker stays whole in a narrow pane; the frame page
  opened on its own is opaque and reaches nothing; the loader is asked for without an `Origin` header (the hub's proxy rule refuses `Origin: null`); a granted read
  answers and one not granted says so; the header cannot be covered; `?safe` and the key load no plugin and the built-in theme; a
  package loads through the dialog.
- Reload on save in a real browser (`scripts/screenshots-plugin-reload.mjs`, headless Chromium under the hub's headers per path, the
  file picker stood in for): a plugin loaded through the picker is listed as watched, a changed package that passes starts a new frame
  that draws the new version and is inert, and one that fails keeps the running frame with the reason above it; screenshots
  `docs/screenshots/plugin-reload-dialog-{dark,light}.png` and `plugin-reload-refused-{dark,light}.png`.
- Visual: `plugin-desktop-dark` and `plugin-desktop-light` (`scripts/visual-regression.mjs`, under the hub's headers per path).

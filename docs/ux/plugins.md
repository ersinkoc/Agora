# Plugins: sandboxed panels in the window

ADR-0114 (Accepted) lets community plugins add panels to the web window (and the desktop app, which loads it). This page is build steps
2 and 3. Step 2: the frame, its page policy, the message set with the rung-1 read capabilities, the window's header and marker, safe mode,
and loading a package from a local file for development. Step 3: plugins the hub's owner installs for the hub, and grants kept per person
on the hub ("Plugins installed on the hub"). The reviewed catalog is step 4.

The code is in `apps/web/src/plugins/` (`hub.ts` for the hub's plugins); the frame page and its loader are
`apps/web/public/plugin-frame.html` and `plugin-frame.js`; the hub's headers for them are in `crates/agora-hub/src/web.rs`; the hub's
package check, storage and messages are `crates/agora-hub/src/plugin_package.rs`, `plugins.rs` and `schema/agora/v1/plugins.proto`; a
sample plugin is `apps/web/plugins/sample/`.

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

**The same checks on the hub.** The hub checks a package with the same rules, in the same order and with the same codes
(`crates/agora-hub/src/plugin_package.rs`, a port of `package.ts`), so it refuses at install what a window would refuse at load. It reads
the JSON as the window does: one leading byte-order mark is dropped, a key given twice keeps its last value, `1.0` is `1`, base64 bits left
over are ignored, and texts are measured in UTF-16 units. A text with a lone surrogate (`"\ud800"`) is refused by both. The two are held
in agreement by shared packages that both test suites check against one list (`crates/agora-hub/tests/fixtures/plugin-packages/`).

**Packing.** `node scripts/pack-plugin.mjs <folder> [--out <file>] [--write-hashes]` packs a flat folder and refuses a file not listed or
a wrong hash, as the window would.

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
start again with it. A development plugin's grant is kept in this browser's storage beside the package (`agora.plugins.grants.v1`); a hub
plugin's is kept by the hub for the person (below). The two never share a grant.

## Plugins installed on the hub

**The owner installs, on the hub's machine.** `agora hub plugin add FILE` checks a package (as above) and keeps it for the hub;
`agora hub plugin list` shows what is installed (id, version, name, what it asks to see, when); `agora hub plugin remove ID` removes one.
They work like `agora hub fleet`: run where the hub's folder is, by whoever can open it, and each add (refused or not) and remove is a row
of the audit trail as the owner's (`plugin.add`, `plugin.remove`; the row names the plugin where a row names its target, and a refused
package's row says why). No protocol message
installs or removes a plugin, so nobody installs one from a window, the owner's included. Adding a plugin whose id is installed replaces
it (another version). A hub keeps at most 16.

**What the hub keeps.** The package as it came (opaque bytes, never run), its SHA-256, and what it read from the checked manifest (id,
version, name, capabilities). Per person and plugin: the version a grant was given for, what that version asked for (`seen`, taken from the
installed manifest, never from the window) and what the person allowed. Removing a plugin forgets everyone's grant for it; removing a
person forgets theirs. The hub reads its store on every request, so a running hub follows `add` and `remove` at once.

**The window.** Once connected, the window asks the hub for the installed plugins (`PluginList`) and this person's grants
(`PluginGrantList`), fetches each package in pieces of at most 512 KiB (`PluginFetch`, by the listed SHA-256: a frame carries at most
1 MiB and a package up to 2 MiB), and checks it again exactly as a file: the bytes must hash to the listed SHA-256 and the checked
manifest must name the listed id and version, or the plugin is left out. Packages already checked are kept for the visit by their hash.
A hub plugin shows the **Plugin** marker without **development**, and "installed on this hub" in the plugins list.

- **Showing one is each person's choice.** A hub plugin nobody chose is listed as not shown; none of its panels is offered.
  **Look at it** opens the install screen; **Show it** sends the grant (`PluginGrantSet`: the id, the version and package shown, and the
  capabilities allowed), and the window changes only once the hub kept it. The hub refuses a grant for any other package than the installed
  one, so a package the owner put in meanwhile, even under the same version, is never taken as seen. **What it can see** changes it; **Stop showing** forgets the person's grant
  (`PluginGrantRemove`) and closes its panes. A hub plugin has no **Remove**: only the owner removes it.
- **A version that asks for more is held**, as for a development plugin: the grant names what the older version asked for, so its panels
  wait until the person looks again. One that asks for nothing new stays shown.
- **Removed on the hub, gone at the next load.** A plugin the owner removed is not in the next list, so a window that loads again (or
  signs in again) drops it, and a pane that showed it shows the "unknown panel" placeholder. A hub plugin is offered only while the hub's
  current list names it.
- **An older hub** that does not know these messages refuses them: the window then has no hub plugins.
- **A development plugin with the same id** stands in for the hub's in this browser, with its own grant in this browser; removing it brings
  the hub's back with the person's hub grant.

**Who may ask.** Every signed-in person, owner, member or viewer: each lists the installed plugins, fetches their packages and sets and reads
only their own grants. A plugin only reads what its person can already see in the window, and choosing what it may see changes nothing on
any machine, so a viewer may show a plugin too. The hub answers these itself; a request that names a node is refused
(`protocol.invalid_message`, `target_node`). Setting and removing a grant are audit rows of the person (`plugin.grant`, `plugin.ungrant`).
The messages are in `docs/specs/protocol.md`, "Plugins (ADR-0114)".

## Safe mode

`?safe` in the address, or the safe-mode key **Ctrl+Alt+Shift+S** (⌘⌥⇧S on a Mac, by the key's place), opens the window with no plugins
and the built-in theme (`plugins/safe.ts`): nothing is asked of the hub, neither the plugin store nor the theme store reads this browser's storage, so another tab's
change cannot bring a stored theme back either, and a choice made in safe mode holds for that visit only. A note at the bottom says so,
with **Leave safe mode**.

The key is heard by the window first, in the capture phase, before any handler of the page's own; no plugin, theme or keybinding can
change it (it is a fixed key, `keybindings.md`, and in safe mode the keys are the defaults). **Open in safe mode** is also in every plugin pane's menu (⋯) and in the command palette, for the mouse and for the desktop
app, which shows no address.

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

## Writing a plugin

The sample (`apps/web/plugins/sample/`) lists the agents and their states in the theme's colours:

```js
agora.watch('theme.read', (theme) => { /* set --surface-0, --text, … from theme.tokens */ });
agora.watch('agents.read', (data) => { /* draw data.agents with textContent */ }, (e) => { /* e.message is "not-granted" … */ });
```

Draw with `textContent`, not `innerHTML`: agent names are other people's text. Bundle everything into the entry script; only the entry
runs.

## Tests

- Unit (`apps/web/src/plugins/*.test.ts*`): the package checks (and the shared packages the hub checks too), the message filter by source,
  type, size and rate, grants, the store, the projection (no approvals, terminal data or actions), the registry's host-set facts, the
  marker and safe mode; hub plugins (`hub.test.ts`, `hubDialog.test.tsx`): loaded in pieces and checked again, grants kept on the hub and
  read back, gone once removed, held on a new capability, left out when the bytes or the list do not agree, none from an older hub or in
  safe mode, a development plugin standing in with its own grant, and the dialog without Remove.
- Hub: the package check agrees with the window on the shared packages (`tests/plugin_package_agreement.rs`); storage, pieces, the cap
  and grants (`src/plugins.rs`); over real sockets, owner, member and viewer each list, fetch and keep grants of their own, a node named is
  refused, and a removed or replaced package is not served (`tests/plugins.rs`); `agora hub plugin add|list|remove` with its audit rows
  (`crates/agora/tests/hub_cli.rs`); golden fixtures of the messages (`schema/fixtures/plugins/`).
- Hub (`web.rs`): the exact headers of each path; only `/plugin-frame.html` has the frame page's.
- Real browser (`scripts/e2e-plugin-sandbox.mjs`, headless Chromium under the hub's headers per path, read from `web.rs` by
  `scripts/hub-headers.mjs`): every probe of ADR-0114's Verification list fails from inside a frame, and so do a fetch to a hub endpoint
  and the frame going to a blob page it made, judged by the servers' request logs; WebRTC to a STUN and a TURN server reaches neither
  (judged by a UDP and a TCP listener), the `RTC…` constructors are gone for good and no frame the plugin makes hands one back; four
  malformed or oversized message (an unknown type, a long name, a 16 MB array, a 200,000-field object) stops the frame at once, as do
  40 well-formed messages over the rate, without stalling the window; a plugin taking the focus cannot keep it, and the
  pane's menu opens safe mode, whose plugins dialog removes a kept plugin; the marker stays whole in a narrow pane; the frame page
  opened on its own is opaque and reaches nothing; the loader is asked for without an `Origin` header (the hub's proxy rule refuses `Origin: null`); a granted read
  answers and one not granted says so; the header cannot be covered; `?safe` and the key load no plugin and the built-in theme; a
  package loads through the dialog.
- Real hub (`scripts/e2e-plugin-hub.mjs`, headless Chromium with a real hub and node): the owner installs the sample plugin; the window lists
  it as the hub's without Remove and draws none of its panes until the person shows it; then it draws the real agents, and after a reload
  it is drawn with the grant the hub kept (nothing in this browser's storage); once the owner removes it, the next load drops it.
- Visual: `plugin-desktop-dark` and `plugin-desktop-light` (`scripts/visual-regression.mjs`, under the hub's headers per path).

# 0114. Community plugins run sandboxed in the window and ask for what they see
Status: Accepted
Date: 2026-11-25

Accepted: 2026-11-25 (the maintainer, in the merger chat, chose "Sandboxed" for decision 1, the trade-off). The open questions were not answered; the proposed answers stand.
Amended: 2026-11-26 (Amendment A below: the frame is a page the hub serves under its own header policy).

Answers ADR-0068's open question on custom panels ("the contract first, the loader later, or a plugin system now?"). Touches
ADR-0020 and ADR-0028 (approvals are signed by a device key in the window), ADR-0048 (the desktop app), ADR-0097 (Proposed: the hub
serves the window's code), the hub's page policy (`crates/agora-hub/src/web.rs`) and the threat model (new T63). The themes half
is ADR-0113.

## Context

The maintainer wants community plugins next to community themes, as the most extensible agent workspaces have.

**What a comparable agent workspace does (research of 2026-11-25, sources in the PR).**
- **The terminal app:** plugins are programs in any language that call the product's whole command line and socket API. In its
  own words they "run as your user" and it "does not review or sandbox plugin code". Its marketplace indexes any GitHub
  repository with its plugin topic and says it is not reviewed.
- **The desktop app:** "mods" are JavaScript that can run programs, fetch URLs, read files and "run programs on connected
  machines", with no permission prompt. Its security page calls them "trusted JavaScript".

**What others do.**
- **The isolation-and-permissions end:**
  - **Zellij:** WASM plugins with 17 named permissions, asked of the person and remembered.
  - **Zed:** WASM extensions with capabilities and a reviewed registry; its incidents were in unpacking, not in the sandbox.
- **The no-sandbox end:**
  - **VS Code:** extensions have the editor's full rights. The marketplace scans, but self-spreading credential stealers have
    still got through (GlassWorm, 2025–2026).
  - **Obsidian:** plugins also have full rights, behind a restricted mode on by default and review for popular plugins.

**What is different here.**
- **The window holds the person's signing key.** It is a non-extractable WebCrypto key (ADR-0028), but any code running in the
  window's origin can sign with it:
  - approvals, including skip-permission ones;
  - grants for signed commands;
  - endorsements of new devices (ADR-0097).

  So code in the window can say yes for the person on every machine.
- **Agents run code on those machines.** A plugin that could send a prompt to an agent in a mode that skips permissions could run
  code there.
- **Terminal bytes are not redacted** (only chat text is, ADR-0089), so a plugin that reads terminals and also reaches the
  network can carry secrets away.
- **The panel registry does not exist yet** (ADR-0068 rung 3): panels are a fixed list (`layout.md`), so there is nothing for a
  plugin to plug into today.

## Decision

1. **The trade-off: sandboxed plugins, not trusted code.** Plugins get less power than that product's mods: no programs, no files, no
   network at first, no signing, ever. In exchange, installing one cannot hand anyone the person's machines.
   - We recommend this because of the key in the window and the agents on the nodes.
   - A trusted, unsandboxed mode is listed as an alternative and rejected. The power such mods have on a machine already
     exists here as agents, manifests and adapters (`docs/architecture.md#integration-levels`), which run on the node under its policy.
2. **Where plugins run: in the web window (and the desktop app, which loads it), as panels, each in a sandboxed frame.**
   - **The frame:**
     - It is `sandbox="allow-scripts"` and never `allow-same-origin`. So it has an opaque origin and cannot reach the window's
       storage, its IndexedDB key, its cookies or its connection to the hub.
     - It has no `allow-forms`, `allow-popups`, `allow-modals`, `allow-top-navigation` or `allow-downloads`.
   - **The frame's page (Amendment A):** one page the hub serves beside the window's files (`/plugin-frame.html`), under a
     header policy of its own, never a page the window makes:
     - `sandbox allow-scripts; default-src 'none'; script-src 'self' blob:; style-src 'self' blob: 'unsafe-inline'; img-src blob:
       data:; font-src blob:; connect-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'; webrtc 'block';
       frame-ancestors 'self'`, with `X-Frame-Options: SAMEORIGIN`.
     - The `sandbox` directive keeps the page opaque even when it is opened on its own.
     - No connection to any other origin: no fetch, WebSocket, image, font or page from anywhere else, and no WebRTC (the loader takes
       its constructors away before the plugin runs, as not every engine enforces `webrtc 'block'`). Towards the hub itself it can
       only make credential-less GET requests for the hub's own paths (a script, a stylesheet, its own frame going to a hub page),
       whose answers it cannot read.
     - It runs only the window's loader (`/plugin-frame.js`).
   - **Loading:** the frame is built from the plugin's checked files (decision 5). The window sends the bytes to its own frame by
     message; the loader makes every file a blob inside the frame's opaque origin and runs the entry. Nothing is loaded from a
     plugin's URL.
   - **The window's policy** is not loosened: it gains only `frame-src 'self'`, and every other page of the hub still refuses to be
     framed (`X-Frame-Options: DENY`, `frame-ancestors 'none'`).
   - **The desktop app:** its webview grants the hub's page nothing (ADR-0048 decision 2), so a frame inside it has nothing to
     reach either. (ADR-0048 Amendment A, Proposed, would grant the hub's top frame one command that asks for a notification; a
     plugin's opaque frame still reaches nothing.)
3. **The interface: a small, versioned message set over `postMessage`, and capabilities the person grants.**
   - **Messages:** the host accepts only messages from its own frames, of known types, within size limits, at a bounded rate.
   - **Capabilities (rung 1), each read-only and scoped to what the person can already see:**
     - `agents.read`: names, kinds, states, machine, project
     - `usage.read`
     - `tasks.read`
     - `changes.read`: file lists and diffs of the bound agent
     - `chat.read`: the bound agent's conversation as the window shows it, already redacted
     - `theme.read`: tokens, so a plugin matches the theme
     - `pane.context`: what the pane is bound to
   - **Granting:** the manifest declares the capabilities, the install screen lists them in plain words, and the person grants
     them per plugin.
   - **Updates:** a plugin version that asks for more needs a new grant.
   - **Not in rung 1, and each needs an amendment to this ADR when wanted:**
     - **Terminal reading:** never together with any network capability.
     - **Actions:** UI actions (open a pane, focus an agent, copy through the host) and requests the person completes in host UI
       (for example a prepared prompt they read and send themselves).
     - **Network:** to declared hosts only, shown at install, and never with `chat.read` or terminal reading.
   - **Never, in any rung:**
     - signing anything, or reaching approvals, grants, endorsements or devices;
     - secrets;
     - typing into a terminal;
     - starting, stopping or prompting agents on its own;
     - changing settings, policy, themes or other plugins.
4. **The host draws the frame, and nothing can draw over it.**
   - **Header:** every plugin panel has a header drawn by the window with the plugin's name and a "plugin" marker. Themes cannot
     restyle the marker (ADR-0113 decision 3), and the plugin cannot reach it.
   - **Containment:**
     - A plugin draws only inside its pane.
     - It opens no overlays, dialogs or notifications in rung 1.
     - The approvals area, the "needs you" list and the sign-in screens never host plugin content.
   - **Safe mode:** a key that no plugin, theme or keybinding can rebind, and a `?safe` address, open the window with no plugins
     and the built-in theme.
5. **The package.**
   - **Contents:** a plugin is one archive holding a manifest `agora-plugin.json` (id, name, version, author, licence, minimum
     Agora version, panels, capabilities, entry file) and its files, each listed with its SHA-256.
   - **Limits:** the whole archive is at most 2 MiB.
   - **No remote code:** everything a plugin runs is in the archive.
   - **Unpacking:** done in memory against the list: no paths, no links, nothing outside the list (the Zed unpacking CVEs).
6. **Installing and keeping plugins.**
   - **Who installs:** the hub's owner installs a plugin for the hub (`agora hub plugin add FILE`, or from the catalog,
     decision 7).
   - **Storage:** the hub keeps the archive as opaque data, checks the manifest and hashes, never runs it, and gives it to
     windows.
   - **Use:** each person chooses whether to show it and grants its capabilities. Grants are kept per person.
   - **Removal:** removing a plugin removes it from every window at its next load.
   - **Schema:** storing and listing plugins and keeping grants needs new schema messages, added append-only.
   - **Development:** loading a plugin from a local file into one browser only, with a "development" marker, works without the
     hub.
7. **A reviewed community catalog,** off until the owner turns it on.
   - **Where plugins live:** a public repository of plugin archives, separate from themes.
   - **Review:**
     - Every addition and every update is reviewed.
     - A licence is required, and minified or obfuscated code is refused.
     - CI checks the manifest, sizes, hashes and the capabilities against what the description says.
   - **Signing:** each release is signed with the catalog's key, and the hub checks that signature before it offers an install
     or update.
   - **Fetching and updates:** the hub fetches the catalog on the owner's command, as for themes (ADR-0113). An update is never
     automatic: the owner sees what changed, capabilities first.
8. **Other clients and the node.**
   - **Terminal client:** no plugins (it has no web frame). A plugin panel shows its name and "open in the window".
   - **Phone layout:** the same placeholder in rung 1.
   - **Node:** no plugin runtime. Extending what runs on a machine stays agents-as-data (manifests, ACP, adapters). Automations
     that run on events (event hooks in that product's plugins) would run code on a machine and need their own ADR.
9. **Threat model: new row T63.**
   - **What a plugin may see:** the read-only, granted, already-redacted views above.
   - **Isolation:** the frame's opaque origin and no network keep it from the key, the hub connection and outside servers. The
     host-drawn marker and the safe mode keep it from posing as the window.
   - **Residuals:**
     - A plugin can show misleading content inside its own pane.
     - DNS prefetch hints are not covered by every browser's page policy.
     - A compromised hub can already serve any code to the window (ADR-0097), and plugins do not change that.
     - The catalog's key is a new key to protect.

## Alternatives considered

- **Trusted plugins with full rights** (that product's mods, VS Code, Obsidian): the most power and the easiest to write. Rejected: one
  bad or hijacked plugin could sign approvals and run code on every machine. Review and scanning have not stopped this
  elsewhere.
- **WASM plugins with a declarative UI** (Zellij, Zed): strong isolation and a clear capability model. A sandboxed frame gives the
  same isolation in a browser with no runtime to ship. Plugin authors also write ordinary HTML and JavaScript, and the browser
  already enforces the policy. WASM stays an option for node-side automation (decision 8).
- **Plugins in the terminal client:** no shared runtime that would keep them out of the key and the files; later, if wanted.
- **No plugins, only the panel registry for built-in panels:** safest. It misses the community the maintainer asked for.

## Build order

1. **The panel registry** (ADR-0068 rung 3): built-in panels move onto it. Plugins need it. Done: `docs/ux/layout.md`, "The
   panel registry (rung 3)".
2. **The sandbox host:** the frame, the page policy, the message set with the read capabilities, the host-drawn header, safe
   mode, and local development loading. Done: `docs/ux/plugins.md`.
3. **Storage on the hub,** install and remove, and per-person grants (schema messages, fixtures). Done: `docs/ux/plugins.md`, "Plugins installed on the hub". This narrows ADR-0049's "viewers have no commands" for one case: a viewer may set and remove their own plugin grants (hub only; nothing reaches a node).
4. **The catalog:** repository, review rules, signatures, `agora hub plugin` commands. Done on the hub's side: `docs/ux/plugins.md`, "The
   community catalog" (`agora hub plugins key set|on|off|status|update|install|upgrade`; the index signed with `ssh-keygen -Y sign` and
   checked under the key the owner pinned, a serial against going back, an archive installed only when it is exactly the signed entry,
   the owner asked first; windows list it read only with `PluginCatalogList`; threat-model row T68). The catalog's repository, its CI
   and its key are the maintainer's to make, by the procedure on that page.
5. **Further capabilities,** each by amendment.

## Verification

- **From inside a plugin frame, each of these fails:**
  - `indexedDB.open`, `localStorage`, `document.cookie`;
  - `fetch` to the hub and to an outside host, `new WebSocket` to both, an image or font from an outside host;
  - navigating the top window, `window.open`, a form post;
  - reading `parent.document`;
  - `postMessage` of an unknown type or an oversized message (ignored and counted; since build step 2 the frame is stopped too).
- **Grants:** a capability not granted answers "not granted". A version that adds a capability is held until granted.
- **Containment:**
  - In a screenshot test the plugin header is drawn above the frame, and the frame cannot cover it or the approvals area.
  - `?safe` and the safe-mode key load no plugin and the built-in theme.
- **Packaging:** an archive with a path, a link, a file not in the list, a wrong hash or over 2 MiB is refused whole.
- **Catalog:** an update with an invalid signature is not offered.
- **Desktop:** the same frame tests pass in the desktop app's webview on Linux, macOS and Windows.

## Open questions

- **Should members install plugins** for themselves, or only the owner for the hub? Proposed: only the owner in rung 3.
- **Which UI actions should come first** (decision 3)? Proposed: open a pane and focus an agent.
- **Should a plugin panel be allowed in a theme pack's layout** before the plugin is installed? Proposed: yes, as the "unknown
  panel" placeholder (ADR-0113 decision 5).
- **Node-side automations on events:** a separate ADR when wanted.

## Amendment A (accepted by the maintainer 2026-11-26, in the merger chat): the frame is a page the hub serves under its own header policy; the window's policy is not loosened

Accepted: 2026-11-26 (the maintainer, in the merger chat, chose the hub-served frame page over loosening the window's `script-src`).
Decision 2 is worded with it.

**Why.** Found while building step 2, in Chromium 156:

- **The window's policy stacks on a blob frame.** A frame whose page is a blob the window made keeps the window's page policy as well as
  its own (local-scheme pages inherit their maker's policy container). The window has no `script-src`, so `default-src 'self'` governs
  scripts in the frame, while the frame allowed scripts only from `blob:`: no script could run in it. Making it run would have meant
  `blob:` in the window's own `script-src`.
- **An opaque frame cannot load the window's blobs.** A script blob the window made is refused in an opaque frame ("Not allowed to load
  local resource": blob addresses are partitioned by origin), so the plugin's files could not be blobs of the window's either.

**Decided.**

1. **The frame's page is served by the hub** at `/plugin-frame.html`, from the web app's files, with headers of its own: the policy in
   decision 2 (with `sandbox allow-scripts`, so the page is opaque even on its own, `webrtc 'block'` and `frame-ancestors 'self'`) and
   `X-Frame-Options: SAMEORIGIN`. A page from the network keeps only its own policy, so the window's does not stack on it.
2. **The window's loader** (`/plugin-frame.js`) is the one script of that page. The window sends the plugin's checked files to that
   frame only, by message; the loader makes them into blobs inside the frame's opaque origin and runs the entry.
3. **The window's policy** gains only `frame-src 'self'` (it was to gain `frame-src blob:`). Every other path of the hub keeps today's
   headers exactly, `X-Frame-Options: DENY` and `frame-ancestors 'none'` included, so the frame page is the only page that can be
   framed, and only by the hub's own pages. `web.rs` tests the headers of each path.

**What it costs.** `script-src 'self'` in the frame page also allows the hub's other scripts there; they run in the frame's opaque
origin with no connections, like the plugin. `'self'` in `script-src` and `style-src`, and `frame-src 'self'` in the window, let the
frame make credential-less GET requests for the hub's own paths, whose answers it cannot read. A frame can navigate itself only to the hub's own pages (`frame-src 'self'`), which refuse to
be framed. The frame page is one more file the hub serves with headers of its own.

**Evidence.** `scripts/e2e-plugin-sandbox.mjs` runs every check of the Verification list under the hub's headers per path (read from
`web.rs`), the frame page opened on its own included; the real hub binary serves the same headers and the sample plugin draws in it.

## Amendment B (Proposed, 2026-11-26): plugins can act: open their panels, show an agent, prepare a prompt, add commands the person can give keys

Status: Proposed. Waits for the maintainer; nothing below is built. ADR-0114 itself stays Accepted, and its decisions stand as written
until this amendment is accepted.

Origin: roadmap M5.4 (steps P1 and P2) and the gap review of 2026-11-26 (`docs/reviews/2026-11-26-architecture-gap-review.md`, section
4, "The capability gap"). Decision 3 lists actions as "not in rung 1", each needing an amendment, and the open question "Which UI actions
should come first?" proposed opening a pane and focusing an agent.

### Context

- **Rung 1 only reads.** A plugin can show something but cannot do anything. Extension systems elsewhere run actions, add commands to
  the palette and bind keys. Without some of that, few people will write plugins.
- **What does not move.** Decision 3's "never" list stays whole: no signing, no answering a request, no typing into a terminal, no
  starting, stopping or prompting an agent on its own, no settings, themes or other plugins. Decision 4 stays too: the window draws, a
  plugin draws only inside its pane, and safe mode has a key no one can rebind.
- **What exists to build on:**
  - the frame is `inert` until the person clicks into it, and inert again once they click or move the focus elsewhere (`plugins.md`,
    "The focus is the person's");
  - one message filter with exact fields, a rate and a stop (`plugins/protocol.ts`);
  - grants per capability, with a held version when one asks for more (`plugins/grants.ts`);
  - commands as data, a keybindings file, `FIXED_KEYS`, `NEVER_COMMAND`, and unknown commands kept and inert (#1224,
    `apps/web/src/keys/`, `keybindings.md`).
- **The hard part is telling the person's gesture from the plugin's.** The window's loader runs in the plugin's own realm, so anything
  the frame says about a click can be forged. The window must decide from what it observes itself.
- **A prepared prompt is close to "prompting an agent".** An agent that acts without asking runs code on its machine (Context, above). So
  the person must read the text, and only the person sends it.

### Decision

**B1. Four new capabilities, granted like the rung-1 ones.**

| Capability | What it lets the plugin do | The install screen says (`plugin-cap-*`) |
| --- | --- | --- |
| `pane.open` | open one of its own panels in a pane beside its own | "Open its own panels beside this one" |
| `agent.focus` | bring an agent the person can see to the front | "Show you an agent" |
| `prompt.prepare` | write a prompt into a chat agent's message box, for the person to read and send | "Write a message for you to read and send. It never sends one." |
| `commands.add` | list its commands in the command palette, where the person can give them keys | "Add its commands to the command palette" |

- **Declared** in the manifest's `capabilities`, as today. `commands.add` also needs the manifest's `commands` list (B6); one without
  the other refuses the package.
- **Granted** in the same screen and the same record (`seen`, `granted`), none by default. The screen lists what a plugin can see,
  then what it can do, under **What it can do**; under `commands.add` it lists the commands by name. The plugin's own words are shown
  as text, never as the window's. **What it can see** in the Plugins list becomes **What it can see and do**.
- **A version that asks for one of these** is held until the person looks again (the existing rule). A version that only changes its
  command list keeps its grant: a command does nothing until the person picks it or gives it a key.
- **Still not added:** copy through the host, terminal reading, network, overlays and notifications (P3), events (P5).

**B2. The messages.** Still version 1: new types are added, no existing type changes.

| From | Type | Fields | |
| --- | --- | --- | --- |
| frame | `open` | `id`, `panel`, `agent` | `panel`: one of its own panel ids; `agent`: an agent id or `null` |
| frame | `focus` | `id`, `agent` | |
| frame | `prepare` | `id`, `agent`, `text` | |
| window | `result` | `id`, `ok`, `data` or `error` | as today; `data` is `null` for an action |
| window | `command` | `command` | the plugin's own command id (B6) |

- **Refusals** (stable codes, in `error`, never words):
  - `not-granted`: the capability was not allowed (as today);
  - `no-gesture`: not during the person's turn with this frame (B3);
  - `busy`: the approvals area, a dialog or a Review has the window (B4), or the message box already holds text (B5);
  - `unavailable`: no such panel, an agent the person cannot see or that cannot take a prompt here (an unknown id and a hidden one
    answer the same, so the plugin learns nothing new);
  - `no-room`: the layout is full (32 panes or 6 levels);
  - `bad-text`: text the window will not put in a message box (B5);
  - `too-fast`: over the action rate.
- **Bounds:**
  - `panel` at most 40 characters, `agent` at most 128, `text` at most 4,096 characters.
  - Over a bound is `size`, which stops the frame (as an oversized capability name does today). The loader checks first and rejects
    with `too-long`, so a plugin that uses `agora.prepare` is never stopped for it.
  - The filter's field bound grows from four to five (`open` and `prepare` have five with `agora` and `type`).
  - On top of 20 messages a second: at most one action a second per frame (a burst of two). Over it answers `too-fast`.
- **Safe mode, held, stopped:**
  - In safe mode no plugin runs, so no frame asks and no command is listed; nothing needs a code.
  - A held plugin has no panels, so no frame and no commands.
  - A stopped frame's messages are never read.
- **The loader adds** `agora.open(panel, agentId?)`, `agora.focus(agentId)` and `agora.prepare(agentId, text)`. Each returns a Promise
  that is rejected with an `Error` whose message is the code. `agora.onCommand(fn)` sets the one handler for `command`.
- **Older windows and hubs:**
  - An older window never loads a plugin that asks for these: an unknown capability or manifest field refuses the package, and
    `min_agora` says why in words. The loader is the window's own, so it never sends a type its window does not know.
  - The hub's list of capability names and manifest fields (step 3, #1226, `plugin_package.rs`) grows with the window's.
  - Capabilities and commands are strings inside the package and the grant record. No schema message changes.

**B3. Only during the person's turn.** An `open`, `focus` or `prepare` is taken only during the person's turn with that frame. A turn is
one of two things, both judged from what the window observes, never from the frame's word:

- **In use:**
  - the frame is in use (the person clicked into it or pressed Enter on it, and nothing else since: the window's own `active` state,
    set from its own events);
  - it has the window's focus (`document.activeElement` is the frame);
  - its pane is on screen;
  - and the window has transient user activation (`navigator.userActivation.isActive`).

  A click or a key inside a frame gives the window activation too (HTML's activation notification reaches every ancestor document,
  whatever its origin). While the frame is in use, a click anywhere else in the window makes it inert. So the activation can only come
  from the person's input in that frame, or from the click that put it in use.
- **A command:** the person ran one of the plugin's commands (palette or key), and the window sent `command` to this frame. For 2 s
  after that, the frame may take one action.

The rest:

- **Never** from a timer, a watch's `update`, `load`, or another frame.
- **One gesture, one action.** Every action the window takes moves the focus to a pane of the window's own (B4), so the frame turns
  inert and the next action needs a new click into it. A command's turn allows one action.
- **No "clicked" flag.** No message carries one: the plugin could forge it.
- **To be proven first** (build step 1), in Chromium and WebKit (Playwright's WPE in CI), WebKitGTK and WebView2:
  - a click in an opaque frame gives the window activation;
  - a message sent from a timer with no input finds none.

  An engine without `navigator.userActivation`, or where the proof fails, answers `no-gesture` to every "in use" action: there only
  commands can act. It fails closed.
- **Residual:**
  - Activation lasts a few seconds (about five in Chromium), so an action may come a moment after the click, not only in its handler.
  - Any input into the frame counts as a gesture, the plugin's own scrolling included.

  Both are bounded by one action per gesture, and by what an action can do (B4, B5).

**B4. Where actions land, and the focus.**

- **Not while the person decides.** While the approvals area, the "needs you" list, the attention dialog, a Review, or any dialog of
  the window has the focus or is open, every action answers `busy` and nothing moves. An action never takes the focus from those.
  When an action moves or resizes a pane that shows a request, its answer guard's dwell starts again (T44, `useAnswerGuard`).
- **`pane.open`:**
  - Only one of the plugin's own panels, never a built-in one.
  - It opens in a new pane beside the plugin's pane, as **split right** does, within the layout's limits (`no-room` when full,
    nothing changes).
  - A `tool` panel is bound to `agent`, which must be one the person can see; a `global` panel takes `null`.
  - A pane that already shows that panel with that binding is brought to the front instead.
  - It never replaces or closes a pane.
- **`agent.focus`:**
  - The agent id must be one the plugin's own view holds (`agents.read` or `pane.context`), for an agent the person can see.
  - A pane that follows the selection shows it. Otherwise its pane comes to the front, or it opens in a new pane beside the plugin's.
  - Unlike selecting in the agent list, it never closes a pane (selecting the agent of the pane in focus does) and never opens a tab
    over the plugin's own pane.
- **The keyboard after an action:**
  - The window focuses the pane it showed: the pane itself, never a terminal, a message box or any text field inside it.
  - So keys the person was still typing into the plugin go nowhere that types. They click or press Enter to type there.
- **Nothing over the window:** an action opens no overlay, dialog, toast or notification. The pane bar of a pane a plugin opened is the
  window's, as always.

**B5. A prepared prompt.**

- **Only into a chat agent's message box:**
  - A structured (ACP) agent the person can see and may talk to.
  - A terminal agent answers `unavailable`, since typing into a terminal is "never". So do an ended agent and a viewer's window
    (ADR-0106: no box to type in).
  - Never into an answer to a request, a Review, a reason for a Deny, a terminal, the team chat, a task, a setting or another plugin.
- **Shown where the person can read it.** The agent's pane comes forward as for `agent.focus` (B4), and the focus goes to that pane,
  not into its box.
- **The person's draft is never touched.** A box that already holds text, the person's or a plugin's, answers `busy`.
- **Text the window refuses (`bad-text`):**
  - Control characters other than a new line and a tab.
  - Invisible and direction-changing characters (zero-width, bidi overrides and isolates), as for a plugin's name. The person sees
    exactly what would be sent.
  - A leading `/`: slash commands can switch a mode or end the agent (`/exit`), so a plugin cannot prepare one.
- **Marked:**
  - Above the box the window draws one line: the plugin's name as plain text, the **Plugin** marker (theme floor, ADR-0113), and
    "wrote this. Read it before you send it.", with **Clear**.
  - The line stays until the text is sent or the box is empty.
- **Never sent by itself:**
  - No message sends it. The focus is not moved into the box (B4).
  - Send and Enter work only once the person has put the keyboard in the box themselves, and Send is off for the surface's short
    dwell after the text arrives. So a click or a key already on its way does not send it.
- **An agent that acts without asking** (its Acts-without-asking sign): the line says so in the risk colour. The first Send asks once
  more, in the window's own words: "Send text a plugin wrote to an agent that acts without asking?"
- **Untrusted data.**
  - Once sent, it is the person's prompt like any other. What the agent then does still goes through the node's policy and approvals
    (ADR-0020); a prompt widens nothing.
  - Nothing about it is signed.
  - The draft lives in this window only and is never stored on the hub.

**B6. Commands and keys.**

- **Declared in the manifest:** `commands`, 1 to 16 of `{ id, name, panel? }`.
  - `id`: lower-case letters, digits and `-`, starting with a letter, at most 32. It may not be `request`, `approval`, `approve`,
    `deny` or `allow` (`NEVER_COMMAND` applied to the plugin's own part; the full id starts with `plugin.`, which that pattern does not
    see).
  - `name`: one line of at most 60 characters, no control or invisible characters, shown as text, never as a Fluent key.
  - `panel`: which of its panels receives the command (the first by default).
- **The full id** is `plugin.<plugin id>.<command id>`, like a panel's. No built-in id starts with `plugin.` (a test keeps it so), and
  an id is registered once. So a plugin command cannot shadow a built-in one.
- **No default keys.** The manifest has no `keys` field (an unknown field refuses the package). The person gives keys in **Settings >
  Keyboard**, or in the keybindings file, like any other command.
- **Shown as the plugin's:**
  - In the palette, under **Plugins**, as "⟨plugin name⟩: ⟨command name⟩" with the **Plugin** marker (and **development** where it
    applies). So it never stands alone as the window's own words, an "Allow" included.
  - In Settings > Keyboard, as rows under the plugin's name with the marker.
- **Offered only while its panel is on screen.**
  - The plugin's frame registers the handler while it is drawn, as parts of the window register theirs (`keys/registry.ts`).
  - The command goes to the panel in the focused pane if it shows one, otherwise to the one the person used last. It never goes to a
    frame that is not on screen.
  - With no pane showing it, the command is not offered, and its key does nothing and goes on (as a built-in command that cannot act).
- **Can a command wake an inert frame?** It reaches it, and that is all.
  - `inert` blocks input and focus, not the frame's script, so the frame gets `command`.
  - The window does not put the frame in use and does not focus it.
  - The command's turn allows one action (B3). There is never a hidden frame: no command starts a plugin whose panel is not shown.
- **Keys a plugin command may not take:**
  - The fixed keys (`FIXED_KEYS`): safe mode; Tab, Shift+Tab, Esc, Enter, Space and the arrows (which also press a focused card's
    buttons); a terminal's copy, find and paste; editing text.
  - The keys `palette.open` and `attention.next` have in force (by default Ctrl+K, ⌘K, Ctrl+Shift+K, and N), so the palette and what
    needs you can always be reached.
  - Refused with its reason (`keys-error-plugin-key`) in the file reader and the recorder.
  - The web window has no keys that answer a request (`keybindings.md`, "No command answers a request"). Alt+Y, Alt+N and Alt+A are
    the terminal client's, which has no plugins.
- **Where a key acts is unchanged:**
  - A terminal, a text field, and a plugin frame in use keep their keys. A key pressed inside a frame goes to the frame, not to the
    window's commands.
  - A held key runs a plugin command once (`repeat` is ignored), and `command` goes to one frame at most four times a second.
- **When the plugin goes:**
  - A plugin not shown, held, without `commands.add`, removed for the hub, or in safe mode has no commands in the palette or Settings.
  - Its bindings stay in the file as unknown commands that bind nothing (the existing rule). Settings lists them as "plugin not
    loaded", with **Clear**.
  - **Remove** in the Plugins list also takes its bindings out of the file.
  - In safe mode the keys are the defaults and no plugin runs.

**B7. Threat model.** A new row, numbered when built (T65 to T68 are taken by open pull requests; T69 if none of them moves):

- **The attacks:**
  - A plugin acts without the person: from a timer, a watch update, or a forged "clicked" flag.
  - It moves the layout or the focus under a click or a key meant for a request card (T44), or sends the person's typing into a
    terminal or a message box.
  - It prepares a prompt that the person sends unread to an agent that acts without asking, hides words in it (invisible or bidi
    characters, T46), or prepares a slash command.
  - It overwrites the person's draft.
  - Its command or key poses as the window's ("Allow"), takes the palette's or the safe-mode key, or shadows a built-in command.
  - A command keeps a hidden plugin running.
- **The answers:** B3 to B6.
- **Residuals:**
  - An action may come a few seconds after the person's input in the frame, and any input into the frame counts.
  - A plugin can still write a misleading prompt that a person sends without reading it.
  - A keybindings file shared by someone else can bind a plugin command to a key the person uses, shown as a conflict.

T63 changes with it: "there are no write messages at all" becomes the three actions and their rules.

**B8. What changes in the web window.**

- **`plugins/`:**
  - `capabilities.ts`: the four names and their words.
  - `protocol.ts`: the new types, five fields, the action rate, the codes.
  - `PluginFrame.tsx`: turns, actions, delivering `command`, registering commands while on screen.
  - `package.ts`: the `commands` field and its checks.
  - `PluginsDialog.tsx`: **What it can do**.
- **The loader** (`public/plugin-frame.js`): `agora.open`, `focus`, `prepare` and `onCommand`.
- **The layout** (`layout/engine.ts`, the workspace): open beside the plugin's pane; bring an agent forward without closing a pane;
  focus a pane, not its contents.
- **The chat's message box:** the plugin line, the dwell, the extra question for an agent that acts without asking.
- **`keys/`:** commands added while a plugin is loaded, the reserved keys, `keys-error-plugin-key`, and the rows with the marker.
- **Elsewhere:**
  - the hub's lists (`plugin_package.rs`);
  - `locales/en` and `locales/tr`;
  - `plugins.md`, `keybindings.md` and `threat-model.md` in the pull requests that build each part.

### Build order

1. **The gesture proof:** the real-browser check (B3) in Chromium and WebKit, and in the desktop app's webviews. If it fails, stop and
   amend B3 before anything else.
2. **P1, open and show:** `pane.open` and `agent.focus`, the messages, turns, the focus rule, the grant screen.
3. **P1, prepared prompts:** `prompt.prepare` and the message box's line, dwell and extra question.
4. **P2, commands and keys:** the manifest field, palette and Settings rows, `command`, the reserved keys.
5. **The hub's lists** (after #1226), the threat-model row, the docs, and screenshots in both themes.

### Verification

- **Real browser** (`scripts/e2e-plugin-sandbox.mjs`, Chromium and WebKit):
  - An action sent from a timer, from a watch update, or right after the person clicked elsewhere answers `no-gesture`. One sent on a
    click in the frame is taken; a second one right after it is not.
  - After an action the frame is inert, and the focus is on the pane the window showed, not inside it.
  - With the approvals area or a dialog in focus, every action answers `busy`, and nothing moves.
  - A command run from the palette lets one action through, within 2 s; a command does not put the frame in use or focus it.
- **Unit:**
  - the filter (five fields, sizes, the action rate);
  - grants (held for a new action capability, kept for a changed command list);
  - the package (`commands` without `commands.add` and the reverse, ids, names, 17 commands);
  - `bad-text` cases and a leading `/`, and the draft never overwritten;
  - Send's dwell and the extra question;
  - reserved keys refused, unknown plugin commands kept, Remove taking bindings out, no built-in id starting with `plugin.`.
- **Visual:** the message box with a plugin's text, the palette with a plugin command, Settings > Keyboard with plugin rows; dark and
  light.

### Alternatives considered

- **Trust a "clicked" flag from the frame.** The loader shares the plugin's realm, so the plugin can forge it. Rejected.
- **A window-drawn confirmation for every action.** Safe, but a dialog per click is noise (ADR-0024). Kept only for sending a plugin's
  text to an agent that acts without asking.
- **Let a plugin send a prompt after a confirmation.** That is "prompting an agent", which decision 3 forbids in any rung. Rejected.
- **Commands registered by the frame while it runs.** They could not be reviewed in the catalog or shown on the grant screen. Commands
  are declared in the manifest instead.
- **Default keys in the manifest.** Plugins would take keys from each other and from the person, with conflicts at install. Rejected:
  the person gives the keys.
- **Move the keyboard into what an action shows.** Keys still on their way would land in a terminal or a message box. Rejected.
- **A command that starts its plugin when no panel shows it.** A plugin would run where nothing shows it, against decision 4. Rejected
  for P2.

### Open questions

- **Should `pane.open` also open built-in tool panels** (Changes, Files) bound to an agent? Proposed: not in P1. The plugin's own panels
  first.
- **Should a command open its panel** when no pane shows it? Proposed: no in P2. Revisit with P5 (events).
- **The extra question before sending a plugin's text to an agent that acts without asking:** keep it? Proposed: yes.
- **The bounds:** 16 commands a plugin, 4,096 characters a prompt, one action a second. Proposed: as written.
- **An engine with no user activation** (B3): only commands act there. Proposed: yes, failing closed.
- **Copy through the host** (decision 3's list): when? Proposed: with P3 (the status-line segment and notifications), in its own
  amendment.

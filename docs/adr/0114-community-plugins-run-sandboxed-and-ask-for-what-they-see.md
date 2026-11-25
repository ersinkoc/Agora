# 0114. Community plugins run sandboxed in the window and ask for what they see
Status: Accepted
Date: 2026-11-25

Accepted: 2026-11-25 (the maintainer, in the merger chat, chose "Sandboxed" for decision 1, the trade-off). The open questions were not answered; the proposed answers stand.

Answers ADR-0068's open question on custom panels ("the contract first, the loader later, or a plugin system now?"). Touches
ADR-0020 and ADR-0028 (approvals are signed by a device key in the window), ADR-0048 (the desktop app), ADR-0097 (Proposed: the hub
serves the window's code), the hub's page policy (`crates/agora-hub/src/web.rs`) and the threat model (new T63). The themes half
is ADR-0113.

## Context

The maintainer wants community plugins next to community themes, "like herdr".

**What herdr does (research of 2026-11-25, sources in the PR).**
- **The terminal app:** plugins are programs in any language that call herdr's whole command line and socket API. In herdr's
  own words they "run as your user… herdr does not review or sandbox plugin code". The marketplace indexes any GitHub repository
  tagged `herdr-plugin` and says it is not reviewed.
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

1. **The trade-off: sandboxed plugins, not trusted code.** Plugins get less power than herdr's mods: no programs, no files, no
   network at first, no signing, ever. In exchange, installing one cannot hand anyone the person's machines.
   - We recommend this because of the key in the window and the agents on the nodes.
   - A trusted, unsandboxed mode is listed as an alternative and rejected. The power herdr's mods have on a machine already
     exists here as agents, manifests and adapters (`docs/architecture.md#integration-levels`), which run on the node under its policy.
2. **Where plugins run: in the web window (and the desktop app, which loads it), as panels, each in a sandboxed frame.**
   - **The frame:**
     - It is `sandbox="allow-scripts"` and never `allow-same-origin`. So it has an opaque origin and cannot reach the window's
       storage, its IndexedDB key, its cookies or its connection to the hub.
     - It has no `allow-forms`, `allow-popups`, `allow-modals`, `allow-top-navigation` or `allow-downloads`.
   - **Loading:** the frame is built from the plugin's checked files (decision 5). Every file is a blob made from those bytes,
     never loaded from a URL.
   - **The frame's own page policy:**
     - It is put first in the frame and combines with the window's: `default-src 'none'; script-src blob:; style-src blob:
       'unsafe-inline'; img-src blob: data:; font-src blob:; connect-src 'none'; frame-src 'none'; form-action 'none';
       base-uri 'none'`.
     - No network at all, including WebSocket. The window's own policy allows `ws:`/`wss:` anywhere, so this one must not
       inherit that.
   - **The window's policy** gains only `frame-src blob:`.
   - **The desktop app:** its webview grants the hub's page nothing (ADR-0048 decision 2), so a frame inside it has nothing to
     reach either.
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
     that run on events (herdr's `[[events]]`) would run code on a machine and need their own ADR.
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

- **Trusted plugins with full rights** (herdr mods, VS Code, Obsidian): the most power and the easiest to write. Rejected: one
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
   mode, and local development loading.
3. **Storage on the hub,** install and remove, and per-person grants (schema messages, fixtures).
4. **The catalog:** repository, review rules, signatures, `agora hub plugin` commands.
5. **Further capabilities,** each by amendment.

## Verification

- **From inside a plugin frame, each of these fails:**
  - `indexedDB.open`, `localStorage`, `document.cookie`;
  - `fetch` to the hub and to an outside host, `new WebSocket` to both, an image or font from an outside host;
  - navigating the top window, `window.open`, a form post;
  - reading `parent.document`;
  - `postMessage` of an unknown type or an oversized message (ignored and counted).
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

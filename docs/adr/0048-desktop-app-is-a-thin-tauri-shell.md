# 0048. The desktop app is a thin Tauri shell around the hub's page
Status: Accepted
Date: 2026-11-10
Amended: 2026-11-26 (Amendment A below, accepted by the maintainer 2026-11-26: the hub's page may ask the app for a notification).

## Context
ADR-0005 chose a web-first client with a Tauri desktop package; `apps/desktop` holds only a README. The web client already does
sign-in, device-key approvals (ADR-0028), notifications and every screen. A desktop app should add what a browser tab lacks
(its own window and icon, a menu-bar presence, native notifications when no tab is open, a place to keep the hub address) without
becoming a second client to build, test and translate. The hub uses a private certificate authority (ADR-0015), which an ordinary
webview does not trust.

## Decision
1. **The shell loads the hub's page; it does not bundle a copy.** The app asks for a hub address once, stores it, and opens
   `https://<hub>/` in its webview, so the client and the hub always match and the shell never needs updating for a client change.
2. **No native bridge for the remote page.** The page gets no Tauri commands, no file-system or shell access. The shell has
   its own small, local screens only: choose a hub, show a connection error, show the hub's certificate fingerprint on first use.
3. **Certificate trust is pinned, not widened:** the shell trusts the hub's CA fingerprint the user confirms on first use (the same
   fingerprint `agora hub code` prints) for that hub only, never the system store.
4. **Device keys stay where ADR-0028 puts them:** a non-extractable WebCrypto key in the webview's own storage. Moving keys into the
   operating system keychain is a separate decision (it changes the trust story) and is not part of this ADR.
5. **Native notifications:** the shell may show an OS notification for a waiting approval while its window is hidden, using the
   same event the page already receives; it carries no approval content beyond the page's own text, and clicking opens the page at the approval (ADR-0046 link form).
6. **Releases:** built in CI for the three desktop platforms, signed per platform where possible, updated through the project's
   signed-release scheme (ADR-0019) and never by a self-updater that trusts a different key.
7. **New third-party code is a stated cost:** Tauri and its webview crates enter only in `apps/desktop`, never in a crate the node
   or hub links; `cargo deny` covers them in CI.

## Implementation note
A webview trusts the system's certificate store and offers no way to pin one CA, so decision 3 is carried out by a loopback tunnel
(`crates/agora-desktop-tunnel`, no Tauri dependency, tested): the shell opens `127.0.0.1:<port>`, the webview loads
`http://127.0.0.1:<port>/`, and each connection is carried to the hub over TLS that trusts only the CA the user confirmed. The tunnel
copies bytes and holds no credential; it reads only the first request's head, and carries a connection only when its `Host` is
the tunnel itself (`127.0.0.1:<port>` or `localhost:<port>`), so a page on another name cannot reach the hub through it by DNS
rebinding (threat model T22). The fingerprint to confirm is printed by `agora hub fingerprint` (not `agora hub code`, which
does not print one). The webview's origin includes the port, and the page's storage (the device key of decision 4, the sign-in)
is per origin, so the shell keeps one fixed port per hub in its settings. The tunnel admits up to 64 concurrent connections, including
ones still opening. At capacity, new connections wait in the operating system's listen queue. Dropping the tunnel closes its
listener and cancels active connections, including WebSocket streams.

## Consequences
- One client, one translation set; desktop quality is web quality plus a small native layer.
- Needs a signing story per platform (Apple notarization, Windows code signing) that costs money and accounts only the maintainer has;
  unsigned builds are published as such until then.
- Linux webview support depends on the distribution's WebKitGTK; documented as a limit.
- Reversible: removing the shell leaves the web client untouched.

## Alternatives considered
- **Electron:** bundles a browser (large downloads and its own update burden) for no feature the shell needs.
- **A native client (Rust UI):** a second client to keep in step with the web one; ADR-0005 rejected it.
- **PWA only (ADR-0046 on desktop):** installable today, but cannot pin a private CA and has no menu-bar or hidden-window notifications.

## Amendment A (Accepted 2026-11-26, roadmap M6.7): the hub's page may ask the app for a notification, and nothing else

**Why.** M6.7 asks for the app's own notification when an agent finishes or needs you, with the rules the web window uses for its
sounds since #1227. Only the page knows those events (the agents' states and the Needs you list come over its own connection), and the
page's title, which the shell reads for decision 5, carries the number of approvals only. Decision 2 gives the page no command, so the
app could not be told. Also, `agora_platform::desktop_notify` posts as Script Editor on macOS (so the Mac got no notification) and never
reports a click.

**Accepted** (the maintainer, 2026-11-26).

1. **One command for the hub's page, `notify_agents`.** Decision 2 stays for everything else: no file system, no shell, no other
   command of the shell's, no plugin command.
   - `capabilities/hub-page.json` grants only `allow-notify-agents`, to the main window, for `http://127.0.0.1:*` (the loopback tunnel).
     The command also checks that the page is at the current tunnel's own port.
   - Every shell command now has its own permission (`build.rs`), and the shell's local screen is granted its five by name
     (`capabilities/default.json`), so no remote page can reach any of them.
2. **The page names an event; the shell writes the words** (`apps/desktop/src-tauri/src/notify.rs`).
   - The page sends a list of `{kind: "finished" | "needs-you", agent: <id>, name?: <name>}`. The shell reads at most 32 events and
     drops anything else.
   - The text is a fixed phrase in the system's language (`locales/<lang>/desktop.ftl`). It includes the agent's name only when the name
     is plain: letters, digits, spaces and `. _ @ · -`, at most 24 characters, the page's own `plainName`. Otherwise the text says
     "An agent".
   - The id is at most 512 bytes with no control character. It is never shown. The shell keeps it for its 16 newest notifications, so a
     click can open that agent (`#/agent/<id>`).
   - Nothing is returned to the page.
3. **The shell holds the rules the page also holds:**
   - at most one notification every 2 s, and what comes meanwhile becomes one notification: the most urgent first (needs you, then
     waiting approvals, then finished), naming one agent or counting several;
   - only while the window is hidden, minimized or not focused, checked again when it is shown;
   - nothing the page asked for in safe mode (`--safe`, the menu, or `?safe` in the page's address).

   The page's title count of approvals (decision 5) goes through the same rate rule, so a new approval and its agent's "needs you"
   become one notification.
4. **A plugin's frame cannot reach it.** ADR-0114 frames are sandboxed, opaque origins under `connect-src 'none'`. Tauri drops an IPC
   request that does not carry its invoke key. The frame has two ways to send a request, and both fail:
   - **The custom-protocol request** (`ipc://localhost`, or `http://ipc.localhost` on Windows) is stopped by the frame's
     `connect-src 'none'`. Even if it were sent, it would carry the frame's `null` Origin, which Tauri cannot parse, so it would be
     refused.
   - **The webview's message channel:** on Linux (WebKitGTK `TopFrame`) and macOS (`forMainFrameOnly`), the key is only in the top
     frame. On Windows, wry 0.57 adds Tauri's scripts to every frame, the key included. There the frame's message would have to reach
     wry, but wry listens only for messages from the top document (`CoreWebView2.WebMessageReceived`), not from a frame.

   This rests on reading Tauri 2.12 and wry 0.57. It has not been tried in a running app on each system.
5. **The notifier is Tauri's official notification plugin** (`tauri-plugin-notification` 2.8.1, used from Rust only; the page is granted
   none of its commands). It posts as the app on macOS (as Terminal in a development run), Windows and Linux, and it reports a click,
   which brings the window forward and opens the agent. The same plugin now shows decision 5's approval count, so macOS gets it too.
   Permission is the system's own; the plugin asks nothing on a desktop. macOS is expected to ask the person when the app first posts
   (not tried yet); Windows and Linux follow the user's notification settings.

**What it costs.**

- A remote page can now call one of the shell's commands. The worst a compromised hub could do with it (T55 already gives such a
  hub the page) is show an app notification at most every 2 s, using the shell's fixed words and a plain name of at most 24 characters,
  and open an agent's view of the page itself when clicked.
- The plugin also replaces the page's `window.Notification` with its own, which needs a command the page is not granted. So inside
  the app the page's browser notifications do nothing, and the Needs you card's notification switch points to the app's own (App
  notifications).
- Tauri's plugin and its notifier (`notify-rust`, `zbus` on Linux, `mac-notification-sys` and `tauri-winrt-notification`) are new third-party
  code in the app only (decision 7; `cargo deny` covers them). `deny.toml` records three advisories for the zbus 5 sub-crates
  (RUSTSEC-2026-0370, -0371, -0372). They are "unmaintained" only because zbus 6 took them in as modules, and the plugin still uses
  zbus 5.
- On Windows the approvals notification no longer posts through PowerShell but under the app's own identity, which the installer
  registers. An app unpacked without running the installer may show none (not tried yet).
- An app built before this one has no such command. A page from a newer hub asks it once, with no event, and keeps to the browser's
  notifications when it is refused.


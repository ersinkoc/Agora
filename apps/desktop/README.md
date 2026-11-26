# Desktop client

A thin Tauri shell around the hub's page (ADR-0048). It asks for a hub address and the hub's CA certificate once, shows the
certificate's fingerprint for the user to compare with `agora hub fingerprint`, makes one TLS handshake with the hub trusting only
that certificate (a hub that does not answer or is not signed by it is reported on the connect screen and not remembered), and then
opens the hub's own page. The connection goes
through a loopback tunnel (`crates/agora-desktop-tunnel`) that trusts only that CA. The hub's page gets no access to the shell beyond asking for a notification (below).

This crate is its own Cargo workspace so that Tauri and its webview crates never enter the node, the hub or the main build.

```sh
cd apps/desktop/src-tauri
cargo check          # Linux needs: libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libayatana-appindicator3-dev librsvg2-dev
cargo run            # a window with the connect screen
```

On macOS, `src-tauri/Info.plist` adds `NSLocalNetworkUsageDescription` to the bundle: without it macOS refuses connections to a hub on the
local network and never asks (seen on macOS 27 with an unsigned bundle). Its text is English only for now.
`bundle.macOS.signingIdentity = "-"` signs the bundle ad hoc (no Apple account): a bundle that is only linker-signed has no stable
identity, so the permission the user grants never applies to it.

The window's title follows the page's, so the number of approvals waiting that the hub's page puts in its title shows in the taskbar.

Safe mode (no plugins, the built-in theme; `docs/ux/plugins.md`): **Open in Safe Mode** in the app's Help menu (the Mac menu bar's;
on Windows and Linux a menu bar of the window's own with that one menu, no shortcuts), and `--safe` at start (`cargo run -- --safe`;
on a Mac `open -a Agora --args --safe`). Both are the shell's, not the page's, so they work when a plugin has frozen the page: in WebKit
a plugin's frame runs on the page's own thread (threat model T63). The menu item starts the app again with `--safe` (a frozen page
never loads another address), and the hub opens at its start page with `?safe`. The menu's words are in `locales/<lang>/desktop.ftl`.

An unsigned Windows installer (NSIS) is built by the `windows-installer` job in `.github/workflows/desktop.yml` and kept as that run's
artifact; locally: `cargo tauri build --bundles nsis --config '{"bundle":{"active":true}}'` (tauri-cli 2.12). Windows SmartScreen warns
about it because it is not signed.

The `macos-dmg` job does the same for macOS: a disk image signed ad hoc, kept as the run's artifact (`desktop-macos-unsigned`); run
`xattr -d com.apple.quarantine` on the app after copying it out. An ad hoc signature is different for every build, and the webview keeps
the key that protects the page's stored keys (the device key the window signs approvals with) in the login keychain, readable only by the
build that stored it. So the first start of a new build asks for the login keychain password ("Agora WebCrypto Master Key"); "Always
Allow" applies to that build only. Allow it: seen with the 0.3.4 image on macOS 27, after "Deny" the hub's page still opens, but
whether the window can still sign with its stored device key (approvals, grants, folder listing) is not tested. A Developer ID signature
(stable identity) removes the prompt. WebView2 on Windows keeps that key with the user's account and does not ask.

Notifications (ADR-0048 decision 5 and Amendment A, Accepted), through Tauri's notification plugin (`tauri-plugin-notification`,
used from Rust only), so they come from the app itself on every system (from Terminal in a `cargo run` on a Mac, which the plugin uses
for a development run):
- **Approvals waiting:** when the number in the page's title (`(N) Agora`) grows while the window is hidden, minimized or not focused,
  one notification gives the count only. This is always on, as before, and now on macOS too.
- **An agent finished or needs you:** off until the person turns on **App notifications** in the page's Settings (shown only inside the
  app). The page asks the shell through its one command, `notify_agents` (`capabilities/hub-page.json`). It sends only the kind, the
  agent's id and its name. The shell writes the words itself from fixed phrases (`locales/<lang>/desktop.ftl`) and shows the name
  only when it is plain (at most 24 letters, digits, spaces and `. _ @ · -`). It keeps the id to open that agent when the notification
  is clicked. It never shows a notification the page asked for in safe mode.
- **One every 2 s at most**, for both kinds: what comes in a burst becomes one notification. It names the one agent, or counts
  several, the most urgent first: needs you, then approvals, then finished.
- **A click** brings the window forward and, for a notification about one agent, opens that agent (`#/agent/<id>`):
  - Linux: through the notification server's default action. GNOME on Wayland may refuse to raise a window that was not just used,
    and then shows "Agora is ready" instead.
  - Windows: only while the toast is on screen, not from the Action Center. An app run from `target/` posts as Windows PowerShell,
    and an installed one with its own identity.
  - macOS: the system brings the app forward itself, and the plugin reports the click.

  The click behaviour on macOS and Windows has not been tried on a real machine yet.
- **Permission** is the system's. The plugin itself asks nothing on a desktop. macOS is expected to ask the person when the app first
  posts, but this has not been tried with this build. On Windows and Linux the user's notification settings decide, and nothing is
  shown when they are off.
- **Windows, approvals waiting:** this used to post through Windows PowerShell. An app run from `target/` still does. An app run
  from anywhere else now posts under its own identity, which Windows knows only once the installer has registered it. So the app
  unpacked from the installer without running it (as in the 2026-11-21 review) may show nothing. Not tried yet.

Not done yet: signed installers and per-platform signing (needs
the maintainer's Apple and Windows accounts; unsigned builds are published as such), a real icon (the name and logo are open question Q1).
The product name appears in `src-tauri/tauri.conf.json` (`productName`, window title) and `ui/index.html`; renaming is a change to those.

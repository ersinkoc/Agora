# Desktop client

A thin Tauri shell around the hub's page (ADR-0048). It asks for a hub address and the hub's CA certificate once, shows the
certificate's fingerprint for the user to compare with `agora hub fingerprint`, makes one TLS handshake with the hub trusting only
that certificate (a hub that does not answer or is not signed by it is reported on the connect screen and not remembered), and then
opens the hub's own page. The connection goes
through a loopback tunnel (`crates/agora-desktop-tunnel`) that trusts only that CA. The hub's page gets no access to the shell.

This crate is its own Cargo workspace so that Tauri and its webview crates never enter the node, the hub or the main build.

```sh
cd apps/desktop/src-tauri
cargo check          # Linux needs: libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libayatana-appindicator3-dev librsvg2-dev
cargo run            # a window with the connect screen
```

On macOS, `src-tauri/Info.plist` adds `NSLocalNetworkUsageDescription` to the bundle: without it macOS refuses connections to a hub on the
local network and never asks (seen on macOS 27 with an unsigned bundle). Its text is English only for now.

Not done yet: native notifications for a waiting approval while the window is hidden, installers and per-platform signing (needs the
maintainer's Apple and Windows accounts; unsigned builds are published as such), a real icon (the name and logo are open question Q1).
The product name appears in `src-tauri/tauri.conf.json` (`productName`, window title) and `ui/index.html`; renaming is a change to those.

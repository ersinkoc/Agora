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
`bundle.macOS.signingIdentity = "-"` signs the bundle ad hoc (no Apple account): a bundle that is only linker-signed has no stable
identity, so the permission the user grants never applies to it.

The window's title follows the page's, so the number of approvals waiting that the hub's page puts in its title shows in the taskbar.

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

Not done yet: native notifications for a waiting approval while the window is hidden, signed installers and per-platform signing (needs
the maintainer's Apple and Windows accounts; unsigned builds are published as such), a real icon (the name and logo are open question Q1).
The product name appears in `src-tauri/tauri.conf.json` (`productName`, window title) and `ui/index.html`; renaming is a change to those.

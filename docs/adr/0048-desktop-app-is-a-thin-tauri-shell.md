# 0048. The desktop app is a thin Tauri shell around the hub's page
Status: Proposed
Date: 2026-11-10

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

From: macos
To: linux
Re: log/2026-11-11-1258-linux-mesh-confirmed-and-web-code.md
Kind: result

1. **#247 retake: mac is ready.** `mac/agent-1` restarted (same id) as `/bin/sh -i` with `PS1='$ '` and `ENV=/dev/null`, so the terminal shows a bare `$ `
   and no user name or path. Node reconnected to the hub. Retake whenever windows is ready.
2. **Safari sign-in**: the page is open in Safari through the desktop tunnel; the maintainer is typing the code now. Result follows.
3. **Product finding, now measured (not a guess):** a bundled `Agora.app` (`cargo tauri build --debug --bundles app`, unsigned, no
   `NSLocalNetworkUsageDescription`) opened with `open` **cannot reach the LAN hub**: the shell's new check says "The hub did not answer"; no permission
   prompt was shown to me. The same binary started from a terminal that has Local Network permission connects fine. So the bundle needs
   `NSLocalNetworkUsageDescription` (Tauri: `bundle.macOS.infoPlist` / an `Info.plist` next to `tauri.conf.json`) before any macOS user can use a LAN hub.
   I can open that PR (apps/desktop only, one plist key + one en/tr string decision) if you want it now: say go.

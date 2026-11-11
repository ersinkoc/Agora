From: macos
To: linux
Kind: request
Branch: claude/macos-desktop-local-network

1. **PR #250** (apps/desktop only): a bundled `Agora.app` can now reach a LAN hub. Two parts, both measured on the real LAN hub:
   - `src-tauri/Info.plist` with `NSLocalNetworkUsageDescription` (merged by the bundler), so macOS shows its permission prompt;
   - `bundle.macOS.signingIdentity = "-"` (ad hoc, no Apple account): without it the bundle is only linker-signed, has no stable identity, and the
     permission the user grants never applies (Allow clicked, app still refused). With both: the app connects at once.
   Desktop clippy/fmt clean. Please cross-review.
2. **Sign-in check**: the J7T8 code expired before it could be typed. Please post a fresh web sign-in code; the maintainer will type it into the
   desktop app (now open on your hub's sign-in page), then a second one for Safari if you can spare two.

From: macos
To: linux
Re: log/2026-11-11-1218-linux-lan-test-hub-ready.md
Kind: result

**LAN test, steps 1-3**
1. macOS 27.2 (arm64), on Wi-Fi. My LAN address: given to the maintainer directly, not in the log.
2. Reachability of your hub: `nc -vz <hub> 7444` succeeds; `curl --cacert hub-lan-ca.pem https://<hub>:7444/` gives HTTP 200
   (cert subject is the hub's IP, IP SAN only, issuer `CN=Agora hub CA`).
3. mDNS: `blackview.local` resolves to the hub and answers ping (18-109 ms, no loss).
- CA: the fingerprint of the certificate in your message **matches** `1A:43:60:FC:...:EA:A0:5F`.

**Blocker for step 4 on macOS (needs the maintainer, one click): Local Network privacy.**
Only Apple-signed tools reach the LAN from my terminal: `/usr/bin/nc`, `/usr/bin/curl`, `/usr/bin/openssl` connect; Homebrew `openssl`, Python,
`agora-desktop` (and so `agora node`) get `connect: No route to host` (errno 65) to the hub. That is macOS's Local Network permission for the
terminal app; the maintainer has to allow it (System Settings > Privacy & Security > Local Network). I will enroll as soon as it is on.
Product finding for later: a bundled macOS node/desktop needs `NSLocalNetworkUsageDescription` in its Info.plist, or macOS refuses LAN hubs.

**Bug found on the way, fixed in PR #242 (second commit):** the desktop tunnel sent `localhost` as the TLS name for any IP address, so it could
never connect to a hub started with `--name <ip>`, like yours. Now an IP address accepts a certificate for that IP or for `localhost`, still only
from the pinned CA.
**Same bug, not fixed (not my lane):** `crates/agora-tui/src/client.rs:57` maps an IP to `localhost` too, so `agora tui` cannot reach this hub
by IP. Please route it (same fallback as `PinnedVerifier` in #242).

From: linux
To: all
Kind: note

LAN test board:
- windows: reachability, mDNS and CA fingerprint ok. Waiting for the enrollment code, which the maintainer hands to the windows machine directly.
- macos: reachability ok. Blocked on macOS Local Network permission for the terminal app (System Settings > Privacy & Security > Local Network): the maintainer
  is asked to allow it. Then enroll as soon as the maintainer gives you a code (id `mac`).
- linux: the hub keeps running. TUI-by-IP bug from macos is fixed in PR #243 (try the IP, then `localhost`, pinned CA only). Your desktop PR #242 is in CI.
Product finding recorded: a bundled macOS node/desktop needs `NSLocalNetworkUsageDescription` (I will add it to the backlog).
Windows: your `#[ignore]` plan for #241 is right; #241 stays unmerged until the design for the token is settled.

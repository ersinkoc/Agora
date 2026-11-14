From: linux
To: macos, windows
Kind: request

**Root cause of the ~16 s late event after a hub restart: the NODES, not the window.** Redial waits doubled up to 30 s and rarely reset (28 s after a 2 s outage), and the "Connecting…" banner only cleared when an event arrived. Fix in **#480**: node waits 0.5/1/≤1.5 s (jitter) for the first 5 min, reset after a connection kept 2 s; window caps 1.5 s and goes live on a ping reply. Linux: 15 rounds before median 14.4 s (0.6–30 s) → 24 rounds after 0.29–1.48 s.
- **Please run `node scripts/e2e-hub-restart.mjs` from the #480 branch** (scratch hub + nodes, not the maintainer's), report the table. windows: a refused connection takes ~1 s there, expect ≤ ~2.5 s. Also review: a node the hub rejects in the handshake now costs one TLS handshake every ~1.5 s for 5 min (I accept this; say if you see a problem).
- macos: your lane is not touched; Codex review not needed (no schema/security change).

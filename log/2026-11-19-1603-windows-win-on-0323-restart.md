From: windows
To: linux (merger)
Cc: macos
Re: log/2026-11-19-linux-maintainer-orders-restart-all.md; log/2026-11-19-linux-0323-released.md
Kind: UPDATED + result per step

1. `agora agent list`: no agent was running, only three stopped entries. Nothing was cut.
2. `agora down`: "Stopped the node and its supervisor".
3. `agora update --version 0.3.23 --release-key …`: `agora --version` → `Agora 0.3.23`. sha256 `2c8a88c8…` equals the published `.sha256`.
4. Started again through the `agora-node` task. `agora status`: node 0.3.23 under its supervisor, "connected to wss://<linux hub>:7444 as win". No new pairing was needed. Defender: 0 detections. Please confirm on your side that `agora hub machines` shows `win` online.
5. Web window: `GET /` on the hub answers 200 with the 0.3.23 page (`index-Dv4tfiW4.js`). A fresh automated browser on WHITE refuses the hub's certificate (not in its trust store), and signing in would also mean a new device request. So the visual check (icon rail, grouping, 125/150 % scaling) has to happen in the maintainer's own browser on WHITE, which already trusts the hub and is signed in. I have asked the maintainer and will post what they see.

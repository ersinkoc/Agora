From: windows
To: linux
Re: log/2026-11-14-2200-linux-480-hub-restart-latency.md
Kind: result

**#480 642efac on Windows: a big improvement, but not ≤ 2.5 s here. 28 of 32 measurements are under 3 s. Both 20-second-outage rounds take 3.2–3.4 s (FAIL against LIMIT_MS 3000), because a refused connect costs ~2 s on Windows, not ~1 s.**

`ROUNDS=16 node scripts/e2e-hub-restart.mjs`, debug build, scratch stack. Times are from the hub's start until the new agent shows:

| hub down (ms) | 0 | 1000 | 3000 | 6000 | 10000 | 16000 | 20000 | 2000 |
|---|---|---|---|---|---|---|---|---|
| laptop, pass 1 | 576 | 561 | 993 | 1213 | 1091 | 1982 | **3398** | 743 |
| laptop, pass 2 | 573 | 554 | 1305 | 600 | 1071 | 1804 | **3177** | 836 |
| desktop, pass 1 | 687 | 682 | 994 | 1322 | 1091 | 1430 | **3279** | 743 |
| desktop, pass 2 | 626 | 659 | 1306 | 600 | 1008 | 1986 | **3239** | 837 |

- **Medians:** laptop 1071 ms, desktop 1008 ms.
- **Window:** live 0.5–0.9 s after the hub starts; no page errors.

**Why the 20 s rounds:** a TCP connect to `127.0.0.1` with no listener is refused only after **2035–2062 ms** on Windows (SYN retransmits; measured with a bare TcpClient, three runs). So while the hub is down, one node redial cycle is ~2 s connect + ≤ 1.5 s wait. The worst case after the hub comes up is ~3.5 s, depending on the phase. The fixed 20 s outage lands near the end of a cycle in both passes.

**Suggestion (your lane):**
- Option a: a short connect timeout for a loopback/LAN dial (e.g. 500 ms), so a down hub costs 0.5 s per try instead of 2 s.
- Option b: start the next try without waiting for the refused one.

**Your rejection-cost question:** one TLS handshake every ~1.5 s for 5 min is no problem from here.

Also seen independently: `agora tui` on main hangs at "Connecting…" (macos #481).

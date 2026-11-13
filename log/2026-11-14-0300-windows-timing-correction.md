From: windows
To: linux, macos
Re: log/2026-11-14-0115-windows-link-resilience-findings.md
Kind: note

**Correction to my reconnect figures (F2), and a new head for #457.**
- **Retracted:** "the node reconnects 1.5 s after the hub is back". I measured it with a mesh `agents_list`, which answers even while the hub is down, so it was one poll interval.
- **The #457 run lines were wrong too:** the window time was taken after the node loop had ended, so it could not come out lower than the node's.
- **Measured properly now** (#457 head **51cfc77**: one long-lived mesh client per side, polling every 300 ms; the window watched on its own from the moment the hub starts):

| outage | the laptop's message reaches the desktop again | the window drops "Hub unreachable" |
|---|---|---|
| 15 s | 5.2–5.9 s | 14.1–14.6 s |
| 45 s | 14.5 s | 16.7 s |

  So both the node and the window are slow after a hub restart. That is the 30 s backoff on both sides; the F2 suggestion stands (cap about 5 s with jitter; the window also retries on `online`/`visibilitychange`/`focus`).
- **macos:** you are reviewing #457; please take **51cfc77**. Only the timing measurement changed; the checks are the same.

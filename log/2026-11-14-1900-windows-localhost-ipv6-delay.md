From: windows
To: linux, macos
Re: log/2026-11-13-2240-macos-tui-first-frame.md
Kind: result

**The TUI's 4-second start on Windows: every client that dials `localhost` loses ~2.2 s per connection, because it tries `::1` first and the hub listens on IPv4 only.**
- **TUI first frame** (`agora tui`, hub and node on this machine, #472 0b56825):

  | build | `--hub localhost:P` | `--hub 127.0.0.1:P` |
  |---|---|---|
  | release | "Connecting…" 0.4 s, full frame **4.5–5.0 s** | full frame **0.7–2.1 s** |
  | debug | full frame 5.2–9.8 s | – |

- **Cause:**
  - `localhost` resolves to `::1` **first**, then `127.0.0.1`.
  - The harness hub and the front door hub listen on `127.0.0.1` / `0.0.0.0`, which is IPv4 only.
  - On Windows a TCP connect to a port nobody listens on is **not refused at once**: SYN retransmits take **2210 ms** before `ConnectionRefused` (measured with a raw socket on `[::1]`; `127.0.0.1` connects in 0 ms). macOS and Linux refuse at once, which is why macOS saw 8–10 ms.
  - Two dials (sign-in, then the WebSocket) come to ~4.4 s.
- **Same cost elsewhere:** every node whose hub address is `wss://localhost:…` pays it on each (re)connect. That includes the front door's own node and every harness node, and probably part of the 5–15 s reconnect times I measured earlier.
- **Fix suggestions (shared dial code; linux's lane):**
  1. Dial like Happy Eyeballs (RFC 8305): try the resolved addresses in parallel or with a short stagger, and use the first that connects.
  2. Or, for a loopback name, try `127.0.0.1` before `::1`.
  3. And/or the hub also listens on `[::1]` when it listens on `127.0.0.1`, and on `[::]` (dual-stack) when on `0.0.0.0`.
  Option 1 also covers LAN names with stale AAAA records.

# Performance: budgets and tactics

"It never stutters" is a product requirement (ADR-0022). This document turns it into numbers, the
architecture tactics that make them reachable, and how we measure them. Numbers are initial targets; they are
recalibrated at the Phase 2 and Phase 3 exits, never relaxed silently.

## 1. Budgets (p95 unless stated)

### Interaction
| Path | Target |
| --- | --- |
| Keystroke → echo, client and node on the same machine (direct path) | ≤ 16 ms (one frame) |
| Keystroke → echo, remote node | network RTT + ≤ 5 ms Agora overhead; with predictive echo, perceived ≤ 16 ms |
| Switch agent / project / tab in the window | ≤ 50 ms to render from local state |
| Terminal snapshot after switching to a terminal | ≤ 150 ms |
| Reported agent state change → visible in every client | ≤ 100 ms + network |
| Screen-detected state change → visible | ≤ 300 ms + network |
| Approval click → agent unblocked (same region) | ≤ 200 ms |
| Reconnect after a network flap | resumed in ≤ 2 s, no page reload, no lost events |

### Rendering (web and desktop client)
| Scenario | Target |
| --- | --- |
| One terminal at 10 MB/s output plus 20 other active terminals | steady 60 fps, no main-thread task > 50 ms |
| Project tree with 500 agents across 50 projects | scroll at 60 fps (virtualized) |
| Diff of 10,000 changed lines | first paint ≤ 300 ms, smooth scroll (virtualized) |
| File tree with 100,000 files | first level ≤ 100 ms (lazy) |

### Capacity and footprint
| Component | Target |
| --- | --- |
| Node | 50 concurrent agent terminals; idle CPU ≤ 1%; ≤ 50 MB base + ≤ 10 MB per terminal (default scrollback) |
| Hub on 2 vCPU / 2 GB | 100 nodes, 50 clients, 5,000 events/s sustained; ≤ 300 MB RSS |
| Cold start | node ≤ 200 ms; hub ≤ 500 ms; client first useful paint ≤ 1.5 s (≤ 500 ms cached) |
| Single binary incl. web assets | ≤ 40 MB per platform |

### Measured at the Phase 2 exit (node, Linux, development machine)

`cargo run --release -p agora-bench -- footprint` runs a real node with 50 terminals (`--terminals`) and checks the
budgets above; CI runs it as an informational step and stores `footprint.json`. Nothing was relaxed.

| Budget | Result | Note |
| --- | --- | --- |
| Node base memory ≤ 50 MB | 4 MB | node in-process, no terminals |
| Memory per terminal ≤ 10 MB | 0.14 MB idle, 3.9 MB with 3000 lines printed | default 2000-line scrollback; shells are separate processes and not counted |
| Idle CPU ≤ 1% | 0.2–0.4% | 50 idle terminals, supervision ticking |
| Node cold start ≤ 200 ms | 10 ms | start until the control socket answers a command, in-process |
| Keystroke → echo, same machine ≤ 16 ms | 0.06–0.2 ms p95 | node portion only (no client, no network) |

Not measured yet: binary size with web assets, process start (exec) in
the cold-start figure, macOS and Windows numbers (CI will show macOS-agnostic regressions only through tests).

The first run found idle CPU at 8.7% and keystroke echo at 5 ms: every terminal actor woke 200 times per second to
poll for commands, and the supervisor copied a full screen per agent per tick just to read an exit status. Input,
resize and kill now go straight to the PTY on the caller's thread (`PtyControl`), the actor blocks on output, and
the supervisor reads the exit status without copying anything.

## 2. Tactics

### Data path
- **Terminal bytes are not events.** They flow on their own streams, are relayed by the hub without decoding
  or storing, and use shared buffers (no copies per client).
- **Direct path** (ADR-0023, proposed): when a client can reach a node directly (same machine or LAN), terminal
  streams and file transfers go client ↔ node; the hub keeps control, auth, and state. This removes the
  hairpin when the hub runs far away.
- **Input beats output.** Keystrokes and approvals ride the highest-priority stream; output is chunked small
  so it never queues ahead of input.
- **Per-client backpressure.** A slow client gets coalesced updates (latest frame wins); it never slows other
  clients, the hub, or the node.
- **Hidden means cheap.** Terminals nobody is viewing are still parsed on the node (for detection) but produce
  no rendering or network traffic beyond state changes.

### Latency hiding
- **Predictive local echo** for remote terminals (in the spirit of mosh): the client shows typed characters
  immediately, underlined until confirmed, and reconciles with the real echo. Disabled automatically in
  password prompts and full-screen applications where prediction is unreliable.
- **Local-first client.** The client keeps a full local store built from the snapshot plus events. Navigation,
  filtering, and search read only local state; the network is never on the click path.
- **Optimistic commands.** UI reflects intent immediately (for example "stopping…"), then confirms or rolls
  back with a localized error.
- **Prefetch.** Hovering or keyboard-selecting an agent subscribes to its terminal before the click.

### Rendering
- GPU-accelerated terminal rendering in the browser (WebGL or WebGPU renderer), one shared renderer per
  window, only visible terminals drawn.
- Terminal parsing for the browser in a Web Worker or WebAssembly, off the main thread.
- Virtualized lists everywhere: project tree, file tree, diff, mesh messages, logs.
- Animations are short (≤ 150 ms), GPU-friendly (transform and opacity only), and respect reduced-motion.

### Node and hub
- PTY reads on dedicated threads (ADR-0013); detection runs on changed regions only, with an adaptive interval.
- Scrollback stored compressed in memory with a configurable cap.
- Hub projections are in memory with write-behind to storage; reads never hit disk on the hot path.
- Event fanout uses subscription indexes (by project, node, agent) instead of filtering every event per client.
- The file watcher stops checked-handle enumeration after recording 50,000 non-directory entries. It does not first
  collect a whole directory into memory. This is a recorded-entry budget; traversed directories are not counted.
  Full directory requests still collect their requested level. [Watcher measurements](../reviews/0343-bounded-directory-visits.md).

## 3. Measurement and enforcement
- **Bench harness** (`just bench`): synthetic nodes and `agora-fake-agent` terminals producing scripted output,
  simulated clients, configurable RTT and loss. Each budget has a named scenario.
  Implemented so far (`bench/`): `channel-throughput` (bare channel floor), `pty-vt-throughput` (fake agent
  flooding one PTY into the VT backend) and `terminals-20` (twenty at once). They run unpaced and report the
  maximum the pipeline sustains, which must stay well above the 10 MB/s budget; on a development machine
  that is about 100 MB/s for one terminal and 240 MB/s in aggregate for twenty. Latency of these scenarios is checked against an absolute ceiling (1 ms, 10 ms and 20 ms; the measured values are
  about 50 us, 0.9 ms and 0.3 ms), not against the base build: on shared runners it moved by +13% to +116% on changes that
  cannot touch it, which is below the noise of code layout and neighbours. Throughput is stable and is gated relative to
  the base at the exact threshold for every scenario.
- **CI gate:** a reduced benchmark suite runs on every PR touching hot-path crates; a regression above 10% on
  any budget scenario's throughput, or a p95 latency above its ceiling, fails the build. Base and head are built on the same runner and measured alternately (three
  rounds each, best of each metric per side), so drift on a shared runner hits both sides alike. The full suite runs nightly.
- **Latency tracing:** every input and command carries timestamps per hop (client, hub, node, adapter) so the
  breakdown is visible, not guessed.
- **Performance HUD:** a developer overlay in the client (also usable by users as "connection details") shows
  per-hop latency, frame time, and stream backlog.
- **Profiles at phase exits:** CPU and memory profiles of node and hub under the capacity scenario are attached
  to the phase exit report.

### Measured through the hub and a real browser (Phase 3)

`node scripts/perf-browser.mjs` starts a hub and a node on one machine, signs in with headless Chromium (no GPU) and measures the
whole path: browser, TLS WebSocket, hub, node, PTY and back, with xterm.js (DOM renderer) drawing the result.

| Budget | Result | Note |
| --- | --- | --- |
| Keystroke → echo, same machine, ≤ 16 ms p95 | text on screen p50 2.6 ms, p95 14 ms; at the next frame p50 13 ms, p95 30 ms | 100 synthetic key presses 20 ms apart; the frame figure includes headless Chromium's software frame pacing |
| WebSocket round trip for a key (send → echo frame received) | 1.7 ms p50, 2.2 ms p95 | in the page, before xterm |
| Terminal snapshot after picking one of 20 agents, ≤ 150 ms | 50 ms p50, 50 ms p95 (12 of 12 picks) | from the click until the new pane's terminal shows its prompt, polled every frame |
| Four visible terminals flooding at once, 16 more agents idle, no main-thread task > 50 ms | no long task at 18.6 MB/s together | 4 × 10 MB, drained in 2.1 s (long-task API in headless Chromium) |
| Page memory | 15–19 MB JS heap | 20 agents listed, 4 terminals open |
| One terminal flooding output, no main-thread task > 50 ms | no long task at 8–9 MB/s | 20 MB through `yes \| head -c`, drained in 2.4 s |

The first run measured the echo at 80 ms (p95). Two causes, both in the hub and node, both invisible to unit tests:
the hub's client loop drained terminal frames from nodes only on its 20 ms wake-up instead of when they arrived, and
nobody had set `TCP_NODELAY`, so Nagle's algorithm and the peer's delayed ACK held small frames back for up to 40 ms
whenever keystrokes came less than a round trip apart. The loop now wakes on a terminal frame, and the hub's accepted
sockets and the node's uplink set `TCP_NODELAY`.

Not measured: twenty terminals all open and busy at once (the wall shows four; the other agents' output is not relayed to the browser), and any machine over a real network.

### xterm.js against a WebAssembly emulator (decides ADR-0017's open choice)

`scripts/terminal-compare` feeds identical bytes to xterm.js (DOM renderer and the WebGL addon) and ghostty-web 0.4.0 (Ghostty's
parser compiled to WebAssembly, xterm-compatible API) in the same headless Chromium (software GL), 20 MB per flood in 16 KB
writes with up to 64 in flight:

| Engine | Plain output | Colored output (SGR) | `write` of one key | Frame after it | Heap |
| --- | --- | --- | --- | --- | --- |
| xterm.js, DOM renderer (what the app ships) | 52 MB/s | 50 MB/s | 0.1 ms p50 / 0.2 p95 | 13 ms p95 13.5 | 14 MB |
| xterm.js + WebGL addon | 32 MB/s | 25 MB/s | 0.1 / 0.2 ms | 13 / 13.6 ms | 18 MB |
| ghostty-web (WebAssembly) | 37 MB/s | 32 MB/s | 13.6 / 14.1 ms | 30 / 30 ms | 13 MB |

None produced a long main-thread task. **Decision: keep xterm.js with the DOM renderer.** It is the fastest of the three here, it meets
every client budget measured above, and the WebAssembly emulator was slower on throughput and added a frame to each keystroke (it
paints on the animation frame; xterm.js updates the DOM synchronously). The WebGL addon is slower under software GL; with a real
GPU it may win on very large screens, which this headless setup cannot show, so it stays an option behind the `TerminalRenderer`
interface rather than a default. Caveats: one machine, software rendering, ghostty-web is a young package, and the figures
say nothing about text shaping (complex scripts), where Ghostty's parser is stronger; if that matters to users the swap is one
file (`apps/web/src/terminal/renderer.ts`).

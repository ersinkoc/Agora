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

## 3. Measurement and enforcement
- **Bench harness** (`just bench`): synthetic nodes and `agora-fake-agent` terminals producing scripted output,
  simulated clients, configurable RTT and loss. Each budget has a named scenario.
  Implemented so far (`bench/`): `channel-throughput` (bare channel floor), `pty-vt-throughput` (fake agent
  flooding one PTY into the VT backend) and `terminals-20` (twenty at once). They run unpaced and report the
  maximum the pipeline sustains, which must stay well above the 10 MB/s budget; on a development machine
  that is about 100 MB/s for one terminal and 240 MB/s in aggregate for twenty. The twenty-terminal scenario's
  latency is reported but not gated (it measures CI scheduler noise); its throughput is.
- **CI gate:** a reduced benchmark suite runs on every PR touching hot-path crates; a regression above 10% on
  any budget scenario fails the build. The full suite runs nightly.
- **Latency tracing:** every input and command carries timestamps per hop (client, hub, node, adapter) so the
  breakdown is visible, not guessed.
- **Performance HUD:** a developer overlay in the client (also usable by users as "connection details") shows
  per-hop latency, frame time, and stream backlog.
- **Profiles at phase exits:** CPU and memory profiles of node and hub under the capacity scenario are attached
  to the phase exit report.

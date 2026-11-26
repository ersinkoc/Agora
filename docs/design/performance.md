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

The first run found idle CPU at 8.7% and keystroke echo at 5 ms: every terminal actor woke 200 times per second to
poll for commands, and the supervisor copied a full screen per agent per tick just to read an exit status. Input,
resize and kill now go straight to the PTY on the caller's thread (`PtyControl`), the actor blocks on output, and
the supervisor reads the exit status without copying anything.

### History replay (ADR-0100 step 2, 2026-11-22)

`cargo run --release -p agora-bench -- history` (Linux, development machine, 30 runs). The lock figures are the capture that runs under
the terminal's lock for a 200×50 terminal whose 2000-line scrollback is full of 200-column lines; the switch figures are a real node on
this machine, from the `TerminalSubscribe` sent on its control socket until the size frame after the repaint arrived.

| Figure | Result | Note |
| --- | --- | --- |
| Lock held for a full 2000-line, 200-column history | p50 5.4 ms, max 5.9 ms (plain); p50 4.0 ms (a colour every 4 characters) | the coloured case stops near the byte cap, at 631 lines; formatting after the lock: 0.1 ms and 2.9 ms |
| Bytes per attach | 404 100 (plain, 2000 lines, 25 frames); 521 768 (coloured, 32 frames, the cap); repaint 11 176 | through the node: 426 100 bytes of history in 27 frames (2000 lines, a colour per line) |
| Terminal snapshot after switching, ≤ 150 ms | full history p50 6.2 ms, p95 6.7 ms; without history p50 0.3 ms | on one connection |
| The same on a new connection per subscription | before #958: full history p95 111 ms, without history p95 94 ms. After (#967, 50 runs): Linux full history p95 9–12 ms, without history p95 0.5 ms; Windows full history p95 107.71 → 12.48 ms, without history 94.78 → 1.14 ms | the node's uplink opens one for each subscription the hub routes. The difference was the control socket's accept loop, which slept 100 ms between tries; it now waits for the connection (`LocalListener::accept_timeout`), so a new connection costs about what an open one does |

The budget holds, so the cap stays at 512 KiB. The lock hold delays that terminal's own output by up to about 6 ms once per attach that
asks for history; input is not behind it (it goes to the PTY on the caller's thread).

The window writes the history and the repaint in one synchronized write (ADR-0100 step 3, `renderer.ts`, `reset`), once per attach that
asked for it, never on the output path. Measured in Chromium (xterm.js 6.0.0, DOM renderer, 15 runs, from the write until the frame
after it was parsed) for a 200×50 terminal with a full 2000-line, 200-column history: p50 23 ms, p95 75 ms (415 KB, plain); p50 29 ms,
p95 42 ms (923 KB, a colour every 4 characters, above the node's 512 KiB cap). With the node's p95 of 6.7 ms that keeps "terminal
snapshot after switching" under 150 ms, and it is one paint: the screen is not drawn until the repaint has been parsed.

### Hub cold start against history size (architecture review R4, 2026-11-14)

The hub rebuilds its view by replaying its whole event log at start (`View::rebuild`), so its cold start grows with history. Measured
with `crates/agora-hub/tests/startup_measure.rs` (ignored; `AGORA_MEASURE_DB=<copy of a hub.db>`, release build, Linux), on a copy
of a real hub's log of 20 982 events from 26 hours (5 machines, 17 agents), repeated to make longer histories with the same view:

| Events | Database | Rebuild | `Hub::open` |
| --- | --- | --- | --- |
| 20 982 | 2.4 MiB | 18 ms | 43 ms |
| 209 820 | 22 MiB | 125 ms | 119 ms |
| 1 049 100 | 110 MiB | 591 ms | 649 ms |
| 2 098 200 | 220 MiB | 1 160 ms | 1 127 ms |

About 0.55 ms per 1 000 events: the 500 ms budget is passed near 900 000 events. **96% of that log was `AgentUsageReported`**
(20 104 events; one agent alone wrote 11 751, about 7 a minute). The node now gathers usage reports of one agent, model and currency
for 5 s and logs one (`agora-node::usage_merge`; sums are exact, so totals do not change). On the same log that leaves 5.5% of the
usage events, which moves the budget from about three months of such use to years. A projection checkpoint (replaying only the log's
tail) stays the next step if a hub's history still grows past it; it needs its own design for versioning, corruption fallback and
backups.

### Measured on macOS and Windows (2026-11-25, `0b7295d1`)

Release `agora-bench` (`footprint`, `history`, `run --secs 2`) run on two real machines: Apple M4 (10 cores, 24 GB, macOS 27.2.1,
`/bin/sh`) and Windows 11 (WHITE, 32 CPUs, 126 GB, Git Bash `sh` on `PATH` through ConPTY). The `footprint` figures are one run
each per OS: a first, cold run with 50 terminals, then a warm run with 10.

| Metric | macOS (50 cold / 10 warm) | Windows 11 (50 cold / 10 warm) | Budget | Note |
| --- | --- | --- | --- | --- |
| Node base memory | 11.27 / 11.23 MB | 9.67 / 9.48 MB | ≤ 50 MB | ok on both |
| Memory per idle terminal | 0.17 / 0.19 MB | 2.08 / 2.14 MB | ≤ 10 MB | ok on both (on Windows probably the ConPTY host's own memory: a guess, not measured apart) |
| Memory per busy terminal | 4.19 / 4.22 MB | 4.05 / 4.07 MB | ≤ 10 MB | ok on both |
| Idle CPU | 0.16 / 0.05% | 0.00 / 0.00% | ≤ 1% | ok on both |
| Node cold start | 51 / 42 ms | 2 760 / 197 ms | ≤ 200 ms | Windows over budget cold (2 760 ms), at the limit warm (197 ms); warm time is mostly three serial `git.exe` spawns in `discover_checkout`; a fix that saves one of them is proposed in #1210 |
| Keystroke → echo, same machine (p95) | 0.10 / 0.07 ms | 18.21 / 17.45 ms | ≤ 16 ms | Windows over budget, likely from the Windows timer tick (~15.6 ms) in ConPTY and MSYS2 `sh`, not measured apart; Agora's terminal path does not poll, but `pty-vt-throughput` on Windows is also far below macOS (13.2 against 164 MB/s) |
| History snapshot after switching (same conn, p95) | 10.23 ms (no history: 0.28 ms) | 6.86 ms (no history: 0.36 ms) | ≤ 150 ms | ok on both; new conn per sub: macOS 2.35 ms, Windows 7.43 ms |
| `channel-throughput` | 1 707 MB/s (p95 704 µs) | 1 345 MB/s (p95 937 µs) | — | — |
| `pty-vt-throughput` | 164 MB/s (p95 6.0 µs) | 13.2 MB/s (p95 76.6 µs) | — | — |
| `terminals-20` | 177 MB/s (p95 46.6 µs) | 18.5 MB/s (p95 166 µs) | — | — |

The two Windows misses (node cold start, keystroke echo) are not accepted as the new budget (ADR-0022: never relaxed silently); they
are tracked on #1150.

Not measured yet: binary size with web assets, process start (exec) in
the cold-start figure, and real-browser frame times with many panes (M1.3).

## 2. Tactics

### Data path
- **Terminal bytes are not events.** They flow on their own streams, are relayed by the hub without decoding
  or storing, and use shared buffers (no copies per client).
- **Direct path** (ADR-0023, accepted, not built yet: terminal streams still go through the hub relay): when a client can reach a node directly (same machine or LAN), terminal
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
- Terminal subscribe and resync capture the screen at the stream boundary and build the ANSI repaint after releasing the terminal lock.
  A subscription that asks for history (ADR-0100) also copies the cells of the newest scrollback lines under that lock, newest
  first, stopping at the count or near the 512 KiB cap; formatting them into kind 4 frames happens after the lock is released.
- Scrollback stored compressed in memory with a configurable cap.
- Hub projections are in memory. A node event's SQLite write, projection update and fanout run together on a blocking worker.
  Each hub admits one job at a time. A connection groups up to 16 immediately available events or 64 KiB per job and then waits
  for that batch before processing later frames. A larger event runs alone. Control frames, sequence gaps and acknowledgement
  boundaries preserve stream order.
- Event fanout uses subscription indexes (by project, node, agent) instead of filtering every event per client.
- The file watcher stops checked-handle enumeration after recording 50,000 non-directory entries. It does not first
  collect a whole directory into memory. This is a recorded-entry budget; traversed directories are not counted.
  Full directory requests still collect their requested level. [Watcher measurements](../reviews/0343-bounded-directory-visits.md).
- HTTP requests that need a fresh proxy trust configuration share one file read on a blocking worker. Requests awaiting
  that read apply its result before checking proxy trust, and the cache is rechecked after two seconds.

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
  rounds each, best of each metric per side), so drift on a shared runner hits both sides alike. It runs on every push to `main` and every pull request; there is no nightly run yet.
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

### Commands through the hub while another machine floods output (architecture review R5)

`crates/agora-hub/tests/flood_latency.rs` (release, `--ignored`; a real hub on loopback, Linux, 4 runtime threads): a window sends an
`agent.list` to one machine every 5 ms, 400 times, and times the answer, while another machine writes 4 KiB terminal frames as fast as
the hub takes them to a window (about 1.5–1.8 GB/s). This times the hub's share of the remote keystroke budget (≤ 5 ms).

| Flood | Before | After |
|---|---|---|
| none | p50 21.4 ms, p99 21.7 ms | p50 0.21 ms, p99 0.45 ms, max 0.7 ms |
| to another window | p50 21.1 ms, p99 21.3 ms | p50 0.21 ms, p99 0.41 ms, max 1.3 ms |
| to the same window | p50 21.1 ms, p99 21.4 ms | p50 0.21 ms, p99 0.62 ms, max 3.8 ms |

The flood never slowed a command: the 21 ms was the window's loop, which sent a node's answer only at its next 20 ms tick (`PUSH`).
The loop now wakes when an answer arrives (`server.rs::an_answer`); a non-ignored test keeps the median of 20 round trips under 10 ms.
Live events to a window still go out at that tick (the subscriber queue is a blocking channel): up to 20 ms on top of the state and
approval budgets, which they meet.

### Measured through the hub in the terminal client (ADR-0063)

`agora tui` (release build, macOS on Apple silicon) against a hub and a node on the same machine, driven in a 160×40 pseudo-terminal:
a key is written to the console, which sends it to the shell agent through the hub, and the time runs until the echoed character is
in the console's output. The flood row times the console's own answer (Ctrl K drawing the palette) while the agent's tab streams
`yes … | head -c 50000000`.

| Budget | Result | Note |
|---|---|---|
| Keystroke → echo, same machine, ≤ 16 ms p95 | p50 3.6 ms, p95 4.6 ms, max 13 ms | 100 keys, 20 ms apart |
| Input outranks output | palette drawn p50 4.1 ms, p95 6.2 ms, max 6.8 ms during the flood | 100 times |
| Idle CPU | 0.2% with a terminal tab open and nothing typed | 10 s |

The first wiring measured the echo at p50 33 ms: frames were capped at 25 a second, a key that only went to the agent still drew a
frame (holding the echo back by up to one more frame), and the loop slept 15 ms on the hub while a key waited on stdin. Now frames are
capped at 60 a second, a key that changes nothing on screen draws nothing, and while the user types (a key in the last second) the loop
reads keys every 2 ms; idle, it sleeps as before.

### Large lists: the static agent tree and a large diff (architecture review R6, 2026-11-14)

`node scripts/perf-tree.mjs` starts a hub on a throw-away home, seeds it with `scripts/perf-seed-agents.py` (machines x projects x
agents; `AGORA_PERF_FLEET`) and measures the tree in headless Chromium (software rendering, 1440x900). No node connects and no
terminal streams: the seeded machines are offline after the hub opens. Existing artifacts can be supplied with
`AGORA_PERF_BIN` and `AGORA_PERF_WEB_DIR` instead of building another target.
`PYTHON` selects the Python executable (default `python3`); Windows can set it to `python`, with `AGORA_PERF_BIN` pointing to
`agora.exe` and `CHROME` to the Chromium executable. `PW_DIR` names the directory containing `node_modules/playwright-core`.
Repository and executable paths may contain spaces. Cleanup waits for the temporary hub to exit before removing its files.

The warm reload timing (`domRowsReadyMs`) ends when the fleet is loaded and its first rows reach the DOM. With an older build
it waits for every row. Picking measures from a programmatic click on a visible row to the next frame after scrolling to it.
Filtering measures Explorer's local state (`filterToFrameMs`).

The historical measurements after #557 used fixture input counts for projects. The fixture needed worktree events to associate
those projects with agents in the client. The corrected benchmark below checks the projected counts in Chromium.

After #557, the Long Tasks observer reports results by phase. Each entry belongs to the phase in which its task started;
the original scroll phase included returning to the top and a settling frame. The current phase checks that scrolling reaches
rows at the bottom. Pending observer records are drained after the final phase. The script reports scroll, pick, filter and
outside-phase counts separately, with their total. Evidence and the original post-scroll control:
[measurement record](../reviews/0557-perf-phases.md).

| Static fleet | Reload → DOM rows ready | Scroll frame gap, p50 / p95 | Pick → frame, p50 / p95 | Filter → frame, p50 / p95 | Long tasks: scroll / pick / filter | Tree DOM / heap |
| --- | --- | --- | --- | --- | --- | --- |
| **500 agents**, 50 projects, 10 machines | 139 ms | 16.9 / 24.4 ms | 29.0 / 37.4 ms | 21.3 / 31.3 ms | 0 / 0 / 0 | 3 810 nodes / 11 MB |
| 2 000 agents, 100 projects, 20 machines | 395 ms | 62.0 / 93.8 ms | 82.4 / 93.2 ms | 75.6 / 111.5 ms | 89 / 10 / 26 | 14 820 nodes / 38 MB |

The 500-agent static case stays below the 50 ms selection budget and has no long tasks, but its slowest frame tail misses a
60 Hz interval. The 2 000-agent case clearly misses both scroll and selection budgets. These runs do not certify the budget for
busy live fleets or change any budget; larger-tree work still needs measurements of those workloads and windowing where required.
The old scroll long-task count included later picks and filters and is superseded by these phase-specific results.

**Explorer windowing (#1126).** At 120 agents, Explorer renders agent rows in batches near its scroll area. Group headings remain
available, and the focused and selected batches stay mounted. Rows keep their identity when an earlier row is added or removed.
Each group has one list, and a mounted item carries its position and the full group size. Browsers without the visibility or size
observers render the whole list. The table records two paired Chromium runs with the corrected fixture and the same benchmark script.
Both builds showed 500 agents, 50 projects and 10 machines. Picking waits for a visible row after moving the scrollbar. The
click-to-frame timer starts when the row is ready. The runner counts samples that need more than two settling frames.

| Metric | Before windowing | With windowing |
| --- | --- | --- |
| Initially mounted agent rows | 500 | 30 |
| Explorer DOM nodes | 8 200 | 918 |
| Scroll frame gap, p50 | 40.7–42.0 ms | 16.6–16.7 ms |
| Scroll frame gap, p95 | 50.9–51.8 ms | 17.4–17.8 ms |
| Pick → frame, p95 | 62.8–69.8 ms | 20.9–22.2 ms |
| Long tasks while scrolling | 3–8 | 0 |

The median reaches one 60 Hz interval in this fixture. The p95 remains just above 16.7 ms. Both builds reached the end of the
tree and visited 20 agent rows with 40 consecutive Tab presses.

**A diff of 10 000 changed lines** is drawn as a window since #511, in the review screen and in `DiffDialog` (it uses the same
`ChangesView`): `scripts/e2e-changes-large.mjs` opens a file rewritten whole (20 000 rows), Show all and side by side with **no
main-thread task over 50 ms** and 72 rows in the DOM.

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

**Revisited, 2026-11-17 (`docs/ux/terminal-fidelity.md`): the default stays the page's drawing; the graphics-card renderer is an opt-in setting.** The table above stands (32 against
52 MB/s under software GL, one frame after a key either way, no long task). What it did not measure is whether the drawing is right:
with the page's own text drawing, a block of full-block characters has 7.4% of its area empty at row height 1.1 and 1.9% at 1.0;
with the addon it is 0.00% at 13, 14 and 15 px (`scripts/terminal-fidelity/run.mjs`). It is not the default because a real GPU and a
scaled (high-density) screen were not seen (the headless browser's canvas stays at 1x there and draws nothing), a browser allows about
sixteen graphics contexts per page, and the fallback to the page's drawing on a lost context blanks that pane for a moment.

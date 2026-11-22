# GPUI spike, Linux part (ADR-0099 stage 0)

Date: 2026-11-22. Code: `apps/desktop-native-spike/` (its own workspace). How to run it elsewhere: `gpui-spike-howto.md`. Raw
results: `gpui-spike-linux-results/` (one JSON file per run and a `summary.json` per run set).

**Bottom line for Linux.** The spike builds and works: it signs in to a real hub with a one-time code, lists the agents, draws up to
four terminals natively from `agora-vt` cells with GPU text, and the input path passes all 35 correctness checks (keys, IME at the
app level, every mouse mode and encoding, wheel, selection, paste, one mouse report end to end through a real PTY). The AT-SPI tree
exposes the agents list, its items and the button, and pressing the button through AT-SPI works. Size (27.0 MB), resident memory
(108 MB with the real GPU driver) and idle CPU (0.10 %) are inside their gates. **No frame-rate, echo or switching number from this
machine is representative**: the agent had no visible window on a GPU (see "Where it ran"). Those gates (F1, F2, F3, switching)
are open until the macOS and Windows sessions and a person on a Linux desktop run the same harness.

## Where it ran and how far each number can be trusted

The machine: AMD Ryzen 5 7430U (12 threads) with Radeon graphics (Mesa 26.0.8, RADV), 29 GB RAM, Ubuntu 26.04.1, GNOME on
Wayland. It was shared: other sessions were building and testing Agora (load average 5 to 10 during the runs, up to 30 earlier).

| Run | Display and GPU | What it can show | Representative? |
| --- | --- | --- | --- |
| **A: Xvfb** | Xvfb 21.1.22 (the Ubuntu package unpacked into a scratch folder, no root), no DRI3, so GPUI chose **llvmpipe** (software Vulkan) by itself; no window manager, so the window was never focused (GPUI then caps drawing at 30 fps); a private D-Bus session (`dbus-run-session`) with its own AT-SPI bus | correctness (bytes in and out, drawing happens, accessibility tree), and relative costs | **No** for every performance number |
| **B: live desktop** | the person's GNOME Wayland session, real GPU (RADV, `software: false`); the spike's window opened there but was never visible or focused (other windows in front), so the compositor sent no frame callbacks and **no frame was presented** after the first ones | start-up, memory and idle CPU with the real driver; the main thread and network path under the floods | **No** (no frames; shared machine); memory and idle CPU are indicative |

Not possible here: a run with a visible window on the GPU. Two attempts at a GPU-backed virtual display failed: Weston 14's headless
and VNC backends (unpacked the same way) offer no `wl_seat` until a client connects, and **GPUI panics on a compositor without a seat**
(`gpui_linux/src/linux/platform.rs:381`, "Wayland compositor does not provide wl_seat"). Taking over the person's screen was not an
option.

## What was built

- **GPUI source:** `zed-industries/zed` at tag v1.29.3, revision `d01b134cddf5e4f71964deb2d6f81cdb38fcf1e7`, crates `gpui` and
  `gpui_platform` (Apache-2.0), features `font-kit`, `wayland`, `x11`. It built and ran here, so `gpui-ce` was not needed and **was not
  built** (ADR-0099 decision 1 says the spike also builds against it; that is still to do). GPUI is named in one module (`src/g.rs`).
  No Zed application code was read or copied; the terminal element was written from the `agora-vt` trait and GPUI's public API.
- **Main workspace:** untouched. The spike depends by path on `agora-vt`, `agora-protocol` and `agora-tui` (for `client::sign_in` and
  `termio`, the pure mouse, wheel and paste code). Depending on `agora-tui` brings ratatui and crossterm into the spike; stage 1's
  shared crate removes that. The TLS and WebSocket setup is the TUI's approach repeated in `net.rs` (its `tls` is private).
- **Threads:** a network thread owns the WebSocket (tokio, current-thread), sends input before reading more output (`select!` with
  `biased`), and **feeds terminal output into each emulator itself**, under a short lock; the main thread is woken once per batch (a
  coalesced flag) and only copies the visible cells and draws. Each frame copies the visible cells under the lock, then draws background
  runs as quads and text as shaped runs per style (`shape_line` with a forced cell width; a wide character is shaped alone at its
  cell). GPUI's line layout cache keeps unchanged rows from being shaped again; there is no damage tracking (`agora-vt` has none).
- **Terminal behaviour** (`term.rs`), following `docs/ux/terminal-fidelity.md`: xterm key forms (DECCKM, modifiers, F-keys, Ctrl and
  Alt combinations), text and IME through GPUI's input handler (preedit drawn underlined at the cursor, the candidate window placed at
  the cursor cell); mouse modes 9/1000/1002/1003 with encodings default/1005/1006/1015 through `termio::encode_mouse`, motion only under
  1003, drags only under 1002/1003, a press the program got always gets its release, Shift keeps everything local, Ctrl+click is never
  sent; wheel: reports when the program tracks the mouse, three arrows (in the DECCKM form) on the alternate screen without it, history
  on the main screen (three lines a notch), typing returns to the live screen; selection by drag, double click (word) and triple click
  (line) over screen and history, Ctrl+Shift+C / Cmd+C / Ctrl+Insert to copy, the primary selection on release, middle-click paste,
  bracketed paste with control characters removed. A background terminal keeps its emulator up to date without being drawn. When the
  node says a viewer fell behind, the terminal is watched again (as the TUI does).
- **Not built:** OSC 8 links and OSC 52, focus reports (1004), the find bar, keyboard protocols (CSI u), splits, themes and i18n (the
  few UI strings are English literals; this is a spike), reconnecting.

## Results

Numbers from run A (Xvfb, software) and run B (real GPU, no frames). F2 and F3 of run A are from a rerun with the final binary,
which counts a window frame once per frame callback (the first count split a frame painted in several panes into several). Limits are ADR-0099 decision 6. Neither column is
representative; "n/a" means the run could not measure it.

| Gate | Limit | A: Xvfb + llvmpipe | B: Wayland + RADV, window not shown | Linux verdict |
| --- | --- | --- | --- | --- |
| F1 key to echo presented, p95 | ≤ 16 ms | 64.5 ms (p50 57.5) | n/a (no frames) | **open** |
| F1 key to frame painted, p95 (before present) | (diagnostic) | 17.8 ms (p50 10.8) | n/a | - |
| F1 key to output back from the node, p95 | (diagnostic) | 1.3 ms (p50 0.7) | n/a | - |
| F2 frame rate, one 10 MB/s flood + 20 active background | 60 fps | 20.8 fps (gaps p50 48 ms: the 30 fps cap of an unfocused window, then the next tick) | n/a | **open** |
| F2 main-thread stalls > 50 ms | 0 | 0 (heartbeat max 23 ms); 1 frame gap of 51 ms | 0 (heartbeat max 29.7 ms) | **open** (main thread is fine; frames unmeasured) |
| F3 frame rate, four 10 MB/s floods | 60 fps | 21.3 fps (every pane in every frame; pane paint p95 1.5 ms) | n/a | **open** |
| F3 main-thread stalls > 50 ms | 0 | 0 (heartbeat max 36 ms); no frame gap over 50 ms | 0 (heartbeat max 4.4 ms) | **open** |
| Switch to a terminal, snapshot shown, p95 | ≤ 150 ms | 118 ms (p50 113) | n/a | open (likely pass) |
| Switch tab / agent, p95 | ≤ 50 ms | 63 / 64 ms (each includes waiting for the next 30 fps frame) | n/a | **open** |
| Cold start to first useful paint | ≤ 500 ms | 590 ms | **438 ms** (first frame 431 ms) | indicative pass on the GPU |
| Warm start, median of 5 | (report) | 182 ms (first frame 131 ms) | **101 ms** (first frame 97 ms) | - |
| F4 resident memory | ≤ 200 MB and below Tauri | 166 MB (llvmpipe's own buffers) | **108 MB** | indicative pass; Tauri not measured here |
| Idle CPU, window open, nothing changing | ≤ 1 % | 0.38 % | **0.10 %** (window not shown) | indicative pass |
| Stripped binary | ≤ 40 MB | 27.0 MB | 27.0 MB | **pass** (a GPUI hello world alone is 23.5 MB) |
| IME (CJK, Turkish) | works | app level only: preedit drawn and nothing sent; commit `日本` sent once; `ş` through the input handler | - | **open** (no OS input method exercised) |
| F5 screen reader | announces list item and button | AT-SPI tree verified (below); Orca not run | - | **open** (tree pass) |

What the floods show even without frames: the node and hub relay each flood at the full rate (10.1 MB/s each, 40.5 MB/s for F3, no
"fell behind" from the node in the final runs), and parsing them costs the network thread about 18 to 20 % of a core per 10 MB/s
(run A; 36 % in run B's F2 on the busier machine). **One network thread therefore tops out around four or five floods**: the product
should parse each terminal on its own task or thread pool. The main thread, which only draws, never stalled in run B.

Start-up: cold means the binary and the 56 files it maps were evicted from the page cache first (`posix_fadvise`, no root); shared
libraries other processes keep mapped (libc, Mesa) stay cached, so this is a partial cold start. The time includes signing in to the
local hub over TLS and receiving the snapshot.

### Correctness: 35 of 35 checks (run A)

`selftest.json`: the checks go through GPUI's own event dispatch (`Window::dispatch_event`, `Window::dispatch_keystroke`), so hit
testing, focus and listeners are part of what is checked; the program's modes are set by feeding output to the emulator and mode
scanner, as the stream does. They cover: 10 keys and DECCKM; a non-ASCII character through the input handler; IME preedit drawn with
nothing sent, then the commit sent once; mouse 1000 with SGR, default, urxvt and UTF-8 (at column 121, where the coordinate takes two
bytes) encodings; 9 (press only); 1000 without drags, 1002 with drags; motion only under 1003; Shift+click sending nothing; the wheel
as reports, as three arrows (both cursor-key forms), and as history scrolling; typing returning to the live screen; drag, word and
line selection and copy; bracketed paste with an ESC removed; and **end to end**: a real program behind the node's PTY (the spike's
`helper probe`, raw mode) switched SGR mouse reporting on, the window saw the mode in the stream, a click was sent and the program
printed exactly the bytes sent (`1b 5b 3c 30 3b 33 3b 32 4d …`). Run B passed 33: the two checks that need a painted frame (the preedit
drawn, a click on a pane laid out after the window stopped drawing) could not run there.

A program that turns the mouse on **before** the window starts watching is not known to have done so (the node's repaint carries no
mouse modes, `terminal-fidelity.md` "Phase 3" item 1): the probe therefore waits for a line before switching the mouse on. The native
client inherits this limit from the protocol, not from GPUI.

### Accessibility (F5): tree verified on Linux, announcement not verified

`gpui-spike-linux-results/xvfb-llvmpipe/a11y.json`: with the AT-SPI bus enabled (private session), GPUI's AccessKit integration
exposed `application "agora-native-spike"` → `frame "Agora native spike"` → `list "Agents"` with 26 `list item`s named after the agents
(`laptop/agent-1 (spike-echo)`, …), and a `push button "Open terminal"` with the action `click`. Invoking that action through AT-SPI
opened a terminal (terminals open 1 → 2). The roles and names came from `div().id(..).role(Role::ListItem).aria_label(..)` and
`Role::Button`; no custom accessibility code. Orca is installed here but was not run (it needs a focused window and audio or its
debug log; a person's step in the howto). Focus movement and announcements of focus changes are therefore unverified.

### RTL (finding, not a gate)

From GPUI's code at the pin (not seen on screen): text shaping is bidirectional (on Linux GPUI uses cosmic-text with `unicode-bidi`;
line wrapping has an RTL workaround commented "lack of complete UAX #9 support"; on Windows the DirectWrite glyph runs are drawn with
`bidiLevel: 0`). There is **no layout direction**: no `dir`/`direction` style, only flex `row-reverse`. A mirrored layout would be the
app's own work: a small layer of direction-aware helpers (start/end instead of left/right in padding, margins, borders and alignment,
row direction chosen by the locale) used by every component, roughly one to two weeks for the parity screens plus a review of each
screen; upstreaming a direction property to GPUI is larger and uncertain. Terminal panes stay left to right by design. The
pseudo-RTL check in the howto (step 4) should be run on macOS and Windows.

### Without a GPU (finding, not a gate)

On an X server without DRI3 GPUI fell back to llvmpipe by itself and stayed usable (warm start 182 ms; F3's four floods drawn at 21 fps
with the unfocused window's 30 fps cap). That fallback needs a software Vulkan driver installed (Mesa's lavapipe is common on desktops; not on every
server or VM). A Wayland compositor without a seat makes GPUI panic at start (above): headless and kiosk setups need a check before
GPUI starts. Remote desktops on macOS and Windows are the howto's step 5.

## Other findings

- **An X11 window that is not visible blocks the main thread.** Under Xwayland with the window covered, each present took about one
  second (frames at 1 fps, the 10 ms heartbeat late by about 980 ms): input handling would stall the same way. Under Wayland a covered
  window simply draws nothing and the main thread stays free. Needs a check on a real X11 desktop before X11 is supported.
- **Not one git source.** GPUI at this revision depends by git revision on wgpu (`gfx-rs/wgpu`, an unreleased commit) and on Zed's forks
  of font-kit, oo7 and xim-rs (a scap fork appears in the lock file but is compiled out). `deny.toml` allows exactly those, each with
  its reason; ADR-0099 decision 1's "the one git source of the pin and nothing else" cannot hold for this GPUI.
- **cargo-deny is clean** with six "unmaintained" advisories ignored (chrono, rustybuzz, ttf-parser, zvariant, zbus_names,
  zvariant_derive: GPUI's text and Linux D-Bus stack, no vulnerability) and four licence exceptions per crate (MPL-2.0: cbindgen,
  build-time; dwrote, Windows; option-ext; CC0-1.0: tiny-keccak). No GPL.
- **Dependencies:** 710 packages in `Cargo.lock` (all targets); compiled: 516 on Linux, 390 on macOS, 365 on Windows (`cargo tree`,
  normal and build edges, including the Agora path crates).
- **Build:** the first build of a GPUI hello world took 6 min 25 s wall (21 min 45 s CPU) on this busy 12-thread laptop, including
  fetching Zed's repository (about 410 MB); after that the spike's own crate rebuilds in 41 to 58 s (release, thin LTO, one codegen
  unit). `target/` is 2.5 GB. Linux needs `libxkbcommon-x11-dev` (here only the runtime library was installed; the link was made with a
  `LIBRARY_PATH` pointing at a symlink).
- **F3 delivered nothing once.** In one of four F3 runs after the "fell behind" handling, the four flood terminals received no output
  during the recording although their screens had arrived and the start line was sent; the cause was not found (the run files now carry
  each terminal's last screen lines to find it). Earlier, before the window watched again after "fell behind", F3 got 1.9 MB/s per
  terminal: the node had dropped the viewer.
- **Frame pacing:** GPUI draws a window that is not focused at most 30 times a second (`inactive_frame_interval`, default 33 ms). A
  terminal visible in an unfocused window (a second monitor) is therefore capped at 30 fps; the product should set this deliberately.

## Not done here, and who does it

| Item | Who |
| --- | --- |
| F1, F2, F3 and switching with a visible window on a GPU | the macOS and Windows sessions (howto), and a person on a Linux desktop (Wayland and X11) |
| Screen readers announcing the list item and the button | a person: VoiceOver, Narrator or NVDA, Orca (howto step 1) |
| OS input methods: Japanese or Chinese, Turkish layout, dead keys | a person on each system (howto steps 2 and 3) |
| The Tauri app measured the same way (F4) | each system (howto step 6); the Tauri app is not built on this machine |
| Building against `gpui-ce` | a follow-up on the spike branch |
| A CI job for the spike | not added: the spike stays out of CI until stage 2 decides |

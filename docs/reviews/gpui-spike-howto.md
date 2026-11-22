# How to run the GPUI spike (ADR-0099 stage 0) on macOS, Windows and a Linux desktop

The spike is `apps/desktop-native-spike/`: its own Cargo workspace (own `Cargo.lock`, own `deny.toml`), outside the main build and
CI, like `apps/desktop/src-tauri`. One binary does everything: the window (`app`), the measurement harness (`run`), a probe for
another process's memory (`measure`) and the fixture agents' program (`helper`). The harness starts a disposable hub and node (its
own temporary `AGORA_HOME`, a free port on `127.0.0.1`), starts the fixture agents through the hub, launches the window once per
fixture, and writes one JSON file per run plus `summary.json` with the go/no-go numbers of ADR-0099 decision 6. It never touches
`~/.agora` or a running Agora. The Linux results and what they mean are in `2026-11-22-gpui-spike-linux.md`.

## What it needs

- The repository at the spike's commit, Rust 1.99.1 (`rust-toolchain.toml`), git, and network access for the first build (Cargo
  fetches Zed's repository at the pinned revision, about 400 MB, and wgpu's).
- **macOS:** Xcode command line tools (`xcode-select --install`). GPUI on macOS uses Metal; the first build compiles its shaders.
- **Windows:** Visual Studio Build Tools with "Desktop development with C++" (MSVC and the Windows SDK). GPUI uses DirectX 11.
- **Linux:** the X11/Wayland development libraries (Debian/Ubuntu: `libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev
  libxcb1-dev libfontconfig-dev libfreetype-dev libvulkan-dev`) and a Vulkan driver (Mesa). For the accessibility check:
  `python3-gi` with `gir1.2-atspi-2.0` (on GNOME already there).

## Build

Run each block from the repository root. The `agora` binary is the hub and node the harness starts; build it in release (a debug
node relays a 10 MB/s flood slower than a release one and would distort F2 and F3).

```sh
cargo build --release -p agora
cd apps/desktop-native-spike
cargo build --release
```

On Windows use the same commands in a Developer PowerShell. Record the wall time of the spike's first build (it includes the Zed
fetch) and of a rebuild after touching `src/main.rs`; the report wants both.

## Run the fixtures

Put the window on the main display and **do not cover it or switch away while it runs**: GPUI draws only when the system says the
window can be seen, and a window that is not focused is throttled to 30 frames per second (`WindowOptions::inactive_frame_interval`).
Close heavy programs first and keep the machine on mains power. A run takes about four minutes; the window opens and closes once per
fixture (start runs seven times).

```sh
# macOS / Linux, from apps/desktop-native-spike
./target/release/agora-native-spike run --out results-$(date +%Y%m%d-%H%M) --label "macos-<model>"
```

```powershell
# Windows, from apps\desktop-native-spike
.\target\release\agora-native-spike.exe run --out results-win --label "windows-<model>"
```

Options: `--only start,f1,f2,f3,switch,f4,selftest,a11y` runs a subset; `--warm-runs N` (default 5); `--agora PATH` if the `agora`
binary is not in the repository's `target/release` or `target/debug`; `--not-representative "why"` marks the whole run (use it if the
window was covered, the machine was busy, it ran in a VM or remote desktop); `--keep` keeps the temporary folder.

**On Linux**, run it inside a private session bus so the accessibility bus is switched on only for the run:

```sh
AGORA_SPIKE_PRIVATE_BUS=1 dbus-run-session -- ./target/release/agora-native-spike run --out results-linux --label "linux-<desktop>"
```

Run it once under Wayland and once under X11 (`env -u WAYLAND_DISPLAY ./target/release/...` in an X11 or Xwayland session).

## What the fixtures do

| Run | What happens | What is written |
| --- | --- | --- |
| `start` | The window signs in with a one-time code and paints the agents list, then quits. Once with the binary and the libraries it maps evicted from the page cache (Linux only; elsewhere this is a "first run"), then `--warm-runs` times. | `start-cold.json`, `start-warm-N.json`: spawn to first frame, spawn to first useful paint (the agents list from the hub). |
| `f1` | One echo terminal; 300 keystrokes dispatched into the window's input path 30 to 75 ms apart (an Enter every 50). | `f1.json`: key to output received, to the frame painted, to the next frame after it (presented). |
| `f2` | One visible terminal receiving 10 MB/s, 20 background terminals at 2 KB/s each, 10 s recorded. | `f2.json`: frames per second, frame gaps, main-thread heartbeat lateness (stalls over 50 ms), paint cost, MB/s received per terminal, parse time on the network thread. |
| `f3` | Four visible terminals each receiving 10 MB/s, 10 s recorded. | `f3.json`: the same. |
| `switch` | Opens six terminals one after another, then brings background ones to the front, then changes the selected agent. | `switch.json`: open request to the node's screen painted; tab and agent switch to the presented frame. |
| `f4` | 26 agents listed (25 on Windows, which has no probe agent), four terminals visible, two in the background, idle for 11 s. | `f4.json`: resident memory (`driver.rss_bytes`), CPU over 10 s with nothing changing (`driver.idle_cpu_percent_of_one_core`). |
| `selftest` | 35 checks of keys, IME, mouse modes and encodings, wheel, selection, paste, and one end-to-end mouse report through a real PTY (Unix). | `selftest.json`: each check with expected and received bytes. |
| `a11y` | The window holds 12 s; on Linux the harness reads its AT-SPI tree and presses "Open terminal" through it. | `a11y.json`: the roles and names found, the button's action, whether pressing it opened a terminal. |

`summary.json` lists each gate with its value, its limit and pass or fail, the machine (OS, version, CPU, no host or user names),
the GPU GPUI chose (`gpu` in each run file: device, driver, whether it is software), and whether the run was marked representative.
Send the whole results folder (it is small) to the spike's PR.

## Steps the harness cannot do (a person, about 15 minutes)

Start the window by hand against the harness's hub: run `run --only a11y --keep`, then in the kept folder read the hub's address
from the run files, or simply run `agora-native-spike app --hub 127.0.0.1:<port> --ca <tmp>/hub/ca.pem --code <code from agora hub
login --hub-dir <tmp>/hub>`. Write pass or fail and what you saw for each.

1. **Screen reader (F5).** macOS: VoiceOver (Cmd+F5); move into the agents list (VO+arrows) and to "Open terminal". Windows:
   Narrator (Win+Ctrl+Enter) or NVDA; Tab to "Open terminal", move into the list with the arrow keys or scan mode. Linux: Orca
   (Super+Alt+S). Pass: the list is announced as a list, an item by its name ("laptop/agent-1 (spike-echo)"), and the button as
   "Open terminal, button"; activating it with the screen reader opens a terminal.
2. **IME (Japanese or Chinese).** Add the system's Japanese input (macOS: Japanese - Romaji; Windows: Microsoft IME; Linux: IBus or
   Fcitx 5 with Mozc). Click an echo terminal, type `nihon` and convert: the underlined preedit must show at the cursor while
   composing, and after confirming, `日本` must appear once (the echo terminal shows what the program received). Also check that
   the candidate window sits next to the cursor.
3. **Turkish layout.** Switch to Turkish Q: type `ğüşıöç İ` in a terminal; each must arrive once. Then a dead key on a layout that
   has them (macOS: Option+E then E for `é`; Windows/Linux US-International: `'` then `e`).
4. **RTL.** No RTL text is drawn by the spike's own UI; check what GPUI does with a mixed line: in the echo terminal type
   `echo "abc עברית 123 العربية"` and Enter. Note whether the Hebrew and Arabic are shaped (joined letters) and in which order the
   runs appear. A terminal grid draws cells left to right by design; the question for decision 5's gate is GPUI's own text layout,
   so also note any RTL behaviour seen in GPUI's text elsewhere.
5. **No GPU.** In a VM without 3D acceleration, or over Remote Desktop (Windows) / Screen Sharing (macOS), start the window and
   record whether it draws, what `gpu` says in a run file (`software: true` means a software fallback), and how it feels.
6. **The Tauri app, measured the same way (F4).** Start the released desktop app connected to the same hub with 20 agents listed,
   four terminals visible and two open in other tabs, wait until idle, then run `agora-native-spike measure --pid <pid of the app>
   --tree --secs 10`. Use `tree_rss_bytes`: the webview runs in helper processes (WebView2 on Windows, WebKit's on macOS and Linux).
   On macOS the WebKit helpers are launched by `launchd`, not as children: add their resident memory from Activity Monitor and say so.
7. **Stripped size.** `summary.json` reports the binary as built (the release profile strips symbols). Also record the size of the
   macOS `.app` bundle if you make one; not required.

## Cleaning up

The harness stops the hub and node it started and deletes its temporary folder (unless `--keep`). `apps/desktop-native-spike/target`
is several GB after a build; delete it when done.

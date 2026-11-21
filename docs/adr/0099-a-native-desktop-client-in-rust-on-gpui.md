# 0099. A spike for a native desktop client in Rust on GPUI (stage 0 accepted; replacing the Tauri shell stays proposed)
Status: Accepted
Date: 2026-11-22
Accepted: 2026-11-22, stage 0 only (the maintainer, in the merger chat, answering "Aşama 0'ı kabul et" to the question that described
stage 0: a spike on all three operating systems measuring GPUI with a native terminal, screen readers, start time, memory and size,
against the go/no-go numbers of decision 6). Stages 1 to 5 stay Proposed: the spike report goes back to the maintainer, who accepts
or rejects them then. Nothing beyond the spike may be built until then.

Touches ADR-0005 (web-first, Tauri desktop), ADR-0048 (the desktop app is a thin Tauri shell; proposed to be superseded once decision 5's
retirement step is reached), ADR-0043 and ADR-0063 (the terminal client, whose state model decision 3 extracts), ADR-0012 (VT backend),
ADR-0022 and `docs/design/performance.md` (budgets), ADR-0024 and `docs/ux/visual-language.md`, ADR-0068 (themes as token files; its
rung 6 said "a native client only if the evaluation says it pays"), ADR-0028 and ADR-0061 (device keys and endorsements), the
Proposed ADR-0097 (PR #874) and threat-model T55. No protocol message changes (decision 1).

## Context
On 2026-11-22 the maintainer asked for a desktop app "written in Rust, working on every OS, as flexible as the web UI, with GPUI or
similar". They see broken terminal screens in the Tauri app. ADR-0048 made the desktop app a webview around the hub's page, and a webview
is the operating system's engine: WebKitGTK on Linux, WKWebView on macOS, WebView2 (Chromium) on Windows. The terminal is xterm.js in
that engine, so it behaves differently per OS, and `docs/ux/terminal-fidelity.md` could only measure one Chromium on Linux (its "Limits
of the evidence"). A terminal input and scroll investigation is running now; its results feed decision 2 later and do not change the
direction proposed here.

ADR-0005 and ADR-0048 rejected a native client as "a second client to keep in step with the web one". Two things changed since:
- **A second Rust client already exists.** The terminal client (`crates/agora-tui`, ~16,700 lines) signs in with the same code, opens
  the same WebSocket, reads the same `Snapshot` and positioned events (ADR-0043), signs approvals and grants with its own key
  (`device.rs`), pairs (`pair.rs`), and renders agent terminals from `agora-vt` with mouse modes and encodings, wheel semantics,
  focus reports and pastes in pure code (`termio.rs`). Most of its files have no ratatui or crossterm in them (`client.rs`, `device.rs`,
  `pair.rs`, `picker.rs`, `termio.rs`, `tool.rs`, `split.rs`, `render.rs`, `preview.rs`, `clip.rs`); `app.rs`, `board.rs`, `files.rs`,
  `review.rs` and `pane.rs` mix state with ratatui types. A native app built on an extracted shared model is the TUI's sibling, not a
  third client from scratch.
- **The key the desktop signs with is reachable by the hub's code (T55).** The Tauri webview loads the hub's page, so a compromised hub
  can serve code that signs with the person's key. ADR-0097 proposes how a node tells such keys apart and, in its decision 4, a
  keychain signer inside the Tauri shell. A client whose every line ships in a signed release, and none from the hub, closes T55 for
  the desktop without a bridge between a remote page and a native signer.

### GPUI and its sources, as of 2026-11-22
Checked on that date (crates.io API, GitHub API, Zed's `Cargo.toml`):
- `gpui` is Apache-2.0 and is Zed's UI framework; it targets macOS (Metal), Linux (Wayland and X11; `wayland` and `x11` are default
  features) and Windows (Win32 with DirectX). Rendering is GPU-based and immediate-mode with retained views.
- **The crates.io release is stale:** `gpui` 0.2.2 was published 2025-10-22. Zed issue #66275 (asking for a new release) was closed
  2026-10-30 as "not planned" with a Zed maintainer's note: automated GPUI releases are being worked on, "for as long as we do not have
  these set up, you will need to pin a git revision".
- **Other sources:** `gpui-ce` (community edition, Apache-2.0, 0.3.3 on crates.io); `gpui-unofficial` (publishes gpui on Zed release
  tags; no licence file detected on the repository); `longbridge/gpui-fast` (Apache-2.0, GPUI extracted from the monorepo with a
  layout engine that keeps unchanged frames); `gpui-pre` (crates.io snapshots of Zed commits, 0.3.13 published 2026-11-16, by the
  gpui-component maintainer).
- **`gpui-component`** (Longbridge, the repository is now named `gpui-kit`; crate 0.10.0, Apache-2.0 on crates.io) offers about 60
  components extracted from the shipped Longbridge Pro app; 0.10 depends on `gpui-pre` =0.3.12 (or `gpui-fast` behind a feature).
- **Accessibility:** Zed's main branch now lists `accesskit`, `accesskit_macos`, `accesskit_windows` and `accesskit_unix` (AccessKit:
  NSAccessibility, UI Automation, AT-SPI) as `gpui` dependencies. Whether custom elements expose roles, names and actions to screen
  readers on all three systems is **not verified**; the forks document AccessKit support, also **not verified**.
- **Zed's application code is GPL-3.0** (its `terminal_view`, editor and workspace crates among it, see zed.dev/software-overview).
  It is never copied, read into a prompt to be reproduced, or depended on. `gpui` itself and `alacritty_terminal` (Apache-2.0, already
  Agora's VT backend through `agora-vt`, ADR-0012) are reusable. `gpui-terminal` (MIT OR Apache-2.0, 0.1.0, 2025-12-24, small) is a
  terminal element for GPUI on `alacritty_terminal`: a reference, not a dependency, unless the spike shows it saves real work.
- **Not verified here:** RTL and bidirectional text layout in GPUI (no evidence either way was found; treat as missing until the spike
  shows it), IME quality per OS (GPUI has an input handler with marked text; Linux Wayland text-input and X11 XIM behaviour unmeasured),
  behaviour without a GPU API (virtual machines, remote desktops), and the comparison claims of avaloniaui.net/gpui-compare.

## Decision
1. **A native Rust desktop client on GPUI, outside the main build.**
   - It lives in `apps/desktop-native/`, its own Cargo workspace with its own `Cargo.lock` and `deny.toml`, as `apps/desktop/src-tauri`
     does today, so GPUI and its graphics crates never enter the node, the hub or the `agora` binary. It is built in CI on Linux, macOS
     and Windows.
   - GPUI is **pinned to one git revision** of `zed-industries/zed` (recommended), behind a thin internal module of the app, so moving
     to crates.io `gpui` once Zed's automated releases land, or to a fork, is a change in one place. The spike (decision 6) also builds
     against `gpui-ce`; if upstream proves unusable, a maintained fork is chosen in the spike report, never a mix of two GPUI sources.
     `gpui-component` is adopted only if its GPUI source matches the pin, otherwise its ideas are reimplemented (Apache-2.0 permits it).
   - **Update procedure** (written in `apps/desktop-native/README.md` in stage 2): move the pin at most once a month or for a fix that
     is needed; one PR per move that states the old and new revision, the upstream changes read, `cargo deny check` and the native
     benchmarks before and after. `deny.toml` keeps the licence allow-list of the main workspace (no GPL: a Zed application crate
     pulled in by mistake fails the build), allows the one git source of the pin and nothing else, and checks advisories.
   - **Same protocol as the web and the TUI:** the same sign-in, WebSocket, frames, `Snapshot`, positioned events, commands and terminal
     relay (ADR-0027, ADR-0029); TLS trusts only the hub's CA the person confirmed, as `agora-tui`'s client does, so the loopback tunnel
     of ADR-0048 (`crates/agora-desktop-tunnel`) is not needed by it. If something turns out to need a new message, that is a separate
     schema ADR, not part of this one.
   - No webview anywhere in the native app. A page it cannot draw (an agent's preview, ADR-0088; a Deep-layer screen it lacks) opens in
     the system browser. Embedding a webview later is its own ADR, because it reopens T55's question.
2. **Terminal panes are drawn natively from the node's VT stream.**
   - The stream format is the one the web uses (raw VT plus periodic snapshot, `docs/architecture.md` §4). The screen is kept by
     `agora-vt`'s `Terminal` trait (the `alacritty_terminal` backend, live screen plus `history_len`/`history_cell`); the app draws cells
     as batched GPU text runs per style and redraws only rows that changed. Anything the trait lacks (for example damage tracking) is
     added to `agora-vt` additively in its own PR with a benchmark and the conformance tests, not worked around in the app.
   - Behaviour follows `docs/ux/terminal-fidelity.md` as the specification, item by item: **mouse reporting** in every mode (9, 1000,
     1002, 1003) and encoding (default, 1005, 1006, 1015) plus focus reports (1004), reusing `termio.rs` (moved to the shared crate);
     **wheel** scrolls local history on the main screen, is reported to the program when it tracks the mouse, and becomes arrow keys on
     the alternate screen when it does not; Shift bypasses reporting for selection; **selection and copy** over screen and history,
     word and line selection, bracketed paste, OSC 52 and OSC 8 links shown before acting (program output is untrusted); **IME** with
     preedit drawn at the cursor (CJK on all three systems, dead keys, the Turkish layout); **history** of the default scrollback;
     **input outranks output** (a key is sent before any pending output is drawn).
   - Measured against the ADR-0022 budgets on all three systems with the scenarios of `docs/design/performance.md` (decision 6 lists the
     go/no-go numbers). The terminal investigation running now adds its findings here before stage 2.
3. **A shared client view-model crate, extracted from `agora-tui`.** `crates/agora-client` (name to confirm) in the main workspace,
   with no UI dependency: the snapshot store and its reducers (positioned events in, state out), the commands and their optimistic
   states, selection and attention, the split and layout model of ADR-0068, the board and files models, the device-signing messages
   (`approval_message`, `grant_message`), pairing, and `termio`. Geometry is in cells or abstract units, never ratatui types. The hub
   connection is a module the client drives (the TUI on its loop, the native app on a background thread that posts to GPUI's main
   thread). The TUI and the native app both depend on it, so the same event gives the same state in both, and each only renders it
   (render is pure). Extraction moves code without changing the TUI's behaviour; its tests move with it. This is a crate-boundary change;
   it is part of stage 1, which stays Proposed: accepting stage 0 does not approve it.
4. **The native app holds its device key in the operating system's secure storage, out of reach of hub-served code.**
   - macOS Keychain, Windows DPAPI for the current user (Credential Manager), Linux Secret Service (D-Bus); where no Secret Service is
     running, the TUI's owner-only file, which Settings → Devices says plainly. The code lives in `crates/agora-platform/src/<os>.rs`
     behind a cargo feature the node and hub do not enable (no `#[cfg(target_os)]` elsewhere, AGENTS.md). Hardware-bound,
     non-exportable keys (Secure Enclave and TPM both do P-256, the ADR-0028 scheme) are a follow-up once the app is signed.
   - The app shows the fields it signs and signs only messages it built from them (machine, agent and action for an approval; scope and
     time for a grant; device name and fingerprint for an endorsement); it never signs bytes it was handed.
   - **Relation to ADR-0097:** complementary. Under its decision 2 this key is a `local` key; T1's promise holds for it whether or not
     ADR-0097 is accepted. ADR-0097's decisions 1 to 3 are still needed for browsers (the phone, a remote machine), which keep a `page`
     key. Its decision 4 (a keychain signer inside the Tauri shell) becomes unnecessary once the Tauri shell is retired (open question 3).
5. **Parity first, then the Tauri shell is retired; the web app stays.**
   - The web client stays the primary client (ADR-0005) for the phone (ADR-0046), remote use and any browser; features keep landing
     there first.
   - The native app must reach parity on these core flows before the Tauri shell is retired: sign in and pair; machines; agents
     (list, state, attention); terminals with splits; approvals; start an agent; board give (ADR-0095); files and changes read-only.
     Anything else opens the web window in the system browser at that place (ADR-0046 link form) until it is built.
   - **i18n** through `agora-i18n` (Fluent, en and tr at first): reuse the web's message ids where the text is the same, new ids in a
     `locales/*/desktop.ftl` only for native-only screens; the i18n lint covers it.
   - **Design tokens are shared as data, one source:** the built-in themes become theme files in the ADR-0068 format (`docs/ux/themes.md`,
     JSON v1) in one place (`design/themes/`, as `visual-language.md` §3 already says `design/tokens`), read by the web build and by the
     native app (`include_str!`); a person's own theme file loads in both. The web change (generating `tokens.css`/`themes/tokens.ts`
     values from them, the existing equality test kept) is its own PR.
   - **Acceptance gates for retirement:** both default themes (Dark and Light) and High contrast, screenshots per ADR-0024; RTL with a
     pseudo-RTL locale (mirrored layout, bidirectional labels shaped correctly); keyboard-only operation of every parity flow; screen
     readers (VoiceOver, Narrator or NVDA, Orca) read the agents list, an approval's content and its buttons; the budgets of decision 6
     met on all three systems.
   - Retirement is one PR, after the maintainer agrees: delete `apps/desktop` and, if nothing else uses it, `crates/agora-desktop-tunnel`;
     mark ADR-0048 `Superseded by 0099`; update `docs/architecture.md` §1 and §5. Until then the Tauri shell keeps receiving fixes.
6. **Staged build order, each stage its own PR:**
   0. **Spike** (throwaway unless it passes; report in `docs/reviews/`): a GPUI window on Linux (Wayland and X11), macOS and Windows that
      signs in to a real hub with a code and shows agent terminals from `agora-vt`. **Measurement fixtures** (the same on all three
      systems, scripted so the Mac and Windows sessions run them unchanged; a disposable hub and node, the fake agent):
      - F1 echo: one visible terminal, idle otherwise; keystrokes typed into it.
      - F2 one flood: one visible terminal receiving 10 MB/s while 20 other agents are listed and their 20 terminals are active
        (output at a steady low rate) in the background.
      - F3 four floods: four terminals visible at once, each receiving 10 MB/s.
      - F4 resident: 20 agents listed, four terminals visible and two open in the background, idle.
      - F5 accessibility: one list item and one button exposed to the screen reader.
      **Go/no-go numbers, as the maintainer approved them on 2026-11-22** (each on all three systems; every one is a go/no-go gate):
      - F1: keystroke to echo on screen, node on the same machine: ≤ 16 ms p95 (ADR-0022), measured to the presented frame;
      - F2 and F3: steady 60 fps, no main-thread stall over 50 ms;
      - switching to a terminal: snapshot shown ≤ 150 ms; switching agent or tab ≤ 50 ms;
      - cold start to first useful paint ≤ 500 ms (stricter than the web's cold budget of 1.5 s in ADR-0022; the maintainer approved
        500 ms for the native app); the warm start is reported too;
      - F4: memory ≤ 200 MB resident, and below the Tauri app measured the same way on the same machine;
      - idle CPU with the window visible and nothing changing ≤ 1 %;
      - stripped app binary ≤ 40 MB per platform (the existing single-binary budget);
      - IME: Japanese or Chinese input and the Turkish layout work in a terminal pane (preedit visible, committed text arrives once);
      - F5: a screen reader on each system announces the list item and the button with their names (AccessKit verified, or no-go);
      - RTL: whether GPUI lays out and shapes bidirectional text; if not, the cost of adding it upstream or in the app is estimated;
      - what happens without a GPU API (a VM or remote desktop): a usable fallback, or a documented limit.
      The numbers above (echo, frame rate, switching, start, memory, idle CPU, binary size, IME, accessibility) are all go/no-go gates
      as approved; RTL and the no-GPU case are findings the report states with their cost. A miss on any gate is a no-go for GPUI as
      measured; the report then says what would close it and compares the strongest alternative, and the maintainer decides.
   1. The shared view-model crate (decision 3); the TUI moves onto it with no behaviour change.
   2. Native app: sign-in and pairing, machines, agents list, one terminal; device key in secure storage (decision 4).
   3. Splits, approvals, start an agent.
   4. Remaining parity flows (board give, files and changes read-only), shared theme files, the gates of decision 5.
   5. Installers and signing per platform (Apple Developer ID and notarization, Windows code signing: the maintainer's accounts),
      releases and updates through ADR-0019 (never a self-updater with another key); then the retirement PR of decision 5.
7. **Risks and what stays Proposed.** Everything above except stage 0 stays Proposed until the spike report is read: the GPUI source,
   the numbers, and whether retirement is reachable.

## Consequences
- The desktop gets one terminal implementation on every system instead of three web engines, and it is the same Rust code as the
  TUI's for modes and input; terminal bugs are fixed once.
- **Three clients to maintain** (web, TUI, native) instead of two plus a shell. The shared crate keeps state and behaviour in one place,
  but every screen is drawn twice (web and native). The parity list is deliberately short, and screens outside it open in the browser.
- New dependencies: GPUI and its graphics and windowing crates, inside `apps/desktop-native` only; a secret-store dependency per OS
  behind an `agora-platform` feature. Each needs its one-line justification in its PR (AGENTS.md).
- Two releases on the desktop for a while (Tauri shell and native app), until retirement.
- `crates/agora-desktop-tunnel` and its DNS-rebinding check (T22) leave the desktop path once Tauri is retired.

### Risks
- **Fast-moving upstream without releases:** GPUI's API changes often and crates.io lags; mitigated by the single pin, the internal
  facade, the monthly update procedure, and the fork fallback.
- **Windows and Linux maturity:** Zed's Windows and Linux ports are younger than macOS; X11/Wayland differences, fractional scaling,
  IME and GPU-less machines are spike checks.
- **Accessibility:** if AccessKit integration is incomplete, the app fails a retirement gate; the web stays the accessible desktop path
  until then.
- **RTL:** unverified in GPUI; may need upstream work. No RTL locale ships yet, so the gate uses a pseudo-locale.
- **Licensing:** Zed's application code is GPL-3.0; cargo-deny's licence list and review keep it out. Contributors must not port
  `terminal_view` or other Zed app code.
- **Signing costs** are the maintainer's (as in ADR-0048); until then builds are published as unsigned and the macOS keychain asks again
  per ad hoc build, as the Tauri app does today.

### Threat-model impact
- **T55:** for the native app, mitigated: no hub-served code runs in it, and its key sits in OS storage. T55 stays open for browsers
  and for the Tauri shell until it is retired; ADR-0097 covers those.
- **T1:** the native app's signatures are of keys out of the hub's reach, as the TUI's are.
- **T9 (supply chain):** grows by GPUI's tree; one pinned git source, cargo-deny, signed releases.
- **T22:** shrinks once the loopback tunnel is no longer used by the desktop.
- Terminal output (OSC 52, OSC 8, titles) stays untrusted text in the native app as in the TUI (`termio.rs`): nothing is executed or
  opened without the person seeing it first.
The threat model is updated in the PRs that build these, not by this ADR.

## Alternatives considered
| Option | For | Against |
| --- | --- | --- |
| Keep Tauri, fix the web terminal per engine | no new client; the investigation may fix the worst bugs | three engines stay three engines; T55 needs ADR-0097's bridge into the shell; WebKitGTK quality is the distribution's |
| Electron or CEF (one Chromium everywhere) | the web client unchanged; one engine on every OS | ~100 MB+ per download and its own update burden; the page is still the hub's, so T55 stays; ADR-0048 already rejected it |
| Iced | pure Rust, MIT, Elm-style fits "render is pure" | no shipped terminal at Zed's level; IME and accessibility were incomplete when last checked (not re-verified here) |
| Slint | mature, good embedded story, accessibility work | GPL/commercial/royalty-free licence terms need review; declarative markup separate from Rust; terminal is custom work |
| egui | simple, immediate mode | weak text shaping, IME, RTL and accessibility for a dense app; redraws every frame |
| Dioxus (native/Blitz) | React-like, web-like flexibility | native renderer young; terminal performance unknown |
| Freya (Skia) | Skia text shaping incl. bidi | small community; Skia's build weight |
| Makepad | very fast GPU rendering | own DSL and ecosystem; small community; IME and accessibility unclear |
| GPUI (proposed) | ships Zed (terminal, editor, IME, multiple windows) on three OSes; Apache-2.0; GPU text; AccessKit landing | no current crates.io release; API churn; RTL and accessibility unverified |
None of the claims about the other toolkits were measured for this ADR; the spike measures only the choice it makes and, on a
no-go, the strongest alternative.

## Open questions for the maintainer
1. **Accept stage 0 now and the rest after the spike, or all now?** Recommended: stage 0 now, the rest after reading the spike report.
2. **GPUI source:** recommended, a pinned git revision of `zed-industries/zed`, `gpui-ce` built in the spike as the fallback; switch to
   crates.io `gpui` when Zed's automated releases exist.
3. **ADR-0097 decision 4 (a keychain signer in the Tauri shell):** recommended, do not build it if this ADR is accepted; the native app
   is where the desktop gets a `local` key. ADR-0097 decisions 1 to 3 stand on their own.
4. **Where the code lives:** recommended, `apps/desktop-native/` (own workspace) for the app and `crates/agora-client` for the shared
   model; the alternative `crates/agora-desktop` would put GPUI in the main workspace and lock file.
5. **The go/no-go numbers** of decision 6, especially memory (≤ 200 MB) and binary size (≤ 40 MB): recommended as written.
6. **Parity list:** recommended as written in decision 5; anything else opens in the browser until built.
7. **RTL as a retirement gate** while no RTL locale ships: recommended yes, with a pseudo-locale, because vision goal 6 promises full RTL
   on the desktop.

## References
All checked or quoted on 2026-11-22.
- Zed issue #66275, GPUI crates.io releases: https://github.com/zed-industries/zed/issues/66275
- GPUI on crates.io (0.2.2): https://crates.io/crates/gpui
- gpui-ce: https://crates.io/crates/gpui-ce
- gpui-unofficial: https://github.com/gpui-archipelago/gpui-unofficial
- gpui-fast: https://github.com/longbridge/gpui-fast
- gpui-component (now gpui-kit): https://github.com/longbridge/gpui-component
- gpui-terminal: https://docs.rs/crate/gpui-terminal
- Zed's licences per component: https://zed.dev/software-overview (not re-read for this ADR)
- Avalonia's GPUI comparison: https://avaloniaui.net/gpui-compare (not re-read for this ADR; its claims are not relied on)
- `docs/ux/terminal-fidelity.md`, `docs/design/performance.md`, `docs/ux/themes.md`, `docs/security/threat-model.md` (T1, T22, T55)

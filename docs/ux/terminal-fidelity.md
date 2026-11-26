# Terminal fidelity: does the window's terminal feel like a real one?

Goal (the maintainer's): the terminal in the web window feels exactly like a real terminal: mouse, wheel, selection, paste, the
font, everything. This page is the audit of `apps/web/src/terminal/` against that goal, what this change fixed, and what lives
elsewhere (Phase 3, the node, the terminal client and the hub) with a proposed fix for each. Binding context: ADR-0012 (VT backend),
ADR-0029 (the hub relays terminal bytes), ADR-0072 (a shared terminal takes the smallest window's size), ADR-0022 and
`docs/design/performance.md` (budgets), `docs/ux/visual-language.md`.

## How it was measured

Every "before" below was measured on `main` at `0363f668` (xterm.js 6.0.0, DOM renderer) and every "now" on this change, in the same
way: the real `TerminalView` on a fake source in headless Chromium (`scripts/terminal-fidelity/`, its checks; run it with the steps in
its README), plus a real program on a real PTY in the loop: `mouse-echo.py` switches on mouse reporting (modes 1000, 1002, 1006, 1004)
and prints the bytes it receives; the page's clicks, drags, wheel notches and Shift+drag are what it printed back. Unit tests cover the
pure parts (`wheel.test.ts`, `links.test.ts`, `keys.test.ts`).

Limits of the evidence: one Chromium (Linux, software GL; Firefox and WebKit were run later for the pointer and the wheel, see
"Pointer and wheel in every engine"); a wheel from a real trackpad cannot be produced through the DevTools
protocol (it always reports a whole notch), so a trackpad over the main screen is left to xterm.js's own scroll bar as before, and
the rule that tells a trackpad from a wheel (`NotchDetector` in `wheel.ts`) is tested with recorded-like values, not a real device; a high-density screen cannot be
emulated for the graphics-card renderer (see "Rendering" below); no macOS or Windows browser was run.

## Audit

Verdicts: **wrong** (a real terminal does otherwise and it matters), **partial**, **ok** (as a real terminal), **n/a** (cannot be
done in a web page). "Now" is the state after this change.

### Mouse and wheel

| Feature | Real terminal | Agora before (evidence) | Verdict | Now |
| --- | --- | --- | --- | --- |
| Mouse modes 9, 1000, 1002, 1003 | the program gets presses, releases, drags, motion as it asked | xterm.js reports all four; SGR press, drag (`<32;..M`) and any-motion (`<35;..M`) arrived intact (lab) | ok | unchanged; real program through a PTY receives click, drag, wheel (check 1-4) |
| SGR (1006) encoding | `ESC [ < b ; x ; y M/m` | correct | ok | unchanged |
| Default (X10) encoding | `ESC [ M` and three raw bytes | **nothing was sent**: xterm.js hands these to `onBinary`, which the renderer never subscribed to (`renderer.ts:106` only `onData`; lab: click with mode 1000 and no 1006 sent no bytes) | **wrong** | `onBinary` is sent as raw bytes (`binaryBytes`, a column over 95 stays one byte); check 5. This is the X10 form only |
| urxvt (1015) and UTF-8 (1005) encodings | extended coordinates | **not supported**: xterm.js has no such encodings; a program that asks for them gets the default (X10) bytes, which it will misread | partial | **1015 done** (see "Pointer and wheel in every engine" below): `ESC [ 32 ; 6 ; 4 M` for a click on (5,3), release as button 3. 1005 unchanged: the X10 bytes, the same below column 95 |
| Wheel to a program that has the mouse on | buttons 4 and 5, once per notch | one SGR 64/65 per notch (lab) | ok | a trackpad swipe no longer floods the program: one report per row height of swipe (`wheel.ts`, tested) |
| Wheel with no mouse mode, main screen | history scrolls, three lines a notch | three lines per 100 px notch (lab: `top line 2974 -> 2971`) | ok | three lines a notch in every engine (it was 7 in Firefox and WebKit, see below), counted in `wheel.ts`; a trackpad's pixels still go to xterm.js's scroll bar (smooth, as before); Shift+wheel, which browsers send sideways, scrolls too; the history stays put when new output arrives |
| Wheel over the alternate screen with no mouse mode (mode 1007) | arrow keys, several per notch | xterm.js sent **one** arrow per notch, so a pager or `man` crawled | partial | three arrows per notch, in the cursor-key form the program asked for (DECCKM), never more than the rows; check 2. In Firefox and WebKit it was 5 or 6 until "Pointer and wheel in every engine" |
| Shift+drag forces local selection while the program has the mouse | yes | works: no bytes reach the program, selection made (check 1 with the real program) | ok | unchanged; Shift+wheel scrolls the history 3 lines and sends nothing (check 2) |
| Touch scrolling (phone, tablet) | a drag scrolls | **did not scroll**: the browser took the drag as a page pan (`pointercancel`), xterm.js 6 does nothing with it (lab, 390 px touch: no movement) | **wrong** | a drag scrolls the history one line per row height, or sends arrows on the alternate screen (`TouchScroll`); `touch-action: none` on the screen; check 5 |
| Focus events (1004) | `ESC [ I` / `ESC [ O` | works (lab) | ok | unchanged; the real program receives them |
| Right click | a menu or paste | the browser's menu (copy, paste); with mouse mode on, button 2 is reported | ok | unchanged |
| Middle click | pastes the selection (X11) | the browser does it on Linux; none elsewhere, which matches the native terminals of those systems | ok | unchanged |

### History, selection, clipboard

| Feature | Real terminal | Agora before | Verdict | Now |
| --- | --- | --- | --- | --- |
| History kept | configurable, often 10 000 lines | 2000, fixed (`renderer.ts:63`; the node keeps 2000 too, `agora-vt/src/alacritty.rs:16`) | partial | 5000 by default; 1000 to 20 000 in "Terminal settings" (memory: this window only, about 1.5 MB per 1000 lines of 120 columns) |
| Scroll position when output arrives while scrolled up | the view stays | stays (lab: `line 2902` before and after 50 new lines) | ok | unchanged; **new** "Jump to latest" button appears while scrolled up |
| History after opening or reconnecting | what happened before | only what arrived since: the repaint is the live screen (`agora-node/src/terminal.rs:175`) | see Phase 3 | replayed on open: a first open, a reload or a remount asks the node for its history before the repaint; a reconnect keeps the window's own (ADR-0100 step 3, Phase 3 item 1) |
| Double / triple click, Alt+drag | word / line / rectangle | all work (lab) | ok | unchanged (checks 2) |
| Copy | Ctrl+Shift+C (Cmd+C on a Mac); Ctrl+C is the program's | **no shortcut**: Ctrl+Shift+C copied nothing (lab: clipboard empty) | **wrong** | Ctrl+Shift+C, Ctrl+Insert, Cmd+C copy the selection; Ctrl+C still interrupts; check 2 |
| Copy on select | X11 terminals do it for the primary selection | none | partial | a setting (off by default: the browser has no primary selection, so it would replace the clipboard) |
| Selection colour | clearly visible | `--surface-2` on `--surface-0`: nearly invisible | **wrong** | the accent at 40% (20% when the terminal is not focused); screenshots |
| Paste, bracketed (2004) | wraps in `ESC [ 200 ~` when asked | works, but **pasted text was not cleaned**: `a ESC[201~ rm -rf ~ LF b` ended the bracket early and the rest reached the program as typed (found in review, reproduced) | **wrong** | every paste (keyboard, menu, the touch key bar) loses ESC and the other control characters except tab and line breaks (`sanitizePaste`, tested with that payload; check 2) |
| OSC 52 (a program writes the clipboard) | applied silently by most | ignored | partial | **policy**: a program's output is untrusted (threat model), and clipboard text ends up in a shell, so it is never written by itself: the window shows the **whole** text (up to 8192 characters, with its length) with Copy / Ignore, and copies only on Copy. An offer with any control, line-break, bidirectional or invisible character is dropped, a new offer never replaces a pending one, and at most one is shown per 2 s. A request to read the clipboard is never answered |
| OSC 8 links and plain addresses | click or Ctrl+click opens | OSC 8 only, through xterm.js's default handler (a confirm box, then opens whatever the scheme); plain addresses not linked | partial | OSC 8 **and** plain addresses are links; Ctrl/Cmd+click opens, a plain click never does (it may start a selection); only `http` and `https`, no credentials, opened with `noopener,noreferrer` (`links.ts`, tested); the address is shown at the terminal's bottom edge while the pointer is over a link; a `javascript:` link does nothing, not even a prompt; check 2 |
| Title (OSC 0 / 2) | the window title | not used, `document.title` untouched | ok | unchanged (never raw into the page); check 2 |
| Bell | a sound or a flash | nothing | partial | a brief ring round the terminal (no sound) |
| Find in history | the terminal's find | none | **wrong** | Ctrl+Shift+F (Cmd+F on a Mac): find bar, Enter / Shift+Enter, Esc; matches are highlighted |

### Keys

| Feature | Real terminal | Agora before | Verdict | Now |
| --- | --- | --- | --- | --- |
| Ctrl+arrows, Alt+arrows, Alt+letter, Shift+Tab | xterm forms | correct (lab) | ok | unchanged |
| Shift+Enter, Ctrl+Enter, Ctrl+Backspace distinguishable | CSI u keyboard protocol or modifyOtherKeys, when the program switched it on | all were a plain `\r` / `\b`; the switch-on requests were ignored and the query `CSI ? u` unanswered, so a coding agent could not tell Shift+Enter from Enter | **wrong** | the flag stack (`CSI > / < / = flags u`), `CSI ? u` reply and `CSI > 4 ; n m` are honoured; only then are these keys sent as `CSI 13 ; 2 u` or `CSI 27 ; 2 ; 13 ~`; a program that did not ask gets the same bytes as before (`keys.ts`, tested) |
| Mac Option as Meta | per setting | off (Option composes characters) | ok | unchanged |
| IME composition | composition shown at the cursor, committed as text | works: composing shows, nothing is sent until the commit (CDP lab) | ok | unchanged |
| Application cursor keys (DECCKM) | `ESC O x` | works | ok | unchanged |

### Rendering

| Feature | Real terminal | Agora before | Verdict | Now |
| --- | --- | --- | --- | --- |
| Row height | 1.0 of the font's height | 1.1 (`renderer.ts:55`): rows 18 px for a 17 px font | **wrong** | 1.0 by default; 1.1 and 1.2 in settings |
| Box-drawing and block characters | join exactly, drawn to the cell | **gaps**: at 1.1 a block of full-block characters had 7.4% of its area empty (horizontal gaps and coloured vertical seams), at 1.0 still 1.9% (`blocks-page-1.1.png`) | **wrong** | the graphics-card renderer (`@xterm/addon-webgl`), when switched on, draws them itself: 0.00% empty at every size measured (13, 14, 15 px); off, the default row height 1.0 leaves 1.6-1.9%; it is **opt-in** (Terminal settings, off by default until a person has seen it on a real scaled screen), and it falls back to the page's own drawing when the browser refuses a context or sizes the canvas wrongly |
| Colours: true colour, 256, bright bold | exact | exact | ok | unchanged |
| Minimum contrast | none: a program's colours are drawn as chosen | forced `minimumContrastRatio: 4` (`renderer.ts:57`) lifted any colour; `38;2;30;30;70` was drawn as `110,110,135` | **wrong** | off by default; "Make hard-to-read colours readable" in settings |
| Wide characters and emoji | two cells, as on the node's screen | **one cell** (xterm.js' default width table is older): `A😀B` put `B` in the wrong column, so every row with an emoji was misaligned against the node | **wrong** | the Unicode 11 width table is on; an emoji is 15.6 px = two cells; check 3 |
| Powerline and icon glyphs | from the user's font | the theme's font then the icon fonts (`fonts.ts`) | ok | unchanged; the graphics-card renderer also draws the Powerline arrows itself |
| Ligatures | the font's, where it has any | the browser's default (`normal`) with the page's text drawing; none with the graphics-card renderer | partial | unchanged (off in the default renderer; not asked for) |
| Letter spacing | 0 | 0 | ok | unchanged |
| Cursor shapes and blink, DECSCUSR | block, bar, underline, steady or blinking | all six forms apply (lab) | ok | unchanged |
| Synchronized output (2026) | a frame appears whole | works (nothing painted while held) | ok | used for the repaint below |
| Fractional zoom / high-density screens | crisp | the page's text drawing: crisp (cell 7.80 x 17.59 css px at 1.25, 21.99 device px high) | ok | the graphics-card renderer could not be shown on a scaled screen in headless Chromium (its canvas stays at 1x and draws nothing there); the window checks the canvas size and falls back, and **a person should look at it on a real scaled screen before turning it on by default** |
| Repaint on reconnect or after falling behind | no flicker | **an empty screen was painted between the clear and the repaint**: the renderer did `reset()` and then wrote the snapshot, and a frame sampler saw 40 blank frames in 40 snapshots | **wrong** | one write wrapped in synchronized output, no reset; 0 blank frames in the same run. The window's history and modes survive a repaint |
| Fit: columns and rows | whole cells that fit | 111 x 26 in 900 x 500: 18 px of width unused (the scrollbar strip plus the part of a cell), no off-by-one (lab) | ok | unchanged |
| Smaller terminal in a bigger window (ADR-0072) | letterbox | top left, a hairline at its right and bottom edge, the pane's surface around it | ok | unchanged (`letterbox-{dark,light}.png`) |

## What this change did (web client only)

Files: `apps/web/src/terminal/` (`renderer.ts`, `wheel.ts`, `links.ts`, `clipboard.ts`, `keys.ts`, `prefs.ts`, `FindBar.tsx`,
`TerminalSettings.tsx`, `TerminalView.tsx`, `frames.ts`), `design/app.css`, `phone/phone.css`, strings in `locales/{en,tr}/web.ftl`.

- New dependencies, all from the xterm.js project, all MIT, versions pinned like `@xterm/xterm`: `@xterm/addon-webgl` (exact box and
  block drawing), `@xterm/addon-unicode11` (character widths that match the node), `@xterm/addon-search` (find),
  `@xterm/addon-web-links` (plain addresses). They load with the terminal chunk, which is already lazy.
- **The renderer decision in `docs/design/performance.md` stands as the default**: the page's own drawing. The graphics-card renderer is
  an opt-in setting, because it could not be confirmed on a real GPU or a scaled screen, a browser allows about sixteen graphics contexts
  per page (with more panes the oldest lose it and go back to the page's drawing, which blanks that pane for a moment), and the
  fallback check (250 ms after start and after each resize) cannot catch a browser that settles later. What it fixes is real (see
  Rendering). Flip `DEFAULT_PREFS.gpu` in `prefs.ts` once it is confirmed.
- Settings the user can change (browser-local, `agora.terminal.prefs`): row height, history, readable colours, copy on select, graphics
  card (off by default). They sit in a "Terminal settings" menu in the pane's header, next to the text size.

## Pointer and wheel in every engine (web client)

The first audit ran one Chromium. A later run of the same probes in Chromium, Firefox and WebKit (Playwright 1.63, a real shell on a
real PTY behind a throw-away hub and node, 100, 125 and 150 % zoom) found three defects in the window, fixed here in
`apps/web/src/terminal/` (`renderer.ts`, `wheel.ts`, new `mouse.ts`), `design/app.css` and `phone/phone.css`:

- **The lowest pixels of the bottom row were dead to the mouse** (125 % in Chromium and Firefox, 100 % in WebKit: about 5 px of the
  last row reported nothing). xterm.js gives its own element the class `terminal`, so our `.terminal` rule (padding, full height,
  clipping) applied to it as well as to the element we mount it in. The fit (xterm's FitAddon) counts the padding of xterm's element
  only, so the screen was sized for one padding, sat inside two, stuck out of xterm's box and was clipped by ours; a click there hit
  our element. Our element is now `.terminal-host` (all uses, the loading placeholders and the scripts renamed), and its padding is on
  xterm's element, where the fit looks for it. Same rows and columns as before; the text sits 8 px from the pane's edge instead of
  16 px at the top and left. A test keeps any bare `.terminal` selector out of the stylesheets (`hostClass.test.ts`). Proof: a click
  0.5, 2, 3.5 and 5 px above the screen's bottom edge on the last row was reported in all three engines at all three zooms (before:
  lost in the three failing cases, the pointer on our element).
- **A wheel notch was not three lines in Firefox and WebKit.** Both report a 100 px notch with the legacy `wheelDeltaY` at 300 (three
  times `deltaY`, for a trackpad too, so it counts nothing) where Chromium reports 120. `wheel.ts` took that for a trackpad's pixels
  (5 or 6 arrows on the alternate screen), and xterm.js's own scroll bar, which handles the main screen, divides by 120 (7 lines).
  Every wheel event over the terminal now goes to `wheel.ts` first, in a capture listener. `NotchDetector` tells a wheel from a
  trackpad without knowing a device's notch size: a step of 40 px or more after a pause (200 ms) is a notch, and its size is the
  notch's; within a gesture a step is a notch only right after a notch and only as a whole multiple of that size (Chromium's legacy
  120s count as they are); everything else, and everything after a trackpad's step in the same gesture (momentum included), is pixels.
  A notch is 3 lines, 3 arrows or one report. Pixels over the main screen go on to xterm.js's scroll bar as before (smooth); elsewhere
  they are one line per row height. A first version counted every step of 40 px or more as a notch, which made a fast trackpad swipe
  jump three lines at a time (found in review). Also: Shift+wheel, which browsers turn into a sideways scroll, now scrolls the history,
  and Ctrl+wheel and a trackpad pinch are left to the browser (zoom) instead of being cancelled. Proof: one notch is 3 history lines,
  3 arrows (`\eOA` with DECCKM, `\e[A` without) and one SGR report in all three engines; `wheel.test.ts` covers each engine's notch,
  a fast swipe with momentum, a wheel spun fast, Shift and Ctrl. Limit: the browsers' synthetic wheel is always a 100 px notch, so no
  real trackpad or wheel was driven; a trackpad gesture whose very first step is 40 px or more takes that one step as a notch.
- **urxvt (1015) mouse encoding.** xterm.js logs "not supported" and keeps the encoding it had. xterm.js's own encoding still decides:
  SGR (1006) or SGR pixels (1016) win whenever they are on (as in VTE, and what programs got before); where xterm.js would write the
  classic form while the program has 1015 on, the report is written as `ESC [ Cb ; Cx ; Cy M` from xterm.js's event, in cells with no
  column limit (`followMouseEncoding` in `mouse.ts` replaces xterm.js's classic encoding through its mouse service, which is not public
  API; without it xterm.js keeps its own). So `1006 → 1015` is SGR, `1006 → 1015 → reset 1006` is urxvt, `1016 → 1015` stays SGR
  pixels, and a reset of 1015 or a full reset goes back to the classic form. A first version followed the encodings on its own and could
  disagree with xterm.js (classic while xterm.js wrote SGR, pixels passed off as cells; found in review). Tested against a real xterm.js
  instance for each of these transitions (`mouse.test.ts`). Proof: a click on (5,3) with 1015 is `\e[32;6;4M\e[35;6;4M` in all three
  engines.

Not changed by this, and failing in all three engines when it was written: a window opened, reloaded or reconnected while a program
has the mouse, focus reporting or the alternate screen on did not know it, and its history started at the repaint. Fixed since by
ADR-0100 (Phase 3 item 1 below: the repaint carries the modes, and the history comes before it when a window asks).

## The terminal client (TUI): what a program now gets

Done in `crates/agora-tui/src/termio.rs` (pure, unit-tested), `app.rs` (the pane's pointer and focus), `input.rs` (the host terminal's
mouse reports, now with button, Shift/Alt/Ctrl and motion), `preview.rs` (reads the modes), `pane.rs` (`link_at`) and
`crates/agora/src/tuicmd.rs` (sends, opens, shows). No protocol or schema change.

- **Mouse modes** 9, 1000, 1002, 1003 with the encodings default (X10), UTF-8 (1005), SGR (1006) and urxvt (1015). The modes come from
  the program's own output: `ModeScanner` reads `CSI ? Pm h/l` (also `ESC c`, `CSI ! p`) across chunk boundaries, in the same
  bytes the client feeds its emulator, the node's repaint included, which sets or resets 1000/1002/1003, 1005/1006 and 1004 (Phase 3, item 1, step 1). It is tested against
  the emulator for every byte that can sit inside an escape (`ESC <byte> c` resets only where the emulator resets). The client keeps
  asking its own terminal for presses, drags and the wheel; it asks for motion with no button (1003) only while the program in front
  asked for it. A coordinate that does not fit the form (column over 223 in the classic one) is dropped, never wrapped.
- **Who gets the pointer.** With a mouse mode on, the pane not scrolled back, and no palette or box over the pane, presses, releases,
  motion (1003) and the wheel go to the program; drags and releases only of a press the program got. **Shift** keeps everything local
  (selection and the OSC 52 copy the client already had, wheel scrolls the history) and a Shift-held event is never sent: while the
  program has a press, a Shift- or Ctrl-held drag goes nowhere and a Shift- or Ctrl-held release is sent as a plain release (the
  modifiers cut off) so the program's button does not stay down. A press the program has never loses its release: the wheel in the
  middle of it is the program's and does not end it; when the pane stops being under the pointer's hand (the palette opens, a box
  covers the pane, the mouse is given back to the terminal, another tab comes to the front) the program gets one release (no
  modifiers, where it last was) in the encoding it had, in the pane that had the press, even if that tab is no longer in front.
  Nothing is sent when the program itself switched the mouse off. **Ctrl+click** is the link gesture and is never sent. With no mouse mode: the wheel on the alternate screen is
  arrow keys, three a notch, in the cursor-key form the program asked (DECCKM); on the normal screen it scrolls the client's history,
  as before. Middle and right buttons do nothing in the client itself.
- **Focus events** (1004): `ESC [ I` / `ESC [ O` when the pane gains or loses the keyboard (the tab comes to the front with the
  keyboard in the pane, or the keyboard goes to the tree, the inspector or the palette), only if the program asked.
- **Paste**: bracketed only when the program asked (the emulator's own flag); every paste loses ESC and every other control
  character except tab and line breaks, C1 included, so it cannot end its own bracket.
- **Links**: plain `http` and `https` addresses on one drawn row, printable ASCII only, none of `^ " ( ) ! < > |`, and `%` only as a percent-encoding (`%20`); `&` is allowed (no shell reads the address; measured on Windows: `rundll32` leaves `%NAME%` literal, the shape is refused anyway), no credentials,
  at most 2048 characters. Only the RFC 3986 characters minus those are accepted (letters, digits, `-._~:/?#[]@$'*+,;=`; no space,
  backtick, brace, backslash or non-ASCII byte). `%` is refused whole, so a percent-encoded address is not offered as a link (it can still be copied).
  An address with any such character is no link at all, never a shorter one. Ctrl+click shows the whole address in the footer; a
  second Ctrl+click on the same address opens it only if nothing happened in between (a key, another pointer event, a tab switch,
  a different footer text, or 10 seconds forget it) and the footer with the address has been drawn (clicks in the same batch of
  events cannot open it; clicking the same address again shows it again and restarts the 10 seconds; `LinkConfirm`). The opener (`agora_platform::open_url`) refuses anything else
  and, on Windows, runs `%SystemRoot%\System32\rundll32.exe url.dll,FileProtocolHandler` (the address is one argument, never `cmd.exe` and never
  found through `PATH`; before, `cmd /c start` let `&` and `%VAR%` through). A plain click never opens or asks; nothing opens by itself. An address broken over two rows is not joined.
- **Title** (OSC 0 / 2): drawn after the tabs; control, bidirectional, zero-width and other invisible characters (including
  U+206A-206F, tag characters, U+3164, U+2800, U+00AD, U+FE0F, U+180E) as visible escapes, cut to 60 characters and to the room by
  display width. The count of agents waiting is drawn first, in a place of its own; the title gets what is left.
- **Leaving**: one reset sequence (`backend::RESET`: mouse 1000/1002/1003/1006, paste 2004, focus 1004, cursor, alternate screen) is
  written when the screen is dropped: a quit, an early return (the sign-in-failed message is printed after the screen is left), a panic
  on the thread that made the screen (a hook writes it before the message; a panic on another thread leaves the screen as it is), and on
  Unix a SIGTERM, SIGHUP or SIGINT first writes it and puts the terminal's settings back, then ends the process by that signal as
  before (`agora_platform::restore_terminal_on_stop`; it needs no loop to poll for it, so no wait can miss it; tested with a real pty and
  real signals). Not covered: SIGKILL, and closing a Windows console window or Ctrl+Break.

How it was checked: unit tests on encoders, the scanner (split chunks, strings, resets), paste, links, titles, and the pane's pointer
rules against a fake program's byte stream; a mutation run (each guard reverted alone: paste cleaning, bracket only when asked, mouse off,
Shift, Ctrl, scrolled-back, palette, release and press forgotten, wheel arrows only on the alternate screen, DECCKM, X10 modifiers, classic
column limit, SGR release button, reset and what resets, focus only when asked, link scheme, credentials, forbidden characters, ASCII,
the two-click rule, title escape, cap and width, the waiting count's priority, the reset sequence and its drop, the opener's refusal)
made a test fail every time.
**Not** checked: a real vim, tmux or htop in a real terminal emulator, any host terminal's own handling of Shift+drag, Windows or macOS
consoles, a real `xdg-open` or `rundll32` from the TUI (the Windows opener compiles for Windows, `cargo check --target x86_64-pc-windows-gnu`,
but only Windows CI can run it); the glue in `tuicmd.rs` (sending the focus bytes, passing the pure rules the events, the box over the
pane, polling for a stop request, switching 1003 on and off) has no automated test.

## Phase 3: what the web client cannot fix

In priority order; each needs work outside `apps/web`. Item 1 (the modes and the history in the repaint, ADR-0100) is built; nothing else here is.
Items 1, 3 and 6 change the protocol or a recorded decision, so each needs an ADR first.

1. **The repaint a window receives loses the program's modes and the history.** `subscribed_snapshot`
   (`crates/agora-node/src/terminal.rs:175`) writes the screen, the cursor, bracketed paste and application cursor keys, and nothing else;
   `Modes` (`crates/agora-vt/src/lib.rs:70`) has only those two plus `alt_screen`, which the repaint does not send either. So a window opened (or
   reconnected) while a full-screen program is running gets **no mouse reporting** (1000/1002/1003/1006), no focus events (1004), no
   alternate-screen switch, no cursor shape, and no history: the wheel cannot reach an editor or a monitor that was started before
   the window opened, until the program switches the mode on again, and the history starts empty. Proposed fix: extend `Modes` with
   the mouse tracking and encoding modes, focus reporting, cursor style and the alternate screen, emit them in the repaint, and add a
   history replay (the node already keeps 2000 lines, `alacritty.rs:20`; "never read by clients" is the comment to remove) sent before
   the repaint and bounded by a client-chosen count. Needs an ADR (it extends the snapshot). The ADR is ADR-0100 (modes in the repaint, then a
   history replay the window asks for).
   **Step 1 is done** (ADR-0100 decision 1): `Modes` and `Cursor` carry every mode the node's emulator tracks (mouse tracking and
   encoding, focus, alternate scroll, the alternate screen, keypad, autowrap, insert, newline, the kitty keyboard flags, the cursor's
   shape and blink), and the repaint writes each one as an explicit set or reset after entering the program's screen and drawing it,
   so a window that opens, reloads or reconnects gets the mouse, the wheel, focus reports and the alternate screen (D1, D2, D3, D5 of
   the ADR; `scripts/e2e-terminal-repaint.mjs` in Chromium and the terminal client, node and client tests). The scroll region
   (DECSTBM) and origin mode (DECOM) are carried too (M7.10): after the modes and before the cursor, whose row is then relative to the
   top margin; a cursor outside the region under origin mode (DECRC can leave it there) keeps origin mode off at its absolute row
   (pinned by a node test). Not carried: modes the node does not track (9, 1015, modifyOtherKeys, 47/1047); in the
   web window xterm.js switches 9 off on the repaint's 1000/1002/1003 resets (the terminal client keeps it); the keyboard flags are
   the current ones, not the program's stack (a pop after a late attach gives the window none, pinned by a node test); mode 12 is not
   written (xterm.js would keep it as the window's own blink setting).
   `agora terminal attach` writes the repaint to the user's own terminal. It leaves out the repaint's keyboard pop, pushes a
   keyboard entry of its own, and on detach pops that entry together with any the program left on the main screen, then gives the
   whole screen back as the scroll region (the cursor kept) and resets the modes. On Windows the console host (ConPTY) sits between `agora` and the user's terminal, and it is a terminal itself:
   - it adds its own start (`CSI ? 9001 h`, `CSI ? 1004 h`);
   - it keeps the pen, cursor visibility, autowrap, insert, newline, the character set and focus reporting itself, re-rendering them
     for the real terminal;
   - it passes the other modes through (mouse tracking and encodings, the alternate screen, keypad, bracketed paste, the kitty
     keyboard flags, the cursor shape);
   - at the end it writes its own reset (`CSI ? 9001 l`, `CSI ? 1004 l`).

   So a Windows user's terminal ends a session with the same modes as elsewhere, as `crates/agora/tests/cli.rs` checks on all three
   systems.
   **Step 2 is done on the node** (ADR-0100 decision 2): a `TerminalSubscribe` with `history_lines` gets the newest
   `min(history_lines, kept)` lines (at most 512 KiB, in kind 4 frames of at most 16 KiB, at most 32) before the repaint. They are
   re-encoded from the scrollback's cells with their colours, and a line the node holds as wrapped is written without a break, so the
   window rewraps it. A program on the alternate screen gets none (the backend cannot read the main screen under it), and a resync
   never carries history. Measured with `agora-bench history`; the figures are in `docs/design/performance.md`.
   **Step 3 is done** (the clients, ADR-0100 decisions 2 and 4). The web window asks for its own history setting (1000 to 20 000
   lines) when its terminal has none of its own (a first open, a reload, a remount), and for none on a reconnect or after falling
   behind, where it keeps its own (`apps/web/src/net/live.ts`, `historyToAsk`). It holds the kind 4 frames with the snapshot, which
   already waits for its size frame (`frames.ts`, `snapshotAfterSize`), and writes history and repaint in one synchronized write (mode
   2026, `renderer.ts`, `reset`), so a reload is one paint at the node's width. It never waits for history: an older node sends none and
   the snapshot is painted as before. History is paired with the snapshot of its own subscription only (`frames.ts`,
   `historyPairing`): when the connection is lost or the relay ends before the snapshot, it is dropped, so it never reaches a later
   subscription's snapshot, and at most one replay (512 KiB) is ever held (Codex review of #964; the terminal client does the same,
   `client.rs`, `HeldHistory`). The terminal client asks for its scrollback (2000 lines) on a fresh pane and for none when it
   watches again; it holds the history with the screen until the size that follows, feeds both to its emulator once (the wheel reads
   that scrollback), and keeps only the screen as the one it feeds again when the screen's own size follows it (a later size only
   resizes the emulator, #977), clearing its screen first without moving it into the history (`crates/agora-tui/src/preview.rs`, `PaneFeed`). This is the agent's terminal panel; the full-screen terminal opened
   from it (`crates/agora/src/tuicmd.rs`) still asks for none. Tested by D4 in `scripts/e2e-terminal-repaint.mjs` (after a reload the
   first line of a 300-line output is in the window's history; Chromium), `crates/agora-tui/tests/history_replay.rs` (a probe on a
   real pseudo-terminal: the pane's scrollback is the node's, stays the node's through a taller and a shorter terminal, and a repaint
   fed again at its own size never copies the screen into it) and web unit tests. Still not replayed: the main screen's history while
   a program is on the alternate screen (the backend cannot read it). Screens: `docs/ux/screens/adr100-history-after-reload-light.png` and `-dark.png`.
2. **Nobody owns the answers to terminal queries.** The node's emulator drops `PtyWrite` events (`crates/agora-vt/src/alacritty.rs:41`,
   only titles are kept), so a program that asks for the device attributes, the cursor position, the colours or the keyboard protocol
   when **no window is attached** gets no answer, and with **two windows** it gets two, each with its own theme's colours. Proposed
   fix: the node answers the queries itself from its emulator and the window's emulator suppresses its own replies for queries the node
   answers; the window keeps only the replies that depend on the window (its size in pixels). No protocol change if the node answers
   in its PTY writer; an ADR is wise because it changes who speaks for the terminal.
3. **Graphics and rich output are lost at the node.** The node's grid (alacritty's) has no sixel or image protocol, no grapheme-cluster
   handling (combining marks are kept, but a joined emoji sequence takes the cells of its parts where a real terminal draws one), and
   it sizes wide characters by the Unicode tables of its crate; the window now matches it with the Unicode 11 table, but the two
   are only guaranteed to agree because both are old. Proposed fix: choose one width table in `agora-vt` and tell windows which (a
   capability, ADR) so the window loads the same; image protocols would need a new frame kind and are not proposed.
4. **The terminal client (TUI) redraws cells.** Mouse, wheel, focus, paste, links and the title are done (next section). What is
   still missing, because `crates/agora-tui/src/pane.rs:1-6` ("the agent's own escape sequences never reach the user's terminal") draws
   `agora-vt`'s cells into ratatui and `agora-vt` interprets and drops what it does not model:
   - **cursor shape** (DECSCUSR) and blink are ignored (the client draws its own cursor, `pane.rs`); **hyperlinks** (OSC 8) and
     **clipboard** (OSC 52 from a program) are interpreted and dropped by the emulator, so only plain addresses are links and a program
     cannot write the clipboard (which is also the safe policy, see the threat model, T46);
   - **graphics** (sixel, image protocols) cannot exist, and the keyboard protocols (Shift+Enter) are not passed on;
   - ~~**a program already running when the client starts watching** is not known to have the mouse (or focus reporting) on~~: done
     by item 1's first step. The repaint now sets or resets mouse tracking 1000/1002/1003, encodings 1005/1006, focus 1004 and the
     alternate screen, and the client's `termio::ModeScanner` and emulator read them from it like live output
     (`crates/agora-tui/tests/repaint_modes.rs`). Mouse 9 and the urxvt encoding 1015 are still learned only from live output;
   - **focus of the user's own terminal window** (not the pane) is not forwarded: the client does not switch on its own terminal's
     focus reports;
   - ~~**the history before the client started watching** is not there~~: done by item 1's step 3. A fresh pane asks for its 2000
     lines of scrollback and the wheel scrolls through them (`crates/agora-tui/tests/history_replay.rs`).
   Passthrough of the raw stream would fix all of it but gives up the cell model ADR-0063 chose; that is a decision for the maintainer,
   not an implementation detail.
5. **Hub resize policy** (`crates/agora-hub/src/router.rs:101`, `smallest`, ADR-0072). The window side is as the ADR says and looks clean, but
   one small viewer (a phone with its keyboard open) shrinks every other window's terminal for as long as it watches, and a window
   cannot say it is only looking. Proposed fix, as the ADR's open question says: let a window mark itself as "only looking" so its
   size does not count, and let a window whose tab is hidden stop counting; both are a field on the subscription, so an ADR.
6. **Scrollback is a node setting nobody can change.** The window's history is configurable now (1000 to 20 000 lines), and a fresh
   window now gets the node's history replayed (item 1, ADR-0100 step 3), but what it can see is bounded by what the node keeps (2000,
   `alacritty.rs:20`) and by the replay's 512 KiB cap: a window set to more than 2000 lines gets 2000 after a reload, and keeps more
   only from what it watches live. Proposed fix: a node setting for the cap (the performance budget in `docs/design/performance.md`
   says "default scrollback" for the 10 MB per terminal).

## Left for later (web client)

- The UTF-8 (1005) mouse encoding is not implemented by xterm.js (it writes X10, the same below column 95), and `mouse.ts` writes it
  the same way; urxvt (1015) is done (above).
- A trackpad over the main screen is xterm.js's own scroll bar, as before; how `wheel.ts` tells it from a wheel (`NotchDetector`) was
  not driven by a real trackpad or wheel (the browsers' synthetic wheel is always a notch); check it on real devices, the macOS app
  (WKWebView) first.
- Momentum after a touch drag stops at once (a real fling keeps going).
- Right click does not paste (the browser's menu opens); a "paste on right click" setting would match the terminals that do it.
- Find does not search a program's alternate screen differently from the main one, and has no case or regex switches.
- The graphics-card renderer on a scaled screen (see Rendering) needs a person to look at it on real hardware; if it misbehaves
  there, flip the default.
- Ligatures in the graphics-card renderer need another addon; not added.

# 0100. The terminal repaint carries the program's modes and its history
Status: Accepted
Date: 2026-11-22
Accepted: 2026-11-22 (the maintainer, in the merger chat: "Kabul, şema dahil" to the question that described decision 1, the modes in the
repaint with no schema change, and decision 2, the history before the repaint with the new optional `TerminalSubscribe.history_lines`
field). The schema step of decision 2 is approved with it. The open questions' proposed answers stand.

Touches ADR-0012 (VT backend: the `Terminal` trait's `modes`), ADR-0029 (terminal relay: a new terminal frame kind, relayed
unchanged), ADR-0072 (the size frame after a repaint), ADR-0022 and `docs/design/performance.md` (bytes per attach, input outranks
output, terminal bytes never go through the event store), ADR-0006 (append-only schema: one new optional field), ADR-0043 and
ADR-0063 (the terminal client's cell model), `docs/specs/protocol.md` ("Node-local control socket", "Terminal relay"),
`docs/ux/terminal-fidelity.md` (Phase 3 items 1, 4 and 6) and the threat model (terminal output as an asset, T46).

## Context

A window that starts watching a terminal (opening it, a reload, going Home and back, a remount, a reconnect, falling behind) gets one
repaint from the node: `Repaint::into_snapshot` in `crates/agora-node/src/terminal.rs` (called from `subscribe` and `resync`), sent
as the kind 1 frame and followed by the kind 2 size frame (`control.rs`, `pump`). Today it writes `ESC [0m ESC [2J`, every screen row
at an absolute position with its colours, the cursor position and visibility, bracketed paste (2004), application cursor keys
(DECCKM) and the pen. That is all `agora_vt::Modes` holds (`crates/agora-vt/src/lib.rs`, `Modes`: `alt_screen`, `bracketed_paste`,
`application_cursor`), and the repaint does not even send `alt_screen`. Everything else the program switched on before the window
attached is lost for that window, and so is everything that scrolled off the screen, although the node keeps 2000 lines of it
(`TerminalOptions::scrollback`, `crates/agora-vt/src/alacritty.rs` `SCROLLBACK`).

The maintainer reports that terminals feel broken (mouse, scroll). The investigation of 2026-11-22 (main at `3d0b2b2e`; Chromium,
Firefox and WebKit through the real window, and `agora tui`; a probe program on a real PTY that switches modes on and prints the bytes
it receives) found this repaint to be the root cause of five defects, in every engine and in the terminal client:

| | What happens | Why |
| --- | --- | --- |
| D1 | Mouse and wheel stop reaching the program when the window attaches after the program switched mouse modes on (reload, Home and back, remount): a click sends nothing, the wheel sends nothing. Web and TUI | the repaint carries no mouse tracking (1000/1002/1003) or encoding (1006) |
| D2 | Late attach to a full-screen program: the wheel sends no arrow keys | the repaint draws on the normal screen, so the window does not know the program is on the alternate screen |
| D3 | After a reconnect the window keeps stale modes: the program left its full-screen view and switched the mouse off while the window was away, and clicks and wheel reports land in the shell as garbage, the wheel becomes history recall | the repaint never switches a mode off; the web window keeps what it learned ("the modes stay", `apps/web/src/terminal/renderer.ts`, `reset`) |
| D4 | After a reload only the visible screen is in the window's history: 37 of a 300-line output were reachable, the node keeps 2000 | no history is sent |
| D5 | Focus reports (1004) are lost on late attach | the repaint carries no 1004 |

The terminal client works around part of this by reading mode sequences from the bytes it receives (`ModeScanner`,
`crates/agora-tui/src/termio.rs`), which cannot recover a mode set before it started watching (`terminal-fidelity.md`, item 4).

What the node's emulator can tell us today (`alacritty_terminal` 0.26, `Term::mode()`, `Term::cursor_style()`): cursor keys (DECCKM),
keypad (DECKPAM/DECKPNM), cursor visibility (DECTCEM), mouse tracking 1000, 1002 and 1003 (one at a time), encodings 1005 and 1006,
focus reporting 1004, alternate scroll 1007, the alternate screen (1049 only: 47 and 1047 are ignored by the backend), bracketed
paste 2004, autowrap (DECAWM), origin (DECOM), insert (IRM), newline (LNM), the cursor's shape and blink (DECSCUSR, mode 12), and the
kitty keyboard flags when its `kitty_keyboard` setting is on (it is off in `AlacrittyTerminal::with_scrollback`). It does **not**
track mouse mode 9 (X10), the urxvt encoding 1015, xterm's modifyOtherKeys, the scroll region (DECSTBM), the saved cursor or the
character sets, and while the alternate screen is active it gives no access to the main screen or its history (`inactive_grid` is
private; `swap_alt` back into the alternate screen clears it, so it cannot be used to peek).

## Decision

### 1. The repaint carries every mode the node's emulator tracks, as an explicit set or reset, in a fixed order

`agora_vt::Modes` (plain data, still `Copy`) grows the modes listed above as tracked, and the cursor grows its shape:

- mouse tracking: none, 1000, 1002 or 1003; mouse encoding: default, 1005 or 1006;
- focus reporting (1004), alternate scroll (1007), bracketed paste (2004, kept), cursor keys (DECCKM, kept), keypad (DECKPAM);
- the alternate screen (`alt_screen`, kept, now sent);
- autowrap (DECAWM), origin (DECOM), insert (IRM), newline (LNM);
- the kitty keyboard flags (current flags only; the backend does not expose the stack's depth). The node turns the backend's
  `kitty_keyboard` tracking on. This only makes the emulator follow the program's `CSI > / < / = u`; the reply to a `CSI ? u`
  query is still dropped like every other `PtyWrite` (who answers queries is Phase 3 item 2, not this ADR);
- `Cursor` gets the DECSCUSR shape and blink, or "default" when it is the backend's default (the program never set one, or set
  the default), so the window keeps the user's own cursor preference in that case.

The repaint writes **every** carried mode, set or reset, never only the ones that are on, so a window that kept stale modes is put
right (D3). A mode the node does not track (9, 1015, modifyOtherKeys, 47/1047, the scroll region until M7.10) is **left alone**: resetting what
the node cannot see would switch off something the program did switch on while the window watched. The order:

1. **Enter the program's screen**: `CSI ? 1049 h` when the program is on the alternate screen, `CSI ? 1049 l` when not (D2, D3).
2. **Reset what the drawing depends on**: origin off (`CSI ? 6 l`, so rows land at absolute positions), insert off (`CSI 4 l`),
   `CSI 0 m`, and ASCII in G0 shifted in (`ESC ( B`, SI; added in step 1 after the review of #953: a window left in the line-drawing
   set by an earlier program drew the whole repaint in it; the character sets stay untracked, so this is a protective reset, not the
   program's state). The scroll region is not touched (untracked, see above): stale margins in a returning window remain a gap
   (until M7.10: see "Built" below).
3. **Draw** the screen as today (`CSI 2 J`, every row at an absolute position with its styles).
4. **Modes**: mouse tracking (reset 1000, 1002, 1003, then set the program's one), mouse encoding (reset 1005, 1006, then set),
   1004, 1007, 2004, DECCKM, DECKPAM or DECKPNM, DECAWM, IRM, LNM; **not DECOM** (see below; restored since M7.10, with the scroll region); the keyboard flags (`CSI < 32 u` pops whatever stack the
   window holds, then `CSI = flags ; 1 u` sets the program's). Modes come after drawing because autowrap changes how the drawing lands.
5. **Cursor**: position, visibility (DECTCEM), shape (DECSCUSR, `CSI 0 SP q` for the default).

**DECOM is not restored** (Codex's review of this ADR, 2026-11-22, reproduced with the `agora-vt` backend): under origin mode a cursor
position (CUP) is relative to the scroll margins, and the margins (DECSTBM) are not tracked, so restoring DECOM and then sending the
node's absolute cursor row puts a reconnecting window with stale margins on the wrong row, and a fresh window drifts on the program's
next origin-relative move. The repaint therefore leaves origin mode off after step 2 (absolute positions are right), and DECOM joins the
untracked modes until the scroll region is tracked and restored too, with the cursor encoded relative to it (a later step, own test).
Step 1's tests assert that the cursor of a fresh and of a reconnecting emulator equals the node's right after the repaint, and pin the
known divergence on the program's next origin-relative move while origin mode and the margins are untracked (a test that documents
it); equality through that live move belongs to the follow-up that restores the margins.
**Built (M7.10, the follow-up):** the scroll region is read from the backend without a scanner on the feed path: `Terminal::margins`
(`&mut`, `None` for a backend that cannot say) finds it in `alacritty_terminal`, which keeps it private, by moving the cursor under
origin mode (a position there is clamped to the region) and putting the cursor and the mode back exactly, once per repaint. Between
steps 4 and 5 the repaint writes `CSI top ; bottom r` (always: the whole screen as `CSI r`, so a returning window's stale margins go), then
`CSI ? 6 h` when the program has origin mode on, and the cursor's row relative to the top margin. A cursor outside the region under
origin mode (DECRC can leave it there) cannot be addressed under DECOM, so origin mode then stays off at the absolute row (a pinned
gap). The two step-1 tests became equality, and a golden test checks that a fresh and a returning window put the program's next output
(a line feed at the bottom margin, a reverse index at the top, origin-relative moves) where the node does. `agora terminal attach`
gives the whole screen back on detach.
6. **Pen** (the program's current SGR), as today, last.

Every byte of this is generated by the node from booleans and numbers in `Modes` and `Cursor`; nothing is copied from the program's
output. The repaint grows by about 200 bytes. No schema change: kind 1 is "ANSI that repaints the screen", and this is still that.

Alternatives:
- **Send only the modes that are on** (smaller): leaves D3 as it is.
- **A structured modes frame** (a new message the client applies itself): every client would need code before any of them is fixed,
  while explicit sequences fix every existing web window and terminal client the day the node is updated (see 3).
- **Track the untracked modes too** (9, 1015, modifyOtherKeys, 47/1047, the scroll region) with a scanner in `agora-vt` on the node's
  feed path (`ModeScanner` moved there). That is the PTY read hot path, which scales with panes (ADR-0022), so it ships only as its
  own step with a `vt-flood` benchmark under the 10% rule, or by a patch upstream. Not proposed for the first step: the programs that
  matter use 1049, 1000/1002/1003 and 1006, which are covered.

### 2. A history replay before the repaint, on the terminal stream, only when the window asks for it

**The count is the window's**, through one new optional field, appended (ADR-0006):

```proto
message TerminalSubscribe {
  string terminal_id = 1;
  string format = 2;
  // ADR-0100: how many lines of history the window wants before the repaint; 0 or absent: none (as before).
  uint32 history_lines = 3;
}
```

The node sends the newest `min(history_lines, lines it keeps)` lines, and at most **512 KiB** of history bytes (the
oldest lines are dropped past the cap). It is sent as a new terminal frame **kind 4 (history)**: ANSI that puts those lines into the
window's scrollback at the size of the snapshot that follows, in chunks of at most 16 KiB (the live chunk size; at most 32 frames,
far below the hub's 512-frame lag limit and the 1 MiB payload limit), split only between lines or cells, never inside a sequence or
a character. A subscription with history is then: kind 4 frames, kind 1 repaint, kind 2 size, live output. A resync after falling
behind (`TerminalHandle::resync`, node-local) never carries history; a window that resubscribes decides again.

- **Shape.** The history leaves the alternate screen first (`CSI ? 1049 l`), clears the screen and then the scrollback (`CSI 2 J`,
  `CSI 3 J`, in that order: some emulators push a cleared screen into history), writes each line with its styles from the
  scrollback's cells (`Terminal::history_cell`, which exists already for the terminal client's wheel) and scrolls every replayed line
  off the screen, so that, fed to an empty emulator, the history holds exactly the replayed lines in order (no blank lines, no screen
  row twice) and the repaint then draws over a clear screen. A line the node holds as soft-wrapped is written without a line break, so
  the window can rewrap it on a resize (needs a small `history_wrapped(back)` on the trait; without it every line ends in a break).
- **On the alternate screen** history is of the main screen only (the alternate screen has none, as in every terminal). The backend
  cannot read the main screen while the alternate one is active (see Context), so in the first build **a program on the alternate
  screen gets no history**; when it leaves, the window's main screen shows what the window had. The trait method that would read the
  inactive screen is named and returns nothing for this backend until `alacritty_terminal` exposes it (an upstream accessor) or the
  in-house backend ADR-0012 allows replaces it.
- **The window holds kind 4 with the snapshot** (the web window's `snapshotAfterSize` already holds the snapshot for its size frame) and
  writes history and repaint together in one synchronized write (mode 2026), so it is one paint, at the right width.
- **What windows ask.** The web window asks for its own history setting (1000 to 20 000 lines; the node caps it at what it keeps) when
  its terminal has no history of its own (first open, reload, remount), and 0 on a reconnect or after falling behind, where it keeps
  its own longer history (the gap of dropped output is as today). The terminal client asks for its scrollback on a fresh pane and 0
  on a resubscribe. A phone may ask for less.
- **Cost against ADR-0022.** Per attach: one capture under the terminal's lock, newest line first and stopping at the line count or the
  byte cap, so the lock is held for at most the cap's worth of cells, then formatted after the lock is released (as the screen repaint
  is, `performance.md` "Node and hub"); at most 512 KiB on the wire per attaching window (2000 lines of 100 characters are about
  200 KB); no new memory on the node beyond that buffer while it is sent. Input is not behind it: keystrokes go to the PTY on the
  caller's thread (`PtyControl`) and the hub forwards input ahead of output on the node connection; the history travels the other way,
  like output. Ten windows reloading at once cost at most 5 MiB once. The PTY read, VT parse and frame relay paths are unchanged. The
  step that builds it reports, in `agora-bench`, the lock hold time for a full 2000-line, 200-column history, the bytes per attach, and
  "terminal snapshot after switching ≤ 150 ms" with a full history on the same-machine run; if the budget is missed, the cap goes
  down, not the budget.
- **Terminal bytes still never go through the event store**: the history comes from the node's in-memory emulator, goes only on the
  terminal's dynamic stream, and the hub relays kind 4 like every other terminal frame (`router::relay_frame` copies any kind), never
  decoding or storing it.

Alternatives:
- **A node default for every subscriber** (all kept lines up to the cap; no schema change). Rejected: the node cannot know what the
  window can do (capabilities are per connection, and the hub sits between them), and existing clients mishandle it: the web window
  keeps its own history across a reconnect, so it would be duplicated; the terminal client feeds its last snapshot again on a resize
  (`preview.rs`), which would duplicate history in the snapshot; and every resync after falling behind would resend it.
- **History inside the kind 1 payload** (no new frame kind): one frame up to the 1 MiB payload limit, sent whole ahead of everything
  else on the node's uplink ("output is chunked small", `performance.md`), and the terminal client's re-feed problem above.
- **History as kind 0 output frames**: changes what kind 0 means (bytes the process wrote, ADR-0006 forbids changing a published
  meaning), and the window would write it before it has the node's size (the size frame follows the snapshot).
- **A handshake capability as the opt-in**: the node sees the hub's capabilities, not the window's; the hub would have to rewrite the
  subscription, which ADR-0029 forbids.
- **A separate paged `terminal.history` command** (fetch as the user scrolls up, like ADR-0089's chat pages): cheaper per attach, but an
  emulator cannot insert lines above its existing scrollback, so the window would have to rebuild its terminal on every page. Worth it
  only if the node keeps far more than 2000 lines (Phase 3 item 6).

### 3. Compatibility: no capability flag

- **New node, old clients.** The modes ride the same kind 1 frame and every existing client applies them: xterm.js obeys the
  sequences, and the terminal client's emulator enters the alternate screen while its `ModeScanner` reads the mouse and focus modes
  from the repaint bytes it already scans. D1, D2, D3 and D5 are fixed for them with no client change. They never ask for history, so
  they never get kind 4. The terminal client's re-feed of the repaint on a resize stays harmless: `CSI ? 1049 h` on the alternate
  screen changes nothing, and the rest repaints the same screen (a test says so).
- **Old node, new clients.** The repaint carries no modes, so the window keeps what it learned (today's "modes stay"), and the history
  field is ignored (the node's decoder drops an unknown field), so no kind 4 arrives. A window never waits for history: kind 4, when
  sent, always comes before the kind 1 it belongs to.
- **Old hub.** It decodes and re-encodes the subscription and drops the unknown field: no history, everything else as before.
- **No capability flag** (`terminal.repaint_modes` was considered). Nothing in a client needs to branch: against a new node the
  repaint's explicit resets are authoritative by themselves, against an old one keeping the learned modes is still the best guess. A
  window does not see node capabilities today (`NodeSnapshot` has no list), so a flag would itself need a schema field. The web
  window's "modes stay" comment becomes "a current node's repaint sets every mode it tracks; an older node's sets none, and then the
  modes learned from the stream stay"; no code reset is added.

### 4. The terminal client

The same repaint and history feed the client's own `agora-vt` emulator (its alternate screen, its scrollback for the wheel) and its
`ModeScanner` (mouse tracking and encoding, focus), which the client then applies to the user's terminal as it does for live output:
it asks its own terminal for motion (1003) only while the program asked, sends focus reports and mouse reports in the program's
encoding, and turns the wheel into arrow keys on the alternate screen. `ModeScanner` stays (it covers 9 and 1015, which the node does
not track, and live changes); a test checks that, fed a repaint, it agrees with the node's `Modes` for every carried mode. The client
asks for history on a fresh pane, keeps only the kind 1 bytes as the "last screen" it may re-feed, and never re-feeds kind 4. Cursor
shape, hyperlinks and keyboard protocols in the client stay as `terminal-fidelity.md` item 4 describes (its cell model, ADR-0063).

### 5. Tests: the investigation's proofs become regression tests

- **Node unit tests** (`agora-node`, `agora-vt`): for each carried mode on and off, the repaint's bytes contain the explicit set or
  reset in the decided order; a repaint fed to a fresh `AlacrittyTerminal` reproduces the source's `Modes`, `Cursor` and screen; a
  window emulator primed with every stale mode on ends with the source's modes (D3); a history replay fed to an empty emulator gives
  exactly the kept lines, capped by lines and by bytes, newest kept; no history on the alternate screen or without the field; chunks
  at most 16 KiB and never split inside a sequence or a character.
- **Protocol**: a new golden fixture for `TerminalSubscribe` with `history_lines`; no existing fixture edited.
- **Browser e2e** (the investigation's `proof-mouse`, `proof-wheel` and the reconnect case, against a real node and a probe program on
  a real PTY): late attach after reload and after Home and back (click, drag, wheel, focus reports arrive: D1, D5); late attach to an
  alternate-screen program (wheel sends arrows: D2); reconnect after the program left full screen and switched the mouse off (no
  report reaches the shell: D3); after a reload the 300-line output's first line is reachable in the window's history (D4). Chromium in
  CI; Firefox and WebKit where the harness has them.
- **Terminal client** (`proof-tui` as a test on a pty): attach after the program switched the mouse on; a click reaches it, the wheel
  on its alternate screen sends arrows, and the scrollback holds the replayed history.
- Each test is seen failing on today's code first.

### 6. Build order, each its own PR

1. **Modes in the repaint** (`agora-vt`, `agora-node`; no schema): `Modes` and `Cursor` grow, the backend maps them and tracks the
   kitty flags, the repaint writes them in the decided order, `protocol.md` kind 1 description updated, node unit tests, the TUI
   agreement test and the browser late-attach and reconnect tests. Fixes D1, D2, D3, D5 for every client.
2. **History replay** (schema step: `TerminalSubscribe.history_lines` with its fixture; `agora-protocol`, `agora-vt`, `agora-node`):
   kind 4 frames, caps, the bench numbers named in decision 2, `protocol.md`, node tests.
3. **Clients** (`apps/web`, `agora-tui`): ask for history as decision 2 says, hold kind 4 with the snapshot and paint once, the "modes
   stay" comment, the e2e history test (D4) and the TUI history test; `terminal-fidelity.md` items 1, 4 and 6 updated.

Optional later: a scanner for the untracked modes under a benchmark (decision 1, alternatives), and the main screen under the
alternate screen once the backend can read it (decision 2).

### 7. Threat model: no new trust

The repaint stays node-generated from untrusted program output, as it is today. Mode bytes are produced by the node from its own state,
never copied from the program, so a program can only make the repaint say what it could already make a live window do by printing
the same switches. History lines are re-encoded from the emulator's cells (characters and SGR), so the escape sequences, OSC strings
(titles, OSC 52 clipboard, OSC 8 links) and control bytes the program printed are not replayed at all: what a window gets is narrower
than the live stream it would have seen. Both are bounded (lines, 512 KiB, 16 KiB chunks). The one real change is exposure in time:
a window that subscribes later now sees up to 2000 lines of earlier output (which may contain secrets, the threat model's "terminal
output" asset) that before it could only have seen by watching live. It is the same access (`TerminalSubscribe` scope, the same
people who may watch the terminal live), the hub still never stores it, and the threat model's terminal-output line says so when
step 2 is built.

## Consequences

- A window that opens, reloads or reconnects gets the program's mouse, wheel, focus, screen and cursor as a window that watched from the
  start, and (after step 3) its history; the five defects close in that order.
- `agora_vt::Modes` and `Cursor` become a little larger; they are plain data and nothing on the hot path copies them per byte.
- One appended schema field and one new terminal frame kind; nothing published changes meaning.
- Known gaps, each named above: modes the backend does not track (9, 1015, modifyOtherKeys, 47/1047, the saved
  cursor, character sets), the kitty keyboard stack's depth, history and the main screen while a program is on the alternate screen,
  and combining marks in history (a `Cell` cannot carry them, as in today's screen repaint). Pinned in step 1's tests or noted there
  (review of #953): only the current keyboard flags are carried, so a program that pushed twice and pops after a late attach is
  back on its first flags while the window has none; xterm.js switches mouse mode 9 off on any 1000/1002/1003 reset, so in the web
  window (not the terminal client) the repaint ends a 9 the window had; mode 12 (blink without a chosen shape) is not written,
  because xterm.js applies it to the window's own cursor-blink setting; and `agora terminal attach`, which writes the repaint to the
  user's own terminal, leaves out its keyboard pop, pushes an entry of its own on attach, and on detach pops it with the entries
  the program left pushed on the main screen and resets the modes.
- The node's scrollback stays fixed at 2000 lines (item 6 is separate).

## Open questions

1. Should read-only viewers get history? Proposed: yes, the same as the live stream they may already watch.
2. The main screen under the alternate screen: accept the gap, or ask `alacritty_terminal` upstream for a read accessor (or carry a
   small patch, which is a dependency decision)? Proposed: accept now, ask upstream.
3. Should a window send its focus state once after a repaint that switches 1004 on? Proposed: no; real terminals report only changes.

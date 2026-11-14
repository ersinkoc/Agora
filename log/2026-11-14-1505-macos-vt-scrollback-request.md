From: macos
To: linux
Kind: request

**agora-vt request (your lane), for TUI wheel scrollback: the last SHOULD item of the review.** Today the `Terminal` trait reads only the live screen. AlacrittyTerminal keeps scrollback (`SCROLLBACK`), but nothing can read it.

**Smallest change I'd need.** It is append-only, has default bodies so other impls are untouched, and leaves the live screen and detection exactly as they are:
```rust
/// Lines kept above the live screen (scrollback), 0 when none.
fn history_len(&self) -> usize { 0 }
/// The cell at `col` of the scrollback line `back` rows above the live screen's top (1 = just above); None out of range.
fn history_cell(&self, back: usize, col: u16) -> Option<Cell> { None }
```
- **AlacrittyTerminal:** `grid.history_size()`, and `grid[Line(-(back as i32))][Column(col)]`.
- **Conformance:** the live-screen tests stay as they are (`bottom_lines_never_include_scrollback`), plus one new test, "history_cell(1, ..) is the line that scrolled off".

**TUI side (mine, once this lands):**
- `App` keeps a scroll offset per tab: the wheel and Shift+PgUp/PgDn move it, a key sent to the agent resets it;
- `TerminalPane` draws history rows above the live rows;
- an "↑ N lines back" marker shows while scrolled.

If you'd rather I write the agora-vt part, say so and I'll send it as a PR for your review.

From: macos
To: linux
Cc: linux2
Kind: request

**agora-vt: 1-column panic/hang, stuck synchronized update, dropped combining marks (F701 High, F702, F703)** (evidence audit on origin/main 502b69bb; one message per crate.)

**F701 (input-handling, High).** `AlacrittyTerminal::with_scrollback`/`resize` clamp cols to `max(1)`. A 1-column grid cannot hold a double-width char: alacritty_terminal 0.26 writes the spacer at col 1 and panics (index out of bounds), and reflowing wide text to 1 column never finishes. The node accepts `cols = 1`, so `feed` kills the terminal actor thread and `resize` spins forever holding the shared mutex. Repro on main: `feed("日")` at 1x3 → panic; `feed("a✅")` at 0x3 → panic; `resize(10x3 -> 1x3)` with `日本語` → hang (5 s deadline); control at 2 columns OK. Fix: `MIN_COLS = 2`. The node may want to clamp the PTY to 2 columns too.

**F702 (bug, Medium).** After `CSI ? 2026 h`, vte 0.15 holds every later byte until ESU, 2 MiB, or the owner calls `stop_sync` once the 150 ms timeout passed; `AlacrittyTerminal::feed` never calls it, so an agent that dies mid-frame leaves detection and snapshots frozen. Fix: call `stop_sync` before `advance` when the deadline passed. Note: repro and test wait 300-400 ms past vte's own `Instant`-based timeout (no clock seam in vte), and the fix flushes only on the next `feed`; an idle program stays frozen until the node has a tick.

**F703 (bug, Low).** `line()`/`cell()` ignore `Cell::zerowidth()`, so NFD accents, VS16 and ZWJ vanish from detection text and snapshot lines (`cafe\u{301}.txt` → `cafe.txt`). Fix: override `line()` in `AlacrittyTerminal`; measured faster than the cell-based default (debug, 80x24, 3 runs). The node's repaint (`terminal.rs`, uses `cell()`) still drops them.

Repro: each diff carries a regression test (checked to fail without the fix hunk on an earlier main, 867740ab; on 502b69bb the standalone repro below is what was re-run). Checked on a clean origin/main worktree: `git apply <diff>`, a standalone repro binary (PROBLEM CONFIRMED before, PROBLEM NOT REPRODUCED after), a verifier with edge cases run 3 times (FIX VERIFIED 3/3), then `cargo test -p agora-vt -p agora-tui` (all passed), `cargo clippy -p agora-vt -p agora-tui --all-targets -- -D warnings`, `cargo fmt --check` and `git diff --check`: all clean. Nothing is edited from the macos lane; take, change or drop as you see fit, or send `Kind: assign`.

`fix_F701_single_column_wide_char.diff`:

```diff
--- a/crates/agora-vt/src/lib.rs
+++ b/crates/agora-vt/src/lib.rs
@@ -117,6 +117,7 @@
 pub trait Terminal: Send {
     /// Process bytes written by the program. Never panics on malformed input.
     fn feed(&mut self, bytes: &[u8]);
+    /// Change the screen size. A backend may keep a minimum size; [`size`](Terminal::size) reports the size it has.
     fn resize(&mut self, size: Size);
     fn size(&self) -> Size;
     /// The cell at `row`, `col` of the live screen (0-based, top-left origin); `None` if out of range.
--- a/crates/agora-vt/src/alacritty.rs
+++ b/crates/agora-vt/src/alacritty.rs
@@ -14,6 +14,10 @@
 /// Scrolled-back lines kept per terminal. Memory is bounded by this (design/resource-limits.md).
 const SCROLLBACK: usize = 2000;
 
+/// The fewest columns the screen has. A double-width character needs two cells: on a one-column grid `alacritty_terminal`
+/// indexes past the row when one is printed and never finishes reflowing one on resize (F701).
+const MIN_COLS: u16 = 2;
+
 #[derive(Clone, Default)]
 struct Listener {
     title: Arc<Mutex<Option<String>>>,
@@ -96,6 +100,7 @@
 }
 
 impl AlacrittyTerminal {
+    /// A terminal of `size`, at least 2 columns by 1 row ([`size`](Terminal::size) reports what it got).
     pub fn new(size: Size) -> Self {
         Self::with_scrollback(size, SCROLLBACK)
     }
@@ -104,7 +109,7 @@
     /// detection; the cap only bounds memory.
     pub fn with_scrollback(size: Size, scrollback: usize) -> Self {
         let size = Size {
-            cols: size.cols.max(1),
+            cols: size.cols.max(MIN_COLS),
             rows: size.rows.max(1),
         };
         let listener = Listener::default();
@@ -129,7 +134,7 @@
 
     fn resize(&mut self, size: Size) {
         let size = Size {
-            cols: size.cols.max(1),
+            cols: size.cols.max(MIN_COLS),
             rows: size.rows.max(1),
         };
         self.term.resize(Dims(size));
--- a/crates/agora-vt/src/conformance.rs
+++ b/crates/agora-vt/src/conformance.rs
@@ -22,6 +22,7 @@
     pen_is_the_style_of_the_next_character(&make);
     styling_sequences_print_nothing(&make);
     malformed_input_does_not_panic(&make);
+    narrow_terminals_take_wide_characters(&make);
 }
 
 fn text_and_newlines<T: Terminal>(make: &impl Fn(Size) -> T) {
@@ -232,6 +233,25 @@
     t.feed(b"ok");
 }
 
+/// The narrowest terminal still prints and reflows double-width characters (F701).
+fn narrow_terminals_take_wide_characters<T: Terminal>(make: &impl Fn(Size) -> T) {
+    for cols in [0, 1, 2] {
+        let mut t = make(size(cols, 3));
+        t.feed("a日✅b".as_bytes());
+        let s = t.size();
+        let c = t.cursor();
+        assert!(
+            c.row < s.rows && c.col < s.cols,
+            "{cols} columns: {c:?} in {s:?}"
+        );
+    }
+    let mut t = make(size(10, 3));
+    t.feed("日本語".as_bytes());
+    t.resize(size(1, 3));
+    t.feed("日".as_bytes());
+    assert!(t.size().cols >= 1 && t.cursor().col < t.size().cols);
+}
+
 #[cfg(test)]
 mod tests {
     use super::*;
```

`fix_F702_sync_update_never_expires.diff`:

```diff
--- a/crates/agora-vt/src/alacritty.rs
+++ b/crates/agora-vt/src/alacritty.rs
@@ -1,6 +1,7 @@
 //! Backend on `alacritty_terminal` (ADR-0012).
 
 use std::sync::{Arc, Mutex};
+use std::time::Instant;
 
 use alacritty_terminal::event::{Event, EventListener};
 use alacritty_terminal::grid::Dimensions;
@@ -124,6 +125,13 @@
 
 impl Terminal for AlacrittyTerminal {
     fn feed(&mut self, bytes: &[u8]) {
+        // A synchronized update (`CSI ? 2026 h`) the program never ended is applied once its timeout has passed, as a
+        // terminal window does; the parser only buffers it and leaves the expiry to its owner (F702).
+        if let Some(deadline) = self.parser.sync_timeout().sync_timeout() {
+            if Instant::now() >= deadline {
+                self.parser.stop_sync(&mut self.term);
+            }
+        }
         self.parser.advance(&mut self.term, bytes);
     }
 
@@ -181,3 +189,19 @@
         self.listener.title.lock().ok().and_then(|t| t.clone())
     }
 }
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+
+    /// A synchronized update the program never ends is applied once its timeout has passed (F702).
+    #[test]
+    fn unfinished_synchronized_update_expires() {
+        let mut t = AlacrittyTerminal::new(Size { cols: 20, rows: 3 });
+        t.feed(b"\x1b[?2026hframe");
+        assert_eq!(t.line(0), "", "held while the update may still end");
+        std::thread::sleep(std::time::Duration::from_millis(300));
+        t.feed(b"\r\n$ ");
+        assert_eq!(t.bottom_non_empty_lines(2), vec!["frame", "$"]);
+    }
+}
```

`fix_F703_line_drops_combining_marks.diff`:

```diff
--- a/crates/agora-vt/src/alacritty.rs
+++ b/crates/agora-vt/src/alacritty.rs
@@ -154,6 +154,28 @@
         })
     }
 
+    /// Like the default, but keeps the zero-width characters (combining accents, variation selectors, joiners) the
+    /// backend stores on each cell, which [`Cell`] cannot carry (F703).
+    fn line(&self, row: u16) -> String {
+        let mut s = String::new();
+        if row >= self.size.rows {
+            return s;
+        }
+        let line = &self.term.grid()[Line(i32::from(row))];
+        for col in 0..usize::from(self.size.cols) {
+            let c = &line[Column(col)];
+            if c.flags.contains(Flags::WIDE_CHAR_SPACER) {
+                continue;
+            }
+            s.push(if c.c == '\0' { ' ' } else { c.c });
+            if let Some(zw) = c.zerowidth() {
+                s.extend(zw);
+            }
+        }
+        s.truncate(s.trim_end().len());
+        s
+    }
+
     fn pen(&self) -> Style {
         let t = &self.term.grid().cursor.template;
         style(t.fg, t.bg, t.flags)
--- a/crates/agora-vt/src/conformance.rs
+++ b/crates/agora-vt/src/conformance.rs
@@ -17,6 +17,7 @@
     alt_screen_is_separate_and_restores(&make);
     title_from_osc(&make);
     wide_characters_use_two_cells(&make);
+    line_keeps_zero_width_characters(&make);
     modes_toggle(&make);
     styles_are_kept_per_cell(&make);
     pen_is_the_style_of_the_next_character(&make);
@@ -131,6 +132,14 @@
     assert_eq!(t.cursor().col, 5);
 }
 
+/// Combining accents and variation selectors stay with their character in the row's text (F703).
+fn line_keeps_zero_width_characters<T: Terminal>(make: &impl Fn(Size) -> T) {
+    let mut t = make(size(20, 2));
+    t.feed("cafe\u{301} \u{26a0}\u{fe0f}!".as_bytes());
+    assert_eq!(t.line(0), "cafe\u{301} \u{26a0}\u{fe0f}!");
+    assert_eq!(t.bottom_non_empty_lines(1), vec![t.line(0)]);
+}
+
 fn modes_toggle<T: Terminal>(make: &impl Fn(Size) -> T) {
     let mut t = make(size(10, 2));
     let m = t.modes();
```

//! The history replay a window may ask for when it subscribes to a terminal (ADR-0100 decision 2): the newest scrollback lines as
//! ANSI that puts them into the window's own scrollback, sent as kind 4 frames before the kind 1 repaint.
//!
//! Two steps, so the terminal's lock is held only for the first: [`HistoryCapture::capture`] copies the cells of the newest lines
//! under the lock (newest first, stopping at the line count or at the byte cap's worth of cells), and
//! [`HistoryCapture::into_frames`] formats them after the lock is released. Every byte is made here from the captured cells
//! (characters and SGR): nothing the program printed (escape sequences, OSC strings, control bytes) is replayed.
//!
//! The shape: leave the alternate screen (`CSI ? 1049 l`), reset what the writing depends on, clear the screen and then the
//! scrollback (`CSI 2 J`, `CSI 3 J`, in that order: some emulators push a cleared screen into history), write each line with its
//! styles from the top row (a line the node holds as wrapped without a line break, so the window can rewrap it), and finally scroll
//! every replayed line still on the screen off it from the bottom row. Fed to an empty emulator of the snapshot's size, its
//! history then holds exactly the replayed lines in order, its screen is clear, and the repaint that follows draws over it.

use agora_vt::{Cell, Size, Style, Terminal};

/// The most history bytes one subscription gets (ADR-0100 decision 2).
pub const HISTORY_MAX_BYTES: usize = 512 * 1024;
/// The most bytes in one kind 4 frame: the live output chunk size.
pub const HISTORY_CHUNK: usize = 16 * 1024;
/// The most kind 4 frames one subscription gets.
pub const HISTORY_MAX_FRAMES: usize = 32;
/// The longest piece a frame is never split inside (an escape sequence; an SGR with every attribute and two 24-bit colours is 52
/// bytes). A frame is cut only between pieces, so it may end up to this much short of [`HISTORY_CHUNK`].
const MAX_PIECE: usize = 64;
/// The bytes a replay may take: [`HISTORY_MAX_BYTES`], less what cutting only between pieces may leave unused in each of the
/// [`HISTORY_MAX_FRAMES`] frames, so the replay always fits in that many frames.
const BUDGET: usize = {
    let fits = HISTORY_MAX_FRAMES * (HISTORY_CHUNK - MAX_PIECE);
    if fits < HISTORY_MAX_BYTES {
        fits
    } else {
        HISTORY_MAX_BYTES
    }
};
/// Before the lines: leave the alternate screen; full-screen margins, origin and insert off, autowrap on (a wrapped line wraps in
/// the window as it did on the node), a plain pen, ASCII in G0 shifted in; clear the screen, then the scrollback; start at the top.
/// The margins are reset although the node does not track them (ADR-0100 decision 1 leaves them alone in the repaint): only lines
/// that scroll the whole screen go into a window's history, and a window that asks for history has none of its own to keep.
const HEAD: &str = "\x1b[?1049l\x1b[r\x1b[?6l\x1b[4l\x1b[?7h\x1b[0m\x1b(B\x0f\x1b[2J\x1b[3J\x1b[H";

/// One scrollback line as captured: its text (spacer cells left out), the style changes at byte offsets in it, and how it ends.
struct Row {
    text: String,
    styles: Vec<(usize, Style)>,
    /// It continues on the next line (the terminal wrapped it).
    wrapped: bool,
    /// Its first character is double-width (a wrapped line before it may end in the blank the terminal left for it).
    starts_wide: bool,
}

/// The cells of the newest scrollback lines, taken under the terminal's lock; formatting them is [`HistoryCapture::into_frames`].
pub struct HistoryCapture {
    /// Oldest first.
    rows: Vec<Row>,
    size: Size,
}

impl HistoryCapture {
    /// The newest `min(lines, kept)` scrollback lines of `term`, newest first until the count or until the lines' smallest possible
    /// size passes the byte cap (so the work, and the time the caller holds the terminal's lock, is bounded by the cap). While the
    /// program is on the alternate screen the history is the main screen's ([`Terminal::inactive_history_len`]), which this backend
    /// cannot read, so there is none.
    pub fn capture(term: &dyn Terminal, lines: usize) -> Self {
        let size = term.size();
        let alt = term.modes().alt_screen;
        let kept = if alt {
            term.inactive_history_len()
        } else {
            term.history_len()
        };
        let cell = |back: usize, col: u16| -> Option<Cell> {
            if alt {
                term.inactive_history_cell(back, col)
            } else {
                term.history_cell(back, col)
            }
        };
        let wrapped = |back: usize| {
            if alt {
                term.inactive_history_wrapped(back)
            } else {
                term.history_wrapped(back)
            }
        };
        let mut rows = Vec::new();
        let mut least = HEAD.len();
        for back in 1..=lines.min(kept) {
            let row = capture_row(size.cols, back, &cell, wrapped(back));
            // At least the text, every style change's shortest SGR (`CSI 0 m`) and the line break: what the line costs formatted.
            least += row.text.len() + 4 * row.styles.len() + 2;
            if least > BUDGET {
                break;
            }
            rows.push(row);
        }
        rows.reverse();
        Self { rows, size }
    }

    /// No lines were captured: no kind 4 frame is sent.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The kind 4 frames, in order: each at most [`HISTORY_CHUNK`] bytes, cut only between pieces (never inside an escape sequence or
    /// a character), at most [`HISTORY_MAX_FRAMES`] of them and [`HISTORY_MAX_BYTES`] in all. Past the cap the oldest lines are left
    /// out. Empty when nothing was captured.
    pub fn into_frames(self) -> Vec<Vec<u8>> {
        if self.rows.is_empty() {
            return Vec::new();
        }
        let rows = usize::from(self.size.rows.max(1));
        // The tail, at its longest: the bottom row's position and one line feed per screen row but the last.
        let tail_max = format!("\x1b[{rows};1H").len() + rows - 1;
        // How many of the newest lines fit, measured exactly as they will be written. A line's cost depends on whether it is the oldest
        // written (an empty line after a wrapped one prints a space to reach its own row, the oldest does not), so adding line `i` as the
        // new oldest also re-measures line `i + 1`, which stops being the oldest.
        let mut scratch = String::new();
        let mut measure = |i: usize, oldest: usize| {
            let mut count = Count(0);
            write_row(&mut count, &self.rows, i, oldest, &mut scratch);
            count.0
        };
        let mut total = HEAD.len() + tail_max;
        let mut first = self.rows.len();
        let mut oldest_cost = 0;
        for i in (0..self.rows.len()).rev() {
            let own = measure(i, i);
            let next = if i + 1 < self.rows.len() {
                measure(i + 1, i) - oldest_cost
            } else {
                0
            };
            if total + own + next > BUDGET {
                break;
            }
            total += own + next;
            oldest_cost = own;
            first = i;
        }
        let kept = &self.rows[first..];
        if kept.is_empty() {
            return Vec::new();
        }
        let mut out = Chunks::default();
        out.piece(HEAD);
        for i in 0..kept.len() {
            write_row(&mut out, kept, i, 0, &mut scratch);
        }
        // Every replayed line still on the screen goes into the scrollback: from the bottom row, one line feed each. They are the top
        // rows, the rest of the screen is blank, so the screen ends clear and the scrollback holds exactly the replayed lines.
        let on_screen = kept.len().min(rows - 1);
        out.piece(&format!("\x1b[{rows};1H"));
        out.text(&"\n".repeat(on_screen));
        out.finish()
    }
}

/// One scrollback line's cells. A line that ends where it was written loses its trailing blanks (plain spaces); a wrapped line keeps
/// them, so it fills the row and wraps in the window where it wrapped on the node.
fn capture_row(
    cols: u16,
    back: usize,
    cell: &impl Fn(usize, u16) -> Option<Cell>,
    wrapped: bool,
) -> Row {
    let mut row = Row {
        text: String::with_capacity(usize::from(cols)),
        styles: Vec::new(),
        wrapped,
        starts_wide: false,
    };
    let mut current = Style::default();
    let mut visible = 0;
    for col in 0..cols {
        let Some(c) = cell(back, col) else {
            break;
        };
        if c.spacer {
            if col == 1 {
                row.starts_wide = true;
            }
            continue;
        }
        if c.style != current {
            row.styles.push((row.text.len(), c.style));
            current = c.style;
        }
        row.text.push(c.ch);
        if c.ch != ' ' || c.style != Style::default() {
            visible = row.text.len();
        }
    }
    if !wrapped {
        row.text.truncate(visible);
        row.styles.retain(|&(at, _)| at < visible);
    }
    row
}

/// Where the replay goes: counted (to fit the cap) or cut into frames.
trait Sink {
    /// An escape sequence, never cut.
    fn piece(&mut self, s: &str);
    /// Text, cut only between characters.
    fn text(&mut self, s: &str);
}

struct Count(usize);

impl Sink for Count {
    fn piece(&mut self, s: &str) {
        self.0 += s.len();
    }
    fn text(&mut self, s: &str) {
        self.0 += s.len();
    }
}

#[derive(Default)]
struct Chunks {
    done: Vec<Vec<u8>>,
    current: Vec<u8>,
}

impl Chunks {
    fn flush(&mut self) {
        if !self.current.is_empty() {
            self.done.push(std::mem::take(&mut self.current));
        }
    }

    fn finish(mut self) -> Vec<Vec<u8>> {
        self.flush();
        self.done
    }
}

impl Sink for Chunks {
    fn piece(&mut self, s: &str) {
        debug_assert!(s.len() <= MAX_PIECE, "{s:?}");
        if self.current.len() + s.len() > HISTORY_CHUNK {
            self.flush();
        }
        self.current.extend_from_slice(s.as_bytes());
    }

    fn text(&mut self, mut s: &str) {
        while self.current.len() + s.len() > HISTORY_CHUNK {
            let mut room = HISTORY_CHUNK - self.current.len();
            while !s.is_char_boundary(room) {
                room -= 1;
            }
            self.current.extend_from_slice(&s.as_bytes()[..room]);
            self.flush();
            s = &s[room..];
        }
        self.current.extend_from_slice(s.as_bytes());
    }
}

/// Writes line `i` of `rows` (oldest first). `oldest` is the first line written: it starts a row of its own whatever came before.
/// The newest line always ends in a line break: what it wrapped into is the live screen, which the repaint draws.
fn write_row(sink: &mut impl Sink, rows: &[Row], i: usize, oldest: usize, scratch: &mut String) {
    let row = &rows[i];
    let newest = i + 1 == rows.len();
    let after_wrap = i > oldest && rows[i - 1].wrapped;
    let wraps = row.wrapped && !newest;
    let mut text = row.text.as_str();
    // The terminal left the last column blank because the next line's first character is double-width and did not fit: leave it
    // out, so the window wraps that character the same way.
    if wraps && rows[i + 1].starts_wide && text.ends_with(' ') {
        text = &text[..text.len() - 1];
    }
    let mut start = 0;
    let mut styled = false;
    for &(at, style) in &row.styles {
        if at > text.len() {
            break;
        }
        sink.text(&text[start..at]);
        scratch.clear();
        style.append_sgr(scratch);
        sink.piece(scratch);
        styled = style != Style::default();
        start = at;
    }
    sink.text(&text[start..]);
    // A line that follows a wrapped one must print something to move onto its own row (the cursor waits at the right edge).
    if after_wrap && text.is_empty() {
        sink.text(" ");
    }
    // A plain pen before any line break, so a scroll never fills a row with this line's colours.
    if styled {
        sink.piece("\x1b[0m");
    }
    if !wraps {
        sink.text("\r\n");
    }
}

/// The kind 4 frames a window asking for `lines` lines of history gets from `term`: the same bytes
/// [`TerminalHandle::subscribe_with_history`](crate::terminal::TerminalHandle::subscribe_with_history) sends, for tests and tools that
/// hold an emulator rather than a running terminal.
pub fn history_of(term: &dyn Terminal, lines: usize) -> Vec<Vec<u8>> {
    HistoryCapture::capture(term, lines).into_frames()
}

#[cfg(test)]
mod tests {
    use super::*;
    use agora_vt::AlacrittyTerminal;

    fn size(cols: u16, rows: u16) -> Size {
        Size { cols, rows }
    }

    /// A terminal of `size` that printed `lines`, each ended with CR LF.
    fn printed(size: Size, lines: impl IntoIterator<Item = String>) -> AlacrittyTerminal {
        let mut t = AlacrittyTerminal::new(size);
        for l in lines {
            t.feed(l.as_bytes());
            t.feed(b"\r\n");
        }
        t
    }

    fn history(t: &dyn Terminal) -> Vec<String> {
        (1..=t.history_len())
            .rev()
            .map(|b| t.history_line(b))
            .collect()
    }

    /// An empty window of `size` fed the replay of `lines` lines of `src`, then (when `repaint`) the node's repaint.
    fn window(
        src: &mut dyn Terminal,
        lines: usize,
        repaint: bool,
    ) -> (AlacrittyTerminal, Vec<Vec<u8>>) {
        let frames = history_of(&*src, lines);
        let mut w = AlacrittyTerminal::new(src.size());
        for f in &frames {
            w.feed(f);
        }
        if repaint {
            w.feed(&crate::terminal::repaint_of(src));
        }
        (w, frames)
    }

    /// Decision 5: a replay fed to an empty emulator gives exactly the kept lines, in order, no blank line and no screen row twice; the
    /// repaint after it draws the same screen as the source's.
    #[test]
    fn a_replay_puts_exactly_the_kept_lines_into_an_empty_window() {
        for rows in [1u16, 3, 5, 24] {
            for printed_lines in [1usize, 2, 4, 7, 40] {
                let mut src = printed(
                    size(20, rows),
                    (1..=printed_lines).map(|i| format!("\x1b[3{}mline {i}\x1b[0m", i % 8)),
                );
                let kept = history(&src);
                let what = format!("{rows} rows, {printed_lines} lines");
                if kept.is_empty() {
                    // Nothing kept, nothing sent. (This backend's own `CSI 2 J` in the repaint then pushes one blank row into a window
                    // with no history at all, as before this replay existed; with a replay in front there is history and it pushes none.)
                    assert!(history_of(&src, 10_000).is_empty(), "{what}");
                    continue;
                }
                let (w, frames) = window(&mut src, 10_000, true);
                assert_eq!(history(&w), kept, "{what}");
                assert!(!frames.is_empty(), "{what}");
                for row in 0..rows {
                    assert_eq!(w.line(row), src.line(row), "{what}");
                }
                assert_eq!(w.cursor(), src.cursor(), "{what}");
                // The styles came along.
                for back in 1..=kept.len() {
                    assert_eq!(w.history_cell(back, 0), src.history_cell(back, 0), "{what}");
                }
            }
        }
    }

    /// Capped by the window's count: the newest lines.
    #[test]
    fn the_count_keeps_the_newest_lines() {
        let mut src = printed(size(20, 5), (1..=100).map(|i| format!("line {i}")));
        let all = history(&src);
        assert_eq!(all.len(), 96);
        for n in [1usize, 10, 95, 96, 500] {
            let (w, _) = window(&mut src, n, false);
            let want = &all[all.len() - n.min(all.len())..];
            assert_eq!(history(&w), want, "{n}");
        }
    }

    /// Capped by bytes: at most 512 KiB in at most 32 frames, the newest lines kept, the oldest dropped.
    #[test]
    fn the_byte_cap_keeps_the_newest_lines() {
        // 200 columns of a colour per character: about 2.4 KB a line formatted, so 2000 lines are far past the cap.
        let line = |i: usize| -> String {
            let mut s = format!("{i:>6}");
            for c in 0..194 {
                s.push_str(&format!(
                    "\x1b[38;2;{};{};{}m{}",
                    c,
                    i % 256,
                    255 - c,
                    (b'a' + (c % 26) as u8) as char
                ));
            }
            s
        };
        let mut src = printed(size(200, 10), (1..=2100).map(line));
        let all = history(&src);
        assert_eq!(all.len(), 2000);
        // The capture (under the terminal's lock) already stops near the cap rather than copying all 2000 lines.
        let captured = HistoryCapture::capture(&src, 2000).rows.len();
        assert!(captured < 2000, "{captured}");
        let (w, frames) = window(&mut src, 2000, false);
        let got = history(&w);
        let bytes: usize = frames.iter().map(Vec::len).sum();
        assert!(bytes <= HISTORY_MAX_BYTES, "{bytes}");
        assert!(frames.len() <= HISTORY_MAX_FRAMES, "{}", frames.len());
        assert!(frames.iter().all(|f| f.len() <= HISTORY_CHUNK));
        assert!(got.len() > 100 && got.len() < 2000, "{}", got.len());
        assert_eq!(got, all[all.len() - got.len()..], "the newest, in order");
        // Plain lines of the same width: all 2000 fit.
        let mut src = printed(size(200, 10), (1..=2100).map(|i| format!("{i:0>200}")));
        let (w, frames) = window(&mut src, 2000, false);
        assert_eq!(history(&w), history(&src));
        assert!(frames.len() <= HISTORY_MAX_FRAMES);
    }

    /// The budget counts exactly what is written: an empty line after a wrapped one costs the space that moves it onto its own row,
    /// and the oldest line written costs nothing of the kind (review of #959: alternating wrapped lines with the longest SGRs and
    /// blank continuations went past the budget when every line was measured as the oldest).
    #[test]
    fn the_budget_counts_blank_continuations_of_wrapped_lines() {
        let mut src = AlacrittyTerminal::new(size(10, 3));
        for i in 0..2100usize {
            // Ten cells, each in a style of its own with every attribute and two 24-bit colours (a 52-byte SGR), then a plain space
            // that wraps onto the next row and is all that row holds.
            let mut line = String::new();
            for c in 0..10usize {
                let v = 100 + (i + c) % 2 * 100;
                line.push_str(&format!(
                    "\x1b[1;2;3;4;7;8;9;38;2;{v};{v};{v};48;2;{v};{v};{v}m{}",
                    (b'a' + c as u8) as char
                ));
            }
            src.feed(line.as_bytes());
            src.feed(b"\x1b[0m \r\n");
        }
        assert_eq!(src.history_len(), 2000);
        let frames = history_of(&src, 2000);
        let total: usize = frames.iter().map(Vec::len).sum();
        assert!(total <= BUDGET, "{total} > {BUDGET}");
        assert!(frames.len() <= HISTORY_MAX_FRAMES, "{}", frames.len());
        let mut w = AlacrittyTerminal::new(src.size());
        for f in &frames {
            w.feed(f);
        }
        let (got, all) = (history(&w), history(&src));
        assert!(!got.is_empty() && got.len() < all.len(), "{}", got.len());
        assert_eq!(got, all[all.len() - got.len()..]);
    }

    /// A line the node holds as wrapped is written without a break: the window has the same rows, and rewraps them on a resize.
    #[test]
    fn wrapped_lines_rewrap_in_the_window() {
        let long: String = (0..35).map(|i| (b'a' + (i % 26) as u8) as char).collect();
        let mut src = printed(
            size(10, 3),
            [
                "first".to_string(),
                long.clone(),
                "after".into(),
                "x".into(),
                "y".into(),
                "z".into(),
            ],
        );
        let (mut w, _) = window(&mut src, 100, true);
        assert_eq!(history(&w), history(&src));
        for back in 1..=src.history_len() {
            assert_eq!(w.history_wrapped(back), src.history_wrapped(back), "{back}");
        }
        // Wider: the long line is one row again (it would be four rows had it been written with breaks).
        w.resize(size(40, 3));
        let mut wide = history(&w);
        wide.extend((0..3).map(|r| w.line(r)));
        let at = |l: &str| wide.iter().position(|x| x == l);
        assert!(
            at("first") < at(&long) && at(&long) < at("after"),
            "{wide:?}"
        );
        assert!(at(&long).is_some(), "{wide:?}");
    }

    /// A wrapped line whose last column the terminal left blank for a double-width character, a wrapped line followed by an empty
    /// continuation, and wide characters elsewhere: the window gets the same rows.
    #[test]
    fn wide_characters_and_blank_continuations_keep_the_rows() {
        let mut src = printed(
            size(10, 3),
            [
                "123456789界x".to_string(),
                format!("{}{}", "abcdefghij", " ".repeat(10)),
                "日本語テキスト".into(),
                "a".into(),
                "b".into(),
                "c".into(),
            ],
        );
        let (mut w, _) = window(&mut src, 100, true);
        assert_eq!(history(&w), history(&src));
        for back in 1..=src.history_len() {
            assert_eq!(w.history_wrapped(back), src.history_wrapped(back), "{back}");
            for col in 0..10 {
                assert_eq!(
                    w.history_cell(back, col).map(|c| c.ch),
                    src.history_cell(back, col).map(|c| c.ch),
                    "{back},{col}"
                );
            }
        }
        // Wider, both rewrap the same way: the blank the terminal left before 界 is not a character of the line.
        src.resize(size(20, 3));
        w.resize(size(20, 3));
        assert_eq!(history(&w), history(&src));
        assert!(
            history(&src).contains(&"123456789界x".to_string()),
            "{:?}",
            history(&src)
        );
    }

    /// No history while the program is on the alternate screen (this backend cannot read the main screen's), and none for a count of 0.
    #[test]
    fn no_history_on_the_alternate_screen_or_for_zero_lines() {
        let mut src = printed(size(20, 5), (1..=50).map(|i| format!("line {i}")));
        assert!(history_of(&src, 0).is_empty());
        assert!(!history_of(&src, 1).is_empty());
        src.feed(b"\x1b[?1049h\x1b[Hfull screen");
        assert!(history_of(&src, 2000).is_empty());
        let empty = AlacrittyTerminal::new(size(20, 5));
        assert!(
            history_of(&empty, 2000).is_empty(),
            "nothing kept, nothing sent"
        );
    }

    /// Frames are at most 16 KiB and are never cut inside an escape sequence or a UTF-8 character: each frame parses on its own.
    #[test]
    fn frames_are_cut_between_sequences_and_characters() {
        let line = |i: usize| -> String {
            let mut s = String::new();
            for c in 0..100 {
                s.push_str(&format!(
                    "\x1b[38;5;{}m{}",
                    (i + c) % 256,
                    ['é', '界', 'x', '🙂'][c % 4]
                ));
            }
            s
        };
        let mut src = printed(size(200, 10), (1..=600).map(line));
        let frames = history_of(&src, 2000);
        assert!(frames.len() > 2);
        for (n, f) in frames.iter().enumerate() {
            assert!(f.len() <= HISTORY_CHUNK, "frame {n}: {}", f.len());
            assert!(std::str::from_utf8(f).is_ok(), "frame {n} cuts a character");
            // The last escape in the frame is complete: a CSI has its final byte, `ESC ( B` its designator.
            let s = std::str::from_utf8(f).unwrap_or_default();
            if let Some(at) = s.rfind('\x1b') {
                let rest = &s.as_bytes()[at + 1..];
                let complete = match rest.first() {
                    Some(b'[') => rest[1..].iter().any(|b| (0x40..=0x7e).contains(b)),
                    Some(b'(') => rest.len() >= 2,
                    _ => false,
                };
                assert!(complete, "frame {n} ends inside {:?}", &s[at..]);
            }
        }
        // Past the byte cap: the newest lines, in order.
        let (w, _) = window(&mut src, 2000, false);
        let (got, all) = (history(&w), history(&src));
        assert!(!got.is_empty() && got.len() < all.len(), "{}", got.len());
        assert_eq!(got, all[all.len() - got.len()..]);
    }
}

//! A terminal pane drawn natively from `agora-vt` cells (ADR-0099 decision 2). Written for this spike from the `agora-vt` trait and
//! GPUI's public API; no Zed application code (its `terminal_view` is GPL-3.0) was read or copied.
//!
//! Behaviour follows `docs/ux/terminal-fidelity.md`: mouse modes 9/1000/1002/1003 and encodings default/1005/1006/1015 through
//! `agora_tui::termio` (the same pure code the TUI uses), Shift keeps the mouse local, the wheel scrolls history on the main screen,
//! becomes arrow keys on the alternate screen with no mouse mode and reports when the program tracks the mouse; selection and copy over
//! screen and history; IME preedit drawn at the cursor; input is sent before any pending output is drawn.

use std::cell::{Cell as StdCell, RefCell};
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use crate::g::*;
use crate::metrics::Metrics;
use crate::net::{Cmd, TermShared};
use agora_tui::termio::{self, Phase, Report, Track};
use agora_vt::{Cell, Color, Size, Terminal};
use tokio::sync::mpsc::UnboundedSender;

pub const FONT_SIZE: f32 = 13.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelMode {
    Char,
    Word,
    Line,
}

/// A selection in buffer lines: line 0 is the oldest history line, `history_len + row` a live row.
#[derive(Clone, Copy, Debug)]
struct Sel {
    anchor: (i64, u16),
    head: (i64, u16),
    mode: SelMode,
}

#[derive(Clone, Copy)]
pub struct CellMetrics {
    pub width: Pixels,
    pub height: Pixels,
}

pub struct TermPane {
    pub id: usize,
    pub shared: Arc<TermShared>,
    pub focus: FocusHandle,
    tx: UnboundedSender<Cmd>,
    metrics: Rc<RefCell<Metrics>>,
    /// Lines scrolled back into history (0: the live screen).
    scroll: usize,
    last_hist: usize,
    sel: Option<Sel>,
    selecting: bool,
    /// The button of a press the program got: its drags and release go to the program too.
    program_press: Option<u8>,
    last_motion_cell: Option<(u16, u16)>,
    preedit: Option<String>,
    wheel_accum: f32,
    cell: Rc<StdCell<Option<CellMetrics>>>,
    bounds: Rc<StdCell<Bounds<Pixels>>>,
    last_size: Rc<StdCell<(u16, u16)>>,
    font: Font,
}

/// The 16 classic colours (a dark theme close to the web client's), then the 6x6x6 cube and the grey ramp.
fn palette(i: u8) -> Hsla {
    const BASE: [u32; 16] = [
        0x1d1f21, 0xcc6666, 0xb5bd68, 0xf0c674, 0x81a2be, 0xb294bb, 0x8abeb7, 0xc5c8c6, 0x666666,
        0xd54e53, 0xb9ca4a, 0xe7c547, 0x7aa6da, 0xc397d8, 0x70c0b1, 0xeaeaea,
    ];
    if i < 16 {
        return rgb(BASE[usize::from(i)]).into();
    }
    if i < 232 {
        let n = i - 16;
        let c = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
        return rgb8(c(n / 36), c((n / 6) % 6), c(n % 6));
    }
    let v = 8 + (i - 232) * 10;
    rgb8(v, v, v)
}

fn rgb8(r: u8, g: u8, b: u8) -> Hsla {
    Rgba {
        r: f32::from(r) / 255.0,
        g: f32::from(g) / 255.0,
        b: f32::from(b) / 255.0,
        a: 1.0,
    }
    .into()
}

const DEFAULT_FG: u32 = 0xd4d4d4;
const DEFAULT_BG: u32 = 0x18181b;

fn colors(c: &Cell) -> (Hsla, Option<Hsla>) {
    let conv = |c: Color| match c {
        Color::Default => None,
        Color::Indexed(i) => Some(palette(i)),
        Color::Rgb(r, g, b) => Some(rgb8(r, g, b)),
    };
    let mut fg = conv(c.style.fg).unwrap_or_else(|| rgb(DEFAULT_FG).into());
    let mut bg = conv(c.style.bg);
    if c.style.inverse {
        let b = bg.unwrap_or_else(|| rgb(DEFAULT_BG).into());
        bg = Some(fg);
        fg = b;
    }
    if c.style.dim {
        fg.a *= 0.6;
    }
    if c.style.hidden {
        fg.a = 0.0;
    }
    (fg, bg)
}

/// What one frame of the pane needs, copied out under the emulator's lock so drawing never holds it.
struct Frame {
    cols: u16,
    rows: u16,
    cells: Vec<Cell>,
    cursor: Option<(u16, u16)>,
    generation: u64,
    got_screen: bool,
    first_after_send: Option<Instant>,
}

impl TermPane {
    pub fn new(
        id: usize,
        shared: Arc<TermShared>,
        tx: UnboundedSender<Cmd>,
        metrics: Rc<RefCell<Metrics>>,
        cx: &mut Context<Self>,
    ) -> Self {
        TermPane {
            id,
            shared,
            focus: cx.focus_handle(),
            tx,
            metrics,
            scroll: 0,
            last_hist: 0,
            sel: None,
            selecting: false,
            program_press: None,
            last_motion_cell: None,
            preedit: None,
            wheel_accum: 0.0,
            cell: Rc::new(StdCell::new(None)),
            bounds: Rc::new(StdCell::new(Bounds::default())),
            last_size: Rc::new(StdCell::new((0, 0))),
            font: font(crate::mono_family()),
        }
    }

    /// Sends bytes the person typed (or pasted) to the program.
    pub fn send(&mut self, bytes: Vec<u8>) {
        let generation = {
            let mut st = self.shared.lock();
            st.mark_sent();
            st.generation
        };
        {
            let mut m = self.metrics.borrow_mut();
            m.tap(self.id, &bytes);
            m.sent(self.id, generation, Instant::now());
        }
        let _ = self.tx.send(Cmd::Input {
            node: self.shared.node.clone(),
            terminal_id: self.shared.terminal_id.clone(),
            data: bytes,
        });
    }

    /// The window position of a cell's centre (self-test).
    pub fn cell_center(&self, col: u16, row: u16) -> Point<Pixels> {
        let b = self.bounds.get();
        let m = self.cell.get().unwrap_or(CellMetrics {
            width: px(8.0),
            height: px(16.0),
        });
        point(
            b.origin.x + m.width * (f32::from(col) + 0.5),
            b.origin.y + m.height * (f32::from(row) + 0.5),
        )
    }

    /// Lines scrolled back into history.
    pub fn scrolled(&self) -> usize {
        self.scroll
    }

    /// The text of a displayed row, history included (self-test).
    pub fn visible_line(&self, row: u16) -> String {
        let st = self.shared.lock();
        let scroll = self.scroll.min(st.term.history_len());
        if usize::from(row) < scroll {
            st.term.history_line(scroll - usize::from(row))
        } else {
            st.term.line(row - scroll as u16)
        }
    }

    fn modes(&self) -> (termio::ProgramModes, agora_vt::Modes, usize) {
        let st = self.shared.lock();
        (st.scanner.modes(), st.term.modes(), st.term.history_len())
    }

    fn cell_at(&self, p: Point<Pixels>) -> (u16, u16) {
        let b = self.bounds.get();
        let Some(m) = self.cell.get() else {
            return (0, 0);
        };
        let (cols, rows) = self.last_size.get();
        let x = ((p.x - b.origin.x) / m.width).floor().max(0.0) as u16;
        let y = ((p.y - b.origin.y) / m.height).floor().max(0.0) as u16;
        (x.min(cols.saturating_sub(1)), y.min(rows.saturating_sub(1)))
    }

    fn line_of_row(&self, row: u16, hist: usize) -> i64 {
        hist as i64 - self.scroll.min(hist) as i64 + i64::from(row)
    }

    fn mods(m: &Modifiers) -> u8 {
        let mut v = 0;
        if m.shift {
            v |= termio::SHIFT;
        }
        if m.alt {
            v |= termio::ALT;
        }
        if m.control {
            v |= termio::CTRL;
        }
        v
    }

    fn report(&mut self, phase: Phase, button: u8, mods: u8, col: u16, row: u16) -> bool {
        let (pm, _, _) = self.modes();
        let r = Report {
            phase,
            button,
            mods,
            col,
            row,
        };
        match termio::encode_mouse(&pm, &r) {
            Some(bytes) => {
                self.metrics.borrow_mut().tap(self.id, &bytes);
                let _ = self.tx.send(Cmd::Input {
                    node: self.shared.node.clone(),
                    terminal_id: self.shared.terminal_id.clone(),
                    data: bytes,
                });
                true
            }
            None => false,
        }
    }

    fn button_code(b: MouseButton) -> Option<u8> {
        match b {
            MouseButton::Left => Some(0),
            MouseButton::Middle => Some(1),
            MouseButton::Right => Some(2),
            MouseButton::Navigate(_) => None,
        }
    }

    pub fn mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let Some(button) = Self::button_code(ev.button) else {
            return;
        };
        let (col, row) = self.cell_at(ev.position);
        let (pm, _, hist) = self.modes();
        // Shift keeps the mouse local (selection); Ctrl+click is the link gesture and is never sent.
        if pm.track != Track::Off && !ev.modifiers.shift && !ev.modifiers.control {
            if self.report(Phase::Press, button, Self::mods(&ev.modifiers), col, row) {
                self.program_press = Some(button);
            }
            return;
        }
        if button == 0 {
            let line = self.line_of_row(row, hist);
            let mode = match ev.click_count {
                2 => SelMode::Word,
                n if n >= 3 => SelMode::Line,
                _ => SelMode::Char,
            };
            self.sel = Some(Sel {
                anchor: (line, col),
                head: (line, col),
                mode,
            });
            self.selecting = true;
            cx.notify();
        } else if button == 1 {
            // Middle click pastes the primary selection where the system has one (X11, Wayland).
            if let Some(text) = crate::os::read_primary(cx) {
                self.paste(&text);
            }
        }
    }

    pub fn mouse_move(
        &mut self,
        ev: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (col, row) = self.cell_at(ev.position);
        if let Some(button) = self.program_press {
            if self.last_motion_cell != Some((col, row))
                && !ev.modifiers.shift
                && !ev.modifiers.control
            {
                self.last_motion_cell = Some((col, row));
                self.report(Phase::Drag, button, Self::mods(&ev.modifiers), col, row);
            }
            return;
        }
        if self.selecting && ev.pressed_button == Some(MouseButton::Left) {
            let hist = self.modes().2;
            let line = self.line_of_row(row, hist);
            if let Some(s) = &mut self.sel {
                s.head = (line, col);
            }
            cx.notify();
            return;
        }
        if ev.pressed_button.is_none() && !ev.modifiers.shift {
            // Motion with no button only when the program asked for it (1003).
            let (pm, _, _) = self.modes();
            if pm.track == Track::Any && self.last_motion_cell != Some((col, row)) {
                self.last_motion_cell = Some((col, row));
                self.report(Phase::Move, 0, Self::mods(&ev.modifiers), col, row);
            }
        }
    }

    pub fn mouse_up(&mut self, ev: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let (col, row) = self.cell_at(ev.position);
        if let Some(button) = self.program_press.take() {
            // A press the program has never loses its release; Shift/Ctrl are cut off so the button does not stay down.
            let mods = Self::mods(&ev.modifiers) & !(termio::SHIFT | termio::CTRL);
            self.report(Phase::Release, button, mods, col, row);
            self.last_motion_cell = None;
            return;
        }
        if self.selecting {
            self.selecting = false;
            let no_clipboard = self.metrics.borrow().no_clipboard;
            if let Some(text) = self
                .selection_text()
                .filter(|t| !t.is_empty() && !no_clipboard)
            {
                crate::os::write_primary(cx, text);
            } else if matches!(self.sel, Some(s) if s.mode == SelMode::Char && s.anchor == s.head) {
                self.sel = None;
            }
            cx.notify();
        }
    }

    pub fn scroll_wheel(&mut self, ev: &ScrollEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let lh = self.cell.get().map_or(px(16.0), |m| m.height);
        let lines = match ev.delta {
            ScrollDelta::Lines(p) => p.y,
            ScrollDelta::Pixels(p) => p.y / lh,
        };
        self.metrics.borrow_mut().note_wheel(lines);
        self.wheel_accum += lines;
        let notches = self.wheel_accum.trunc() as i32;
        if notches == 0 {
            return;
        }
        self.wheel_accum -= notches as f32;
        let up = notches > 0;
        let (col, row) = self.cell_at(ev.position);
        let (pm, vm, hist) = self.modes();
        for _ in 0..notches.unsigned_abs() {
            if pm.track != Track::Off && !ev.modifiers.shift {
                self.report(
                    if up { Phase::WheelUp } else { Phase::WheelDown },
                    0,
                    Self::mods(&ev.modifiers),
                    col,
                    row,
                );
            } else if vm.alt_screen && !ev.modifiers.shift {
                let bytes = termio::wheel_arrows(up, vm);
                self.send(bytes);
            } else if up {
                self.scroll = (self.scroll + 3).min(hist);
            } else {
                self.scroll = self.scroll.saturating_sub(3);
            }
        }
        cx.notify();
    }

    fn selection_bounds(&self) -> Option<((i64, u16), (i64, u16))> {
        let s = self.sel?;
        let (a, b) = if s.anchor <= s.head {
            (s.anchor, s.head)
        } else {
            (s.head, s.anchor)
        };
        Some(match s.mode {
            SelMode::Char => (a, b),
            SelMode::Line => ((a.0, 0), (b.0, u16::MAX)),
            SelMode::Word => {
                let st = self.shared.lock();
                let hist = st.term.history_len() as i64;
                let chars = |line: i64| -> Vec<char> {
                    let cols = st.term.size().cols;
                    (0..cols)
                        .map(|c| {
                            if line < hist {
                                st.term.history_cell((hist - line) as usize, c)
                            } else {
                                st.term.cell((line - hist) as u16, c)
                            }
                            .map_or(' ', |c| c.ch)
                        })
                        .collect()
                };
                let word = |ch: char| !ch.is_whitespace();
                let la = chars(a.0);
                let mut s0 = usize::from(a.1).min(la.len().saturating_sub(1));
                while s0 > 0 && la.get(s0 - 1).is_some_and(|c| word(*c)) {
                    s0 -= 1;
                }
                let lb = chars(b.0);
                let mut e0 = usize::from(b.1).min(lb.len().saturating_sub(1));
                while lb.get(e0 + 1).is_some_and(|c| word(*c)) {
                    e0 += 1;
                }
                ((a.0, s0 as u16), (b.0, e0 as u16))
            }
        })
    }

    /// The selected text over screen and history, rows joined with newlines, trailing blanks trimmed.
    pub fn selection_text(&self) -> Option<String> {
        let ((l0, c0), (l1, c1)) = self.selection_bounds()?;
        let st = self.shared.lock();
        let hist = st.term.history_len() as i64;
        let cols = st.term.size().cols;
        let mut out = String::new();
        for line in l0..=l1 {
            let from = if line == l0 { c0 } else { 0 };
            let to = if line == l1 {
                c1.min(cols.saturating_sub(1))
            } else {
                cols.saturating_sub(1)
            };
            let mut s = String::new();
            for c in from..=to {
                let cell = if line < hist {
                    st.term.history_cell((hist - line) as usize, c)
                } else {
                    st.term.cell((line - hist) as u16, c)
                };
                match cell {
                    Some(Cell { spacer: true, .. }) | None => {}
                    Some(c) => s.push(c.ch),
                }
            }
            out.push_str(s.trim_end());
            if line != l1 {
                out.push('\n');
            }
        }
        Some(out)
    }

    pub fn copy(&mut self, cx: &mut Context<Self>) {
        if let Some(t) = self.selection_text() {
            self.metrics.borrow_mut().copied = Some(t.clone());
            if !self.metrics.borrow().no_clipboard {
                cx.write_to_clipboard(ClipboardItem::new_string(t));
            }
        }
    }

    pub fn paste(&mut self, text: &str) {
        let bracketed = self.shared.lock().term.modes().bracketed_paste;
        self.scroll = 0;
        self.send(termio::paste_bytes(text, bracketed));
    }

    pub fn key_down(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let m = ks.modifiers;
        let key = ks.key.as_str();
        if (m.control && m.shift && key == "c")
            || (m.platform && key == "c")
            || (m.control && key == "insert")
        {
            self.copy(cx);
            cx.stop_propagation();
            return;
        }
        if (m.control && m.shift && key == "v")
            || (m.platform && key == "v")
            || (m.shift && key == "insert")
        {
            if let Some(t) = cx.read_from_clipboard().and_then(|i| i.text()) {
                self.paste(&t);
            }
            cx.stop_propagation();
            return;
        }
        let app_cursor = self.shared.lock().term.modes().application_cursor;
        if let Some(bytes) = key_bytes(ks, app_cursor) {
            self.scroll = 0;
            self.sel = None;
            self.send(bytes);
            cx.stop_propagation();
            cx.notify();
        }
        // Anything else is text: it arrives through the input handler (IME, dead keys, compose), see `replace_text_in_range`.
    }

    fn snapshot(&mut self) -> Frame {
        let st = self.shared.lock();
        let size = st.term.size();
        let hist = st.term.history_len();
        // New output while scrolled back keeps the same lines in view.
        if self.scroll > 0 && hist > self.last_hist {
            self.scroll = (self.scroll + hist - self.last_hist).min(hist);
        }
        self.last_hist = hist;
        let scroll = self.scroll.min(hist);
        let mut cells = Vec::with_capacity(usize::from(size.cols) * usize::from(size.rows));
        for r in 0..size.rows {
            for c in 0..size.cols {
                let cell = if usize::from(r) < scroll {
                    st.term.history_cell(scroll - usize::from(r), c)
                } else {
                    st.term.cell(r - scroll as u16, c)
                };
                cells.push(cell.unwrap_or(Cell {
                    ch: ' ',
                    spacer: false,
                    style: Default::default(),
                }));
            }
        }
        let cur = st.term.cursor();
        let cursor = (cur.visible && cur.row as usize + scroll < usize::from(size.rows))
            .then_some((cur.col, cur.row + scroll as u16));
        Frame {
            cols: size.cols,
            rows: size.rows,
            cells,
            cursor,
            generation: st.generation,
            got_screen: st.got_screen,
            first_after_send: st.first_after_send,
        }
    }
}

/// Bytes for a key that is not plain text: special keys, Ctrl and Alt combinations (xterm forms, as the web client and TUI send).
pub fn key_bytes(ks: &Keystroke, app_cursor: bool) -> Option<Vec<u8>> {
    let m = ks.modifiers;
    let key = ks.key.as_str();
    let modn = 1 + u8::from(m.shift) + 2 * u8::from(m.alt) + 4 * u8::from(m.control);
    let csi_mod = |final_: char| -> Vec<u8> {
        if modn > 1 {
            format!("\x1b[1;{modn}{final_}").into_bytes()
        } else if app_cursor && "ABCDHF".contains(final_) {
            format!("\x1bO{final_}").into_bytes()
        } else {
            format!("\x1b[{final_}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if modn > 1 {
            format!("\x1b[{n};{modn}~").into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };
    let out = match key {
        "enter" => {
            if m.alt {
                b"\x1b\r".to_vec()
            } else {
                b"\r".to_vec()
            }
        }
        "tab" if m.shift => b"\x1b[Z".to_vec(),
        "tab" => b"\t".to_vec(),
        "backspace" if m.control => vec![0x08],
        "backspace" if m.alt => b"\x1b\x7f".to_vec(),
        "backspace" => vec![0x7f],
        "escape" => vec![0x1b],
        "up" => csi_mod('A'),
        "down" => csi_mod('B'),
        "right" => csi_mod('C'),
        "left" => csi_mod('D'),
        "home" => csi_mod('H'),
        "end" => csi_mod('F'),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "insert" => tilde(2),
        "delete" => tilde(3),
        "f1" => b"\x1bOP".to_vec(),
        "f2" => b"\x1bOQ".to_vec(),
        "f3" => b"\x1bOR".to_vec(),
        "f4" => b"\x1bOS".to_vec(),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        "space" if m.control => vec![0],
        _ if m.control && !m.platform => {
            let c = key.chars().next()?;
            if key.chars().count() != 1 {
                return None;
            }
            let b = match c {
                'a'..='z' => c as u8 - b'a' + 1,
                '@' | '2' => 0,
                '[' | '3' => 0x1b,
                '\\' | '4' => 0x1c,
                ']' | '5' => 0x1d,
                '^' | '6' => 0x1e,
                '_' | '-' | '7' => 0x1f,
                '8' => 0x7f,
                _ => return None,
            };
            if m.alt {
                vec![0x1b, b]
            } else {
                vec![b]
            }
        }
        _ if m.alt && !m.platform => {
            let text = ks.key_char.clone().unwrap_or_else(|| ks.key.clone());
            if text.chars().count() != 1 && key != "space" {
                return None;
            }
            let mut v = vec![0x1b];
            v.extend_from_slice(if key == "space" {
                b" "
            } else {
                text.as_bytes()
            });
            v
        }
        _ => return None,
    };
    Some(out)
}

impl EntityInputHandler for TermPane {
    fn text_for_range(
        &mut self,
        _r: std::ops::Range<usize>,
        _a: &mut Option<std::ops::Range<usize>>,
        _w: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        None
    }
    fn selected_text_range(
        &mut self,
        _i: bool,
        _w: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let n = self
            .preedit
            .as_ref()
            .map_or(0, |p| p.encode_utf16().count());
        Some(UTF16Selection {
            range: n..n,
            reversed: false,
        })
    }
    fn marked_text_range(
        &self,
        _w: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<std::ops::Range<usize>> {
        self.preedit.as_ref().map(|p| 0..p.encode_utf16().count())
    }
    fn unmark_text(&mut self, _w: &mut Window, cx: &mut Context<Self>) {
        self.preedit = None;
        cx.notify();
    }
    fn replace_text_in_range(
        &mut self,
        _r: Option<std::ops::Range<usize>>,
        text: &str,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Committed text (a key's character, a finished IME composition): sent once, preedit cleared.
        self.preedit = None;
        if !text.is_empty() {
            self.scroll = 0;
            self.sel = None;
            self.send(text.as_bytes().to_vec());
        }
        cx.notify();
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        _r: Option<std::ops::Range<usize>>,
        new_text: &str,
        _sel: Option<std::ops::Range<usize>>,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preedit = (!new_text.is_empty()).then(|| new_text.to_string());
        if let Some(p) = &self.preedit {
            self.metrics.borrow_mut().preedit_seen = Some(p.clone());
        }
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        _r: std::ops::Range<usize>,
        element_bounds: Bounds<Pixels>,
        _w: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // The candidate window goes under the cursor cell.
        let m = self.cell.get()?;
        let st = self.shared.lock();
        let c = st.term.cursor();
        Some(Bounds::new(
            point(
                element_bounds.origin.x + m.width * f32::from(c.col),
                element_bounds.origin.y + m.height * f32::from(c.row),
            ),
            size(m.width, m.height),
        ))
    }
    fn character_index_for_point(
        &mut self,
        _p: Point<Pixels>,
        _w: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl Render for TermPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Cleared before reading, so output that lands while this frame is copied wakes the next one.
        self.shared.dirty.store(false, Ordering::Release);
        let frame = self.snapshot();
        let sel = self.selection_bounds();
        let hist = self.last_hist;
        let top_line = hist as i64 - self.scroll.min(hist) as i64;
        let preedit = self.preedit.clone();
        let entity = cx.entity();
        let focus = self.focus.clone();
        let focused = focus.is_focused(_window);
        let cell_rc = self.cell.clone();
        let bounds_rc = self.bounds.clone();
        let size_rc = self.last_size.clone();
        let metrics = self.metrics.clone();
        let shared = self.shared.clone();
        let tx = self.tx.clone();
        let id = self.id;
        let fnt = self.font.clone();
        let measure_font = self.font.clone();
        div()
            .id(("pane", self.id))
            .track_focus(&self.focus)
            .size_full()
            .bg(rgb(DEFAULT_BG))
            .border_1()
            .border_color(if focused {
                rgb(0x4c8dff)
            } else {
                rgb(0x2a2a2e)
            })
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::mouse_up))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_scroll(cx.listener(Self::scroll_wheel))
            .child(
                canvas(
                    move |bounds, window, _cx| {
                        let ts = window.text_system();
                        let font_size = px(FONT_SIZE);
                        let font_id = ts.resolve_font(&measure_font);
                        let width = ts
                            .advance(font_id, font_size, 'm')
                            .map(|s| s.width)
                            .unwrap_or(px(8.0));
                        let height = (font_size * 1.35).round();
                        let m = CellMetrics { width, height };
                        cell_rc.set(Some(m));
                        bounds_rc.set(bounds);
                        let cols = ((bounds.size.width / width).floor() as u16).max(2);
                        let rows = ((bounds.size.height / height).floor() as u16).max(1);
                        if size_rc.get() != (cols, rows) {
                            size_rc.set((cols, rows));
                            // The local screen follows at once; the machine's PTY follows the resize command.
                            shared.lock().term.resize(Size { cols, rows });
                            let _ = tx.send(Cmd::Resize {
                                node: shared.node.clone(),
                                terminal_id: shared.terminal_id.clone(),
                                cols,
                                rows,
                            });
                        }
                        m
                    },
                    move |bounds, m, window, cx| {
                        let started = Instant::now();
                        window.handle_input(
                            &focus,
                            ElementInputHandler::new(bounds, entity.clone()),
                            cx,
                        );
                        let font_size = px(FONT_SIZE);
                        let cols = usize::from(frame.cols);
                        let origin = bounds.origin;
                        let bold = Font {
                            weight: FontWeight::BOLD,
                            ..fnt.clone()
                        };
                        let italic = Font {
                            style: FontStyle::Italic,
                            ..fnt.clone()
                        };
                        for r in 0..usize::from(frame.rows) {
                            let row = &frame.cells[r * cols..(r + 1) * cols];
                            let y = origin.y + m.height * r as f32;
                            if y > bounds.origin.y + bounds.size.height {
                                break;
                            }
                            // Backgrounds: one quad per run of the same colour.
                            let mut c = 0;
                            while c < cols {
                                let (_, bg) = colors(&row[c]);
                                let start = c;
                                while c < cols && colors(&row[c]).1 == bg {
                                    c += 1;
                                }
                                if let Some(bg) = bg {
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            point(origin.x + m.width * start as f32, y),
                                            size(m.width * (c - start) as f32, m.height),
                                        ),
                                        bg,
                                    ));
                                }
                            }
                            // Selection.
                            if let Some(((l0, c0), (l1, c1))) = sel {
                                let line = top_line + r as i64;
                                if line >= l0 && line <= l1 {
                                    let from = if line == l0 { usize::from(c0) } else { 0 };
                                    let to = if line == l1 {
                                        usize::from(c1).min(cols - 1)
                                    } else {
                                        cols - 1
                                    };
                                    if to >= from {
                                        window.paint_quad(fill(
                                            Bounds::new(
                                                point(origin.x + m.width * from as f32, y),
                                                size(m.width * (to - from + 1) as f32, m.height),
                                            ),
                                            Hsla {
                                                a: 0.4,
                                                ..rgb(0x4c8dff).into()
                                            },
                                        ));
                                    }
                                }
                            }
                            // Text: segments of narrow characters shaped as one line each, a wide character on its own at its cell.
                            let mut seg_start = 0usize;
                            let mut text = String::new();
                            let mut runs: Vec<TextRun> = Vec::new();
                            let flush = |start: usize,
                                         text: &mut String,
                                         runs: &mut Vec<TextRun>,
                                         window: &mut Window,
                                         cx: &mut App| {
                                if text.trim().is_empty() {
                                    text.clear();
                                    runs.clear();
                                    return;
                                }
                                let line = window.text_system().shape_line(
                                    SharedString::from(std::mem::take(text)),
                                    font_size,
                                    runs,
                                    Some(m.width),
                                );
                                runs.clear();
                                let _ = line.paint(
                                    point(origin.x + m.width * start as f32, y),
                                    m.height,
                                    TextAlign::Left,
                                    None,
                                    window,
                                    cx,
                                );
                            };
                            let mut c = 0;
                            while c < cols {
                                let cell = &row[c];
                                if cell.spacer {
                                    c += 1;
                                    continue;
                                }
                                let wide = row.get(c + 1).is_some_and(|n| n.spacer);
                                if wide {
                                    flush(seg_start, &mut text, &mut runs, window, cx);
                                    seg_start = c;
                                }
                                let (fg, _) = colors(cell);
                                let f = if cell.style.bold {
                                    bold.clone()
                                } else if cell.style.italic {
                                    italic.clone()
                                } else {
                                    fnt.clone()
                                };
                                let ch = if cell.ch == '\0' { ' ' } else { cell.ch };
                                let len = ch.len_utf8();
                                let underline = cell.style.underline.then_some(UnderlineStyle {
                                    thickness: px(1.0),
                                    color: Some(fg),
                                    wavy: false,
                                });
                                let strike = cell.style.strike.then_some(StrikethroughStyle {
                                    thickness: px(1.0),
                                    color: Some(fg),
                                });
                                match runs.last_mut() {
                                    Some(last)
                                        if last.color == fg
                                            && last.font == f
                                            && last.underline == underline
                                            && last.strikethrough == strike =>
                                    {
                                        last.len += len
                                    }
                                    _ => runs.push(TextRun {
                                        len,
                                        font: f,
                                        color: fg,
                                        background_color: None,
                                        underline,
                                        strikethrough: strike,
                                    }),
                                }
                                text.push(ch);
                                c += 1;
                                if wide {
                                    flush(seg_start, &mut text, &mut runs, window, cx);
                                    seg_start = c + 1;
                                }
                            }
                            flush(seg_start, &mut text, &mut runs, window, cx);
                        }
                        // Cursor (a block when focused, an outline otherwise) and the IME preedit at it.
                        if let Some((cc, cr)) = frame.cursor {
                            let at = point(
                                origin.x + m.width * f32::from(cc),
                                origin.y + m.height * f32::from(cr),
                            );
                            if let Some(p) = &preedit {
                                let run = TextRun {
                                    len: p.len(),
                                    font: fnt.clone(),
                                    color: rgb(0xffffff).into(),
                                    background_color: Some(rgb(0x3a3a40).into()),
                                    underline: Some(UnderlineStyle {
                                        thickness: px(1.0),
                                        color: Some(rgb(0xffffff).into()),
                                        wavy: false,
                                    }),
                                    strikethrough: None,
                                };
                                let line = window.text_system().shape_line(
                                    SharedString::from(p.clone()),
                                    font_size,
                                    &[run],
                                    None,
                                );
                                let _ = line.paint(at, m.height, TextAlign::Left, None, window, cx);
                            } else if focused {
                                window.paint_quad(fill(
                                    Bounds::new(at, size(m.width, m.height)),
                                    Hsla {
                                        a: 0.7,
                                        ..rgb(0xd4d4d4).into()
                                    },
                                ));
                            } else {
                                window.paint_quad(outline(
                                    Bounds::new(at, size(m.width, m.height)),
                                    rgb(0x9a9a9a),
                                    BorderStyle::Solid,
                                ));
                            }
                        }
                        crate::metrics::painted(
                            &metrics,
                            crate::metrics::Painted {
                                pane: id,
                                generation: frame.generation,
                                got_screen: frame.got_screen,
                                first_after_send: frame.first_after_send,
                                with_preedit: preedit.is_some(),
                                started,
                            },
                            window,
                        );
                    },
                )
                .size_full(),
            )
    }
}

//! The bytes a terminal sends for keys, the mouse and pastes, read as the app shell's [`Event`]s.
//!
//! `agora tui` reads stdin as bytes on every platform (the sub-screens that ask a question read the same byte stream), so the shell's
//! events are parsed here rather than by crossterm: printable UTF-8, C0 controls as Ctrl keys, `ESC x` as Alt, CSI and SS3 keys
//! (arrows, Home/End, Page, Delete, Shift-Tab, F1–F12, xterm modifiers), SGR mouse reports (`ESC [ < b ; x ; y M/m`, enabled with
//! `?1000h ?1002h ?1006h`) and bracketed pastes (`ESC [ 200~ … ESC [ 201~`).
//!
//! A lone ESC is ambiguous until the next byte arrives: [`Parser::push`] keeps an unfinished sequence, and [`Parser::flush`] (called
//! when no byte came for a short while) turns what is left into keys.

use crate::app::{Event, Key, Mouse, MouseKind};

/// Longest escape sequence kept while waiting for its end; anything longer is not a sequence this parser knows and is dropped.
const MAX_SEQUENCE: usize = 32;
const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";

#[derive(Debug, Default)]
pub struct Parser {
    buf: Vec<u8>,
    /// Inside a bracketed paste: everything up to its end marker is text.
    paste: Option<Vec<u8>>,
}

enum Step {
    /// An event and how many bytes it used.
    Done(Option<Event>, usize),
    /// The bytes so far may be the start of a longer sequence.
    More,
}

impl Parser {
    /// Adds bytes and returns the events they complete.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Event> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            if let Some(p) = self.paste.as_mut() {
                match find(&self.buf, PASTE_END) {
                    Some(end) => {
                        p.extend_from_slice(&self.buf[..end]);
                        self.buf.drain(..end + PASTE_END.len());
                        let text = String::from_utf8_lossy(p).into_owned();
                        self.paste = None;
                        out.push(Event::Paste(text));
                    }
                    None => {
                        // Keep a tail that may be the start of the end marker.
                        let keep = self.buf.len().min(PASTE_END.len() - 1);
                        let take = self.buf.len() - keep;
                        p.extend(self.buf.drain(..take));
                        return out;
                    }
                }
                continue;
            }
            if self.buf.is_empty() {
                return out;
            }
            match step(&self.buf) {
                Step::Done(ev, used) => {
                    self.buf.drain(..used);
                    if let Some(ev) = ev {
                        out.push(ev);
                    }
                }
                Step::More if self.buf.starts_with(PASTE_START) => {
                    self.buf.drain(..PASTE_START.len());
                    self.paste = Some(Vec::new());
                }
                Step::More if self.buf.len() > MAX_SEQUENCE => {
                    self.buf.clear();
                }
                Step::More => return out,
            }
        }
    }

    /// No byte came for a while: what is left is keys as typed (a lone ESC is Esc; `ESC [` is Alt-[).
    pub fn flush(&mut self) -> Vec<Event> {
        if self.paste.is_some() || self.buf.is_empty() {
            return Vec::new();
        }
        let rest = std::mem::take(&mut self.buf);
        let mut out = Vec::new();
        let mut i = 0;
        while i < rest.len() {
            if rest[i] == 0x1b {
                match rest.get(i + 1) {
                    Some(&b) if (0x20..0x7f).contains(&b) => {
                        out.push(Event::Key(Key::Alt(char::from(b))));
                        i += 2;
                    }
                    _ => {
                        out.push(Event::Key(Key::Esc));
                        i += 1;
                    }
                }
            } else {
                out.extend(self.push(&rest[i..]));
                break;
            }
        }
        out
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn key(k: Key) -> Option<Event> {
    Some(Event::Key(k))
}

fn step(b: &[u8]) -> Step {
    match b[0] {
        0x1b => escape(b),
        b'\r' | b'\n' => Step::Done(key(Key::Enter), 1),
        b'\t' => Step::Done(key(Key::Tab), 1),
        0x7f | 0x08 => Step::Done(key(Key::Backspace), 1),
        0 => Step::Done(key(Key::Ctrl(' ')), 1),
        c @ 1..=26 => Step::Done(key(Key::Ctrl(char::from(b'a' + c - 1))), 1),
        c @ 0x1c..=0x1f => Step::Done(key(Key::Ctrl(char::from(b'\\' + c - 0x1c))), 1),
        _ => utf8(b),
    }
}

/// One UTF-8 character; an invalid byte is skipped.
fn utf8(b: &[u8]) -> Step {
    let len = match b[0] {
        0x00..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return Step::Done(None, 1),
    };
    if b.len() < len {
        return Step::More;
    }
    match std::str::from_utf8(&b[..len])
        .ok()
        .and_then(|s| s.chars().next())
    {
        Some(c) => Step::Done(key(Key::Char(c)), len),
        None => Step::Done(None, 1),
    }
}

fn escape(b: &[u8]) -> Step {
    let Some(&second) = b.get(1) else {
        return Step::More;
    };
    match second {
        b'[' => csi(b),
        b'O' => match b.get(2) {
            None => Step::More,
            Some(&c) => Step::Done(
                match c {
                    b'A' => key(Key::Up),
                    b'B' => key(Key::Down),
                    b'C' => key(Key::Right),
                    b'D' => key(Key::Left),
                    b'H' => key(Key::Home),
                    b'F' => key(Key::End),
                    b'P'..=b'S' => key(Key::F(c - b'P' + 1)),
                    _ => None,
                },
                3,
            ),
        },
        0x1b => Step::Done(key(Key::Esc), 1),
        c if (0x20..0x7f).contains(&c) => Step::Done(key(Key::Alt(char::from(c))), 2),
        // Alt with a non-ASCII character: ESC then its UTF-8.
        _ => match utf8(&b[1..]) {
            Step::Done(Some(Event::Key(Key::Char(c))), used) => {
                Step::Done(key(Key::Alt(c)), used + 1)
            }
            Step::More => Step::More,
            Step::Done(_, _) => Step::Done(key(Key::Esc), 1),
        },
    }
}

/// `ESC [` parameters final: keys, xterm-modified keys, and SGR mouse reports.
fn csi(b: &[u8]) -> Step {
    let body = &b[2..];
    let Some(end) = body.iter().position(|c| (0x40..=0x7e).contains(c)) else {
        return Step::More;
    };
    let used = 2 + end + 1;
    let params = &body[..end];
    let fin = body[end];
    if params.first() == Some(&b'<') {
        return Step::Done(mouse(&params[1..], fin), used);
    }
    if params == b"200" && fin == b'~' {
        return Step::More; // the start of a paste: `push` takes it from here
    }
    let nums: Vec<u16> = std::str::from_utf8(params)
        .unwrap_or("")
        .split(';')
        .map(|n| n.parse().unwrap_or(0))
        .collect();
    // xterm sends a modifier as the second parameter (2 Shift, 3 Alt, 5 Ctrl…); the shell needs only the key.
    let ev = match (fin, nums.first().copied().unwrap_or(0)) {
        (b'A', _) => key(Key::Up),
        (b'B', _) => key(Key::Down),
        (b'C', _) => key(Key::Right),
        (b'D', _) => key(Key::Left),
        (b'H', _) => key(Key::Home),
        (b'F', _) => key(Key::End),
        (b'Z', _) => key(Key::BackTab),
        (b'~', 1 | 7) => key(Key::Home),
        (b'~', 4 | 8) => key(Key::End),
        (b'~', 3) => key(Key::Delete),
        (b'~', 5) => key(Key::PageUp),
        (b'~', 6) => key(Key::PageDown),
        (b'~', n @ 11..=15) => key(Key::F(u8::try_from(n - 10).unwrap_or(0))),
        (b'~', n @ 17..=21) => key(Key::F(u8::try_from(n - 11).unwrap_or(0))),
        (b'~', 23) => key(Key::F(11)),
        (b'~', 24) => key(Key::F(12)),
        _ => None,
    };
    Step::Done(ev, used)
}

/// An SGR mouse report: `b;x;y` with `M` for press/drag, `m` for release; x and y count from 1.
fn mouse(params: &[u8], fin: u8) -> Option<Event> {
    let text = std::str::from_utf8(params).ok()?;
    let mut it = text.split(';').map(|n| n.parse::<u16>().ok());
    let (b, x, y) = (it.next()??, it.next()??, it.next()??);
    let kind = match (b & !0b0001_1100, fin) {
        (64, b'M') => MouseKind::ScrollUp,
        (65, b'M') => MouseKind::ScrollDown,
        (0, b'M') => MouseKind::Down,
        (32, b'M') => MouseKind::Drag,
        (0, b'm') => MouseKind::Up,
        _ => return None,
    };
    Some(Event::Mouse(Mouse {
        kind,
        col: x.saturating_sub(1),
        row: y.saturating_sub(1),
    }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn keys(bytes: &[u8]) -> Vec<Event> {
        let mut p = Parser::default();
        let mut out = p.push(bytes);
        out.extend(p.flush());
        out
    }

    fn k(k: Key) -> Event {
        Event::Key(k)
    }

    #[test]
    fn text_controls_and_utf8() {
        assert_eq!(
            keys("aı\r".as_bytes()),
            vec![k(Key::Char('a')), k(Key::Char('ı')), k(Key::Enter)]
        );
        assert_eq!(
            keys(b"\x03\x0b\x1d\x7f\t"),
            vec![
                k(Key::Ctrl('c')),
                k(Key::Ctrl('k')),
                k(Key::Ctrl(']')),
                k(Key::Backspace),
                k(Key::Tab)
            ]
        );
    }

    #[test]
    fn a_character_split_across_reads_waits_for_its_end() {
        let mut p = Parser::default();
        let s = "ş".as_bytes();
        assert_eq!(p.push(&s[..1]), vec![]);
        assert_eq!(p.push(&s[1..]), vec![k(Key::Char('ş'))]);
    }

    #[test]
    fn arrows_function_keys_and_modified_keys() {
        assert_eq!(
            keys(b"\x1b[A\x1bOB\x1b[1;5C\x1b[Z"),
            vec![k(Key::Up), k(Key::Down), k(Key::Right), k(Key::BackTab)]
        );
        assert_eq!(
            keys(b"\x1bOQ\x1b[15~\x1b[24~\x1b[3~\x1b[5~"),
            vec![
                k(Key::F(2)),
                k(Key::F(5)),
                k(Key::F(12)),
                k(Key::Delete),
                k(Key::PageUp)
            ]
        );
    }

    #[test]
    fn esc_alone_waits_for_the_flush_and_alt_is_esc_then_a_key() {
        let mut p = Parser::default();
        assert_eq!(p.push(b"\x1b"), vec![]);
        assert_eq!(p.flush(), vec![k(Key::Esc)]);
        assert_eq!(keys(b"\x1b1"), vec![k(Key::Alt('1'))]);
        assert_eq!(keys("\x1bı".as_bytes()), vec![k(Key::Alt('ı'))]);
        // An escape split across reads is still one key.
        let mut p = Parser::default();
        assert_eq!(p.push(b"\x1b["), vec![]);
        assert_eq!(p.push(b"B"), vec![k(Key::Down)]);
    }

    #[test]
    fn sgr_mouse_reports() {
        let m = |kind, col, row| Event::Mouse(Mouse { kind, col, row });
        assert_eq!(
            keys(b"\x1b[<0;29;6M\x1b[<32;41;6M\x1b[<0;41;6m"),
            vec![
                m(MouseKind::Down, 28, 5),
                m(MouseKind::Drag, 40, 5),
                m(MouseKind::Up, 40, 5)
            ]
        );
        assert_eq!(
            keys(b"\x1b[<64;10;3M\x1b[<65;10;3M"),
            vec![m(MouseKind::ScrollUp, 9, 2), m(MouseKind::ScrollDown, 9, 2)]
        );
        // A right click and a move without a button are not the shell's.
        assert_eq!(keys(b"\x1b[<2;1;1M\x1b[<35;1;1M"), vec![]);
    }

    #[test]
    fn a_bracketed_paste_is_one_text_even_across_reads_and_with_escapes_inside() {
        let mut p = Parser::default();
        assert_eq!(p.push(b"\x1b[200~line one\r\n\x1b[A"), vec![]);
        assert_eq!(p.push(b"tail\x1b[20"), vec![]);
        assert_eq!(
            p.push(b"1~x"),
            vec![
                Event::Paste("line one\r\n\x1b[Atail".into()),
                k(Key::Char('x'))
            ]
        );
    }

    #[test]
    fn an_unknown_or_endless_sequence_is_dropped_without_eating_what_follows() {
        assert_eq!(keys(b"\x1b[99xq"), vec![k(Key::Char('q'))]);
        let mut p = Parser::default();
        let mut long = b"\x1b[".to_vec();
        long.extend(std::iter::repeat_n(b'1', 40));
        assert_eq!(p.push(&long), vec![]);
        assert_eq!(p.push(b"q"), vec![k(Key::Char('q'))]);
    }
}

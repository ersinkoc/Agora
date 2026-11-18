//! What the terminal pane tells the program besides keys, and what the program's output tells the pane (ADR-0063, ADR-0012):
//! mouse reports, the wheel on a full-screen program, focus events, pastes, links and the title.
//!
//! All pure: bytes in, bytes or text out, no I/O. The emulator (`agora-vt`) keeps the screen, the cursor, bracketed paste, the
//! alternate screen and the title, but not the mouse modes, and the node's repaint (`subscribed_snapshot`) does not carry them
//! either. So this module reads the mode-setting sequences from the byte stream the client already feeds the emulator
//! ([`ModeScanner`]): a program that switched the mouse on **before** this client started watching it is not known to have done so
//! until it switches the mode on again (`docs/ux/terminal-fidelity.md`). Everything a program sends is untrusted text: nothing here
//! executes or opens anything; the caller decides, after showing the user what it would do.

use agora_vt::Modes;

/// Which events the program asked to be told about (DECSET 9, 1000, 1002, 1003).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Track {
    #[default]
    Off,
    /// 9: presses only, no modifiers.
    X10,
    /// 1000: presses, releases and the wheel.
    Normal,
    /// 1002: and drags (motion with a button down).
    Button,
    /// 1003: and motion with no button.
    Any,
}

/// How the program wants the coordinates written (DECSET 1005, 1006, 1015; none is the classic one-byte form).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Encoding {
    #[default]
    Default,
    Utf8,
    Sgr,
    Urxvt,
}

/// The modes of the running program that this client has seen it switch on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ProgramModes {
    pub track: Track,
    pub encoding: Encoding,
    /// 1004: tell the program when its pane gains or loses the keyboard.
    pub focus: bool,
}

/// Where the scanner is in the escape sequence it may be inside (a sequence can be split between two chunks).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum St {
    #[default]
    Ground,
    Esc,
    Csi,
}

/// Reads `CSI ? Pm h` / `CSI ? Pm l` (and the resets `ESC c`, `CSI ! p`) from the program's output.
#[derive(Clone, Debug, Default)]
pub struct ModeScanner {
    st: St,
    params: Vec<u8>,
    modes: ProgramModes,
}

/// Longest parameter list kept; a longer one is not a mode switch.
const MAX_PARAMS: usize = 64;

impl ModeScanner {
    pub fn modes(&self) -> ProgramModes {
        self.modes
    }

    /// Reads a chunk of the program's output.
    pub fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            match self.st {
                St::Ground => {
                    if b == 0x1b {
                        self.st = St::Esc;
                    }
                }
                St::Esc => match b {
                    b'[' => {
                        self.params.clear();
                        self.st = St::Csi;
                    }
                    b'c' => {
                        self.modes = ProgramModes::default();
                        self.st = St::Ground;
                    }
                    0x1b => {}
                    0x20..=0x2f => {}
                    _ => self.st = St::Ground,
                },
                St::Csi => match b {
                    0x1b => self.st = St::Esc,
                    0x18 | 0x1a => self.st = St::Ground,
                    0x20..=0x3f => {
                        if self.params.len() < MAX_PARAMS {
                            self.params.push(b);
                        } else {
                            // Too long to be a mode switch: its final byte changes nothing.
                            self.params.clear();
                            self.params.push(b'#');
                        }
                    }
                    0x40..=0x7e => {
                        self.csi(b);
                        self.st = St::Ground;
                    }
                    _ => {}
                },
            }
        }
    }

    fn csi(&mut self, fin: u8) {
        if self.params == b"!" && fin == b'p' {
            self.modes = ProgramModes::default();
            return;
        }
        let set = match fin {
            b'h' => true,
            b'l' => false,
            _ => return,
        };
        let Some(list) = self.params.strip_prefix(b"?") else {
            return;
        };
        let list = list.to_vec();
        for n in list.split(|&c| c == b';') {
            let Some(n) = std::str::from_utf8(n)
                .ok()
                .and_then(|s| s.parse::<u16>().ok())
            else {
                continue;
            };
            self.mode(n, set);
        }
    }

    fn mode(&mut self, n: u16, set: bool) {
        let track = match n {
            9 => Some(Track::X10),
            1000 => Some(Track::Normal),
            1002 => Some(Track::Button),
            1003 => Some(Track::Any),
            _ => None,
        };
        if let Some(t) = track {
            if set {
                self.modes.track = t;
            } else if self.modes.track == t {
                self.modes.track = Track::Off;
            }
            return;
        }
        let enc = match n {
            1005 => Some(Encoding::Utf8),
            1006 => Some(Encoding::Sgr),
            1015 => Some(Encoding::Urxvt),
            _ => None,
        };
        if let Some(e) = enc {
            if set {
                self.modes.encoding = e;
            } else if self.modes.encoding == e {
                self.modes.encoding = Encoding::Default;
            }
            return;
        }
        if n == 1004 {
            self.modes.focus = set;
        }
    }
}

/// Shift, Alt and Ctrl as the mouse protocol adds them to the button code.
pub const SHIFT: u8 = 4;
pub const ALT: u8 = 8;
pub const CTRL: u8 = 16;

/// What the pointer did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Press,
    Release,
    /// Motion with a button down.
    Drag,
    /// Motion with no button.
    Move,
    WheelUp,
    WheelDown,
}

/// A pointer event in the pane's cells (0-based, top left is 0,0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Report {
    pub phase: Phase,
    /// 0 left, 1 middle, 2 right.
    pub button: u8,
    /// Any of [`SHIFT`], [`ALT`], [`CTRL`].
    pub mods: u8,
    pub col: u16,
    pub row: u16,
}

/// The bytes that tell the program about `r`, or `None` when it asked for no such event or the position does not fit the encoding
/// it asked for (the classic form stops at column 223; a coordinate is never wrapped into a different cell).
pub fn encode_mouse(m: &ProgramModes, r: &Report) -> Option<Vec<u8>> {
    let wheel = matches!(r.phase, Phase::WheelUp | Phase::WheelDown);
    let wanted = match m.track {
        Track::Off => false,
        Track::X10 => r.phase == Phase::Press || wheel,
        Track::Normal => !matches!(r.phase, Phase::Drag | Phase::Move),
        Track::Button => r.phase != Phase::Move,
        Track::Any => true,
    };
    if !wanted {
        return None;
    }
    let button = r.button.min(2);
    let mut code = match r.phase {
        Phase::Press => button,
        Phase::Drag => button | 32,
        Phase::Move => 35,
        Phase::WheelUp => 64,
        Phase::WheelDown => 65,
        // A release has no button in the classic forms; SGR keeps it.
        Phase::Release if m.encoding == Encoding::Sgr => button,
        Phase::Release => 3,
    };
    if m.track != Track::X10 {
        code |= r.mods & (SHIFT | ALT | CTRL);
    }
    let (x, y) = (u32::from(r.col) + 1, u32::from(r.row) + 1);
    let code = u32::from(code);
    match m.encoding {
        Encoding::Sgr => Some(
            format!(
                "\x1b[<{code};{x};{y}{}",
                if r.phase == Phase::Release { 'm' } else { 'M' }
            )
            .into_bytes(),
        ),
        Encoding::Urxvt => Some(format!("\x1b[{};{x};{y}M", code + 32).into_bytes()),
        Encoding::Utf8 => {
            let mut out = b"\x1b[M".to_vec();
            for v in [code + 32, x + 32, y + 32] {
                // The form carries values up to 2047; a larger one is not sent rather than sent as another cell.
                let c = char::from_u32(v).filter(|_| v <= 2047)?;
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
            Some(out)
        }
        Encoding::Default => {
            let bytes = [code + 32, x + 32, y + 32];
            if bytes.iter().any(|&b| b > 255) {
                return None;
            }
            let mut out = b"\x1b[M".to_vec();
            out.extend(bytes.iter().map(|&b| b as u8));
            Some(out)
        }
    }
}

/// Arrow keys sent for one wheel notch on a full-screen program that has no mouse on (a pager, `man`): three, as the web window does.
pub const WHEEL_ARROWS: usize = 3;

/// `WHEEL_ARROWS` arrow keys in the form the program asked for (application cursor keys or not).
pub fn wheel_arrows(up: bool, modes: Modes) -> Vec<u8> {
    let key = match (up, modes.application_cursor) {
        (true, true) => &b"\x1bOA"[..],
        (false, true) => b"\x1bOB",
        (true, false) => b"\x1b[A",
        (false, false) => b"\x1b[B",
    };
    key.repeat(WHEEL_ARROWS)
}

/// The report that tells a program its pane gained (`ESC [ I`) or lost (`ESC [ O`) the keyboard; nothing unless it asked (1004).
pub fn focus_bytes(m: &ProgramModes, gained: bool) -> Option<&'static [u8]> {
    m.focus
        .then_some(if gained { b"\x1b[I" } else { b"\x1b[O" })
}

/// Pasted text without any control character but tab and line breaks, C1 controls included: with no ESC left, a paste cannot carry
/// the end-of-paste marker (or any other sequence) and have the rest typed as keys.
pub fn sanitize_paste(text: &str) -> String {
    text.chars()
        .filter(|&c| matches!(c, '\t' | '\n' | '\r') || !(c.is_control()))
        .collect()
}

/// A paste as the program should get it: inside the bracketed-paste markers when it asked for them (`bracketed`), always sanitized.
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let body = sanitize_paste(text);
    if bracketed {
        format!("\x1b[200~{body}\x1b[201~").into_bytes()
    } else {
        body.into_bytes()
    }
}

/// The longest address that is offered.
const MAX_LINK: usize = 2048;

/// The address when it is a plain web address that is safe to show and open as it is written: `http` or `https`, ASCII only with no
/// space or control character (so what the user reads is what is opened, no look-alike letters), a host, no credentials.
pub fn safe_link(uri: &str) -> Option<&str> {
    if uri.is_empty() || uri.len() > MAX_LINK || !uri.bytes().all(|b| b.is_ascii_graphic()) {
        return None;
    }
    let lower = uri.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains(['@', '\\']) || authority.starts_with(':') {
        return None;
    }
    Some(uri)
}

/// The web address in `chars` (one screen row, one char per column) under column `at`, if there is one.
pub fn link_in_row(chars: &[char], at: usize) -> Option<String> {
    let is_url = |c: char| {
        c.is_ascii_graphic() && !matches!(c, '<' | '>' | '"' | '`' | '{' | '}' | '|' | '\\' | '^')
    };
    if !chars.get(at).copied().is_some_and(is_url) {
        return None;
    }
    let mut start = at;
    while start > 0 && is_url(chars[start - 1]) {
        start -= 1;
    }
    let mut end = at + 1;
    while end < chars.len() && is_url(chars[end]) {
        end += 1;
    }
    let run: String = chars[start..end].iter().collect();
    let lower = run.to_ascii_lowercase();
    // The click may be on any part of the address, and a run may hold more than one (`a,http://b`): the one that covers the column.
    let mut from = 0;
    while let Some(i) = ["http://", "https://"]
        .iter()
        .filter_map(|p| lower[from..].find(p).map(|i| i + from))
        .min()
    {
        let mut tail = &run[i..];
        // Punctuation that ends a sentence is not part of the address; a closing bracket only when it has no opener.
        loop {
            let last = tail.chars().last();
            let drop = match last {
                Some('.' | ',' | ';' | ':' | '!' | '?' | '\'' | '*') => true,
                Some(')') => tail.matches('(').count() < tail.matches(')').count(),
                Some(']') => tail.matches('[').count() < tail.matches(']').count(),
                _ => false,
            };
            if !drop {
                break;
            }
            tail = &tail[..tail.len() - 1];
        }
        let (a, b) = (start + i, start + i + tail.len());
        if (a..b).contains(&at) {
            return safe_link(tail).map(str::to_string);
        }
        from = i + 1;
    }
    None
}

/// The longest title shown, in characters.
pub const MAX_TITLE: usize = 60;

/// A program's title made safe to draw: control, bidirectional and invisible characters become visible escapes (`\x1b`, `\u{202e}`),
/// so a title can neither move the cursor nor reorder the text around it; cut to [`MAX_TITLE`] characters.
pub fn safe_title(title: &str) -> String {
    let mut out = String::new();
    let mut n = 0;
    for c in title.chars() {
        if n >= MAX_TITLE {
            out.push('…');
            break;
        }
        let bad = c.is_control()
            || matches!(c,
                '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{061c}' | '\u{feff}');
        if bad {
            let esc = if u32::from(c) < 0x100 {
                format!("\\x{:02x}", u32::from(c))
            } else {
                format!("\\u{{{:x}}}", u32::from(c))
            };
            n += esc.chars().count();
            out.push_str(&esc);
        } else {
            out.push(c);
            n += 1;
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn scan(chunks: &[&[u8]]) -> ProgramModes {
        let mut s = ModeScanner::default();
        for c in chunks {
            s.feed(c);
        }
        s.modes()
    }

    fn sgr_any() -> ProgramModes {
        ProgramModes {
            track: Track::Any,
            encoding: Encoding::Sgr,
            focus: true,
        }
    }

    fn report(phase: Phase, col: u16, row: u16) -> Report {
        Report {
            phase,
            button: 0,
            mods: 0,
            col,
            row,
        }
    }

    #[test]
    fn the_scanner_follows_what_a_program_switches_on_and_off() {
        let m = scan(&[b"\x1b[?1000h\x1b[?1006h\x1b[?1004h"]);
        assert_eq!(
            m,
            ProgramModes {
                track: Track::Normal,
                encoding: Encoding::Sgr,
                focus: true
            }
        );
        let m = scan(&[b"\x1b[?1000;1002;1006h"]);
        assert_eq!((m.track, m.encoding), (Track::Button, Encoding::Sgr));
        // Switching off a mode that is not the current one changes nothing; the current one turns the mouse off.
        assert_eq!(scan(&[b"\x1b[?1003h\x1b[?1000l"]).track, Track::Any);
        assert_eq!(scan(&[b"\x1b[?1003h\x1b[?1003l"]).track, Track::Off);
        assert_eq!(scan(&[b"\x1b[?9h"]).track, Track::X10);
        assert_eq!(scan(&[b"\x1b[?1015h"]).encoding, Encoding::Urxvt);
        assert_eq!(
            scan(&[b"\x1b[?1006h\x1b[?1006l"]).encoding,
            Encoding::Default
        );
        // A reset (RIS, soft reset) puts everything back.
        assert_eq!(scan(&[b"\x1b[?1000h\x1bc"]), ProgramModes::default());
        assert_eq!(scan(&[b"\x1b[?1004h\x1b[!p"]), ProgramModes::default());
    }

    #[test]
    fn a_sequence_split_between_chunks_is_still_read() {
        let m = scan(&[b"text\x1b", b"[?10", b"06;10", b"00h", b" more"]);
        assert_eq!((m.track, m.encoding), (Track::Normal, Encoding::Sgr));
        let mut s = ModeScanner::default();
        for b in b"\x1b[?1002h\x1b[?1006h" {
            s.feed(&[*b]);
        }
        assert_eq!(s.modes().track, Track::Button);
    }

    #[test]
    fn text_that_looks_like_a_mode_switch_is_not_one() {
        // (An ESC ends a string, as in the emulator, so `ESC ] 0 ; ESC [ ? 1000 h` is a real switch there and here.)
        // Inside a title or any string, in plain text without the escape, in a non-private sequence, or in an over-long one.
        assert_eq!(scan(&[b"\x1b]0;[?1000h\x07"]), ProgramModes::default());
        assert_eq!(scan(&[b"[?1000h"]), ProgramModes::default());
        assert_eq!(scan(&[b"\x1b[1000h"]), ProgramModes::default());
        assert_eq!(scan(&[b"\x1bP1;?1000h\x1b\\"]), ProgramModes::default());
        assert_eq!(scan(&[b"\x1b_?1000h\x07"]), ProgramModes::default());
        let mut long = b"\x1b[?".to_vec();
        long.extend(std::iter::repeat_n(b'1', 100));
        long.extend_from_slice(b"h");
        assert_eq!(scan(&[&long]), ProgramModes::default());
        // After a string ends, a switch counts again.
        assert_eq!(scan(&[b"\x1b]0;t\x07\x1b[?1000h"]).track, Track::Normal);
        assert_eq!(scan(&[b"\x1b]0;t\x1b\\\x1b[?1000h"]).track, Track::Normal);
    }

    #[test]
    fn sgr_reports_carry_button_modifiers_and_release() {
        let m = sgr_any();
        let press = report(Phase::Press, 4, 9);
        assert_eq!(encode_mouse(&m, &press).unwrap(), b"\x1b[<0;5;10M");
        let release = Report {
            phase: Phase::Release,
            button: 2,
            ..press
        };
        assert_eq!(encode_mouse(&m, &release).unwrap(), b"\x1b[<2;5;10m");
        let drag = Report {
            phase: Phase::Drag,
            mods: CTRL | ALT,
            ..press
        };
        assert_eq!(encode_mouse(&m, &drag).unwrap(), b"\x1b[<56;5;10M");
        assert_eq!(
            encode_mouse(&m, &report(Phase::Move, 0, 0)).unwrap(),
            b"\x1b[<35;1;1M"
        );
        assert_eq!(
            encode_mouse(&m, &report(Phase::WheelUp, 0, 0)).unwrap(),
            b"\x1b[<64;1;1M"
        );
        assert_eq!(
            encode_mouse(&m, &report(Phase::WheelDown, 0, 0)).unwrap(),
            b"\x1b[<65;1;1M"
        );
    }

    #[test]
    fn the_classic_form_never_wraps_a_column() {
        let m = ProgramModes {
            track: Track::Normal,
            ..ProgramModes::default()
        };
        assert_eq!(
            encode_mouse(&m, &report(Phase::Press, 0, 0)).unwrap(),
            b"\x1b[M !!"
        );
        // A release has no button in this form.
        assert_eq!(
            encode_mouse(&m, &report(Phase::Release, 1, 1)).unwrap(),
            b"\x1b[M#\"\""
        );
        // Column 223 is the last that fits in a byte (255 - 32); 224 is dropped, not sent as another cell.
        assert!(encode_mouse(&m, &report(Phase::Press, 222, 0)).is_some());
        assert!(encode_mouse(&m, &report(Phase::Press, 223, 0)).is_none());
        assert!(encode_mouse(&m, &report(Phase::Press, 0, 223)).is_none());
    }

    #[test]
    fn urxvt_and_utf8_forms() {
        let r = report(Phase::Press, 299, 1);
        let urxvt = ProgramModes {
            track: Track::Normal,
            encoding: Encoding::Urxvt,
            ..ProgramModes::default()
        };
        assert_eq!(encode_mouse(&urxvt, &r).unwrap(), b"\x1b[32;300;2M");
        let utf8 = ProgramModes {
            encoding: Encoding::Utf8,
            ..urxvt
        };
        let got = encode_mouse(&utf8, &r).unwrap();
        assert_eq!(got, "\x1b[M \u{14c}\"".as_bytes());
        assert!(encode_mouse(&utf8, &report(Phase::Press, 2100, 0)).is_none());
    }

    #[test]
    fn only_the_events_the_program_asked_for_are_reported() {
        let with = |track, phase| {
            let m = ProgramModes {
                track,
                encoding: Encoding::Sgr,
                focus: false,
            };
            encode_mouse(&m, &report(phase, 1, 1)).is_some()
        };
        for phase in [
            Phase::Press,
            Phase::Release,
            Phase::Drag,
            Phase::Move,
            Phase::WheelUp,
        ] {
            assert!(!with(Track::Off, phase), "{phase:?} with the mouse off");
        }
        assert!(with(Track::X10, Phase::Press) && with(Track::X10, Phase::WheelDown));
        assert!(!with(Track::X10, Phase::Release) && !with(Track::X10, Phase::Drag));
        assert!(with(Track::Normal, Phase::Release) && !with(Track::Normal, Phase::Drag));
        assert!(with(Track::Button, Phase::Drag) && !with(Track::Button, Phase::Move));
        assert!(with(Track::Any, Phase::Move));
        // X10 carries no modifiers.
        let x10 = ProgramModes {
            track: Track::X10,
            encoding: Encoding::Sgr,
            focus: false,
        };
        let r = Report {
            mods: CTRL,
            ..report(Phase::Press, 0, 0)
        };
        assert_eq!(encode_mouse(&x10, &r).unwrap(), b"\x1b[<0;1;1M");
    }

    #[test]
    fn the_wheel_on_a_full_screen_program_is_three_arrows_in_its_cursor_form() {
        let plain = Modes::default();
        let app = Modes {
            application_cursor: true,
            ..Modes::default()
        };
        assert_eq!(wheel_arrows(true, plain), b"\x1b[A\x1b[A\x1b[A");
        assert_eq!(wheel_arrows(false, app), b"\x1bOB\x1bOB\x1bOB");
    }

    #[test]
    fn focus_is_reported_only_when_asked() {
        assert_eq!(focus_bytes(&sgr_any(), true), Some(&b"\x1b[I"[..]));
        assert_eq!(focus_bytes(&sgr_any(), false), Some(&b"\x1b[O"[..]));
        assert_eq!(focus_bytes(&ProgramModes::default(), true), None);
    }

    #[test]
    fn a_paste_cannot_end_its_own_bracket_or_carry_controls() {
        let evil = "a\x1b[201~rm -rf ~\r\nb\tc\x07\x08\u{9b}d\x7f";
        assert_eq!(sanitize_paste(evil), "a[201~rm -rf ~\r\nb\tcd");
        assert_eq!(
            paste_bytes(evil, true),
            b"\x1b[200~a[201~rm -rf ~\r\nb\tcd\x1b[201~".to_vec()
        );
        // Not bracketed: the same text, no markers.
        assert_eq!(paste_bytes("x\x1b[A", false), b"x[A".to_vec());
        // Markers pieced together from bytes around a removed one cannot form: there is no ESC left to start one.
        let pieced = paste_bytes("\x1b[201\x1b[200~~", true);
        assert_eq!(pieced, b"\x1b[200~[201[200~~\x1b[201~".to_vec());
        assert_eq!(sanitize_paste("héllo 漢字 😀"), "héllo 漢字 😀");
    }

    #[test]
    fn only_plain_web_addresses_are_links() {
        for ok in [
            "http://a.test",
            "https://a.test/x?y=1#z",
            "HTTPS://A.TEST",
            "http://127.0.0.1:8080/p",
            "https://a.test/a@b",
        ] {
            assert_eq!(safe_link(ok), Some(ok), "{ok}");
        }
        for bad in [
            "",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,x",
            "ftp://a.test",
            "https://",
            "https:///x",
            "https://user:pw@a.test/",
            "https://a.test@evil.test/",
            "https://a.test\\@evil.test",
            "https://a.test/ x",
            "https://a.test/\x1b[31m",
            "https://а.test/",
            "https://a.test/é",
            "//a.test",
        ] {
            assert_eq!(safe_link(bad), None, "{bad:?}");
        }
        let long = format!("https://a.test/{}", "x".repeat(MAX_LINK));
        assert_eq!(safe_link(&long), None);
    }

    fn at(row: &str, col: usize) -> Option<String> {
        let chars: Vec<char> = row.chars().collect();
        link_in_row(&chars, col)
    }

    #[test]
    fn a_link_is_found_under_the_pointer_and_only_there() {
        let row = "see https://a.test/x, or (https://b.test/y). ok";
        assert_eq!(at(row, 4).as_deref(), Some("https://a.test/x"));
        assert_eq!(at(row, 19).as_deref(), Some("https://a.test/x"));
        // The comma, the spaces and the words around it are not part of it.
        assert_eq!(at(row, 20), None);
        assert_eq!(at(row, 0), None);
        assert_eq!(at(row, 28).as_deref(), Some("https://b.test/y"));
        assert_eq!(at(row, 43), None);
        assert_eq!(
            at("go to http://x.test/a_(b) now", 10).as_deref(),
            Some("http://x.test/a_(b)")
        );
        // Not a web address: nothing, even under the pointer.
        assert_eq!(at("open javascript:alert(1) now", 10), None);
        assert_eq!(at("file:///etc/passwd", 3), None);
        assert_eq!(at("x,https://a.test/p", 1), None);
        assert_eq!(
            at("x,https://a.test/p", 5).as_deref(),
            Some("https://a.test/p")
        );
        assert_eq!(at("https://user:pw@a.test/", 3), None);
    }

    #[test]
    fn a_title_is_drawn_without_control_or_reordering_characters() {
        assert_eq!(safe_title("vim - main.rs"), "vim - main.rs");
        assert_eq!(safe_title("a\x1b[2Jb\x07"), "a\\x1b[2Jb\\x07");
        assert_eq!(safe_title("x\u{202e}gpj.exe"), "x\\u{202e}gpj.exe");
        assert_eq!(
            safe_title("a\u{200b}b\u{2066}c\u{feff}"),
            "a\\u{200b}b\\u{2066}c\\u{feff}"
        );
        assert_eq!(safe_title("a\u{9b}b\x7f"), "a\\x9bb\\x7f");
        assert_eq!(safe_title("漢字 ok 😀"), "漢字 ok 😀");
        let long = "x".repeat(500);
        let cut = safe_title(&long);
        assert_eq!(cut.chars().count(), MAX_TITLE + 1);
        assert!(cut.ends_with('…'));
        // An escape that would cross the cap still ends the title.
        assert!(
            safe_title(&format!("{}\x1b", "y".repeat(MAX_TITLE - 1)))
                .chars()
                .count()
                <= MAX_TITLE + 4
        );
    }
}

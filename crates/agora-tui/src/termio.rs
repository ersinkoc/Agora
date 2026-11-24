//! What the terminal pane tells the program besides keys, and what the program's output tells the pane (ADR-0063, ADR-0012):
//! mouse reports, the wheel on a full-screen program, focus events, pastes, links and the title.
//!
//! All pure: bytes in, bytes or text out, no I/O. The emulator (`agora-vt`) keeps the screen, the cursor, bracketed paste, the
//! alternate screen and the title. The mouse and focus modes are read here from the byte stream the client already feeds the
//! emulator ([`ModeScanner`]). The node's repaint sets or resets every mode its emulator tracks (ADR-0100), mouse tracking 1000/1002/1003,
//! encodings 1005/1006 and focus 1004 among them, so a program that switched the mouse on before this client started watching is
//! known from the first repaint; mouse 9 and the urxvt encoding 1015, which the node does not track, are known once the program
//! switches them while this client watches (`docs/ux/terminal-fidelity.md`). Everything a program sends is untrusted text: nothing here
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
    /// After an intermediate (`ESC SP`, `ESC (`): the next final byte ends the sequence and is not a reset.
    EscI,
    Csi,
    /// Inside a CSI sequence that can no longer switch a mode or reset (most cursor moves and colours): only its end matters.
    CsiSkip,
}

/// Reads `CSI ? Pm h` / `CSI ? Pm l` (and the resets `ESC c`, `CSI ! p`) from the program's output.
#[derive(Clone, Debug, Default)]
pub struct ModeScanner {
    st: St,
    params: CsiParams,
    modes: ProgramModes,
}

/// Most numbers (parameters and `:` sub-parameters together) in one sequence, as the emulator's parser (`vte`) keeps them; a
/// sequence with more is dropped there, so it is no mode switch here either.
const MAX_PARAMS: usize = 32;

/// The sequence being read after `ESC [`, kept the way the emulator's parser keeps it: numbers rather than bytes (each saturating
/// at 65535), so a list of any length the emulator acts on is read here too, and one it drops changes nothing.
#[derive(Clone, Debug, Default)]
struct CsiParams {
    /// A byte has been read since `ESC [` (a private marker counts only as the first one).
    started: bool,
    /// The private marker (`<`, `=`, `>`, `?`) when it was the first byte.
    private: Option<u8>,
    /// How many intermediate bytes (`SP` to `/`), and whether they are exactly one `!`.
    inter: u8,
    bang: bool,
    /// A parameter byte (digit, `:`, `;`) has been read.
    any_param: bool,
    /// Numbers stored so far, as the emulator's parser counts them.
    stored: usize,
    /// The first number of each parameter (what a mode switch reads), `firsts` of them: fixed storage, so reading output never
    /// allocates (it runs for every byte of every pane).
    first: [u16; MAX_PARAMS],
    firsts: usize,
    /// The number being read, and whether it is a sub-parameter (after `:`).
    cur: u16,
    sub: bool,
    /// The emulator ignores this sequence.
    ignore: bool,
}

impl CsiParams {
    /// A new sequence. The stored numbers are left as they are: only the first `firsts` are read.
    fn reset(&mut self) {
        self.started = false;
        self.private = None;
        self.inter = 0;
        self.bang = false;
        self.any_param = false;
        self.stored = 0;
        self.firsts = 0;
        self.cur = 0;
        self.sub = false;
        self.ignore = false;
    }

    fn byte(&mut self, b: u8) {
        let first = !self.started;
        self.started = true;
        if self.ignore {
            return;
        }
        match b {
            0x3c..=0x3f if first => self.private = Some(b),
            0x3c..=0x3f => self.ignore = true,
            0x20..=0x2f => {
                self.inter = self.inter.saturating_add(1);
                self.bang = self.inter == 1 && b == b'!';
            }
            // Only a `?` list switches a mode: in any other sequence (cursor moves, colours: most of the output) a parameter only
            // matters as being there (it rules out the `CSI ! p` reset), so its numbers are not read.
            _ if self.private != Some(b'?') => self.any_param = true,
            // A parameter after an intermediate: the emulator ignores the sequence.
            _ if self.inter > 0 => self.ignore = true,
            _ if self.stored == MAX_PARAMS => self.ignore = true,
            b'0'..=b'9' => {
                self.any_param = true;
                self.cur = self
                    .cur
                    .saturating_mul(10)
                    .saturating_add(u16::from(b - b'0'));
            }
            _ => {
                self.any_param = true;
                self.end_number();
                self.sub = b == b':';
            }
        }
    }

    /// Whether the sequence can no longer be a mode switch (a `?` list the emulator acts on) or the `CSI ! p` reset (no
    /// parameter): what [`ModeScanner::csi`] would do with it at its final byte is nothing.
    fn dead(&self) -> bool {
        self.ignore || (self.any_param && self.private != Some(b'?'))
    }

    /// Stores the number being read (at a `:`, a `;` or the final byte).
    fn end_number(&mut self) {
        if !self.sub {
            // Never full here: a number is stored only while fewer than MAX_PARAMS are.
            if let Some(slot) = self.first.get_mut(self.firsts) {
                *slot = self.cur;
                self.firsts += 1;
            }
        }
        self.stored += 1;
        self.cur = 0;
    }

    /// The final byte: whether the emulator acts on the sequence, the last number stored.
    fn finish(&mut self) -> bool {
        if self.ignore || self.stored == MAX_PARAMS {
            return false;
        }
        self.end_number();
        true
    }
}

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
                        self.params.reset();
                        self.st = St::Csi;
                    }
                    b'c' => {
                        self.modes = ProgramModes::default();
                        self.st = St::Ground;
                    }
                    0x1b => {}
                    0x00..=0x17 | 0x19 | 0x1c..=0x1f | 0x7f | 0x80..=0xff => {}
                    0x20..=0x2f => self.st = St::EscI,
                    _ => self.st = St::Ground,
                },
                St::EscI => match b {
                    0x1b => self.st = St::Esc,
                    0x20..=0x2f | 0x00..=0x17 | 0x19 | 0x1c..=0x1f | 0x7f | 0x80..=0xff => {}
                    _ => self.st = St::Ground,
                },
                St::Csi => match b {
                    0x1b => self.st = St::Esc,
                    0x18 | 0x1a => self.st = St::Ground,
                    0x20..=0x3f => {
                        self.params.byte(b);
                        if self.params.dead() {
                            self.st = St::CsiSkip;
                        }
                    }
                    0x40..=0x7e => {
                        self.csi(b);
                        self.st = St::Ground;
                    }
                    _ => {}
                },
                St::CsiSkip => match b {
                    0x1b => self.st = St::Esc,
                    0x18 | 0x1a | 0x40..=0x7e => self.st = St::Ground,
                    _ => {}
                },
            }
        }
    }

    fn csi(&mut self, fin: u8) {
        let p = &self.params;
        if p.bang && p.private.is_none() && !p.any_param && !p.ignore && fin == b'p' {
            self.modes = ProgramModes::default();
            return;
        }
        let set = match fin {
            b'h' => true,
            b'l' => false,
            _ => return,
        };
        if p.private != Some(b'?') || p.inter > 0 || !self.params.finish() {
            return;
        }
        let (first, firsts) = (self.params.first, self.params.firsts);
        for &n in first.iter().take(firsts) {
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

/// The bytes an address may hold: the RFC 3986 unreserved and reserved characters minus `^ " ( ) ! < > |` (`cmd.exe` and shells act
/// on those; `%` is allowed only as a percent-encoding (`%` and two hex digits), a `%NAME%` shape is not a link; `&` is allowed, no shell reads the address), that is letters, digits and `-._~:/?#[]@$'*+,;=&`.
/// Nothing else: no space, control character, backtick, brace, backslash or non-ASCII byte. A link that is not made of these is not
/// offered, so the footer never shows one that cannot be opened safely (`agora_platform::is_openable_url` refuses the same set; the
/// opener is the last line of defence, this is the first).
fn url_byte_ok(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"-._~:/?#[]@$'*+,;=&".contains(&b)
}

/// Whether every byte of `url` is allowed: the bytes of `url_byte_ok`, and a `%` only as the start of a percent-encoding (`%` and two hex
/// digits). So `%20` passes, while `%NAME%`, `%1` and a lone `%` do not.
fn url_bytes_ok(url: &str) -> bool {
    let b = url.as_bytes();
    b.iter().enumerate().all(|(i, &c)| {
        if c == b'%' {
            b.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                && b.get(i + 2).is_some_and(u8::is_ascii_hexdigit)
        } else {
            url_byte_ok(c)
        }
    })
}

/// The address when it is a plain web address that is safe to show and open as it is written: `http` or `https`, only the bytes of
/// [`url_byte_ok`], a host, no credentials, at most 2048 bytes.
pub fn safe_link(uri: &str) -> Option<&str> {
    if uri.is_empty() || uri.len() > MAX_LINK || !url_bytes_ok(uri) {
        return None;
    }
    let lower = uri.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') || authority.starts_with(':') {
        return None;
    }
    let port_ok = |port: &str| {
        !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) && port.parse::<u16>().is_ok()
    };
    if let Some(host) = authority.strip_prefix('[') {
        let close = host.find(']')?;
        if close == 0 {
            return None;
        }
        let rest = &host[close + 1..];
        if !rest.is_empty() && rest.strip_prefix(':').is_none_or(|p| !port_ok(p)) {
            return None;
        }
    } else if let Some((host, port)) = authority.split_once(':') {
        if host.is_empty() || host.contains(':') || !port_ok(port) {
            return None;
        }
    }
    Some(uri)
}

/// The web address in `chars` (one screen row, one char per column) under column `at`, if there is one. The address is the whole
/// run of non-blank characters from its `http://` or `https://` (the last one at or before the column) to the end of the run, minus
/// the punctuation that ends a sentence; if that is not a [`safe_link`] there is no link (it is never cut down to a part that is).
pub fn link_in_row(chars: &[char], at: usize) -> Option<String> {
    let blank = |c: char| c.is_whitespace() || c.is_control();
    if chars.get(at).copied().is_none_or(blank) {
        return None;
    }
    let mut start = at;
    while start > 0 && !blank(chars[start - 1]) {
        start -= 1;
    }
    let mut end = at + 1;
    while end < chars.len() && !blank(chars[end]) {
        end += 1;
    }
    let run = &chars[start..end];
    let lower: Vec<char> = run.iter().map(char::to_ascii_lowercase).collect();
    let starts = |i: usize, p: &str| {
        lower.len() >= i + p.len() && lower[i..i + p.len()].iter().copied().eq(p.chars())
    };
    let first = (0..=at - start)
        .rev()
        .find(|&i| starts(i, "http://") || starts(i, "https://"))?;
    let mut last = run.len();
    while last > first
        && matches!(
            run[last - 1],
            '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '*' | ')' | ']' | '}' | '>' | '"'
        )
    {
        last -= 1;
    }
    if start + last <= at {
        return None;
    }
    let text: String = run[first..last].iter().collect();
    safe_link(&text).map(str::to_string)
}

/// What a Ctrl+click on an address does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkStep {
    /// Show the address (in the footer) and wait for a second click on it.
    Confirm,
    /// Open it: the same address was shown, and nothing has happened since.
    Open,
}

#[derive(Clone, Debug)]
struct Armed {
    url: String,
    at: std::time::Instant,
    /// The footer's text while it asked: when the footer says anything else the question is gone.
    notice: String,
    /// The release of the click that armed it is still to come, and is not "something else".
    release_due: bool,
    /// The footer with the address has been drawn: until then a second click cannot open it (the person has not seen it).
    drawn: bool,
}

/// The two-click rule for opening an address, as a state machine with no terminal in it. An address is opened only by a second
/// Ctrl+click on the very address the first one showed, with nothing in between: the footer still says it (and has not run out),
/// no key, no other pointer event, no tab switch. The caller feeds it what happens ([`LinkConfirm::release`], [`LinkConfirm::other`],
/// [`LinkConfirm::check`]) and acts on [`LinkStep`].
#[derive(Clone, Debug, Default)]
pub struct LinkConfirm {
    armed: Option<Armed>,
}

impl LinkConfirm {
    /// A Ctrl+click on `url`; `notice` is the footer text that shows it, `life` how long that stays up.
    pub fn click(
        &mut self,
        url: &str,
        notice: &str,
        now: std::time::Instant,
        life: std::time::Duration,
    ) -> LinkStep {
        let same = self
            .armed
            .as_ref()
            .is_some_and(|a| a.drawn && a.url == url && now.saturating_duration_since(a.at) < life);
        if same {
            self.armed = None;
            return LinkStep::Open;
        }
        self.armed = Some(Armed {
            url: url.to_string(),
            at: now,
            notice: notice.to_string(),
            release_due: true,
            drawn: false,
        });
        LinkStep::Confirm
    }

    /// The footer was drawn with the address in it: from now on a second Ctrl+click can open it.
    pub fn drawn(&mut self) {
        if let Some(a) = self.armed.as_mut() {
            a.drawn = true;
        }
    }

    /// A mouse release: the first one after the click that armed is that click's own and changes nothing; any later one is "something
    /// else".
    pub fn release(&mut self) {
        match self.armed.as_mut() {
            Some(a) if a.release_due => a.release_due = false,
            _ => self.armed = None,
        }
    }

    /// Anything else happened (a key, a move, a click elsewhere, a tab switch, a Ctrl+click that is on no address): forget.
    pub fn other(&mut self) {
        self.armed = None;
    }

    /// Looked at every turn: forget when the footer shows something other than what asked, or it ran out.
    pub fn check(&mut self, now: std::time::Instant, notice: &str, life: std::time::Duration) {
        if self
            .armed
            .as_ref()
            .is_some_and(|a| a.notice != notice || now.saturating_duration_since(a.at) >= life)
        {
            self.armed = None;
        }
    }

    pub fn is_armed(&self) -> bool {
        self.armed.is_some()
    }
}

/// The longest title shown, in characters.
pub const MAX_TITLE: usize = 60;

/// Characters that draw as nothing, as something else, or that reorder text, besides the control characters: written as escapes.
fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(c,
            '\u{200b}'..='\u{200f}'
            | '\u{2028}'..='\u{202e}'
            | '\u{2060}'..='\u{206f}'
            | '\u{e0000}'..='\u{e007f}'
            | '\u{061c}' | '\u{feff}' | '\u{3164}' | '\u{2800}' | '\u{00ad}' | '\u{fe0f}' | '\u{180e}')
}

/// A program's title made safe to draw: control, bidirectional and invisible characters become visible escapes (`\x1b`, `\u{202e}`),
/// so a title can neither move the cursor nor reorder the text around it; cut to [`MAX_TITLE`] characters (the caller also cuts it to
/// the width it has, by display width).
pub fn safe_title(title: &str) -> String {
    let mut out = String::new();
    let mut n = 0;
    for c in title.chars() {
        if n >= MAX_TITLE {
            out.push('…');
            break;
        }
        if hidden(c) {
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
    fn the_scanner_resets_exactly_when_the_emulator_does() {
        use agora_vt::{AlacrittyTerminal, Size, Terminal};
        // `ESC <byte> c` between the escape and the final: a reset or not, decided by the emulator the screen is drawn by.
        let mids: Vec<Option<u8>> = std::iter::once(None)
            .chain((0u8..=0xff).filter(|b| !matches!(b, b'c' | b'[')).map(Some))
            .collect();
        let both = [&b""[..], b"("]
            .into_iter()
            .flat_map(|pre| mids.iter().map(move |m| (pre, *m)));
        for (pre, mid) in both {
            let mut seq = b"x\x1b[?1000h\x1b".to_vec();
            seq.extend_from_slice(pre);
            seq.extend(mid);
            seq.push(b'c');
            let mut vt = AlacrittyTerminal::new(Size { cols: 10, rows: 2 });
            vt.feed(&seq);
            let cleared = (0..2)
                .flat_map(|r| (0..10).map(move |c| (r, c)))
                .all(|(r, c)| vt.cell(r, c).is_none_or(|c| c.ch == ' '));
            let mut s = ModeScanner::default();
            s.feed(&seq);
            assert_eq!(s.modes().track == Track::Off, cleared, "byte {mid:?}");
        }
    }

    #[test]
    fn the_scanner_never_buffers_more_than_a_short_parameter_list() {
        let mut s = ModeScanner::default();
        s.feed(b"\x1b[?");
        for _ in 0..10_000 {
            s.feed(b"1111111111;");
            assert!(s.params.firsts <= MAX_PARAMS, "{}", s.params.firsts);
        }
        // The sequence that ended an over-long list changes nothing.
        s.feed(b"h");
        assert_eq!(s.modes(), ProgramModes::default());
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
            "https://a.test/a%20b%E2%80%93",
        ] {
            assert_eq!(safe_link(ok), Some(ok), "{ok}");
        }
        for bad in [
            "",
            "javascript:alert(1)",
            "javascript:a.test/x",
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
            "https://a.test/%GITHUB_TOKEN%",
            "https://a.test/%4",
            "https://a.test/%41%",
            "https://a.test/^x",
            "https://a.test/\"x",
            "https://a.test/a(b)",
            "https://a.test/!x",
            "https://a.test/<x>",
            "https://a.test/a|b",
            "https://a.test/`x`",
            "https://a.test/{x}",
            "https://a.test/\\x",
            "https://a.test/\u{7f}",
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
        // An address with a character that cannot be opened safely is no link at all, not a shorter one.
        assert_eq!(at("go to http://x.test/a_(b) now", 10), None);
        assert_eq!(
            at("go to https://a.test/?x=1&y=2 now", 10).as_deref(),
            Some("https://a.test/?x=1&y=2")
        );
        assert_eq!(
            at("(https://a.test/p)", 5).as_deref(),
            Some("https://a.test/p")
        );
        assert_eq!(
            at("<https://a.test/p>", 5).as_deref(),
            Some("https://a.test/p")
        );
        // The click on the second of two addresses in one run gets the second.
        assert_eq!(
            at("http://a.test/1,https://b.test/2", 25).as_deref(),
            Some("https://b.test/2")
        );
        assert_eq!(at("https://a.test/\u{e9}x", 3), None);
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
    fn a_second_ctrl_click_on_the_same_address_opens_it_and_nothing_else_does() {
        use std::time::{Duration, Instant};
        let life = Duration::from_secs(10);
        let t0 = Instant::now();
        let url = "https://a.test/x";
        let shown = "Link: https://a.test/x";
        let arm =
            |l: &mut LinkConfirm| assert_eq!(l.click(url, shown, t0, life), LinkStep::Confirm);
        // The first click only shows; the second one, after the first one's own release, opens, once.
        let mut l = LinkConfirm::default();
        arm(&mut l);
        assert!(l.is_armed());
        l.release();
        l.check(t0, shown, life);
        l.drawn();
        assert_eq!(l.click(url, shown, t0, life), LinkStep::Open);
        assert!(!l.is_armed());
        assert_eq!(
            l.click(url, shown, t0, life),
            LinkStep::Confirm,
            "once only"
        );
        // Clicks that arrive before the footer was drawn (one batch of events) cannot open it, and the same address clicked again
        // is shown again with the clock restarted.
        let mut l = LinkConfirm::default();
        arm(&mut l);
        l.release();
        assert_eq!(l.click(url, shown, t0, life), LinkStep::Confirm);
        let later = t0 + life - Duration::from_secs(1);
        assert_eq!(l.click(url, shown, later, life), LinkStep::Confirm);
        l.release();
        l.drawn();
        let after_first = t0 + life + Duration::from_secs(5);
        assert_eq!(l.click(url, shown, after_first, life), LinkStep::Open);
        // Another address is a first click.
        let mut l = LinkConfirm::default();
        arm(&mut l);
        assert_eq!(
            l.click("https://b.test/", shown, t0, life),
            LinkStep::Confirm
        );
        // A key, a move, a click elsewhere, a tab switch (all `other`) forget it.
        let mut l = LinkConfirm::default();
        arm(&mut l);
        l.other();
        assert_eq!(l.click(url, shown, t0, life), LinkStep::Confirm);
        // A second release (not the arming click's own) forgets it.
        let mut l = LinkConfirm::default();
        arm(&mut l);
        l.release();
        l.release();
        assert!(!l.is_armed());
        // A different footer text forgets it; so does running out; a still-valid one does not.
        let mut l = LinkConfirm::default();
        arm(&mut l);
        l.check(t0 + life / 2, shown, life);
        assert!(l.is_armed());
        l.check(t0 + life / 2, "Copied.", life);
        assert!(!l.is_armed());
        arm(&mut l);
        l.check(t0 + life, shown, life);
        assert!(!l.is_armed());
        // A second click after the first one ran out is a first click again.
        arm(&mut l);
        assert_eq!(l.click(url, shown, t0 + life, life), LinkStep::Confirm);
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
        // More characters that draw as nothing or as something else.
        for c in [
            '\u{206a}',
            '\u{206f}',
            '\u{e0001}',
            '\u{e007f}',
            '\u{3164}',
            '\u{2800}',
            '\u{ad}',
            '\u{fe0f}',
            '\u{180e}',
        ] {
            let got = safe_title(&format!("a{c}b"));
            let n = u32::from(c);
            let want = if n < 0x100 {
                format!("a\\x{n:02x}b")
            } else {
                format!("a\\u{{{n:x}}}b")
            };
            assert_eq!(got, want, "{c:?}");
        }
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

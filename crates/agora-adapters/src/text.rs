//! What the node and `agora acp-bridge` do to text from an agent before anyone sees it: clean it (escape sequences, control
//! characters, direction controls, by how it is shown) and bound it. Shared so the bridge cleans exactly as the node does: what the
//! bridge sends, the node then cleans to the same text, so a place the bridge cut never moves into what the node keeps (review of
//! #1103, round 3).
//!
//! **Every cut of an agent's text goes through [`cap_end`] or [`cap_middle`]**, which say where they cut: a secret value cut there
//! leaves a piece on each side, which the node's transcript writer looks for (`agora_node::transcript`). A test reads the node's chat
//! code and the bridge for any other cut.

/// The longest secret value the node looks for in what it writes (a value is at most 8 KiB, ADR-0084), with room for JSON's escapes:
/// how much of an agent's earlier words the node keeps beside new ones.
pub const MAX_SECRET_SPAN: usize = 16 * 1024;

/// `text` as is when it fits `max` bytes; otherwise its first bytes (cut on a character boundary) and where it now ends.
pub fn cap_end(mut text: String, max: usize) -> (String, Option<usize>) {
    if text.len() <= max {
        return (text, None);
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    (text, Some(end))
}

/// `text` as is when it fits `max` bytes; otherwise its first and its last bytes (cut on character boundaries, `max` in all), with
/// where the head ends in what is kept (the place a value may be cut in two) and where the kept tail began in `text`.
pub fn cap_middle(text: String, max: usize) -> (String, Option<(usize, usize)>) {
    if text.len() <= max {
        return (text, None);
    }
    let mut head = max / 2;
    while !text.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = text.len() - (max - head);
    while !text.is_char_boundary(tail) {
        tail += 1;
    }
    (
        format!("{}{}", &text[..head], &text[tail..]),
        Some((head, tail)),
    )
}

/// How a text is shown, which decides what is kept of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Prose over several lines (thinking, tool output): line breaks and tabs stay; the direction marks of Arabic and Hebrew text
    /// (LRM, RLM, ALM) stay, and so do isolates (U+2066 to U+2069) when each one is closed.
    Prose,
    /// Prose on one line (a plan step): as `Prose`, with line breaks as spaces.
    Step,
    /// Code on several lines (a tool's arguments): line breaks stay, every direction control goes.
    Code,
    /// A name, a path or a label on one line: every direction control goes, since there they only serve to make one thing read as
    /// another (`gpj.exe` written as `exe.jpg`).
    Label,
}

impl Style {
    pub fn multiline(self) -> bool {
        matches!(self, Self::Prose | Self::Code)
    }
    pub fn keeps_marks(self) -> bool {
        matches!(self, Self::Prose | Self::Step)
    }
}

/// The longest payload an unterminated CSI, OSC, DCS, ... sequence may swallow: to the next line break or this many characters.
pub const ESCAPE_PAYLOAD_MAX: usize = 4096;

/// Removes what would act on a terminal or a reader instead of being shown: escape sequences (CSI, OSC, DCS, ...) with their payload
/// (a sequence that is never closed swallows at most to the next line break or `ESCAPE_PAYLOAD_MAX` characters, never the rest of the
/// text), every control character (C0, DEL, C1) and, always, the characters that override or embed a direction (U+202A to U+202E:
/// the ones that spoof). Other direction marks are kept or removed by the [`Style`]. A bare carriage return is a line break in
/// multi-line text and a space otherwise; `\r\n` is one line break.
pub fn strip_controls(s: &str, style: Style) -> String {
    balanced(strip_core(s, style), style)
}

/// [`strip_controls`] without the isolates' balance (done once over a whole text that was stripped in parts).
pub fn strip_core(s: &str, style: Style) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' | '\u{9b}' | '\u{9d}' | '\u{90}' | '\u{98}' | '\u{9e}' | '\u{9f}' => {
                let kind = match c {
                    // A line break right after an ESC is left for the text.
                    '\u{1b}' => chars.next_if(|&c| !line_break(c)),
                    '\u{9b}' => Some('['),
                    '\u{9d}' => Some(']'),
                    _ => Some('P'),
                };
                match kind {
                    Some('[') => {
                        let mut taken = 0;
                        while let Some(&c) = chars.peek() {
                            // A line break ends a sequence that was never closed (it is left for the text, #1113).
                            if line_break(c) || taken >= ESCAPE_PAYLOAD_MAX {
                                break;
                            }
                            chars.next();
                            taken += 1;
                            // The final byte ends a CSI sequence.
                            if ('@'..='~').contains(&c) {
                                break;
                            }
                        }
                    }
                    Some(']' | 'P' | '_' | '^' | 'X') => {
                        let mut taken = 0;
                        while let Some(&c) = chars.peek() {
                            // A line break ends a sequence that was never closed (it is left for the text).
                            if line_break(c) || taken >= ESCAPE_PAYLOAD_MAX {
                                break;
                            }
                            chars.next();
                            taken += 1;
                            if c == '\u{7}' || c == '\u{9c}' {
                                break;
                            }
                            if c == '\u{1b}' {
                                if chars.peek() == Some(&'\\') {
                                    chars.next();
                                }
                                break;
                            }
                        }
                    }
                    _ => {}
                }
            }
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    continue;
                }
                out.push(if style.multiline() { '\n' } else { ' ' });
            }
            '\n' | '\t' if style.multiline() => out.push(c),
            '\n' | '\t' => out.push(' '),
            c if c.is_control() => {}
            // Override and embedding controls: the spoofing risk, never kept.
            '\u{202a}'..='\u{202e}' => {}
            // LRM, RLM, ALM and the isolates: Arabic and Hebrew need them in prose.
            '\u{200e}' | '\u{200f}' | '\u{61c}' | '\u{2066}'..='\u{2069}'
                if !style.keeps_marks() => {}
            c => out.push(c),
        }
    }
    out
}

/// `\n` or `\r` (a bare carriage return is a line break too): where an escape sequence that was never closed ends.
fn line_break(c: char) -> bool {
    c == '\n' || c == '\r'
}

pub fn balanced(out: String, style: Style) -> String {
    if style.keeps_marks() && out.contains(['\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}']) {
        return balance_isolates(&out);
    }
    out
}

/// Keeps an isolate (FSI, LRI, RLI) only with the PDI that closes it, and a PDI only after its opener: an unclosed one would reorder
/// everything that follows it in the page.
pub fn balance_isolates(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut keep = vec![true; chars.len()];
    let mut open: Vec<usize> = Vec::new();
    for (i, c) in chars.iter().enumerate() {
        match c {
            '\u{2066}'..='\u{2068}' => open.push(i),
            '\u{2069}' if open.pop().is_none() => keep[i] = false,
            _ => {}
        }
    }
    for i in open {
        keep[i] = false;
    }
    chars
        .into_iter()
        .zip(keep)
        .filter_map(|(c, k)| k.then_some(c))
        .collect()
}

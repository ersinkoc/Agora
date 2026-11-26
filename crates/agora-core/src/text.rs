//! Text an agent wrote, as a person is shown it (ADR-0057). Characters that change how text looks without being seen (format
//! characters, tags, variation selectors, invisible fillers) can make an approval show something other than what is signed and run:
//! the node refuses them in a start request, and clients show any that reach them as visible escapes.

/// Code point ranges (inclusive) of characters that are invisible or change how text around them looks: Default_Ignorable_Code_Point
/// and every format character (Cf), the line and paragraph separators, and the private use areas. Pinned to `hidden-characters.txt`,
/// which the web client is tested against too.
pub const HIDDEN: &[(u32, u32)] = &[
    (0x00AD, 0x00AD),
    (0x034F, 0x034F),
    (0x0600, 0x0605),
    (0x061C, 0x061C),
    (0x06DD, 0x06DD),
    (0x070F, 0x070F),
    (0x0890, 0x0891),
    (0x08E2, 0x08E2),
    (0x115F, 0x1160),
    (0x17B4, 0x17B5),
    (0x180B, 0x180F),
    (0x200B, 0x200F),
    (0x2028, 0x202E),
    (0x2060, 0x206F),
    (0x2800, 0x2800),
    (0x3164, 0x3164),
    (0xFE00, 0xFE0F),
    (0xFEFF, 0xFEFF),
    (0xFFA0, 0xFFA0),
    (0xFFF0, 0xFFFB),
    (0x110BD, 0x110BD),
    (0x110CD, 0x110CD),
    (0x13430, 0x1343F),
    (0x1BCA0, 0x1BCA3),
    (0x1D173, 0x1D17A),
    (0xE0000, 0xE0FFF),
    (0xE000, 0xF8FF),
    (0xF0000, 0xFFFFD),
    (0x100000, 0x10FFFD),
];

/// A character in [`HIDDEN`].
pub fn is_hidden(c: char) -> bool {
    let c = u32::from(c);
    HIDDEN.iter().any(|(lo, hi)| (*lo..=*hi).contains(&c))
}

/// `text` with every hidden character and every control character other than a line break or a tab written as a visible `\u{…}`
/// escape, so what is shown is what was sent.
pub fn visible(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if is_hidden(c) || (c.is_control() && c != '\n' && c != '\t') {
            out.push_str(&format!("\\u{{{:x}}}", u32::from(c)));
        } else {
            out.push(c);
        }
    }
    out
}

/// The text of a file that is UTF-16 with a byte-order mark (little-endian `FF FE` or big-endian `FE FF`; Windows PowerShell 5.1's
/// `>` writes the former), without the mark, for showing it (#940). `None` for anything else: no mark, an odd length, a lone
/// surrogate, or a control character other than a tab, a line break, a carriage return or a form feed (a NUL among them), so
/// binary data that happens to start with a mark is still not text. The bytes themselves are never changed or written back.
pub fn utf16_text(data: &[u8]) -> Option<String> {
    let little = match data.get(..2)? {
        [0xff, 0xfe] => true,
        [0xfe, 0xff] => false,
        _ => return None,
    };
    let (pairs, odd) = data[2..].as_chunks::<2>();
    if !odd.is_empty() {
        return None;
    }
    let units = pairs.iter().map(|&p| {
        if little {
            u16::from_le_bytes(p)
        } else {
            u16::from_be_bytes(p)
        }
    });
    let mut out = String::with_capacity(data.len());
    for c in char::decode_utf16(units) {
        let c = c.ok()?;
        if c.is_control() && !matches!(c, '\t' | '\n' | '\r' | '\x0c') {
            return None;
        }
        out.push(c);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_with_a_byte_order_mark_is_text_and_nothing_else_is() {
        let enc = |text: &str, little: bool| {
            let mut out = if little {
                vec![0xff, 0xfe]
            } else {
                vec![0xfe, 0xff]
            };
            for u in text.encode_utf16() {
                out.extend(if little {
                    u.to_le_bytes()
                } else {
                    u.to_be_bytes()
                });
            }
            out
        };
        assert_eq!(
            utf16_text(&enc("hello\r\n", true)).as_deref(),
            Some("hello\r\n")
        );
        assert_eq!(
            utf16_text(&enc("h\u{e9}llo \u{1f600}\tx\x0c", false)).as_deref(),
            Some("h\u{e9}llo \u{1f600}\tx\x0c")
        );
        assert_eq!(utf16_text(&[0xff, 0xfe]).as_deref(), Some(""));
        // No mark, an odd length, a lone surrogate, a NUL, another control character: not text.
        assert_eq!(utf16_text(b"h\0i\0"), None);
        assert_eq!(utf16_text(&[0xff, 0xfe, b'h', 0, b'i']), None);
        assert_eq!(utf16_text(&[0xff, 0xfe, 0x00, 0xd8, b'h', 0]), None);
        assert_eq!(utf16_text(&enc("a\0b", true)), None);
        assert_eq!(utf16_text(&enc("a\u{1}b", true)), None);
        assert_eq!(utf16_text(&enc("a\u{85}b", false)), None);
        assert_eq!(utf16_text(&[]), None);
    }

    #[test]
    fn the_list_is_the_shared_file() {
        let file: Vec<(u32, u32)> = include_str!("hidden-characters.txt")
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| {
                let (lo, hi) = l.split_once('-').unwrap_or((l, l));
                (
                    u32::from_str_radix(lo, 16).unwrap_or(0),
                    u32::from_str_radix(hi, 16).unwrap_or(0),
                )
            })
            .collect();
        assert_eq!(file, HIDDEN);
    }

    #[test]
    fn hidden_characters_are_named_and_shown() {
        for c in [
            '\u{ad}',
            '\u{61c}',
            '\u{180e}',
            '\u{200b}',
            '\u{200f}',
            '\u{202e}',
            '\u{2060}',
            '\u{2064}',
            '\u{2066}',
            '\u{2069}',
            '\u{feff}',
            '\u{e0041}',
            '\u{fe0f}',
            '\u{115f}',
            '\u{1160}',
            '\u{2800}',
            '\u{3164}',
            '\u{ffa0}',
        ] {
            assert!(is_hidden(c), "{:x}", u32::from(c));
        }
        for c in ['a', 'ü', 'ş', '🚀', ' ', '\u{a0}'] {
            assert!(!is_hidden(c), "{:x}", u32::from(c));
        }
        assert_eq!(visible("a\u{200b}b\u{1b}c\nd"), "a\\u{200b}b\\u{1b}c\nd");
    }
}

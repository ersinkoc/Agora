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

#[cfg(test)]
mod tests {
    use super::*;

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

//! Text an agent wrote, as a person is shown it (ADR-0057). Characters that change how text looks without being seen (format
//! characters, tags, variation selectors, invisible fillers) can make an approval show something other than what is signed and run:
//! the node refuses them in a start request, and clients show any that reach them as visible escapes.

/// A character that is invisible or changes how the text around it is displayed: every Unicode format character (general category
/// Cf, as of Unicode 15.1), the tag block, variation selectors and the Hangul and halfwidth fillers.
pub fn is_hidden(c: char) -> bool {
    matches!(
        c,
        '\u{00ad}'
            | '\u{0600}'..='\u{0605}'
            | '\u{061c}'
            | '\u{06dd}'
            | '\u{070f}'
            | '\u{0890}'..='\u{0891}'
            | '\u{08e2}'
            | '\u{115f}'..='\u{1160}'
            | '\u{180e}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206f}'
            | '\u{3164}'
            | '\u{fe00}'..='\u{fe0f}'
            | '\u{feff}'
            | '\u{ffa0}'
            | '\u{fff9}'..='\u{fffb}'
            | '\u{110bd}'
            | '\u{110cd}'
            | '\u{13430}'..='\u{1343f}'
            | '\u{1bca0}'..='\u{1bca3}'
            | '\u{1d173}'..='\u{1d17a}'
            | '\u{e0000}'..='\u{e007f}'
            | '\u{e0100}'..='\u{e01ef}'
    )
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

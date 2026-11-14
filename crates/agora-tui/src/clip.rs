//! Copying text to the user's clipboard through the terminal (OSC 52): works over SSH and in most terminals, with nothing to
//! install. A terminal that does not support it ignores the sequence.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// The bytes that put `text` on the clipboard.
pub fn osc52(text: &str) -> Vec<u8> {
    let mut out = b"\x1b]52;c;".to_vec();
    out.extend(base64(text.as_bytes()));
    out.push(0x07);
    out
}

fn base64(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize]);
            } else {
                out.push(b'=');
            }
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard_alphabet_and_padding() {
        let enc = |s: &str| String::from_utf8(base64(s.as_bytes())).unwrap();
        assert_eq!(enc(""), "");
        assert_eq!(enc("f"), "Zg==");
        assert_eq!(enc("fo"), "Zm8=");
        assert_eq!(enc("foo"), "Zm9v");
        assert_eq!(enc("foobar"), "Zm9vYmFy");
        assert_eq!(enc("ağ"), "YcSf");
    }

    #[test]
    fn the_sequence_is_osc_52_ended_by_bel() {
        assert_eq!(osc52("hi"), b"\x1b]52;c;aGk=\x07");
    }
}

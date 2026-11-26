//! Colours of a theme as numbers: reading `#rgb` / `#rrggbb`, WCAG 2.2 contrast, and the two mixes the web window computes
//! (`color-mix(in oklab)` for the kit's soft backgrounds, `apps/web/src/themes/tints.ts`, and a translucent colour over another).
//! The same arithmetic as the web, so a theme file is judged the same in both clients.

/// A colour as red, green and blue bytes.
pub type Rgb = [u8; 3];

/// A colour value of a theme file: `#rgb` or `#rrggbb` (ASCII hex, either case, surrounding blanks allowed), or `None`. Named
/// colours, `rgb()` and transparency are not colours here: contrast must be computable exactly.
pub fn parse_hex(v: &str) -> Option<Rgb> {
    let h = v.trim().strip_prefix('#')?;
    if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    // Every byte is an ASCII hex digit, so slicing by byte is slicing by character.
    let digit = |i: usize| u8::from_str_radix(&h[i..=i], 16).ok();
    match h.len() {
        3 => Some([digit(0)? * 17, digit(1)? * 17, digit(2)? * 17]),
        6 => Some([
            u8::from_str_radix(&h[0..2], 16).ok()?,
            u8::from_str_radix(&h[2..4], 16).ok()?,
            u8::from_str_radix(&h[4..6], 16).ok()?,
        ]),
        _ => None,
    }
}

/// `#rrggbb`, lowercase.
pub fn hex(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn linear(v: u8) -> f64 {
    let c = f64::from(v) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(c: Rgb) -> f64 {
    0.2126 * linear(c[0]) + 0.7152 * linear(c[1]) + 0.0722 * linear(c[2])
}

/// The WCAG 2.2 contrast ratio of two colours, 1 to 21.
pub fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// A ratio as a person reads it: rounded down to one decimal, so a shown 4.5 always passes.
pub fn shown_ratio(r: f64) -> String {
    format!("{:.1}", (r * 10.0).floor() / 10.0)
}

fn to_lab(c: Rgb) -> [f64; 3] {
    let (r, g, b) = (linear(c[0]), linear(c[1]), linear(c[2]));
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    [
        0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s,
    ]
}

fn from_lab([ll, a, b]: [f64; 3]) -> Rgb {
    let l = (ll + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
    let m = (ll - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
    let s = (ll - 0.089_484_177_5 * a - 1.291_485_548 * b).powi(3);
    let rgb = [
        4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s,
        -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s,
        -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701 * s,
    ];
    rgb.map(|c| {
        let d = if c <= 0.003_130_8 {
            12.92 * c
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        };
        // Clamped to 0..=1 first, so the rounded value is 0..=255.
        (d.clamp(0.0, 1.0) * 255.0).round() as u8
    })
}

/// `color-mix(in oklab, a p, b)` for `p` in 0..=1.
pub fn mix(a: Rgb, p: f64, b: Rgb) -> Rgb {
    let (x, y) = (to_lab(a), to_lab(b));
    from_lab([
        x[0] * p + y[0] * (1.0 - p),
        x[1] * p + y[1] * (1.0 - p),
        x[2] * p + y[2] * (1.0 - p),
    ])
}

/// `fg` at `alpha` over `bg`, as a browser blends a translucent colour (sRGB).
pub fn over(fg: Rgb, alpha: f64, bg: Rgb) -> Rgb {
    let ch = |i: usize| (f64::from(fg[i]) * alpha + f64::from(bg[i]) * (1.0 - alpha)).round() as u8;
    [ch(0), ch(1), ch(2)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_short_and_long_hex_and_nothing_else() {
        assert_eq!(parse_hex("#FfF"), Some([255, 255, 255]));
        assert_eq!(parse_hex(" #0a0B0c "), Some([10, 11, 12]));
        for bad in ["fff", "#ffff", "#gggggg", "red", "#ff00001", "#ééé", "rgb(0,0,0)", ""] {
            assert_eq!(parse_hex(bad), None, "{bad}");
        }
        assert_eq!(hex([10, 11, 12]), "#0a0b0c");
    }

    #[test]
    fn contrast_is_wcag() {
        assert!((contrast([0, 0, 0], [255, 255, 255]) - 21.0).abs() < 1e-9);
        assert!((contrast([1, 2, 3], [1, 2, 3]) - 1.0).abs() < 1e-9);
        assert_eq!(shown_ratio(4.4999), "4.4");
        assert_eq!(shown_ratio(4.5), "4.5");
    }

    #[test]
    fn mixes_as_the_browser_does() {
        // Mixing a colour with itself is that colour; the ends of the mix are the two colours.
        assert_eq!(mix([139, 147, 255], 0.14, [139, 147, 255]), [139, 147, 255]);
        assert_eq!(mix([139, 147, 255], 1.0, [17, 17, 20]), [139, 147, 255]);
        assert_eq!(mix([139, 147, 255], 0.0, [17, 17, 20]), [17, 17, 20]);
        assert_eq!(over([255, 255, 255], 0.4, [0, 0, 0]), [102, 102, 102]);
    }
}

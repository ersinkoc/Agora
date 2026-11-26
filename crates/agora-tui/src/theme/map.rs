//! A theme mapped to the user's terminal (ADR-0113 decision 4): its colours as 24-bit colour, or the nearest of the 256-colour
//! palette or of the 16 colours, and the chrome this client can draw. Computed once when a theme is chosen (or the file changes),
//! never per cell or per frame: drawing only reads the table.
//!
//! Colours are mapped in pairs, not one by one: with 256 or 16 colours the nearest entry of a text colour can land on (or near) the
//! entry of its background, so each text colour is the nearest entry that still reads on its mapped background (4.5:1, by xterm's
//! default values of the palette; black or white always do), and a line the nearest one that can still be seen.

use ratatui::style::{Color, Modifier, Style};

use super::colour::{contrast, over, Rgb};
use super::file::{region_colour, terminal_colour, SELECTION_ALPHA};
use super::{
    Look, Scheme, Theme, ACCENT, BORDER, C_DENSITY, C_LINE_WIDTH, C_PANE_BORDERS, C_PANE_GAP,
    ON_ACCENT, PART_LINE, PART_SURFACE, PART_TEXT, SIDEBAR, STATUS_LINE, SURFACE_0, SURFACE_1,
    TAB_BAR, TEXT, TEXT_MUTED, T_ANSI, T_BACKGROUND, T_FOREGROUND, T_SELECTION,
};
use crate::pane::{nearest16, nearest256, xterm_rgb, ColorMode};

/// Text and the secondary text drawn on a background.
const TEXT_MIN: f64 = 4.5;
/// A line between panes: decoration, but it must still be seen (the web's own lines are about 1.2:1 to 1.3:1 on the background).
const LINE_MIN: f64 = 1.2;

/// How pane borders are drawn (`pane-borders`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Borders {
    /// A line between panes that touch (the dividers); with a gap, the gap alone.
    #[default]
    Auto,
    /// The dividers, and a line under the tab bar and above the footer.
    Always,
    Off,
}

/// The chrome the app shell draws. The default is the shell as it is without a theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShellChrome {
    pub borders: Borders,
    /// 0 (no line), 1 (`│`) or 2 (`┃`).
    pub line_width: u8,
    /// A gap between panes: the dividers are blank (`pane-gap` above 0).
    pub gap: bool,
    /// One column of room inside the agents list and the inspector (`density: roomy`).
    pub roomy: bool,
}

impl Default for ShellChrome {
    fn default() -> Self {
        ShellChrome {
            borders: Borders::Auto,
            line_width: 1,
            gap: false,
            roomy: false,
        }
    }
}

impl ShellChrome {
    pub fn of(theme: &Theme) -> Self {
        let c = &theme.chrome;
        ShellChrome {
            borders: match c.word(C_PANE_BORDERS) {
                Some("always") => Borders::Always,
                Some("off") => Borders::Off,
                _ => Borders::Auto,
            },
            line_width: c.0[C_LINE_WIDTH].min(2),
            gap: c.0[C_PANE_GAP] > 0,
            roomy: c.word(C_DENSITY) == Some("roomy"),
        }
    }

    /// The glyph of a vertical divider between panes; blank when there is no line.
    pub fn divider(&self) -> &'static str {
        match (self.borders, self.line_width, self.gap) {
            (Borders::Off, _, _) | (_, 0, _) | (Borders::Auto, _, true) => " ",
            (_, 2, _) => "\u{2503}",
            _ => "\u{2502}",
        }
    }

    /// The glyph of the lines under the tab bar and above the footer ([`Borders::Always`]).
    pub fn rule(&self) -> &'static str {
        match self.line_width {
            0 => " ",
            2 => "\u{2501}",
            _ => "\u{2500}",
        }
    }
}

/// A part of the window: its text on its background, and its secondary text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Part {
    pub fill: Style,
    pub dim: Style,
}

/// How a terminal pane draws the colours a program leaves to the terminal (default text and background, the 16 colours) and a
/// selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PanePalette {
    pub fg: Color,
    pub bg: Color,
    pub ansi: [Color; 16],
    /// The selection's background; `None` where it would not stand out from the background (the selection is inverted then).
    pub selection: Option<Color>,
}

/// What the app shell draws with under a theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Styles {
    /// The window: the terminal pane's area, the welcome, the palette and a long notice.
    pub base: Part,
    /// The agents list (`sidebar`).
    pub tree: Part,
    /// The tab row (`tab-bar`).
    pub tabs: Part,
    /// The inspector (the panels' colours).
    pub inspector: Part,
    /// The footer (`status-line`).
    pub footer: Part,
    /// The selected row and the tab in front: the accent and the text on it.
    pub chosen: Style,
    /// Dividers and rules.
    pub line: Style,
    pub pane: PanePalette,
}

/// A theme as the app shell draws it: the styles and the chrome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shell {
    pub styles: Styles,
    pub chrome: ShellChrome,
}

impl Shell {
    /// `theme`'s colours for `scheme` in `mode`; `None` without colour (`NO_COLOR`: the shell stays as it is) or when the theme has
    /// no colours for that scheme.
    pub fn new(theme: &Theme, scheme: Scheme, mode: ColorMode) -> Option<Shell> {
        if mode == ColorMode::Mono {
            return None;
        }
        let look = theme.look(scheme)?;
        Some(Shell {
            styles: styles(look, scheme, mode),
            chrome: ShellChrome::of(theme),
        })
    }
}

/// The palette entries a mode can use, as xterm shows them: the cube and the grey ramp with 256 colours (the first 16 are the
/// user's terminal's own and may look like anything), the 16 classic colours otherwise.
fn entries(mode: ColorMode) -> std::ops::RangeInclusive<u8> {
    match mode {
        ColorMode::Ansi16 => 0..=15,
        _ => 16..=255,
    }
}

fn distance(a: Rgb, b: Rgb) -> i32 {
    let d = |i: usize| i32::from(a[i]) - i32::from(b[i]);
    d(0) * d(0) + d(1) * d(1) + d(2) * d(2)
}

/// A background colour in `mode`, and the colour it is shown as.
pub fn background(c: Rgb, mode: ColorMode) -> (Color, Rgb) {
    match mode {
        ColorMode::Ansi256 => {
            let i = nearest256(c[0], c[1], c[2]);
            (Color::Indexed(i), rgb_of(xterm_rgb(i)))
        }
        ColorMode::Ansi16 => {
            let i = nearest16(c[0], c[1], c[2]);
            (Color::Indexed(i), rgb_of(xterm_rgb(i)))
        }
        ColorMode::Full | ColorMode::Mono => (Color::Rgb(c[0], c[1], c[2]), c),
    }
}

fn rgb_of((r, g, b): (u8, u8, u8)) -> Rgb {
    [r, g, b]
}

/// A colour drawn on `on` (as shown) in `mode`: itself with 24-bit colour; otherwise the nearest palette entry with at least `min`
/// contrast on it, or the entry with the most contrast when none has (deterministic: ties go to the lower index).
pub fn foreground(c: Rgb, on: Rgb, min: f64, mode: ColorMode) -> Color {
    if matches!(mode, ColorMode::Full | ColorMode::Mono) {
        return Color::Rgb(c[0], c[1], c[2]);
    }
    let mut best: Option<(i32, u8)> = None;
    let mut strongest: Option<(f64, u8)> = None;
    for i in entries(mode) {
        let shown = rgb_of(xterm_rgb(i));
        let ratio = contrast(shown, on);
        if strongest.is_none_or(|(r, _)| ratio > r) {
            strongest = Some((ratio, i));
        }
        let d = distance(shown, c);
        if ratio >= min && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, i));
        }
    }
    Color::Indexed(
        best.map(|(_, i)| i)
            .or(strongest.map(|(_, i)| i))
            .unwrap_or(0),
    )
}

fn part(look: &Look, surface: Rgb, text: Rgb, mode: ColorMode) -> Part {
    let (bg, shown) = background(surface, mode);
    Part {
        fill: Style::default()
            .bg(bg)
            .fg(foreground(text, shown, TEXT_MIN, mode)),
        dim: Style::default().fg(foreground(look.colours[TEXT_MUTED], shown, TEXT_MIN, mode)),
    }
}

fn region(look: &Look, scheme: Scheme, r: usize, mode: ColorMode) -> Part {
    part(
        look,
        region_colour(look, scheme, r, PART_SURFACE),
        region_colour(look, scheme, r, PART_TEXT),
        mode,
    )
}

/// The style table of one scheme of a theme in `mode`.
pub fn styles(look: &Look, scheme: Scheme, mode: ColorMode) -> Styles {
    let c = &look.colours;
    let base = part(look, c[SURFACE_0], c[TEXT], mode);
    let (accent, accent_shown) = background(c[ACCENT], mode);
    let (_, base_shown) = background(c[SURFACE_0], mode);
    let line_colour = look.regions[SIDEBAR][PART_LINE].unwrap_or(c[BORDER]);
    Styles {
        base,
        tree: region(look, scheme, SIDEBAR, mode),
        tabs: region(look, scheme, TAB_BAR, mode),
        inspector: part(look, c[SURFACE_1], c[TEXT], mode),
        footer: region(look, scheme, STATUS_LINE, mode),
        chosen: Style::default()
            .bg(accent)
            .fg(foreground(c[ON_ACCENT], accent_shown, TEXT_MIN, mode)),
        line: Style::default()
            .bg(base.fill.bg.unwrap_or(Color::Reset))
            .fg(foreground(line_colour, base_shown, LINE_MIN, mode)),
        pane: pane_palette(look, scheme, mode),
    }
}

/// The terminal pane's colours: the theme's terminal palette, or the pane's background, the theme's text and the scheme's palette.
pub fn pane_palette(look: &Look, scheme: Scheme, mode: ColorMode) -> PanePalette {
    let bg_rgb = terminal_colour(look, scheme, T_BACKGROUND);
    let (bg, shown) = background(bg_rgb, mode);
    let fg = foreground(terminal_colour(look, scheme, T_FOREGROUND), shown, TEXT_MIN, mode);
    let ansi = std::array::from_fn(|i| {
        let c = terminal_colour(look, scheme, T_ANSI + i);
        match mode {
            // With 16 colours the terminal's own red is the best red there is.
            ColorMode::Ansi16 => Color::Indexed(u8::try_from(i).unwrap_or(0)),
            ColorMode::Ansi256 => Color::Indexed(nearest256(c[0], c[1], c[2])),
            ColorMode::Full | ColorMode::Mono => Color::Rgb(c[0], c[1], c[2]),
        }
    });
    let sel_rgb = over(terminal_colour(look, scheme, T_SELECTION), SELECTION_ALPHA, bg_rgb);
    let (sel, _) = background(sel_rgb, mode);
    PanePalette {
        fg,
        bg,
        ansi,
        selection: (sel != bg).then_some(sel),
    }
}

/// The style a pane gives its selected cells under a palette: the selection's background, the cell's own text (an inverted cell's
/// text is its background), never inverted; or inverted, as without a theme, where the selection would not show.
pub fn selected(style: Style, palette: &PanePalette) -> Style {
    let Some(sel) = palette.selection else {
        return if style.add_modifier.contains(Modifier::REVERSED) {
            style.remove_modifier(Modifier::REVERSED)
        } else {
            style.add_modifier(Modifier::REVERSED)
        };
    };
    let fg = if style.add_modifier.contains(Modifier::REVERSED) {
        style.bg
    } else {
        style.fg
    };
    Style {
        fg,
        bg: Some(sel),
        ..style
    }
    .remove_modifier(Modifier::REVERSED)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::theme::{built_ins, file::read_text};

    fn shown(c: Color, mode: ColorMode) -> Rgb {
        match (c, mode) {
            (Color::Rgb(r, g, b), _) => [r, g, b],
            (Color::Indexed(i), _) => rgb_of(xterm_rgb(i)),
            _ => panic!("{c:?}"),
        }
    }

    /// Every pair the shell draws, as the terminal shows it.
    fn pairs(s: &Styles, mode: ColorMode) -> Vec<(&'static str, Rgb, Rgb, f64)> {
        let mut out = Vec::new();
        for (name, p) in [
            ("base", s.base),
            ("tree", s.tree),
            ("tabs", s.tabs),
            ("inspector", s.inspector),
            ("footer", s.footer),
        ] {
            let bg = shown(p.fill.bg.unwrap(), mode);
            out.push((name, shown(p.fill.fg.unwrap(), mode), bg, TEXT_MIN));
            out.push((name, shown(p.dim.fg.unwrap(), mode), bg, TEXT_MIN));
        }
        out.push(("chosen", shown(s.chosen.fg.unwrap(), mode), shown(s.chosen.bg.unwrap(), mode), TEXT_MIN));
        out.push(("line", shown(s.line.fg.unwrap(), mode), shown(s.line.bg.unwrap(), mode), LINE_MIN));
        out.push(("pane", shown(s.pane.fg, mode), shown(s.pane.bg, mode), TEXT_MIN));
        out
    }

    /// ADR-0113 verification: a truecolor theme on a 16-colour (or 256-colour) terminal maps every colour to a readable one.
    #[test]
    fn every_drawn_pair_stays_readable_in_every_mode() {
        let harbour = read_text(include_str!("testdata/harbour.json"));
        let mut themes: Vec<(String, Theme)> =
            built_ins().into_iter().map(|b| (b.id.to_string(), b.theme)).collect();
        themes.push(("harbour".into(), harbour.theme().unwrap().clone()));
        for (id, theme) in &themes {
            for scheme in theme.schemes() {
                for mode in [ColorMode::Full, ColorMode::Ansi256, ColorMode::Ansi16] {
                    let shell = Shell::new(theme, scheme, mode).unwrap();
                    for (what, fg, bg, min) in pairs(&shell.styles, mode) {
                        let r = contrast(fg, bg);
                        assert!(r >= min, "{id} {scheme:?} {mode:?} {what}: {r:.2}");
                    }
                }
            }
        }
    }

    #[test]
    fn mapping_is_deterministic_and_follows_the_mode() {
        let slate = built_ins().into_iter().find(|b| b.id == "slate").unwrap().theme;
        let full = Shell::new(&slate, Scheme::Dark, ColorMode::Full).unwrap();
        assert_eq!(full.styles.base.fill.bg, Some(Color::Rgb(0x0f, 0x14, 0x19)));
        assert_eq!(full.styles.base.fill.fg, Some(Color::Rgb(0xe6, 0xed, 0xf3)));
        let a = Shell::new(&slate, Scheme::Dark, ColorMode::Ansi256).unwrap();
        let b = Shell::new(&slate, Scheme::Dark, ColorMode::Ansi256).unwrap();
        assert_eq!(a, b);
        // 256 colours: the cube and the grey ramp only, never the user's own 16.
        assert!(matches!(a.styles.base.fill.bg, Some(Color::Indexed(i)) if i >= 16));
        assert_eq!(a.styles.base.fill.bg, Some(Color::Indexed(nearest256(0x0f, 0x14, 0x19))));
        // 16 colours: a dark surface is black, its text white; the pane keeps the terminal's own 16.
        let c = Shell::new(&slate, Scheme::Dark, ColorMode::Ansi16).unwrap();
        assert_eq!(c.styles.base.fill.bg, Some(Color::Indexed(0)));
        assert!(matches!(c.styles.base.fill.fg, Some(Color::Indexed(7 | 15))));
        assert_eq!(c.styles.pane.ansi[1], Color::Indexed(1));
        // No colour at all: no theme.
        assert_eq!(Shell::new(&slate, Scheme::Dark, ColorMode::Mono), None);
        // A scheme the theme does not have: no theme.
        assert_eq!(Shell::new(&slate, Scheme::Light, ColorMode::Full), None);
    }

    #[test]
    fn a_text_colour_that_would_vanish_is_moved_to_a_readable_entry() {
        // A grey that is fine on black in 24-bit colour, but whose nearest 16-colour entry is the black itself.
        let on = rgb_of(xterm_rgb(0));
        let fg = foreground([0x30, 0x30, 0x30], on, TEXT_MIN, ColorMode::Ansi16);
        assert!(contrast(shown(fg, ColorMode::Ansi16), on) >= TEXT_MIN);
        assert_eq!(fg, foreground([0x30, 0x30, 0x30], on, TEXT_MIN, ColorMode::Ansi16));
        assert_eq!(
            foreground([1, 2, 3], on, TEXT_MIN, ColorMode::Full),
            Color::Rgb(1, 2, 3)
        );
    }

    #[test]
    fn chrome_maps_to_what_the_shell_draws() {
        let read = |chrome: &str| {
            let text = format!(r#"{{"agora-theme": 2, "name": "x", "inherits": "dark", "chrome": {chrome}}}"#);
            ShellChrome::of(read_text(&text).theme().unwrap())
        };
        assert_eq!(read("{}"), ShellChrome::default());
        assert_eq!(ShellChrome::default().divider(), "\u{2502}");
        assert_eq!(read(r#"{"pane-borders": "off"}"#).divider(), " ");
        assert_eq!(read(r#"{"line-width": 0}"#).divider(), " ");
        assert_eq!(read(r#"{"line-width": 2}"#).divider(), "\u{2503}");
        assert_eq!(read(r#"{"pane-gap": 8}"#).divider(), " ");
        let always = read(r#"{"pane-borders": "always", "pane-gap": 8}"#);
        assert_eq!((always.borders, always.divider(), always.rule()), (Borders::Always, "\u{2502}", "\u{2500}"));
        assert!(read(r#"{"density": "roomy"}"#).roomy);
        assert!(!read(r#"{"density": "compact"}"#).roomy);
    }

    #[test]
    fn a_selection_uses_the_theme_unless_it_would_not_show() {
        let p = PanePalette {
            fg: Color::Rgb(1, 1, 1),
            bg: Color::Rgb(0, 0, 0),
            ansi: [Color::Reset; 16],
            selection: Some(Color::Rgb(9, 9, 9)),
        };
        let plain = Style::default().fg(Color::Rgb(5, 5, 5)).bg(Color::Rgb(0, 0, 0));
        assert_eq!(selected(plain, &p).bg, Some(Color::Rgb(9, 9, 9)));
        assert_eq!(selected(plain, &p).fg, Some(Color::Rgb(5, 5, 5)));
        let inverse = plain.add_modifier(Modifier::REVERSED);
        let s = selected(inverse, &p);
        assert_eq!((s.fg, s.add_modifier.contains(Modifier::REVERSED)), (Some(Color::Rgb(0, 0, 0)), false));
        let none = PanePalette { selection: None, ..p };
        assert!(selected(plain, &none).add_modifier.contains(Modifier::REVERSED));
    }
}

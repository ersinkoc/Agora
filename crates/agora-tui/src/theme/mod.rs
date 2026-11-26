//! Themes in the terminal client (ADR-0113 build step 3, ADR-0068 rung 5; `docs/ux/themes.md`, "The terminal client").
//!
//! A theme is data: the same JSON file the web window reads (format 1 or 2), checked by the same rules ([`file`]), then mapped to
//! what the user's terminal can show ([`map`]): 24-bit colour, the 256-colour palette or the 16 colours, and the chrome this client
//! can draw (pane borders, line width, gaps, density). Everything else a theme sets is listed as not used here ([`Unused`]).
//! What needs the person (the "needs you" marks, a request's box) is never drawn in a theme's colours (the floor, ADR-0113
//! decision 3). Nothing in this module but [`folder`] touches the file system, and nothing touches the terminal.

pub mod colour;
pub mod file;
pub mod folder;
pub mod list;
pub mod map;
pub mod pack;
pub mod starter;

pub use colour::Rgb;

/// The two schemes a theme can have colours for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Scheme {
    Dark,
    Light,
}

impl Scheme {
    pub const BOTH: [Scheme; 2] = [Scheme::Dark, Scheme::Light];
    /// The scheme's name in a theme file (`"dark"`, `"light"`).
    pub fn word(self) -> &'static str {
        match self {
            Scheme::Dark => "dark",
            Scheme::Light => "light",
        }
    }
    fn index(self) -> usize {
        match self {
            Scheme::Dark => 0,
            Scheme::Light => 1,
        }
    }
}

// ---- Tokens ------------------------------------------------------------------------------------------------------------------

/// The 13 colour tokens of format 1, in the editor's order. A theme's [`Look::colours`] holds them in this order.
pub const COLOUR_TOKENS: [&str; 13] = [
    "surface-0",
    "surface-1",
    "surface-2",
    "border",
    "text",
    "text-muted",
    "accent",
    "on-accent",
    "state-working",
    "state-needs-you",
    "state-finished",
    "state-idle",
    "state-unknown",
];
pub const SURFACE_0: usize = 0;
pub const SURFACE_1: usize = 1;
pub const SURFACE_2: usize = 2;
pub const BORDER: usize = 3;
pub const TEXT: usize = 4;
pub const TEXT_MUTED: usize = 5;
pub const ACCENT: usize = 6;
pub const ON_ACCENT: usize = 7;
/// The five state colours, from [`COLOUR_TOKENS`] index 8 on.
pub const STATES: [usize; 5] = [8, 9, 10, 11, 12];

/// Sizes and corners (shared by both schemes; `shape` in format 2) and their ranges in px. `text-16` is 12–24 in format 1 and at
/// least 16 in format 2 (the floor: no text under 12 px).
pub const SHAPE_TOKENS: [(&str, f64, f64); 6] = [
    ("text-16", 12.0, 24.0),
    ("space-1", 3.0, 6.0),
    ("radius-xs", 0.0, 16.0),
    ("radius-s", 0.0, 24.0),
    ("radius-m", 0.0, 32.0),
    ("radius-l", 0.0, 40.0),
];
/// The two font lists (`text` in format 2).
pub const FONT_TOKENS: [&str; 2] = ["font-ui", "font-mono"];
/// The smallest base text size format 2 accepts (the smallest text is three quarters of it, and none is under 12 px).
pub const MIN_BASE_TEXT: f64 = 16.0;
/// Tokens of `tokens.css` that follow from another: a file that sets one is told to set that one instead.
pub const DERIVED: [(&str, &str); 10] = [
    ("text-12", "text-16"),
    ("text-13", "text-16"),
    ("text-14", "text-16"),
    ("text-20", "text-16"),
    ("text-28", "text-16"),
    ("space-2", "space-1"),
    ("space-3", "space-1"),
    ("space-4", "space-1"),
    ("space-6", "space-1"),
    ("space-8", "space-1"),
];
pub const MAX_NAME: usize = 60;
const FONT_UI: &str =
    "'Inter Variable', Inter, system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif";
const FONT_MONO: &str = "ui-monospace, 'SF Mono', 'JetBrains Mono', Menlo, Consolas, monospace";
/// The default sizes and corners, in [`SHAPE_TOKENS`] order, as the web writes them.
pub const DEFAULT_SHAPE: [&str; 6] = ["16px", "4px", "5px", "8px", "12px", "18px"];
pub const DEFAULT_FONTS: [&str; 2] = [FONT_UI, FONT_MONO];

// ---- Regions, terminal, syntax ---------------------------------------------------------------------------------------------------

/// The parts of the window a format-2 theme may colour on their own (`apps/web/src/themes/look.ts`).
pub const REGIONS: [&str; 10] = [
    "sidebar",
    "tab-bar",
    "pane-bar",
    "status-line",
    "cards",
    "inputs",
    "buttons",
    "agent-entries",
    "person-entries",
    "code-blocks",
];
pub const SIDEBAR: usize = 0;
pub const TAB_BAR: usize = 1;
pub const STATUS_LINE: usize = 3;
pub const PERSON_ENTRIES: usize = 8;
/// The regions this client draws: the agents list, the tab row, and the footer (the status line).
pub const DRAWN_REGIONS: [usize; 3] = [SIDEBAR, TAB_BAR, STATUS_LINE];
pub const REGION_PARTS: [&str; 3] = ["surface", "text", "line"];
pub const PART_SURFACE: usize = 0;
pub const PART_TEXT: usize = 1;
pub const PART_LINE: usize = 2;

/// What a region part is drawn in when the theme leaves it out: a token, or a colour the web's kit mixes from them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionDefault {
    Token(usize),
    /// The accent mixed into the panels (`--tint-accent`).
    TintAccent,
    /// The accent's hairline (`--line-accent`): decoration, never judged.
    LineAccent,
}

/// [`REGIONS`] × [`REGION_PARTS`]: the web's `REGION_DEFAULT`.
pub const REGION_DEFAULT: [[RegionDefault; 3]; 10] = {
    use RegionDefault::{LineAccent, TintAccent, Token};
    [
        [Token(SURFACE_1), Token(TEXT), Token(BORDER)],
        [Token(SURFACE_0), Token(TEXT), Token(BORDER)],
        [Token(SURFACE_1), Token(TEXT), Token(BORDER)],
        [Token(SURFACE_1), Token(TEXT_MUTED), Token(BORDER)],
        [Token(SURFACE_1), Token(TEXT), Token(BORDER)],
        [Token(SURFACE_1), Token(TEXT), Token(BORDER)],
        [Token(SURFACE_2), Token(TEXT), Token(BORDER)],
        [Token(SURFACE_0), Token(TEXT), Token(BORDER)],
        [TintAccent, Token(TEXT), LineAccent],
        [Token(SURFACE_1), Token(TEXT), Token(BORDER)],
    ]
};

/// The four colours the terminal itself draws, then the 16 programs draw with.
pub const TERMINAL_KEYS: [&str; 20] = [
    "background",
    "foreground",
    "cursor",
    "selection",
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "bright-black",
    "bright-red",
    "bright-green",
    "bright-yellow",
    "bright-blue",
    "bright-magenta",
    "bright-cyan",
    "bright-white",
];
pub const T_BACKGROUND: usize = 0;
pub const T_FOREGROUND: usize = 1;
pub const T_CURSOR: usize = 2;
pub const T_SELECTION: usize = 3;
/// Where the 16 colours start in [`TERMINAL_KEYS`].
pub const T_ANSI: usize = 4;

/// The palette a terminal pane uses when a theme gives none (`look.ts` `ANSI`), per scheme.
pub const ANSI: [[Rgb; 16]; 2] = [
    [
        [0x3b, 0x3b, 0x44],
        [0xf8, 0x71, 0x71],
        [0x4a, 0xde, 0x80],
        [0xfb, 0xbf, 0x24],
        [0x60, 0xa5, 0xfa],
        [0xc0, 0x84, 0xfc],
        [0x22, 0xd3, 0xee],
        [0xd4, 0xd4, 0xd8],
        [0x71, 0x71, 0x7a],
        [0xfc, 0xa5, 0xa5],
        [0x86, 0xef, 0xac],
        [0xfd, 0xe6, 0x8a],
        [0x93, 0xc5, 0xfd],
        [0xd8, 0xb4, 0xfe],
        [0x67, 0xe8, 0xf9],
        [0xfa, 0xfa, 0xfa],
    ],
    [
        [0x27, 0x27, 0x2a],
        [0xb9, 0x1c, 0x1c],
        [0x15, 0x80, 0x3d],
        [0xa1, 0x62, 0x07],
        [0x1d, 0x4e, 0xd8],
        [0x7e, 0x22, 0xce],
        [0x0e, 0x74, 0x90],
        [0x52, 0x52, 0x5b],
        [0x52, 0x52, 0x5b],
        [0xdc, 0x26, 0x26],
        [0x16, 0xa3, 0x4a],
        [0xca, 0x8a, 0x04],
        [0x25, 0x63, 0xeb],
        [0x93, 0x33, 0xea],
        [0x08, 0x91, 0xb2],
        [0x18, 0x18, 0x1b],
    ],
];

/// Code colours (the web's `--syntax-*`): not drawn by this client.
pub const SYNTAX_KEYS: [&str; 8] = [
    "keyword", "string", "comment", "number", "function", "type", "attr", "meta",
];

// ---- Chrome ------------------------------------------------------------------------------------------------------------------

/// The kind of a chrome value: a word from a list (default first in the web's table, here by index), or whole pixels in a range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChromeKind {
    List {
        values: &'static [&'static str],
        default: u8,
    },
    Px {
        min: u8,
        max: u8,
        default: u8,
    },
}

/// The window's frame (`look.ts` `CHROME`), in the web's order.
pub const CHROME: [(&str, ChromeKind); 10] = [
    (
        "pane-borders",
        ChromeKind::List {
            values: &["auto", "always", "off"],
            default: 0,
        },
    ),
    (
        "pane-gap",
        ChromeKind::Px {
            min: 0,
            max: 16,
            default: 0,
        },
    ),
    (
        "line-width",
        ChromeKind::Px {
            min: 0,
            max: 2,
            default: 1,
        },
    ),
    (
        "tab-bar",
        ChromeKind::List {
            values: &["top", "bottom"],
            default: 0,
        },
    ),
    (
        "tab-style",
        ChromeKind::List {
            values: &["underline", "filled", "pill"],
            default: 0,
        },
    ),
    (
        "density",
        ChromeKind::List {
            values: &["compact", "normal", "roomy"],
            default: 1,
        },
    ),
    (
        "state-display",
        ChromeKind::List {
            values: &["symbol", "dot", "word"],
            default: 0,
        },
    ),
    (
        "icons",
        ChromeKind::List {
            values: &["lines"],
            default: 0,
        },
    ),
    (
        "shadow",
        ChromeKind::List {
            values: &["none", "soft", "strong"],
            default: 1,
        },
    ),
    (
        "motion",
        ChromeKind::List {
            values: &["system", "reduced", "off"],
            default: 0,
        },
    ),
];
pub const C_PANE_BORDERS: usize = 0;
pub const C_PANE_GAP: usize = 1;
pub const C_LINE_WIDTH: usize = 2;
pub const C_DENSITY: usize = 5;
/// The chrome values this client draws; the others are listed as not used here.
pub const DRAWN_CHROME: [usize; 4] = [C_PANE_BORDERS, C_PANE_GAP, C_LINE_WIDTH, C_DENSITY];

/// A theme's chrome: for a list value its index in the list, for pixels the number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chrome(pub [u8; 10]);

impl Default for Chrome {
    fn default() -> Self {
        Chrome(CHROME.map(|(_, k)| match k {
            ChromeKind::List { default, .. } | ChromeKind::Px { default, .. } => default,
        }))
    }
}

impl Chrome {
    /// A list value's word, or `None` for a pixel value.
    pub fn word(&self, i: usize) -> Option<&'static str> {
        match CHROME.get(i)?.1 {
            ChromeKind::List { values, .. } => values.get(usize::from(self.0[i])).copied(),
            ChromeKind::Px { .. } => None,
        }
    }
}

/// The interface text (`text` in format 2 besides the fonts): not drawn by this client, whose text is the terminal's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextStyle {
    /// 300, 400 or 500.
    pub weight: u16,
    /// `None` is "normal"; otherwise hundredths (1.2–1.8 as 120–180).
    pub line_height: Option<u16>,
    pub ligatures: bool,
}

pub const WEIGHTS: [u16; 3] = [300, 400, 500];
pub const DEFAULT_TEXT: TextStyle = TextStyle {
    weight: 400,
    line_height: None,
    ligatures: true,
};

// ---- The floor ---------------------------------------------------------------------------------------------------------------

/// What no theme can change (ADR-0113 decision 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FloorItem {
    Danger,
    DiffRemoved,
    ApprovalCard,
    FocusRing,
    NeedsYou,
    PluginMarker,
}

/// The floor, by the name a file would use for it, matched as a whole key (`state-needs-you` and `accent` are themeable tokens).
pub fn floor_of(key: &str) -> Option<FloorItem> {
    Some(match key {
        "danger" | "on-danger" => FloorItem::Danger,
        "diff-removed" => FloorItem::DiffRemoved,
        "approval-card" | "approval" => FloorItem::ApprovalCard,
        "focus-ring" | "focus" => FloorItem::FocusRing,
        "needs-you" | "attention" => FloorItem::NeedsYou,
        "plugin-marker" => FloorItem::PluginMarker,
        _ => return None,
    })
}

/// Whether a value holds a piece that could end a CSS declaration or reach outside the page: refused wherever it is.
pub fn is_unsafe(s: &str) -> bool {
    let lower = s.to_lowercase();
    lower.contains("url(")
        || lower.contains("@import")
        || lower.contains("expression(")
        || s.contains(';')
        || s.contains('}')
        || s.contains('<')
}

// ---- A theme -----------------------------------------------------------------------------------------------------------------

/// One scheme's colours: the 13 tokens, and the region, terminal and syntax colours the theme sets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Look {
    pub colours: [Rgb; 13],
    pub regions: [[Option<Rgb>; 3]; 10],
    pub terminal: [Option<Rgb>; 20],
    pub syntax: [Option<Rgb>; 8],
}

impl Look {
    pub fn of(colours: [Rgb; 13]) -> Self {
        Look {
            colours,
            regions: [[None; 3]; 10],
            terminal: [None; 20],
            syntax: [None; 8],
        }
    }
}

/// A theme as read from a file or built in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Theme {
    /// A file's own name (control and text-direction characters removed); a built-in theme's is its id (shown by its locale key).
    pub name: String,
    /// Dark, then light: the schemes the theme has.
    pub looks: [Option<Look>; 2],
    /// [`SHAPE_TOKENS`] values as `Npx`.
    pub shape: [String; 6],
    pub fonts: [String; 2],
    pub text: TextStyle,
    pub chrome: Chrome,
}

impl Theme {
    pub fn look(&self, s: Scheme) -> Option<&Look> {
        self.looks[s.index()].as_ref()
    }
    pub fn schemes(&self) -> Vec<Scheme> {
        Scheme::BOTH
            .into_iter()
            .filter(|s| self.look(*s).is_some())
            .collect()
    }

    /// What this theme sets that the terminal client does not draw, in the order the theme list names them.
    pub fn unused(&self) -> Vec<Unused> {
        let mut out = Vec::new();
        if self.shape.iter().zip(DEFAULT_SHAPE).any(|(a, b)| a != b) {
            out.push(Unused::Sizes);
        }
        if self.fonts.iter().zip(DEFAULT_FONTS).any(|(a, b)| a != b) || self.text != DEFAULT_TEXT {
            out.push(Unused::Text);
        }
        let looks = || self.looks.iter().flatten();
        for (r, _) in REGIONS.iter().enumerate() {
            if !DRAWN_REGIONS.contains(&r)
                && looks().any(|l| l.regions[r].iter().any(Option::is_some))
            {
                out.push(Unused::Region(r));
            }
        }
        if looks().any(|l| l.terminal[T_CURSOR].is_some()) {
            out.push(Unused::Cursor);
        }
        if looks().any(|l| l.syntax.iter().any(Option::is_some)) {
            out.push(Unused::Syntax);
        }
        let defaults = Chrome::default();
        for (i, _) in CHROME.iter().enumerate() {
            if !DRAWN_CHROME.contains(&i) && self.chrome.0[i] != defaults.0[i] {
                out.push(Unused::Chrome(i));
            }
        }
        out
    }
}

/// A part of a theme the terminal client does not draw (shown in its theme list, ADR-0113 decision 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unused {
    /// Text size, spacing and corners: the terminal's own cells.
    Sizes,
    /// Fonts, weight, line height and ligatures: the terminal's own font.
    Text,
    /// A region this client has no part for ([`REGIONS`] index).
    Region(usize),
    /// The terminal's cursor colour: the cursor is the user's terminal's own.
    Cursor,
    Syntax,
    /// A chrome value ([`CHROME`] index).
    Chrome(usize),
}

// ---- Built-in themes -----------------------------------------------------------------------------------------------------------

const fn rgb(v: u32) -> Rgb {
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

const DARK: [Rgb; 13] = [
    rgb(0x09090b),
    rgb(0x111114),
    rgb(0x1b1b20),
    rgb(0x26262c),
    rgb(0xf4f4f5),
    rgb(0xa1a1aa),
    rgb(0x8b93ff),
    rgb(0x0b0b14),
    rgb(0x60a5fa),
    rgb(0xfbbf24),
    rgb(0x4ade80),
    rgb(0x8b8b96),
    rgb(0x8f8f9b),
];
const LIGHT: [Rgb; 13] = [
    rgb(0xf6f6f8),
    rgb(0xffffff),
    rgb(0xefeff3),
    rgb(0xe1e1e8),
    rgb(0x17171c),
    rgb(0x5b5b68),
    rgb(0x4f46e5),
    rgb(0xffffff),
    rgb(0x175fb5),
    rgb(0xa35a00),
    rgb(0x146c36),
    rgb(0x5b5b68),
    rgb(0x6a6a76),
];

/// The default colours of a scheme (`apps/web/src/design/tokens.css`; a test holds them equal to it).
pub fn base_colours(s: Scheme) -> [Rgb; 13] {
    match s {
        Scheme::Dark => DARK,
        Scheme::Light => LIGHT,
    }
}

/// A built-in theme: its id (`inherits` names it, the locale key is `tui-theme-name-<id>`), its scheme and its theme.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuiltIn {
    pub id: &'static str,
    pub scheme: Scheme,
    pub theme: Theme,
}

const SOFT_CORNERS: [&str; 4] = ["6px", "10px", "14px", "20px"];

fn built_in(id: &'static str, scheme: Scheme, colours: [Rgb; 13], shape: [&str; 6]) -> BuiltIn {
    let mut looks = [None, None];
    looks[scheme.index()] = Some(Look::of(colours));
    BuiltIn {
        id,
        scheme,
        theme: Theme {
            name: id.to_string(),
            looks,
            shape: shape.map(str::to_string),
            fonts: DEFAULT_FONTS.map(str::to_string),
            text: DEFAULT_TEXT,
            chrome: Chrome::default(),
        },
    }
}

/// The six built-in themes (`apps/web/src/themes/tokens.ts` `BUILT_IN`), in the web's order.
pub fn built_ins() -> Vec<BuiltIn> {
    let soft = |text: &'static str| {
        [
            text,
            "4px",
            SOFT_CORNERS[0],
            SOFT_CORNERS[1],
            SOFT_CORNERS[2],
            SOFT_CORNERS[3],
        ]
    };
    vec![
        built_in("dark", Scheme::Dark, DARK, DEFAULT_SHAPE),
        built_in("light", Scheme::Light, LIGHT, DEFAULT_SHAPE),
        // Pure black and white, borders you can see, a yellow accent.
        built_in(
            "high-contrast",
            Scheme::Dark,
            [
                rgb(0x000000),
                rgb(0x0a0a0a),
                rgb(0x1c1c1c),
                rgb(0x9a9a9a),
                rgb(0xffffff),
                rgb(0xe2e2e2),
                rgb(0xffd60a),
                rgb(0x000000),
                rgb(0x7cc4ff),
                rgb(0xffb000),
                rgb(0x4ff08d),
                rgb(0xd0d0d0),
                rgb(0xb4b4b4),
            ],
            DEFAULT_SHAPE,
        ),
        // Dark with text and spacing a quarter larger: here only its colours, since the terminal's cells are its own.
        built_in("large-text", Scheme::Dark, DARK, soft("20px")),
        // A cool blue-grey dark with a teal accent.
        built_in(
            "slate",
            Scheme::Dark,
            [
                rgb(0x0f1419),
                rgb(0x151c24),
                rgb(0x1e2732),
                rgb(0x2a3541),
                rgb(0xe6edf3),
                rgb(0x9fb0c0),
                rgb(0x5cc8c2),
                rgb(0x062320),
                rgb(0x7aa2f7),
                rgb(0xe0af68),
                rgb(0x9ece6a),
                rgb(0x8b98a5),
                rgb(0x8593a1),
            ],
            DEFAULT_SHAPE,
        ),
        // A warm paper-like light with a terracotta accent and softer corners.
        built_in(
            "sand",
            Scheme::Light,
            [
                rgb(0xf5f1e8),
                rgb(0xfbf8f2),
                rgb(0xece5d6),
                rgb(0xddd3c0),
                rgb(0x2b2620),
                rgb(0x5e5446),
                rgb(0x9a4a1f),
                rgb(0xffffff),
                rgb(0x2f6690),
                rgb(0x8a5300),
                rgb(0x3b6e2a),
                rgb(0x5e5446),
                rgb(0x6c6356),
            ],
            soft("16px"),
        ),
    ]
}

/// Every theme and scheme the tests hold to the floor and to a readable mapping: each built-in theme, and each scheme of each starter
/// theme, with its id.
#[cfg(test)]
pub fn every_look() -> Vec<(String, Theme, Scheme)> {
    let mut out: Vec<(String, Theme, Scheme)> = built_ins()
        .into_iter()
        .map(|b| (b.id.to_string(), b.theme, b.scheme))
        .collect();
    for s in starter::starters() {
        for scheme in s.theme.schemes() {
            out.push((format!("starter-{}", s.id), s.theme.clone(), scheme));
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// The web's `tokens.css` is the source of Dark and Light; these values must stay equal to it.
    #[test]
    fn dark_and_light_are_the_web_tokens() {
        let css = include_str!("../../../../apps/web/src/design/tokens.css");
        let block = |start: &str| {
            let at = css.find(start).unwrap();
            let end = at + css[at..].find('}').unwrap();
            css[at..end].to_string()
        };
        for (scheme, start) in [
            (Scheme::Dark, "[data-theme='dark'] {"),
            (Scheme::Light, "[data-theme='light'] {"),
        ] {
            let block = block(start);
            for (i, name) in COLOUR_TOKENS.iter().enumerate() {
                let line = block
                    .lines()
                    .find(|l| l.trim_start().starts_with(&format!("--{name}:")))
                    .unwrap_or_else(|| panic!("{name}"));
                let value = line.split(':').nth(1).unwrap().trim().trim_end_matches(';');
                assert_eq!(
                    colour::parse_hex(value),
                    Some(base_colours(scheme)[i]),
                    "{name} {scheme:?}"
                );
            }
        }
    }

    /// The built-in themes are the web's (`tokens.ts`): every colour value of each appears in its definition there.
    #[test]
    fn built_ins_are_the_web_themes() {
        let ts = include_str!("../../../../apps/web/src/themes/tokens.ts");
        let ids: Vec<&str> = built_ins().iter().map(|b| b.id).collect();
        assert_eq!(
            ids,
            [
                "dark",
                "light",
                "high-contrast",
                "large-text",
                "slate",
                "sand"
            ]
        );
        for b in built_ins() {
            let at = ts.find(&format!("builtIn('{}'", b.id)).unwrap();
            let rest = &ts[at + 1..];
            let end = [rest.find("builtIn('"), rest.find("\n];")]
                .into_iter()
                .flatten()
                .min()
                .unwrap();
            let def = &ts[at..at + 1 + end];
            assert!(def.contains(&format!("'{}'", b.scheme.word())), "{}", b.id);
            let look = b.theme.look(b.scheme).unwrap();
            for (i, name) in COLOUR_TOKENS.iter().enumerate() {
                if def.contains(&format!("'{name}': '")) || def.contains(&format!(" {name}: '")) {
                    assert!(
                        def.contains(&colour::hex(look.colours[i])),
                        "{} {name}",
                        b.id
                    );
                } else {
                    assert_eq!(
                        look.colours[i],
                        base_colours(b.scheme)[i],
                        "{} {name}",
                        b.id
                    );
                }
            }
            for (i, (name, _, _)) in SHAPE_TOKENS.iter().enumerate() {
                if def.contains(&format!("'{name}'")) {
                    assert!(
                        def.contains(&format!("'{name}': '{}'", b.theme.shape[i])),
                        "{} {name}",
                        b.id
                    );
                }
            }
        }
    }

    #[test]
    fn the_floor_is_matched_as_a_whole_name() {
        assert_eq!(floor_of("approval"), Some(FloorItem::ApprovalCard));
        assert_eq!(floor_of("state-needs-you"), None);
        assert_eq!(floor_of("accent"), None);
        assert!(
            is_unsafe("x URL(y)")
                && is_unsafe("a;b")
                && is_unsafe("<")
                && !is_unsafe("Inter, sans-serif")
        );
    }
}

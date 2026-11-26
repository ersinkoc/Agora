//! Reading a theme file, format 1 or 2 (`docs/ux/themes.md`): the one door from untrusted text into a [`Theme`]. It never panics,
//! reports every problem at once, refuses unknown fields at every level, values outside their kind, anything that names what no
//! theme may change (the floor) and any value with a piece that could become CSS. The rules and the readability pairs that refuse
//! a theme are the web window's (`apps/web/src/themes/file.ts`, `tokens.ts`), so a file is accepted or refused alike in both
//! clients. The warnings are the ones about what this client draws (the terminal's 16 colours and its selection); the web's
//! warnings about its widget kit's colours and the code colours are not repeated here.

use serde_json::{Map, Value};

use super::colour::{contrast, js_blank, js_trim, mix, over, parse_hex, Rgb};
use super::{
    base_colours, built_ins, floor_of, is_unsafe, BuiltIn, Chrome, ChromeKind, FloorItem, Look,
    RegionDefault, Scheme, TextStyle, Theme, ACCENT, ANSI, CHROME, COLOUR_TOKENS, DEFAULT_FONTS,
    DEFAULT_SHAPE, DEFAULT_TEXT, DERIVED, FONT_TOKENS, MAX_NAME, MIN_BASE_TEXT, ON_ACCENT, REGIONS,
    REGION_DEFAULT, REGION_PARTS, SHAPE_TOKENS, STATES, SURFACE_0, SURFACE_1, SURFACE_2,
    SYNTAX_KEYS, TERMINAL_KEYS, TEXT, TEXT_MUTED, T_ANSI, T_BACKGROUND, T_FOREGROUND, T_SELECTION,
    WEIGHTS,
};

/// The largest theme file read (the per-person settings cap of ADR-0113 decision 6).
pub const MAX_FILE_BYTES: u64 = 256 * 1024;
/// The most problems listed; a file with more says how many more.
pub const MAX_ISSUES: usize = 50;
const MAX_DEPTH: usize = 6;
const MAX_FONT: usize = 200;
const LINE_HEIGHT: (f64, f64) = (1.2, 1.8);
/// How strong the accent's soft background is (`--tint` in the web's app.css), per scheme.
const TINT_STRENGTH: [f64; 2] = [0.14, 0.1];
/// How much of the accent the web mixes into the panels for a hairline (`--line`).
const LINE_STRENGTH: f64 = 0.4;
/// The selection is drawn at 40 % over the terminal's background; one nearer to it than this cannot be seen.
pub const SELECTION_ALPHA: f64 = 102.0 / 255.0;
const MIN_SELECTION: f64 = 1.3;

/// What a value of the wrong kind should have been.
#[derive(Clone, Debug, PartialEq)]
pub enum Expect {
    Group,
    Colour,
    Size { min: u8, max: u8 },
    List(String),
    LineHeight,
    Switch,
}

/// The colour a region's pair is about: the region's own text, or a theme token ([`COLOUR_TOKENS`] index).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionFg {
    Text,
    Token(usize),
}

/// One problem with a theme file (the web's `ThemeIssue`, by the same codes).
#[derive(Clone, Debug, PartialEq)]
pub enum Issue {
    Json,
    Invalid,
    Newer,
    Name,
    Scheme,
    NoLook,
    UnknownField(String),
    UnknownToken(String),
    DerivedToken {
        token: String,
        base: &'static str,
    },
    MovedToken {
        field: String,
        token: String,
        to: &'static str,
    },
    /// A token's value is not of its kind (the token's name).
    BadValue(&'static str),
    BadField {
        field: String,
        expect: Expect,
    },
    Unsafe(String),
    Floor {
        field: String,
        item: FloorItem,
    },
    TextFloor {
        field: String,
    },
    /// A warning: a format-1 base text size under 16 px is read as 16 px.
    TextLifted {
        from: String,
    },
    Inherits(String),
    /// A token on a token ([`COLOUR_TOKENS`] indexes).
    Contrast {
        fg: usize,
        bg: usize,
        ratio: f64,
        min: f64,
    },
    RegionContrast {
        region: usize,
        fg: RegionFg,
        ratio: f64,
        min: f64,
    },
    TerminalContrast {
        ratio: f64,
    },
    /// A warning: one of the 16 colours (0..16) on the terminal's background.
    AnsiContrast {
        colour: usize,
        ratio: f64,
    },
    /// A warning: the selection over the terminal's background.
    SelectionContrast {
        ratio: f64,
    },
    TooLarge,
    /// The file could not be read (gone, a folder, no permission).
    Unreadable,
    More(usize),
}

/// A problem, and the scheme it is in when the theme has both.
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub issue: Issue,
    pub scheme: Option<Scheme>,
}

impl From<Issue> for Problem {
    fn from(issue: Issue) -> Self {
        Problem {
            issue,
            scheme: None,
        }
    }
}

/// A file read: the theme and its warnings, or every reason it is refused.
#[derive(Clone, Debug, PartialEq)]
pub enum Read {
    Ok {
        theme: Box<Theme>,
        warnings: Vec<Problem>,
    },
    Refused(Vec<Problem>),
}

impl Read {
    pub fn theme(&self) -> Option<&Theme> {
        match self {
            Read::Ok { theme, .. } => Some(theme),
            Read::Refused(_) => None,
        }
    }
    /// The warnings of an accepted theme, or the reasons for a refused one.
    pub fn problems(&self) -> &[Problem] {
        match self {
            Read::Ok { warnings, .. } => warnings,
            Read::Refused(p) => p,
        }
    }
}

fn refused(issues: Vec<Issue>) -> Read {
    Read::Refused(issues.into_iter().map(Problem::from).collect())
}

/// Reads a theme file's text: at most [`MAX_FILE_BYTES`], JSON, then [`read_value`].
pub fn read_text(text: &str) -> Read {
    if u64::try_from(text.len()).unwrap_or(u64::MAX) > MAX_FILE_BYTES {
        return refused(vec![Issue::TooLarge]);
    }
    match serde_json::from_str::<Value>(text) {
        Ok(v) => read_value(&v),
        Err(_) => refused(vec![Issue::Json]),
    }
}

/// Reads a parsed theme file of either format.
pub fn read_value(value: &Value) -> Read {
    let Some(obj) = value.as_object() else {
        return refused(vec![Issue::Invalid]);
    };
    let format = obj.get("agora-theme").and_then(Value::as_f64);
    let Some(format) = format.filter(|f| f.fract() == 0.0 && *f >= 1.0) else {
        return refused(vec![Issue::Invalid]);
    };
    if format > 2.0 {
        return refused(vec![Issue::Newer]);
    }
    capped(if format == 1.0 {
        read_format1(obj)
    } else {
        read_format2(obj)
    })
}

fn capped(r: Read) -> Read {
    match r {
        Read::Refused(mut p) if p.len() > MAX_ISSUES => {
            let more = p.len() - MAX_ISSUES;
            p.truncate(MAX_ISSUES);
            p.push(Issue::More(more).into());
            Read::Refused(p)
        }
        r => r,
    }
}

// ---- Values ------------------------------------------------------------------------------------------------------------------

/// Control and text-direction characters never reach the screen in a name someone else wrote.
fn clean_name(v: Option<&Value>) -> String {
    let Some(s) = v.and_then(Value::as_str) else {
        return String::new();
    };
    let s = s
        .chars()
        .filter(|&c| {
            !matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}'..='\u{9f}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .collect::<String>();
    js_trim(&s).to_string()
}

/// The words of `s` joined by one space: the web's `.trim().replace(/\s+/g, ' ')`, blanks as it counts them ([`js_blank`]).
fn words(s: &str) -> String {
    s.split(js_blank)
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A key as a problem names it: cleaned, at most 40 characters.
fn short_key(k: &str) -> String {
    clean_name(Some(&Value::String(k.to_string())))
        .chars()
        .take(40)
        .collect()
}

/// The length a name has on the web (UTF-16 units), where the limit is set.
fn name_ok(name: &str) -> bool {
    let n = name.encode_utf16().count();
    n > 0 && n <= MAX_NAME
}

/// The pieces of a format-1 name that format 2 refuses, removed (a name of nothing else becomes a dash).
fn safe_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut rest = name;
    'scan: while let Some(c) = rest.chars().next() {
        for piece in ["url(", "@import", "expression("] {
            if rest.len() >= piece.len()
                && rest.is_char_boundary(piece.len())
                && rest[..piece.len()].eq_ignore_ascii_case(piece)
            {
                rest = &rest[piece.len()..];
                continue 'scan;
            }
        }
        if !matches!(c, ';' | '}' | '<') {
            out.push(c);
        }
        rest = &rest[c.len_utf8()..];
    }
    let joined = words(&out);
    if joined.is_empty() {
        "\u{2014}".to_string()
    } else {
        joined
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Colour(usize),
    Font(usize),
    Size(usize),
}

fn kind_of(name: &str) -> Option<(Kind, &'static str)> {
    if let Some(i) = COLOUR_TOKENS.iter().position(|t| *t == name) {
        return Some((Kind::Colour(i), COLOUR_TOKENS[i]));
    }
    if let Some(i) = FONT_TOKENS.iter().position(|t| *t == name) {
        return Some((Kind::Font(i), FONT_TOKENS[i]));
    }
    SHAPE_TOKENS
        .iter()
        .position(|t| t.0 == name)
        .map(|i| (Kind::Size(i), SHAPE_TOKENS[i].0))
}

fn derived_base(name: &str) -> Option<&'static str> {
    DERIVED.iter().find(|(d, _)| *d == name).map(|(_, b)| *b)
}

fn read_colour(v: &Value) -> Option<Rgb> {
    v.as_str().and_then(parse_hex)
}

/// A font list: family names, quoted or not, separated by commas; nothing that could end a CSS declaration.
fn read_font(v: &Value) -> Option<String> {
    let f = words(v.as_str()?);
    let ok_chars = f.chars().all(|c| {
        c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '.' | ',' | '\'' | '"' | '-')
    });
    let quotes = f.chars().filter(|c| matches!(c, '\'' | '"')).count();
    (!f.is_empty() && f.len() <= MAX_FONT && ok_chars && quotes % 2 == 0).then_some(f)
}

/// A size token's value: the number, or `"18px"` / `"18.5px"` (up to three digits, two decimals), within its range, as `Npx`.
fn read_size(v: &Value, min: f64, max: f64) -> Option<String> {
    let n = match v {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => {
            let s = js_trim(s).strip_suffix("px")?;
            let (whole, frac) = s.split_once('.').unwrap_or((s, ""));
            let digits = |d: &str| d.bytes().all(|b| b.is_ascii_digit());
            let shape_ok = (1..=3).contains(&whole.len())
                && digits(whole)
                && (s.contains('.') == !frac.is_empty())
                && frac.len() <= 2
                && digits(frac);
            if !shape_ok {
                return None;
            }
            s.parse::<f64>().ok()?
        }
        _ => return None,
    };
    (n.is_finite() && n >= min && n <= max).then(|| format!("{}px", (n * 100.0).round() / 100.0))
}

fn read_token(kind: Kind, v: &Value) -> Option<String> {
    match kind {
        Kind::Colour(_) => read_colour(v).map(super::colour::hex),
        Kind::Font(_) => read_font(v),
        Kind::Size(i) => read_size(v, SHAPE_TOKENS[i].1, SHAPE_TOKENS[i].2),
    }
}

/// Whole pixels in a range: the number, or `"6px"`.
fn read_px(v: &Value, min: u8, max: u8) -> Option<u8> {
    let n = match v {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => {
            let d = js_trim(s).strip_suffix("px")?;
            if !(1..=3).contains(&d.len()) || !d.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            d.parse::<f64>().ok()?
        }
        _ => return None,
    };
    (n.fract() == 0.0 && n >= f64::from(min) && n <= f64::from(max)).then_some(n as u8)
}

fn px_of(v: &str) -> f64 {
    v.trim_end_matches("px").parse().unwrap_or(0.0)
}

fn dedupe(issues: Vec<Issue>) -> Vec<Issue> {
    let mut out: Vec<Issue> = Vec::with_capacity(issues.len());
    for i in issues {
        if !out.contains(&i) {
            out.push(i);
        }
    }
    out
}

// ---- Format 1 ----------------------------------------------------------------------------------------------------------------

struct Format1 {
    name: String,
    scheme: Scheme,
    colours: [Rgb; 13],
    shape: [String; 6],
    fonts: [String; 2],
}

/// Format 1 exactly as the web's `parseTheme` reads it.
fn parse_format1(value: &Map<String, Value>) -> Result<Format1, Vec<Issue>> {
    let mut issues = Vec::new();
    for k in value.keys() {
        if !["agora-theme", "name", "scheme", "tokens"].contains(&k.as_str()) {
            issues.push(Issue::UnknownField(short_key(k)));
        }
    }
    let name = clean_name(value.get("name"));
    if !name_ok(&name) {
        issues.push(Issue::Name);
    }
    let scheme = match value.get("scheme").and_then(Value::as_str) {
        Some("dark") => Scheme::Dark,
        Some("light") => Scheme::Light,
        _ => {
            issues.push(Issue::Scheme);
            return Err(issues);
        }
    };
    let tokens = match value.get("tokens") {
        None => None,
        Some(Value::Object(t)) => Some(t),
        Some(_) => {
            issues.push(Issue::Invalid);
            return Err(issues);
        }
    };
    let mut colours = base_colours(scheme);
    let mut shape = DEFAULT_SHAPE.map(str::to_string);
    let mut fonts = DEFAULT_FONTS.map(str::to_string);
    let mut bad = false;
    for (k, v) in tokens.into_iter().flatten() {
        if let Some(base) = derived_base(k) {
            issues.push(Issue::DerivedToken {
                token: k.clone(),
                base,
            });
            continue;
        }
        let Some((kind, name)) = kind_of(k) else {
            issues.push(Issue::UnknownToken(short_key(k)));
            continue;
        };
        match (kind, read_token(kind, v)) {
            (_, None) => {
                bad = true;
                issues.push(Issue::BadValue(name));
            }
            (Kind::Colour(i), Some(_)) => colours[i] = read_colour(v).unwrap_or(colours[i]),
            (Kind::Font(i), Some(f)) => fonts[i] = f,
            (Kind::Size(i), Some(s)) => shape[i] = s,
        }
    }
    // Contrast is judged only on colours that were all readable: a bad value already says what is wrong.
    if !bad {
        issues.extend(contrast_issues(&colours));
    }
    if issues.is_empty() {
        Ok(Format1 {
            name,
            scheme,
            colours,
            shape,
            fonts,
        })
    } else {
        Err(dedupe(issues))
    }
}

/// Format 1, plus what the floor asks of it: the base text size is lifted to 16 px with a warning, and a token that names the
/// floor says so instead of "not a theme token".
fn read_format1(value: &Map<String, Value>) -> Read {
    let mut floor = Vec::new();
    if let Some(Value::Object(t)) = value.get("tokens") {
        for k in t.keys() {
            if let Some(item) = floor_of(k) {
                floor.push((k.clone(), item));
            }
        }
    }
    let floor_issues = || {
        floor
            .iter()
            .map(|(k, item)| Issue::Floor {
                field: format!("tokens.{k}"),
                item: *item,
            })
            .collect::<Vec<_>>()
    };
    let p = match parse_format1(value) {
        Ok(p) => p,
        Err(issues) => {
            let named: Vec<String> = floor.iter().map(|(k, _)| short_key(k)).collect();
            let rest = issues
                .into_iter()
                .filter(|i| !matches!(i, Issue::UnknownToken(t) if named.contains(t)));
            return refused(floor_issues().into_iter().chain(rest).collect());
        }
    };
    if !floor.is_empty() {
        return refused(floor_issues());
    }
    let mut warnings = Vec::new();
    let mut shape = p.shape;
    if px_of(&shape[0]) < MIN_BASE_TEXT {
        warnings.push(Problem::from(Issue::TextLifted {
            from: shape[0].clone(),
        }));
        shape[0] = format!("{MIN_BASE_TEXT}px");
    }
    let mut looks = [None, None];
    looks[usize::from(p.scheme == Scheme::Light)] = Some(Look::of(p.colours));
    Read::Ok {
        theme: Box::new(Theme {
            name: safe_name(&p.name),
            looks,
            shape,
            fonts: p.fonts,
            text: DEFAULT_TEXT,
            chrome: Chrome::default(),
        }),
        warnings,
    }
}

// ---- Format 2 ----------------------------------------------------------------------------------------------------------------

const TOP_FIELDS: [&str; 8] = [
    "agora-theme",
    "name",
    "inherits",
    "dark",
    "light",
    "shape",
    "text",
    "chrome",
];
const LOOK_FIELDS: [&str; 4] = ["tokens", "regions", "terminal", "syntax"];
const TEXT_FIELDS: [&str; 5] = ["font-ui", "font-mono", "weight", "line-height", "ligatures"];

/// Every string in a file with where it is, so an unsafe one is refused wherever it is. `false` when the file nests too deep.
fn strings(v: &Value, path: &str, depth: usize, out: &mut Vec<(String, String)>) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    match v {
        Value::String(s) => out.push((path.to_string(), s.clone())),
        Value::Array(a) => {
            for (i, x) in a.iter().enumerate() {
                if !strings(x, &format!("{path}[{i}]"), depth + 1, out) {
                    return false;
                }
            }
        }
        Value::Object(o) => {
            for (k, x) in o {
                let p = if path.is_empty() {
                    short_key(k)
                } else {
                    format!("{path}.{}", short_key(k))
                };
                if !strings(x, &p, depth + 1, out) {
                    return false;
                }
            }
        }
        _ => {}
    }
    true
}

/// Collects the problems of a format-2 file as the web's `readFormat2` does.
struct Reader {
    issues: Vec<Issue>,
    unsafe_paths: Vec<String>,
}

impl Reader {
    /// A field whose value was refused as unsafe is not judged again as a value.
    fn ok(&self, path: &str) -> bool {
        !self.unsafe_paths.iter().any(|p| p == path)
    }
    fn floor_key(&mut self, key: &str, field: &str) -> bool {
        match floor_of(key) {
            Some(item) => {
                self.issues.push(Issue::Floor {
                    field: field.to_string(),
                    item,
                });
                true
            }
            None => false,
        }
    }
    fn unknown(&mut self, field: &str) {
        self.issues.push(Issue::UnknownField(short_key(field)));
    }
    fn group<'v>(&mut self, v: Option<&'v Value>, field: &str) -> Option<&'v Map<String, Value>> {
        match v? {
            Value::Object(o) => Some(o),
            _ => {
                if self.ok(field) {
                    self.issues.push(Issue::BadField {
                        field: field.to_string(),
                        expect: Expect::Group,
                    });
                }
                None
            }
        }
    }
    fn colour(&mut self, v: &Value, field: &str) -> Option<Rgb> {
        let c = read_colour(v);
        if c.is_none() && self.ok(field) {
            self.issues.push(Issue::BadField {
                field: field.to_string(),
                expect: Expect::Colour,
            });
        }
        c
    }
}

struct Raw {
    colours: Vec<(usize, Rgb)>,
    regions: [[Option<Rgb>; 3]; 10],
    terminal: [Option<Rgb>; 20],
    syntax: [Option<Rgb>; 8],
}

fn read_format2(value: &Map<String, Value>) -> Read {
    let mut found = Vec::new();
    if !strings(&Value::Object(value.clone()), "", 0, &mut found) {
        return refused(vec![Issue::Invalid]);
    }
    let mut r = Reader {
        issues: Vec::new(),
        unsafe_paths: Vec::new(),
    };
    for (path, s) in &found {
        if is_unsafe(s) && !r.unsafe_paths.contains(path) {
            r.unsafe_paths.push(path.clone());
            r.issues.push(Issue::Unsafe(path.clone()));
        }
    }

    for k in value.keys() {
        if !TOP_FIELDS.contains(&k.as_str()) && !r.floor_key(k, k) {
            r.unknown(k);
        }
    }
    let name = clean_name(value.get("name"));
    if r.ok("name") && !name_ok(&name) {
        r.issues.push(Issue::Name);
    }

    // `inherits`: a built-in theme whose values stand in for the ones the file leaves out.
    let mut base: Option<BuiltIn> = None;
    if let Some(v) = value.get("inherits") {
        base = v
            .as_str()
            .and_then(|id| built_ins().into_iter().find(|b| b.id == id));
        if base.is_none() && r.ok("inherits") {
            r.issues.push(Issue::Inherits(
                v.as_str().map(short_key).unwrap_or_default(),
            ));
        }
    }

    // Shape and fonts: shared by both schemes.
    let mut shape: [Option<String>; 6] = Default::default();
    let mut fonts: [Option<String>; 2] = Default::default();
    let shape_group = r.group(value.get("shape"), "shape");
    for (k, v) in shape_group.into_iter().flatten() {
        let field = format!("shape.{k}");
        if r.floor_key(k, &field) {
            continue;
        }
        if let Some(b) = derived_base(k) {
            r.issues.push(Issue::DerivedToken {
                token: k.clone(),
                base: b,
            });
            continue;
        }
        let kind = kind_of(k);
        let Some((Kind::Size(i), tname)) = kind else {
            match kind {
                Some((Kind::Font(_), _)) => r.issues.push(Issue::MovedToken {
                    field,
                    token: k.clone(),
                    to: "text",
                }),
                Some((Kind::Colour(_), _)) => r.issues.push(Issue::MovedToken {
                    field,
                    token: k.clone(),
                    to: "tokens",
                }),
                _ => r.unknown(&field),
            }
            continue;
        };
        if !r.ok(&field) {
            continue;
        }
        match read_size(v, SHAPE_TOKENS[i].1, SHAPE_TOKENS[i].2) {
            None => r.issues.push(Issue::BadValue(tname)),
            Some(s) if i == 0 && px_of(&s) < MIN_BASE_TEXT => {
                r.issues.push(Issue::TextFloor { field })
            }
            Some(s) => shape[i] = Some(s),
        }
    }
    let mut text: TextStyle = DEFAULT_TEXT;
    let text_group = r.group(value.get("text"), "text");
    for (k, v) in text_group.into_iter().flatten() {
        let field = format!("text.{k}");
        if r.floor_key(k, &field) {
            continue;
        }
        if !TEXT_FIELDS.contains(&k.as_str()) {
            if matches!(kind_of(k), Some((Kind::Size(_), _))) {
                r.issues.push(Issue::MovedToken {
                    field,
                    token: k.clone(),
                    to: "shape",
                });
            } else {
                r.unknown(&field);
            }
            continue;
        }
        if !r.ok(&field) {
            continue;
        }
        match k.as_str() {
            "font-ui" | "font-mono" => {
                let i = usize::from(k == "font-mono");
                match read_font(v) {
                    Some(f) => fonts[i] = Some(f),
                    None => r.issues.push(Issue::BadValue(FONT_TOKENS[i])),
                }
            }
            "weight" => match v
                .as_f64()
                .filter(|w| w.fract() == 0.0)
                .and_then(|w| WEIGHTS.iter().find(|x| f64::from(**x) == w))
            {
                Some(w) => text.weight = *w,
                None => r.issues.push(Issue::BadField {
                    field,
                    expect: Expect::List(WEIGHTS.map(|w| w.to_string()).join(", ")),
                }),
            },
            "line-height" => match v {
                Value::String(s) if s == "normal" => text.line_height = None,
                Value::Number(n)
                    if n.as_f64().is_some_and(|x| {
                        x.is_finite() && x >= LINE_HEIGHT.0 && x <= LINE_HEIGHT.1
                    }) =>
                {
                    let x = n.as_f64().unwrap_or(LINE_HEIGHT.0);
                    // 1.2..=1.8 as hundredths: always 120..=180.
                    text.line_height = Some((x * 100.0).round() as u16);
                }
                _ => r.issues.push(Issue::BadField {
                    field,
                    expect: Expect::LineHeight,
                }),
            },
            _ => match v {
                Value::Bool(b) => text.ligatures = *b,
                _ => r.issues.push(Issue::BadField {
                    field,
                    expect: Expect::Switch,
                }),
            },
        }
    }

    let mut chrome = Chrome::default();
    let chrome_group = r.group(value.get("chrome"), "chrome");
    for (k, v) in chrome_group.into_iter().flatten() {
        let field = format!("chrome.{k}");
        if r.floor_key(k, &field) {
            continue;
        }
        let Some(i) = CHROME.iter().position(|(n, _)| n == k) else {
            r.unknown(&field);
            continue;
        };
        if !r.ok(&field) {
            continue;
        }
        match CHROME[i].1 {
            ChromeKind::Px { min, max, .. } => match read_px(v, min, max) {
                Some(n) => chrome.0[i] = n,
                None => r.issues.push(Issue::BadField {
                    field,
                    expect: Expect::Size { min, max },
                }),
            },
            ChromeKind::List { values, .. } => {
                // The value kept is the list's own index, never the file's string.
                match v.as_str().and_then(|s| values.iter().position(|w| *w == s)) {
                    // A list has at most a few words.
                    Some(at) => chrome.0[i] = u8::try_from(at).unwrap_or(0),
                    None => r.issues.push(Issue::BadField {
                        field,
                        expect: Expect::List(values.join(", ")),
                    }),
                }
            }
        }
    }

    // The schemes: each section present, and the scheme of the theme it inherits from.
    let mut raw: [Option<Raw>; 2] = [None, None];
    for (si, scheme) in Scheme::BOTH.into_iter().enumerate() {
        let word = scheme.word();
        let section = r.group(value.get(word), word);
        let inherited =
            base.as_ref().is_some_and(|b| b.scheme == scheme) && value.get(word).is_none();
        if section.is_none() && !inherited {
            continue;
        }
        let mut out = Raw {
            colours: Vec::new(),
            regions: [[None; 3]; 10],
            terminal: [None; 20],
            syntax: [None; 8],
        };
        for k in section.into_iter().flat_map(Map::keys) {
            if !LOOK_FIELDS.contains(&k.as_str()) && !r.floor_key(k, &format!("{word}.{k}")) {
                r.unknown(&format!("{word}.{k}"));
            }
        }
        let tokens = r.group(
            section.and_then(|s| s.get("tokens")),
            &format!("{word}.tokens"),
        );
        for (k, v) in tokens.into_iter().flatten() {
            let field = format!("{word}.tokens.{k}");
            if r.floor_key(k, &field) {
                continue;
            }
            if let Some(b) = derived_base(k) {
                r.issues.push(Issue::DerivedToken {
                    token: k.clone(),
                    base: b,
                });
                continue;
            }
            let Some((kind, tname)) = kind_of(k) else {
                r.issues.push(Issue::UnknownToken(short_key(k)));
                continue;
            };
            let Kind::Colour(i) = kind else {
                r.issues.push(Issue::MovedToken {
                    field,
                    token: k.clone(),
                    to: if matches!(kind, Kind::Font(_)) {
                        "text"
                    } else {
                        "shape"
                    },
                });
                continue;
            };
            if !r.ok(&field) {
                continue;
            }
            match read_colour(v) {
                Some(c) => out.colours.push((i, c)),
                None => r.issues.push(Issue::BadValue(tname)),
            }
        }
        let regions = r.group(
            section.and_then(|s| s.get("regions")),
            &format!("{word}.regions"),
        );
        for (rk, v) in regions.into_iter().flatten() {
            let field = format!("{word}.regions.{rk}");
            if r.floor_key(rk, &field) {
                continue;
            }
            let Some(ri) = REGIONS.iter().position(|x| x == rk) else {
                r.unknown(&field);
                continue;
            };
            let parts = r.group(Some(v), &field);
            for (p, c) in parts.into_iter().flatten() {
                let pf = format!("{field}.{p}");
                if r.floor_key(p, &pf) {
                    continue;
                }
                let Some(pi) = REGION_PARTS.iter().position(|x| x == p) else {
                    r.unknown(&pf);
                    continue;
                };
                if !r.ok(&pf) {
                    continue;
                }
                if let Some(c) = r.colour(c, &pf) {
                    out.regions[ri][pi] = Some(c);
                }
            }
        }
        let terminal = r.group(
            section.and_then(|s| s.get("terminal")),
            &format!("{word}.terminal"),
        );
        for (k, c) in terminal.into_iter().flatten() {
            let field = format!("{word}.terminal.{k}");
            if r.floor_key(k, &field) {
                continue;
            }
            let Some(ti) = TERMINAL_KEYS.iter().position(|x| x == k) else {
                r.unknown(&field);
                continue;
            };
            if !r.ok(&field) {
                continue;
            }
            if let Some(c) = r.colour(c, &field) {
                out.terminal[ti] = Some(c);
            }
        }
        let syntax = r.group(
            section.and_then(|s| s.get("syntax")),
            &format!("{word}.syntax"),
        );
        for (k, c) in syntax.into_iter().flatten() {
            let field = format!("{word}.syntax.{k}");
            if r.floor_key(k, &field) {
                continue;
            }
            let Some(yi) = SYNTAX_KEYS.iter().position(|x| x == k) else {
                r.unknown(&field);
                continue;
            };
            if !r.ok(&field) {
                continue;
            }
            if let Some(c) = r.colour(c, &field) {
                out.syntax[yi] = Some(c);
            }
        }
        raw[si] = Some(out);
    }
    if raw.iter().all(Option::is_none) && r.issues.is_empty() {
        r.issues.push(Issue::NoLook);
    }
    if !r.issues.is_empty() {
        return refused(dedupe(r.issues));
    }

    // Every value is readable: resolve each scheme from its base, then judge readability.
    let base_theme = base.as_ref().map(|b| &b.theme);
    let resolved_shape: [String; 6] = std::array::from_fn(|i| {
        shape[i].clone().unwrap_or_else(|| {
            base_theme.map_or_else(|| DEFAULT_SHAPE[i].to_string(), |b| b.shape[i].clone())
        })
    });
    let resolved_fonts: [String; 2] = std::array::from_fn(|i| {
        fonts[i].clone().unwrap_or_else(|| {
            base_theme.map_or_else(|| DEFAULT_FONTS[i].to_string(), |b| b.fonts[i].clone())
        })
    });
    let mut looks: [Option<Look>; 2] = [None, None];
    for (si, scheme) in Scheme::BOTH.into_iter().enumerate() {
        let Some(raw) = raw[si].take() else {
            continue;
        };
        let mut colours = match &base {
            Some(b) if b.scheme == scheme => b
                .theme
                .look(scheme)
                .map_or_else(|| base_colours(scheme), |l| l.colours),
            _ => base_colours(scheme),
        };
        for (i, c) in raw.colours {
            colours[i] = c;
        }
        looks[si] = Some(Look {
            colours,
            regions: raw.regions,
            terminal: raw.terminal,
            syntax: raw.syntax,
        });
    }
    let theme = Theme {
        name,
        looks,
        shape: resolved_shape,
        fonts: resolved_fonts,
        text,
        chrome,
    };
    let (refusals, warnings) = judge(&theme);
    if !refusals.is_empty() {
        return Read::Refused(refusals);
    }
    Read::Ok {
        theme: Box::new(theme),
        warnings,
    }
}

// ---- Readability ---------------------------------------------------------------------------------------------------------------

/// The pairs every theme's tokens must pass (WCAG 2.2 AA): text, secondary text and the accent at 4.5:1 on each surface, the text
/// on the accent at 4.5:1, each state at 3:1 on each surface.
pub fn contrast_pairs() -> Vec<(usize, usize, f64)> {
    let surfaces = [SURFACE_0, SURFACE_1, SURFACE_2];
    let mut out = Vec::new();
    for fg in [TEXT, TEXT_MUTED, ACCENT] {
        out.extend(surfaces.map(|bg| (fg, bg, 4.5)));
    }
    out.push((ON_ACCENT, ACCENT, 4.5));
    for fg in STATES {
        out.extend(surfaces.map(|bg| (fg, bg, 3.0)));
    }
    out
}

fn contrast_issues(c: &[Rgb; 13]) -> Vec<Issue> {
    contrast_pairs()
        .into_iter()
        .filter_map(|(fg, bg, min)| {
            let ratio = contrast(c[fg], c[bg]);
            (ratio < min).then_some(Issue::Contrast { fg, bg, ratio, min })
        })
        .collect()
}

/// The colour a region's part is drawn in for one scheme: the theme's own, or the token (or kit colour) it follows.
pub fn region_colour(look: &Look, scheme: Scheme, region: usize, part: usize) -> Rgb {
    if let Some(c) = look.regions[region][part] {
        return c;
    }
    let tint = TINT_STRENGTH[usize::from(scheme == Scheme::Light)];
    match REGION_DEFAULT[region][part] {
        RegionDefault::Token(i) => look.colours[i],
        RegionDefault::TintAccent => mix(look.colours[ACCENT], tint, look.colours[SURFACE_1]),
        RegionDefault::LineAccent => {
            mix(look.colours[ACCENT], LINE_STRENGTH, look.colours[SURFACE_1])
        }
    }
}

/// The colour a terminal pane draws one of its colours in: the theme's own, or what a pane draws without it.
pub fn terminal_colour(look: &Look, scheme: Scheme, k: usize) -> Rgb {
    if let Some(c) = look.terminal[k] {
        return c;
    }
    match k {
        T_BACKGROUND => look.colours[SURFACE_0],
        T_FOREGROUND => look.colours[TEXT],
        k if k < T_ANSI => look.colours[ACCENT],
        k => ANSI[usize::from(scheme == Scheme::Light)][(k - T_ANSI).min(15)],
    }
}

/// The problems of one scheme's colours: refusals keep the theme from being used, warnings are listed with it.
fn look_issues(look: &Look, scheme: Scheme) -> (Vec<Issue>, Vec<Issue>) {
    let mut refusals = contrast_issues(&look.colours);
    for r in 0..REGIONS.len() {
        let own = look.regions[r];
        if own[0].is_none() && own[1].is_none() {
            continue;
        }
        let bg = region_colour(look, scheme, r, 0);
        let mut pairs = vec![(RegionFg::Text, region_colour(look, scheme, r, 1), 4.5)];
        if own[0].is_some() {
            pairs.push((RegionFg::Token(TEXT_MUTED), look.colours[TEXT_MUTED], 4.5));
            pairs.push((RegionFg::Token(ACCENT), look.colours[ACCENT], 4.5));
            for s in STATES {
                pairs.push((RegionFg::Token(s), look.colours[s], 3.0));
            }
        }
        for (fg, colour, min) in pairs {
            let ratio = contrast(colour, bg);
            if ratio < min {
                refusals.push(Issue::RegionContrast {
                    region: r,
                    fg,
                    ratio,
                    min,
                });
            }
        }
    }
    let bg = terminal_colour(look, scheme, T_BACKGROUND);
    if look.terminal[T_BACKGROUND].is_some() || look.terminal[T_FOREGROUND].is_some() {
        let ratio = contrast(terminal_colour(look, scheme, T_FOREGROUND), bg);
        if ratio < 4.5 {
            refusals.push(Issue::TerminalContrast { ratio });
        }
    }
    // Programs choose the 16 colours: a hard one is only warned about, never the one the background stands in for.
    let mut warnings = Vec::new();
    let twin = if contrast(bg, [255; 3]) > contrast(bg, [0; 3]) {
        0
    } else {
        7
    };
    for n in 0..16 {
        if n == twin
            || (look.terminal[T_ANSI + n].is_none() && look.terminal[T_BACKGROUND].is_none())
        {
            continue;
        }
        let ratio = contrast(terminal_colour(look, scheme, T_ANSI + n), bg);
        if ratio < 3.0 {
            warnings.push(Issue::AnsiContrast { colour: n, ratio });
        }
    }
    if look.terminal[T_SELECTION].is_some() || look.terminal[T_BACKGROUND].is_some() {
        let sel = over(
            terminal_colour(look, scheme, T_SELECTION),
            SELECTION_ALPHA,
            bg,
        );
        let ratio = contrast(sel, bg);
        if ratio < MIN_SELECTION {
            warnings.push(Issue::SelectionContrast { ratio });
        }
    }
    (refusals, warnings)
}

/// Both schemes; a problem names its scheme when the theme has two.
pub fn judge(theme: &Theme) -> (Vec<Problem>, Vec<Problem>) {
    let both = theme.looks.iter().all(Option::is_some);
    let (mut refusals, mut warnings) = (Vec::new(), Vec::new());
    for scheme in Scheme::BOTH {
        let Some(look) = theme.look(scheme) else {
            continue;
        };
        let (r, w) = look_issues(look, scheme);
        let tag = |issue| Problem {
            issue,
            scheme: both.then_some(scheme),
        };
        refusals.extend(r.into_iter().map(tag));
        warnings.extend(w.into_iter().map(tag));
    }
    (refusals, warnings)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;

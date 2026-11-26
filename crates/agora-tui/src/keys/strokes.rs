//! Key strings as the keybindings file writes them (`docs/ux/keybindings.md`, "Keys"; the web's `keys/strokes.ts`), and the stroke a
//! key typed in the terminal client is.
//!
//! The grammar and the one spelling are the web window's, so a file one client writes the other reads. A terminal reports characters,
//! not key places: a letter with Shift arrives as the capital letter (read as `shift+` the letter), and Shift with a digit or a sign,
//! Ctrl with Shift, and Meta never arrive as such. Such a binding is valid (the web uses it) and never acts here.

use crate::app::Key;

/// The longest key string, and the most strokes in one (a chord of two).
pub const MAX_KEY_STRING: usize = 64;
pub const MAX_STROKES: usize = 2;

/// One stroke: modifiers and one key, by the name the file writes (`k`, `1`, `enter`, `f5`, `/`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Stroke {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
    pub key: String,
}

/// One or two strokes.
pub type Binding = Vec<Stroke>;

const NAMED: [&str; 15] = [
    "enter",
    "escape",
    "tab",
    "space",
    "backspace",
    "delete",
    "insert",
    "home",
    "end",
    "pageup",
    "pagedown",
    "up",
    "down",
    "left",
    "right",
];
/// The signs a key may be, by their place on a US keyboard (the web's `PUNCT`).
const PUNCT: [char; 11] = ['-', '=', '[', ']', '\\', ';', '\'', ',', '.', '/', '`'];

fn is_letter(k: &str) -> bool {
    k.len() == 1 && k.as_bytes()[0].is_ascii_lowercase()
}

fn is_digit(k: &str) -> bool {
    k.len() == 1 && k.as_bytes()[0].is_ascii_digit()
}

fn is_punct(k: &str) -> bool {
    let mut c = k.chars();
    matches!((c.next(), c.next()), (Some(p), None) if PUNCT.contains(&p))
}

fn is_fkey(k: &str) -> bool {
    k.strip_prefix('f')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=12).contains(&n) && !k.starts_with("f0"))
}

/// Whether `k` is a key a stroke may end in.
pub fn is_key_name(k: &str) -> bool {
    is_letter(k) || is_digit(k) || is_punct(k) || NAMED.contains(&k) || is_fkey(k)
}

fn alias(k: &str) -> &str {
    match k {
        "esc" => "escape",
        "return" => "enter",
        "del" => "delete",
        "ins" => "insert",
        "arrowup" => "up",
        "arrowdown" => "down",
        "arrowleft" => "left",
        "arrowright" => "right",
        "pgup" => "pageup",
        "pgdn" => "pagedown",
        other => other,
    }
}

/// One stroke from its text (`ctrl+shift+k`), or `None`: an unknown key or modifier, a modifier twice, no key.
pub fn parse_stroke(text: &str) -> Option<Stroke> {
    let lower = text.to_lowercase();
    let parts: Vec<&str> = lower.split('+').collect();
    if parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    let (last, mods) = parts.split_last()?;
    let mut s = Stroke::default();
    for m in mods {
        let flag = match *m {
            "ctrl" | "control" => &mut s.ctrl,
            "alt" | "option" | "opt" => &mut s.alt,
            "shift" => &mut s.shift,
            "meta" | "cmd" | "command" | "super" | "win" => &mut s.meta,
            _ => return None,
        };
        if *flag {
            return None;
        }
        *flag = true;
    }
    let key = alias(last);
    if !is_key_name(key) {
        return None;
    }
    s.key = key.to_string();
    Some(s)
}

/// A key string (`ctrl+k ctrl+s`) as its strokes, or `None` when any part is malformed. Exactly one space between strokes.
pub fn parse_binding(text: &str) -> Option<Binding> {
    if text.is_empty() || text.len() > MAX_KEY_STRING || text != text.trim() {
        return None;
    }
    let parts: Vec<&str> = text.split(' ').collect();
    if parts.len() > MAX_STROKES {
        return None;
    }
    parts.into_iter().map(parse_stroke).collect()
}

/// The one spelling of a stroke: modifiers in a fixed order (ctrl, alt, shift, meta), then the key.
pub fn stroke_text(s: &Stroke) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for (on, name) in [
        (s.ctrl, "ctrl"),
        (s.alt, "alt"),
        (s.shift, "shift"),
        (s.meta, "meta"),
    ] {
        if on {
            parts.push(name);
        }
    }
    parts.push(&s.key);
    parts.join("+")
}

pub fn binding_text(b: &[Stroke]) -> String {
    b.iter().map(stroke_text).collect::<Vec<_>>().join(" ")
}

/// A stroke as the terminal client shows it (`Ctrl+K`, `Alt+Shift+1`, `Esc`): key caps, the same in every language, as the web shows them.
pub fn stroke_label(s: &Stroke) -> String {
    let cap = match s.key.as_str() {
        "enter" => "Enter".to_string(),
        "escape" => "Esc".to_string(),
        "tab" => "Tab".to_string(),
        "space" => "Space".to_string(),
        "backspace" => "Backspace".to_string(),
        "delete" => "Delete".to_string(),
        "insert" => "Insert".to_string(),
        "home" => "Home".to_string(),
        "end" => "End".to_string(),
        "pageup" => "PageUp".to_string(),
        "pagedown" => "PageDown".to_string(),
        "up" => "↑".to_string(),
        "down" => "↓".to_string(),
        "left" => "←".to_string(),
        "right" => "→".to_string(),
        k => k.to_uppercase(),
    };
    let mut parts: Vec<String> = Vec::new();
    for (on, name) in [
        (s.ctrl, "Ctrl"),
        (s.alt, "Alt"),
        (s.shift, "Shift"),
        (s.meta, "Meta"),
    ] {
        if on {
            parts.push(name.to_string());
        }
    }
    parts.push(cap);
    parts.join("+")
}

pub fn binding_label(b: &[Stroke]) -> String {
    b.iter().map(stroke_label).collect::<Vec<_>>().join(" ")
}

/// The key a character of the grammar is: a letter, a digit or a sign (`None` for anything else: `<`, `!`, a letter of another
/// alphabet).
fn char_key(c: char) -> Option<String> {
    let k = c.to_string();
    (is_letter(&k) || is_digit(&k) || is_punct(&k)).then_some(k)
}

/// The stroke a key typed in the terminal client is, or `None` for one no binding can name. A capital letter is Shift with it.
pub fn stroke_of(k: &Key) -> Option<Stroke> {
    let with = |ctrl: bool, alt: bool, c: char| -> Option<Stroke> {
        if c == ' ' {
            return Some(Stroke {
                ctrl,
                alt,
                key: "space".into(),
                ..Stroke::default()
            });
        }
        let shift = c.is_ascii_uppercase();
        let key = char_key(c.to_ascii_lowercase())?;
        Some(Stroke {
            ctrl,
            alt,
            shift,
            meta: false,
            key,
        })
    };
    let named = |key: &str, shift: bool| {
        Some(Stroke {
            shift,
            key: key.to_string(),
            ..Stroke::default()
        })
    };
    match k {
        Key::Char(c) => with(false, false, *c),
        Key::Ctrl(c) => with(true, false, *c),
        Key::Alt(c) => with(false, true, *c),
        Key::Enter => named("enter", false),
        Key::Esc => named("escape", false),
        Key::Tab => named("tab", false),
        Key::BackTab => named("tab", true),
        Key::Backspace => named("backspace", false),
        Key::Delete => named("delete", false),
        Key::Up => named("up", false),
        Key::Down => named("down", false),
        Key::Left => named("left", false),
        Key::Right => named("right", false),
        Key::Home => named("home", false),
        Key::End => named("end", false),
        Key::PageUp => named("pageup", false),
        Key::PageDown => named("pagedown", false),
        Key::F(n @ 1..=12) => named(&format!("f{n}"), false),
        Key::F(_) => None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn strokes_parse_in_any_order_and_case_and_write_one_spelling() {
        let s = parse_stroke("Shift+CTRL+k").unwrap();
        assert_eq!(stroke_text(&s), "ctrl+shift+k");
        assert_eq!(
            stroke_text(&parse_stroke("cmd+opt+esc").unwrap()),
            "alt+meta+escape"
        );
        assert_eq!(stroke_text(&parse_stroke("f12").unwrap()), "f12");
        for bad in [
            "",
            "ctrl+",
            "+k",
            "ctrl+ctrl+k",
            "hyper+k",
            "f13",
            "f0",
            "f01",
            "<",
            "ctrl+kk",
            "é",
        ] {
            assert_eq!(parse_stroke(bad), None, "{bad}");
        }
    }

    #[test]
    fn bindings_are_one_or_two_strokes_with_one_space() {
        assert_eq!(
            binding_text(&parse_binding("ctrl+k 2").unwrap()),
            "ctrl+k 2"
        );
        for bad in ["ctrl+k  2", " ctrl+k", "a b c", "ctrl+k ", &"a".repeat(65)] {
            assert_eq!(parse_binding(bad), None, "{bad}");
        }
    }

    #[test]
    fn typed_keys_are_strokes_and_a_capital_is_shift() {
        let text = |k: Key| stroke_of(&k).map(|s| stroke_text(&s));
        assert_eq!(text(Key::Char('N')).as_deref(), Some("shift+n"));
        assert_eq!(text(Key::Alt('3')).as_deref(), Some("alt+3"));
        assert_eq!(text(Key::Alt('K')).as_deref(), Some("alt+shift+k"));
        assert_eq!(text(Key::Ctrl(']')).as_deref(), Some("ctrl+]"));
        assert_eq!(text(Key::BackTab).as_deref(), Some("shift+tab"));
        assert_eq!(text(Key::Char(' ')).as_deref(), Some("space"));
        assert_eq!(text(Key::F(5)).as_deref(), Some("f5"));
        assert_eq!(text(Key::Char('<')), None);
        assert_eq!(text(Key::Char('ü')), None);
    }

    #[test]
    fn labels_are_key_caps() {
        assert_eq!(
            binding_label(&parse_binding("ctrl+k alt+shift+1").unwrap()),
            "Ctrl+K Alt+Shift+1"
        );
        assert_eq!(stroke_label(&parse_stroke("esc").unwrap()), "Esc");
    }
}

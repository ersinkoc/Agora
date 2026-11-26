//! Theme packs in the terminal client (ADR-0113 decision 5, `docs/ux/themes.md` "Theme packs"): a pack is one JSON file that bundles
//! format-2 themes, saved layouts and keybindings (`apps/web/src/themes/pack.ts` reads and writes them). A pack in the themes folder
//! gives this client its themes; the layouts and keybindings are the web window's (this client has neither a layout model nor
//! keybindings yet), so they are left alone and the theme list says so ("not used in the terminal client").
//!
//! Like a theme file it is data and never run. The pack's own fields are a closed set, its name is held to the theme name rules, and
//! every theme is read by the theme reader ([`read_value`]): one refused theme refuses the whole pack, with every problem at once, as
//! the window does. The parts this client does not use are checked only for their form (a list of at most [`MAX_LAYOUTS`], an
//! object), since their rules are the window's.

use serde_json::Value;

use super::file::{clean_name, read_value, short_key, Issue, Problem, Read, MAX_ISSUES};
use super::{is_unsafe, Theme, MAX_NAME};

/// The field that marks a pack, and the only format this client reads.
pub const PACK_KEY: &str = "agora-pack";
pub const PACK_FORMAT: f64 = 1.0;
/// The most themes and layouts a pack holds (the window's own limits).
pub const MAX_THEMES: usize = 20;
pub const MAX_LAYOUTS: usize = 50;
const TOP: [&str; 5] = [PACK_KEY, "name", "themes", "layouts", "keybindings"];

/// A pack read: its name, its themes with their warnings, and how much of what this client does not use it holds.
#[derive(Clone, Debug, PartialEq)]
pub struct Pack {
    pub name: String,
    pub themes: Vec<Read>,
    pub layouts: usize,
    /// The commands its keybindings set.
    pub keys: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PackRead {
    Ok(Box<Pack>),
    Refused(Vec<Problem>),
}

impl PackRead {
    pub fn problems(&self) -> &[Problem] {
        match self {
            PackRead::Ok(_) => &[],
            PackRead::Refused(p) => p,
        }
    }
}

/// Whether a parsed file is a pack (it has the pack's marker) rather than a theme.
pub fn is_pack(value: &Value) -> bool {
    value.as_object().is_some_and(|o| o.contains_key(PACK_KEY))
}

fn refused(issues: Vec<Issue>) -> PackRead {
    let mut p: Vec<Problem> = issues.into_iter().map(Problem::from).collect();
    if p.len() > MAX_ISSUES {
        let more = p.len() - MAX_ISSUES;
        p.truncate(MAX_ISSUES);
        p.push(Issue::More(more).into());
    }
    PackRead::Refused(p)
}

/// Reads a parsed pack. Never panics.
pub fn read_pack(value: &Value) -> PackRead {
    let Some(obj) = value.as_object() else {
        return refused(vec![Issue::Invalid]);
    };
    let Some(format) = obj
        .get(PACK_KEY)
        .and_then(Value::as_f64)
        .filter(|f| f.fract() == 0.0 && *f >= 1.0)
    else {
        return refused(vec![Issue::Invalid]);
    };
    if format > PACK_FORMAT {
        return refused(vec![Issue::Newer]);
    }
    let mut issues = Vec::new();
    for k in obj.keys().filter(|k| !TOP.contains(&k.as_str())) {
        issues.push(Issue::PackField(short_key(k)));
    }
    let name = clean_name(obj.get("name"));
    let name_ok = obj.get("name").is_some_and(Value::is_string)
        && !name.is_empty()
        && name.chars().count() <= MAX_NAME
        && !is_unsafe(&name);
    if !name_ok {
        issues.push(Issue::PackName);
    }

    let mut themes = Vec::new();
    match obj.get("themes") {
        None => {}
        Some(Value::Array(list)) if list.len() > MAX_THEMES => issues.push(Issue::PackTooMany {
            part: "themes",
            max: MAX_THEMES,
        }),
        Some(Value::Array(list)) => {
            for (i, v) in list.iter().enumerate() {
                // Format 2 only, as the window writes; a newer theme is left to the theme reader, which says so.
                let f = v.get("agora-theme").and_then(Value::as_f64);
                if !f.is_some_and(|f| f >= 2.0) {
                    issues.push(Issue::PackThemeFormat(i + 1));
                    continue;
                }
                match read_value(v) {
                    Read::Refused(problems) => {
                        issues.extend(problems.into_iter().map(|p| Issue::InPackTheme {
                            n: i + 1,
                            problem: Box::new(p),
                        }));
                    }
                    ok => themes.push(ok),
                }
            }
        }
        Some(_) => issues.push(Issue::PackPart("themes")),
    }
    let layouts = match obj.get("layouts") {
        None => 0,
        Some(Value::Array(list)) if list.len() > MAX_LAYOUTS => {
            issues.push(Issue::PackTooMany {
                part: "layouts",
                max: MAX_LAYOUTS,
            });
            0
        }
        Some(Value::Array(list)) if list.iter().all(Value::is_object) => list.len(),
        Some(_) => {
            issues.push(Issue::PackPart("layouts"));
            0
        }
    };
    let keys = match obj.get("keybindings") {
        None => 0,
        Some(Value::Object(k)) => match k.get("bindings") {
            Some(Value::Array(b)) => b.len(),
            _ => {
                issues.push(Issue::PackPart("keybindings"));
                0
            }
        },
        Some(_) => {
            issues.push(Issue::PackPart("keybindings"));
            0
        }
    };
    if !issues.is_empty() {
        return refused(issues);
    }
    if themes.is_empty() {
        return refused(vec![Issue::PackNoThemes { layouts, keys }]);
    }
    PackRead::Ok(Box::new(Pack {
        name,
        themes,
        layouts,
        keys,
    }))
}

impl Pack {
    /// Theme `index` (from 0), when the pack has it.
    pub fn theme(&self, index: usize) -> Option<&Theme> {
        self.themes.get(index).and_then(Read::theme)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::theme::file::read_text;
    use crate::theme::FloorItem;

    fn harbour() -> Value {
        serde_json::from_str(include_str!("testdata/harbour.json")).unwrap()
    }
    fn pack(extra: Value) -> Value {
        let mut p = serde_json::json!({
            "agora-pack": 1,
            "name": "Harbour set",
            "themes": [harbour()],
            "layouts": [{"name": "Review", "layout": {"version": 1, "root": {"type": "pane", "panel": "plugin.clock.face"}}}],
            "keybindings": {"version": 1, "bindings": [{"command": "palette.open", "keys": ["ctrl+p"]}]}
        });
        if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) {
            for (k, v) in e {
                o.insert(k.clone(), v.clone());
            }
        }
        p
    }
    fn issues(v: &Value) -> Vec<Issue> {
        read_pack(v)
            .problems()
            .iter()
            .map(|p| p.issue.clone())
            .collect()
    }

    #[test]
    fn a_pack_gives_its_themes_read_by_the_theme_rules_and_counts_what_it_does_not_use() {
        let PackRead::Ok(p) = read_pack(&pack(serde_json::json!({}))) else {
            panic!("refused");
        };
        assert_eq!(p.name, "Harbour set");
        assert_eq!(p.themes.len(), 1);
        // The same theme as the file on its own.
        assert_eq!(
            p.theme(0),
            read_text(include_str!("testdata/harbour.json")).theme()
        );
        assert_eq!(p.theme(1), None);
        assert_eq!((p.layouts, p.keys), (1, 1));
        assert!(is_pack(&pack(serde_json::json!({}))));
        assert!(!is_pack(&harbour()));
    }

    #[test]
    fn one_bad_theme_or_field_refuses_the_whole_pack_with_every_problem() {
        let mut bad = harbour();
        bad["dark"]["tokens"] = serde_json::json!({"danger": "#ff0000", "accent": "url(x)"});
        let got = issues(&pack(serde_json::json!({
            "script": "alert(1)",
            "name": "a<b",
            "themes": [harbour(), bad, {"agora-theme": 1, "name": "Old", "scheme": "dark", "tokens": {}}]
        })));
        assert_eq!(got[0], Issue::PackField("script".into()));
        assert_eq!(got[1], Issue::PackName);
        assert!(got.iter().any(|i| matches!(i, Issue::InPackTheme { n: 2, problem } if matches!(problem.issue, Issue::Floor { item: FloorItem::Danger, .. }))));
        assert!(got.iter().any(|i| matches!(i, Issue::InPackTheme { n: 2, problem } if matches!(problem.issue, Issue::Unsafe(_)))));
        assert_eq!(got.last(), Some(&Issue::PackThemeFormat(3)));
    }

    #[test]
    fn not_a_pack_a_newer_one_parts_of_the_wrong_form_and_one_without_themes() {
        assert_eq!(issues(&serde_json::json!([])), [Issue::Invalid]);
        assert_eq!(
            issues(&serde_json::json!({"agora-pack": 2, "name": "x"})),
            [Issue::Newer]
        );
        assert_eq!(
            issues(&pack(
                serde_json::json!({"themes": {}, "layouts": [1], "keybindings": []})
            )),
            [
                Issue::PackPart("themes"),
                Issue::PackPart("layouts"),
                Issue::PackPart("keybindings")
            ]
        );
        assert_eq!(
            issues(&pack(serde_json::json!({"themes": []}))),
            [Issue::PackNoThemes {
                layouts: 1,
                keys: 1
            }]
        );
        let many: Vec<Value> = (0..=MAX_THEMES).map(|_| harbour()).collect();
        assert_eq!(
            issues(&pack(serde_json::json!({ "themes": many }))),
            [Issue::PackTooMany {
                part: "themes",
                max: MAX_THEMES
            }]
        );
    }
}

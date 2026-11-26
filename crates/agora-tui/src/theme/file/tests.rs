//! The web's `format2.test.ts` cases, read by this client's parser: the same files are accepted and refused, for the same reasons.

use super::*;
use crate::theme::{base_colours, built_ins, colour::hex, Scheme, C_DENSITY, C_PANE_GAP};
use serde_json::json;

fn v1(tokens: Value) -> Value {
    json!({ "agora-theme": 1, "name": "Mine", "scheme": "dark", "tokens": tokens })
}
fn v2(extra: Value) -> Value {
    let mut v = json!({ "agora-theme": 2, "name": "Harbour" });
    if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
        for (k, x) in e {
            o.insert(k.clone(), x.clone());
        }
    }
    v
}
fn issues(r: &Read) -> Vec<Issue> {
    match r {
        Read::Refused(p) => p.iter().map(|p| p.issue.clone()).collect(),
        Read::Ok { .. } => Vec::new(),
    }
}
fn has(r: &Read, i: &Issue) -> bool {
    issues(r).contains(i)
}
fn field(f: &str) -> String {
    f.to_string()
}

/// A theme that uses every part of format 2: both schemes, inherits, regions, the terminal, syntax, chrome and text.
fn harbour() -> Value {
    v2(json!({
        "inherits": "slate",
        "dark": {
            "tokens": { "accent": "#5cc8c2" },
            "regions": { "sidebar": { "surface": "#0b1016", "text": "#e6edf3" }, "code-blocks": { "line": "#2a3541" } },
            "terminal": { "background": "#05080c", "foreground": "#d8e1ea", "red": "#ff6b6b", "bright-black": "#5c6773" },
            "syntax": { "keyword": "#c099ff" }
        },
        "light": { "tokens": { "accent": "#0b6e69", "on-accent": "#ffffff" } },
        "shape": { "radius-m": 8 },
        "text": { "font-mono": "'JetBrains Mono', monospace", "weight": 500, "line-height": 1.5, "ligatures": false },
        "chrome": { "pane-gap": 6, "tab-style": "pill", "density": "compact", "pane-borders": "always", "tab-bar": "bottom",
            "state-display": "word", "shadow": "none", "motion": "reduced", "line-width": 2 }
    }))
}

#[test]
fn format_1_reads_as_before_as_a_one_scheme_theme() {
    for b in built_ins() {
        let look = b.theme.look(b.scheme).unwrap();
        let tokens: Map<String, Value> = COLOUR_TOKENS
            .iter()
            .enumerate()
            .map(|(i, n)| (n.to_string(), json!(hex(look.colours[i]))))
            .chain(SHAPE_TOKENS.iter().enumerate().map(|(i, (n, _, _))| (n.to_string(), json!(b.theme.shape[i]))))
            .collect();
        let file = json!({ "agora-theme": 1, "name": "x", "scheme": b.scheme.word(), "tokens": tokens });
        let r = read_value(&file);
        let theme = r.theme().unwrap_or_else(|| panic!("{} {:?}", b.id, r));
        assert_eq!(theme.schemes(), vec![b.scheme], "{}", b.id);
        assert_eq!(theme.look(b.scheme).unwrap().colours, look.colours, "{}", b.id);
        assert_eq!(theme.chrome, Chrome::default());
    }
}

#[test]
fn format_1_lifts_a_base_text_size_under_16_px_with_a_warning() {
    for (size, lifted) in [(12, true), (15, true), (16, false), (24, false)] {
        let r = read_value(&v1(json!({ "text-16": size })));
        let Read::Ok { theme, warnings } = &r else {
            panic!("{size}: {r:?}");
        };
        assert_eq!(theme.shape[0], if lifted { "16px".to_string() } else { format!("{size}px") });
        let lift: Vec<&Issue> = warnings.iter().map(|w| &w.issue).filter(|i| matches!(i, Issue::TextLifted { .. })).collect();
        assert_eq!(lift.len(), usize::from(lifted), "{size}");
    }
    // A name with pieces format 2 refuses keeps the rest.
    let r = read_value(&json!({ "agora-theme": 1, "name": "Small; tight", "scheme": "dark" }));
    assert_eq!(r.theme().unwrap().name, "Small tight");
}

#[test]
fn format_1_says_why_a_floor_token_is_refused() {
    let r = read_value(&v1(json!({ "danger": "#ff0000", "accent": "#8b93ff" })));
    assert_eq!(
        issues(&r),
        vec![Issue::Floor {
            field: field("tokens.danger"),
            item: FloorItem::Danger
        }]
    );
}

#[test]
fn format_2_reads_both_schemes_inherits_regions_terminal_syntax_chrome_and_text() {
    let r = read_value(&harbour());
    let theme = r.theme().unwrap_or_else(|| panic!("{r:?}"));
    let slate = built_ins().into_iter().find(|b| b.id == "slate").unwrap();
    let dark = theme.look(Scheme::Dark).unwrap();
    let light = theme.look(Scheme::Light).unwrap();
    // Dark comes from Slate (inherited), Light from the default Light; the shape and fonts are shared.
    let mut expect = slate.theme.look(Scheme::Dark).unwrap().colours;
    expect[ACCENT] = [0x5c, 0xc8, 0xc2];
    assert_eq!(dark.colours, expect);
    let mut expect = base_colours(Scheme::Light);
    expect[ACCENT] = [0x0b, 0x6e, 0x69];
    expect[ON_ACCENT] = [255, 255, 255];
    assert_eq!(light.colours, expect);
    assert_eq!(theme.shape[4], "8px");
    assert_eq!(theme.fonts[1], "'JetBrains Mono', monospace");
    assert_eq!(dark.regions[0], [Some([0x0b, 0x10, 0x16]), Some([0xe6, 0xed, 0xf3]), None]);
    assert_eq!(dark.regions[9], [None, None, Some([0x2a, 0x35, 0x41])]);
    assert_eq!(dark.terminal[T_BACKGROUND], Some([0x05, 0x08, 0x0c]));
    assert_eq!(dark.terminal[T_ANSI + 1], Some([0xff, 0x6b, 0x6b]));
    assert_eq!(dark.syntax[0], Some([0xc0, 0x99, 0xff]));
    assert_eq!(light.regions, [[None; 3]; 10]);
    assert_eq!(theme.chrome.0[C_PANE_GAP], 6);
    assert_eq!(theme.chrome.word(C_DENSITY), Some("compact"));
    assert_eq!(theme.chrome.word(0), Some("always"));
    assert_eq!(theme.chrome.0[2], 2);
    assert_eq!(theme.text, TextStyle { weight: 500, line_height: Some(150), ligatures: false });
}

#[test]
fn a_theme_can_be_a_few_lines() {
    let r = read_value(&v2(json!({ "inherits": "sand", "chrome": { "tab-style": "filled" } })));
    let theme = r.theme().unwrap();
    let sand = built_ins().into_iter().find(|b| b.id == "sand").unwrap();
    assert_eq!(theme.schemes(), vec![Scheme::Light]);
    assert_eq!(theme.looks, sand.theme.looks);
    assert_eq!(theme.shape, sand.theme.shape);
}

#[test]
fn the_documented_sample_reads() {
    let r = read_text(include_str!("../testdata/harbour.json"));
    assert!(matches!(r, Read::Ok { .. }), "{r:?}");
}

#[test]
fn refuses_a_newer_format_unknown_fields_at_every_level_and_bad_values_all_at_once() {
    assert_eq!(issues(&read_value(&json!({ "agora-theme": 3, "name": "x" }))), vec![Issue::Newer]);
    assert_eq!(issues(&read_value(&v2(json!({})))), vec![Issue::NoLook]);
    let r = read_value(&v2(json!({
        "wallpaper": "x", "inherits": "nope",
        "dark": {
            "tokens": { "text-16": "18px", "surface-9": "#000000", "accent": "blue", "text-14": "15px" },
            "colours": {}, "regions": { "header": {}, "sidebar": { "glow": "#ffffff", "text": "white" } },
            "terminal": { "pink": "#ff00ff", "red": 12 }, "syntax": { "tag": "#ffffff" }
        },
        "light": "yes",
        "shape": { "font-ui": "Inter", "space-1": "9px", "text-12": "12px", "radius-q": 1 },
        "text": { "weight": 450, "line-height": 3, "ligatures": "no", "text-16": "16px", "kerning": true },
        "chrome": { "pane-gap": 17, "line-width": 1.5, "tab-style": "tabs", "tab-bar": "left", "glow": 1, "icons": "emoji", "motion": "OFF" }
    })));
    let bad = |f: &str, expect: Expect| Issue::BadField { field: field(f), expect };
    let moved = |f: &str, token: &str, to: &'static str| Issue::MovedToken { field: field(f), token: field(token), to };
    let expected = vec![
        Issue::UnknownField(field("wallpaper")),
        Issue::Inherits(field("nope")),
        bad("light", Expect::Group),
        moved("shape.font-ui", "font-ui", "text"),
        Issue::BadValue("space-1"),
        Issue::DerivedToken { token: field("text-12"), base: "text-16" },
        Issue::UnknownField(field("shape.radius-q")),
        bad("text.weight", Expect::List("300, 400, 500".into())),
        bad("text.line-height", Expect::LineHeight),
        bad("text.ligatures", Expect::Switch),
        moved("text.text-16", "text-16", "shape"),
        Issue::UnknownField(field("text.kerning")),
        bad("chrome.pane-gap", Expect::Size { min: 0, max: 16 }),
        bad("chrome.line-width", Expect::Size { min: 0, max: 2 }),
        bad("chrome.tab-style", Expect::List("underline, filled, pill".into())),
        bad("chrome.tab-bar", Expect::List("top, bottom".into())),
        Issue::UnknownField(field("chrome.glow")),
        bad("chrome.icons", Expect::List("lines".into())),
        bad("chrome.motion", Expect::List("system, reduced, off".into())),
        Issue::UnknownField(field("dark.colours")),
        moved("dark.tokens.text-16", "text-16", "shape"),
        Issue::UnknownToken(field("surface-9")),
        Issue::BadValue("accent"),
        Issue::DerivedToken { token: field("text-14"), base: "text-16" },
        Issue::UnknownField(field("dark.regions.header")),
        Issue::UnknownField(field("dark.regions.sidebar.glow")),
        bad("dark.regions.sidebar.text", Expect::Colour),
        Issue::UnknownField(field("dark.terminal.pink")),
        bad("dark.terminal.red", Expect::Colour),
        Issue::UnknownField(field("dark.syntax.tag")),
    ];
    let got = issues(&r);
    for e in &expected {
        assert!(got.contains(e), "missing {e:?} in {got:?}");
    }
    assert_eq!(got.len(), 30, "{got:?}");
}

#[test]
fn refuses_a_piece_that_could_become_css_wherever_it_is() {
    for piece in ["url(", "URL(", "@import", ";", "}", "expression(", "<"] {
        let cases = [
            ("name", v2(json!({ "name": format!("Night {piece}"), "inherits": "dark" }))),
            ("text.font-ui", v2(json!({ "inherits": "dark", "text": { "font-ui": format!("Inter{piece}") } }))),
            ("chrome.tab-style", v2(json!({ "inherits": "dark", "chrome": { "tab-style": format!("pill{piece}") } }))),
            ("dark.regions.cards.surface", v2(json!({ "dark": { "regions": { "cards": { "surface": format!("#111111{piece}") } } } }))),
            ("dark.terminal.red", v2(json!({ "dark": { "terminal": { "red": format!("{piece}x") } } }))),
            ("inherits", v2(json!({ "inherits": format!("dark{piece}") }))),
        ];
        for (at, file) in cases {
            let r = read_value(&file);
            assert!(r.theme().is_none(), "{piece} in {at}");
            assert!(has(&r, &Issue::Unsafe(field(at))), "{piece} in {at}: {:?}", issues(&r));
        }
    }
    // Keys that name a built-in property of every object are unknown keys like any other.
    for text in [
        r#"{"agora-theme":2,"name":"x","inherits":"dark","shape":{"constructor":1}}"#,
        r##"{"agora-theme":2,"name":"x","dark":{"tokens":{"constructor":"#000000","toString":1}}}"##,
        r##"{"agora-theme":1,"name":"x","scheme":"dark","tokens":{"constructor":"#000000"}}"##,
        r#"{"agora-theme":2,"name":"x","inherits":"dark","shape":{"__proto__":1},"chrome":{"__proto__":"x","constructor":"y"}}"#,
    ] {
        assert!(read_text(text).theme().is_none(), "{text}");
    }
    // A value refused as unsafe is not also judged as a bad value.
    assert_eq!(
        issues(&read_value(&v2(json!({ "dark": { "terminal": { "red": "url(x)" } } })))),
        vec![Issue::Unsafe(field("dark.terminal.red"))]
    );
}

#[test]
fn refuses_what_no_theme_can_change_saying_why() {
    let cases = [
        (json!({ "dark": { "tokens": { "danger": "#ff0000" } } }), "dark.tokens.danger", FloorItem::Danger),
        (json!({ "dark": { "tokens": { "diff-removed": "#ff0000" } } }), "dark.tokens.diff-removed", FloorItem::DiffRemoved),
        (json!({ "dark": { "tokens": { "on-danger": "#000000" } } }), "dark.tokens.on-danger", FloorItem::Danger),
        (json!({ "dark": { "regions": { "approval-card": { "surface": "#000000" } } } }), "dark.regions.approval-card", FloorItem::ApprovalCard),
        (json!({ "inherits": "dark", "chrome": { "focus-ring": "off" } }), "chrome.focus-ring", FloorItem::FocusRing),
        (json!({ "dark": { "regions": { "needs-you": { "surface": "#000000" } } } }), "dark.regions.needs-you", FloorItem::NeedsYou),
        (json!({ "inherits": "dark", "chrome": { "needs-you": "hidden" } }), "chrome.needs-you", FloorItem::NeedsYou),
        (json!({ "inherits": "dark", "plugin-marker": false }), "plugin-marker", FloorItem::PluginMarker),
        (json!({ "dark": { "approval": {} } }), "dark.approval", FloorItem::ApprovalCard),
        (json!({ "dark": { "regions": { "cards": { "focus-ring": "#000000" } } } }), "dark.regions.cards.focus-ring", FloorItem::FocusRing),
    ];
    for (extra, f, item) in cases {
        let r = read_value(&v2(extra));
        assert!(has(&r, &Issue::Floor { field: field(f), item }), "{f}: {:?}", issues(&r));
    }
    // Matched as a whole name: the colour of the "needs you" state and the accent are themeable.
    assert!(read_value(&v2(json!({ "dark": { "tokens": { "state-needs-you": "#ffb000", "accent": "#ff9f43" } } }))).theme().is_some());
    // No text under 12 px: in format 2 the base size is at least 16 px.
    assert_eq!(
        issues(&read_value(&v2(json!({ "inherits": "dark", "shape": { "text-16": 15 } })))),
        vec![Issue::TextFloor { field: field("shape.text-16") }]
    );
    assert!(read_value(&v2(json!({ "inherits": "dark", "shape": { "text-16": 16 } }))).theme().is_some());
}

#[test]
fn refuses_a_region_that_is_hard_to_read() {
    let r = read_value(&v2(json!({ "dark": { "regions": { "status-line": { "text": "#26262c" } } } })));
    let ratio = contrast([0x26, 0x26, 0x2c], base_colours(Scheme::Dark)[SURFACE_1]);
    assert_eq!(issues(&r), vec![Issue::RegionContrast { region: 3, fg: RegionFg::Text, ratio, min: 4.5 }]);
    let light = read_value(&v2(json!({ "dark": { "regions": { "sidebar": { "surface": "#d0d0d0", "text": "#000000" } } } })));
    let fgs: Vec<(RegionFg, f64)> = issues(&light)
        .into_iter()
        .filter_map(|i| match i {
            Issue::RegionContrast { fg, min, .. } => Some((fg, min)),
            _ => None,
        })
        .collect();
    for want in [(RegionFg::Token(TEXT_MUTED), 4.5), (RegionFg::Token(ACCENT), 4.5), (RegionFg::Token(8), 3.0), (RegionFg::Token(10), 3.0), (RegionFg::Token(9), 3.0)] {
        assert!(fgs.contains(&want), "{want:?} in {fgs:?}");
    }
    assert!(!fgs.contains(&(RegionFg::Text, 4.5)));
    // Every region's pair is asked, the person's entries on the accent's soft background included.
    for (ri, region) in REGIONS.iter().enumerate() {
        let r = read_value(&v2(json!({ "dark": { "regions": { *region: { "surface": "#ffffff", "text": "#fefefe" } } } })));
        assert!(
            issues(&r).iter().any(|i| matches!(i, Issue::RegionContrast { region, fg: RegionFg::Text, .. } if *region == ri)),
            "{region}"
        );
    }
    let tint = read_value(&v2(json!({ "dark": { "regions": { "person-entries": { "text": "#202020" } } } })));
    assert!(matches!(issues(&tint).as_slice(), [Issue::RegionContrast { region: 8, .. }]));
}

#[test]
fn refuses_terminal_text_that_is_hard_to_read_and_only_warns_about_the_16_colours() {
    assert!(matches!(
        issues(&read_value(&v2(json!({ "dark": { "terminal": { "background": "#202020", "foreground": "#505050" } } })))).as_slice(),
        [Issue::TerminalContrast { .. }]
    ));
    let r = read_value(&v2(json!({ "dark": { "terminal": { "blue": "#0a0a30", "cursor": "#101012", "black": "#000000" } } })));
    let Read::Ok { warnings, .. } = &r else {
        panic!("{r:?}");
    };
    // The cursor is the user's terminal's own here, so only the blue is warned about (the web also warns about the cursor).
    assert!(matches!(warnings.as_slice(), [Problem { issue: Issue::AnsiContrast { colour: 4, .. }, scheme: None }]));
    // The default palette is never warned about on the default background.
    for b in built_ins() {
        assert_eq!(judge(&b.theme), (vec![], vec![]), "{}", b.id);
    }
}

#[test]
fn says_which_scheme_a_problem_is_in_when_the_theme_has_both() {
    let both = read_value(&v2(json!({ "dark": { "tokens": { "text": "#202020" } }, "light": {} })));
    assert!(matches!(&both, Read::Refused(p) if !p.is_empty() && p.iter().all(|p| p.scheme == Some(Scheme::Dark))));
    let one = read_value(&v2(json!({ "dark": { "tokens": { "text": "#202020" } } })));
    assert!(matches!(&one, Read::Refused(p) if p.iter().all(|p| p.scheme.is_none())));
}

#[test]
fn caps_size_and_problem_count_and_never_panics_on_odd_input() {
    let big = format!(r#"{{"agora-theme":2,"name":"{}"}}"#, "x".repeat(usize::try_from(MAX_FILE_BYTES).unwrap()));
    assert_eq!(issues(&read_text(&big)), vec![Issue::TooLarge]);
    // Sixty unknown fields: fifty listed, then how many more.
    let many: Map<String, Value> = (0..60).map(|i| (format!("f{i}"), json!(1))).collect();
    let mut file = v2(json!({ "inherits": "dark" }));
    file.as_object_mut().unwrap().extend(many);
    let got = issues(&read_value(&file));
    assert_eq!(got.len(), MAX_ISSUES + 1);
    assert_eq!(got.last(), Some(&Issue::More(10)));
    for text in [
        "", "null", "[]", "1", "\"x\"", r#"{"agora-theme":"2"}"#, r#"{"agora-theme":2.5}"#, r#"{"agora-theme":0}"#,
        r#"{"agora-theme":1e300}"#, r##"{"agora-theme":2,"name":"x","dark":{"tokens":{"text":"#ééé"}}}"##,
        r#"{"agora-theme":2,"name":"x","a":{"b":{"c":{"d":{"e":{"f":{"g":1}}}}}}}"#,
        r#"{"agora-theme":2,"name":"","inherits":5,"dark":[1,2],"shape":{"text-16":"16.px","space-1":"1e1px"}}"#,
    ] {
        let _ = read_text(text);
    }
    assert_eq!(issues(&read_text(r#"{"agora-theme":2.0,"name":"x","inherits":"dark"}"#)), vec![]);
    assert_eq!(
        issues(&read_text(r#"{"agora-theme":2,"name":"x","a":{"b":{"c":{"d":{"e":{"f":{"g":"deep"}}}}}}}"#)),
        vec![Issue::Invalid]
    );
    // A name is counted as the web counts it: 60 UTF-16 units.
    let name = |s: &str| read_value(&v2(json!({ "name": s, "inherits": "dark" })));
    assert!(name(&"é".repeat(60)).theme().is_some());
    assert!(has(&name(&"😀".repeat(31)), &Issue::Name));
    assert!(has(&name("\u{202e}\u{0007}"), &Issue::Name));
}

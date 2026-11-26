//! The terminal client's theme list (`t`, or "Theme…" in the commands): every theme it can use, whole-screen, drawn in the theme the
//! keyboard is on so it shows at once; below the list, what that theme sets that this client does not draw ("not used in the
//! terminal client", ADR-0113 decision 4), its warnings, or every reason a file cannot be used. A pure function of what it is
//! given (render never mutates state); the words come from `locales/<lang>/tui.ftl`.

use agora_i18n::{key, Arg};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget, Wrap};
use ratatui::Frame;

use super::colour::shown_ratio;
use super::file::{Expect, Issue, Problem, Read, RegionFg};
use super::folder::{Choice, Entry, Source};
use super::map::Shell;
use super::{FloorItem, Scheme, Unused, CHROME, MAX_NAME, MIN_BASE_TEXT, SHAPE_TOKENS};
use crate::pane::ColorMode;
use crate::render::Text;

/// What one frame of the list shows.
pub struct ListView<'a> {
    pub entries: &'a [Entry],
    pub selected: usize,
    /// The theme in use, marked in the list.
    pub current: &'a Choice,
    /// The themes folder, as the person would type it.
    pub folder: &'a str,
    pub notice: &'a str,
    /// A request arrived meanwhile (the console's banner): drawn in the "needs you" style, never the theme's.
    pub banner: &'a str,
    pub colors: ColorMode,
    pub t: Text<'a>,
}

fn n(v: usize) -> Arg {
    Arg::Num(i64::try_from(v).unwrap_or(i64::MAX))
}

/// A scheme as a person reads it.
pub fn scheme_text(s: Scheme, t: Text<'_>) -> String {
    t(
        match s {
            Scheme::Dark => key!("tui-theme-scheme-dark"),
            Scheme::Light => key!("tui-theme-scheme-light"),
        },
        &[],
    )
}

/// A built-in theme's name.
pub fn built_in_name(id: &str, t: Text<'_>) -> String {
    let k = match id {
        "dark" => key!("tui-theme-name-dark"),
        "light" => key!("tui-theme-name-light"),
        "high-contrast" => key!("tui-theme-name-high-contrast"),
        "large-text" => key!("tui-theme-name-large-text"),
        "slate" => key!("tui-theme-name-slate"),
        _ => key!("tui-theme-name-sand"),
    };
    t(k, &[])
}

/// An entry's name in the list: a file's theme with both schemes names the scheme of its row.
pub fn entry_label(e: &Entry, t: Text<'_>) -> String {
    match &e.source {
        Source::Terminal => t(key!("tui-theme-terminal"), &[]),
        Source::BuiltIn(id) => built_in_name(id, t),
        Source::File {
            read: Read::Ok { theme, .. },
            ..
        } => {
            let shown: String = theme.name.chars().take(MAX_NAME).collect();
            if theme.schemes().len() > 1 {
                t(
                    key!("tui-theme-with-scheme"),
                    &[
                        ("name", Arg::Str(shown)),
                        ("scheme", Arg::Str(scheme_text(e.scheme, t))),
                    ],
                )
            } else {
                shown
            }
        }
        Source::File { name, .. } => t(
            key!("tui-theme-refused-row"),
            &[("file", Arg::Str(name.clone()))],
        ),
    }
}

fn token_label(name: &str, t: Text<'_>) -> String {
    let k = match name {
        "surface-0" => key!("tui-theme-token-surface-0"),
        "surface-1" => key!("tui-theme-token-surface-1"),
        "surface-2" => key!("tui-theme-token-surface-2"),
        "border" => key!("tui-theme-token-border"),
        "text" => key!("tui-theme-token-text"),
        "text-muted" => key!("tui-theme-token-text-muted"),
        "accent" => key!("tui-theme-token-accent"),
        "on-accent" => key!("tui-theme-token-on-accent"),
        "state-working" => key!("tui-theme-token-state-working"),
        "state-needs-you" => key!("tui-theme-token-state-needs-you"),
        "state-finished" => key!("tui-theme-token-state-finished"),
        "state-idle" => key!("tui-theme-token-state-idle"),
        "state-unknown" => key!("tui-theme-token-state-unknown"),
        "font-ui" => key!("tui-theme-token-font-ui"),
        "font-mono" => key!("tui-theme-token-font-mono"),
        "text-16" => key!("tui-theme-token-text-16"),
        "space-1" => key!("tui-theme-token-space-1"),
        "radius-xs" => key!("tui-theme-token-radius-xs"),
        "radius-s" => key!("tui-theme-token-radius-s"),
        "radius-m" => key!("tui-theme-token-radius-m"),
        _ => key!("tui-theme-token-radius-l"),
    };
    t(k, &[])
}

fn region_label(r: usize, t: Text<'_>) -> String {
    let k = match r {
        0 => key!("tui-theme-region-sidebar"),
        1 => key!("tui-theme-region-tab-bar"),
        2 => key!("tui-theme-region-pane-bar"),
        3 => key!("tui-theme-region-status-line"),
        4 => key!("tui-theme-region-cards"),
        5 => key!("tui-theme-region-inputs"),
        6 => key!("tui-theme-region-buttons"),
        7 => key!("tui-theme-region-agent-entries"),
        8 => key!("tui-theme-region-person-entries"),
        _ => key!("tui-theme-region-code-blocks"),
    };
    t(k, &[])
}

fn ansi_label(i: usize, t: Text<'_>) -> String {
    let k = match i {
        0 => key!("tui-theme-terminal-black"),
        1 => key!("tui-theme-terminal-red"),
        2 => key!("tui-theme-terminal-green"),
        3 => key!("tui-theme-terminal-yellow"),
        4 => key!("tui-theme-terminal-blue"),
        5 => key!("tui-theme-terminal-magenta"),
        6 => key!("tui-theme-terminal-cyan"),
        7 => key!("tui-theme-terminal-white"),
        8 => key!("tui-theme-terminal-bright-black"),
        9 => key!("tui-theme-terminal-bright-red"),
        10 => key!("tui-theme-terminal-bright-green"),
        11 => key!("tui-theme-terminal-bright-yellow"),
        12 => key!("tui-theme-terminal-bright-blue"),
        13 => key!("tui-theme-terminal-bright-magenta"),
        14 => key!("tui-theme-terminal-bright-cyan"),
        _ => key!("tui-theme-terminal-bright-white"),
    };
    t(k, &[])
}

fn floor_reason(item: FloorItem, t: Text<'_>) -> String {
    let k = match item {
        FloorItem::Danger => key!("tui-theme-floor-danger"),
        FloorItem::DiffRemoved => key!("tui-theme-floor-diff-removed"),
        FloorItem::ApprovalCard => key!("tui-theme-floor-approval-card"),
        FloorItem::FocusRing => key!("tui-theme-floor-focus-ring"),
        FloorItem::NeedsYou => key!("tui-theme-floor-needs-you"),
        FloorItem::PluginMarker => key!("tui-theme-floor-plugin-marker"),
    };
    t(k, &[])
}

fn pair(fg: String, bg: String, ratio: f64, min: f64, seen: bool, t: Text<'_>) -> String {
    t(
        if seen {
            key!("tui-theme-error-seen")
        } else {
            key!("tui-theme-error-contrast")
        },
        &[
            ("fg", Arg::Str(fg)),
            ("bg", Arg::Str(bg)),
            ("ratio", Arg::Str(shown_ratio(ratio))),
            ("min", Arg::Str(format!("{min}"))),
        ],
    )
}

/// A problem as a sentence (the web's wording); one in a theme with both schemes says which.
pub fn issue_text(p: &Problem, t: Text<'_>) -> String {
    let text = issue_sentence(&p.issue, t);
    match p.scheme {
        Some(s) => t(
            key!("tui-theme-issue-in-scheme"),
            &[
                ("scheme", Arg::Str(scheme_text(s, t))),
                ("text", Arg::Str(text)),
            ],
        ),
        None => text,
    }
}

fn issue_sentence(i: &Issue, t: Text<'_>) -> String {
    let s = |v: &str| Arg::Str(v.to_string());
    let terminal_bg = || t(key!("tui-theme-terminal-background"), &[]);
    match i {
        Issue::Json => t(key!("tui-theme-error-json"), &[]),
        Issue::Invalid => t(key!("tui-theme-error-invalid"), &[]),
        Issue::Newer => t(key!("tui-theme-error-newer"), &[]),
        Issue::Name => t(key!("tui-theme-error-name"), &[("max", n(MAX_NAME))]),
        Issue::Scheme => t(key!("tui-theme-error-scheme"), &[]),
        Issue::NoLook => t(key!("tui-theme-error-no-look"), &[]),
        Issue::UnknownField(f) => t(key!("tui-theme-error-unknown-field"), &[("field", s(f))]),
        Issue::UnknownToken(k) => t(key!("tui-theme-error-unknown-token"), &[("token", s(k))]),
        Issue::DerivedToken { token, base } => t(
            key!("tui-theme-error-derived-token"),
            &[("token", s(token)), ("base", s(base))],
        ),
        Issue::MovedToken { field, token, to } => t(
            key!("tui-theme-error-moved-token"),
            &[("field", s(field)), ("token", s(token)), ("to", s(to))],
        ),
        Issue::BadValue(token) => {
            let label = Arg::Str(token_label(token, t));
            match SHAPE_TOKENS.iter().find(|(n, _, _)| n == token) {
                Some((_, min, max)) => t(
                    key!("tui-theme-error-bad-size"),
                    &[
                        ("label", label),
                        ("token", s(token)),
                        ("min", Arg::Str(format!("{min}"))),
                        ("max", Arg::Str(format!("{max}"))),
                    ],
                ),
                None if token.starts_with("font-") => t(
                    key!("tui-theme-error-bad-font"),
                    &[("label", label), ("token", s(token))],
                ),
                None => t(
                    key!("tui-theme-error-bad-color"),
                    &[("label", label), ("token", s(token))],
                ),
            }
        }
        Issue::BadField { field, expect } => match expect {
            Expect::Group => t(key!("tui-theme-error-field-group"), &[("field", s(field))]),
            Expect::Colour => t(key!("tui-theme-error-field-color"), &[("field", s(field))]),
            Expect::Switch => t(key!("tui-theme-error-field-switch"), &[("field", s(field))]),
            Expect::LineHeight => t(
                key!("tui-theme-error-field-line-height"),
                &[("field", s(field)), ("min", s("1.2")), ("max", s("1.8"))],
            ),
            Expect::Size { min, max } => t(
                key!("tui-theme-error-field-size"),
                &[
                    ("field", s(field)),
                    ("min", n(usize::from(*min))),
                    ("max", n(usize::from(*max))),
                ],
            ),
            Expect::List(values) => t(
                key!("tui-theme-error-field-list"),
                &[("field", s(field)), ("values", s(values))],
            ),
        },
        Issue::Unsafe(f) => t(key!("tui-theme-error-unsafe"), &[("field", s(f))]),
        Issue::Floor { field, item } => t(
            key!("tui-theme-error-floor"),
            &[
                ("field", s(field)),
                ("reason", Arg::Str(floor_reason(*item, t))),
            ],
        ),
        Issue::TextFloor { field } => t(
            key!("tui-theme-error-text-floor"),
            &[
                ("field", s(field)),
                ("min", Arg::Str(format!("{MIN_BASE_TEXT}"))),
            ],
        ),
        Issue::TextLifted { from } => t(
            key!("tui-theme-warning-text-lifted"),
            &[
                ("from", s(from)),
                ("to", Arg::Str(format!("{MIN_BASE_TEXT}px"))),
            ],
        ),
        Issue::Inherits(id) => t(key!("tui-theme-error-inherits"), &[("id", s(id))]),
        Issue::Contrast { fg, bg, ratio, min } => pair(
            token_label(super::COLOUR_TOKENS[*fg], t),
            token_label(super::COLOUR_TOKENS[*bg], t),
            *ratio,
            *min,
            false,
            t,
        ),
        Issue::RegionContrast {
            region,
            fg,
            ratio,
            min,
        } => {
            let region_name = Arg::Str(region_label(*region, t));
            let fg = match fg {
                RegionFg::Text => t(
                    key!("tui-theme-region-part-text"),
                    &[("region", region_name.clone())],
                ),
                RegionFg::Token(i) => token_label(super::COLOUR_TOKENS[*i], t),
            };
            let bg = t(
                key!("tui-theme-region-part-surface"),
                &[("region", region_name)],
            );
            pair(fg, bg, *ratio, *min, *min < 4.5, t)
        }
        Issue::TerminalContrast { ratio } => pair(
            t(key!("tui-theme-terminal-foreground"), &[]),
            terminal_bg(),
            *ratio,
            4.5,
            false,
            t,
        ),
        Issue::AnsiContrast { colour, ratio } => {
            pair(ansi_label(*colour, t), terminal_bg(), *ratio, 3.0, true, t)
        }
        Issue::SelectionContrast { ratio } => pair(
            t(key!("tui-theme-terminal-selection"), &[]),
            terminal_bg(),
            *ratio,
            1.3,
            true,
            t,
        ),
        Issue::TooLarge => t(
            key!("tui-theme-error-too-large"),
            &[(
                "max",
                n(usize::try_from(super::file::MAX_FILE_BYTES / 1024).unwrap_or(256)),
            )],
        ),
        Issue::Unreadable => t(key!("tui-theme-error-unreadable"), &[]),
        Issue::More(count) => t(key!("tui-theme-error-more"), &[("count", n(*count))]),
    }
}

/// What a theme sets that this client does not draw, as a person reads it.
pub fn unused_text(u: Unused, t: Text<'_>) -> String {
    match u {
        Unused::Sizes => t(key!("tui-theme-unused-sizes"), &[]),
        Unused::Text => t(key!("tui-theme-unused-text"), &[]),
        Unused::Region(r) => t(
            key!("tui-theme-unused-region"),
            &[("region", Arg::Str(region_label(r, t)))],
        ),
        Unused::Cursor => t(key!("tui-theme-unused-cursor"), &[]),
        Unused::Syntax => t(key!("tui-theme-unused-syntax"), &[]),
        Unused::Chrome(i) => t(
            match CHROME.get(i).map_or("", |c| c.0) {
                "tab-bar" => key!("tui-theme-chrome-tab-bar"),
                "tab-style" => key!("tui-theme-chrome-tab-style"),
                "state-display" => key!("tui-theme-chrome-state-display"),
                "icons" => key!("tui-theme-chrome-icons"),
                "shadow" => key!("tui-theme-chrome-shadow"),
                _ => key!("tui-theme-chrome-motion"),
            },
            &[],
        ),
    }
}

/// The lines under the list about the selected entry.
pub fn details(v: &ListView<'_>) -> Vec<String> {
    let t = v.t;
    let mut out = Vec::new();
    let Some(e) = v.entries.get(v.selected) else {
        return out;
    };
    if let Source::File { name, .. } = &e.source {
        out.push(t(
            key!("tui-theme-in-file"),
            &[("file", Arg::Str(name.clone()))],
        ));
    }
    match &e.source {
        Source::Terminal => out.push(t(key!("tui-theme-terminal-hint"), &[])),
        Source::File {
            read: Read::Refused(problems),
            ..
        } => {
            out.push(t(key!("tui-theme-refused"), &[]));
            out.extend(problems.iter().map(|p| format!("  - {}", issue_text(p, t))));
        }
        _ => {
            out.push(t(
                key!("tui-theme-shows"),
                &[("scheme", Arg::Str(scheme_text(e.scheme, t)))],
            ));
            if let Some(theme) = e.theme() {
                let unused: Vec<String> = theme
                    .unused()
                    .into_iter()
                    .map(|u| unused_text(u, t))
                    .collect();
                if !unused.is_empty() {
                    out.push(t(
                        key!("tui-theme-unused"),
                        &[("list", Arg::Str(unused.join("; ")))],
                    ));
                }
            }
            if let Source::File {
                read: Read::Ok { warnings, .. },
                ..
            } = &e.source
            {
                if !warnings.is_empty() {
                    out.push(t(key!("tui-theme-warnings"), &[]));
                    out.extend(warnings.iter().map(|p| format!("  - {}", issue_text(p, t))));
                }
            }
        }
    }
    if v.colors == ColorMode::Mono {
        out.push(t(key!("tui-theme-no-colour"), &[]));
    }
    out
}

/// One frame of the list, drawn in the selected entry's theme (the user's terminal's colours for no theme or a file that cannot be
/// used).
pub fn draw(f: &mut Frame<'_>, v: &ListView<'_>) {
    let t = v.t;
    let area = f.area();
    let shell = v
        .entries
        .get(v.selected)
        .and_then(|e| e.theme().and_then(|th| Shell::new(&th, e.scheme, v.colors)));
    let (fill, dim, chosen) = match &shell {
        Some(s) => (s.styles.base.fill, s.styles.base.dim, s.styles.chosen),
        None => (
            Style::default(),
            Style::default().add_modifier(Modifier::DIM),
            Style::default().add_modifier(Modifier::REVERSED),
        ),
    };
    let strong = Style::default().add_modifier(Modifier::BOLD);
    let buf = f.buffer_mut();
    buf.set_style(area, fill);

    let width = area.width.max(1);
    let wrapped = |lines: &[String]| -> u16 {
        lines
            .iter()
            .map(|l| {
                let w = u16::try_from(Span::raw(l.as_str()).width()).unwrap_or(u16::MAX);
                w.div_ceil(width).max(1)
            })
            .sum()
    };
    let head = vec![
        Line::styled(t(key!("tui-theme-title"), &[]), strong),
        Line::styled(
            t(
                key!("tui-theme-folder"),
                &[("folder", Arg::Str(v.folder.to_string()))],
            ),
            dim,
        ),
    ];
    let about = details(v);
    // The details take at most half the screen; the list takes the rest, scrolled so the selected row stays in view.
    let body = area.height.saturating_sub(2 + 1);
    let about_h = wrapped(&about).min(body / 2);
    let list_h = body.saturating_sub(about_h + u16::from(about_h > 0));
    Paragraph::new(head).render(
        Rect {
            height: area.height.min(2),
            ..area
        },
        buf,
    );
    let room = usize::from(list_h).max(1);
    let top = v.selected.saturating_sub(room - 1);
    let rows: Vec<Line<'_>> = v
        .entries
        .iter()
        .enumerate()
        .skip(top)
        .take(room)
        .map(|(i, e)| {
            let mark = if e.choice().as_ref() == Some(v.current) {
                "\u{2022} "
            } else {
                "  "
            };
            let line = Line::raw(format!("{mark}{}", entry_label(e, t)));
            if i == v.selected {
                line.style(chosen)
            } else if e.choice().is_none() {
                line.style(dim)
            } else {
                line
            }
        })
        .collect();
    Paragraph::new(rows).render(
        Rect {
            y: area.y + 2,
            height: list_h,
            ..area
        },
        buf,
    );
    if about_h > 0 {
        Paragraph::new(about.into_iter().map(Line::raw).collect::<Vec<_>>())
            .wrap(Wrap { trim: false })
            .render(
                Rect {
                    y: area.y + 2 + list_h + 1,
                    height: about_h,
                    ..area
                },
                buf,
            );
    }
    let mut foot = Vec::new();
    if !v.banner.is_empty() {
        foot.push(Span::styled(
            format!("{}  ", v.banner),
            crate::app::attention_style(v.colors),
        ));
    }
    if !v.notice.is_empty() {
        foot.push(Span::styled(format!("{}  ", v.notice), strong));
    }
    foot.push(Span::styled(t(key!("tui-theme-keys"), &[]), dim));
    Line::from(foot).render(
        Rect {
            y: area.bottom().saturating_sub(1),
            height: area.height.min(1),
            ..area
        },
        buf,
    );
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::theme::built_ins;
    use crate::theme::file::read_text;
    use crate::theme::folder::entries;
    use agora_i18n::Catalog;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal as Tui;

    fn every_issue() -> Vec<Problem> {
        let mut out: Vec<Problem> = vec![
            Issue::Json,
            Issue::Invalid,
            Issue::Newer,
            Issue::Name,
            Issue::Scheme,
            Issue::NoLook,
            Issue::UnknownField("x".into()),
            Issue::UnknownToken("x".into()),
            Issue::DerivedToken {
                token: "text-12".into(),
                base: "text-16",
            },
            Issue::MovedToken {
                field: "shape.font-ui".into(),
                token: "font-ui".into(),
                to: "text",
            },
            Issue::BadValue("accent"),
            Issue::BadValue("font-ui"),
            Issue::BadValue("space-1"),
            Issue::BadField {
                field: "f".into(),
                expect: Expect::Group,
            },
            Issue::BadField {
                field: "f".into(),
                expect: Expect::Colour,
            },
            Issue::BadField {
                field: "f".into(),
                expect: Expect::Size { min: 0, max: 16 },
            },
            Issue::BadField {
                field: "f".into(),
                expect: Expect::List("a, b".into()),
            },
            Issue::BadField {
                field: "f".into(),
                expect: Expect::LineHeight,
            },
            Issue::BadField {
                field: "f".into(),
                expect: Expect::Switch,
            },
            Issue::Unsafe("name".into()),
            Issue::TextFloor {
                field: "shape.text-16".into(),
            },
            Issue::TextLifted {
                from: "12px".into(),
            },
            Issue::Inherits("nope".into()),
            Issue::Contrast {
                fg: 4,
                bg: 0,
                ratio: 1.6,
                min: 4.5,
            },
            Issue::RegionContrast {
                region: 3,
                fg: RegionFg::Text,
                ratio: 1.2,
                min: 4.5,
            },
            Issue::RegionContrast {
                region: 0,
                fg: RegionFg::Token(8),
                ratio: 1.2,
                min: 3.0,
            },
            Issue::TerminalContrast { ratio: 2.0 },
            Issue::AnsiContrast {
                colour: 4,
                ratio: 1.1,
            },
            Issue::SelectionContrast { ratio: 1.0 },
            Issue::TooLarge,
            Issue::Unreadable,
            Issue::More(3),
            Issue::More(1),
        ]
        .into_iter()
        .map(Problem::from)
        .collect();
        for item in [
            FloorItem::Danger,
            FloorItem::DiffRemoved,
            FloorItem::ApprovalCard,
            FloorItem::FocusRing,
            FloorItem::NeedsYou,
            FloorItem::PluginMarker,
        ] {
            out.push(
                Issue::Floor {
                    field: "x".into(),
                    item,
                }
                .into(),
            );
        }
        out.push(Problem {
            issue: Issue::Name,
            scheme: Some(Scheme::Light),
        });
        out
    }

    /// Every problem, every name of what is not used, and every built-in theme reads as a whole sentence in English and Turkish.
    #[test]
    fn every_message_is_a_whole_sentence_in_every_language() {
        for cat in [Catalog::english(), Catalog::turkish()] {
            let t = |id: &str, args: &[(&str, Arg)]| {
                cat.render(id, args)
                    .unwrap_or_else(|| panic!("missing {id}"))
            };
            let mut all: Vec<String> = every_issue().iter().map(|p| issue_text(p, &t)).collect();
            let unused = [Unused::Sizes, Unused::Text, Unused::Cursor, Unused::Syntax]
                .into_iter()
                .chain((0..10).map(Unused::Region))
                .chain((0..10).map(Unused::Chrome));
            all.extend(unused.map(|u| unused_text(u, &t)));
            all.extend(built_ins().iter().map(|b| built_in_name(b.id, &t)));
            for s in all {
                assert!(
                    !s.is_empty() && !s.contains("{ $") && !s.contains("tui-theme-"),
                    "{s}"
                );
            }
        }
    }

    fn screen(v: &ListView<'_>) -> (Vec<String>, ratatui::buffer::Buffer) {
        let mut tui = Tui::new(TestBackend::new(110, 30)).unwrap();
        tui.draw(|f| draw(f, v)).unwrap();
        let buf = tui.backend().buffer().clone();
        let lines = (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect();
        (lines, buf)
    }

    #[test]
    fn the_list_shows_every_theme_what_it_does_not_use_and_why_a_file_cannot_be_used() {
        let d = std::env::temp_dir().join(format!("agora-tui-theme-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("harbour.json"),
            include_str!("testdata/harbour.json"),
        )
        .unwrap();
        std::fs::write(d.join("broken.json"), r#"{"agora-theme": 2, "name": "B", "inherits": "dark", "chrome": {"needs-you": "off"}}"#).unwrap();
        let rows = entries(&d);
        let cat = Catalog::english();
        let t =
            |id: &str, args: &[(&str, Arg)]| cat.render(id, args).unwrap_or_else(|| id.to_string());
        let current = Choice::BuiltIn("slate".into());
        let at = |pred: &dyn Fn(&Entry) -> bool| rows.iter().position(pred).unwrap();
        let view = |selected: usize, colors: ColorMode| ListView {
            entries: &rows,
            selected,
            current: &current,
            folder: "~/.agora/themes",
            notice: "",
            banner: "",
            colors,
            t: &t,
        };
        // The list: no theme, the six built-in themes, the broken file, and both schemes of Harbour; the one in use is marked.
        let (lines, _) = screen(&view(0, ColorMode::Full));
        let all = lines.join("\n");
        for want in [
            "Your terminal's colours",
            "High contrast",
            "\u{2022} Slate",
            "broken.json · can't be used",
            "Harbour · dark",
            "Harbour · light",
            "~/.agora/themes",
        ] {
            assert!(all.contains(want), "{want} in\n{all}");
        }
        // Large text: its sizes are the terminal's own here, and the list says so.
        let large = at(&|e| matches!(e.source, Source::BuiltIn("large-text")));
        let all = screen(&view(large, ColorMode::Full)).0.join("\n");
        assert!(
            all.contains("Not used in the terminal client: text size, spacing and corners."),
            "{all}"
        );
        // Harbour: what it sets that the terminal client does not draw, by name.
        let harbour = at(&|e| {
            e.choice()
                == Some(Choice::File {
                    name: "harbour.json".into(),
                    scheme: Scheme::Dark,
                })
        });
        let all = screen(&view(harbour, ColorMode::Full)).0.join("\n");
        for want in [
            "corners",
            "fonts and text style",
            "colours of your",
            "code colours",
            "tab style",
        ] {
            assert!(all.contains(want), "{want} in\n{all}");
        }
        assert!(
            !all.contains("density") && !all.contains("Space between"),
            "{all}"
        );
        // The broken file: every reason, in words.
        let broken = at(&|e| {
            e.choice().is_none() && !matches!(e.source, Source::Terminal | Source::BuiltIn(_))
        });
        let all = screen(&view(broken, ColorMode::Full)).0.join("\n");
        assert!(
            all.contains("This file can't be used:")
                && all.contains("\"chrome.needs-you\" can't be set."),
            "{all}"
        );
        // The list is drawn in the theme the keyboard is on.
        let slate = at(&|e| matches!(e.source, Source::BuiltIn("slate")));
        let (_, buf) = screen(&view(slate, ColorMode::Full));
        assert_eq!(
            buf[(100, 5)].bg,
            ratatui::style::Color::Rgb(0x0f, 0x14, 0x19)
        );
        // Without colour it says why nothing changes.
        let all = screen(&view(slate, ColorMode::Mono)).0.join("\n");
        assert!(all.contains("NO_COLOR"), "{all}");
        let _ = std::fs::remove_dir_all(&d);
        let _ = read_text;
    }
}

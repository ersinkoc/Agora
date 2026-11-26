//! What an agent changed (the web's review screen, in the terminal): a unified diff drawn whole-screen with ratatui, coloured by
//! line, scrolled by the caller. Pure: the diff text in, a frame out.

use agora_i18n::{key, Arg};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use ratatui::Frame;

use crate::pane::ColorMode;
use crate::render::{plain, Text};

/// What kind of line of a unified diff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `diff --git`, `index`, `---`, `+++`: which file.
    File,
    /// `@@ … @@`: where in it.
    Hunk,
    Added,
    Removed,
    Context,
    /// A changed secret file's content is not shown (ADR-0080): this row says so, with its counts.
    Hidden,
}

/// The diff's lines with their kind, as plain text (an agent's file can hold control characters: they never reach the screen).
pub fn lines(diff: &str) -> Vec<(Kind, String)> {
    let mut in_hunk = false;
    diff.lines()
        .map(|l| {
            let kind = if l.starts_with("diff ") {
                in_hunk = false;
                Kind::File
            } else if l.starts_with("@@") {
                in_hunk = true;
                Kind::Hunk
            } else if !in_hunk
                && (l.starts_with("index ")
                    || l.starts_with("--- ")
                    || l.starts_with("+++ ")
                    || l.starts_with("new file")
                    || l.starts_with("deleted file")
                    || l.starts_with("rename "))
            {
                Kind::File
            } else if l.starts_with('+') {
                Kind::Added
            } else if l.starts_with('-') {
                Kind::Removed
            } else {
                Kind::Context
            };
            (kind, plain(&l.replace('\t', "    ")))
        })
        .collect()
}

/// The diff's lines, with a row saying "content hidden" after the header of each changed secret file (`secret_files` of `git.diff`):
/// the node sends such a file with no hunk, and its counts, which hold no value.
pub fn lines_with_secret(
    diff: &str,
    secret: &[agora_protocol::v1::SecretFileChange],
    t: Text<'_>,
) -> Vec<(Kind, String)> {
    let mut out = lines(diff);
    if secret.is_empty() {
        return out;
    }
    let mut i = 0;
    while i < out.len() {
        if let Some((_, l)) = out.get(i).filter(|(_, l)| l.starts_with("diff ")) {
            let hit = secret
                .iter()
                .find(|c| l.ends_with(&format!(" b/{}", plain(&c.path))));
            if let Some(c) = hit {
                // After the header lines of this file (up to the next file or hunk).
                let mut end = i + 1;
                while out
                    .get(end)
                    .is_some_and(|(k, l)| *k == Kind::File && !l.starts_with("diff "))
                {
                    end += 1;
                }
                let say = t(
                    key!("tui-review-hidden"),
                    &[
                        ("added", Arg::Num(i64::from(c.added))),
                        ("removed", Arg::Num(i64::from(c.removed))),
                    ],
                );
                out.insert(end, (Kind::Hidden, say));
                i = end;
            }
        }
        i += 1;
    }
    out
}

/// How many files the diff touches.
pub fn files(lines: &[(Kind, String)]) -> usize {
    lines
        .iter()
        .filter(|(k, l)| *k == Kind::File && l.starts_with("diff "))
        .count()
}

fn style(kind: Kind, colors: ColorMode) -> Style {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    if colors == ColorMode::Mono {
        return if kind == Kind::File {
            bold
        } else {
            Style::default()
        };
    }
    match kind {
        Kind::File => bold,
        Kind::Hunk => Style::default().fg(Color::Cyan),
        Kind::Added => Style::default().fg(Color::Green),
        Kind::Removed => Style::default().fg(Color::Red),
        Kind::Context => Style::default(),
        Kind::Hidden => Style::default().add_modifier(Modifier::ITALIC),
    }
}

/// The first line shown for a wanted `top` in a body of `room` rows: never past the last screenful.
pub fn clamp_top(top: usize, count: usize, room: usize) -> usize {
    top.min(count.saturating_sub(room))
}

/// Draws the review: a title (whose changes, how many files), the diff from `top`, and the keys.
pub fn draw(
    f: &mut Frame<'_>,
    title: &str,
    lines: &[(Kind, String)],
    top: usize,
    banner: &str,
    colors: ColorMode,
    t: Text<'_>,
) {
    let area = f.area();
    let buf = f.buffer_mut();
    let strong = Style::default().add_modifier(Modifier::BOLD);
    let dim = Style::default().add_modifier(Modifier::DIM);
    let head = Rect {
        height: area.height.min(1),
        ..area
    };
    Line::from(vec![
        Span::styled(title.to_string(), strong),
        Span::styled(
            format!(
                "  {}",
                t(
                    key!("tui-review-files"),
                    &[(
                        "count",
                        Arg::Num(i64::try_from(files(lines)).unwrap_or(i64::MAX))
                    )]
                )
            ),
            dim,
        ),
    ])
    .render(head, buf);
    let body = Rect {
        y: area.y + head.height,
        height: area.height.saturating_sub(head.height + 1),
        ..area
    };
    let shown: Vec<Line<'_>> = if lines.is_empty() {
        vec![Line::styled(t(key!("tui-review-none"), &[]), dim)]
    } else {
        lines
            .iter()
            .skip(top)
            .take(usize::from(body.height))
            .map(|(k, l)| Line::styled(l.clone(), style(*k, colors)))
            .collect()
    };
    Paragraph::new(shown).render(body, buf);
    let foot = Rect {
        y: area.bottom().saturating_sub(1),
        height: area.height.min(1),
        ..area
    };
    let at = if lines.is_empty() {
        String::new()
    } else {
        format!(
            "{}–{}/{}  ",
            top + 1,
            (top + usize::from(body.height)).min(lines.len()),
            lines.len()
        )
    };
    // Something that wants the user meanwhile (a new request) leads the footer.
    let mut spans = Vec::new();
    if !banner.is_empty() {
        spans.push(Span::styled(format!("{banner}  "), strong));
    }
    spans.push(Span::styled(at, dim));
    spans.push(Span::styled(t(key!("tui-review-keys"), &[]), dim));
    Line::from(spans).render(foot, buf);
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const DIFF: &str = "diff --git a/src/a.rs b/src/a.rs\nindex 1..2 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,2 +1,2 @@\n fn a() {}\n-old\x1b[2J\n+new\tline\n";

    #[test]
    fn a_diff_is_read_line_by_line_with_control_characters_removed() {
        let l = lines(DIFF);
        let kinds: Vec<Kind> = l.iter().map(|(k, _)| *k).collect();
        assert_eq!(
            kinds,
            [
                Kind::File,
                Kind::File,
                Kind::File,
                Kind::File,
                Kind::Hunk,
                Kind::Context,
                Kind::Removed,
                Kind::Added
            ]
        );
        assert_eq!(files(&l), 1);
        assert!(!l.iter().any(|(_, s)| s.contains('\x1b')), "{l:?}");
        assert_eq!(l[7].1, "+new    line");
    }

    #[test]
    fn a_changed_secret_file_shows_a_content_hidden_row_with_its_counts_and_no_content() {
        use agora_protocol::v1::SecretFileChange;
        let diff = "diff --git a/.env b/.env\nindex 1..2 100644\ndiff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-old\n+new\n";
        let text = |id: &str, a: &[(&str, Arg)]| {
            format!(
                "{id}{}",
                a.iter()
                    .map(|(k, v)| match v {
                        Arg::Num(n) => format!(" {k}={n}"),
                        _ => String::new(),
                    })
                    .collect::<String>()
            )
        };
        let secret = [SecretFileChange {
            path: ".env".into(),
            added: 2,
            removed: 1,
        }];
        let l = lines_with_secret(diff, &secret, &text);
        assert_eq!(
            l[2],
            (Kind::Hidden, "tui-review-hidden added=2 removed=1".into())
        );
        assert_eq!(l[3].1, "diff --git a/src/a.rs b/src/a.rs");
        assert_eq!(files(&l), 2);
        // Negative control: with no secret file the lines are exactly the plain reading.
        assert_eq!(lines_with_secret(diff, &[], &text), lines(diff));
        assert_eq!(l.iter().filter(|(k, _)| *k == Kind::Hidden).count(), 1);
    }

    #[test]
    fn scrolling_stops_at_the_last_screenful() {
        assert_eq!(clamp_top(0, 100, 20), 0);
        assert_eq!(clamp_top(95, 100, 20), 80);
        assert_eq!(clamp_top(5, 10, 20), 0);
    }

    #[test]
    fn the_review_is_drawn_with_its_title_lines_and_keys() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        let text = |id: &str, _: &[(&str, Arg)]| id.to_string();
        let l = lines(DIFF);
        let mut term = Terminal::new(TestBackend::new(40, 6)).unwrap();
        term.draw(|f| draw(f, "codex · amber", &l, 4, "", ColorMode::Full, &text))
            .unwrap();
        let buf = term.backend().buffer().clone();
        let row = |y: u16| {
            (0..40)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        };
        assert!(
            row(0).starts_with("codex · amber  tui-review-files"),
            "{}",
            row(0)
        );
        assert!(row(1).starts_with("@@ -1,2 +1,2 @@"), "{}", row(1));
        assert!(row(3).starts_with("-old"), "{}", row(3));
        assert_eq!(buf[(0, 3)].fg, Color::Red);
        assert_eq!(buf[(0, 4)].fg, Color::Green);
        assert!(row(5).starts_with("5–8/8  tui-review-keys"), "{}", row(5));
    }
}

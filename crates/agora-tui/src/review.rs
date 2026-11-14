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
}

/// The diff's lines with their kind, as plain text (an agent's file can hold control characters: they never reach the screen).
pub fn lines(diff: &str) -> Vec<(Kind, String)> {
    diff.lines()
        .map(|l| {
            let kind = if l.starts_with("diff ")
                || l.starts_with("index ")
                || l.starts_with("--- ")
                || l.starts_with("+++ ")
                || l.starts_with("new file")
                || l.starts_with("deleted file")
                || l.starts_with("rename ")
            {
                Kind::File
            } else if l.starts_with("@@") {
                Kind::Hunk
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
    Line::from(vec![
        Span::styled(at, dim),
        Span::styled(t(key!("tui-review-keys"), &[]), dim),
    ])
    .render(foot, buf);
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
        term.draw(|f| draw(f, "codex · amber", &l, 4, ColorMode::Full, &text))
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

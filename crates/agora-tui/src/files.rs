//! An agent's files (the web's file browser, in the terminal): a folder's entries to choose from, and a file's text to read. Pure:
//! what the node answered in, a frame out; the caller asks the node and reads the keys.

use agora_i18n::{key, Arg};
use agora_protocol::v1::FileEntry;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use ratatui::Frame;

use crate::render::{plain, Text};

/// A folder's entries as shown: folders first, then files, each by name.
pub fn sorted(mut entries: Vec<FileEntry>) -> Vec<FileEntry> {
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    entries
}

/// `name` inside the relative folder `path` ("" is the top).
pub fn child(path: &str, name: &str) -> String {
    if path.is_empty() {
        name.to_string()
    } else {
        format!("{}/{name}", path.trim_end_matches('/'))
    }
}

/// The folder above the relative `path`; "" for the top.
pub fn parent(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit_once('/')
        .map_or_else(String::new, |(p, _)| p.to_string())
}

/// A file's bytes as shown: its text by line (control characters removed, tabs expanded, at most `MAX_LINES`), or that it is binary
/// (a NUL in its first 8 KiB, as git decides) and how big.
pub enum Content {
    Text(Vec<String>),
    Binary(usize),
}

const MAX_LINES: usize = 20_000;

pub fn content(data: &[u8]) -> Content {
    if data.iter().take(8192).any(|b| *b == 0) {
        return Content::Binary(data.len());
    }
    Content::Text(
        String::from_utf8_lossy(data)
            .lines()
            .take(MAX_LINES)
            .map(|l| plain(&l.replace('\t', "    ")))
            .collect(),
    )
}

fn size(n: u64) -> String {
    match n {
        n if n >= 1 << 20 => format!("{:.1} MB", n as f64 / f64::from(1 << 20)),
        n if n >= 1 << 10 => format!("{:.1} kB", n as f64 / f64::from(1 << 10)),
        n => format!("{n} B"),
    }
}

fn frame_parts(f: &mut Frame<'_>, head: Line<'_>, body: Vec<Line<'_>>, foot: Line<'_>) {
    let area = f.area();
    let buf = f.buffer_mut();
    head.render(
        Rect {
            height: area.height.min(1),
            ..area
        },
        buf,
    );
    Paragraph::new(body).render(
        Rect {
            y: area.y + area.height.min(1),
            height: area.height.saturating_sub(2),
            ..area
        },
        buf,
    );
    foot.render(
        Rect {
            y: area.bottom().saturating_sub(1),
            height: area.height.min(1),
            ..area
        },
        buf,
    );
}

/// The first entry or line shown so that `at` stays in a body of `room` rows.
pub fn top_for(at: usize, top: usize, room: usize) -> usize {
    if at < top {
        at
    } else if at >= top + room {
        at + 1 - room
    } else {
        top
    }
}

/// A folder of the agent's: whose and where, its entries (folders with a trailing `/`, files with their size) with `selected`
/// chosen, and the keys. `notice` (a failed read) is said above the keys.
#[allow(clippy::too_many_arguments)] // one screen: what it shows
pub fn draw_listing(
    f: &mut Frame<'_>,
    title: &str,
    path: &str,
    entries: &[FileEntry],
    selected: usize,
    notice: &str,
    t: Text<'_>,
) {
    let strong = Style::default().add_modifier(Modifier::BOLD);
    let dim = Style::default().add_modifier(Modifier::DIM);
    let chosen = Style::default().add_modifier(Modifier::REVERSED);
    let room = usize::from(f.area().height.saturating_sub(2)).max(1);
    let width = usize::from(f.area().width);
    let top = top_for(
        selected,
        selected.saturating_sub(room - 1).min(selected),
        room,
    );
    let head = Line::from(vec![
        Span::styled(title.to_string(), strong),
        Span::styled(format!("  /{}", plain(path)), dim),
    ]);
    let body: Vec<Line<'_>> = if entries.is_empty() {
        vec![Line::styled(t(key!("tui-files-empty"), &[]), dim)]
    } else {
        entries
            .iter()
            .enumerate()
            .skip(top)
            .take(room)
            .map(|(i, e)| {
                let name = if e.is_dir {
                    format!("{}/", plain(&e.name))
                } else {
                    plain(&e.name)
                };
                let right = if e.is_dir {
                    String::new()
                } else {
                    size(e.size_bytes)
                };
                let gap = width
                    .saturating_sub(name.chars().count() + right.chars().count() + 2)
                    .max(1);
                let line = Line::from(vec![
                    Span::raw(format!(" {name}")),
                    Span::raw(" ".repeat(gap)),
                    Span::styled(right, dim),
                ]);
                if i == selected {
                    line.style(chosen)
                } else {
                    line
                }
            })
            .collect()
    };
    let mut foot = Vec::new();
    if !notice.is_empty() {
        foot.push(Span::styled(format!("{notice}  "), strong));
    }
    foot.push(Span::styled(t(key!("tui-files-keys"), &[]), dim));
    frame_parts(f, head, body, Line::from(foot));
}

/// A file of the agent's: its path, its lines from `top` (or that it is binary), and the keys.
pub fn draw_file(
    f: &mut Frame<'_>,
    title: &str,
    path: &str,
    content: &Content,
    top: usize,
    t: Text<'_>,
) {
    let strong = Style::default().add_modifier(Modifier::BOLD);
    let dim = Style::default().add_modifier(Modifier::DIM);
    let room = usize::from(f.area().height.saturating_sub(2)).max(1);
    let head = Line::from(vec![
        Span::styled(title.to_string(), strong),
        Span::styled(format!("  /{}", plain(path)), dim),
    ]);
    let (body, at) = match content {
        Content::Binary(n) => (
            vec![Line::styled(
                t(
                    key!("tui-files-binary"),
                    &[(
                        "size",
                        Arg::Str(size(u64::try_from(*n).unwrap_or(u64::MAX))),
                    )],
                ),
                dim,
            )],
            String::new(),
        ),
        Content::Text(lines) => (
            lines
                .iter()
                .skip(top)
                .take(room)
                .map(|l| Line::raw(l.clone()))
                .collect(),
            if lines.is_empty() {
                String::new()
            } else {
                format!(
                    "{}–{}/{}  ",
                    top + 1,
                    (top + room).min(lines.len()),
                    lines.len()
                )
            },
        ),
    };
    let foot = Line::from(vec![
        Span::styled(at, dim),
        Span::styled(t(key!("tui-files-file-keys"), &[]), dim),
    ]);
    frame_parts(f, head, body, foot);
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn entry(name: &str, is_dir: bool, size: u64) -> FileEntry {
        FileEntry {
            name: name.into(),
            is_dir,
            size_bytes: size,
        }
    }

    fn text(id: &str, args: &[(&str, Arg)]) -> String {
        let mut s = id.to_string();
        for (k, v) in args {
            if let Arg::Str(v) = v {
                s.push_str(&format!(" {k}={v}"));
            }
        }
        s
    }

    #[test]
    fn folders_come_first_and_paths_join_and_go_up() {
        let s = sorted(vec![
            entry("b.rs", false, 3),
            entry("src", true, 0),
            entry("a.rs", false, 1),
        ]);
        let names: Vec<&str> = s.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["src", "a.rs", "b.rs"]);
        assert_eq!(child("", "src"), "src");
        assert_eq!(child("src/", "main.rs"), "src/main.rs");
        assert_eq!(parent("src/agora/main.rs"), "src/agora");
        assert_eq!(parent("src"), "");
        assert_eq!(parent(""), "");
    }

    #[test]
    fn a_file_is_text_without_control_characters_or_binary() {
        match content(b"fn main() {\n\tprint!(\"\x1b[2Jhi\");\n}\n") {
            Content::Text(l) => {
                assert_eq!(l.len(), 3);
                assert!(!l[1].contains('\x1b'), "{l:?}");
                assert!(l[1].starts_with("    print!"), "{l:?}");
            }
            Content::Binary(_) => panic!("text"),
        }
        assert!(matches!(content(b"PNG\0\x01\x02"), Content::Binary(6)));
    }

    #[test]
    fn the_listing_keeps_the_chosen_entry_in_view() {
        assert_eq!(top_for(0, 0, 10), 0);
        assert_eq!(top_for(12, 0, 10), 3);
        assert_eq!(top_for(2, 5, 10), 2);
        let entries: Vec<FileEntry> = (0..30)
            .map(|i| entry(&format!("f{i:02}"), false, i))
            .collect();
        let mut term = Terminal::new(TestBackend::new(30, 8)).unwrap();
        term.draw(|f| draw_listing(f, "shell · teal", "src", &entries, 20, "", &text))
            .unwrap();
        let buf = term.backend().buffer().clone();
        let row = |y: u16| {
            (0..30)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        };
        assert!(row(0).starts_with("shell · teal  /src"), "{}", row(0));
        // Six rows of room: the chosen f20 is the last of them, drawn chosen.
        assert!(row(6).starts_with(" f20"), "{}", row(6));
        assert!(buf[(0, 6)].modifier.contains(Modifier::REVERSED));
        assert!(row(7).starts_with("tui-files-keys"), "{}", row(7));
    }
}

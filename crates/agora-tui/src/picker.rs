//! The folder picker of "start an agent in a folder" (ADR-0053): one folder of a machine at a time, its subfolders to choose from.
//! A pure function of what the machine answered and where the keyboard points (render never mutates state, AGENTS.md); it returns
//! the bytes that paint one whole frame.

use std::fmt::Write as _;

use agora_i18n::{key, Arg};
use agora_protocol::v1::DirectoryEntries;

use crate::render::Text;

/// What one frame of the picker shows.
pub struct Picker<'a> {
    /// The machine's name, as the tree shows it.
    pub machine: &'a str,
    pub folder: &'a DirectoryEntries,
    /// The subfolder the keyboard points at.
    pub selected: usize,
    pub notice: &'a str,
    pub color: bool,
    pub cols: u16,
    pub rows: u16,
}

/// A subfolder's path on the machine: the machine's own separator (a Windows machine answers `C:\…`), whatever this terminal runs on.
pub fn child(folder: &str, name: &str) -> String {
    let sep = if folder.contains('\\') && !folder.contains('/') {
        '\\'
    } else {
        '/'
    };
    if folder.ends_with(sep) {
        format!("{folder}{name}")
    } else {
        format!("{folder}{sep}{name}")
    }
}

/// Text cut to `width` characters (folder names are shown as plain text: no control characters reach the terminal).
fn fit(text: &str, width: usize) -> String {
    let clean: String = text.chars().filter(|c| !c.is_control()).collect();
    if clean.chars().count() <= width {
        return clean;
    }
    let mut out: String = clean.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub fn frame(p: &Picker<'_>, t: Text<'_>) -> String {
    let (cols, rows) = (usize::from(p.cols.max(20)), usize::from(p.rows.max(6)));
    let (bold, dim, rev, reset) = if p.color {
        ("\x1b[1m", "\x1b[2m", "\x1b[7m", "\x1b[0m")
    } else {
        ("", "", "", "")
    };
    let mut lines = vec![
        format!(
            "{bold}{}{reset}",
            fit(
                &t(
                    key!("tui-folder-title"),
                    &[("machine", Arg::Str(p.machine.to_string()))]
                ),
                cols
            )
        ),
        format!("{dim}{}{reset}", fit(&p.folder.path, cols)),
        String::new(),
    ];
    // The list fills what the title, the path, a blank line and the status line leave; the selected folder stays in view.
    let room = rows.saturating_sub(4).max(1);
    let entries = &p.folder.entries;
    if entries.is_empty() {
        lines.push(format!(
            "  {dim}{}{reset}",
            t(key!("tui-folder-empty"), &[])
        ));
    }
    let first = p
        .selected
        .saturating_sub(room.saturating_sub(1))
        .min(entries.len().saturating_sub(room));
    for (i, e) in entries.iter().enumerate().skip(first).take(room) {
        let git = if e.is_git {
            format!("  {dim}{}{reset}", t(key!("tui-folder-git"), &[]))
        } else {
            String::new()
        };
        let name = fit(&e.name, cols.saturating_sub(10));
        if i == p.selected {
            if p.color {
                lines.push(format!("{rev} ▸ {name} {reset}{git}"));
            } else {
                lines.push(format!(">  {name}{git}"));
            }
        } else {
            lines.push(format!("   {name}{git}"));
        }
    }
    if p.folder.truncated {
        lines.push(format!(
            "  {dim}{}{reset}",
            t(
                key!("tui-folder-more"),
                &[("n", Arg::Num(entries.len() as i64))]
            )
        ));
    }
    let status = if p.notice.is_empty() {
        t(key!("tui-folder-keys"), &[])
    } else {
        p.notice.to_string()
    };
    let mut out = String::from("\x1b[?25l\x1b[H\x1b[2J");
    for (r, l) in lines.iter().take(rows - 1).enumerate() {
        let _ = write!(out, "\x1b[{};1H{l}", r + 1);
    }
    let _ = write!(
        out,
        "\x1b[{rows};1H{rev}{}{reset}\x1b[K",
        fit(&format!(" {status}"), cols)
    );
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use agora_protocol::v1::DirectoryEntry;
    use agora_vt::{AlacrittyTerminal, Size, Terminal};

    fn text(id: &str, args: &[(&str, Arg)]) -> String {
        let mut s = id.to_string();
        for (k, v) in args {
            let v = match v {
                Arg::Num(n) => n.to_string(),
                Arg::Str(s) => s.clone(),
                #[allow(unreachable_patterns)]
                _ => String::new(),
            };
            let _ = write!(s, " {k}={v}");
        }
        s
    }

    fn folder(n: usize, truncated: bool) -> DirectoryEntries {
        DirectoryEntries {
            path: "/Users/me/code".into(),
            parent: "/Users/me".into(),
            entries: (0..n)
                .map(|i| DirectoryEntry {
                    name: format!("p{i:02}"),
                    is_git: i % 2 == 0,
                })
                .collect(),
            truncated,
        }
    }

    fn rows(f: &str, rows: u16) -> Vec<String> {
        let mut t = AlacrittyTerminal::new(Size { cols: 80, rows });
        t.feed(f.as_bytes());
        (0..rows).map(|r| t.line(r)).collect()
    }

    #[test]
    fn a_child_uses_the_machines_own_separator() {
        assert_eq!(child("/Users/me", "code"), "/Users/me/code");
        assert_eq!(child("/", "Users"), "/Users");
        assert_eq!(child(r"C:\Users\me", "code"), r"C:\Users\me\code");
        assert_eq!(child(r"C:\", "Users"), r"C:\Users");
    }

    #[test]
    fn the_picker_shows_the_folder_its_subfolders_and_keeps_the_selection_in_view() {
        let f = folder(30, true);
        let p = Picker {
            machine: "mac2",
            folder: &f,
            selected: 25,
            notice: "",
            color: false,
            cols: 80,
            rows: 12,
        };
        let r = rows(&frame(&p, &text), 12);
        assert!(r[0].contains("tui-folder-title machine=mac2"), "{r:#?}");
        assert_eq!(r[1], "/Users/me/code");
        // Eight list rows (12 minus title, path, blank and status): the selected one is in them, marked, git folders tagged.
        assert!(r.iter().any(|l| l.starts_with(">  p25")), "{r:#?}");
        assert!(r.iter().any(|l| l == "   p24  tui-folder-git"), "{r:#?}");
        assert!(!r.iter().any(|l| l.contains("p00")), "scrolled: {r:#?}");
        assert!(r[11].contains("tui-folder-keys"), "{r:#?}");
    }

    #[test]
    fn an_empty_folder_says_so_and_a_long_one_says_there_are_more() {
        let f = folder(0, false);
        let p = Picker {
            machine: "m",
            folder: &f,
            selected: 0,
            notice: "",
            color: true,
            cols: 80,
            rows: 10,
        };
        assert!(rows(&frame(&p, &text), 10)
            .iter()
            .any(|l| l.contains("tui-folder-empty")));
        let f = folder(3, true);
        let p = Picker {
            machine: "m",
            folder: &f,
            selected: 0,
            notice: "",
            color: true,
            cols: 80,
            rows: 10,
        };
        assert!(rows(&frame(&p, &text), 10)
            .iter()
            .any(|l| l.contains("tui-folder-more n=3")));
    }
}

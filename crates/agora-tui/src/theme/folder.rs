//! Where the terminal client's themes come from: the built-in themes, and theme files in its themes folder (`themes` in the Agora
//! folder: `$AGORA_HOME/themes`, else `~/.agora/themes`, beside the client's device key), and the one it uses, kept in
//! `tui-theme` there. The caller passes both paths, so tests never touch the real folder.
//!
//! A file is read only when it is at most [`MAX_FILE_BYTES`](super::file::MAX_FILE_BYTES) (asked of the file system first), and
//! only `*.json` files directly in the folder are themes. Live reload: the caller asks for [`stamp`] of the file in use now and then
//! (its size and modification time, no reading), and reads it again when that changed.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::file::{read_text, Issue, Read, MAX_FILE_BYTES};
use super::{built_ins, Scheme, Theme};

/// The themes folder's name in the Agora folder.
pub const FOLDER: &str = "themes";
/// The file in the Agora folder that keeps the theme in use.
pub const CHOICE_FILE: &str = "tui-theme";
/// The most theme files listed from the folder (sorted by name).
pub const MAX_FILES: usize = 200;

/// The theme the client uses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Choice {
    /// No theme: the user's terminal's own colours, as the client draws without one.
    #[default]
    Terminal,
    /// A built-in theme, by id.
    BuiltIn(String),
    /// One scheme of a theme file in the folder, by file name.
    File { name: String, scheme: Scheme },
}

impl Choice {
    /// The choice kept in [`CHOICE_FILE`]: one line, `terminal`, `built-in <id>` or `file <dark|light> <file name>`. Anything else is
    /// no theme.
    pub fn parse(text: &str) -> Choice {
        let line = text.lines().next().unwrap_or("").trim();
        if let Some(id) = line.strip_prefix("built-in ") {
            if built_ins().iter().any(|b| b.id == id) {
                return Choice::BuiltIn(id.to_string());
            }
        }
        if let Some(rest) = line.strip_prefix("file ") {
            let scheme = match rest.split_once(' ') {
                Some(("dark", name)) => Some((Scheme::Dark, name)),
                Some(("light", name)) => Some((Scheme::Light, name)),
                _ => None,
            };
            if let Some((scheme, name)) = scheme.filter(|(_, n)| is_theme_file_name(n)) {
                return Choice::File {
                    name: name.to_string(),
                    scheme,
                };
            }
        }
        Choice::Terminal
    }

    pub fn line(&self) -> String {
        match self {
            Choice::Terminal => "terminal\n".to_string(),
            Choice::BuiltIn(id) => format!("built-in {id}\n"),
            Choice::File { name, scheme } => format!("file {} {name}\n", scheme.word()),
        }
    }
}

/// A file name the folder lists as a theme: `*.json`, one name (no folder in it), not hidden.
fn is_theme_file_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\'])
        && name.to_ascii_lowercase().ends_with(".json")
}

/// Reads one theme file: refused as too large from its size alone, before anything is read.
pub fn load_file(path: &Path) -> Read {
    let too_large = || Read::Refused(vec![Issue::TooLarge.into()]);
    match std::fs::metadata(path) {
        Ok(m) if !m.is_file() => return Read::Refused(vec![Issue::Unreadable.into()]),
        Ok(m) if m.len() > MAX_FILE_BYTES => return too_large(),
        Ok(_) => {}
        Err(_) => return Read::Refused(vec![Issue::Unreadable.into()]),
    }
    match std::fs::read(path) {
        Ok(bytes) if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_FILE_BYTES => too_large(),
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => read_text(&text),
            Err(_) => Read::Refused(vec![Issue::Json.into()]),
        },
        Err(_) => Read::Refused(vec![Issue::Unreadable.into()]),
    }
}

/// What tells a file changed without reading it: its size and modification time.
pub type Stamp = Option<(u64, Option<SystemTime>)>;

pub fn stamp(path: &Path) -> Stamp {
    std::fs::metadata(path)
        .ok()
        .map(|m| (m.len(), m.modified().ok()))
}

/// Where an entry of the theme list comes from.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    Terminal,
    BuiltIn(&'static str),
    File { name: String, read: Read },
}

/// One row of the theme list: a theme and the scheme it is shown in (a file with both schemes is two rows), or a file that cannot
/// be used.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub source: Source,
    pub scheme: Scheme,
}

impl Entry {
    /// The choice that picks this entry; `None` for a file that cannot be used.
    pub fn choice(&self) -> Option<Choice> {
        match &self.source {
            Source::Terminal => Some(Choice::Terminal),
            Source::BuiltIn(id) => Some(Choice::BuiltIn((*id).to_string())),
            Source::File {
                name,
                read: Read::Ok { .. },
            } => Some(Choice::File {
                name: name.clone(),
                scheme: self.scheme,
            }),
            Source::File { .. } => None,
        }
    }

    /// The theme this entry shows, when it has one.
    pub fn theme(&self) -> Option<Theme> {
        match &self.source {
            Source::Terminal => None,
            Source::BuiltIn(id) => built_ins()
                .into_iter()
                .find(|b| b.id == *id)
                .map(|b| b.theme),
            Source::File { read, .. } => read.theme().cloned(),
        }
    }
}

/// The theme files in `folder`, sorted by name, at most [`MAX_FILES`]; a missing folder has none.
pub fn file_names(folder: &Path) -> Vec<String> {
    let Ok(dir) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut names: Vec<String> = dir
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| is_theme_file_name(n))
        .collect();
    names.sort();
    names.truncate(MAX_FILES);
    names
}

/// Every row of the theme list: no theme, the built-in themes, then the folder's files.
pub fn entries(folder: &Path) -> Vec<Entry> {
    let mut out = vec![Entry {
        source: Source::Terminal,
        scheme: Scheme::Dark,
    }];
    for b in built_ins() {
        out.push(Entry {
            source: Source::BuiltIn(b.id),
            scheme: b.scheme,
        });
    }
    for name in file_names(folder) {
        let read = load_file(&folder.join(&name));
        let schemes = read.theme().map(Theme::schemes).unwrap_or_default();
        if schemes.is_empty() {
            out.push(Entry {
                source: Source::File { name, read },
                scheme: Scheme::Dark,
            });
            continue;
        }
        for scheme in schemes {
            out.push(Entry {
                source: Source::File {
                    name: name.clone(),
                    read: read.clone(),
                },
                scheme,
            });
        }
    }
    out
}

/// The theme a choice names now, with the scheme to show: `Ok(None)` for no theme, `Err` with the reasons when its file cannot be
/// used (any more), so the client falls back to no theme and says why.
pub fn resolve(choice: &Choice, folder: &Path) -> Result<Option<(Theme, Scheme)>, Read> {
    match choice {
        Choice::Terminal => Ok(None),
        Choice::BuiltIn(id) => Ok(built_ins()
            .into_iter()
            .find(|b| b.id == *id)
            .map(|b| (b.theme, b.scheme))),
        Choice::File { name, scheme } => match load_file(&folder.join(name)) {
            Read::Ok { theme, .. } if theme.look(*scheme).is_some() => Ok(Some((*theme, *scheme))),
            // A file that no longer has the chosen scheme: its first one.
            Read::Ok { theme, .. } => {
                let first = theme.schemes().first().copied().unwrap_or(Scheme::Dark);
                Ok(Some((*theme, first)))
            }
            refused => Err(refused),
        },
    }
}

/// The path of the file a choice reads, for [`stamp`].
pub fn path_of(choice: &Choice, folder: &Path) -> Option<PathBuf> {
    match choice {
        Choice::File { name, .. } => Some(folder.join(name)),
        _ => None,
    }
}

/// The kept choice (no theme when the file is missing or says something else).
pub fn load_choice(file: &Path) -> Choice {
    match std::fs::metadata(file) {
        Ok(m) if m.len() <= 4096 => std::fs::read_to_string(file)
            .map(|t| Choice::parse(&t))
            .unwrap_or_default(),
        _ => Choice::Terminal,
    }
}

/// Keeps the choice; the folder is made when missing.
pub fn save_choice(file: &Path, choice: &Choice) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, choice.line())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("agora-tui-themes-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_choice_round_trips_and_anything_else_is_no_theme() {
        for c in [
            Choice::Terminal,
            Choice::BuiltIn("slate".into()),
            Choice::File {
                name: "my theme.json".into(),
                scheme: Scheme::Light,
            },
        ] {
            assert_eq!(Choice::parse(&c.line()), c);
        }
        for bad in ["", "built-in nope", "file dark ../x.json", "file blue x.json", "file dark x.txt", "slate"] {
            assert_eq!(Choice::parse(bad), Choice::Terminal, "{bad}");
        }
    }

    #[test]
    fn the_folder_lists_json_files_by_name_one_row_per_scheme() {
        let d = scratch("list");
        std::fs::write(d.join("b.json"), include_str!("testdata/harbour.json")).unwrap();
        std::fs::write(d.join("a.json"), "{ not json").unwrap();
        std::fs::write(d.join("notes.txt"), "x").unwrap();
        std::fs::write(d.join(".hidden.json"), "{}").unwrap();
        let rows = entries(&d);
        // No theme, the six built-in themes, the broken file (one row), then Harbour's two schemes.
        assert_eq!(rows.len(), 1 + 6 + 1 + 2);
        assert_eq!(rows[0].choice(), Some(Choice::Terminal));
        assert!(matches!(&rows[7].source, Source::File { name, read: Read::Refused(_) } if name == "a.json"));
        assert_eq!(rows[7].choice(), None);
        assert_eq!(
            rows[8..].iter().map(|r| r.choice().unwrap()).collect::<Vec<_>>(),
            [
                Choice::File { name: "b.json".into(), scheme: Scheme::Dark },
                Choice::File { name: "b.json".into(), scheme: Scheme::Light },
            ]
        );
        assert_eq!(entries(&d.join("missing")).len(), 7);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_file_over_the_cap_is_refused_before_it_is_read() {
        let d = scratch("cap");
        let big = d.join("big.json");
        let f = std::fs::File::create(&big).unwrap();
        f.set_len(MAX_FILE_BYTES + 1).unwrap();
        assert_eq!(load_file(&big), Read::Refused(vec![Issue::TooLarge.into()]));
        assert_eq!(load_file(&d.join("none.json")), Read::Refused(vec![Issue::Unreadable.into()]));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_choice_resolves_and_a_changed_file_is_noticed_and_read_again() {
        let d = scratch("reload");
        let path = d.join("t.json");
        std::fs::write(&path, include_str!("testdata/harbour.json")).unwrap();
        let choice = Choice::File {
            name: "t.json".into(),
            scheme: Scheme::Light,
        };
        let (theme, scheme) = resolve(&choice, &d).unwrap().unwrap();
        assert_eq!((theme.name.as_str(), scheme), ("Harbour", Scheme::Light));
        let before = stamp(&path);
        // A broken edit: the choice cannot be used, and says why.
        std::fs::write(&path, r##"{"agora-theme": 2, "name": "Harbour", "dark": {"tokens": {"danger": "#ff0000"}}}"##).unwrap();
        assert_ne!(stamp(&path), before);
        assert!(matches!(resolve(&choice, &d), Err(Read::Refused(_))));
        assert_eq!(resolve(&Choice::Terminal, &d), Ok(None));
        let file = d.join(CHOICE_FILE);
        save_choice(&file, &choice).unwrap();
        assert_eq!(load_choice(&file), choice);
        assert_eq!(load_choice(&d.join("nothing")), Choice::Terminal);
        let _ = std::fs::remove_dir_all(&d);
    }
}

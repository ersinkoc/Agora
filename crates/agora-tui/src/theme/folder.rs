//! Where the terminal client's themes come from: the built-in themes, the starter themes ([`starter`](super::starter)), and theme files in its themes folder (`themes` in the Agora
//! folder: `$AGORA_HOME/themes`, else `~/.agora/themes`, beside the client's device key), and the one it uses, kept in
//! `tui-theme` there. The caller passes both paths, so tests never touch the real folder.
//!
//! A file is read only when it is at most [`MAX_FILE_BYTES`](super::file::MAX_FILE_BYTES) (asked of the file system first), and
//! only `*.json` files directly in the folder are themes. A file that is a theme pack (`super::pack`) gives each of its themes; its
//! layouts and keybindings are the web window's and are listed as not used here. Live reload: the caller asks for [`stamp`] of the file in use now and then
//! (its size and modification time, no reading), and reads it again when that changed.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

use super::file::{read_value, Issue, Read, MAX_FILE_BYTES};
use super::pack::{is_pack, read_pack, Pack, PackRead};
use super::starter::{starter, starters, Starter, FILES};
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
    /// One scheme of a starter theme, by id.
    Starter { id: String, scheme: Scheme },
    /// One scheme of a theme file in the folder, by file name.
    File { name: String, scheme: Scheme },
    /// One scheme of a pack's theme (`index` from 0) in the folder, by file name.
    Pack {
        name: String,
        index: usize,
        scheme: Scheme,
    },
}

impl Choice {
    /// The choice kept in [`CHOICE_FILE`]: one line, `terminal`, `built-in <id>`, `starter <dark|light> <id>`,
    /// `file <dark|light> <file name>` or `pack <dark|light> <theme number from 0> <file name>`. Anything else is no theme.
    pub fn parse(text: &str) -> Choice {
        let line = text.lines().next().unwrap_or("").trim();
        if let Some(id) = line.strip_prefix("built-in ") {
            if built_ins().iter().any(|b| b.id == id) {
                return Choice::BuiltIn(id.to_string());
            }
        }
        if let Some((scheme, id)) = line.strip_prefix("starter ").and_then(scheme_and) {
            if FILES.iter().any(|f| f.id == id) {
                return Choice::Starter {
                    id: id.to_string(),
                    scheme,
                };
            }
        }
        if let Some(rest) = line.strip_prefix("file ") {
            let scheme = scheme_and(rest);
            if let Some((scheme, name)) = scheme.filter(|(_, n)| is_theme_file_name(n)) {
                return Choice::File {
                    name: name.to_string(),
                    scheme,
                };
            }
        }
        if let Some(rest) = line.strip_prefix("pack ") {
            let mut parts = rest.splitn(3, ' ');
            let scheme = match parts.next() {
                Some("dark") => Some(Scheme::Dark),
                Some("light") => Some(Scheme::Light),
                _ => None,
            };
            let index = parts
                .next()
                .filter(|n| !n.is_empty() && n.len() <= 3 && n.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|n| n.parse::<usize>().ok());
            if let (Some(scheme), Some(index), Some(name)) = (scheme, index, parts.next()) {
                if is_theme_file_name(name) {
                    return Choice::Pack {
                        name: name.to_string(),
                        index,
                        scheme,
                    };
                }
            }
        }
        Choice::Terminal
    }

    pub fn line(&self) -> String {
        match self {
            Choice::Terminal => "terminal\n".to_string(),
            Choice::BuiltIn(id) => format!("built-in {id}\n"),
            Choice::Starter { id, scheme } => format!("starter {} {id}\n", scheme.word()),
            Choice::File { name, scheme } => format!("file {} {name}\n", scheme.word()),
            Choice::Pack {
                name,
                index,
                scheme,
            } => format!("pack {} {index} {name}\n", scheme.word()),
        }
    }

    /// The file in the themes folder this choice reads, if any.
    pub fn file_name(&self) -> Option<&str> {
        match self {
            Choice::File { name, .. } | Choice::Pack { name, .. } => Some(name),
            _ => None,
        }
    }
}

/// `dark <rest>` or `light <rest>`: the scheme and the rest.
fn scheme_and(rest: &str) -> Option<(Scheme, &str)> {
    match rest.split_once(' ') {
        Some(("dark", r)) => Some((Scheme::Dark, r)),
        Some(("light", r)) => Some((Scheme::Light, r)),
        _ => None,
    }
}

/// A file name the folder lists as a theme: `*.json`, one name (no folder in it), not hidden.
fn is_theme_file_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\'])
        && name.to_ascii_lowercase().ends_with(".json")
}

/// A file of the folder, read: a theme, or a theme pack.
#[derive(Clone, Debug, PartialEq)]
pub enum Loaded {
    Theme(Read),
    Pack(PackRead),
}

impl Loaded {
    /// The file as one theme row would show it: a pack that cannot be used is a file that cannot be used, with its reasons.
    fn refused(&self) -> Option<Read> {
        match self {
            Loaded::Theme(r @ Read::Refused(_)) => Some(r.clone()),
            Loaded::Pack(PackRead::Refused(p)) => Some(Read::Refused(p.clone())),
            _ => None,
        }
    }
}

/// Reads one file of the folder, a theme or a pack (the same size cap for both): refused as too large from its size alone, before
/// anything is read.
pub fn load(path: &Path) -> Loaded {
    let theme = |i: Issue| Loaded::Theme(Read::Refused(vec![i.into()]));
    match std::fs::metadata(path) {
        Ok(m) if !m.is_file() => return theme(Issue::Unreadable),
        Ok(m) if m.len() > MAX_FILE_BYTES => return theme(Issue::TooLarge),
        Ok(_) => {}
        Err(_) => return theme(Issue::Unreadable),
    }
    match std::fs::read(path) {
        Ok(bytes) if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_FILE_BYTES => {
            theme(Issue::TooLarge)
        }
        // Decoded as the web's import decodes a file (`Blob.text()`, UTF-8 decode): a leading byte-order mark is dropped and a byte
        // that is not UTF-8 becomes U+FFFD, so a file is accepted or refused alike in both clients (audit F1800).
        Ok(bytes) => {
            let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
            let text = String::from_utf8_lossy(body);
            // The decoded text is held to the cap too, as `read_text` holds it: a byte that is not UTF-8 grows to three.
            if u64::try_from(text.len()).unwrap_or(u64::MAX) > MAX_FILE_BYTES {
                return theme(Issue::TooLarge);
            }
            match serde_json::from_str::<Value>(&text) {
                Ok(v) if is_pack(&v) => Loaded::Pack(read_pack(&v)),
                Ok(v) => Loaded::Theme(read_value(&v)),
                Err(_) => theme(Issue::Json),
            }
        }
        Err(_) => theme(Issue::Unreadable),
    }
}

/// Reads one theme file (a pack is not one theme: it is refused here as not a theme file).
pub fn load_file(path: &Path) -> Read {
    match load(path) {
        Loaded::Theme(r) => r,
        Loaded::Pack(_) => Read::Refused(vec![Issue::Invalid.into()]),
    }
}

/// What tells a file changed without reading it: its size and modification time.
pub type Stamp = Option<(u64, Option<SystemTime>)>;

pub fn stamp(path: &Path) -> Stamp {
    std::fs::metadata(path)
        .ok()
        .map(|m| (m.len(), m.modified().ok()))
}

/// What the list says about the pack a theme came from: its name, which of its themes this is, and what it holds that this client
/// does not use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InPack {
    pub pack: String,
    pub themes: usize,
    pub layouts: usize,
    pub keys: usize,
}

/// Where an entry of the theme list comes from.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    Terminal,
    BuiltIn(&'static str),
    /// A starter theme, read when the list was made.
    Starter(Box<Starter>),
    File {
        name: String,
        read: Read,
    },
    /// Theme `index` (from 0) of the pack in file `name`.
    Pack {
        name: String,
        index: usize,
        read: Read,
        pack: InPack,
    },
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
            Source::Starter(s) => Some(Choice::Starter {
                id: s.id.to_string(),
                scheme: self.scheme,
            }),
            Source::File {
                name,
                read: Read::Ok { .. },
            } => Some(Choice::File {
                name: name.clone(),
                scheme: self.scheme,
            }),
            Source::File { .. } => None,
            Source::Pack {
                name,
                index,
                read: Read::Ok { .. },
                ..
            } => Some(Choice::Pack {
                name: name.clone(),
                index: *index,
                scheme: self.scheme,
            }),
            Source::Pack { .. } => None,
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
            Source::Starter(s) => Some(s.theme.clone()),
            Source::File { read, .. } | Source::Pack { read, .. } => read.theme().cloned(),
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

/// Every row of the theme list: no theme, the built-in themes, the starter themes (a row per scheme each has), then the folder's
/// files.
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
    for s in starters() {
        for scheme in s.theme.schemes() {
            out.push(Entry {
                source: Source::Starter(Box::new(s.clone())),
                scheme,
            });
        }
    }
    for name in file_names(folder) {
        let loaded = load(&folder.join(&name));
        if let Loaded::Pack(PackRead::Ok(pack)) = &loaded {
            pack_rows(&mut out, &name, pack);
            continue;
        }
        let read = match loaded {
            Loaded::Theme(r) => r,
            other => other.refused().unwrap_or(Read::Refused(Vec::new())),
        };
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

/// A pack's rows: one per scheme of each of its themes.
fn pack_rows(out: &mut Vec<Entry>, name: &str, pack: &Pack) {
    let info = InPack {
        pack: pack.name.clone(),
        themes: pack.themes.len(),
        layouts: pack.layouts,
        keys: pack.keys,
    };
    for (index, read) in pack.themes.iter().enumerate() {
        for scheme in read.theme().map(Theme::schemes).unwrap_or_default() {
            out.push(Entry {
                source: Source::Pack {
                    name: name.to_string(),
                    index,
                    read: read.clone(),
                    pack: info.clone(),
                },
                scheme,
            });
        }
    }
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
        Choice::Starter { id, scheme } => {
            Ok(starter(id, *scheme).map(|(s, shown)| (s.theme, shown)))
        }
        Choice::File { name, scheme } => match load_file(&folder.join(name)) {
            Read::Ok { theme, .. } if theme.look(*scheme).is_some() => Ok(Some((*theme, *scheme))),
            // A file that no longer has the chosen scheme: its first one.
            Read::Ok { theme, .. } => {
                let first = theme.schemes().first().copied().unwrap_or(Scheme::Dark);
                Ok(Some((*theme, first)))
            }
            refused => Err(refused),
        },
        Choice::Pack {
            name,
            index,
            scheme,
        } => match load(&folder.join(name)) {
            Loaded::Pack(PackRead::Ok(pack)) => match pack.theme(*index) {
                Some(theme) if theme.look(*scheme).is_some() => Ok(Some((theme.clone(), *scheme))),
                Some(theme) => {
                    let first = theme.schemes().first().copied().unwrap_or(Scheme::Dark);
                    Ok(Some((theme.clone(), first)))
                }
                // The pack no longer has that theme.
                None => Err(Read::Refused(vec![Issue::Invalid.into()])),
            },
            Loaded::Pack(PackRead::Refused(p)) => Err(Read::Refused(p)),
            // The file is a theme now, not a pack.
            Loaded::Theme(_) => Err(Read::Refused(vec![Issue::Invalid.into()])),
        },
    }
}

/// The path of the file a choice reads, for [`stamp`].
pub fn path_of(choice: &Choice, folder: &Path) -> Option<PathBuf> {
    choice.file_name().map(|name| folder.join(name))
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
            Choice::Starter {
                id: "gruvbox".into(),
                scheme: Scheme::Light,
            },
            Choice::File {
                name: "my theme.json".into(),
                scheme: Scheme::Light,
            },
        ] {
            assert_eq!(Choice::parse(&c.line()), c);
        }
        assert_eq!(
            Choice::Starter {
                id: "dracula".into(),
                scheme: Scheme::Dark
            }
            .line(),
            "starter dark dracula\n"
        );
        for bad in [
            "",
            "built-in nope",
            "starter dark nope",
            "starter blue gruvbox",
            "starter gruvbox",
            "file dark ../x.json",
            "file blue x.json",
            "file dark x.txt",
            "slate",
        ] {
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
        // No theme, the six built-in themes, the starter themes (a row per scheme), the broken file (one row), then Harbour's two
        // schemes.
        let starter_rows: usize = starters().iter().map(|s| s.theme.schemes().len()).sum();
        let fixed = 1 + 6 + starter_rows;
        assert_eq!(rows.len(), fixed + 1 + 2);
        assert_eq!(rows[0].choice(), Some(Choice::Terminal));
        assert_eq!(
            rows[7..fixed]
                .iter()
                .map(|r| r.choice().unwrap().line())
                .collect::<Vec<_>>(),
            [
                "starter dark abyss\n",
                "starter light arctic\n",
                "starter dark aurora\n",
                "starter light blossom\n",
                "starter dark bonsai\n",
                "starter dark catppuccin-mocha\n",
                "starter dark cobalt\n",
                "starter light dawn\n",
                "starter dark dracula\n",
                "starter dark ember\n",
                "starter dark graphite\n",
                "starter dark gruvbox\n",
                "starter light gruvbox\n",
                "starter light lagoon\n",
                "starter dark lavender\n",
                "starter light lilac\n",
                "starter light meadow\n",
                "starter light mint\n",
                "starter dark mulberry\n",
                "starter light parchment\n",
                "starter light porcelain\n",
                "starter dark rose-pine\n",
                "starter light sepia\n",
                "starter dark tokyo-night\n",
                "starter dark ultraviolet\n",
            ]
        );
        assert!(
            matches!(&rows[fixed].source, Source::File { name, read: Read::Refused(_) } if name == "a.json")
        );
        assert_eq!(rows[fixed].choice(), None);
        assert_eq!(
            rows[fixed + 1..]
                .iter()
                .map(|r| r.choice().unwrap())
                .collect::<Vec<_>>(),
            [
                Choice::File {
                    name: "b.json".into(),
                    scheme: Scheme::Dark
                },
                Choice::File {
                    name: "b.json".into(),
                    scheme: Scheme::Light
                },
            ]
        );
        assert_eq!(entries(&d.join("missing")).len(), fixed);
        // A starter choice resolves to its theme in the scheme chosen, or its first when it has no such scheme.
        let gruvbox = Choice::Starter {
            id: "gruvbox".into(),
            scheme: Scheme::Light,
        };
        let (theme, scheme) = resolve(&gruvbox, &d).unwrap().unwrap();
        assert_eq!((theme.name.as_str(), scheme), ("Gruvbox", Scheme::Light));
        let dracula = Choice::Starter {
            id: "dracula".into(),
            scheme: Scheme::Light,
        };
        assert_eq!(resolve(&dracula, &d).unwrap().unwrap().1, Scheme::Dark);
        assert_eq!(path_of(&gruvbox, &d), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    fn a_pack(themes: &str) -> String {
        format!(
            r#"{{"agora-pack": 1, "name": "Harbour set", "themes": [{themes}],
               "layouts": [{{"name": "Review", "layout": {{"version": 1, "root": {{"type": "pane", "panel": "plugin.clock.face"}}}}}}],
               "keybindings": {{"version": 1, "bindings": [{{"command": "palette.open", "keys": ["ctrl+p"]}}]}}}}"#
        )
    }

    #[test]
    fn a_pack_in_the_folder_gives_its_themes_and_says_what_it_does_not_use() {
        let d = scratch("pack");
        let harbour = include_str!("testdata/harbour.json");
        std::fs::write(d.join("set.json"), a_pack(harbour)).unwrap();
        let rows = entries(&d);
        // No theme, the six built-in themes, the starter themes, then the pack's theme in both its schemes.
        let starter_rows: usize = starters().iter().map(|s| s.theme.schemes().len()).sum();
        let fixed = 1 + 6 + starter_rows;
        assert_eq!(rows.len(), fixed + 2);
        let chosen: Vec<Choice> = rows[fixed..].iter().map(|r| r.choice().unwrap()).collect();
        assert_eq!(
            chosen,
            [
                Choice::Pack {
                    name: "set.json".into(),
                    index: 0,
                    scheme: Scheme::Dark
                },
                Choice::Pack {
                    name: "set.json".into(),
                    index: 0,
                    scheme: Scheme::Light
                },
            ]
        );
        assert!(matches!(
            &rows[fixed].source,
            Source::Pack { pack, .. } if *pack == InPack { pack: "Harbour set".into(), themes: 1, layouts: 1, keys: 1 }
        ));
        // The same theme as the file on its own.
        assert_eq!(
            rows[fixed].theme().as_ref(),
            super::super::file::read_text(harbour).theme()
        );
        let (theme, scheme) = resolve(&chosen[1], &d).unwrap().unwrap();
        assert_eq!((theme.name.as_str(), scheme), ("Harbour", Scheme::Light));
        assert_eq!(path_of(&chosen[1], &d), Some(d.join("set.json")));
        // A theme the pack no longer has, and a pack that can no longer be used, fall back with the reasons.
        let gone = Choice::Pack {
            name: "set.json".into(),
            index: 3,
            scheme: Scheme::Dark,
        };
        assert!(resolve(&gone, &d).is_err());
        std::fs::write(
            d.join("set.json"),
            a_pack(r#"{"agora-theme": 1, "name": "Old", "scheme": "dark", "tokens": {}}"#),
        )
        .unwrap();
        assert!(
            matches!(resolve(&chosen[0], &d), Err(Read::Refused(p)) if p[0].issue == Issue::PackThemeFormat(1))
        );
        // A pack that cannot be used is one row that says why, never offered.
        let rows = entries(&d);
        assert_eq!(rows.len(), fixed + 1);
        assert!(
            matches!(&rows[fixed].source, Source::File { name, read: Read::Refused(_) } if name == "set.json")
        );
        assert_eq!(rows[fixed].choice(), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_pack_choice_round_trips_and_a_bad_one_is_no_theme() {
        let c = Choice::Pack {
            name: "my set.json".into(),
            index: 12,
            scheme: Scheme::Dark,
        };
        assert_eq!(c.line(), "pack dark 12 my set.json\n");
        assert_eq!(Choice::parse(&c.line()), c);
        for bad in [
            "pack dark x set.json",
            "pack dark -1 set.json",
            "pack dark 1234 set.json",
            "pack blue 0 set.json",
            "pack dark 0 ../set.json",
            "pack dark 0",
        ] {
            assert_eq!(Choice::parse(bad), Choice::Terminal, "{bad}");
        }
    }

    #[test]
    fn a_file_over_the_cap_is_refused_before_it_is_read() {
        let d = scratch("cap");
        let big = d.join("big.json");
        let f = std::fs::File::create(&big).unwrap();
        f.set_len(MAX_FILE_BYTES + 1).unwrap();
        assert_eq!(load_file(&big), Read::Refused(vec![Issue::TooLarge.into()]));
        assert_eq!(
            load_file(&d.join("none.json")),
            Read::Refused(vec![Issue::Unreadable.into()])
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Audit F1800: a file is decoded as the web's import decodes it (`Blob.text()`): a leading byte-order mark (Windows PowerShell 5.1's
    /// `Out-File -Encoding utf8`, older Notepad) is dropped and a byte that is not UTF-8 becomes U+FFFD, so the file is accepted here too.
    #[test]
    fn a_file_is_decoded_as_the_web_decodes_it() {
        let d = scratch("decode");
        let theme = br#"{"agora-theme": 2, "name": "Harbour", "inherits": "dark"}"#;
        let bom = d.join("bom.json");
        std::fs::write(&bom, [&[0xEF, 0xBB, 0xBF][..], theme].concat()).unwrap();
        assert!(
            matches!(load_file(&bom), Read::Ok { .. }),
            "{:?}",
            load_file(&bom)
        );
        let latin = d.join("latin1.json");
        let mut bytes = br#"{"agora-theme": 2, "name": "Caf"#.to_vec();
        bytes.push(0xE9);
        bytes.extend_from_slice(br#"", "inherits": "dark"}"#);
        std::fs::write(&latin, bytes).unwrap();
        match load_file(&latin) {
            Read::Ok { theme, .. } => assert_eq!(theme.name, "Caf\u{fffd}"),
            refused => panic!("{refused:?}"),
        }
        // A BOM is dropped only at the start: one inside the text is not JSON there.
        let inside = d.join("inside.json");
        std::fs::write(
            &inside,
            [&b" "[..], &[0xEF, 0xBB, 0xBF][..], theme].concat(),
        )
        .unwrap();
        assert_eq!(load_file(&inside), Read::Refused(vec![Issue::Json.into()]));
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
        std::fs::write(
            &path,
            r##"{"agora-theme": 2, "name": "Harbour", "dark": {"tokens": {"danger": "#ff0000"}}}"##,
        )
        .unwrap();
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

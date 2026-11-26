//! Starter themes (roadmap M5.2; `docs/ux/themes.md`, "Starter themes"): well-known palettes ported as theme files of format 2. They
//! are the web window's own files (`apps/web/src/themes/starter/`, compiled in), read by the same reader as a file in the themes
//! folder ([`read_text`]), so a starter theme is accepted or refused alike in both clients. Only the palettes' published colour values
//! are in them; their licences are in `THIRD_PARTY.md`.

use super::file::{read_text, Read};
use super::{Scheme, Theme};

/// A starter theme's file: its id (the file's name; the web's theme id is `starter-<id>`), the palette's project and its licence (an
/// SPDX id), and the file's text.
pub struct StarterFile {
    pub id: &'static str,
    pub project: &'static str,
    pub licence: &'static str,
    text: &'static str,
}

/// The files, in the web's order (`apps/web/src/themes/starter.ts` `STARTER_FILES`).
pub const FILES: [StarterFile; 5] = [
    StarterFile {
        id: "catppuccin-mocha",
        project: "Catppuccin",
        licence: "MIT",
        text: include_str!("../../../../apps/web/src/themes/starter/catppuccin-mocha.json"),
    },
    StarterFile {
        id: "dracula",
        project: "Dracula",
        licence: "MIT",
        text: include_str!("../../../../apps/web/src/themes/starter/dracula.json"),
    },
    StarterFile {
        id: "gruvbox",
        project: "Gruvbox",
        licence: "MIT",
        text: include_str!("../../../../apps/web/src/themes/starter/gruvbox.json"),
    },
    StarterFile {
        id: "rose-pine",
        project: "Rosé Pine",
        licence: "MIT",
        text: include_str!("../../../../apps/web/src/themes/starter/rose-pine.json"),
    },
    StarterFile {
        id: "tokyo-night",
        project: "Tokyo Night",
        licence: "Apache-2.0",
        text: include_str!("../../../../apps/web/src/themes/starter/tokyo-night.json"),
    },
];

impl StarterFile {
    /// The file as the reader reads it.
    pub fn read(&self) -> Read {
        read_text(self.text)
    }

    /// The starter theme, when the reader accepts the file.
    fn starter(&self) -> Option<Starter> {
        match self.read() {
            Read::Ok { theme, .. } => Some(Starter {
                id: self.id,
                project: self.project,
                licence: self.licence,
                theme: *theme,
            }),
            Read::Refused(_) => None,
        }
    }
}

/// A starter theme the reader accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Starter {
    pub id: &'static str,
    pub project: &'static str,
    pub licence: &'static str,
    pub theme: Theme,
}

/// Every starter theme, read once by the caller (the theme list holds them while it is open). A file the reader refuses is left out;
/// a test holds every file to no problems and no warnings, so none is.
pub fn starters() -> Vec<Starter> {
    FILES.iter().filter_map(StarterFile::starter).collect()
}

/// One starter theme by id (only its file is read), and the scheme to show: the one asked for when the theme has it, else its first.
pub fn starter(id: &str, scheme: Scheme) -> Option<(Starter, Scheme)> {
    let s = FILES.iter().find(|f| f.id == id)?.starter()?;
    let shown = if s.theme.look(scheme).is_some() {
        scheme
    } else {
        s.theme.schemes().first().copied().unwrap_or(Scheme::Dark)
    };
    Some((s, shown))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Every starter file passes this client's reader with no problem and no warning, as it passes the web's (`starter.test.tsx`).
    #[test]
    fn every_starter_file_is_read_with_no_problem_and_no_warning() {
        for f in &FILES {
            match f.read() {
                Read::Ok { theme, warnings } => {
                    assert!(warnings.is_empty(), "{}: {warnings:?}", f.id);
                    assert!(theme.name.starts_with(f.project), "{}", f.id);
                    assert!(!theme.schemes().is_empty(), "{}", f.id);
                }
                Read::Refused(p) => panic!("{}: {p:?}", f.id),
            }
        }
        assert_eq!(starters().len(), FILES.len());
    }

    /// The list is the web's: the same ids, projects and licences, in the same order, and every file in the folder is one of them.
    #[test]
    fn the_starter_files_are_the_webs() {
        let ts = include_str!("../../../../apps/web/src/themes/starter.ts");
        let mut at = 0;
        for f in &FILES {
            let line = format!(
                "{{ id: '{}', text: {}, source: {{ project: '{}', licence: '{}' }} }}",
                f.id,
                camel(f.id),
                f.project,
                f.licence
            );
            let found = ts[at..].find(&line).unwrap_or_else(|| panic!("{line}"));
            at += found + line.len();
        }
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/web/src/themes/starter");
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.ends_with(".json"))
            .collect();
        names.sort();
        let mut ids: Vec<String> = FILES.iter().map(|f| format!("{}.json", f.id)).collect();
        ids.sort();
        assert_eq!(names, ids);
    }

    fn camel(id: &str) -> String {
        let mut out = String::new();
        let mut up = false;
        for c in id.chars() {
            if c == '-' {
                up = true;
            } else if up {
                out.extend(c.to_uppercase());
                up = false;
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn a_starter_is_shown_in_a_scheme_it_has() {
        let (s, shown) = starter("dracula", Scheme::Light).unwrap();
        assert_eq!((s.id, shown), ("dracula", Scheme::Dark));
        let (_, shown) = starter("gruvbox", Scheme::Light).unwrap();
        assert_eq!(shown, Scheme::Light);
        assert!(starter("nope", Scheme::Dark).is_none());
    }
}

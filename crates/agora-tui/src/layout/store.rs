//! Where the terminal client keeps layouts (ADR-0068 decision 4): in its Agora folder (`$AGORA_HOME`, else `~/.agora`), beside the
//! `tui-theme` choice and the `themes` folder.
//!
//! - `layouts/<name>.json`: each saved layout, one file each, in exactly the format the web window's Export writes and its Import
//!   reads (`docs/ux/layout.md`, "Format, version 1"). Saving here is exporting: the file goes to the web as it is, and a
//!   `layout.json` the web exported, put in this folder, is a saved layout here.
//! - `tui-layout.json`: the arrangement in use, `{ "version": 1, "layout": …, "focus": …, "zoom": … }` (the web keeps the same
//!   object in its browser storage), so it survives a restart.
//!
//! A file is read only when it is at most [`MAX_FILE_BYTES`] (asked of the file system first). Reading never panics, and a file that
//! cannot be read (or that a newer version wrote) is left alone. The caller passes the paths, so tests never touch the real folder.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::engine::{reduce, Action, Workspace};
use super::tree::{
    deserialise_layout, layout_value, parse_layout, serialise_layout, Layout, LayoutError,
};

/// The saved layouts' folder in the Agora folder.
pub const FOLDER: &str = "layouts";
/// The file in the Agora folder that keeps the arrangement in use.
pub const CURRENT_FILE: &str = "tui-layout.json";
/// The largest layout file read; a real one is a few kilobytes (32 panes at most).
pub const MAX_FILE_BYTES: u64 = 256 * 1024;
/// The most saved layouts listed (sorted by name), as many as the web keeps.
pub const MAX_SAVED: usize = 50;
/// The longest name, as in the web.
pub const MAX_NAME: usize = 60;

/// Why a saved layout could not be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadError {
    Missing,
    TooLarge,
    Unreadable,
    Layout(LayoutError),
}

/// The file a layout named `name` is kept in: the name with `.json`, when it is one a file system takes everywhere (no folder, not
/// hidden, no character Windows refuses, at most [`MAX_NAME`] characters). `None` for a name that cannot be a file.
pub fn file_name(name: &str) -> Option<String> {
    let n = name.trim();
    let bad = |c: char| {
        c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
    };
    if n.is_empty()
        || n.starts_with('.')
        || n.ends_with('.')
        || n.chars().count() > MAX_NAME
        || n.chars().any(bad)
    {
        return None;
    }
    Some(format!("{n}.json"))
}

/// The saved layouts' names (their file names without `.json`), sorted, at most [`MAX_SAVED`]. A missing folder lists none.
pub fn list(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter_map(|f| {
            let stem = f
                .strip_suffix(".json")
                .or_else(|| f.strip_suffix(".JSON"))?
                .to_string();
            file_name(&stem)
                .is_some_and(|back| back == f || back.eq_ignore_ascii_case(&f))
                .then_some(stem)
        })
        .collect();
    names.sort();
    names.dedup();
    names.truncate(MAX_SAVED);
    names
}

/// Reads a file's text when it is a file of at most [`MAX_FILE_BYTES`].
fn read_capped(path: &Path) -> Result<String, LoadError> {
    let meta = std::fs::metadata(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            LoadError::Missing
        } else {
            LoadError::Unreadable
        }
    })?;
    if !meta.is_file() {
        return Err(LoadError::Unreadable);
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(LoadError::TooLarge);
    }
    let bytes = std::fs::read(path).map_err(|_| LoadError::Unreadable)?;
    // The file may have grown between the two looks.
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(LoadError::TooLarge);
    }
    String::from_utf8(bytes).map_err(|_| LoadError::Layout(LayoutError::Json))
}

/// Reads one layout file (a saved one, or one the web exported).
pub fn read_file(path: &Path) -> Result<Layout, LoadError> {
    deserialise_layout(&read_capped(path)?).map_err(LoadError::Layout)
}

/// Reads the saved layout `name`.
pub fn load(dir: &Path, name: &str) -> Result<Layout, LoadError> {
    let file = file_name(name).ok_or(LoadError::Missing)?;
    read_file(&dir.join(file))
}

/// Writes `text` to `path` through a file beside it, so a crash never leaves half a file.
fn write_whole(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".new");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// Saves `layout` as `name` (replacing one of that name). Answers the file written.
pub fn save(dir: &Path, name: &str, layout: &Layout) -> std::io::Result<PathBuf> {
    let file =
        file_name(name).ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let path = dir.join(file);
    let mut text = serialise_layout(layout);
    text.push('\n');
    write_whole(&path, &text)?;
    Ok(path)
}

pub fn delete(dir: &Path, name: &str) -> std::io::Result<()> {
    let file =
        file_name(name).ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    std::fs::remove_file(dir.join(file))
}

/// The arrangement in use as the object the web keeps (`agora.workspace.v1`, without its own `touched`).
pub fn workspace_value(ws: &Workspace) -> Value {
    json!({ "version": 1, "layout": layout_value(&ws.layout), "focus": ws.focus, "zoom": ws.zoom })
}

/// Reads the arrangement in use back; `None` when it is not one (never saved, unreadable, or a newer version's).
pub fn parse_workspace(v: &Value) -> Option<Workspace> {
    let o = v.as_object()?;
    let layout = parse_layout(o.get("layout")?).ok()?;
    let mut ws = Workspace::of(layout);
    let text = |k: &str| {
        o.get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    if let Some(zoom) = text("zoom") {
        ws = reduce(&ws, &Action::Maximize { pane: zoom });
    }
    if let Some(focus) = text("focus") {
        ws = reduce(&ws, &Action::Focus { pane: focus });
    }
    Some(ws)
}

pub fn load_workspace(path: &Path) -> Option<Workspace> {
    let text = read_capped(path).ok()?;
    parse_workspace(&serde_json::from_str(&text).ok()?)
}

pub fn save_workspace(path: &Path, ws: &Workspace) -> std::io::Result<()> {
    let text = serde_json::to_string(&workspace_value(ws)).map_err(std::io::Error::other)?;
    write_whole(path, &text)
}

#[cfg(test)]
mod tests {
    use super::super::engine::Preset;
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("agora-tui-layouts-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn grid(preset: Preset, agents: &[&str]) -> Workspace {
        reduce(
            &Workspace::of(Layout::default_layout()),
            &Action::Preset {
                preset,
                agents: agents.iter().map(|a| a.to_string()).collect(),
            },
        )
    }

    #[test]
    fn names_become_safe_file_names_or_nothing() {
        assert_eq!(file_name(" Four agents "), Some("Four agents.json".into()));
        assert_eq!(file_name("Dört ajan"), Some("Dört ajan.json".into()));
        for bad in [
            "",
            "  ",
            ".hidden",
            "a/b",
            "a\\b",
            "c:",
            "x?",
            "end.",
            "tab\there",
        ] {
            assert_eq!(file_name(bad), None, "{bad:?}");
        }
        assert_eq!(
            file_name(&"x".repeat(MAX_NAME)).map(|f| f.len()),
            Some(MAX_NAME + 5)
        );
        assert_eq!(file_name(&"x".repeat(MAX_NAME + 1)), None);
    }

    #[test]
    fn saves_lists_loads_and_deletes_layouts() {
        let dir = temp("saved");
        assert!(list(&dir).is_empty());
        let four = grid(
            Preset::TwoByTwo,
            &["mac/agent-1", "mac/agent-2", "vps/agent-1"],
        );
        let two = grid(Preset::TwoByOne, &["mac/agent-1"]);
        assert!(save(&dir, "Four", &four.layout).is_ok());
        assert!(save(&dir, "Two", &two.layout).is_ok());
        // Saving under a name that exists replaces it.
        assert!(save(&dir, "Two", &four.layout).is_ok());
        assert_eq!(list(&dir), ["Four", "Two"]);
        assert!(load(&dir, "Two").is_ok_and(|l| super::super::tree::same_layout(&l, &four.layout)));
        assert_eq!(load(&dir, "None"), Err(LoadError::Missing));
        assert!(delete(&dir, "Two").is_ok());
        assert_eq!(list(&dir), ["Four"]);
        // Files that are not layouts by their name are not listed: hidden, another extension, a half-written one.
        let _ = std::fs::write(dir.join(".hidden.json"), "{}");
        let _ = std::fs::write(dir.join("notes.txt"), "x");
        let _ = std::fs::write(dir.join("Four.json.new"), "x");
        assert_eq!(list(&dir), ["Four"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_too_large_is_refused_before_it_is_read_and_a_broken_one_says_why() {
        let dir = temp("refused");
        let _ = std::fs::create_dir_all(&dir);
        let big = dir.join("big.json");
        let _ = std::fs::write(&big, vec![b' '; (MAX_FILE_BYTES + 1) as usize]);
        assert_eq!(read_file(&big), Err(LoadError::TooLarge));
        let _ = std::fs::write(dir.join("broken.json"), "{");
        assert_eq!(
            load(&dir, "broken"),
            Err(LoadError::Layout(LayoutError::Json))
        );
        let _ = std::fs::write(
            dir.join("newer.json"),
            r#"{"version":2,"root":{"type":"pane","panel":"x"}}"#,
        );
        assert_eq!(
            load(&dir, "newer"),
            Err(LoadError::Layout(LayoutError::Newer))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_arrangement_in_use_survives_a_restart_with_its_focus_and_zoom() {
        let dir = temp("current");
        let path = dir.join(CURRENT_FILE);
        assert_eq!(load_workspace(&path), None);
        let mut ws = grid(Preset::TwoByTwo, &["a", "b", "c", "d"]);
        let third = super::super::tree::panes_of(&ws.layout.root)[2].id.clone();
        ws = reduce(&ws, &Action::Maximize { pane: third });
        assert!(save_workspace(&path, &ws).is_ok());
        let back = load_workspace(&path);
        assert!(back
            .as_ref()
            .is_some_and(|b| super::super::tree::same_layout(&b.layout, &ws.layout)));
        assert_eq!(
            back.map(|b| (b.focus, b.zoom)),
            Some((ws.focus.clone(), ws.zoom.clone()))
        );
        // A file that is not one is ignored: the client starts from its default.
        let _ = std::fs::write(&path, "[1,2]");
        assert_eq!(load_workspace(&path), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

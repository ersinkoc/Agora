//! Where a node's agent kinds come from (`docs/specs/agent-manifest.md`, "Where agent kinds come from").
//!
//! The binary carries the repository's shipped manifests (`manifests/agents/*.toml`) as built-in defaults, so a fresh install can
//! start every shipped agent without copying files. The node's manifests folder is read after them: a file there with the same
//! `kind` replaces the built-in one, a file with a new `kind` adds one. A file that does not parse is skipped (and reported, so
//! `agora doctor` can name it); a built-in kind it meant to replace stays. Fixture screens are not compiled in.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use agora_adapters::manifest::Manifest;

/// The shipped manifests, compiled in: `(file name, text)`. The test `built_in_list_matches_the_shipped_folder` fails when a file
/// in `manifests/agents/` is missing here, so a new shipped manifest cannot be forgotten.
pub const BUILT_IN: &[(&str, &str)] = &[
    (
        "antigravity-acp.toml",
        include_str!("../../../manifests/agents/antigravity-acp.toml"),
    ),
    (
        "claude-code.toml",
        include_str!("../../../manifests/agents/claude-code.toml"),
    ),
    (
        "codex.toml",
        include_str!("../../../manifests/agents/codex.toml"),
    ),
    (
        "opencode-acp.toml",
        include_str!("../../../manifests/agents/opencode-acp.toml"),
    ),
    (
        "wrongstack-acp.toml",
        include_str!("../../../manifests/agents/wrongstack-acp.toml"),
    ),
    (
        "wrongstack.toml",
        include_str!("../../../manifests/agents/wrongstack.toml"),
    ),
];

/// Where a loaded kind came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Compiled into this binary.
    BuiltIn,
    /// A file in the node's manifests folder.
    Folder(PathBuf),
}

impl Source {
    /// A stable name for messages (`built-in` or `folder`); the sentence around it lives in `locales/`.
    pub fn name(&self) -> &'static str {
        match self {
            Source::BuiltIn => "built-in",
            Source::Folder(_) => "folder",
        }
    }
}

/// One agent kind the node can offer, and where its manifest came from.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub manifest: Manifest,
    pub source: Source,
}

/// The built-in kinds, then the folder's: one entry per kind, sorted by kind. The second value lists each folder file (or, never in
/// a tested build, a built-in one) that could not be read or parsed, with the reason.
pub fn load(dir: &Path) -> (Vec<Loaded>, Vec<(PathBuf, String)>) {
    let mut by_kind: BTreeMap<String, Loaded> = BTreeMap::new();
    let mut problems = Vec::new();
    for (name, text) in BUILT_IN {
        match Manifest::parse(text) {
            Ok(m) => {
                by_kind.insert(
                    m.kind.clone(),
                    Loaded {
                        manifest: m,
                        source: Source::BuiltIn,
                    },
                );
            }
            Err(e) => problems.push((PathBuf::from(name), e.to_string())),
        }
    }
    if let Ok(rd) = std::fs::read_dir(dir) {
        let mut paths: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .collect();
        paths.sort();
        for p in paths {
            match std::fs::read_to_string(&p)
                .map_err(|e| e.to_string())
                .and_then(|s| Manifest::parse(&s).map_err(|e| e.to_string()))
            {
                Ok(m) => {
                    by_kind.insert(
                        m.kind.clone(),
                        Loaded {
                            manifest: m,
                            source: Source::Folder(p),
                        },
                    );
                }
                Err(e) => problems.push((p, e)),
            }
        }
    }
    (by_kind.into_values().collect(), problems)
}

/// The program the node looks for to tell whether a kind is installed: the L3 adapter's command, the ACP command, or the terminal
/// launch command, whichever the manifest uses (the same order the node starts it in).
pub fn program(m: &Manifest) -> Option<&str> {
    let acp = m.structured.as_ref().and_then(|s| s.acp.as_ref());
    match (&m.adapter, acp) {
        (Some(a), _) => a.command.first(),
        (None, Some(a)) => a.argv.first(),
        (None, None) => m.process.launch.first(),
    }
    .map(String::as_str)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn shipped_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../manifests/agents")
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "agora-builtin-manifests-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn built_in_list_matches_the_shipped_folder() {
        let mut shipped: Vec<String> = std::fs::read_dir(shipped_dir())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        shipped.sort();
        let mut embedded: Vec<String> = BUILT_IN.iter().map(|(n, _)| n.to_string()).collect();
        embedded.sort();
        assert_eq!(
            embedded, shipped,
            "every manifests/agents/*.toml is compiled into the node (add new ones to BUILT_IN)"
        );
        for (name, text) in BUILT_IN {
            let on_disk = std::fs::read_to_string(shipped_dir().join(name)).unwrap();
            assert_eq!(*text, on_disk, "{name}");
            Manifest::parse(text).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }

    #[test]
    fn an_empty_or_missing_folder_offers_every_shipped_kind() {
        let d = tmp("empty");
        for dir in [d.clone(), d.join("not-there")] {
            let (loaded, problems) = load(&dir);
            assert!(problems.is_empty());
            assert_eq!(loaded.len(), BUILT_IN.len());
            assert!(loaded.iter().all(|l| l.source == Source::BuiltIn));
        }
        let kinds: Vec<_> = load(&d).0.into_iter().map(|l| l.manifest.kind).collect();
        for k in ["claude-code", "codex", "wrongstack"] {
            assert!(kinds.contains(&k.to_string()), "{k}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_folder_file_overrides_or_adds_and_a_broken_override_keeps_the_built_in() {
        let d = tmp("override");
        std::fs::write(
            d.join("mine.toml"),
            "kind = \"codex\"\ndisplay_name = \"My Codex\"\nversion = \"9\"\nmin_engine = 1\n[process]\nlaunch = [\"my-codex\"]\n",
        )
        .unwrap();
        std::fs::write(
            d.join("extra.toml"),
            "kind = \"extra\"\ndisplay_name = \"Extra\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"extra\"]\n",
        )
        .unwrap();
        // Meant to replace claude-code, but does not parse.
        std::fs::write(d.join("claude-code.toml"), "kind = \"claude-code\"\n[").unwrap();
        let (loaded, problems) = load(&d);
        let by: BTreeMap<_, _> = loaded
            .iter()
            .map(|l| (l.manifest.kind.as_str(), l))
            .collect();
        assert_eq!(by["codex"].manifest.display_name, "My Codex");
        assert_eq!(by["codex"].source, Source::Folder(d.join("mine.toml")));
        assert_eq!(program(&by["codex"].manifest), Some("my-codex"));
        assert_eq!(by["extra"].source.name(), "folder");
        assert_eq!(by["claude-code"].source, Source::BuiltIn);
        assert_eq!(program(&by["claude-code"].manifest), Some("claude"));
        assert_eq!(loaded.len(), BUILT_IN.len() + 1);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].0, d.join("claude-code.toml"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_program_looked_for_follows_the_integration_level() {
        let (loaded, _) = load(Path::new(""));
        let by: BTreeMap<_, _> = loaded
            .iter()
            .map(|l| (l.manifest.kind.as_str(), program(&l.manifest)))
            .collect();
        assert_eq!(by["wrongstack"], Some("wstack"));
        assert_eq!(by["wrongstack-acp"], Some("wstack"));
        assert_eq!(by["antigravity-acp"], Some("agy"));
        // The embedded text is the whole file: ADR-0059 launch options come along.
        for l in &loaded {
            let file =
                std::fs::read_to_string(shipped_dir().join(format!("{}.toml", l.manifest.kind)))
                    .unwrap_or_default();
            if file.contains("[[launch.option]]") {
                assert!(!l.manifest.launch.option.is_empty(), "{}", l.manifest.kind);
            }
        }
    }
}

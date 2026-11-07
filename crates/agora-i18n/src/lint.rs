// i18n-lint: ignore-file
//! i18n lint: keeps Rust key references and locale files consistent (P0-04).
//!
//! Rules:
//! - every `key!("id")` in Rust code must exist in `locales/en/*.ftl` (error);
//! - a non-English locale with an id missing from English has an extra key (error);
//! - a non-English locale missing an English id is a warning.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The source locale directory name.
pub const SOURCE_LOCALE: &str = "en";

/// One lint finding.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Finding {
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Warning,
    Error,
}

/// Extract top-level message and term ids from Fluent source (`id = ...` at column 0).
pub fn ftl_ids(src: &str) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for line in src.lines() {
        let Some((lhs, _)) = line.split_once('=') else {
            continue;
        };
        let id = lhs.trim_end();
        let name = id.strip_prefix('-').unwrap_or(id);
        let mut chars = name.chars();
        let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if valid && !line.starts_with(char::is_whitespace) {
            ids.insert(id.to_string());
        }
    }
    ids
}

/// Marker that excludes a whole file from key scanning (used by the lint's own tests).
pub const IGNORE_FILE_MARKER: &str = "i18n-lint: ignore-file";

fn is_valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Extract ids from `key!("id")` references in Rust source. Comment lines are skipped, as are
/// files containing [`IGNORE_FILE_MARKER`].
pub fn rust_keys(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    if src.contains(IGNORE_FILE_MARKER) {
        return out;
    }
    for line in src.lines().filter(|l| !l.trim_start().starts_with("//")) {
        let mut rest = line;
        while let Some(pos) = rest.find("key!(") {
            rest = &rest[pos + 5..];
            if let Some(lit) = rest.trim_start().strip_prefix('"') {
                if let Some(end) = lit.find('"') {
                    if is_valid_id(&lit[..end]) {
                        out.push(lit[..end].to_string());
                    }
                }
            }
        }
    }
    out
}

fn walk(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) -> io::Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let name = e.file_name();
        if p.is_dir() {
            if name != "target" && name != ".git" {
                walk(&p, ext, out)?;
            }
        } else if p.extension().is_some_and(|x| x == ext) {
            out.push(p);
        }
    }
    Ok(())
}

/// Load ids per locale from `<root>/locales/<lang>/*.ftl`.
pub fn load_locales(root: &Path) -> io::Result<BTreeMap<String, BTreeSet<String>>> {
    let mut map = BTreeMap::new();
    let dir = root.join("locales");
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        if !entry.path().is_dir() {
            continue;
        }
        let mut files = Vec::new();
        walk(&entry.path(), "ftl", &mut files)?;
        let mut ids = BTreeSet::new();
        for f in files {
            ids.extend(ftl_ids(&fs::read_to_string(f)?));
        }
        map.insert(entry.file_name().to_string_lossy().into_owned(), ids);
    }
    Ok(map)
}

/// Compare locales with English and Rust references against English. Pure; no I/O.
pub fn check(
    locales: &BTreeMap<String, BTreeSet<String>>,
    rust_refs: &[(String, String)], // (file, id)
) -> Vec<Finding> {
    let mut findings = Vec::new();
    let empty = BTreeSet::new();
    let en = locales.get(SOURCE_LOCALE).unwrap_or(&empty);
    if !locales.contains_key(SOURCE_LOCALE) {
        findings.push(Finding {
            severity: Severity::Error,
            message: format!("source locale `{SOURCE_LOCALE}` is missing"),
        });
    }
    for (file, id) in rust_refs {
        if !en.contains(id) {
            findings.push(Finding {
                severity: Severity::Error,
                message: format!("{file}: key `{id}` is not defined in locales/{SOURCE_LOCALE}"),
            });
        }
    }
    for (lang, ids) in locales.iter().filter(|(l, _)| *l != SOURCE_LOCALE) {
        for id in ids.difference(en) {
            findings.push(Finding {
                severity: Severity::Error,
                message: format!("locales/{lang}: extra key `{id}` (not in {SOURCE_LOCALE})"),
            });
        }
        for id in en.difference(ids) {
            findings.push(Finding {
                severity: Severity::Warning,
                message: format!("locales/{lang}: missing key `{id}`"),
            });
        }
    }
    findings.sort();
    findings
}

/// Run the lint over a repository checkout.
pub fn run(root: &Path) -> io::Result<Vec<Finding>> {
    let locales = load_locales(root)?;
    let mut files = Vec::new();
    for sub in ["crates", "apps"] {
        if root.join(sub).is_dir() {
            walk(&root.join(sub), "rs", &mut files)?;
        }
    }
    let mut refs = Vec::new();
    for f in files {
        let rel = f.strip_prefix(root).unwrap_or(&f).display().to_string();
        for id in rust_keys(&fs::read_to_string(&f)?) {
            refs.push((rel.clone(), id));
        }
    }
    Ok(check(&locales, &refs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_ids_ignoring_comments_continuations_and_selectors() {
        let src = "# c = x\napp-name = Agora\nn = { $c ->\n    [one] a = b\n   *[other] z\n}\n-brand = B\n";
        assert_eq!(ftl_ids(src), set(&["app-name", "n", "-brand"]));
    }

    #[test]
    fn finds_rust_keys() {
        let src = r#"let a = key!("one"); let b = i18n::key!( "two" ); let c = key!(var);"#;
        assert_eq!(rust_keys(src), vec!["one", "two"]);
    }

    #[test]
    fn missing_key_in_rust_is_an_error() {
        let mut l = BTreeMap::new();
        l.insert("en".to_string(), set(&["a"]));
        let f = check(&l, &[("x.rs".into(), "nope".into())]);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].severity, Severity::Error);
    }

    #[test]
    fn locale_drift_rules() {
        let mut l = BTreeMap::new();
        l.insert("en".to_string(), set(&["a", "b"]));
        l.insert("tr".to_string(), set(&["a", "extra"]));
        let f = check(&l, &[]);
        assert!(f
            .iter()
            .any(|f| f.severity == Severity::Error && f.message.contains("extra")));
        assert!(f
            .iter()
            .any(|f| f.severity == Severity::Warning && f.message.contains("`b`")));
    }

    #[test]
    fn clean_repo_has_no_findings() {
        let mut l = BTreeMap::new();
        l.insert("en".to_string(), set(&["a"]));
        assert!(check(&l, &[("x.rs".into(), "a".into())]).is_empty());
    }
}

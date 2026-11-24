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
    for (number, raw) in src.lines().enumerate() {
        let line = if number == 0 {
            raw.strip_prefix('\u{feff}').unwrap_or(raw)
        } else {
            raw
        };
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

fn char_literal_end(src: &str, start: usize) -> Option<usize> {
    let bytes = src.as_bytes();
    if bytes.get(start) != Some(&b'\'') {
        return None;
    }
    let mut i = start + 1;
    if bytes.get(i) == Some(&b'\\') {
        i += 1;
        match bytes.get(i)? {
            b'x' => i += 3,
            b'u' if bytes.get(i + 1) == Some(&b'{') => {
                i += 2;
                while bytes.get(i).is_some_and(|b| *b != b'}') {
                    i += 1;
                }
                i += usize::from(bytes.get(i) == Some(&b'}'));
            }
            _ => i += src[i..].chars().next()?.len_utf8(),
        }
    } else {
        i += src[i..].chars().next()?.len_utf8();
    }
    (bytes.get(i) == Some(&b'\'')).then_some(i + 1)
}

fn decode_rust_string(s: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            '\'' => out.push('\''),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            '0' => out.push('\0'),
            'x' => {
                let hex = [chars.next()?, chars.next()?]
                    .into_iter()
                    .collect::<String>();
                out.push(char::from(u8::from_str_radix(&hex, 16).ok()?));
            }
            'u' if chars.next()? == '{' => {
                let mut hex = String::new();
                loop {
                    let c = chars.next()?;
                    if c == '}' {
                        break;
                    }
                    if c != '_' {
                        hex.push(c);
                    }
                }
                out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
            }
            '\n' => {
                while chars.peek().is_some_and(|c| c.is_ascii_whitespace()) {
                    chars.next();
                }
            }
            '\r' if chars.next() == Some('\n') => {
                while chars.peek().is_some_and(|c| c.is_ascii_whitespace()) {
                    chars.next();
                }
            }
            _ => return None,
        }
    }
    Some(out)
}

fn without_comments(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    let mut depth = 0_u32;
    while i < bytes.len() {
        if depth > 0 {
            if bytes.get(i..i + 2) == Some(b"/*") {
                depth += 1;
                i += 2;
            } else if bytes.get(i..i + 2) == Some(b"*/") {
                depth -= 1;
                i += 2;
            } else {
                if bytes[i] == b'\n' {
                    out.push('\n');
                }
                i += 1;
            }
            continue;
        }
        if bytes.get(i..i + 2) == Some(b"/*") {
            depth = 1;
            i += 2;
            continue;
        }
        if bytes.get(i..i + 2) == Some(b"//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'r' {
            let mut quote = i + 1;
            while bytes.get(quote) == Some(&b'#') {
                quote += 1;
            }
            if bytes.get(quote) == Some(&b'"') {
                let hashes = quote - i - 1;
                let start = i;
                i = quote + 1;
                while i < bytes.len() {
                    if bytes[i] == b'"'
                        && bytes.get(i + 1..i + 1 + hashes) == Some(&bytes[quote - hashes..quote])
                    {
                        i += 1 + hashes;
                        break;
                    }
                    i += 1;
                }
                out.push_str(&src[start..i]);
                continue;
            }
        }
        if bytes[i] == b'\'' {
            if let Some(end) = char_literal_end(src, i) {
                out.push_str(&src[i..end]);
                i = end;
                continue;
            }
        }
        if bytes[i] == b'"' {
            let start = i;
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(bytes.len());
                } else {
                    let end = bytes[i] == b'"';
                    i += 1;
                    if end {
                        break;
                    }
                }
            }
            out.push_str(&src[start..i]);
            continue;
        }
        let ch = src[i..].chars().next().unwrap_or_default();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Extract ids from `key!("id")` references in Rust source. Comment lines are skipped, as are
/// files containing [`IGNORE_FILE_MARKER`].
pub fn rust_keys(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    if src.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("//") && line.contains(IGNORE_FILE_MARKER)
    }) {
        return out;
    }
    let source = without_comments(src)
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let bytes = source.as_bytes();
    let quoted_end = |start: usize| {
        let mut i = start + 1;
        while i < bytes.len() {
            if bytes[i] == b'\\' {
                i = (i + 2).min(bytes.len());
            } else {
                let end = bytes[i] == b'"';
                i += 1;
                if end {
                    return Some(i);
                }
            }
        }
        None
    };
    let raw_literal = |start: usize| {
        if bytes.get(start) != Some(&b'r') {
            return None;
        }
        let mut quote = start + 1;
        while bytes.get(quote) == Some(&b'#') {
            quote += 1;
        }
        if bytes.get(quote) != Some(&b'"') {
            return None;
        }
        let hashes = quote - start - 1;
        let content = quote + 1;
        let mut i = content;
        while i < bytes.len() {
            if bytes[i] == b'"'
                && bytes.get(i + 1..i + 1 + hashes) == Some(&bytes[quote - hashes..quote])
            {
                return Some((&source[content..i], i + 1 + hashes));
            }
            i += 1;
        }
        None
    };
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\'' {
            if let Some(end) = char_literal_end(&source, i) {
                i = end;
                continue;
            }
        }
        if bytes[i] == b'"' {
            i = quoted_end(i).unwrap_or(bytes.len());
            continue;
        }
        if let Some((_, end)) = raw_literal(i) {
            i = end;
            continue;
        }
        let token_start = source[..i]
            .chars()
            .next_back()
            .is_none_or(|c| c.is_ascii() && !c.is_alphanumeric() && c != '_');
        if token_start && bytes.get(i..i + 3) == Some(b"key") {
            let mut p = i + 3;
            while bytes.get(p).is_some_and(u8::is_ascii_whitespace) {
                p += 1;
            }
            if bytes.get(p) != Some(&b'!') {
                let ch = source[i..].chars().next().unwrap_or_default();
                i += ch.len_utf8();
                continue;
            }
            p += 1;
            while bytes.get(p).is_some_and(u8::is_ascii_whitespace) {
                p += 1;
            }
            if bytes
                .get(p)
                .is_some_and(|b| matches!(b, b'(' | b'[' | b'{'))
            {
                p += 1;
                while bytes.get(p).is_some_and(u8::is_ascii_whitespace) {
                    p += 1;
                }
                let literal = if bytes.get(p) == Some(&b'"') {
                    quoted_end(p).and_then(|end| {
                        decode_rust_string(&source[p + 1..end - 1]).map(|id| (id, end))
                    })
                } else {
                    raw_literal(p).map(|(id, end)| (id.to_string(), end))
                };
                if let Some((id, end)) = literal {
                    if is_valid_id(&id) {
                        out.push(id);
                    }
                    i = end;
                    continue;
                }
            }
        }
        let ch = source[i..].chars().next().unwrap_or_default();
        i += ch.len_utf8();
    }
    out
}

fn walk(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) -> io::Result<()> {
    if fs::symlink_metadata(dir)?.file_type().is_symlink() {
        return Ok(());
    }
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let name = e.file_name();
        let file_type = e.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if name != "target" && name != ".git" && name != "node_modules" {
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
    if !fs::symlink_metadata(&dir)?.file_type().is_dir() {
        return Err(io::Error::from(io::ErrorKind::NotADirectory));
    }
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
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
    for sub in ["crates", "apps", "tools"] {
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

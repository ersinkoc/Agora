//! ADR-0122: the bytes of a shared item (its content, hashed by the caller with SHA-256) and of the owner's signed word on it
//! (`agora-share-item-v1`), and the rules an item must keep. One place for the hub and the node; the web window builds the same bytes
//! (vectors: `schema/fixtures/signing/share-item-v1.json`). Pure: no hashing, no I/O.

use crate::v1::{McpServerDef, ShareItem, ShareItemKind, ShareScope, SkillDef};

/// The most items a hub keeps (live names).
pub const MAX_ITEMS: usize = 64;
/// A skill's files: at most this many, and this many bytes of content in all.
pub const MAX_SKILL_FILES: usize = 64;
pub const MAX_SKILL_BYTES: usize = 1024 * 1024;
/// The most shared items given to one agent at its start.
pub const MAX_PER_AGENT: usize = 32;

const MAX_NAME: usize = 64;
const MAX_COMMAND: usize = 256;
const MAX_ARGS: usize = 32;
const MAX_ARG: usize = 1024;
const MAX_VALUES: usize = 32;
const MAX_VALUE_NAME: usize = 64;
const MAX_VALUE: usize = 4096;
const MAX_URL: usize = 2048;
const MAX_DESCRIPTION: usize = 512;
const MAX_PATH: usize = 255;
const MAX_PROJECT: usize = 256;
const MAX_KINDS: usize = 16;
const MAX_KIND: usize = 64;

/// The action a signed word takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Publish,
    Remove,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Action::Publish => "publish",
            Action::Remove => "remove",
        }
    }
}

/// F(x): the decimal byte length, `:`, then x.
fn field(out: &mut Vec<u8>, x: &[u8]) {
    out.extend_from_slice(x.len().to_string().as_bytes());
    out.push(b':');
    out.extend_from_slice(x);
}

/// ADR-0059's list: the count, then `:<len>:<id>` per id in the order given.
fn list<S: AsRef<str>>(items: &[S]) -> String {
    let mut s = items.len().to_string();
    for i in items {
        let i = i.as_ref();
        s.push_str(&format!(":{}:{i}", i.len()));
    }
    s
}

/// ADR-0066's pairs: the count, then `:<len>:<a>:<len>:<b>` per pair in the order given.
fn pairs(items: &[(&str, &str)]) -> String {
    let mut s = items.len().to_string();
    for (a, b) in items {
        s.push_str(&format!(":{}:{a}:{}:{b}", a.len(), b.len()));
    }
    s
}

fn kind_word(kind: i32) -> &'static str {
    match ShareItemKind::try_from(kind) {
        Ok(ShareItemKind::McpServer) => "mcp_server",
        Ok(ShareItemKind::Skill) => "skill",
        _ => "",
    }
}

/// The bytes an item's content hash is taken over: `agora-share-item-content-v1`, NUL, F(kind) F(name), then the kind's fields
/// (env and headers sorted by name, files sorted by path, each file as its path and `file_hash(content)`). `file_hash` is the caller's
/// SHA-256 as lowercase hex.
pub fn content_bytes(item: &ShareItem, file_hash: impl Fn(&[u8]) -> String) -> Vec<u8> {
    let mut out = b"agora-share-item-content-v1\0".to_vec();
    field(&mut out, kind_word(item.kind).as_bytes());
    field(&mut out, item.name.as_bytes());
    match ShareItemKind::try_from(item.kind) {
        Ok(ShareItemKind::McpServer) => {
            let m = item.mcp.clone().unwrap_or_default();
            let sorted = |v: &[crate::v1::ShareValue]| {
                let mut p: Vec<(&str, &str)> = v
                    .iter()
                    .map(|x| (x.name.as_str(), x.value.as_str()))
                    .collect();
                p.sort();
                pairs(&p)
            };
            field(&mut out, m.command.as_bytes());
            field(&mut out, list(&m.args).as_bytes());
            field(&mut out, sorted(&m.env).as_bytes());
            field(&mut out, m.url.as_bytes());
            field(&mut out, sorted(&m.headers).as_bytes());
        }
        Ok(ShareItemKind::Skill) => {
            let s = item.skill.clone().unwrap_or_default();
            field(&mut out, s.description.as_bytes());
            let mut files: Vec<(String, String)> = s
                .files
                .iter()
                .map(|f| (f.path.clone(), file_hash(&f.content)))
                .collect();
            files.sort();
            let refs: Vec<(&str, &str)> = files
                .iter()
                .map(|(a, b)| (a.as_str(), b.as_str()))
                .collect();
            field(&mut out, pairs(&refs).as_bytes());
        }
        _ => {}
    }
    out
}

/// The signed bytes of the owner's word on a name: `agora-share-item-v1`, NUL, then F(action) F(name) F(item_hash) F(project_id)
/// F(list(kinds)) F(issued_ms). A removal has an empty `item_hash`.
pub fn signed_message(
    action: Action,
    name: &str,
    item_hash: &str,
    scope: &ShareScope,
    issued_ms: u64,
) -> Vec<u8> {
    let mut out = b"agora-share-item-v1\0".to_vec();
    field(&mut out, action.as_str().as_bytes());
    field(&mut out, name.as_bytes());
    field(&mut out, item_hash.as_bytes());
    field(&mut out, scope.project_id.as_bytes());
    field(&mut out, list(&scope.kinds).as_bytes());
    field(&mut out, issued_ms.to_string().as_bytes());
    out
}

fn plain(s: &str) -> bool {
    !s.chars().any(|c| c.is_control())
}

/// A name, or a kind id: `[a-z0-9-]`, 1 to `max` bytes.
pub fn valid_name(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The rule an item breaks, as the field it names; `Ok` when it keeps them all.
pub fn check_item(item: &ShareItem) -> Result<(), &'static str> {
    if !valid_name(&item.name, MAX_NAME) {
        return Err("name");
    }
    match ShareItemKind::try_from(item.kind) {
        Ok(ShareItemKind::McpServer) if item.skill.is_none() => {
            check_mcp(item.mcp.as_ref().ok_or("mcp")?)
        }
        Ok(ShareItemKind::Skill) if item.mcp.is_none() => {
            check_skill(item.skill.as_ref().ok_or("skill")?)
        }
        _ => Err("kind"),
    }
}

fn check_values(v: &[crate::v1::ShareValue], what: &'static str) -> Result<(), &'static str> {
    let mut seen = std::collections::HashSet::new();
    if v.len() > MAX_VALUES {
        return Err(what);
    }
    for x in v {
        let name_ok = !x.name.is_empty()
            && x.name.len() <= MAX_VALUE_NAME
            && x.name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if !name_ok
            || x.value.len() > MAX_VALUE
            || !plain(&x.value)
            || !seen.insert(x.name.as_str())
        {
            return Err(what);
        }
    }
    Ok(())
}

fn check_mcp(m: &McpServerDef) -> Result<(), &'static str> {
    match (m.command.is_empty(), m.url.is_empty()) {
        // A program: no URL or headers.
        (false, true) => {
            if m.command.len() > MAX_COMMAND || !plain(&m.command) || !m.headers.is_empty() {
                return Err("command");
            }
            // A secret is never put on a command line (every user of the machine can read it, /proc/<pid>/cmdline).
            if m.args.len() > MAX_ARGS
                || m.args
                    .iter()
                    .any(|a| a.len() > MAX_ARG || a.contains('\0') || a.contains("{secret:"))
            {
                return Err("args");
            }
            check_values(&m.env, "env")
        }
        // A remote server over HTTPS: no program, arguments or environment.
        (true, false) => {
            if m.url.len() > MAX_URL
                || !m.url.starts_with("https://")
                || !plain(&m.url)
                || m.url.contains(char::is_whitespace)
            {
                return Err("url");
            }
            if !m.args.is_empty() || !m.env.is_empty() {
                return Err("args");
            }
            check_values(&m.headers, "headers")
        }
        _ => Err("command"),
    }
}

/// A file's path inside the skill's folder: relative, `/`-separated parts of `[A-Za-z0-9._-]`, none of them `.` or `..`.
pub fn valid_skill_path(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= MAX_PATH
        && p.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
        })
}

fn check_skill(s: &SkillDef) -> Result<(), &'static str> {
    if s.description.len() > MAX_DESCRIPTION || !plain(&s.description) {
        return Err("description");
    }
    if s.files.is_empty() || s.files.len() > MAX_SKILL_FILES {
        return Err("files");
    }
    let mut total = 0usize;
    let mut seen = std::collections::HashSet::new();
    for f in &s.files {
        if !valid_skill_path(&f.path)
            || !seen.insert(f.path.to_ascii_lowercase())
            || std::str::from_utf8(&f.content).is_err()
        {
            return Err("files");
        }
        total = total.saturating_add(f.content.len());
    }
    if total > MAX_SKILL_BYTES {
        return Err("files");
    }
    Ok(())
}

/// The rule a scope breaks, as the field it names.
pub fn check_scope(scope: &ShareScope) -> Result<(), &'static str> {
    if scope.project_id.len() > MAX_PROJECT || !plain(&scope.project_id) {
        return Err("project_id");
    }
    if scope.kinds.len() > MAX_KINDS || scope.kinds.iter().any(|k| !valid_name(k, MAX_KIND)) {
        return Err("kinds");
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::v1::{ShareValue, SkillFile};

    fn mcp(command: &str, url: &str) -> ShareItem {
        ShareItem {
            name: "db-tools".into(),
            kind: ShareItemKind::McpServer as i32,
            mcp: Some(McpServerDef {
                command: command.into(),
                url: url.into(),
                ..Default::default()
            }),
            skill: None,
        }
    }

    fn skill(paths: &[&str]) -> ShareItem {
        ShareItem {
            name: "review".into(),
            kind: ShareItemKind::Skill as i32,
            mcp: None,
            skill: Some(SkillDef {
                description: "d".into(),
                files: paths
                    .iter()
                    .map(|p| SkillFile {
                        path: (*p).into(),
                        content: b"x".to_vec(),
                    })
                    .collect(),
            }),
        }
    }

    #[test]
    fn an_item_keeps_its_rules() {
        assert_eq!(check_item(&mcp("npx", "")), Ok(()));
        assert_eq!(check_item(&mcp("", "https://mcp.example.com/sse")), Ok(()));
        assert_eq!(check_item(&mcp("", "http://mcp.example.com")), Err("url"));
        assert_eq!(
            check_item(&mcp("npx", "https://x")),
            Err("command"),
            "a program or a URL, never both"
        );
        assert_eq!(check_item(&mcp("", "")), Err("command"));
        let mut bad = mcp("npx", "");
        bad.name = "DB Tools".into();
        assert_eq!(check_item(&bad), Err("name"));
        let mut env = mcp("npx", "");
        env.mcp.as_mut().unwrap().env = vec![
            ShareValue {
                name: "A".into(),
                value: "1".into(),
            },
            ShareValue {
                name: "A".into(),
                value: "2".into(),
            },
        ];
        assert_eq!(check_item(&env), Err("env"), "a name twice");
        let mut on_the_command_line = mcp("npx", "");
        on_the_command_line.mcp.as_mut().unwrap().args = vec!["--token={secret:T}".into()];
        assert_eq!(
            check_item(&on_the_command_line),
            Err("args"),
            "a secret on a command line"
        );
        assert_eq!(check_item(&skill(&["SKILL.md", "notes/tr.md"])), Ok(()));
        for p in ["../x", "/etc/passwd", "a//b", "a/./b", "a\\b", "", "a b"] {
            assert_eq!(check_item(&skill(&[p])), Err("files"), "{p:?}");
        }
        assert_eq!(
            check_item(&skill(&["A.md", "a.md"])),
            Err("files"),
            "two paths a case-blind disk takes for one"
        );
        assert_eq!(check_item(&skill(&[])), Err("files"));
        let mut both = skill(&["SKILL.md"]);
        both.mcp = Some(McpServerDef::default());
        assert_eq!(check_item(&both), Err("kind"));
    }

    #[test]
    fn a_scope_keeps_its_rules() {
        assert_eq!(check_scope(&ShareScope::default()), Ok(()));
        let s = ShareScope {
            project_id: "github.com/x/y".into(),
            kinds: vec!["claude-code".into()],
        };
        assert_eq!(check_scope(&s), Ok(()));
        let s = ShareScope {
            project_id: String::new(),
            kinds: vec!["Claude".into()],
        };
        assert_eq!(check_scope(&s), Err("kinds"));
    }
}

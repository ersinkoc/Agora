//! A plugin package, checked as the window checks it (ADR-0114 decisions 5 and 6; docs/ux/plugins.md, "The package"). This is a port
//! of `apps/web/src/plugins/package.ts`: the same rules, the same order and the same refusal codes, so the hub refuses at install what a
//! window would refuse at load, and keeps nothing it would not run. Both are held in agreement by the shared fixtures in
//! `crates/agora-hub/tests/fixtures/plugin-packages/` (`expected.json`, read by `tests/plugin_package_agreement.rs` and by
//! `apps/web/src/plugins/package.test.ts`).
//!
//! The hub never runs a package: it checks the manifest and the hashes, then keeps the bytes as they came.
//!
//! The window reads the package with `TextDecoder` (fatal, which drops one leading byte-order mark) and `JSON.parse`, so this reads it
//! the same way, with a JSON reader of its own rather than serde's:
//! - a key given twice keeps its last value, in the place it first had;
//! - keys come in JavaScript's order: array indexes (`"0"`, `"17"`) first, ascending, then the others as written;
//! - a number is a double (`1.0` and `1e0` are `1`, `1e400` is infinite);
//! - a string may hold a lone surrogate (`"\ud800"`), which no check accepts;
//! - the lengths of texts count UTF-16 units, as JavaScript's `length` does.
//!
//! Base64 is decoded as `atob` does once the window's own pattern has passed: padding is optional only as the pattern allows, and bits
//! left over in the last character are ignored.

use sha2::{Digest, Sha256};

/// The largest package, in bytes.
pub const MAX_PACKAGE_BYTES: usize = 2 * 1024 * 1024;
/// The most files besides the manifest.
pub const MAX_FILES: usize = 64;
pub const MANIFEST: &str = "agora-plugin.json";
pub const BUNDLE_VERSION: f64 = 1.0;
/// The rung-1 capabilities (`apps/web/src/plugins/capabilities.ts`), the only names a manifest may ask for.
pub const CAPABILITIES: &[&str] = &[
    "agents.read",
    "usage.read",
    "tasks.read",
    "changes.read",
    "chat.read",
    "theme.read",
    "pane.context",
];

/// Why a package was refused: the window's codes (`PackageError` in `package.ts`), worded by `plugin-error-*` (window) and
/// `cli-hub-plugin-error-*` (the hub's command line).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    TooLarge,
    Format,
    NewerFormat,
    TooMany,
    Path,
    Link,
    Type,
    Manifest,
    Capability,
    Unlisted,
    Missing,
    Hash,
    Newer,
}

impl Refusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TooLarge => "too-large",
            Self::Format => "format",
            Self::NewerFormat => "newer-format",
            Self::TooMany => "too-many",
            Self::Path => "path",
            Self::Link => "link",
            Self::Type => "type",
            Self::Manifest => "manifest",
            Self::Capability => "capability",
            Self::Unlisted => "unlisted",
            Self::Missing => "missing",
            Self::Hash => "hash",
            Self::Newer => "newer",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub code: Refusal,
    /// What it was refused at (a file name, a manifest field), as the window says it; `None` where the window gives none.
    pub detail: Option<String>,
}

fn refuse(code: Refusal) -> Refused {
    Refused { code, detail: None }
}

fn refuse_at(code: Refusal, detail: impl Into<String>) -> Refused {
    Refused {
        code,
        detail: Some(detail.into()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelDecl {
    pub id: String,
    /// `global` or `tool`.
    pub kind: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub licence: String,
    pub min_agora: String,
    pub panels: Vec<PanelDecl>,
    /// In the manifest's order, each once.
    pub capabilities: Vec<String>,
    pub entry: String,
    /// Every file but the manifest, with its SHA-256 in lowercase hex, in the window's order.
    pub files: Vec<(String, String)>,
}

/// A package that passed: its manifest, and the SHA-256 of the whole package (what windows fetch it by).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    pub manifest: Manifest,
    pub sha256: String,
}

/// Checks a package's bytes against its own list and `agora_version` (the oldest Agora the plugin may ask for is this one). Nothing of a
/// package that fails is kept.
pub fn check(bytes: &[u8], agora_version: &str) -> Result<Checked, Refused> {
    if bytes.len() > MAX_PACKAGE_BYTES {
        return Err(refuse(Refusal::TooLarge));
    }
    let top = decode_json(bytes).ok_or(refuse(Refusal::Format))?;
    let top = match &top {
        J::Obj(o) if o.len() == 2 && get(o, "agora-plugin-bundle").is_some() => o,
        _ => return Err(refuse(Refusal::Format)),
    };
    let Some(files_value) = get(top, "files") else {
        return Err(refuse(Refusal::Format));
    };
    let ver = get(top, "agora-plugin-bundle");
    if let Some(J::Num(n)) = ver {
        if is_integer(*n) && *n > BUNDLE_VERSION {
            return Err(refuse(Refusal::NewerFormat));
        }
    }
    let entries = match (ver, files_value) {
        (Some(J::Num(n)), J::Obj(f)) if *n == BUNDLE_VERSION => f,
        _ => return Err(refuse(Refusal::Format)),
    };
    if entries.len() > MAX_FILES + 1 {
        return Err(refuse(Refusal::TooMany));
    }
    let mut files: Vec<(String, Vec<u8>)> = Vec::with_capacity(entries.len());
    for (name, value) in entries {
        let shown = name.lossy();
        if !is_plain_name(name) {
            return Err(refuse_at(Refusal::Path, name.prefix(100)));
        }
        let J::Str(text) = value else {
            return Err(refuse_at(Refusal::Link, shown));
        };
        if shown != MANIFEST && type_of(&shown).is_none() {
            return Err(refuse_at(Refusal::Type, shown));
        }
        let Some(b) = base64(text) else {
            return Err(refuse_at(Refusal::Format, shown));
        };
        files.push((shown, b));
    }
    let Some(at) = files.iter().position(|(n, _)| n == MANIFEST) else {
        return Err(refuse_at(Refusal::Manifest, MANIFEST));
    };
    let (_, raw) = files.remove(at);
    let json = decode_json(&raw).ok_or_else(|| refuse_at(Refusal::Manifest, MANIFEST))?;
    let manifest = read_manifest(&json)?;
    for (name, _) in &files {
        if !manifest.files.iter().any(|(n, _)| n == name) {
            return Err(refuse_at(Refusal::Unlisted, name.clone()));
        }
    }
    for (name, _) in &manifest.files {
        if !files.iter().any(|(n, _)| n == name) {
            return Err(refuse_at(Refusal::Missing, name.clone()));
        }
    }
    for (name, b) in &files {
        let want = manifest
            .files
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, h)| h.as_str());
        if want != Some(hex(&Sha256::digest(b)).as_str()) {
            return Err(refuse_at(Refusal::Hash, name.clone()));
        }
    }
    if compare_versions(&manifest.min_agora, agora_version) == std::cmp::Ordering::Greater {
        return Err(refuse_at(Refusal::Newer, manifest.min_agora.clone()));
    }
    Ok(Checked {
        manifest,
        sha256: hex(&Sha256::digest(bytes)),
    })
}

const MANIFEST_KEYS: &[&str] = &[
    "id",
    "name",
    "version",
    "author",
    "licence",
    "min_agora",
    "panels",
    "capabilities",
    "entry",
    "files",
];
const PANEL_KEYS: &[&str] = &["id", "kind", "name"];

/// The manifest as a plugin wrote it, checked field by field in the window's order. An unknown field is refused: a newer manifest may
/// ask for something this hub does not know how to keep a plugin out of.
fn read_manifest(v: &J) -> Result<Manifest, Refused> {
    let bad = |detail: &str| refuse_at(Refusal::Manifest, detail);
    let J::Obj(o) = v else {
        return Err(bad("manifest"));
    };
    for (k, _) in o {
        let k = k.lossy();
        if !MANIFEST_KEYS.contains(&k.as_str()) {
            return Err(refuse_at(Refusal::Manifest, k));
        }
    }
    let str_of = |key: &str| match get(o, key) {
        Some(J::Str(s)) => Some(s),
        _ => None,
    };
    let id = str_of("id")
        .and_then(|s| s.ascii())
        .filter(|s| is_id(s, 40))
        .ok_or_else(|| bad("id"))?;
    let name = plain_text(get(o, "name"), 60).ok_or_else(|| bad("name"))?;
    let version = str_of("version")
        .and_then(|s| s.ascii())
        .filter(|s| is_version(s))
        .ok_or_else(|| bad("version"))?;
    let author = plain_text(get(o, "author"), 100).ok_or_else(|| bad("author"))?;
    let licence = plain_text(get(o, "licence"), 60).ok_or_else(|| bad("licence"))?;
    let min_agora = str_of("min_agora")
        .and_then(|s| s.ascii())
        .filter(|s| is_version(s))
        .ok_or_else(|| bad("min_agora"))?;
    let mut panels: Vec<PanelDecl> = Vec::new();
    match get(o, "panels") {
        Some(J::Arr(list)) if (1..=4).contains(&list.len()) => {
            for p in list {
                let J::Obj(p) = p else {
                    return Err(bad("panels"));
                };
                if p.iter()
                    .any(|(k, _)| !PANEL_KEYS.contains(&k.lossy().as_str()))
                {
                    return Err(bad("panels"));
                }
                let pid = match get(p, "id") {
                    Some(J::Str(s)) => s.ascii().filter(|s| is_id(s, 20)),
                    _ => None,
                }
                .filter(|pid| !panels.iter().any(|x| &x.id == pid))
                .ok_or_else(|| bad("panels"))?;
                let kind = match get(p, "kind") {
                    None => "global".to_string(),
                    Some(J::Str(s)) if s.is("global") || s.is("tool") => s.lossy(),
                    Some(_) => return Err(bad("panels")),
                };
                let pname = match get(p, "name") {
                    None => None,
                    v => Some(plain_text(v, 60).ok_or_else(|| bad("panels"))?),
                };
                panels.push(PanelDecl {
                    id: pid,
                    kind,
                    name: pname,
                });
            }
        }
        _ => return Err(bad("panels")),
    }
    let mut capabilities: Vec<String> = Vec::new();
    match get(o, "capabilities") {
        Some(J::Arr(list)) if list.len() <= 16 => {
            for c in list {
                let known = match c {
                    J::Str(s) => CAPABILITIES.iter().find(|k| s.is(k)),
                    _ => None,
                };
                let Some(known) = known else {
                    let detail = match c {
                        J::Str(s) => s.prefix(40),
                        _ => "?".to_string(),
                    };
                    return Err(refuse_at(Refusal::Capability, detail));
                };
                if !capabilities.iter().any(|x| x == known) {
                    capabilities.push((*known).to_string());
                }
            }
        }
        _ => return Err(bad("capabilities")),
    }
    let Some(J::Obj(listed)) = get(o, "files") else {
        return Err(bad("files"));
    };
    let mut files: Vec<(String, String)> = Vec::new();
    for (name, hash) in listed {
        let ok_name = is_plain_name(name) && !name.is(MANIFEST);
        let hash = match hash {
            J::Str(h) => h.ascii().filter(|h| is_hash(h)),
            _ => None,
        };
        match (ok_name, hash) {
            (true, Some(h)) => files.push((name.lossy(), h)),
            _ => return Err(bad("files")),
        }
    }
    let entry = str_of("entry")
        .map(|s| s.lossy())
        .filter(|e| files.iter().any(|(n, _)| n == e) && type_of(e) == Some("text/javascript"))
        .ok_or_else(|| bad("entry"))?;
    Ok(Manifest {
        id,
        name,
        version,
        author,
        licence,
        min_agora,
        panels,
        capabilities,
        entry,
        files,
    })
}

/// Orders two `x.y.z` versions by their numbers.
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |s: &str| -> Vec<u64> { s.split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    let (pa, pb) = (parts(a), parts(b));
    for i in 0..3 {
        let o = pa
            .get(i)
            .copied()
            .unwrap_or(0)
            .cmp(&pb.get(i).copied().unwrap_or(0));
        if o != std::cmp::Ordering::Equal {
            return o;
        }
    }
    std::cmp::Ordering::Equal
}

/// What a file may be, by its extension (`TYPES` in `package.ts`).
pub fn type_of(name: &str) -> Option<&'static str> {
    let dot = name.rfind('.')?;
    if dot == 0 {
        return None;
    }
    Some(match name[dot + 1..].to_ascii_lowercase().as_str() {
        "js" => "text/javascript",
        "css" => "text/css",
        "json" => "application/json",
        "txt" => "text/plain",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => return None,
    })
}

fn word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// `^[A-Za-z0-9][A-Za-z0-9_-]*(\.[A-Za-z0-9_-]+)*$`, at most 100: a file name and nothing more.
fn is_plain_name(s: &JStr) -> bool {
    let Some(s) = s.ascii() else { return false };
    let b = s.as_bytes();
    if b.len() > 100 || b.is_empty() || !b[0].is_ascii_alphanumeric() {
        return false;
    }
    let mut segments = s.split('.');
    let first_ok = segments.next().is_some_and(|f| f.bytes().all(word_char));
    first_ok && segments.all(|seg| !seg.is_empty() && seg.bytes().all(word_char))
}

/// `^[a-z0-9][a-z0-9-]{0,max-1}$`.
fn is_id(s: &str, max: usize) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= max
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// `^(0|[1-9]\d{0,5})\.(0|[1-9]\d{0,5})\.(0|[1-9]\d{0,5})$`.
fn is_version(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|p| {
            let b = p.as_bytes();
            !b.is_empty()
                && b.len() <= 6
                && b.iter().all(u8::is_ascii_digit)
                && (b[0] != b'0' || b.len() == 1)
        })
}

fn is_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

/// `UNSAFE_TEXT` in `package.ts`: control characters, and the invisible ones (bidi marks among them) that could make a plugin's text
/// pose as the window's words or reorder what stands beside it.
fn unsafe_unit(u: u16) -> bool {
    matches!(u,
        0x0000..=0x001f | 0x007f..=0x009f | 0x00ad | 0x061c | 0x180e | 0x200b..=0x200f | 0x202a..=0x202e | 0x2060..=0x206f | 0xfeff
        | 0xfff9..=0xfffb)
}

/// JavaScript's `String.prototype.trim` white space (WhiteSpace and LineTerminator).
fn js_space(u: u16) -> bool {
    matches!(u,
        0x0009..=0x000d | 0x0020 | 0x00a0 | 0x1680 | 0x2000..=0x200a | 0x2028 | 0x2029 | 0x202f | 0x205f | 0x3000 | 0xfeff)
}

/// `plainText` in `package.ts`: one line of text, trimmed, 1 to `max` UTF-16 units, none unsafe, and (as the window also holds) no lone
/// surrogate.
fn plain_text(v: Option<&J>, max: usize) -> Option<String> {
    let Some(J::Str(s)) = v else { return None };
    let u = &s.0;
    let ok = !u.is_empty()
        && u.len() <= max
        && !js_space(u[0])
        && !js_space(u[u.len() - 1])
        && !u.iter().any(|c| unsafe_unit(*c))
        && s.well_formed();
    ok.then(|| s.lossy())
}

fn is_integer(n: f64) -> bool {
    n.is_finite() && n.fract() == 0.0
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The window's base64: its pattern (`^[A-Za-z0-9+/]*={0,2}$`, a length a multiple of 4), then `atob`, which ignores bits left over.
fn base64(s: &JStr) -> Option<Vec<u8>> {
    let s = s.ascii()?;
    let b = s.as_bytes();
    if b.len() % 4 != 0 {
        return None;
    }
    let body_len = b.len() - b.iter().rev().take_while(|c| **c == b'=').count();
    if b.len() - body_len > 2 {
        return None;
    }
    let value = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let mut out = Vec::with_capacity(body_len * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &c in &b[..body_len] {
        acc = (acc << 6) | value(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

// ---- the JSON reader: JSON.parse's results, with JavaScript's strings and key order ----

/// A string as JavaScript holds it: UTF-16 units, a lone surrogate allowed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct JStr(Vec<u16>);

impl JStr {
    fn lossy(&self) -> String {
        String::from_utf16_lossy(&self.0)
    }
    fn is(&self, s: &str) -> bool {
        self.0.iter().copied().eq(s.encode_utf16())
    }
    fn ascii(&self) -> Option<String> {
        self.0
            .iter()
            .all(|u| *u < 0x80)
            .then(|| self.0.iter().map(|u| char::from(*u as u8)).collect())
    }
    fn well_formed(&self) -> bool {
        char::decode_utf16(self.0.iter().copied()).all(|c| c.is_ok())
    }
    /// The first `n` UTF-16 units, as `slice(0, n)` gives them.
    fn prefix(&self, n: usize) -> String {
        String::from_utf16_lossy(&self.0[..self.0.len().min(n)])
    }
    /// An array index as JavaScript orders property keys: `0`, or a decimal with no leading zero, below 2^32 - 1.
    fn array_index(&self) -> Option<u32> {
        let s = self.ascii()?;
        if s.is_empty()
            || (s.len() > 1 && s.starts_with('0'))
            || !s.bytes().all(|c| c.is_ascii_digit())
        {
            return None;
        }
        s.parse::<u32>().ok().filter(|n| *n != u32::MAX)
    }
}

#[derive(Debug, Clone, PartialEq)]
enum J {
    Null,
    Bool(bool),
    Num(f64),
    Str(JStr),
    Arr(Vec<J>),
    Obj(Vec<(JStr, J)>),
}

fn get<'a>(o: &'a [(JStr, J)], key: &str) -> Option<&'a J> {
    o.iter().find(|(k, _)| k.is(key)).map(|(_, v)| v)
}

/// The most objects and arrays open at once. The window refuses the same text before `JSON.parse` (`MAX_JSON_DEPTH` in `package.ts`,
/// counted on the text, so a deep value under a key given twice is refused even though `JSON.parse` would drop it): the two agree on
/// every package, with the same code. Nothing a package may hold nests deeper than three.
const MAX_DEPTH: usize = 64;

/// `JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes))`, or `None` where that throws.
fn decode_json(bytes: &[u8]) -> Option<J> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let text = std::str::from_utf8(bytes).ok()?;
    let mut p = Parser {
        s: text.as_bytes(),
        at: 0,
    };
    let v = p.value(0)?;
    p.space();
    (p.at == p.s.len()).then_some(v)
}

struct Parser<'a> {
    s: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.at).copied()
    }
    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }
    fn eat(&mut self, word: &[u8]) -> Option<()> {
        self.s[self.at..]
            .starts_with(word)
            .then(|| self.at += word.len())
    }
    /// A value inside `depth` open objects and arrays; one more would make more than [`MAX_DEPTH`].
    fn value(&mut self, depth: usize) -> Option<J> {
        self.space();
        match self.peek()? {
            b'{' | b'[' if depth >= MAX_DEPTH => None,
            b'{' => self.object(depth),
            b'[' => self.array(depth),
            b'"' => self.string().map(J::Str),
            b't' => self.eat(b"true").map(|()| J::Bool(true)),
            b'f' => self.eat(b"false").map(|()| J::Bool(false)),
            b'n' => self.eat(b"null").map(|()| J::Null),
            _ => self.number().map(J::Num),
        }
    }
    fn object(&mut self, depth: usize) -> Option<J> {
        self.at += 1;
        let mut out: Vec<(JStr, J)> = Vec::new();
        // Where each key is in `out`, so a 2 MiB object of distinct keys is read in linear time, not quadratic.
        let mut place: std::collections::HashMap<JStr, usize> = std::collections::HashMap::new();
        self.space();
        if self.peek()? == b'}' {
            self.at += 1;
            return Some(J::Obj(out));
        }
        loop {
            self.space();
            if self.peek()? != b'"' {
                return None;
            }
            let k = self.string()?;
            self.space();
            if self.peek()? != b':' {
                return None;
            }
            self.at += 1;
            let v = self.value(depth + 1)?;
            // A key given again keeps its first place and takes the new value (JSON.parse's CreateDataProperty).
            match place.get(&k) {
                Some(&i) => out[i].1 = v,
                None => {
                    place.insert(k.clone(), out.len());
                    out.push((k, v));
                }
            }
            self.space();
            match self.peek()? {
                b',' => self.at += 1,
                b'}' => {
                    self.at += 1;
                    break;
                }
                _ => return None,
            }
        }
        // JavaScript's own key order: array indexes first, ascending, then the rest as they came.
        let (mut idx, rest): (Vec<_>, Vec<_>) = out
            .into_iter()
            .partition(|(k, _)| k.array_index().is_some());
        idx.sort_by_key(|(k, _)| k.array_index());
        idx.extend(rest);
        Some(J::Obj(idx))
    }
    fn array(&mut self, depth: usize) -> Option<J> {
        self.at += 1;
        let mut out = Vec::new();
        self.space();
        if self.peek()? == b']' {
            self.at += 1;
            return Some(J::Arr(out));
        }
        loop {
            out.push(self.value(depth + 1)?);
            self.space();
            match self.peek()? {
                b',' => self.at += 1,
                b']' => {
                    self.at += 1;
                    return Some(J::Arr(out));
                }
                _ => return None,
            }
        }
    }
    fn hex4(&mut self) -> Option<u16> {
        let h = self.s.get(self.at..self.at + 4)?;
        let h = std::str::from_utf8(h).ok()?;
        if !h.bytes().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        self.at += 4;
        u16::from_str_radix(h, 16).ok()
    }
    fn string(&mut self) -> Option<JStr> {
        self.at += 1;
        let mut out: Vec<u16> = Vec::new();
        loop {
            let c = self.peek()?;
            match c {
                b'"' => {
                    self.at += 1;
                    return Some(JStr(out));
                }
                b'\\' => {
                    self.at += 1;
                    let e = self.peek()?;
                    self.at += 1;
                    out.push(match e {
                        b'"' => 0x22,
                        b'\\' => 0x5c,
                        b'/' => 0x2f,
                        b'b' => 0x08,
                        b'f' => 0x0c,
                        b'n' => 0x0a,
                        b'r' => 0x0d,
                        b't' => 0x09,
                        b'u' => self.hex4()?,
                        _ => return None,
                    });
                }
                0x00..=0x1f => return None,
                _ => {
                    // One whole UTF-8 character (the text is valid UTF-8, so its length is known from the first byte).
                    let len = match c {
                        0x00..=0x7f => 1,
                        0xc0..=0xdf => 2,
                        0xe0..=0xef => 3,
                        _ => 4,
                    };
                    let ch = std::str::from_utf8(self.s.get(self.at..self.at + len)?).ok()?;
                    out.extend(ch.encode_utf16());
                    self.at += len;
                }
            }
        }
    }
    fn number(&mut self) -> Option<f64> {
        let start = self.at;
        let digits = |p: &mut Self| {
            let from = p.at;
            while p.peek().is_some_and(|c| c.is_ascii_digit()) {
                p.at += 1;
            }
            p.at - from
        };
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        match self.peek()? {
            b'0' => self.at += 1,
            b'1'..=b'9' => {
                digits(self);
            }
            _ => return None,
        }
        if self.peek() == Some(b'.') {
            self.at += 1;
            if digits(self) == 0 {
                return None;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if digits(self) == 0 {
                return None;
            }
        }
        // Rust parses a decimal to the nearest double, as JavaScript does; one too large is infinite, as there.
        std::str::from_utf8(&self.s[start..self.at])
            .ok()?
            .parse::<f64>()
            .ok()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn obj(text: &str) -> Vec<(String, J)> {
        match decode_json(text.as_bytes()).expect("json") {
            J::Obj(o) => o.into_iter().map(|(k, v)| (k.lossy(), v)).collect(),
            _ => panic!("not an object"),
        }
    }

    #[test]
    fn reads_json_as_javascript_does() {
        // The last of a key given twice, in its first place; array indexes first.
        let o = obj(r#"{"b":1,"a":2,"b":3,"10":4,"2":5,"01":6}"#);
        let keys: Vec<&str> = o.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["2", "10", "b", "a", "01"]);
        assert_eq!(o[2].1, J::Num(3.0));
        // Numbers are doubles; one too large is infinite.
        assert_eq!(decode_json(b"1e0"), Some(J::Num(1.0)));
        assert_eq!(decode_json(b"1e400"), Some(J::Num(f64::INFINITY)));
        assert_eq!(decode_json(b"-0.5E+1"), Some(J::Num(-5.0)));
        for bad in [
            &b"01"[..],
            b"1.",
            b".5",
            b"+1",
            b"1e",
            b"[1,]",
            b"{\"a\":1,}",
            b"'a'",
            b"\"\x01\"",
            b"nul",
            b"1 2",
        ] {
            assert_eq!(decode_json(bad), None, "{:?}", String::from_utf8_lossy(bad));
        }
        // A lone surrogate is a string, as in JavaScript; a byte-order mark is dropped once, as TextDecoder does.
        let Some(J::Str(s)) = decode_json(br#""\ud800x""#) else {
            panic!()
        };
        assert!(!s.well_formed());
        assert_eq!(decode_json(b"\xEF\xBB\xBF1"), Some(J::Num(1.0)));
        assert_eq!(decode_json(b"\xEF\xBB\xBF\xEF\xBB\xBF1"), None);
        assert_eq!(decode_json(b"\"\xff\""), None);
        // An object of 200,000 distinct keys is read at once (the duplicate check is not quadratic).
        let many = format!(
            "{{{}}}",
            (0..200_000)
                .map(|i| format!("\"k{i}\":0"))
                .collect::<Vec<_>>()
                .join(",")
        );
        let started = std::time::Instant::now();
        assert!(matches!(decode_json(many.as_bytes()), Some(J::Obj(o)) if o.len() == 200_000));
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        // 64 open at once are read; 65 are refused, as by the window.
        let nest = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
        assert!(decode_json(nest(64).as_bytes()).is_some());
        assert_eq!(decode_json(nest(65).as_bytes()), None);
        assert!(decode_json(format!("{}1{}", "[".repeat(64), "]".repeat(64)).as_bytes()).is_some());
        // Nesting past the reader's depth is refused, never a stack overflow.
        let deep = "[".repeat(100_000);
        assert_eq!(decode_json(deep.as_bytes()), None);
    }

    #[test]
    fn decodes_base64_as_the_window_does() {
        let b = |s: &str| base64(&JStr(s.encode_utf16().collect()));
        assert_eq!(b("aGk="), Some(b"hi".to_vec()));
        assert_eq!(b("aGkh"), Some(b"hi!".to_vec()));
        assert_eq!(b(""), Some(vec![]));
        // atob ignores bits left over in the last character.
        assert_eq!(b("QR=="), Some(vec![0x41]));
        for bad in ["aGk", "aG=k", "a===", "====", "aGk*", "aGk\u{e9}", "a Gk"] {
            assert_eq!(b(bad), None, "{bad}");
        }
    }

    #[test]
    fn names_ids_and_versions_follow_the_window_patterns() {
        let n = |s: &str| is_plain_name(&JStr(s.encode_utf16().collect()));
        assert!(n("panel.js") && n("a") && n("A-b_c.min.js") && n("1"));
        assert!(
            !n(".hidden")
                && !n("a..b")
                && !n("a/b")
                && !n("..")
                && !n("a.")
                && !n("_a")
                && !n("c:x")
                && !n("")
        );
        assert!(!n(&"a".repeat(101)) && n(&"a".repeat(100)));
        assert!(is_version("0.1.0") && is_version("999999.0.10"));
        assert!(
            !is_version("01.0.0")
                && !is_version("1.0")
                && !is_version("1000000.0.0")
                && !is_version("1.0.0.0")
        );
        assert!(
            is_id("sample-agents", 40)
                && !is_id("-a", 40)
                && !is_id("A", 40)
                && !is_id(&"a".repeat(41), 40)
        );
        assert_eq!(
            compare_versions("0.10.0", "0.9.9"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(type_of("a.JS"), Some("text/javascript"));
        assert_eq!(type_of("a.constructor"), None);
    }
}

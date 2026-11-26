//! The community theme catalog (ADR-0113 decision 6, `docs/ux/themes.md`, "The community catalog").
//!
//! The hub, never the browser, fetches the catalog, and only on its owner's command (`agora hub themes update`, like
//! `agora hub prices update`): an index of themes, then each listed theme file. Everything fetched is untrusted:
//! - the index is read strictly (a format marker, no field it does not know, at most [`MAX_ENTRIES`] entries, each field of a closed
//!   kind); an entry with a problem is refused, the others go on;
//! - a theme file is fetched only from a relative path under the index's own folder on the same host, so an index cannot send the
//!   hub to another address; no redirect is followed (a 3xx is a refusal), and plain `http` is accepted only for this machine;
//! - every answer is capped ([`MAX_INDEX_BYTES`], [`MAX_THEME_BYTES`], [`MAX_TOTAL_BYTES`]) and bounded in time (each step and the
//!   whole update);
//! - each file must have the SHA-256 the index gives, and must pass the theme rules (`check`, given by the caller: the terminal
//!   client's reader, the same rules as the web window's), or it is refused.
//!
//! The accepted themes replace the stored catalog in one transaction; a failure to reach the catalog changes nothing. The catalog is
//! off until the owner turns it on; while it is off a window is told so and gets no theme. Turning it on or off and every update are
//! in the audit trail as the owner's. Nothing here writes prose: refusals are codes the command line words from `locales/`.

use std::time::{Duration, Instant};

use agora_protocol::v1;
use agora_protocol::{Error, ErrorCode};
use rusqlite::{params, OptionalExtension};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::deliver::{get_within, Url};
use crate::store::{EventStore, SqliteStore, StoreError};

/// The catalog's address until the owner gives one with `--url`: a placeholder that can never resolve (`.invalid`, RFC 6761), since
/// the catalog's repository does not exist yet. [`index_url`] refuses it, so nothing is ever fetched from it.
pub const DEFAULT_INDEX_URL: &str = "https://themes.agora.invalid/index.json";
/// The field that marks an index, and the only format this hub reads.
pub const INDEX_FORMAT_KEY: &str = "agora-theme-catalog";
pub const INDEX_FORMAT: u64 = 1;
/// The largest index read.
pub const MAX_INDEX_BYTES: usize = 256 * 1024;
/// The largest theme file read (the theme rules' own cap, `docs/ux/themes.md`).
pub const MAX_THEME_BYTES: usize = 256 * 1024;
/// The most theme data one catalog keeps; themes past it are refused.
pub const MAX_TOTAL_BYTES: usize = 4 * 1024 * 1024;
/// The most entries one index may list.
pub const MAX_ENTRIES: usize = 200;
pub const MAX_ID: usize = 40;
pub const MAX_NAME: usize = 60;
pub const MAX_AUTHOR: usize = 100;
pub const MAX_LICENCE: usize = 64;
pub const MAX_FILE: usize = 200;
/// The highest version number read.
pub const MAX_VERSION: u64 = 1_000_000;
/// How long connecting, and then each read or write, may take.
pub const STEP: Duration = Duration::from_secs(10);
/// How long a whole update (the index and every theme) may take.
pub const UPDATE_DEADLINE: Duration = Duration::from_secs(120);

/// The hub settings: the owner's switch (`on` / `off`, off when missing), the address of the last update, and when it was.
pub const SWITCH_SETTING: &str = "theme_catalog";
pub const URL_SETTING: &str = "theme_catalog_url";
pub const UPDATED_SETTING: &str = "theme_catalog_updated_ms";

/// One theme as the index lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub version: u32,
    pub author: String,
    pub licence: String,
    pub sha256: String,
    /// The file's path, relative to the index's folder.
    pub file: String,
}

/// Why the index could not be used; nothing was changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexError {
    /// Not an `https://` address (or `http://` on this machine), or still the placeholder.
    Address,
    /// It could not be reached or read.
    Fetch,
    /// It took longer than allowed.
    TimedOut,
    /// It answered with another status than 200 (a redirect included: none is followed).
    Status(u16),
    TooLarge,
    /// Not UTF-8 JSON.
    Json,
    /// Not an index: no format marker, or no list of themes.
    Format,
    /// An index of a newer format than this hub reads.
    Newer,
    UnknownField(String),
    TooMany,
}

impl IndexError {
    /// The stable code the command line words (`cli-hub-themes-index-<code>`).
    pub fn code(&self) -> &'static str {
        match self {
            IndexError::Address => "address",
            IndexError::Fetch => "fetch",
            IndexError::TimedOut => "timeout",
            IndexError::Status(_) => "status",
            IndexError::TooLarge => "too-large",
            IndexError::Json => "json",
            IndexError::Format => "format",
            IndexError::Newer => "newer",
            IndexError::UnknownField(_) => "unknown-field",
            IndexError::TooMany => "too-many",
        }
    }
}

/// Why one theme was refused; the others are kept.
#[derive(Debug, Clone, PartialEq)]
pub enum Refusal<P> {
    /// A field of the entry is missing or not of its kind (`field` names it).
    Entry(&'static str),
    UnknownField(String),
    Duplicate,
    /// Its file answered another status than 200 (a redirect included).
    Status(u16),
    TooLarge,
    /// The catalog's themes together are over [`MAX_TOTAL_BYTES`].
    Total,
    /// The file's SHA-256 is not the one the index gives.
    Hash,
    NotText,
    /// The theme rules refuse it, with every reason.
    Theme(P),
}

impl<P> Refusal<P> {
    /// The stable code the command line words (`cli-hub-themes-refused-<code>`).
    pub fn code(&self) -> &'static str {
        match self {
            Refusal::Entry(_) => "entry",
            Refusal::UnknownField(_) => "unknown-field",
            Refusal::Duplicate => "duplicate",
            Refusal::Status(_) => "status",
            Refusal::TooLarge => "too-large",
            Refusal::Total => "total",
            Refusal::Hash => "hash",
            Refusal::NotText => "not-text",
            Refusal::Theme(_) => "theme",
        }
    }
}

/// Why an update changed nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateError {
    Index(IndexError),
    /// A theme's file could not be reached in time (`id`); the catalog is kept as it was, so a network failure half way never drops
    /// themes that are fine.
    Theme {
        id: String,
        error: IndexError,
    },
    Store(String),
}

impl From<StoreError> for UpdateError {
    fn from(e: StoreError) -> Self {
        UpdateError::Store(e.to_string())
    }
}

/// A theme that passed every check, with its file exactly as fetched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accepted {
    pub entry: Entry,
    pub data: String,
}

/// What an update found. `refused` names each refused entry by its id (or `#n`, its place in the list, when it has no usable id).
#[derive(Debug, Clone, PartialEq)]
pub struct Fetched<P> {
    pub accepted: Vec<Accepted>,
    pub refused: Vec<(String, Refusal<P>)>,
}

/// Time limits of one update (tests make them short).
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub step: Duration,
    pub deadline: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            step: STEP,
            deadline: UPDATE_DEADLINE,
        }
    }
}

/// The index's address: `https://`, or `http://` only to this machine (`localhost`, `127.0.0.1`), and never the placeholder.
pub fn index_url(s: &str) -> Option<Url> {
    if s == DEFAULT_INDEX_URL {
        return None;
    }
    let url = Url::parse(s)?;
    (url.https || matches!(url.host.as_str(), "localhost" | "127.0.0.1")).then_some(url)
}

/// A theme file's address: `file` is a relative path under the index's folder (letters, digits, `.`, `_`, `-` and `/`, ending in
/// `.json`; no empty, `.` or `..` part), on the same scheme, host and port. `None` for anything else.
pub fn file_url(index: &Url, file: &str) -> Option<Url> {
    if file.is_empty()
        || file.len() > MAX_FILE
        || !file.ends_with(".json")
        || !file
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
        || file
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return None;
    }
    let path = index.path.split(['?', '#']).next().unwrap_or("/");
    let folder = &path[..path.rfind('/').map_or(0, |i| i + 1)];
    Some(Url {
        https: index.https,
        host: index.host.clone(),
        port: index.port,
        path: format!("{}{file}", if folder.is_empty() { "/" } else { folder }),
    })
}

fn plain_text(s: &str, max: usize) -> bool {
    !s.trim().is_empty()
        && s.chars().count() <= max
        && s.trim() == s
        && !s
            .chars()
            .any(|c| c.is_control() || agora_core::text::is_hidden(c))
}

fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_ID
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !s.starts_with('-')
}

fn valid_licence(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_LICENCE
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | ' ' | '(' | ')'))
        && s.trim() == s
}

fn valid_sha256(s: &str) -> bool {
    s.len() == 64
        && s.chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

const ENTRY_FIELDS: [&str; 7] = [
    "id", "name", "version", "author", "licence", "sha256", "file",
];

/// Reads one entry of the index; `Err` names the first problem.
fn read_entry<P>(v: &Value) -> Result<Entry, Refusal<P>> {
    let Some(o) = v.as_object() else {
        return Err(Refusal::Entry("entry"));
    };
    if let Some(k) = o.keys().find(|k| !ENTRY_FIELDS.contains(&k.as_str())) {
        return Err(Refusal::UnknownField(k.chars().take(MAX_ID).collect()));
    }
    let text = |k: &'static str, ok: &dyn Fn(&str) -> bool| -> Result<String, Refusal<P>> {
        o.get(k)
            .and_then(Value::as_str)
            .filter(|s| ok(s))
            .map(str::to_string)
            .ok_or(Refusal::Entry(k))
    };
    let id = text("id", &valid_id)?;
    let name = text("name", &|s| plain_text(s, MAX_NAME))?;
    let version = o
        .get("version")
        .and_then(Value::as_u64)
        .filter(|v| (1..=MAX_VERSION).contains(v))
        .and_then(|v| u32::try_from(v).ok())
        .ok_or(Refusal::Entry("version"))?;
    let author = text("author", &|s| plain_text(s, MAX_AUTHOR))?;
    let licence = text("licence", &valid_licence)?;
    let sha256 = text("sha256", &valid_sha256)?;
    let file = text("file", &|s| !s.is_empty())?;
    Ok(Entry {
        id,
        name,
        version,
        author,
        licence,
        sha256,
        file,
    })
}

/// The id an entry is reported by: its own when it is a usable one, else its place in the list.
fn label_of(v: &Value, i: usize) -> String {
    v.get("id")
        .and_then(Value::as_str)
        .filter(|s| valid_id(s))
        .map_or_else(|| format!("#{}", i + 1), str::to_string)
}

/// Reads an index's text: the entries that can be fetched, and those refused already.
#[allow(clippy::type_complexity)]
pub fn read_index<P>(text: &str) -> Result<(Vec<Entry>, Vec<(String, Refusal<P>)>), IndexError> {
    let v: Value = serde_json::from_str(text).map_err(|_| IndexError::Json)?;
    let o: &Map<String, Value> = v.as_object().ok_or(IndexError::Format)?;
    let format = o
        .get(INDEX_FORMAT_KEY)
        .and_then(Value::as_u64)
        .filter(|f| *f >= 1)
        .ok_or(IndexError::Format)?;
    if format > INDEX_FORMAT {
        return Err(IndexError::Newer);
    }
    if let Some(k) = o.keys().find(|k| *k != INDEX_FORMAT_KEY && *k != "themes") {
        return Err(IndexError::UnknownField(k.chars().take(MAX_ID).collect()));
    }
    let list = o
        .get("themes")
        .and_then(Value::as_array)
        .ok_or(IndexError::Format)?;
    if list.len() > MAX_ENTRIES {
        return Err(IndexError::TooMany);
    }
    let (mut entries, mut refused): (Vec<Entry>, Vec<_>) = (Vec::new(), Vec::new());
    for (i, v) in list.iter().enumerate() {
        match read_entry(v) {
            Ok(e) if entries.iter().any(|x: &Entry| x.id == e.id) => {
                refused.push((e.id, Refusal::Duplicate))
            }
            Ok(e) => entries.push(e),
            Err(r) => refused.push((label_of(v, i), r)),
        }
    }
    Ok((entries, refused))
}

fn io_error(e: &std::io::Error) -> IndexError {
    match e.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => IndexError::TimedOut,
        std::io::ErrorKind::InvalidData if e.to_string().contains("too large") => {
            IndexError::TooLarge
        }
        _ => IndexError::Fetch,
    }
}

/// Fetches the index at `url` and every theme it lists, and checks them (`check`: the theme rules, `Err` with every reason). Changes
/// nothing; [`save`] keeps the result.
pub fn fetch<P>(
    url: &Url,
    limits: Limits,
    check: impl Fn(&str) -> Result<(), P>,
) -> Result<Fetched<P>, UpdateError> {
    let deadline = Instant::now() + limits.deadline;
    let (status, body) = get_within(url, MAX_INDEX_BYTES, limits.step, Some(deadline))
        .map_err(|e| UpdateError::Index(io_error(&e)))?;
    if status != 200 {
        return Err(UpdateError::Index(IndexError::Status(status)));
    }
    let text = String::from_utf8(body).map_err(|_| UpdateError::Index(IndexError::Json))?;
    let (entries, mut refused) = read_index::<P>(&text).map_err(UpdateError::Index)?;
    let mut accepted = Vec::new();
    let mut total = 0usize;
    for entry in entries {
        let Some(at) = file_url(url, &entry.file) else {
            refused.push((entry.id, Refusal::Entry("file")));
            continue;
        };
        let body = match get_within(&at, MAX_THEME_BYTES, limits.step, Some(deadline)) {
            Ok((200, body)) => body,
            Ok((status, _)) => {
                refused.push((entry.id, Refusal::Status(status)));
                continue;
            }
            Err(e) => match io_error(&e) {
                IndexError::TooLarge => {
                    refused.push((entry.id, Refusal::TooLarge));
                    continue;
                }
                error => {
                    return Err(UpdateError::Theme {
                        id: entry.id,
                        error,
                    })
                }
            },
        };
        let hash: String = Sha256::digest(&body)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if hash != entry.sha256 {
            refused.push((entry.id, Refusal::Hash));
            continue;
        }
        let Ok(data) = String::from_utf8(body) else {
            refused.push((entry.id, Refusal::NotText));
            continue;
        };
        if total + data.len() > MAX_TOTAL_BYTES {
            refused.push((entry.id, Refusal::Total));
            continue;
        }
        if let Err(p) = check(&data) {
            refused.push((entry.id, Refusal::Theme(p)));
            continue;
        }
        total += data.len();
        accepted.push(Accepted { entry, data });
    }
    Ok(Fetched { accepted, refused })
}

/// Replaces the stored catalog with `accepted`, and says when, in one transaction.
pub fn save(store: &SqliteStore, accepted: &[Accepted], now_ms: u64) -> Result<(), StoreError> {
    let mut conn = store.lock()?;
    let tx = crate::store::write_tx(&mut conn)?;
    tx.execute("DELETE FROM theme_catalog", [])?;
    for a in accepted {
        let e = &a.entry;
        tx.execute(
            "INSERT INTO theme_catalog (id, name, version, author, licence, sha256, data) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![e.id, e.name, e.version, e.author, e.licence, e.sha256, a.data],
        )?;
    }
    tx.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![UPDATED_SETTING, now_ms.to_string().into_bytes()],
    )?;
    tx.commit()?;
    Ok(())
}

/// `agora hub themes update`: fetches the catalog at `url` (the owner's `--url`, else the address of the last update), keeps what
/// passed, remembers the address, and records the update in the audit trail (`ok`, `partial` when some theme was refused, or
/// `failed`). A failure changes nothing but the audit trail.
pub fn update<P>(
    store: &SqliteStore,
    url: &str,
    limits: Limits,
    check: impl Fn(&str) -> Result<(), P>,
    now_ms: u64,
) -> Result<Fetched<P>, UpdateError> {
    let result = index_url(url)
        .ok_or(UpdateError::Index(IndexError::Address))
        .and_then(|u| fetch(&u, limits, check))
        .and_then(|f| {
            save(store, &f.accepted, now_ms)?;
            store.set_setting(URL_SETTING, url.as_bytes())?;
            Ok(f)
        });
    let outcome = match &result {
        Ok(f) if f.refused.is_empty() => "ok",
        Ok(_) => "partial",
        Err(_) => "failed",
    };
    let _ = crate::audit::Audit::new(store).record(
        now_ms,
        "owner",
        "theme_catalog.update",
        "cli",
        outcome,
    );
    result
}

/// The address the next update uses without `--url`: the last update's, else the placeholder.
pub fn saved_url(store: &SqliteStore) -> Result<String, StoreError> {
    Ok(store
        .setting(URL_SETTING)?
        .and_then(|v| String::from_utf8(v).ok())
        .unwrap_or_else(|| DEFAULT_INDEX_URL.to_string()))
}

/// Whether the owner turned the catalog on. Off when missing.
pub fn is_on(store: &SqliteStore) -> Result<bool, StoreError> {
    Ok(store.setting(SWITCH_SETTING)?.as_deref() == Some(b"on".as_slice()))
}

/// Turns the catalog on or off, recorded in the audit trail as the owner's.
pub fn set_on(store: &SqliteStore, on: bool, now_ms: u64) -> Result<(), StoreError> {
    store.set_setting(SWITCH_SETTING, if on { b"on" } else { b"off" })?;
    let _ = crate::audit::Audit::new(store).record(
        now_ms,
        "owner",
        if on {
            "theme_catalog.on"
        } else {
            "theme_catalog.off"
        },
        "cli",
        "ok",
    );
    Ok(())
}

/// What `agora hub themes status` says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub on: bool,
    pub url: String,
    pub updated_ms: u64,
    pub themes: usize,
}

pub fn status(store: &SqliteStore) -> Result<Status, StoreError> {
    let updated_ms = store
        .setting(UPDATED_SETTING)?
        .and_then(|v| String::from_utf8(v).ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let themes: i64 = store
        .lock()?
        .query_row("SELECT COUNT(*) FROM theme_catalog", [], |r| r.get(0))?;
    Ok(Status {
        on: is_on(store)?,
        url: saved_url(store)?,
        updated_ms,
        themes: usize::try_from(themes).unwrap_or(0),
    })
}

fn entry_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<v1::ThemeCatalogEntry> {
    Ok(v1::ThemeCatalogEntry {
        id: r.get(0)?,
        name: r.get(1)?,
        version: r.get(2)?,
        author: r.get(3)?,
        licence: r.get(4)?,
        sha256: r.get(5)?,
        size: r.get(6)?,
    })
}

/// `ThemeCatalogList`: the catalog's themes by name, without their files; `on: false` and nothing else while it is off.
pub fn list(store: &SqliteStore) -> Result<v1::ThemeCatalogListResult, StoreError> {
    if !is_on(store)? {
        return Ok(v1::ThemeCatalogListResult::default());
    }
    let updated_at_ms = status(store)?.updated_ms;
    let conn = store.lock()?;
    let mut stmt = conn.prepare(
        "SELECT id, name, version, author, licence, sha256, length(CAST(data AS BLOB)) FROM theme_catalog ORDER BY name, id",
    )?;
    let entries = stmt
        .query_map([], entry_of)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(v1::ThemeCatalogListResult {
        on: true,
        entries,
        updated_at_ms,
    })
}

/// `ThemeCatalogGet`: one theme's file. `theme.catalog_off` while the catalog is off, `theme.not_found` for an id it lacks.
pub fn get(store: &SqliteStore, id: &str) -> Result<v1::ThemeCatalogTheme, Error> {
    let internal = |_| Error::new(ErrorCode::Internal);
    if !is_on(store).map_err(internal)? {
        return Err(Error::new(ErrorCode::ThemeCatalogOff));
    }
    let not_found = || {
        Error::new(ErrorCode::ThemeNotFound).with(
            "id",
            agora_protocol::ParamValue::Str(id.chars().take(MAX_ID).collect()),
        )
    };
    if !valid_id(id) {
        return Err(not_found());
    }
    let conn = store.lock().map_err(internal)?;
    let found = conn
        .query_row(
            "SELECT id, name, version, author, licence, sha256, length(CAST(data AS BLOB)), data FROM theme_catalog WHERE id = ?1",
            [id],
            |r| Ok((entry_of(r)?, r.get::<_, String>(7)?)),
        )
        .optional()
        .map_err(|_| Error::new(ErrorCode::Internal))?;
    let (entry, data) = found.ok_or_else(not_found)?;
    Ok(v1::ThemeCatalogTheme {
        entry: Some(entry),
        data,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn a_theme_file_comes_only_from_under_the_index_folder_on_the_same_host() {
        let i = url("https://raw.example.org/agora-themes/main/index.json?x=1");
        assert_eq!(
            file_url(&i, "themes/harbour.json").unwrap(),
            url("https://raw.example.org/agora-themes/main/themes/harbour.json")
        );
        let root = url("https://h.example/index.json");
        assert_eq!(file_url(&root, "a.json").unwrap().path, "/a.json");
        for bad in [
            "",
            "/etc/a.json",
            "../a.json",
            "themes/../../a.json",
            "./a.json",
            "themes//a.json",
            "https://evil.example/a.json",
            "a.json?x",
            "a.json#x",
            "a b.json",
            "a\\b.json",
            "a.txt",
            "%2e%2e/a.json",
        ] {
            assert_eq!(file_url(&i, bad), None, "{bad}");
        }
    }

    #[test]
    fn only_https_or_this_machine_and_never_the_placeholder() {
        assert!(index_url("https://h.example/i.json").is_some());
        assert!(index_url("http://127.0.0.1:9/i.json").is_some());
        assert!(index_url("http://localhost/i.json").is_some());
        assert!(index_url("http://h.example/i.json").is_none());
        assert!(index_url(DEFAULT_INDEX_URL).is_none());
        assert!(index_url("ftp://h/i.json").is_none());
    }

    fn entry(id: &str) -> serde_json::Value {
        serde_json::json!({"id": id, "name": "Harbour", "version": 2, "author": "Ada", "licence": "MIT",
            "sha256": "0".repeat(64), "file": format!("themes/{id}.json")})
    }

    #[test]
    fn the_index_is_read_strictly_and_a_bad_entry_is_refused_alone() {
        let mut extra = entry("extra");
        extra["homepage"] = "https://x".into();
        let mut bad_version = entry("v");
        bad_version["version"] = "2.0".into();
        let mut hidden = entry("hidden");
        hidden["name"] = "Har\u{202e}bour".into();
        let index = serde_json::json!({"agora-theme-catalog": 1, "themes": [
            entry("harbour"), entry("harbour"), extra, bad_version, hidden, {"id": "BAD"}, 7
        ]});
        let (ok, refused) = read_index::<()>(&index.to_string()).unwrap();
        assert_eq!(ok.len(), 1);
        assert_eq!(ok[0].version, 2);
        let codes: Vec<(String, &str)> = refused
            .iter()
            .map(|(id, r)| (id.clone(), r.code()))
            .collect();
        assert_eq!(
            codes,
            [
                ("harbour".into(), "duplicate"),
                ("extra".into(), "unknown-field"),
                ("v".into(), "entry"),
                ("hidden".into(), "entry"),
                ("#6".into(), "entry"),
                ("#7".into(), "entry"),
            ]
        );
    }

    #[test]
    fn an_index_that_is_not_one_is_refused_whole() {
        let r = |s: &str| read_index::<()>(s).map(|_| ()).unwrap_err();
        assert_eq!(r("[]"), IndexError::Format);
        assert_eq!(r("{"), IndexError::Json);
        assert_eq!(r(r#"{"themes": []}"#), IndexError::Format);
        assert_eq!(
            r(r#"{"agora-theme-catalog": 2, "themes": []}"#),
            IndexError::Newer
        );
        assert_eq!(
            r(r#"{"agora-theme-catalog": 1, "themes": [], "x": 1}"#),
            IndexError::UnknownField("x".into())
        );
        let many = serde_json::json!({"agora-theme-catalog": 1, "themes": vec![entry("a"); MAX_ENTRIES + 1]});
        assert_eq!(r(&many.to_string()), IndexError::TooMany);
    }

    fn accepted(id: &str, version: u32) -> Accepted {
        Accepted {
            entry: Entry {
                id: id.into(),
                name: format!("Theme {id}"),
                version,
                author: "Ada".into(),
                licence: "MIT".into(),
                sha256: "a".repeat(64),
                file: format!("{id}.json"),
            },
            data: format!("{{\"agora-theme\": 2, \"name\": \"{id}\"}}"),
        }
    }

    #[test]
    fn the_list_and_a_theme_are_given_only_while_the_catalog_is_on() {
        let store = SqliteStore::open_in_memory().unwrap();
        save(&store, &[accepted("b", 1), accepted("a", 3)], 42).unwrap();
        assert_eq!(list(&store).unwrap(), v1::ThemeCatalogListResult::default());
        assert_eq!(
            get(&store, "a").unwrap_err().code,
            ErrorCode::ThemeCatalogOff
        );
        set_on(&store, true, 50).unwrap();
        let l = list(&store).unwrap();
        assert!(l.on);
        assert_eq!(l.updated_at_ms, 42);
        assert_eq!(
            l.entries
                .iter()
                .map(|e| (e.id.as_str(), e.version))
                .collect::<Vec<_>>(),
            [("a", 3), ("b", 1)]
        );
        let t = get(&store, "a").unwrap();
        assert_eq!(t.data, accepted("a", 3).data);
        assert_eq!(t.entry.unwrap().size as usize, accepted("a", 3).data.len());
        let e = get(&store, "zz").unwrap_err();
        assert_eq!(e.code, ErrorCode::ThemeNotFound);
        assert_eq!(
            e.params.get("id"),
            Some(&agora_protocol::ParamValue::Str("zz".into()))
        );
        // An update replaces the whole catalog.
        save(&store, &[accepted("c", 1)], 60).unwrap();
        assert_eq!(list(&store).unwrap().entries.len(), 1);
        assert_eq!(get(&store, "a").unwrap_err().code, ErrorCode::ThemeNotFound);
        set_on(&store, false, 70).unwrap();
        assert!(!list(&store).unwrap().on);
    }

    #[test]
    fn switching_and_updating_are_in_the_audit_trail_as_the_owners() {
        let store = SqliteStore::open_in_memory().unwrap();
        set_on(&store, true, 1).unwrap();
        set_on(&store, false, 2).unwrap();
        let r = update(
            &store,
            DEFAULT_INDEX_URL,
            Limits::default(),
            |_| Ok::<(), ()>(()),
            3,
        );
        assert_eq!(r.unwrap_err(), UpdateError::Index(IndexError::Address));
        let rows = crate::audit::Audit::new(&store).list(0, None, 10).unwrap();
        let seen: Vec<(String, String, String)> = rows
            .iter()
            .map(|e| (e.who.clone(), e.action.clone(), e.outcome.clone()))
            .collect();
        for want in [
            ("owner", "theme_catalog.on", "ok"),
            ("owner", "theme_catalog.off", "ok"),
            ("owner", "theme_catalog.update", "failed"),
        ] {
            assert!(
                seen.iter()
                    .any(|(w, a, o)| (w.as_str(), a.as_str(), o.as_str()) == want),
                "{want:?} in {seen:?}"
            );
        }
        // A failed update kept nothing and remembered no address.
        assert_eq!(status(&store).unwrap().url, DEFAULT_INDEX_URL);
    }
}

//! The community plugin catalog (ADR-0114 decision 7, build step 4; `docs/ux/plugins.md`, "The community catalog").
//!
//! A public repository holds reviewed plugin archives and an index of them, `index.json`, signed by the catalog's key with
//! `ssh-keygen -Y sign -n agora-plugin-catalog` (`index.json.sig` beside it). The hub, never the browser, reads it, and only on its
//! owner's command (`agora hub plugins update`):
//! - **The key is the owner's to pin** (`agora hub plugins key set FILE`). Without one the catalog cannot be used at all. Pinning
//!   another key forgets the stored catalog and its serial, and nothing verified under the old key is offered again.
//! - **The signature comes first.** The index's exact bytes are checked against `index.json.sig` under the pinned key before any of
//!   them is read as JSON; an index that does not verify changes nothing. The index lists each archive's SHA-256 and size, so the
//!   signature covers every archive it names.
//! - **No going back.** The index carries a `serial`; one lower than the last accepted is refused whole, so an old signed index (with
//!   a version since withdrawn) cannot be served again.
//! - **Strict reading.** A format marker, no field it does not know, at most [`MAX_ENTRIES`] entries, each field of a closed kind,
//!   only the capabilities a window knows; an entry with a problem is refused and reported, the others are kept.
//! - **Fetching** is `theme_catalog`'s: `https` (plain `http` only to this machine), no redirect, every answer capped and bounded in
//!   time, an archive only from a relative path under the index's own folder on the same host.
//! - **Installing and upgrading** (`agora hub plugins install|upgrade ID`) fetch the archive, check its size and SHA-256 against the
//!   signed entry, check the package as a window does ([`crate::plugin_package`]), check that its manifest says what the signed entry
//!   says (id, version, name, author, licence, capabilities), and only then keep it through [`crate::plugins::Plugins`]. The command
//!   line shows what it asks to see (for an upgrade, what changed, capabilities first) and asks before it keeps anything. Nothing is
//!   ever installed or upgraded on its own.
//!
//! The catalog is off until the owner turns it on; while it is off a window is told so and gets no entry, and nothing is installed
//! from it. Every switch, key, update, install and upgrade (refused or not) is a row of the audit trail as the owner's. Nothing here
//! writes prose: refusals are codes the command line words from `locales/`.

use std::time::{Duration, Instant};

use agora_protocol::v1;
use rusqlite::{params, OptionalExtension};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::deliver::{get_within, Url};
use crate::plugin_package::{self, Refused, CAPABILITIES, MAX_PACKAGE_BYTES};
use crate::plugins::{AddError, Added, Plugins};
use crate::sshsig::{self, KeyError, PublicKey, SigError};
use crate::store::{write_tx, EventStore, SqliteStore, StoreError};
use crate::theme_catalog::{
    file_url, index_url as checked_url, io_error, plain_text, printable, valid_id, valid_licence,
    valid_sha256, IndexError,
};

/// The catalog's address until the owner gives one with `--url`: a placeholder that can never resolve (`.invalid`, RFC 6761), since
/// the catalog's repository does not exist yet. [`index_url`] refuses it, so nothing is ever fetched from it.
pub const DEFAULT_INDEX_URL: &str = "https://plugins.agora.invalid/index.json";
/// The field that marks an index, and the only format this hub reads.
pub const INDEX_FORMAT_KEY: &str = "agora-plugin-catalog";
pub const INDEX_FORMAT: u64 = 1;
/// The namespace the index is signed for (`ssh-keygen -Y sign -n`): a signature made for anything else does not verify here.
pub const SIG_NAMESPACE: &str = "agora-plugin-catalog";
/// The largest index read.
pub const MAX_INDEX_BYTES: usize = 256 * 1024;
/// The most entries one index may list.
pub const MAX_ENTRIES: usize = 200;
pub const MAX_NAME: usize = 60;
pub const MAX_AUTHOR: usize = 100;
pub const MAX_LICENCE: usize = 60;
pub const MAX_DESCRIPTION: usize = 300;
/// The highest serial read (what a JSON number holds exactly).
pub const MAX_SERIAL: u64 = (1 << 53) - 1;
/// How long connecting, and then each read or write, may take.
pub const STEP: Duration = Duration::from_secs(10);
/// How long fetching the index and its signature, or one archive, may take.
pub const DEADLINE: Duration = Duration::from_secs(120);

/// The hub settings: the owner's switch (`on` / `off`, off when missing), the address of the last update, when it was, the serial of
/// the last accepted index, the pinned key (one OpenSSH line), and the key the stored catalog was verified under.
pub const SWITCH_SETTING: &str = "plugin_catalog";
pub const URL_SETTING: &str = "plugin_catalog_url";
pub const UPDATED_SETTING: &str = "plugin_catalog_updated_ms";
pub const SERIAL_SETTING: &str = "plugin_catalog_serial";
pub const KEY_SETTING: &str = "plugin_catalog_key";
pub const VERIFIED_KEY_SETTING: &str = "plugin_catalog_verified_key";

/// One plugin as the signed index lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub name: String,
    /// `x.y.z`, as the manifest's.
    pub version: String,
    pub author: String,
    pub licence: String,
    /// One line of what it does; empty when the index gives none.
    pub description: String,
    pub capabilities: Vec<String>,
    pub sha256: String,
    pub size: u64,
    /// The archive's path, relative to the index's folder.
    pub file: String,
}

/// Why one entry of the index was refused; the others are kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryRefusal {
    /// A field is missing or not of its kind (`field` names it).
    Field(&'static str),
    UnknownField(String),
    Duplicate,
}

impl EntryRefusal {
    /// The stable code the command line words (`cli-hub-plugins-refused-<code>`).
    pub fn code(&self) -> &'static str {
        match self {
            EntryRefusal::Field(_) => "field",
            EntryRefusal::UnknownField(_) => "unknown-field",
            EntryRefusal::Duplicate => "duplicate",
        }
    }
}

/// Why an update changed nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateError {
    /// No catalog key is pinned: the catalog cannot be used.
    NoKey,
    /// The index could not be fetched or read.
    Index(IndexError),
    /// Its signature could not be fetched (`index.json.sig`).
    SignatureFetch(IndexError),
    /// Its signature does not verify under the pinned key.
    Signature(SigError),
    /// Its serial is lower than the last accepted one.
    Rollback {
        seen: u64,
        offered: u64,
    },
    Store(String),
}

impl UpdateError {
    /// The code written into the audit trail.
    pub fn code(&self) -> String {
        match self {
            UpdateError::NoKey => "no-key".into(),
            UpdateError::Index(e) => format!("index.{}", e.code()),
            UpdateError::SignatureFetch(e) => format!("signature-fetch.{}", e.code()),
            UpdateError::Signature(e) => format!("signature.{}", e.code()),
            UpdateError::Rollback { .. } => "rollback".into(),
            UpdateError::Store(_) => "internal".into(),
        }
    }
}

impl From<StoreError> for UpdateError {
    fn from(e: StoreError) -> Self {
        UpdateError::Store(e.to_string())
    }
}

/// What an accepted index gave. `refused` names each refused entry by its id (or `#n`, its place in the list).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    pub serial: u64,
    pub entries: Vec<Entry>,
    pub refused: Vec<(String, EntryRefusal)>,
}

/// Time limits of one fetch (tests make them short).
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub step: Duration,
    pub deadline: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            step: STEP,
            deadline: DEADLINE,
        }
    }
}

/// The index's address: `https://`, or `http://` only to this machine, and never the placeholder.
pub fn index_url(s: &str) -> Option<Url> {
    if s == DEFAULT_INDEX_URL {
        return None;
    }
    checked_url(s)
}

/// The signature's address: the index's path (without its query) and `.sig`, on the same scheme, host and port.
pub fn signature_url(index: &Url) -> Url {
    let path = index.path.split(['?', '#']).next().unwrap_or("/");
    Url {
        https: index.https,
        host: index.host.clone(),
        port: index.port,
        path: format!("{path}.sig"),
    }
}

const ENTRY_FIELDS: [&str; 10] = [
    "id",
    "name",
    "version",
    "author",
    "licence",
    "description",
    "capabilities",
    "sha256",
    "size",
    "file",
];

fn cut(s: &str) -> String {
    printable(&s.chars().take(40).collect::<String>())
}

fn read_entry(v: &Value) -> Result<Entry, EntryRefusal> {
    let Some(o) = v.as_object() else {
        return Err(EntryRefusal::Field("entry"));
    };
    if let Some(k) = o.keys().find(|k| !ENTRY_FIELDS.contains(&k.as_str())) {
        return Err(EntryRefusal::UnknownField(cut(k)));
    }
    let text = |k: &'static str, ok: &dyn Fn(&str) -> bool| -> Result<String, EntryRefusal> {
        o.get(k)
            .and_then(Value::as_str)
            .filter(|s| ok(s))
            .map(str::to_string)
            .ok_or(EntryRefusal::Field(k))
    };
    let id = text("id", &valid_id)?;
    let name = text("name", &|s| plain_text(s, MAX_NAME))?;
    let version = text("version", &plugin_package::is_version)?;
    let author = text("author", &|s| plain_text(s, MAX_AUTHOR))?;
    let licence = text("licence", &|s| valid_licence(s) && s.len() <= MAX_LICENCE)?;
    let description = match o.get("description") {
        None => String::new(),
        Some(_) => text("description", &|s| plain_text(s, MAX_DESCRIPTION))?,
    };
    let caps = o
        .get("capabilities")
        .and_then(Value::as_array)
        .ok_or(EntryRefusal::Field("capabilities"))?;
    let mut capabilities: Vec<String> = Vec::new();
    for c in caps {
        match c.as_str() {
            Some(c) if CAPABILITIES.contains(&c) && !capabilities.iter().any(|x| x == c) => {
                capabilities.push(c.to_string())
            }
            _ => return Err(EntryRefusal::Field("capabilities")),
        }
    }
    let sha256 = text("sha256", &valid_sha256)?;
    let size = o
        .get("size")
        .and_then(Value::as_u64)
        .filter(|n| (1..=MAX_PACKAGE_BYTES as u64).contains(n))
        .ok_or(EntryRefusal::Field("size"))?;
    let file = text("file", &|s| !s.is_empty())?;
    // The archive's path is checked as an address is made from it: relative, under the index's folder, a `.json` bundle.
    let probe = Url {
        https: true,
        host: "h".into(),
        port: 443,
        path: "/index.json".into(),
    };
    if file_url(&probe, &file).is_none() {
        return Err(EntryRefusal::Field("file"));
    }
    Ok(Entry {
        id,
        name,
        version,
        author,
        licence,
        description,
        capabilities,
        sha256,
        size,
        file,
    })
}

/// Reads a verified index's text: its serial, the entries kept, and those refused.
pub fn read_index(text: &str) -> Result<Fetched, IndexError> {
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
    if let Some(k) = o
        .keys()
        .find(|k| !matches!(k.as_str(), INDEX_FORMAT_KEY | "serial" | "plugins"))
    {
        return Err(IndexError::UnknownField(cut(k)));
    }
    let serial = o
        .get("serial")
        .and_then(Value::as_u64)
        .filter(|s| (1..=MAX_SERIAL).contains(s))
        .ok_or(IndexError::Format)?;
    let list = o
        .get("plugins")
        .and_then(Value::as_array)
        .ok_or(IndexError::Format)?;
    if list.len() > MAX_ENTRIES {
        return Err(IndexError::TooMany);
    }
    let (mut entries, mut refused): (Vec<Entry>, Vec<_>) = (Vec::new(), Vec::new());
    for (i, v) in list.iter().enumerate() {
        match read_entry(v) {
            Ok(e) if entries.iter().any(|x: &Entry| x.id == e.id) => {
                refused.push((e.id, EntryRefusal::Duplicate))
            }
            Ok(e) => entries.push(e),
            Err(r) => refused.push((
                v.get("id")
                    .and_then(Value::as_str)
                    .filter(|s| valid_id(s))
                    .map_or_else(|| format!("#{}", i + 1), str::to_string),
                r,
            )),
        }
    }
    Ok(Fetched {
        serial,
        entries,
        refused,
    })
}

/// GET within the limits; a status other than 200 (a redirect included) is an error.
fn fetch_ok(
    url: &Url,
    max: usize,
    limits: Limits,
    deadline: Instant,
) -> Result<Vec<u8>, IndexError> {
    match get_within(url, max, limits.step, deadline) {
        Ok((200, body)) => Ok(body),
        Ok((status, _)) => Err(IndexError::Status(status)),
        Err(e) => Err(io_error(&e)),
    }
}

/// Fetches the index at `url` and its signature, verifies the signature under `key` over the index's exact bytes, and only then reads
/// it; refuses a serial lower than `last_serial`. Changes nothing; [`save`] keeps the result.
pub fn fetch(
    url: &Url,
    key: &PublicKey,
    last_serial: u64,
    limits: Limits,
) -> Result<Fetched, UpdateError> {
    let deadline = Instant::now() + limits.deadline;
    let body = fetch_ok(url, MAX_INDEX_BYTES, limits, deadline).map_err(UpdateError::Index)?;
    let sig = fetch_ok(
        &signature_url(url),
        sshsig::MAX_SIGNATURE_TEXT,
        limits,
        deadline,
    )
    .map_err(UpdateError::SignatureFetch)?;
    let sig = String::from_utf8(sig).map_err(|_| UpdateError::Signature(SigError::Malformed))?;
    sshsig::verify(key, SIG_NAMESPACE, &body, &sig).map_err(UpdateError::Signature)?;
    let text = String::from_utf8(body).map_err(|_| UpdateError::Index(IndexError::Json))?;
    let fetched = read_index(&text).map_err(UpdateError::Index)?;
    if fetched.serial < last_serial {
        return Err(UpdateError::Rollback {
            seen: last_serial,
            offered: fetched.serial,
        });
    }
    Ok(fetched)
}

fn put(tx: &rusqlite::Transaction<'_>, key: &str, value: &[u8]) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map(|_| ())
}

fn setting_text(store: &SqliteStore, key: &str) -> Result<Option<String>, StoreError> {
    Ok(store.setting(key)?.and_then(|v| String::from_utf8(v).ok()))
}

fn setting_u64(store: &SqliteStore, key: &str) -> Result<u64, StoreError> {
    Ok(setting_text(store, key)?
        .and_then(|s| s.parse().ok())
        .unwrap_or(0))
}

/// Replaces the stored catalog with `fetched`, remembers the address, the serial and the key it was verified under, and says when, in
/// one transaction.
pub fn save(
    store: &SqliteStore,
    fetched: &Fetched,
    url: &str,
    key: &PublicKey,
    now_ms: u64,
) -> Result<(), StoreError> {
    let mut conn = store.lock()?;
    let tx = write_tx(&mut conn)?;
    tx.execute("DELETE FROM plugin_catalog", [])?;
    for e in &fetched.entries {
        tx.execute(
            "INSERT INTO plugin_catalog (id, name, version, author, licence, description, capabilities, sha256, size, file)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                e.id,
                e.name,
                e.version,
                e.author,
                e.licence,
                e.description,
                e.capabilities.join(" "),
                e.sha256,
                i64::try_from(e.size).unwrap_or(i64::MAX),
                e.file
            ],
        )?;
    }
    put(&tx, UPDATED_SETTING, now_ms.to_string().as_bytes())?;
    put(&tx, URL_SETTING, url.as_bytes())?;
    put(&tx, SERIAL_SETTING, fetched.serial.to_string().as_bytes())?;
    put(&tx, VERIFIED_KEY_SETTING, key.line().as_bytes())?;
    tx.commit()?;
    Ok(())
}

fn record(store: &SqliteStore, now_ms: u64, action: &str, target: &str, outcome: &str) {
    let _ = crate::audit::Audit::new(store).record(now_ms, "owner", action, target, outcome);
}

/// `agora hub plugins update`: fetches and verifies the index at `url` (the owner's `--url`, else the address of the last update)
/// under the pinned key, keeps what passed, and records the update in the audit trail (`ok`, `partial` when some entry was refused,
/// or the failure's code). A failure changes nothing but the audit trail.
pub fn update(
    store: &SqliteStore,
    url: &str,
    limits: Limits,
    now_ms: u64,
) -> Result<Fetched, UpdateError> {
    let result = (|| -> Result<Fetched, UpdateError> {
        let key = key(store)?.ok_or(UpdateError::NoKey)?;
        let at = index_url(url).ok_or(UpdateError::Index(IndexError::Address))?;
        let fetched = fetch(&at, &key, setting_u64(store, SERIAL_SETTING)?, limits)?;
        save(store, &fetched, url, &key, now_ms)?;
        Ok(fetched)
    })();
    let outcome = match &result {
        Ok(f) if f.refused.is_empty() => "ok".to_string(),
        Ok(_) => "partial".to_string(),
        Err(e) => format!("failed.{}", e.code()),
    };
    record(store, now_ms, "plugin_catalog.update", "", &outcome);
    result
}

/// The address the next update uses without `--url`: the last update's, else the placeholder.
pub fn saved_url(store: &SqliteStore) -> Result<String, StoreError> {
    Ok(setting_text(store, URL_SETTING)?.unwrap_or_else(|| DEFAULT_INDEX_URL.to_string()))
}

/// Whether the owner turned the catalog on. Off when missing.
pub fn is_on(store: &SqliteStore) -> Result<bool, StoreError> {
    Ok(store.setting(SWITCH_SETTING)?.as_deref() == Some(b"on".as_slice()))
}

/// Turns the catalog on or off, recorded in the audit trail as the owner's.
pub fn set_on(store: &SqliteStore, on: bool, now_ms: u64) -> Result<(), StoreError> {
    store.set_setting(SWITCH_SETTING, if on { b"on" } else { b"off" })?;
    let action = if on {
        "plugin_catalog.on"
    } else {
        "plugin_catalog.off"
    };
    record(store, now_ms, action, "", "ok");
    Ok(())
}

/// The pinned key, if there is one.
pub fn key(store: &SqliteStore) -> Result<Option<PublicKey>, StoreError> {
    Ok(setting_text(store, KEY_SETTING)?.and_then(|t| PublicKey::parse(&t).ok()))
}

/// Why a key was not pinned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetKeyError {
    Key(KeyError),
    Store(String),
}

/// `agora hub plugins key set FILE`: pins the catalog's public key from a key file's text. A private key or anything but one
/// `ssh-ed25519` public key is refused, and nothing of it is kept. Pinning a key that differs from the one pinned forgets the stored
/// catalog and its serial (nothing verified under another key is offered); pinning the same key again changes nothing else. Recorded
/// in the audit trail as the owner's, with the key's fingerprint (or the refusal).
pub fn set_key(store: &SqliteStore, text: &str, now_ms: u64) -> Result<PublicKey, SetKeyError> {
    let parsed = PublicKey::parse(text);
    let result = parsed.map_err(SetKeyError::Key).and_then(|k| {
        let before = key(store).map_err(|e| SetKeyError::Store(e.to_string()))?;
        let keep = |conn: &mut rusqlite::Connection| -> rusqlite::Result<()> {
            let tx = write_tx(conn)?;
            put(&tx, KEY_SETTING, k.line().as_bytes())?;
            if before.as_ref() != Some(&k) {
                tx.execute("DELETE FROM plugin_catalog", [])?;
                tx.execute(
                    "DELETE FROM settings WHERE key IN (?1, ?2, ?3)",
                    params![SERIAL_SETTING, VERIFIED_KEY_SETTING, UPDATED_SETTING],
                )?;
            }
            tx.commit()
        };
        let mut conn = store
            .lock()
            .map_err(|e| SetKeyError::Store(e.to_string()))?;
        keep(&mut conn).map_err(|e| SetKeyError::Store(e.to_string()))?;
        Ok(k)
    });
    let (target, outcome) = match &result {
        Ok(k) => (k.fingerprint(), "ok".to_string()),
        Err(SetKeyError::Key(KeyError::Private)) => (String::new(), "key.private".to_string()),
        Err(SetKeyError::Key(KeyError::Invalid)) => (String::new(), "key.invalid".to_string()),
        Err(SetKeyError::Store(_)) => (String::new(), "internal".to_string()),
    };
    record(store, now_ms, "plugin_catalog.key", &target, &outcome);
    result
}

/// Whether the stored catalog was verified under the key pinned now (false without a key, or with none verified yet).
fn bound(store: &SqliteStore) -> Result<Option<PublicKey>, StoreError> {
    let Some(k) = key(store)? else {
        return Ok(None);
    };
    let verified = setting_text(store, VERIFIED_KEY_SETTING)?;
    Ok((verified.as_deref() == Some(k.line().as_str())).then_some(k))
}

/// What `agora hub plugins status` says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub on: bool,
    pub url: String,
    pub updated_ms: u64,
    pub serial: u64,
    pub plugins: usize,
    /// The pinned key's fingerprint, if one is pinned.
    pub key: Option<String>,
}

pub fn status(store: &SqliteStore) -> Result<Status, StoreError> {
    let plugins: i64 = store
        .lock()?
        .query_row("SELECT COUNT(*) FROM plugin_catalog", [], |r| r.get(0))?;
    Ok(Status {
        on: is_on(store)?,
        url: saved_url(store)?,
        updated_ms: setting_u64(store, UPDATED_SETTING)?,
        serial: setting_u64(store, SERIAL_SETTING)?,
        plugins: usize::try_from(plugins).unwrap_or(0),
        key: key(store)?.map(|k| k.fingerprint()),
    })
}

fn entry_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    Ok(Entry {
        id: r.get(0)?,
        name: r.get(1)?,
        version: r.get(2)?,
        author: r.get(3)?,
        licence: r.get(4)?,
        description: r.get(5)?,
        capabilities: r
            .get::<_, String>(6)?
            .split(' ')
            .filter(|c| !c.is_empty())
            .map(str::to_string)
            .collect(),
        sha256: r.get(7)?,
        size: u64::try_from(r.get::<_, i64>(8)?).unwrap_or(0),
        file: r.get(9)?,
    })
}

const COLUMNS: &str =
    "id, name, version, author, licence, description, capabilities, sha256, size, file";

/// The stored entries, by name then id, when they were verified under the key pinned now; none otherwise.
pub fn entries(store: &SqliteStore) -> Result<Vec<Entry>, StoreError> {
    if bound(store)?.is_none() {
        return Ok(Vec::new());
    }
    let conn = store.lock()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM plugin_catalog ORDER BY name, id"
    ))?;
    let rows = stmt.query_map([], entry_of)?;
    rows.collect::<Result<_, _>>().map_err(Into::into)
}

/// `PluginCatalogList`: the catalog's plugins, without their archives; `on: false` and nothing else while it is off. With no key
/// pinned, or a key pinned since the last update, it is on with no entry.
pub fn list(store: &SqliteStore) -> Result<v1::PluginCatalogListResult, StoreError> {
    if !is_on(store)? {
        return Ok(v1::PluginCatalogListResult::default());
    }
    let entries = entries(store)?;
    let updated_at_ms = if bound(store)?.is_some() {
        setting_u64(store, UPDATED_SETTING)?
    } else {
        0
    };
    Ok(v1::PluginCatalogListResult {
        on: true,
        entries: entries
            .into_iter()
            .map(|e| v1::PluginCatalogEntry {
                plugin_id: e.id,
                name: e.name,
                version: e.version,
                author: e.author,
                licence: e.licence,
                description: e.description,
                capabilities: e.capabilities,
                sha256: e.sha256,
                size: e.size,
            })
            .collect(),
        updated_at_ms,
    })
}

/// Installing a plugin that is not installed, or upgrading one that is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Install,
    Upgrade,
}

impl Mode {
    fn action(self) -> &'static str {
        match self {
            Mode::Install => "plugin_catalog.install",
            Mode::Upgrade => "plugin_catalog.upgrade",
        }
    }
}

/// Why nothing was installed or upgraded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallError {
    /// The owner has not turned the catalog on.
    Off,
    /// No catalog key is pinned.
    NoKey,
    /// The stored catalog was not verified under the key pinned now (none fetched yet, or the key changed since): update first.
    NotVerified,
    NotInCatalog,
    /// `install` of a plugin already installed (its version).
    Installed(String),
    /// `upgrade` of a plugin not installed.
    NotInstalled,
    /// The catalog's version is not newer than the installed one.
    NotNewer {
        installed: String,
        offered: String,
    },
    /// The archive could not be fetched (a redirect, a timeout, too large ...).
    Fetch(IndexError),
    /// Its size is not the one the signed index gives.
    Size,
    /// Its SHA-256 is not the one the signed index gives.
    Hash,
    /// The package fails the window's checks.
    Package(Refused),
    /// Its manifest says another thing than the signed entry (`field` names it).
    Mismatch(&'static str),
    /// The hub keeps as many plugins as it can.
    Full,
    /// The owner said no.
    Declined,
    Store(String),
}

impl InstallError {
    /// The code written into the audit trail.
    pub fn code(&self) -> String {
        match self {
            InstallError::Off => "off".into(),
            InstallError::NoKey => "no-key".into(),
            InstallError::NotVerified => "not-verified".into(),
            InstallError::NotInCatalog => "not-in-catalog".into(),
            InstallError::Installed(_) => "installed".into(),
            InstallError::NotInstalled => "not-installed".into(),
            InstallError::NotNewer { .. } => "not-newer".into(),
            InstallError::Fetch(e) => format!("fetch.{}", e.code()),
            InstallError::Size => "size".into(),
            InstallError::Hash => "hash".into(),
            InstallError::Package(r) => format!("package.{}", r.code.as_str()),
            InstallError::Mismatch(f) => format!("mismatch.{f}"),
            InstallError::Full => "plugin.full".into(),
            InstallError::Declined => "declined".into(),
            InstallError::Store(_) => "internal".into(),
        }
    }
}

impl From<StoreError> for InstallError {
    fn from(e: StoreError) -> Self {
        InstallError::Store(e.to_string())
    }
}

/// What an upgrade changes in what the plugin asks to see.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilityDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub kept: Vec<String>,
}

/// An archive fetched and checked, ready to keep once the owner agrees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub mode: Mode,
    pub entry: Entry,
    /// The installed version an upgrade replaces.
    pub installed: Option<String>,
    /// From the installed manifest to the checked one (for an install, everything is `added`).
    pub diff: CapabilityDiff,
    package: Vec<u8>,
}

fn target_of(id: &str, version: Option<&str>) -> String {
    match (valid_id(id), version) {
        (true, Some(v)) => format!("{id}@{v}"),
        (true, None) => id.to_string(),
        (false, _) => String::new(),
    }
}

/// `agora hub plugins install|upgrade ID`, before the owner is asked: finds the signed entry, fetches its archive, checks its size and
/// SHA-256 against the entry, checks the package as a window does, and checks that its manifest says what the entry says. Changes
/// nothing; a refusal is a row of the audit trail.
pub fn prepare(
    store: &SqliteStore,
    id: &str,
    mode: Mode,
    agora_version: &str,
    limits: Limits,
    now_ms: u64,
) -> Result<Prepared, InstallError> {
    let mut version = None;
    let result = prepare_checked(store, id, mode, agora_version, limits, &mut version);
    if let Err(e) = &result {
        record(
            store,
            now_ms,
            mode.action(),
            &target_of(id, version.as_deref()),
            &e.code(),
        );
    }
    result
}

fn prepare_checked(
    store: &SqliteStore,
    id: &str,
    mode: Mode,
    agora_version: &str,
    limits: Limits,
    version: &mut Option<String>,
) -> Result<Prepared, InstallError> {
    if !is_on(store)? {
        return Err(InstallError::Off);
    }
    if key(store)?.is_none() {
        return Err(InstallError::NoKey);
    }
    if bound(store)?.is_none() {
        return Err(InstallError::NotVerified);
    }
    let entry = if valid_id(id) {
        let conn = store.lock()?;
        conn.query_row(
            &format!("SELECT {COLUMNS} FROM plugin_catalog WHERE id = ?1"),
            [id],
            entry_of,
        )
        .optional()
        .map_err(|e| InstallError::Store(e.to_string()))?
    } else {
        None
    };
    let entry = entry.ok_or(InstallError::NotInCatalog)?;
    *version = Some(entry.version.clone());
    let installed = Plugins::new(store)
        .list()?
        .into_iter()
        .find(|p| p.plugin_id == entry.id);
    match (mode, &installed) {
        (Mode::Install, Some(p)) => return Err(InstallError::Installed(p.version.clone())),
        (Mode::Upgrade, None) => return Err(InstallError::NotInstalled),
        (Mode::Upgrade, Some(p))
            if plugin_package::compare_versions(&entry.version, &p.version)
                != std::cmp::Ordering::Greater =>
        {
            return Err(InstallError::NotNewer {
                installed: p.version.clone(),
                offered: entry.version.clone(),
            })
        }
        _ => {}
    }
    let index = index_url(&saved_url(store)?).ok_or(InstallError::Fetch(IndexError::Address))?;
    let at = file_url(&index, &entry.file).ok_or(InstallError::Fetch(IndexError::Address))?;
    let deadline = Instant::now() + limits.deadline;
    let package =
        fetch_ok(&at, MAX_PACKAGE_BYTES, limits, deadline).map_err(InstallError::Fetch)?;
    if u64::try_from(package.len()).unwrap_or(u64::MAX) != entry.size {
        return Err(InstallError::Size);
    }
    let hash: String = Sha256::digest(&package)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if hash != entry.sha256 {
        return Err(InstallError::Hash);
    }
    let checked = plugin_package::check(&package, agora_version).map_err(InstallError::Package)?;
    let m = &checked.manifest;
    for (field, same) in [
        ("id", m.id == entry.id),
        ("version", m.version == entry.version),
        ("name", m.name == entry.name),
        ("author", m.author == entry.author),
        ("licence", m.licence == entry.licence),
        ("capabilities", m.capabilities == entry.capabilities),
    ] {
        if !same {
            return Err(InstallError::Mismatch(field));
        }
    }
    let before: Vec<String> = installed
        .as_ref()
        .map(|p| p.capabilities.clone())
        .unwrap_or_default();
    let diff = CapabilityDiff {
        added: m
            .capabilities
            .iter()
            .filter(|c| !before.contains(c))
            .cloned()
            .collect(),
        removed: before
            .iter()
            .filter(|c| !m.capabilities.contains(c))
            .cloned()
            .collect(),
        kept: m
            .capabilities
            .iter()
            .filter(|c| before.contains(c))
            .cloned()
            .collect(),
    };
    Ok(Prepared {
        mode,
        entry,
        installed: installed.map(|p| p.version),
        diff,
        package,
    })
}

/// Keeps a prepared archive once the owner agreed, through the hub's plugin storage (recorded as `plugin_catalog.install` or
/// `.upgrade`).
pub fn commit(
    store: &SqliteStore,
    prepared: &Prepared,
    agora_version: &str,
    now_ms: u64,
) -> Result<Added, InstallError> {
    Plugins::new(store)
        .add_as(
            &prepared.package,
            agora_version,
            now_ms,
            prepared.mode.action(),
        )
        .map_err(|e| match e {
            AddError::Refused(r) => InstallError::Package(r),
            AddError::Full => InstallError::Full,
            AddError::Store(s) => InstallError::Store(s),
        })
}

/// The owner said no: nothing is kept, and the trail says so.
pub fn declined(store: &SqliteStore, prepared: &Prepared, now_ms: u64) {
    record(
        store,
        now_ms,
        prepared.mode.action(),
        &target_of(&prepared.entry.id, Some(&prepared.entry.version)),
        &InstallError::Declined.code(),
    );
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn entry(id: &str) -> Value {
        serde_json::json!({"id": id, "name": "Agent list", "version": "0.1.0", "author": "Ada", "licence": "MIT",
            "capabilities": ["agents.read"], "sha256": "0".repeat(64), "size": 10, "file": format!("plugins/{id}.json")})
    }

    #[test]
    fn the_index_is_read_strictly_and_a_bad_entry_is_refused_alone() {
        let mut extra = entry("extra");
        extra["run"] = "sh".into();
        let mut cap = entry("cap");
        cap["capabilities"] = serde_json::json!(["terminal.write"]);
        let mut twice = entry("twice");
        twice["capabilities"] = serde_json::json!(["agents.read", "agents.read"]);
        let mut escape = entry("escape");
        escape["file"] = "../x.json".into();
        let mut big = entry("big");
        big["size"] = (MAX_PACKAGE_BYTES as u64 + 1).into();
        let mut hidden = entry("hidden");
        hidden["name"] = "Agent\u{202e}list".into();
        let mut described = entry("described");
        described["description"] = "Lists the agents.".into();
        let index = serde_json::json!({"agora-plugin-catalog": 1, "serial": 3, "plugins": [
            entry("ok"), entry("ok"), extra, cap, twice, escape, big, hidden, described, {"id": "BAD"}
        ]});
        let f = read_index(&index.to_string()).unwrap();
        assert_eq!(f.serial, 3);
        assert_eq!(
            f.entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            ["ok", "described"]
        );
        assert_eq!(f.entries[1].description, "Lists the agents.");
        let codes: Vec<(String, String)> = f
            .refused
            .iter()
            .map(|(id, r)| {
                (
                    id.clone(),
                    match r {
                        EntryRefusal::Field(f) => format!("field.{f}"),
                        other => other.code().to_string(),
                    },
                )
            })
            .collect();
        let want: Vec<(String, String)> = [
            ("ok", "duplicate"),
            ("extra", "unknown-field"),
            ("cap", "field.capabilities"),
            ("twice", "field.capabilities"),
            ("escape", "field.file"),
            ("big", "field.size"),
            ("hidden", "field.name"),
            ("#10", "field.id"),
        ]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        assert_eq!(codes, want);
    }

    #[test]
    fn an_index_that_is_not_one_is_refused_whole() {
        let r = |s: &str| read_index(s).unwrap_err();
        assert_eq!(r("[]"), IndexError::Format);
        assert_eq!(r("{"), IndexError::Json);
        assert_eq!(
            r(r#"{"agora-plugin-catalog": 1, "plugins": []}"#),
            IndexError::Format,
            "no serial"
        );
        assert_eq!(
            r(r#"{"agora-plugin-catalog": 2, "serial": 1, "plugins": []}"#),
            IndexError::Newer
        );
        assert_eq!(
            r(r#"{"agora-plugin-catalog": 1, "serial": 1, "plugins": [], "x": 1}"#),
            IndexError::UnknownField("x".into())
        );
        let many = serde_json::json!({"agora-plugin-catalog": 1, "serial": 1, "plugins": vec![entry("a"); MAX_ENTRIES + 1]});
        assert_eq!(r(&many.to_string()), IndexError::TooMany);
    }

    #[test]
    fn the_signature_sits_beside_the_index() {
        let u = Url::parse("https://raw.example.org/cat/main/index.json?x=1").unwrap();
        assert_eq!(signature_url(&u).path, "/cat/main/index.json.sig");
        assert_eq!(index_url(DEFAULT_INDEX_URL), None);
        assert_eq!(index_url("http://h.example/index.json"), None);
    }
}

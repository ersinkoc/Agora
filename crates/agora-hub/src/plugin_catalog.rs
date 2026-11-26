//! The community plugin catalog (ADR-0114 decision 7, build step 4; `docs/ux/plugins.md`, "The community catalog").
//!
//! A public repository holds reviewed plugin archives and an index of them, `index.json`, signed by the catalog's key with
//! `ssh-keygen -Y sign -n agora-plugin-catalog` (`index.json.sig` beside it). The hub, never the browser, reads it, and only on its
//! owner's command (`agora hub plugins update`):
//! - **The key is the owner's to pin** (`agora hub plugins key set FILE`). Without one the catalog cannot be used at all. Pinning
//!   another key forgets the stored catalog, and nothing verified under the old key is offered again.
//! - **The signature comes first.** The index's exact bytes are checked against `index.json.sig` under the pinned key before any of
//!   them is read as JSON; an index that does not verify changes nothing. The index lists each archive's SHA-256 and size, so the
//!   signature covers every archive it names.
//! - **No going back.** The index carries a `serial`; one lower than the last accepted under the same key, or the same serial with other
//!   bytes, is refused whole, so an old signed index (with a version since withdrawn) cannot be served again. Each key keeps its own
//!   floor (the serial and the SHA-256 of the last index accepted under it), checked again in the transaction that keeps a new index,
//!   so two updates at once cannot lower it; pinning a key again finds its floor where it was, and a key never pinned before starts with
//!   none (trust on first use, as a rotation starts the serial anywhere).
//! - **Strict reading.** A format marker, no field it does not know, at most [`MAX_ENTRIES`] entries, each field of a closed kind,
//!   only the capabilities a window knows; an entry with a problem is refused and reported, the others are kept.
//! - **Fetching** is `theme_catalog`'s: `https` (plain `http` only to this machine), no redirect, every answer capped and bounded in
//!   time, an archive only from a relative path under the index's own folder on the same host.
//! - **Installing and upgrading** (`agora hub plugins install|upgrade ID`) fetch the archive, check its size and SHA-256 against the
//!   signed entry, check the package as a window does ([`crate::plugin_package`]), check that its manifest says what the signed entry
//!   says (id, version, name, author, licence, capabilities), and only then keep it through [`crate::plugins::Plugins`]. The command
//!   line shows what it asks to see (for an upgrade, what changed, capabilities first) and asks before it keeps anything. Once the
//!   owner says yes, the transaction that keeps it checks again that the catalog is on, still verified under the key pinned now, still
//!   lists the archive asked about, and that the installed version is the one shown; if anything moved while the owner was asked,
//!   nothing is kept. Nothing is ever installed or upgraded on its own.
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
    day_number, file_url, file_url_ending, index_url as checked_url, io_error, plain_text,
    printable, today, valid_id, valid_licence, valid_sha256, IndexError, MAX_POPULARITY,
};

/// The catalog's address until the owner gives one with `--url`: a placeholder that can never resolve (`.invalid`, RFC 6761), since
/// the catalog's repository does not exist yet. [`index_url`] refuses it, so nothing is ever fetched from it.
pub const DEFAULT_INDEX_URL: &str = "https://plugins.agora.invalid/index.json";
/// The field that marks an index, and the only format this hub reads.
pub const INDEX_FORMAT_KEY: &str = "agora-plugin-catalog";
/// The newest index format this hub reads: 2 adds each entry's optional `added`, `popularity` and `screenshots` (ADR-0120); 1 is still
/// read.
pub const INDEX_FORMAT: u64 = 2;
/// ADR-0120: at most this many screenshots per plugin, each at most [`MAX_SCREENSHOT_BYTES`] and [`MAX_SCREENSHOT_PIXELS`], at most
/// [`MAX_SCREENSHOTS_TOTAL`] of them per catalog, each with an alt text of at most [`MAX_ALT`] characters.
pub const MAX_SCREENSHOTS: usize = 4;
pub const MAX_SCREENSHOT_BYTES: usize = 512 * 1024;
pub const MAX_SCREENSHOT_PIXELS: (u32, u32) = (2560, 1600);
pub const MAX_SCREENSHOTS_TOTAL: usize = 32 * 1024 * 1024;
pub const MAX_ALT: usize = 200;
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
/// the stored index (for `status`), the pinned key (one OpenSSH line), the key the stored catalog was verified under, and, per key,
/// the floor: [`FLOOR_SETTING`] and the key's fingerprint, holding `<serial> <sha256>` of the last index accepted under that key.
pub const SWITCH_SETTING: &str = "plugin_catalog";
pub const URL_SETTING: &str = "plugin_catalog_url";
pub const UPDATED_SETTING: &str = "plugin_catalog_updated_ms";
pub const SERIAL_SETTING: &str = "plugin_catalog_serial";
pub const KEY_SETTING: &str = "plugin_catalog_key";
pub const VERIFIED_KEY_SETTING: &str = "plugin_catalog_verified_key";
pub const FLOOR_SETTING: &str = "plugin_catalog_floor.";

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
    /// ADR-0120 (format 2): the day the catalog first listed it, `YYYY-MM-DD`; empty when not given.
    pub added: String,
    /// ADR-0120 (format 2): the popularity the signed index publishes, 0 when not given. Never counted here.
    pub popularity: u64,
    /// ADR-0120 (format 2): its screenshots as the index lists them (empty for an entry read back from storage: [`list`] reads the
    /// kept ones' alt texts).
    pub screenshots: Vec<Shot>,
}

/// One screenshot as a signed index lists it (ADR-0120).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shot {
    /// Its path under the index's folder, ending in `.png` or `.webp`.
    pub file: String,
    pub sha256: String,
    pub size: u64,
    /// One line for people who cannot see it; empty when not given.
    pub alt: String,
}

/// A screenshot the hub fetched and checked, ready to keep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptShot {
    pub id: String,
    /// Its place among the plugin's kept screenshots, from 0.
    pub n: u32,
    pub media_type: &'static str,
    pub data: Vec<u8>,
    pub alt: String,
}

/// Why one screenshot was not kept; the plugin and its other screenshots are (ADR-0120).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShotRefusal {
    /// It could not be fetched in time, or its server answered another status than 200.
    Fetch,
    /// Larger than the index says or than [`MAX_SCREENSHOT_BYTES`], or the catalog's screenshots together over [`MAX_SCREENSHOTS_TOTAL`].
    Size,
    /// Its SHA-256 is not the index's.
    Hash,
    /// Not a PNG or WebP image, or not the kind its name says.
    Kind,
    /// Larger than [`MAX_SCREENSHOT_PIXELS`] by its header.
    Pixels,
}

impl ShotRefusal {
    /// The stable code the command line words (`cli-hub-plugins-shot-<code>`).
    pub fn code(self) -> &'static str {
        match self {
            ShotRefusal::Fetch => "fetch",
            ShotRefusal::Size => "size",
            ShotRefusal::Hash => "hash",
            ShotRefusal::Kind => "kind",
            ShotRefusal::Pixels => "pixels",
        }
    }
}

/// The media type and size in pixels of a PNG or WebP image, read from its first bytes only; `None` for anything else. Nothing here
/// decodes the image: only a browser does (ADR-0120).
pub fn image_header(b: &[u8]) -> Option<(&'static str, u32, u32)> {
    let be = |i: usize| Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?));
    let le16 = |i: usize| {
        Some(u32::from(u16::from_le_bytes(
            b.get(i..i + 2)?.try_into().ok()?,
        )))
    };
    let le24 = |i: usize| -> Option<u32> {
        let x = b.get(i..i + 3)?;
        Some(u32::from(x[0]) | u32::from(x[1]) << 8 | u32::from(x[2]) << 16)
    };
    if b.starts_with(b"\x89PNG\r\n\x1a\n") && b.get(12..16) == Some(b"IHDR".as_slice()) {
        return Some(("image/png", be(16)?, be(20)?));
    }
    if b.get(0..4) == Some(b"RIFF".as_slice()) && b.get(8..12) == Some(b"WEBP".as_slice()) {
        return match b.get(12..16)? {
            b"VP8 " if b.get(23..26) == Some([0x9d, 0x01, 0x2a].as_slice()) => {
                Some(("image/webp", le16(26)? & 0x3fff, le16(28)? & 0x3fff))
            }
            b"VP8L" if b.get(20) == Some(&0x2f) => {
                let x = b.get(21..25)?;
                let w = 1 + (u32::from(x[0]) | (u32::from(x[1]) & 0x3f) << 8);
                let h = 1
                    + (u32::from(x[1]) >> 6
                        | u32::from(x[2]) << 2
                        | (u32::from(x[3]) & 0x0f) << 10);
                Some(("image/webp", w, h))
            }
            b"VP8X" => Some(("image/webp", 1 + le24(24)?, 1 + le24(27)?)),
            _ => None,
        };
    }
    None
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
    /// Its serial is lower than the last accepted under the pinned key.
    Rollback {
        seen: u64,
        offered: u64,
    },
    /// Its serial is the last accepted under the pinned key, but its bytes are not that index's.
    SerialReused {
        serial: u64,
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
            UpdateError::SerialReused { .. } => "serial-reused".into(),
            UpdateError::Store(_) => "internal".into(),
        }
    }
}

impl From<StoreError> for UpdateError {
    fn from(e: StoreError) -> Self {
        UpdateError::Store(e.to_string())
    }
}

impl From<rusqlite::Error> for UpdateError {
    fn from(e: rusqlite::Error) -> Self {
        UpdateError::Store(e.to_string())
    }
}

/// What an accepted index gave. `refused` names each refused entry by its id (or `#n`, its place in the list).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    pub serial: u64,
    /// The SHA-256 of the index's bytes (64 lowercase hex): with the serial, the key's floor.
    pub sha256: String,
    pub entries: Vec<Entry>,
    pub refused: Vec<(String, EntryRefusal)>,
    /// ADR-0120: the screenshots fetched and checked, and those not kept (plugin id, place in its index entry from 0, why).
    pub shots: Vec<KeptShot>,
    pub shots_refused: Vec<(String, usize, ShotRefusal)>,
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

/// What format 2 adds to an entry (ADR-0120), and the fields of one of its screenshots.
const ENTRY_FIELDS_2: [&str; 3] = ["added", "popularity", "screenshots"];
const SHOT_FIELDS: [&str; 4] = ["file", "sha256", "size", "alt"];

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

fn read_shot(v: &Value) -> Option<Shot> {
    let o = v.as_object()?;
    if o.keys().any(|k| !SHOT_FIELDS.contains(&k.as_str())) {
        return None;
    }
    let file = o.get("file")?.as_str()?.to_string();
    let probe = Url {
        https: true,
        host: "h".into(),
        port: 443,
        path: "/index.json".into(),
    };
    file_url_ending(&probe, &file, &[".png", ".webp"])?;
    let sha256 = o
        .get("sha256")?
        .as_str()
        .filter(|s| valid_sha256(s))?
        .to_string();
    let size = o
        .get("size")?
        .as_u64()
        .filter(|n| (1..=MAX_SCREENSHOT_BYTES as u64).contains(n))?;
    let alt = match o.get("alt") {
        None => String::new(),
        Some(a) => a
            .as_str()
            .filter(|a| a.is_empty() || plain_text(a, MAX_ALT))?
            .to_string(),
    };
    Some(Shot {
        file,
        sha256,
        size,
        alt,
    })
}

fn read_entry(v: &Value, format: u64) -> Result<Entry, EntryRefusal> {
    let Some(o) = v.as_object() else {
        return Err(EntryRefusal::Field("entry"));
    };
    let known = |k: &str| ENTRY_FIELDS.contains(&k) || (format >= 2 && ENTRY_FIELDS_2.contains(&k));
    if let Some(k) = o.keys().find(|k| !known(k.as_str())) {
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
    // Optional: missing and empty are the same.
    let description = match o.get("description") {
        None => String::new(),
        Some(Value::String(d)) if d.is_empty() => String::new(),
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
    let added = match o.get("added") {
        None => String::new(),
        Some(_) => text("added", &|s| day_number(s).is_some())?,
    };
    let popularity = match o.get("popularity") {
        None => 0,
        Some(p) => p
            .as_u64()
            .filter(|p| *p <= MAX_POPULARITY)
            .ok_or(EntryRefusal::Field("popularity"))?,
    };
    let screenshots = match o.get("screenshots") {
        None => Vec::new(),
        Some(list) => {
            let list = list
                .as_array()
                .filter(|l| l.len() <= MAX_SCREENSHOTS)
                .ok_or(EntryRefusal::Field("screenshots"))?;
            list.iter()
                .map(read_shot)
                .collect::<Option<Vec<_>>>()
                .ok_or(EntryRefusal::Field("screenshots"))?
        }
    };
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
        added,
        popularity,
        screenshots,
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
        match read_entry(v, format) {
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
        sha256: hex(&Sha256::digest(text.as_bytes())),
        entries,
        refused,
        shots: Vec::new(),
        shots_refused: Vec::new(),
    })
}

/// ADR-0120: fetches and checks every screenshot `fetched`'s entries list, within `deadline`: the size and SHA-256 its signed entry
/// gives, a PNG or WebP header matching its name, at most [`MAX_SCREENSHOT_PIXELS`], at most [`MAX_SCREENSHOTS_TOTAL`] in all. A
/// screenshot that fails is not kept and is named; the plugin stays.
fn fetch_shots(fetched: &mut Fetched, index: &Url, limits: Limits, deadline: Instant) {
    let mut total = 0usize;
    for e in &fetched.entries {
        let mut n = 0u32;
        for (i, shot) in e.screenshots.iter().enumerate() {
            let checked = (|| -> Result<(&'static str, Vec<u8>), ShotRefusal> {
                let at = file_url_ending(index, &shot.file, &[".png", ".webp"])
                    .ok_or(ShotRefusal::Kind)?;
                let body = match get_within(&at, MAX_SCREENSHOT_BYTES, limits.step, deadline) {
                    Ok((200, body)) => body,
                    Err(e) if io_error(&e) == IndexError::TooLarge => {
                        return Err(ShotRefusal::Size)
                    }
                    _ => return Err(ShotRefusal::Fetch),
                };
                if body.len() as u64 != shot.size || total + body.len() > MAX_SCREENSHOTS_TOTAL {
                    return Err(ShotRefusal::Size);
                }
                if hex(&Sha256::digest(&body)) != shot.sha256 {
                    return Err(ShotRefusal::Hash);
                }
                let (media, w, h) = image_header(&body).ok_or(ShotRefusal::Kind)?;
                let named = if shot.file.ends_with(".png") {
                    "image/png"
                } else {
                    "image/webp"
                };
                if media != named {
                    return Err(ShotRefusal::Kind);
                }
                if w == 0 || h == 0 || w > MAX_SCREENSHOT_PIXELS.0 || h > MAX_SCREENSHOT_PIXELS.1 {
                    return Err(ShotRefusal::Pixels);
                }
                Ok((media, body))
            })();
            match checked {
                Ok((media_type, data)) => {
                    total += data.len();
                    fetched.shots.push(KeptShot {
                        id: e.id.clone(),
                        n,
                        media_type,
                        data,
                        alt: shot.alt.clone(),
                    });
                    n += 1;
                }
                Err(r) => fetched.shots_refused.push((e.id.clone(), i, r)),
            }
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The last index accepted under one key: nothing lower is taken again under that key, nor the same serial with other bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Floor {
    pub serial: u64,
    pub sha256: String,
}

fn floor_setting(key: &PublicKey) -> String {
    format!("{FLOOR_SETTING}{}", key.fingerprint())
}

fn setting_in(conn: &rusqlite::Connection, key: &str) -> rusqlite::Result<Option<Vec<u8>>> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
        r.get(0)
    })
    .optional()
}

/// `key`'s floor, read through `conn` (the store's connection, or a transaction). A value that cannot be read refuses, rather than
/// reading as no floor.
fn floor_in(conn: &rusqlite::Connection, key: &PublicKey) -> Result<Option<Floor>, UpdateError> {
    let Some(v) = setting_in(conn, &floor_setting(key))? else {
        return Ok(None);
    };
    let parsed = std::str::from_utf8(&v).ok().and_then(|t| {
        let (serial, sha256) = t.split_once(' ')?;
        let serial: u64 = serial.parse().ok()?;
        ((1..=MAX_SERIAL).contains(&serial) && valid_sha256(sha256)).then(|| Floor {
            serial,
            sha256: sha256.to_string(),
        })
    });
    parsed
        .map(Some)
        .ok_or_else(|| UpdateError::Store("plugin catalog floor unreadable".into()))
}

/// Refuses `fetched` below `floor`: a lower serial, or the same serial with other bytes.
fn above(floor: Option<&Floor>, fetched: &Fetched) -> Result<(), UpdateError> {
    match floor {
        Some(f) if fetched.serial < f.serial => Err(UpdateError::Rollback {
            seen: f.serial,
            offered: fetched.serial,
        }),
        Some(f) if fetched.serial == f.serial && fetched.sha256 != f.sha256 => {
            Err(UpdateError::SerialReused {
                serial: fetched.serial,
            })
        }
        _ => Ok(()),
    }
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
/// it; refuses one below `floor` (the key's, as read before fetching). Changes nothing; [`save`] keeps the result, and checks the floor
/// again.
pub fn fetch(
    url: &Url,
    key: &PublicKey,
    floor: Option<&Floor>,
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
    let mut fetched = read_index(&text).map_err(UpdateError::Index)?;
    above(floor, &fetched)?;
    fetch_shots(&mut fetched, url, limits, deadline);
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

/// Replaces the stored catalog with `fetched`, remembers the address, the serial and the key it was verified under, and says when, and
/// raises `key`'s floor to it, in one transaction. The floor is read again inside that transaction, so an update that read an older
/// floor (two at once) cannot lower it: `fetched` below it is refused and nothing changes.
pub fn save(
    store: &SqliteStore,
    fetched: &Fetched,
    url: &str,
    key: &PublicKey,
    now_ms: u64,
) -> Result<(), UpdateError> {
    let mut conn = store.lock()?;
    let tx = write_tx(&mut conn)?;
    above(floor_in(&tx, key)?.as_ref(), fetched)?;
    tx.execute("DELETE FROM plugin_catalog", [])?;
    for e in &fetched.entries {
        tx.execute(
            "INSERT INTO plugin_catalog (id, name, version, author, licence, description, capabilities, sha256, size, file, added,
             popularity)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
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
                e.file,
                e.added,
                i64::try_from(e.popularity).unwrap_or(i64::MAX)
            ],
        )?;
    }
    tx.execute("DELETE FROM plugin_catalog_screenshot", [])?;
    for k in &fetched.shots {
        if fetched.entries.iter().any(|e| e.id == k.id) {
            tx.execute(
                "INSERT INTO plugin_catalog_screenshot (id, n, media_type, data, alt) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![k.id, k.n, k.media_type, k.data, k.alt],
            )?;
        }
    }
    put(&tx, UPDATED_SETTING, now_ms.to_string().as_bytes())?;
    put(&tx, URL_SETTING, url.as_bytes())?;
    put(&tx, SERIAL_SETTING, fetched.serial.to_string().as_bytes())?;
    put(&tx, VERIFIED_KEY_SETTING, key.line().as_bytes())?;
    put(
        &tx,
        &floor_setting(key),
        format!("{} {}", fetched.serial, fetched.sha256).as_bytes(),
    )?;
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
        let floor = floor_in(&*store.lock()?, &key)?;
        let mut fetched = fetch(&at, &key, floor.as_ref(), limits)?;
        // ADR-0120: a day after today's (UTC) is not one the catalog can have listed it on.
        let today = today(now_ms);
        let (later, kept): (Vec<Entry>, Vec<Entry>) = std::mem::take(&mut fetched.entries)
            .into_iter()
            .partition(|e| day_number(&e.added).is_some_and(|d| d > today));
        fetched.entries = kept;
        fetched.refused.extend(
            later
                .into_iter()
                .map(|e| (e.id, EntryRefusal::Field("added"))),
        );
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
/// catalog (nothing verified under another key is offered); pinning the same key again changes nothing else. Each key's floor is kept:
/// pinning a key again finds the floor it had. Recorded in the audit trail as the owner's, with the key's fingerprint (or the refusal).
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
        added: r.get(10)?,
        popularity: u64::try_from(r.get::<_, i64>(11)?).unwrap_or(0),
        screenshots: Vec::new(),
    })
}

const COLUMNS: &str =
    "id, name, version, author, licence, description, capabilities, sha256, size, file, added, popularity";

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
    // ADR-0120: the kept screenshots' alt texts, by plugin, in order.
    let mut alts: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    if !entries.is_empty() {
        let conn = store.lock()?;
        let mut stmt =
            conn.prepare("SELECT id, alt FROM plugin_catalog_screenshot ORDER BY id, n")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (id, alt) = row?;
            alts.entry(id).or_default().push(alt);
        }
    }
    Ok(v1::PluginCatalogListResult {
        on: true,
        entries: entries
            .into_iter()
            .map(|e| v1::PluginCatalogEntry {
                screenshots: alts.remove(&e.id).unwrap_or_default(),
                added: e.added,
                popularity: e.popularity,
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

/// `PluginCatalogScreenshot` (ADR-0120): screenshot `n` of catalog plugin `id`, as kept. `plugin.catalog_off` while the catalog is
/// off; `plugin.not_found` for a plugin, or a place, the catalog does not have (with no key pinned, or one pinned since the last
/// update, it has none).
pub fn screenshot(
    store: &SqliteStore,
    id: &str,
    n: u32,
) -> Result<v1::PluginCatalogScreenshotResult, agora_protocol::Error> {
    use agora_protocol::{Error, ErrorCode};
    let internal = |_| Error::new(ErrorCode::Internal).with("incident_id", "plugin_catalog");
    if !is_on(store).map_err(internal)? {
        return Err(Error::new(ErrorCode::PluginCatalogOff));
    }
    let not_found = || Error::new(ErrorCode::PluginNotFound).with("plugin_id", id);
    if bound(store).map_err(internal)?.is_none() {
        return Err(not_found());
    }
    let conn = store.lock().map_err(internal)?;
    conn.query_row(
        "SELECT media_type, data, alt FROM plugin_catalog_screenshot WHERE id = ?1 AND n = ?2",
        params![id, n],
        |r| {
            Ok(v1::PluginCatalogScreenshotResult {
                media_type: r.get(0)?,
                data: r.get(1)?,
                alt: r.get(2)?,
            })
        },
    )
    .optional()
    .map_err(|e| internal(StoreError::from(e)))?
    .ok_or_else(not_found)
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
    /// What the owner was asked about moved before the answer (`catalog`: the stored entry is no longer the archive fetched;
    /// `installed`: the installed version is no longer the one shown).
    Changed(&'static str),
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
            InstallError::Changed(f) => format!("changed.{f}"),
            InstallError::Store(_) => "internal".into(),
        }
    }
}

impl From<StoreError> for InstallError {
    fn from(e: StoreError) -> Self {
        InstallError::Store(e.to_string())
    }
}

impl From<rusqlite::Error> for InstallError {
    fn from(e: rusqlite::Error) -> Self {
        InstallError::Store(e.to_string())
    }
}

impl From<AddError> for InstallError {
    fn from(e: AddError) -> Self {
        match e {
            AddError::Refused(r) => InstallError::Package(r),
            AddError::Full => InstallError::Full,
            AddError::Store(s) => InstallError::Store(s),
        }
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
    if hex(&Sha256::digest(&package)) != entry.sha256 {
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
/// `.upgrade`). The owner may have answered long after [`prepare`]; inside the transaction that keeps it, everything the owner was asked
/// about is checked again ([`unmoved`]), and if anything moved nothing is kept.
pub fn commit(
    store: &SqliteStore,
    prepared: &Prepared,
    agora_version: &str,
    now_ms: u64,
) -> Result<Added, InstallError> {
    let result = Plugins::new(store).add_guarded(
        &prepared.package,
        agora_version,
        now_ms,
        |tx, installed| unmoved(tx, prepared, installed),
    );
    let (target, outcome) = match &result {
        Ok(a) => (format!("{}@{}", a.id, a.version), "ok".to_string()),
        Err(e) => (
            target_of(&prepared.entry.id, Some(&prepared.entry.version)),
            e.code(),
        ),
    };
    record(store, now_ms, prepared.mode.action(), &target, &outcome);
    result
}

/// What [`prepare`] checked, checked again through the write transaction `tx` that keeps the archive: the catalog is on, a key is pinned
/// and the stored catalog was verified under it, the stored entry still names the archive fetched (its SHA-256), and the installed
/// version (`installed`, read in the same transaction) is still the one shown (none for an install).
fn unmoved(
    tx: &rusqlite::Transaction<'_>,
    prepared: &Prepared,
    installed: Option<&str>,
) -> Result<(), InstallError> {
    if setting_in(tx, SWITCH_SETTING)?.as_deref() != Some(b"on".as_slice()) {
        return Err(InstallError::Off);
    }
    let pinned = setting_in(tx, KEY_SETTING)?
        .and_then(|v| String::from_utf8(v).ok())
        .and_then(|t| PublicKey::parse(&t).ok())
        .ok_or(InstallError::NoKey)?;
    if setting_in(tx, VERIFIED_KEY_SETTING)?.as_deref() != Some(pinned.line().as_bytes()) {
        return Err(InstallError::NotVerified);
    }
    let sha256: Option<String> = tx
        .query_row(
            "SELECT sha256 FROM plugin_catalog WHERE id = ?1",
            [&prepared.entry.id],
            |r| r.get(0),
        )
        .optional()?;
    match sha256 {
        None => return Err(InstallError::NotInCatalog),
        Some(s) if s != prepared.entry.sha256 => return Err(InstallError::Changed("catalog")),
        Some(_) => {}
    }
    if installed != prepared.installed.as_deref() {
        return Err(InstallError::Changed("installed"));
    }
    Ok(())
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

    /// ADR-0120: the type and size come from the first bytes of a PNG or of each WebP kind (lossy, lossless, extended); anything else
    /// is not an image here.
    #[test]
    fn an_image_header_gives_its_type_and_pixels() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend_from_slice(&1280u32.to_be_bytes());
        png.extend_from_slice(&800u32.to_be_bytes());
        assert_eq!(image_header(&png), Some(("image/png", 1280, 800)));
        // Lossy: the frame tag, the start code 9d 01 2a, then 14-bit width and height.
        let mut lossy = b"RIFF\0\0\0\0WEBPVP8 \0\0\0\0\0\0\0\x9d\x01\x2a".to_vec();
        lossy.extend_from_slice(&640u16.to_le_bytes());
        lossy.extend_from_slice(&480u16.to_le_bytes());
        assert_eq!(image_header(&lossy), Some(("image/webp", 640, 480)));
        // Lossless: 0x2f, then 14 bits each of width - 1 and height - 1.
        let (w, h) = (300u32 - 1, 200u32 - 1);
        let bits = w | (h << 14);
        let mut lossless = b"RIFF\0\0\0\0WEBPVP8L\0\0\0\0\x2f".to_vec();
        lossless.extend_from_slice(&bits.to_le_bytes());
        assert_eq!(image_header(&lossless), Some(("image/webp", 300, 200)));
        let mut extended = b"RIFF\0\0\0\0WEBPVP8X\0\0\0\0\0\0\0\0".to_vec();
        extended.extend_from_slice(&[0xff, 0x09, 0x00, 0x3f, 0x06, 0x00]);
        assert_eq!(image_header(&extended), Some(("image/webp", 2560, 1600)));
        for not in [
            &b"GIF89a\x01\0\x01\0"[..],
            b"<svg xmlns='http://www.w3.org/2000/svg'/>",
            &png[..20],
            b"",
        ] {
            assert_eq!(image_header(not), None);
        }
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
        let mut blank = entry("blank");
        blank["description"] = "".into();
        let index = serde_json::json!({"agora-plugin-catalog": 1, "serial": 3, "plugins": [
            entry("ok"), entry("ok"), extra, cap, twice, escape, big, hidden, described, {"id": "BAD"}, blank
        ]});
        let f = read_index(&index.to_string()).unwrap();
        assert_eq!(f.serial, 3);
        assert_eq!(
            f.entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            ["ok", "described", "blank"]
        );
        assert_eq!(f.entries[1].description, "Lists the agents.");
        assert_eq!(f.entries[2].description, "", "an empty description is none");
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
            r(r#"{"agora-plugin-catalog": 3, "serial": 1, "plugins": []}"#),
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

//! Which devices this machine trusts, and how that changes (ADR-0061).
//!
//! The trusted-devices file (`<data>/trusted-devices`, owner-only) holds one device per line: `<id> <public key as 130 hex digits>`,
//! as `agora node trust ID KEY` has always written it. ADR-0061 adds optional fields after the key: `since=<ms>` (when its trust here
//! began: the owner's confirmation, or the endorsement's signing time), and for a device another device endorsed `expires=<ms>` and
//! `via=<device>`. A line with any other field is skipped, never trusted, so a typo cannot widen trust; a node from before ADR-0061
//! skips such a line too (it trusts less, never more).
//!
//! Trust grows only two ways: the owner says yes on the machine (`agora node trust`, `agora node enroll`, `agora node trust
//! --from-hub`), or a device this machine already trusts signs an endorsement, verified here ([`DeviceTrust::endorse`]); chains are at
//! most two endorsements deep and an endorsed device never outlasts its endorser. The hub only carries endorsements; it can neither
//! make nor change one. A revocation signed by a trusted device ([`DeviceTrust::revoke`]) and `agora node untrust` narrow it at once
//! (with every device the removed one endorsed), and are remembered (`<data>/device-revocations`) so an older endorsement the hub
//! sends again cannot bring the device back; a revocation removes only trust that began at or before it was signed, so one sent again
//! later cannot undo a newer decision. Every change happens under a lock file shared by every process (`<file>.lock`) and is written
//! beside the file and renamed over it. Every endorsement and revocation applied or refused is written to `<data>/device-trust.log`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use agora_protocol::v1::{DeviceEndorsement, DeviceInfo, DeviceRevocation};
use agora_protocol::{Error, ErrorCode};

/// The longest an endorsement lasts from when it was signed, and how far each use renews it (ADR-0061: 90 days, renewable by use).
pub const MAX_ENDORSEMENT_MS: u64 = 90 * 24 * 60 * 60 * 1000;
/// How far ahead of this machine's clock a signer's clock may be.
const SKEW_MS: u64 = 5 * 60 * 1000;
/// A renewal is written only when it moves the expiry by more than this (a device used all day writes once).
const RENEW_STEP_MS: u64 = 24 * 60 * 60 * 1000;
/// Endorsements in a row: a device the owner confirmed, one it endorsed, one that one endorsed. No deeper.
pub const MAX_DEPTH: u32 = 2;
/// Revocations remembered: at most this many (beyond it, ones older than any endorsement can be are forgotten first)...
pub const MAX_REVOCATIONS: usize = 256;
/// ...and none longer than this: every endorsement signed before such a revocation expired long ago.
pub const REVOCATION_KEPT_MS: u64 = 400 * 24 * 60 * 60 * 1000;

/// Remembered revocations, one `<device id> <issued ms>` per line.
pub const REVOCATIONS_FILE: &str = "device-revocations";
/// What this machine did with each endorsement and revocation, one line each, for its owner.
pub const AUDIT_FILE: &str = "device-trust.log";

/// One device of the trusted-devices file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub key: Vec<u8>,
    /// `None`: confirmed on the machine, no end. `Some`: endorsed, trusted until then (and never past its endorser).
    pub expires_ms: Option<u64>,
    /// The device that endorsed it.
    pub via: Option<String>,
    /// When its trust here began; `None` on a line written before ADR-0061's review (counts as the beginning of time).
    pub since: Option<u64>,
    /// ADR-0049 limits it inherited from a limited endorser (`projects=a,b`; empty: none): it may act only in these projects, like
    /// the device that endorsed it. `None`: no inherited limit (the policy file may still name it).
    pub projects: Option<std::collections::BTreeSet<String>>,
}

impl Entry {
    fn line(&self) -> String {
        let mut l = format!("{} {}", self.id, hex(&self.key));
        if let Some(s) = self.since {
            l.push_str(&format!(" since={s}"));
        }
        if let Some(e) = self.expires_ms {
            l.push_str(&format!(" expires={e}"));
        }
        if let Some(v) = &self.via {
            l.push_str(&format!(" via={v}"));
        }
        if let Some(p) = &self.projects {
            l.push_str(" projects=");
            l.push_str(&p.iter().cloned().collect::<Vec<_>>().join(","));
        }
        l
    }
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn from_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// `[A-Za-z0-9._-]{1,64}`: what the file can hold.
pub fn valid_device_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// An uncompressed P-256 point encoding (65 bytes, `04` first).
pub fn valid_key(key: &[u8]) -> bool {
    key.len() == 65 && key.first() == Some(&4)
}

/// Text from the hub (a device's name, a person) as this machine may show it: every control, escape, bidi, zero-width and other
/// hidden character removed (agora-core's list, ADR-0057), at most `max` characters (then `…`). A name can then neither move the
/// cursor, erase a line nor reorder what is printed after it.
pub fn shown(text: &str, max: usize) -> String {
    let clean: Vec<char> = text
        .chars()
        .filter(|c| !c.is_control() && !agora_core::text::is_hidden(*c))
        .collect();
    if clean.len() > max {
        let mut s: String = clean[..max].iter().collect();
        s.push('…');
        s
    } else {
        clean.into_iter().collect()
    }
}

/// A value for the log: an id as it is when it is one, anything else with every character outside `[A-Za-z0-9._-]` as `?` and at most
/// 64 characters, so a value can never start a line of its own.
fn logged(value: &str) -> String {
    if valid_device_id(value) {
        return value.to_string();
    }
    value
        .chars()
        .take(64)
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '?'
            }
        })
        .collect()
}

/// One line of the file: `None` for a blank line or a comment, `Some(None)` for a line that does not parse (skipped, never
/// trusted), `Some(Some(entry))` for a device.
pub fn parse_line(line: &str) -> Option<Option<Entry>> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let mut words = line.split_whitespace();
    let (Some(id), Some(key)) = (words.next(), words.next()) else {
        return Some(None);
    };
    let Some(key) = from_hex(key).filter(|k| valid_device_id(id) && valid_key(k)) else {
        return Some(None);
    };
    let mut entry = Entry {
        id: id.to_string(),
        key,
        expires_ms: None,
        via: None,
        since: None,
        projects: None,
    };
    for w in words {
        match w.split_once('=') {
            Some(("expires", v)) if entry.expires_ms.is_none() => match v.parse() {
                Ok(ms) => entry.expires_ms = Some(ms),
                Err(_) => return Some(None),
            },
            Some(("since", v)) if entry.since.is_none() => match v.parse() {
                Ok(ms) => entry.since = Some(ms),
                Err(_) => return Some(None),
            },
            Some(("projects", v)) if entry.projects.is_none() => {
                entry.projects = Some(
                    v.split(',')
                        .filter(|p| !p.is_empty())
                        .map(str::to_string)
                        .collect(),
                );
            }
            Some(("via", v)) if entry.via.is_none() && valid_device_id(v) => {
                entry.via = Some(v.to_string());
            }
            _ => return Some(None),
        }
    }
    // An endorsed device has an end; a line naming an endorser without one is not trusted.
    if entry.via.is_some() && entry.expires_ms.is_none() {
        return Some(None);
    }
    Some(Some(entry))
}

/// Every device the file lists (the last line wins for an id written twice) and how many lines were skipped.
pub fn read_entries(path: &Path) -> (Vec<Entry>, usize) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return (Vec::new(), 0);
    };
    let mut out: Vec<Entry> = Vec::new();
    let mut skipped = 0;
    for line in text.lines() {
        match parse_line(line) {
            None => {}
            Some(None) => skipped += 1,
            Some(Some(e)) => {
                out.retain(|o| o.id != e.id);
                out.push(e);
            }
        }
    }
    (out, skipped)
}

/// How many endorsements lead to `e` (0: confirmed on the machine); `None` when the chain is broken (an endorser that is gone), loops,
/// or is deeper than any chain this node accepts.
pub fn depth(entries: &[Entry], e: &Entry) -> Option<u32> {
    let mut d = 0;
    let mut at = e;
    while let Some(v) = &at.via {
        d += 1;
        if d > MAX_DEPTH {
            return None;
        }
        at = entries.iter().find(|x| x.id == *v)?;
    }
    Some(d)
}

/// Until when `e` is trusted here: `Some(None)` with no end (confirmed), `Some(Some(t))` until `t` (its own expiry and every
/// endorser's up the chain, whichever comes first), `None` when its chain is broken.
pub fn effective_expiry(entries: &[Entry], e: &Entry) -> Option<Option<u64>> {
    depth(entries, e)?;
    let mut end = e.expires_ms;
    let mut at = e;
    while let Some(v) = &at.via {
        at = entries.iter().find(|x| x.id == *v)?;
        end = match (end, at.expires_ms) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
    }
    Some(end)
}

/// What `id` may act in here (ADR-0049 with ADR-0061): the policy's line for it, what it inherited when it was endorsed, and what its
/// endorser may act in NOW, up the chain (at most two endorsements), all at once. So a device demoted or narrowed by the owner's
/// policy takes the devices it endorsed down with it, and the policy can never widen what was inherited. `None`: not limited. A chain
/// whose endorser is gone limits to nothing.
pub fn effective_limit(
    entries: &[Entry],
    policy: &std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
    id: &str,
) -> Option<std::collections::BTreeSet<String>> {
    let mut limit: Option<std::collections::BTreeSet<String>> = None;
    let mut at = id.to_string();
    for _ in 0..=MAX_DEPTH {
        let entry = entries.iter().find(|e| e.id == at);
        limit = crate::limits::combine(limit, policy.get(&at).cloned());
        limit = crate::limits::combine(limit, entry.and_then(|e| e.projects.clone()));
        match entry.and_then(|e| e.via.clone()) {
            None => return limit,
            Some(v) if entries.iter().any(|e| e.id == v) => at = v,
            Some(_) => return Some(std::collections::BTreeSet::new()),
        }
    }
    // Deeper than any chain this node accepts.
    Some(std::collections::BTreeSet::new())
}

/// Is `e` trusted at `now_ms`?
pub fn alive(entries: &[Entry], e: &Entry, now_ms: u64) -> bool {
    matches!(effective_expiry(entries, e), Some(end) if end.is_none_or(|t| now_ms < t))
}

/// `entries` without `id` and without every device whose chain of endorsers leads to it.
fn without(entries: &[Entry], id: &str) -> Vec<Entry> {
    let mut gone = vec![id.to_string()];
    loop {
        let more: Vec<String> = entries
            .iter()
            .filter(|e| !gone.contains(&e.id) && e.via.as_ref().is_some_and(|v| gone.contains(v)))
            .map(|e| e.id.clone())
            .collect();
        if more.is_empty() {
            break;
        }
        gone.extend(more);
    }
    entries
        .iter()
        .filter(|e| !gone.contains(&e.id))
        .cloned()
        .collect()
}

/// Holds the lock file beside the trusted-devices file (every process that changes it takes it), released when dropped.
struct FileLock(std::fs::File);

impl FileLock {
    fn take(file: &Path) -> std::io::Result<Self> {
        let mut name = file.as_os_str().to_owned();
        name.push(".lock");
        let f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(PathBuf::from(name))?;
        f.lock()?;
        Ok(Self(f))
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

/// Writes `entries` back: every line of the file that is not a device (comments, blank lines, lines it skipped) is kept as it was,
/// and devices keep their order. Written beside the file and renamed over it, owner-only from the start.
fn write_entries(path: &Path, entries: &[Entry]) -> std::io::Result<()> {
    let old = std::fs::read_to_string(path).unwrap_or_default();
    let mut written: Vec<&str> = Vec::new();
    let mut out = String::new();
    for l in old.lines() {
        match parse_line(l) {
            Some(Some(e)) => {
                if let Some(n) = entries.iter().find(|n| n.id == e.id) {
                    if !written.contains(&n.id.as_str()) {
                        out.push_str(&n.line());
                        out.push('\n');
                        written.push(&n.id);
                    }
                }
            }
            _ => {
                out.push_str(l);
                out.push('\n');
            }
        }
    }
    for n in entries {
        if !written.contains(&n.id.as_str()) {
            out.push_str(&n.line());
            out.push('\n');
        }
    }
    agora_platform::write_private(path, out.as_bytes())
}

fn now() -> u64 {
    crate::grants::now_ms()
}

/// The owner on the machine trusts `id` with `key`, with no end, from now (`agora node trust`, and a yes at enrollment or
/// `--from-hub`). A remembered revocation of it is forgotten: the person at the machine just decided otherwise. It replaces a key
/// already trusted under `id` (what `agora node trust ID KEY` has always done; the hub-fed paths ask first, see `--replace`).
pub fn confirm(file: &Path, id: &str, key: &[u8]) -> Result<(), String> {
    if !valid_device_id(id) {
        return Err("the device id may contain only letters, digits, '.', '_' and '-'".into());
    }
    if !valid_key(key) {
        return Err("the key is not an uncompressed P-256 point (65 bytes)".into());
    }
    let _lock = FileLock::take(file).map_err(|e| e.to_string())?;
    let (mut entries, _) = read_entries(file);
    let replaced = entries.iter().any(|e| e.id == id && e.key != key);
    // A new key under the id ends what the old key endorsed.
    if replaced {
        entries = without(&entries, id);
    }
    entries.retain(|e| e.id != id);
    entries.push(Entry {
        id: id.to_string(),
        key: key.to_vec(),
        expires_ms: None,
        via: None,
        since: Some(now()),
        projects: None,
    });
    write_entries(file, &entries).map_err(|e| e.to_string())?;
    forget_revocation(&file.with_file_name(REVOCATIONS_FILE), id).map_err(|e| e.to_string())
}

/// The owner on the machine stops trusting `id` (`agora node untrust`): removed with every device it endorsed, and remembered as
/// revoked now so the hub cannot bring it back with an endorsement it carried before. `Ok(false)`: it was not trusted (it is remembered
/// as revoked all the same).
pub fn untrust(file: &Path, id: &str, now_ms: u64) -> Result<bool, String> {
    if !valid_device_id(id) {
        return Err("the device id may contain only letters, digits, '.', '_' and '-'".into());
    }
    let _lock = FileLock::take(file).map_err(|e| e.to_string())?;
    let (entries, _) = read_entries(file);
    let was = entries.iter().any(|e| e.id == id);
    if was {
        write_entries(file, &without(&entries, id)).map_err(|e| e.to_string())?;
    }
    remember_revocation(&file.with_file_name(REVOCATIONS_FILE), id, now_ms, now_ms)
        .map_err(|e| e.to_string())?;
    audit(
        &file.with_file_name(AUDIT_FILE),
        now_ms,
        "untrust",
        id,
        "owner",
        "ok",
    );
    Ok(was)
}

fn read_revocations(path: &Path) -> Vec<(String, u64)> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let (id, at) = l.trim().split_once(' ')?;
            valid_device_id(id).then_some(())?;
            Some((id.to_string(), at.trim().parse().ok()?))
        })
        .collect()
}

fn write_revocations(path: &Path, list: &[(String, u64)]) -> std::io::Result<()> {
    let text: String = list.iter().map(|(id, at)| format!("{id} {at}\n")).collect();
    agora_platform::write_private(path, text.as_bytes())
}

/// Keeps the latest revocation time of `id`, and keeps the list bounded: records older than 400 days go (every endorsement signed
/// before them has expired, so forgetting them cannot bring a device back), then, beyond [`MAX_REVOCATIONS`], the oldest of those older
/// than any endorsement can last.
fn remember_revocation(path: &Path, id: &str, at_ms: u64, now_ms: u64) -> std::io::Result<()> {
    let mut list = read_revocations(path);
    let latest = list
        .iter()
        .filter(|(d, _)| d == id)
        .map(|(_, a)| *a)
        .max()
        .unwrap_or(0)
        .max(at_ms);
    list.retain(|(d, a)| d != id && a.saturating_add(REVOCATION_KEPT_MS) >= now_ms);
    list.push((id.to_string(), latest));
    if list.len() > MAX_REVOCATIONS {
        list.sort_by_key(|(_, a)| *a);
        let old = now_ms.saturating_sub(MAX_ENDORSEMENT_MS + SKEW_MS);
        let over = list.len() - MAX_REVOCATIONS;
        let mut dropped = 0;
        list.retain(|(_, a)| {
            if dropped < over && *a < old {
                dropped += 1;
                false
            } else {
                true
            }
        });
    }
    write_revocations(path, &list)
}

fn forget_revocation(path: &Path, id: &str) -> std::io::Result<()> {
    let mut list = read_revocations(path);
    let before = list.len();
    list.retain(|(d, _)| d != id);
    if list.len() == before {
        return Ok(());
    }
    write_revocations(path, &list)
}

/// One line of the machine's own record (best effort: a full disk must not stop a revocation). Every value is passed through
/// [`logged`], so nothing a hub sends can add a line.
fn audit(path: &Path, now_ms: u64, action: &str, device: &str, by: &str, outcome: &str) {
    use std::io::Write;
    let line = format!(
        "{now_ms} {action} {} by={} {}\n",
        logged(device),
        logged(by),
        logged(outcome)
    );
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    if let Ok(mut f) = options.open(path) {
        let _ = f.write_all(line.as_bytes());
    }
    eprint!(
        "devices: {}",
        line.split_once(' ').map_or("", |(_, rest)| rest)
    );
}

fn field(out: &mut Vec<u8>, x: &[u8]) {
    out.extend_from_slice(x.len().to_string().as_bytes());
    out.push(b':');
    out.extend_from_slice(x);
}

/// The exact bytes an endorser signs (docs/specs/protocol.md "Devices"; vectors in `schema/fixtures/signing/device-trust-v1.json`).
pub fn endorse_message(e: &DeviceEndorsement) -> Vec<u8> {
    let mut m = b"agora-device-endorse-v1\0".to_vec();
    field(&mut m, e.device_id.as_bytes());
    field(&mut m, hex(&e.public_key).as_bytes());
    field(&mut m, e.name.as_bytes());
    field(&mut m, e.nodes.len().to_string().as_bytes());
    for n in &e.nodes {
        field(&mut m, n.as_bytes());
    }
    field(&mut m, e.issued_at_ms.to_string().as_bytes());
    field(&mut m, e.expires_at_ms.to_string().as_bytes());
    field(&mut m, e.endorser_device_id.as_bytes());
    m
}

/// The exact bytes a revoker signs.
pub fn revoke_message(r: &DeviceRevocation) -> Vec<u8> {
    let mut m = b"agora-device-revoke-v1\0".to_vec();
    field(&mut m, r.device_id.as_bytes());
    field(&mut m, r.issued_at_ms.to_string().as_bytes());
    field(&mut m, r.revoker_device_id.as_bytes());
    m
}

/// The exact bytes a device signs to register itself (its proof of possession).
pub fn register_message(
    device_id: &str,
    name: &str,
    public_key: &[u8],
    signed_at_ms: u64,
) -> Vec<u8> {
    let mut m = b"agora-device-register-v1\0".to_vec();
    field(&mut m, device_id.as_bytes());
    field(&mut m, name.as_bytes());
    field(&mut m, hex(public_key).as_bytes());
    field(&mut m, signed_at_ms.to_string().as_bytes());
    m
}

/// Did the holder of the device's key register it as listed (id, name, key)? A device the hub lists without that proof is not
/// offered: the hub cannot make one up for a key nobody holds.
pub fn registration_verifies(d: &DeviceInfo) -> bool {
    valid_key(&d.public_key)
        && verifies(
            &d.public_key,
            &register_message(&d.device_id, &d.name, &d.public_key, d.signed_at_ms),
            &d.registration_signature,
        )
}

/// What people compare: the first 12 bytes (96 bits) of the SHA-256 of the key, lowercase hex in groups of 4 digits joined by `-`.
/// Always computed here from the key bytes, never taken from the hub.
pub fn fingerprint(public_key: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, public_key);
    digest
        .as_ref()
        .get(..12)
        .unwrap_or_default()
        .chunks(2)
        .map(hex)
        .collect::<Vec<_>>()
        .join("-")
}

/// A fingerprint as someone typed it: hex digits only, lowercased (`F947 3CB5…`, `f947:3cb5…` and `f947-3cb5-…` are the same). Only the
/// whole fingerprint (24 digits) matches a device.
pub fn normalize_fingerprint(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_hexdigit)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn verifies(key: &[u8], message: &[u8], signature: &[u8]) -> bool {
    ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_FIXED, key)
        .verify(message, signature)
        .is_ok()
}

/// What an endorsement did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endorsed {
    /// The device is trusted here from now on (until the returned expiry).
    Trusted(u64),
    /// It was trusted already; its expiry is now this (or it has none: confirmed on the machine).
    Already(Option<u64>),
}

/// The node's side of ADR-0061: verifies what the hub carries and keeps the trusted-devices file.
pub struct DeviceTrust {
    node_id: String,
    file: PathBuf,
    /// One change at a time from this process (other processes wait on the lock file).
    lock: Mutex<()>,
    /// The node policy's `device.<id>.projects` lines (ADR-0049).
    policy: Mutex<std::collections::BTreeMap<String, std::collections::BTreeSet<String>>>,
}

impl DeviceTrust {
    /// `file` is the trusted-devices file; the revocations and the record live beside it.
    pub fn new(node_id: &str, file: &Path) -> Self {
        Self {
            node_id: node_id.to_string(),
            file: file.to_path_buf(),
            lock: Mutex::new(()),
            policy: Mutex::new(std::collections::BTreeMap::new()),
        }
    }

    /// The node policy's per-device project limits (ADR-0049), which endorsements and revocations respect.
    pub fn set_policy_limits(
        &self,
        limits: std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
    ) {
        *self.policy.lock().unwrap_or_else(|p| p.into_inner()) = limits;
    }

    /// What `id` may act in here: the policy file's line for it, else what it inherited from a limited endorser; `None`: anything.
    fn limits_of(&self, entries: &[Entry], id: &str) -> Option<std::collections::BTreeSet<String>> {
        let policy = self
            .policy
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        effective_limit(entries, &policy, id)
    }

    pub fn file(&self) -> &Path {
        &self.file
    }

    fn revocations(&self) -> PathBuf {
        self.file.with_file_name(REVOCATIONS_FILE)
    }

    fn record(&self, now_ms: u64, action: &str, device: &str, by: &str, outcome: &str) {
        audit(
            &self.file.with_file_name(AUDIT_FILE),
            now_ms,
            action,
            device,
            by,
            outcome,
        );
    }

    /// The devices trusted now, sorted: what `NodeTrustedDevices` reports.
    pub fn trusted_ids(&self, now_ms: u64) -> Vec<String> {
        let (entries, _) = read_entries(&self.file);
        let mut ids: Vec<String> = entries
            .iter()
            .filter(|e| alive(&entries, e, now_ms))
            .map(|e| e.id.clone())
            .collect();
        ids.sort();
        ids
    }

    /// A trusted device that verifies `message`, or the error that says why not.
    fn signer(
        &self,
        entries: &[Entry],
        signer: &str,
        subject: &str,
        message: &[u8],
        signature: &[u8],
        now_ms: u64,
    ) -> Result<Entry, Error> {
        let Some(e) = entries
            .iter()
            .find(|e| e.id == signer && alive(entries, e, now_ms))
        else {
            return Err(Error::new(ErrorCode::DeviceUntrusted)
                .with("device_id", signer)
                .with("node_id", self.node_id.as_str()));
        };
        if !verifies(&e.key, message, signature) {
            return Err(Error::new(ErrorCode::DeviceSignatureInvalid).with("device_id", subject));
        }
        Ok(e.clone())
    }

    fn io(_: std::io::Error) -> Error {
        Error::new(ErrorCode::Internal).with("incident_id", "trusted-devices")
    }

    /// Applies an endorsement the hub carried, after checking everything ADR-0061 asks (docs/specs/protocol.md "Devices").
    pub fn endorse(&self, e: &DeviceEndorsement, now_ms: u64) -> Result<Endorsed, Error> {
        let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let result = self.endorse_locked(e, now_ms);
        let outcome = match &result {
            Ok(Endorsed::Trusted(_)) => "trusted".to_string(),
            Ok(Endorsed::Already(_)) => "already".to_string(),
            Err(err) => err.code.as_str().to_string(),
        };
        self.record(
            now_ms,
            "endorse",
            &e.device_id,
            &e.endorser_device_id,
            &outcome,
        );
        result
    }

    fn endorse_locked(&self, e: &DeviceEndorsement, now_ms: u64) -> Result<Endorsed, Error> {
        let invalid = |f: &str| Error::new(ErrorCode::ProtocolInvalidMessage).with("field", f);
        if !valid_device_id(&e.device_id) {
            return Err(invalid("device_id"));
        }
        if !valid_device_id(&e.endorser_device_id) || e.endorser_device_id == e.device_id {
            return Err(invalid("endorser_device_id"));
        }
        let _lock = FileLock::take(&self.file).map_err(Self::io)?;
        let (entries, _) = read_entries(&self.file);
        let endorser = self.signer(
            &entries,
            &e.endorser_device_id,
            &e.device_id,
            &endorse_message(e),
            &e.signature,
            now_ms,
        )?;
        // Two endorsements deep at most: a device endorsed by an endorsed device cannot endorse.
        if depth(&entries, &endorser).is_none_or(|d| d >= MAX_DEPTH) {
            return Err(Error::new(ErrorCode::DeviceUntrusted)
                .with("device_id", e.endorser_device_id.as_str())
                .with("node_id", self.node_id.as_str()));
        }
        if !valid_key(&e.public_key) {
            return Err(invalid("public_key"));
        }
        if !e.nodes.is_empty() && !e.nodes.contains(&self.node_id) {
            return Err(Error::new(ErrorCode::DeviceNotInScope)
                .with("device_id", e.device_id.as_str())
                .with("node_id", self.node_id.as_str()));
        }
        // Trust starts no later than now, whatever the signer's clock said: a revocation signed now always wins over it.
        let began = e.issued_at_ms.min(now_ms);
        if now_ms >= e.expires_at_ms
            || e.expires_at_ms <= e.issued_at_ms
            || e.issued_at_ms > now_ms.saturating_add(SKEW_MS)
            || e.expires_at_ms - e.issued_at_ms > MAX_ENDORSEMENT_MS
        {
            return Err(Error::new(ErrorCode::DeviceEndorsementExpired)
                .with("device_id", e.device_id.as_str()));
        }
        if read_revocations(&self.revocations())
            .iter()
            .any(|(d, at)| *d == e.device_id && *at >= began)
        {
            return Err(
                Error::new(ErrorCode::DeviceRevoked).with("device_id", e.device_id.as_str())
            );
        }
        // Never longer than the endorser's own trust here.
        let expires = match effective_expiry(&entries, &endorser) {
            Some(Some(x)) => x.min(e.expires_at_ms),
            _ => e.expires_at_ms,
        };
        match entries.iter().find(|x| x.id == e.device_id) {
            Some(x) if x.key != e.public_key => {
                Err(Error::new(ErrorCode::DeviceKeyMismatch)
                    .with("device_id", e.device_id.as_str()))
            }
            Some(x) if x.expires_ms.is_none() => Ok(Endorsed::Already(None)),
            Some(x)
                if alive(&entries, x, now_ms) && x.expires_ms.is_some_and(|old| old >= expires) =>
            {
                Ok(Endorsed::Already(x.expires_ms))
            }
            existing => {
                let endorser_limits = self.limits_of(&entries, &e.endorser_device_id);
                // A project name the trusted-devices file cannot hold (`projects=a,b`): refused, never silently dropped.
                if endorser_limits.as_ref().is_some_and(|p| {
                    p.iter().any(|n| {
                        n.is_empty() || n.contains(',') || n.chars().any(char::is_whitespace)
                    })
                }) {
                    return Err(Error::new(ErrorCode::NodePolicyDenied)
                        .with("node_id", self.node_id.as_str())
                        .with("rule", "device.projects"));
                }
                // Only a still-valid endorsed entry is protected from being taken over; an expired one (or one whose endorser is gone)
                // is endorsed afresh like a new device.
                let existing = existing.filter(|x| alive(&entries, x, now_ms));
                let entry = match existing {
                    // A renewal keeps who endorsed it and what it may do: only its own endorser may renew it, and only while that
                    // endorser's limits still cover the device's (a limited device cannot re-parent a wider one to itself).
                    Some(x) => {
                        let covered = match (&endorser_limits, &x.projects) {
                            (None, _) => true,
                            (Some(_), None) => false,
                            (Some(mine), Some(theirs)) => theirs.is_subset(mine),
                        };
                        if x.via.as_deref() != Some(e.endorser_device_id.as_str()) || !covered {
                            return Err(Error::new(ErrorCode::AuthForbidden)
                                .with("action", "device.endorse"));
                        }
                        Entry {
                            expires_ms: Some(expires),
                            ..x.clone()
                        }
                    }
                    None => Entry {
                        id: e.device_id.clone(),
                        key: e.public_key.clone(),
                        expires_ms: Some(expires),
                        via: Some(e.endorser_device_id.clone()),
                        since: Some(began),
                        // ADR-0049: a limited endorser vouches only within its own projects; the device it endorses carries them.
                        projects: endorser_limits,
                    },
                };
                let mut all: Vec<Entry> = entries
                    .iter()
                    .filter(|x| x.id != e.device_id)
                    .cloned()
                    .collect();
                all.push(entry);
                write_entries(&self.file, &all).map_err(Self::io)?;
                Ok(match existing {
                    Some(_) => Endorsed::Already(Some(expires)),
                    None => Endorsed::Trusted(expires),
                })
            }
        }
    }

    /// Applies a revocation the hub carried. `Ok(true)`: the device was trusted here (from before the revocation was signed) and is not
    /// any more, nor is any device it endorsed.
    pub fn revoke(&self, r: &DeviceRevocation, now_ms: u64) -> Result<bool, Error> {
        let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let result = self.revoke_locked(r, now_ms);
        let outcome = match &result {
            Ok(true) => "revoked".to_string(),
            Ok(false) => "remembered".to_string(),
            Err(err) => err.code.as_str().to_string(),
        };
        self.record(
            now_ms,
            "revoke",
            &r.device_id,
            &r.revoker_device_id,
            &outcome,
        );
        result
    }

    fn revoke_locked(&self, r: &DeviceRevocation, now_ms: u64) -> Result<bool, Error> {
        let invalid = |f: &str| Error::new(ErrorCode::ProtocolInvalidMessage).with("field", f);
        if !valid_device_id(&r.device_id) {
            return Err(invalid("device_id"));
        }
        if !valid_device_id(&r.revoker_device_id) {
            return Err(invalid("revoker_device_id"));
        }
        // A revocation dated ahead of this clock would outlive every bound on what is remembered.
        if r.issued_at_ms > now_ms.saturating_add(SKEW_MS) {
            return Err(invalid("issued_at_ms"));
        }
        let _lock = FileLock::take(&self.file).map_err(Self::io)?;
        let (entries, _) = read_entries(&self.file);
        self.signer(
            &entries,
            &r.revoker_device_id,
            &r.device_id,
            &revoke_message(r),
            &r.signature,
            now_ms,
        )?;
        // ADR-0049: a limited device removes only itself or a device limited to projects it has too (never the owner's).
        if r.device_id != r.revoker_device_id {
            if let Some(mine) = self.limits_of(&entries, &r.revoker_device_id) {
                let within = entries.iter().any(|e| e.id == r.device_id)
                    && self
                        .limits_of(&entries, &r.device_id)
                        .is_some_and(|theirs| theirs.is_subset(&mine));
                if !within {
                    return Err(
                        Error::new(ErrorCode::AuthForbidden).with("action", "device.revoke")
                    );
                }
            }
        }
        remember_revocation(&self.revocations(), &r.device_id, r.issued_at_ms, now_ms)
            .map_err(Self::io)?;
        // Only trust that began at or before the revocation was signed: one the hub sends again later cannot undo a newer decision.
        let applies = entries
            .iter()
            .any(|e| e.id == r.device_id && e.since.unwrap_or(0) <= r.issued_at_ms);
        if applies {
            write_entries(&self.file, &without(&entries, &r.device_id)).map_err(Self::io)?;
        }
        Ok(applies)
    }

    /// A device endorsed here was used (its signature verified): its trust runs 90 days from now, never past its endorser's own.
    /// Written at most once a day per device; a device confirmed on the machine has no end and is left alone.
    pub fn renew(&self, device_id: &str, now_ms: u64) {
        let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let Ok(_lock) = FileLock::take(&self.file) else {
            return;
        };
        let (entries, _) = read_entries(&self.file);
        let Some(e) = entries
            .iter()
            .find(|e| e.id == device_id && alive(&entries, e, now_ms))
        else {
            return;
        };
        let Some(old) = e.expires_ms else { return };
        // Not renewed when its endorser's limits today no longer cover what it was given: it then runs out, and the owner decides.
        if let Some(v) = &e.via {
            let policy = self
                .policy
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone();
            let covered = match (effective_limit(&entries, &policy, v), &e.projects) {
                (None, _) => true,
                (Some(_), None) => false,
                (Some(now), Some(given)) => given.is_subset(&now),
            };
            if !covered {
                return;
            }
        }
        let mut new = now_ms.saturating_add(MAX_ENDORSEMENT_MS);
        if let Some(endorser) = e
            .via
            .as_ref()
            .and_then(|v| entries.iter().find(|x| x.id == *v))
        {
            if let Some(Some(limit)) = effective_expiry(&entries, endorser) {
                new = new.min(limit);
            }
        }
        if new > old.saturating_add(RENEW_STEP_MS) {
            let all: Vec<Entry> = entries
                .iter()
                .map(|x| {
                    let mut x = x.clone();
                    if x.id == device_id {
                        x.expires_ms = Some(new);
                    }
                    x
                })
                .collect();
            let _ = write_entries(&self.file, &all);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};

    const VECTORS: &str = include_str!("../../../schema/fixtures/signing/device-trust-v1.json");
    const DAY: u64 = 24 * 60 * 60 * 1000;

    fn vectors() -> serde_json::Value {
        serde_json::from_str(VECTORS).unwrap()
    }

    fn endorsement_of(c: &serde_json::Value) -> DeviceEndorsement {
        DeviceEndorsement {
            device_id: c["device_id"].as_str().unwrap().into(),
            public_key: from_hex(c["public_key"].as_str().unwrap()).unwrap(),
            name: c["name"].as_str().unwrap().into(),
            nodes: c["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_str().unwrap().to_string())
                .collect(),
            issued_at_ms: c["issued_at_ms"].as_u64().unwrap(),
            expires_at_ms: c["expires_at_ms"].as_u64().unwrap(),
            endorser_device_id: c["endorser_device_id"].as_str().unwrap().into(),
            signature: from_hex(c["signature"].as_str().unwrap()).unwrap(),
        }
    }

    #[test]
    fn the_signed_bytes_and_fingerprints_match_the_shared_vectors() {
        let v = vectors();
        let endorser = from_hex(v["endorser_public_key"].as_str().unwrap()).unwrap();
        for c in v["endorsements"].as_array().unwrap() {
            let e = endorsement_of(c);
            let m = endorse_message(&e);
            assert_eq!(hex(&m), c["hex"].as_str().unwrap(), "{}", c["case"]);
            assert!(verifies(&endorser, &m, &e.signature), "{}", c["case"]);
        }
        for c in v["revocations"].as_array().unwrap() {
            let r = DeviceRevocation {
                device_id: c["device_id"].as_str().unwrap().into(),
                issued_at_ms: c["issued_at_ms"].as_u64().unwrap(),
                revoker_device_id: c["revoker_device_id"].as_str().unwrap().into(),
                signature: from_hex(c["signature"].as_str().unwrap()).unwrap(),
            };
            let m = revoke_message(&r);
            assert_eq!(hex(&m), c["hex"].as_str().unwrap(), "{}", c["case"]);
            assert!(verifies(&endorser, &m, &r.signature));
        }
        for f in v["fingerprints"].as_array().unwrap() {
            let k = from_hex(f["public_key"].as_str().unwrap()).unwrap();
            assert_eq!(fingerprint(&k), f["fingerprint"].as_str().unwrap());
        }
        assert_eq!(
            normalize_fingerprint("3F9A 07c2:11BE-d4e0"),
            "3f9a07c211bed4e0"
        );
    }

    struct Key {
        pair: EcdsaKeyPair,
    }

    impl Key {
        fn new() -> Self {
            let rng = SystemRandom::new();
            let pkcs8 =
                EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
            Key {
                pair: EcdsaKeyPair::from_pkcs8(
                    &ECDSA_P256_SHA256_FIXED_SIGNING,
                    pkcs8.as_ref(),
                    &rng,
                )
                .unwrap(),
            }
        }
        fn public(&self) -> Vec<u8> {
            self.pair.public_key().as_ref().to_vec()
        }
        fn sign(&self, m: &[u8]) -> Vec<u8> {
            self.pair
                .sign(&SystemRandom::new(), m)
                .unwrap()
                .as_ref()
                .to_vec()
        }
        fn endorse(
            &self,
            by: &str,
            device: &str,
            key: &[u8],
            nodes: &[&str],
            issued: u64,
            ttl: u64,
        ) -> DeviceEndorsement {
            let mut e = DeviceEndorsement {
                device_id: device.into(),
                public_key: key.to_vec(),
                name: "New browser".into(),
                nodes: nodes.iter().map(|n| (*n).to_string()).collect(),
                issued_at_ms: issued,
                expires_at_ms: issued + ttl,
                endorser_device_id: by.into(),
                signature: Vec::new(),
            };
            e.signature = self.sign(&endorse_message(&e));
            e
        }
        fn revoke(&self, by: &str, device: &str, issued: u64) -> DeviceRevocation {
            let mut r = DeviceRevocation {
                device_id: device.into(),
                issued_at_ms: issued,
                revoker_device_id: by.into(),
                signature: Vec::new(),
            };
            r.signature = self.sign(&revoke_message(&r));
            r
        }
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("agora-dt-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn an_endorsement_counts_only_from_a_trusted_device_for_this_machine_in_time_and_not_revoked() {
        let d = dir("endorse");
        let file = d.join(crate::approvals::TRUSTED_DEVICES_FILE);
        let (old, new, stranger) = (Key::new(), Key::new(), Key::new());
        confirm(&file, "web-old", &old.public()).unwrap();
        let t = DeviceTrust::new("laptop", &file);
        let now = crate::grants::now_ms();
        let code = |r: Result<Endorsed, Error>| r.unwrap_err().code;

        // The hub lists a device and makes up an endorsement for it: signed by no device this machine trusts.
        let forged = stranger.endorse("web-old", "web-new", &new.public(), &[], now, DAY);
        assert_eq!(
            code(t.endorse(&forged, now)),
            ErrorCode::DeviceSignatureInvalid
        );
        let unknown = stranger.endorse("web-x", "web-new", &new.public(), &[], now, DAY);
        assert_eq!(code(t.endorse(&unknown, now)), ErrorCode::DeviceUntrusted);
        // A real endorsement altered on the way (another key swapped in, a machine dropped).
        let mut swapped = old.endorse("web-old", "web-new", &new.public(), &[], now, DAY);
        swapped.public_key = stranger.public();
        assert_eq!(
            code(t.endorse(&swapped, now)),
            ErrorCode::DeviceSignatureInvalid
        );
        let mut widened = old.endorse("web-old", "web-new", &new.public(), &["vps"], now, DAY);
        widened.nodes.clear();
        assert_eq!(
            code(t.endorse(&widened, now)),
            ErrorCode::DeviceSignatureInvalid
        );
        // Signed, but for other machines, expired, too long, or from the future.
        let other = old.endorse("web-old", "web-new", &new.public(), &["vps"], now, DAY);
        assert_eq!(code(t.endorse(&other, now)), ErrorCode::DeviceNotInScope);
        let expired = old.endorse("web-old", "web-new", &new.public(), &[], now - 2 * DAY, DAY);
        assert_eq!(
            code(t.endorse(&expired, now)),
            ErrorCode::DeviceEndorsementExpired
        );
        let long = old.endorse(
            "web-old",
            "web-new",
            &new.public(),
            &[],
            now,
            MAX_ENDORSEMENT_MS + 1,
        );
        assert_eq!(
            code(t.endorse(&long, now)),
            ErrorCode::DeviceEndorsementExpired
        );
        let ahead = old.endorse("web-old", "web-new", &new.public(), &[], now + DAY, DAY);
        assert_eq!(
            code(t.endorse(&ahead, now)),
            ErrorCode::DeviceEndorsementExpired
        );
        assert_eq!(t.trusted_ids(now), ["web-old"], "nothing above added trust");

        // The real one, for this machine among others.
        let good = old.endorse(
            "web-old",
            "web-new",
            &new.public(),
            &["vps", "laptop"],
            now,
            30 * DAY,
        );
        assert_eq!(
            t.endorse(&good, now).unwrap(),
            Endorsed::Trusted(now + 30 * DAY)
        );
        assert_eq!(
            t.endorse(&good, now + 1).unwrap(),
            Endorsed::Already(Some(now + 30 * DAY))
        );
        assert_eq!(t.trusted_ids(now), ["web-new", "web-old"]);
        // It lasts until its expiry, then the machine stops trusting it.
        assert_eq!(t.trusted_ids(now + 30 * DAY), ["web-old"]);
        // A different key under the same id is refused.
        let clash = old.endorse("web-old", "web-new", &stranger.public(), &[], now, DAY);
        assert_eq!(code(t.endorse(&clash, now)), ErrorCode::DeviceKeyMismatch);

        let log = std::fs::read_to_string(d.join(AUDIT_FILE)).unwrap();
        assert!(log.contains("endorse web-new by=web-old trusted"));
        assert!(log.contains("device.signature_invalid"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn an_endorsed_device_lasts_no_longer_than_its_endorser_and_use_renews_it() {
        let d = dir("chain");
        let file = d.join(crate::approvals::TRUSTED_DEVICES_FILE);
        let (a, b, c) = (Key::new(), Key::new(), Key::new());
        confirm(&file, "a", &a.public()).unwrap();
        let t = DeviceTrust::new("laptop", &file);
        let now = crate::grants::now_ms();
        t.endorse(&a.endorse("a", "b", &b.public(), &[], now, 10 * DAY), now)
            .unwrap();
        // b (trusted for 10 days) endorses c for 60: c ends with b.
        assert_eq!(
            t.endorse(&b.endorse("b", "c", &c.public(), &[], now, 60 * DAY), now)
                .unwrap(),
            Endorsed::Trusted(now + 10 * DAY)
        );
        // Using b renews it to 90 days from the use.
        t.renew("b", now + 5 * DAY);
        let b_entry = read_entries(&file)
            .0
            .into_iter()
            .find(|e| e.id == "b")
            .unwrap();
        assert_eq!(b_entry.expires_ms, Some(now + 5 * DAY + MAX_ENDORSEMENT_MS));
        assert_eq!(b_entry.via.as_deref(), Some("a"));
        // A device confirmed on the machine has no end to renew.
        t.renew("a", now);
        assert_eq!(read_entries(&file).0[0].expires_ms, None);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_revocation_narrows_at_once_and_an_older_endorsement_cannot_bring_the_device_back() {
        let d = dir("revoke");
        let file = d.join(crate::approvals::TRUSTED_DEVICES_FILE);
        let (old, new, stranger) = (Key::new(), Key::new(), Key::new());
        confirm(&file, "web-old", &old.public()).unwrap();
        let t = DeviceTrust::new("laptop", &file);
        let now = crate::grants::now_ms();
        let e = old.endorse("web-old", "web-new", &new.public(), &[], now, DAY);
        t.endorse(&e, now).unwrap();
        // A revocation the hub made up does nothing.
        assert_eq!(
            t.revoke(&stranger.revoke("web-old", "web-new", now + 1), now + 1)
                .unwrap_err()
                .code,
            ErrorCode::DeviceSignatureInvalid
        );
        assert!(t.trusted_ids(now + 1).contains(&"web-new".to_string()));
        assert!(t
            .revoke(&old.revoke("web-old", "web-new", now + 2), now + 2)
            .unwrap());
        assert_eq!(t.trusted_ids(now + 2), ["web-old"]);
        // The hub sends the old endorsement again: refused.
        assert_eq!(
            t.endorse(&e, now + 3).unwrap_err().code,
            ErrorCode::DeviceRevoked
        );
        // A new endorsement signed after the revocation is a new decision.
        let again = old.endorse("web-old", "web-new", &new.public(), &[], now + 4, DAY);
        assert!(matches!(
            t.endorse(&again, now + 4),
            Ok(Endorsed::Trusted(_))
        ));
        // The owner untrusts it on the machine: same effect, without the hub.
        assert!(untrust(&file, "web-new", now + 5).unwrap());
        assert_eq!(
            t.endorse(&again, now + 6).unwrap_err().code,
            ErrorCode::DeviceRevoked
        );
        // A revoked revoker can sign nothing more.
        assert!(t
            .revoke(&old.revoke("web-old", "web-old", now + 7), now + 7)
            .unwrap());
        assert_eq!(t.trusted_ids(now + 7), Vec::<String>::new());
        assert_eq!(
            t.revoke(&old.revoke("web-old", "web-new", now + 8), now + 8)
                .unwrap_err()
                .code,
            ErrorCode::DeviceUntrusted
        );
        // Trusting it again on the machine clears what was remembered.
        confirm(&file, "web-old", &old.public()).unwrap();
        assert_eq!(t.trusted_ids(now + 9), ["web-old"]);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_file_keeps_its_format_and_unknown_fields_are_never_trusted() {
        let d = dir("format");
        let file = d.join(crate::approvals::TRUSTED_DEVICES_FILE);
        let k = Key::new();
        let h = hex(&k.public());
        std::fs::write(
            &file,
            format!(
                "# mine\nphone {h}\nweb-1 {h} expires=5 via=phone\nweb-2 {h} expire=5\nweb-3 {h} expires=x\nweb-4 {h} expires=1 expires=2\n"
            ),
        )
        .unwrap();
        let (entries, skipped) = read_entries(&file);
        assert_eq!(skipped, 3, "a typo in a field never widens trust");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].expires_ms, Some(5));
        // Changing one device keeps every other line (and the comment) as it was, owner-only.
        confirm(&file, "web-9", &k.public()).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.starts_with("# mine\nphone "));
        assert!(text
            .lines()
            .last()
            .unwrap()
            .starts_with(&format!("web-9 {h} since=")));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_revocation_sent_again_never_undoes_a_newer_trust() {
        // Review finding 2: the hub resends every revocation on each reconnect.
        let d = dir("replay");
        let file = d.join(crate::approvals::TRUSTED_DEVICES_FILE);
        let (old, new) = (Key::new(), Key::new());
        confirm(&file, "web-old", &old.public()).unwrap();
        let t = DeviceTrust::new("laptop", &file);
        let now = crate::grants::now_ms();
        t.endorse(
            &old.endorse("web-old", "web-new", &new.public(), &[], now, DAY),
            now,
        )
        .unwrap();
        let revocation = old.revoke("web-old", "web-new", now + 2);
        assert!(t.revoke(&revocation, now + 2).unwrap());
        // A newer endorsement (signed after the revocation) trusts it again...
        let again = old.endorse("web-old", "web-new", &new.public(), &[], now + 4, DAY);
        assert!(matches!(
            t.endorse(&again, now + 4),
            Ok(Endorsed::Trusted(_))
        ));
        // ...and the old revocation sent again keeps it.
        assert!(!t.revoke(&revocation, now + 5).unwrap());
        assert!(t.trusted_ids(now + 5).contains(&"web-new".to_string()));
        // The same after the owner confirms it on the machine.
        assert!(t
            .revoke(&old.revoke("web-old", "web-new", now + 6), now + 6)
            .unwrap());
        std::thread::sleep(std::time::Duration::from_millis(5));
        confirm(&file, "web-new", &new.public()).unwrap();
        assert!(!t
            .revoke(
                &old.revoke("web-old", "web-new", now + 6),
                crate::grants::now_ms()
            )
            .unwrap());
        assert!(t
            .trusted_ids(crate::grants::now_ms())
            .contains(&"web-new".to_string()));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn nothing_a_hub_sends_can_add_a_line_to_the_log() {
        // Review finding 3.
        let d = dir("log");
        let file = d.join(crate::approvals::TRUSTED_DEVICES_FILE);
        let old = Key::new();
        confirm(&file, "web-old", &old.public()).unwrap();
        let t = DeviceTrust::new("laptop", &file);
        let now = crate::grants::now_ms();
        let evil = "x\n2 untrust web-old by=owner ok";
        let e = old.endorse("web-old", evil, &old.public(), &[], now, DAY);
        assert_eq!(
            t.endorse(&e, now).unwrap_err().code,
            ErrorCode::ProtocolInvalidMessage
        );
        let mut e = old.endorse(evil, "web-new", &old.public(), &[], now, DAY);
        e.device_id = "web-new".into();
        assert!(t.endorse(&e, now).is_err());
        let r = old.revoke(evil, "web-new", now);
        assert_eq!(
            t.revoke(&r, now).unwrap_err().code,
            ErrorCode::ProtocolInvalidMessage
        );
        let r = old.revoke("web-old", evil, now);
        assert_eq!(
            t.revoke(&r, now).unwrap_err().code,
            ErrorCode::ProtocolInvalidMessage
        );
        let log = std::fs::read_to_string(d.join(AUDIT_FILE)).unwrap();
        assert_eq!(log.lines().count(), 4, "{log}");
        assert!(log.lines().all(|l| !l.starts_with("2 ")), "{log}");
        assert!(
            log.lines().all(|l| l.split(' ').count() == 5),
            "one record per line: {log}"
        );
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn revoking_a_device_removes_what_it_endorsed_and_chains_stay_short() {
        // Review finding 4.
        let d = dir("cascade");
        let file = d.join(crate::approvals::TRUSTED_DEVICES_FILE);
        let (a, b, c, x) = (Key::new(), Key::new(), Key::new(), Key::new());
        confirm(&file, "a", &a.public()).unwrap();
        let t = DeviceTrust::new("laptop", &file);
        let now = crate::grants::now_ms();
        t.endorse(&a.endorse("a", "b", &b.public(), &[], now, 10 * DAY), now)
            .unwrap();
        t.endorse(&b.endorse("b", "c", &c.public(), &[], now, 60 * DAY), now)
            .unwrap();
        // c was endorsed by an endorsed device: it cannot endorse in turn.
        assert_eq!(
            t.endorse(&c.endorse("c", "x", &x.public(), &[], now, DAY), now)
                .unwrap_err()
                .code,
            ErrorCode::DeviceUntrusted
        );
        // Using c does not carry it past b's own trust.
        t.renew("c", now + DAY + 1);
        let c_entry = read_entries(&file)
            .0
            .into_iter()
            .find(|e| e.id == "c")
            .unwrap();
        assert!(c_entry.expires_ms.unwrap() <= now + 10 * DAY, "{c_entry:?}");
        // b stolen: revoking it removes c too.
        assert!(t.revoke(&a.revoke("a", "b", now + 3), now + 3).unwrap());
        assert_eq!(t.trusted_ids(now + 3), ["a"]);
        // The owner untrusting a on the machine removes all it endorsed.
        t.endorse(
            &a.endorse("a", "x", &x.public(), &[], now + 4, DAY),
            now + 4,
        )
        .unwrap();
        assert!(untrust(&file, "a", now + 5).unwrap());
        assert!(t.trusted_ids(now + 5).is_empty());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn remembered_revocations_are_bounded_and_forgotten_only_when_harmless() {
        // Review finding 5.
        let d = dir("bounded");
        let path = d.join(REVOCATIONS_FILE);
        let now = crate::grants::now_ms();
        remember_revocation(&path, "ancient", now - REVOCATION_KEPT_MS - 1, now).unwrap();
        for i in 0..MAX_REVOCATIONS + 10 {
            remember_revocation(&path, &format!("old-{i}"), now - 200 * DAY, now).unwrap();
        }
        remember_revocation(&path, "recent", now - DAY, now).unwrap();
        let list = read_revocations(&path);
        assert!(list.len() <= MAX_REVOCATIONS, "{}", list.len());
        assert!(!list.iter().any(|(id, _)| id == "ancient"));
        assert!(
            list.iter().any(|(id, _)| id == "recent"),
            "a recent one is never dropped"
        );
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_change_waits_for_another_process_holding_the_lock() {
        let d = dir("lock");
        let file = d.join(crate::approvals::TRUSTED_DEVICES_FILE);
        let k = Key::new();
        let held = FileLock::take(&file).unwrap();
        let (f2, key) = (file.clone(), k.public());
        let other = std::thread::spawn(move || confirm(&f2, "web-1", &key));
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(read_entries(&file).0.is_empty(), "it waited");
        drop(held);
        other.join().unwrap().unwrap();
        assert_eq!(read_entries(&file).0.len(), 1);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn names_from_the_hub_are_shown_without_anything_that_moves_or_hides_text() {
        let evil = "Phone\u{1b}[2K\r\u{1b}[1A  fake · 0000-0000\u{202e}gnp\u{200b}x";
        let s = shown(evil, 40);
        assert!(
            !s.chars()
                .any(|c| c.is_control() || agora_core::text::is_hidden(c)),
            "{s:?}"
        );
        assert_eq!(shown(&"a".repeat(50), 40).chars().count(), 41);
        assert_eq!(shown("Firefox · Linux", 40), "Firefox · Linux");
    }

    #[test]
    fn a_registration_counts_only_with_the_devices_own_signature() {
        let v = vectors();
        for c in v["registrations"].as_array().unwrap() {
            let mut d = DeviceInfo {
                device_id: c["device_id"].as_str().unwrap().into(),
                name: c["name"].as_str().unwrap().into(),
                public_key: from_hex(c["public_key"].as_str().unwrap()).unwrap(),
                signed_at_ms: c["signed_at_ms"].as_u64().unwrap(),
                registration_signature: from_hex(c["signature"].as_str().unwrap()).unwrap(),
                ..Default::default()
            };
            assert_eq!(
                hex(&register_message(
                    &d.device_id,
                    &d.name,
                    &d.public_key,
                    d.signed_at_ms
                )),
                c["hex"].as_str().unwrap()
            );
            assert!(registration_verifies(&d));
            d.name.push('!');
            assert!(!registration_verifies(&d), "a hub cannot rename it");
            d.name.pop();
            d.public_key = Key::new().public();
            assert!(!registration_verifies(&d), "nor put another key under it");
        }
    }
}

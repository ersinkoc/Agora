//! ADR-0087 rung 2: the signed list of agent packages. Data only: nothing here reads the network or the disk, installs anything
//! or runs anything. The list rides in the ADR-0044 envelope and is opened by the same code as the adapter index
//! (`registry::open_signed`: signature under the pinned root key, expiry, rollback check). The payload then holds publishers (some
//! revoked) and agent-package entries, each of which is checked here, strictly: a list with one bad entry is refused whole, because
//! a signature says who wrote an entry, not that it is safe to act on (a publisher can be hacked, or can make a mistake).
//! See docs/specs/agent-packages.md for the format and the codes.

use std::collections::BTreeMap;

use regex::{Regex, RegexBuilder};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::package::valid_name;
use crate::registry::{open_signed, Publisher, RegistryError};
use crate::registry_convert::{clean_text, exact_version, forbidden_executable, npm_name};

/// Largest signed list (the envelope text) this reads, and most entries it accepts.
pub const MAX_LIST_BYTES: usize = 1 << 20;
pub const MAX_ENTRIES: usize = 256;
/// Longest `verify.regex`, in bytes, and the compiled-size limit handed to the engine. The engine matches in linear time.
pub const MAX_REGEX_LEN: usize = 200;
const REGEX_SIZE_LIMIT: usize = 100_000;
/// `verify` output beyond this many bytes is never matched against.
pub const MAX_VERIFY_OUTPUT: usize = 4096;

/// Why one entry is not acceptable. `code` is stable; the window's words come from `locales/` when a rung shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    NotObject,
    UnknownField,
    MissingField,
    BadName,
    BadText,
    BadUrl,
    BadVersion,
    BadPolicy,
    BadInstaller,
    BadHash,
    BadToken,
    BadRegex,
    TooMany,
    BadPlatform,
    BadCommand,
    ForbiddenExecutable,
}

impl Reason {
    pub fn code(self) -> &'static str {
        match self {
            Reason::NotObject => "agent_list.not_object",
            Reason::UnknownField => "agent_list.unknown_field",
            Reason::MissingField => "agent_list.missing_field",
            Reason::BadName => "agent_list.bad_name",
            Reason::BadText => "agent_list.bad_text",
            Reason::BadUrl => "agent_list.bad_url",
            Reason::BadVersion => "agent_list.bad_version",
            Reason::BadPolicy => "agent_list.bad_policy",
            Reason::BadInstaller => "agent_list.bad_installer",
            Reason::BadHash => "agent_list.bad_hash",
            Reason::BadToken => "agent_list.bad_token",
            Reason::BadRegex => "agent_list.bad_regex",
            Reason::TooMany => "agent_list.too_many",
            Reason::BadPlatform => "agent_list.bad_platform",
            Reason::BadCommand => "agent_list.bad_command",
            Reason::ForbiddenExecutable => "agent_list.forbidden_executable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListError {
    /// The envelope, signature, expiry or rollback check of ADR-0044 failed (`agent.install.index_invalid`).
    Signed(RegistryError),
    /// The list is larger than `MAX_LIST_BYTES` or holds more than `MAX_ENTRIES` entries.
    TooLarge,
    /// Entry number `index` (named `name` when it has a readable name) is invalid; the whole list is refused.
    Entry {
        index: usize,
        name: String,
        reason: Reason,
    },
    /// Two entries have the same name and installer kind.
    Duplicate {
        name: String,
        installer: &'static str,
    },
    /// An entry names a publisher the list does not declare.
    UnknownPublisher(String),
    /// A revoked publisher's entry is not offered (`agent.install.revoked`).
    Revoked(String),
    /// No entry of that name.
    NotOffered(String),
}

/// How the version of an entry is decided. `version` is always the one exact version the entry installs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionPolicy {
    /// Exactly `version`.
    Pin,
    /// The newest the signed list names: `version` is that newest, and a new list moves it.
    Track,
    /// `version` lies in `min..=max` (compared on major.minor.patch); the node refuses outside it.
    Range { min: String, max: String },
}

/// One file to download for one platform (`<os>-<arch>[-musl]`, as the ADR-0092 converter names them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Download {
    pub url: String,
    /// Lower-case hex SHA-256 of the downloaded file.
    pub sha256: String,
    /// Relative path of the program inside the archive, when it is not the first `provides` name.
    pub cmd: Option<String>,
    /// A detached signature (the ssh-signature check of ADR-0034): where to get it and the key id the entry trusts for it.
    pub signature: Option<(String, String)>,
}

/// The closed set of installer kinds. An entry names one and fills in its fixed parameters; it never carries a script to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Installer {
    /// `npm install --prefix <agent dir> PACKAGE@VERSION`; `integrity` is the registry's `sha512-<base64>` value.
    NpmGlobal { package: String, integrity: String },
    BinaryDownload {
        platforms: BTreeMap<String, Download>,
    },
    /// An OS package manager's user-level mode only.
    PackageManager { manager: String, package: String },
    /// A pinned script: the URL and its SHA-256. There is no unpinned form.
    VendorScript {
        url: String,
        sha256: String,
        interpreter: String,
    },
}

impl Installer {
    /// The kind's name in the format.
    pub fn kind(&self) -> &'static str {
        match self {
            Installer::NpmGlobal { .. } => "npm-global",
            Installer::BinaryDownload { .. } => "binary-download",
            Installer::PackageManager { .. } => "package-manager",
            Installer::VendorScript { .. } => "vendor-script",
        }
    }
}

/// What must print the agent's version: plain-token arguments, and a pattern the output must match.
#[derive(Debug, Clone)]
pub struct Verify {
    pub args: Vec<String>,
    regex: Regex,
}

impl Verify {
    /// Whether `output` (cut to `MAX_VERIFY_OUTPUT` bytes) matches the pattern.
    pub fn matches(&self, output: &str) -> bool {
        let mut end = output.len().min(MAX_VERIFY_OUTPUT);
        while !output.is_char_boundary(end) {
            end -= 1;
        }
        self.regex.is_match(&output[..end])
    }
    pub fn pattern(&self) -> &str {
        self.regex.as_str()
    }
}

impl PartialEq for Verify {
    fn eq(&self, other: &Self) -> bool {
        self.args == other.args && self.pattern() == other.pattern()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentPackage {
    /// The agent kind: equal to a manifest's `kind`.
    pub name: String,
    pub display: String,
    pub changelog: String,
    pub homepage: String,
    /// A publisher id declared in the list.
    pub publisher: String,
    pub installer: Installer,
    pub version: String,
    pub policy: VersionPolicy,
    /// Executable names the install must leave on the agent's path.
    pub provides: Vec<String>,
    pub verify: Verify,
}

/// A verified list: signed by the pinned root key, current, not rolled back, every entry valid.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentList {
    pub version: u64,
    pub expires_ms: u64,
    pub publishers: Vec<Publisher>,
    pub agents: Vec<AgentPackage>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    version: u64,
    expires_ms: u64,
    publishers: Vec<Publisher>,
    agents: Vec<Value>,
}

/// Opens a signed agent-package list. `last_seen_version` is the newest list version this node accepted (0 for none).
pub fn verify_list(
    signed: &str,
    root_key_hex: &str,
    now_ms: u64,
    last_seen_version: u64,
) -> Result<AgentList, ListError> {
    if signed.len() > MAX_LIST_BYTES {
        return Err(ListError::TooLarge);
    }
    let raw: Raw =
        open_signed(signed, root_key_hex, now_ms, last_seen_version).map_err(ListError::Signed)?;
    if raw.agents.len() > MAX_ENTRIES || raw.publishers.len() > MAX_ENTRIES {
        return Err(ListError::TooLarge);
    }
    for p in &raw.publishers {
        let key_ok = p.key.len() == 64 && p.key.chars().all(|c| c.is_ascii_hexdigit());
        if !valid_name(&p.id) || !key_ok {
            return Err(ListError::Signed(RegistryError::Malformed));
        }
    }
    let mut agents: Vec<AgentPackage> = Vec::new();
    for (index, v) in raw.agents.iter().enumerate() {
        let name = v
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .chars()
            .take(64)
            .collect::<String>();
        let entry = parse_entry(v).map_err(|reason| ListError::Entry {
            index,
            name: name.clone(),
            reason,
        })?;
        if !raw.publishers.iter().any(|p| p.id == entry.publisher) {
            return Err(ListError::UnknownPublisher(entry.publisher));
        }
        if agents
            .iter()
            .any(|a| a.name == entry.name && a.installer.kind() == entry.installer.kind())
        {
            return Err(ListError::Duplicate {
                name: entry.name,
                installer: entry.installer.kind(),
            });
        }
        agents.push(entry);
    }
    Ok(AgentList {
        version: raw.version,
        expires_ms: raw.expires_ms,
        publishers: raw.publishers,
        agents,
    })
}

impl AgentList {
    /// The entries for agent kind `name` that may be acted on now: refused when the publisher is revoked, the same rule as the
    /// adapter index (a revoked publisher refuses new installs). Nothing is installed by asking.
    pub fn offer(&self, name: &str) -> Result<Vec<&AgentPackage>, ListError> {
        let all: Vec<&AgentPackage> = self.agents.iter().filter(|a| a.name == name).collect();
        if all.is_empty() {
            return Err(ListError::NotOffered(name.to_string()));
        }
        for a in &all {
            if self
                .publishers
                .iter()
                .any(|p| p.id == a.publisher && p.revoked)
            {
                return Err(ListError::Revoked(a.publisher.clone()));
            }
        }
        Ok(all)
    }

    /// Entry names that are no manifest's `kind` (ADR-0087 refuses an entry whose `name` differs from the manifest). The caller
    /// decides what to do with the user-level Node entry, which has no manifest.
    pub fn without_manifest<'a>(&'a self, kinds: &[&str]) -> Vec<&'a str> {
        self.agents
            .iter()
            .map(|a| a.name.as_str())
            .filter(|n| !kinds.contains(n))
            .collect()
    }
}

// ---- entry validation ----------------------------------------------------------------------------------------------------

type R<T> = Result<T, Reason>;

fn object<'a>(v: &'a Value, allowed: &[&str]) -> R<&'a Map<String, Value>> {
    let m = v.as_object().ok_or(Reason::NotObject)?;
    if m.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err(Reason::UnknownField);
    }
    Ok(m)
}

fn string<'a>(m: &'a Map<String, Value>, key: &str) -> R<&'a str> {
    m.get(key)
        .ok_or(Reason::MissingField)?
        .as_str()
        .ok_or(Reason::MissingField)
}

fn list<'a>(m: &'a Map<String, Value>, key: &str, max: usize) -> R<&'a Vec<Value>> {
    let l = m
        .get(key)
        .ok_or(Reason::MissingField)?
        .as_array()
        .ok_or(Reason::MissingField)?;
    if l.len() > max {
        return Err(Reason::TooMany);
    }
    Ok(l)
}

/// Display text: shown in the window, so no control, format, bidirectional or separator character.
fn display(s: &str) -> R<String> {
    if s.is_empty() || s.chars().count() > 80 || !clean_text(s) {
        return Err(Reason::BadText);
    }
    Ok(s.to_string())
}

/// An `https://` URL with a host and nothing a shell, a header or a log could misread.
fn https_url(s: &str) -> R<String> {
    let rest = s.strip_prefix("https://").ok_or(Reason::BadUrl)?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let bad_char = |c: char| {
        c.is_whitespace()
            || c.is_control()
            || agora_core::text::is_hidden(c)
            || "\"'\\<>`{}|^".contains(c)
            || !c.is_ascii()
    };
    if s.len() > 500
        || host.is_empty()
        || host.contains('@')
        || host.starts_with(['.', ':'])
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-:".contains(c))
        || s.chars().any(bad_char)
    {
        return Err(Reason::BadUrl);
    }
    Ok(s.to_string())
}

fn sha256(s: &str) -> R<String> {
    if s.len() == 64 && s.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')) {
        Ok(s.to_string())
    } else {
        Err(Reason::BadHash)
    }
}

/// npm's integrity value: `sha512-` and the standard base64 of 64 bytes (88 characters ending `==`).
fn integrity(s: &str) -> R<String> {
    let b = s.strip_prefix("sha512-").ok_or(Reason::BadHash)?;
    let body = b.strip_suffix("==").ok_or(Reason::BadHash)?;
    if b.len() == 88
        && body
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/')
    {
        Ok(s.to_string())
    } else {
        Err(Reason::BadHash)
    }
}

/// An executable name: what `provides` lists and a package manager or npm may name.
fn exe_name(s: &str) -> R<String> {
    let ok = !s.is_empty()
        && s.len() <= 64
        && !s.starts_with(['.', '-'])
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    if !ok {
        return Err(Reason::BadToken);
    }
    if forbidden_executable(s) {
        return Err(Reason::ForbiddenExecutable);
    }
    Ok(s.to_string())
}

/// A package id for a package manager: letters, digits and `._-/@+` (a winget id has dots, a brew tap has a slash).
fn package_id(s: &str) -> R<String> {
    let ok = !s.is_empty()
        && s.len() <= 100
        && !s.starts_with(['-', '.', '/'])
        && !s.contains("..")
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-/@+".contains(c));
    if ok {
        Ok(s.to_string())
    } else {
        Err(Reason::BadToken)
    }
}

/// A `verify` argument: plain characters, a leading `-` allowed (`--version`); never a space, quote, `$`, `;`, `|`, `&`, or a path.
fn verify_arg(s: &str) -> R<String> {
    let ok = !s.is_empty()
        && s.len() <= 32
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._=-".contains(c));
    if ok {
        Ok(s.to_string())
    } else {
        Err(Reason::BadToken)
    }
}

fn version_core(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.split(['-', '+']).next()?;
    let mut p = core.split('.').map(|x| x.parse::<u64>().ok());
    Some((p.next()??, p.next()??, p.next()??))
}

fn exact(s: &str) -> R<String> {
    if exact_version(s) && s.len() <= 64 && version_core(s).is_some() {
        Ok(s.to_string())
    } else {
        Err(Reason::BadVersion)
    }
}

fn compile(pattern: &str) -> R<Regex> {
    if pattern.is_empty() || pattern.len() > MAX_REGEX_LEN || !clean_text(pattern) {
        return Err(Reason::BadRegex);
    }
    let re = RegexBuilder::new(pattern)
        .size_limit(REGEX_SIZE_LIMIT)
        .dfa_size_limit(REGEX_SIZE_LIMIT)
        .nest_limit(20)
        .build()
        .map_err(|_| Reason::BadRegex)?;
    // A pattern that matches nothing at all (the empty string) matches every output, so it verifies nothing.
    if re.is_match("") {
        return Err(Reason::BadRegex);
    }
    Ok(re)
}

fn parse_entry(v: &Value) -> R<AgentPackage> {
    let m = object(
        v,
        &[
            "name",
            "display",
            "changelog",
            "homepage",
            "publisher",
            "installer",
            "version_policy",
            "version",
            "min",
            "max",
            "provides",
            "verify",
        ],
    )?;
    let name = string(m, "name")?;
    if !valid_name(name) {
        return Err(Reason::BadName);
    }
    let display = display(string(m, "display")?)?;
    let changelog = https_url(string(m, "changelog")?)?;
    let homepage = https_url(string(m, "homepage")?)?;
    let publisher = string(m, "publisher")?;
    if !valid_name(publisher) {
        return Err(Reason::BadName);
    }
    let version = exact(string(m, "version")?)?;
    let policy = match string(m, "version_policy")? {
        "pin" | "track" if m.contains_key("min") || m.contains_key("max") => {
            return Err(Reason::BadPolicy)
        }
        "pin" => VersionPolicy::Pin,
        "track" => VersionPolicy::Track,
        "range" => {
            let (min, max) = (exact(string(m, "min")?)?, exact(string(m, "max")?)?);
            let (lo, hi, at) = (
                version_core(&min).ok_or(Reason::BadVersion)?,
                version_core(&max).ok_or(Reason::BadVersion)?,
                version_core(&version).ok_or(Reason::BadVersion)?,
            );
            if lo > hi || at < lo || at > hi {
                return Err(Reason::BadPolicy);
            }
            VersionPolicy::Range { min, max }
        }
        _ => return Err(Reason::BadPolicy),
    };
    let provides = list(m, "provides", 4)?
        .iter()
        .map(|p| exe_name(p.as_str().ok_or(Reason::BadToken)?))
        .collect::<R<Vec<String>>>()?;
    if provides.is_empty() {
        return Err(Reason::MissingField);
    }
    let vm = object(
        m.get("verify").ok_or(Reason::MissingField)?,
        &["args", "regex"],
    )?;
    let args = list(vm, "args", 4)?
        .iter()
        .map(|a| verify_arg(a.as_str().ok_or(Reason::BadToken)?))
        .collect::<R<Vec<String>>>()?;
    let verify = Verify {
        args,
        regex: compile(string(vm, "regex")?)?,
    };
    let installer = parse_installer(m.get("installer").ok_or(Reason::MissingField)?)?;
    Ok(AgentPackage {
        name: name.to_string(),
        display,
        changelog,
        homepage,
        publisher: publisher.to_string(),
        installer,
        version,
        policy,
        provides,
        verify,
    })
}

fn parse_installer(v: &Value) -> R<Installer> {
    let kind = v
        .get("kind")
        .and_then(Value::as_str)
        .ok_or(Reason::BadInstaller)?;
    match kind {
        "npm-global" => {
            let m = object(v, &["kind", "package", "integrity"])?;
            let package = string(m, "package")?;
            if !npm_name(package) {
                return Err(Reason::BadToken);
            }
            Ok(Installer::NpmGlobal {
                package: package.to_string(),
                integrity: integrity(string(m, "integrity")?)?,
            })
        }
        "binary-download" => {
            let m = object(v, &["kind", "platforms"])?;
            let ps = m
                .get("platforms")
                .and_then(Value::as_object)
                .ok_or(Reason::MissingField)?;
            if ps.is_empty() || ps.len() > 16 {
                return Err(Reason::BadPlatform);
            }
            let mut platforms = BTreeMap::new();
            for (key, p) in ps {
                platform_key(key)?;
                platforms.insert(key.clone(), download(p)?);
            }
            Ok(Installer::BinaryDownload { platforms })
        }
        "package-manager" => {
            let m = object(v, &["kind", "manager", "package"])?;
            let manager = string(m, "manager")?;
            if !["brew", "scoop", "winget"].contains(&manager) {
                return Err(Reason::BadInstaller);
            }
            Ok(Installer::PackageManager {
                manager: manager.to_string(),
                package: package_id(string(m, "package")?)?,
            })
        }
        "vendor-script" => {
            let m = object(v, &["kind", "url", "sha256", "interpreter"])?;
            let interpreter = string(m, "interpreter")?;
            if !["sh", "powershell"].contains(&interpreter) {
                return Err(Reason::BadInstaller);
            }
            Ok(Installer::VendorScript {
                url: https_url(string(m, "url")?)?,
                sha256: sha256(string(m, "sha256")?)?,
                interpreter: interpreter.to_string(),
            })
        }
        _ => Err(Reason::BadInstaller),
    }
}

/// `<os>-<arch>` with an optional `-musl`, the converter's names (`darwin-aarch64`, `linux-x86_64-musl`, `windows-x86_64`).
fn platform_key(k: &str) -> R<()> {
    let ok = k.len() <= 32
        && ["linux-", "darwin-", "windows-"]
            .iter()
            .any(|os| k.starts_with(os))
        && k.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(Reason::BadPlatform)
    }
}

fn download(v: &Value) -> R<Download> {
    let m = object(
        v,
        &["url", "sha256", "cmd", "signature_url", "signature_key"],
    )?;
    let cmd = match m.get("cmd") {
        None => None,
        Some(c) => Some(command(c.as_str().ok_or(Reason::BadCommand)?)?),
    };
    let signature = match (m.get("signature_url"), m.get("signature_key")) {
        (None, None) => None,
        (Some(u), Some(k)) => {
            let key = k.as_str().ok_or(Reason::BadToken)?;
            if !valid_name(key) {
                return Err(Reason::BadToken);
            }
            Some((
                https_url(u.as_str().ok_or(Reason::BadUrl)?)?,
                key.to_string(),
            ))
        }
        _ => return Err(Reason::MissingField),
    };
    Ok(Download {
        url: https_url(string(m, "url")?)?,
        sha256: sha256(string(m, "sha256")?)?,
        cmd,
        signature,
    })
}

/// A relative path inside the archive (`./tool`, `bin/tool`, `.\\pkg\\tool.exe` as the converter writes it): no `..`, no drive, no
/// absolute path, and never a shell or interpreter as the program.
fn command(c: &str) -> R<String> {
    let unix = c.replace('\\', "/");
    let ok = !c.is_empty()
        && c.len() <= 200
        && !unix.starts_with('/')
        && !unix.contains(':')
        && unix.split('/').all(|p| p != "..")
        && unix
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || "._/-".contains(ch));
    if !ok {
        return Err(Reason::BadCommand);
    }
    let file = unix.rsplit('/').next().unwrap_or("");
    let exe = file.strip_suffix(".exe").unwrap_or(file);
    if exe.is_empty() || forbidden_executable(exe) {
        return Err(Reason::ForbiddenExecutable);
    }
    Ok(c.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use serde_json::json;

    fn key() -> Ed25519KeyPair {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
    }
    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }
    fn pubhex(k: &Ed25519KeyPair) -> String {
        hex(k.public_key().as_ref())
    }
    fn sign(k: &Ed25519KeyPair, payload: &str) -> String {
        json!({"payload": payload, "sig": hex(k.sign(payload.as_bytes()).as_ref())}).to_string()
    }
    fn entry() -> Value {
        json!({
            "name": "demo-agent", "display": "Demo Agent",
            "changelog": "https://example.invalid/demo/changes", "homepage": "https://example.invalid/demo",
            "publisher": "official", "version_policy": "pin", "version": "1.2.3",
            "provides": ["demo"], "verify": {"args": ["--version"], "regex": "^demo [0-9]+\\.[0-9]+\\.[0-9]+"},
            "installer": {"kind": "npm-global", "package": "@demo/agent",
                "integrity": format!("sha512-{}==", "A".repeat(86))},
        })
    }
    fn payload(
        version: u64,
        publisher: &Ed25519KeyPair,
        revoked: bool,
        agents: Vec<Value>,
    ) -> String {
        json!({"version": version, "expires_ms": 1000,
            "publishers": [{"id": "official", "key": pubhex(publisher), "revoked": revoked}],
            "agents": agents})
        .to_string()
    }
    fn with(f: impl Fn(&mut Value)) -> Value {
        let mut e = entry();
        f(&mut e);
        e
    }
    fn open_with(e: Value) -> Result<AgentList, ListError> {
        let (root, publisher) = (key(), key());
        verify_list(
            &sign(&root, &payload(1, &publisher, false, vec![e])),
            &pubhex(&root),
            0,
            0,
        )
    }
    fn refused(e: Value) -> Reason {
        match open_with(e) {
            Err(ListError::Entry { reason, .. }) => reason,
            other => panic!("expected an entry error, got {other:?}"),
        }
    }

    #[test]
    fn a_good_list_opens_and_offers_its_entry() {
        let l = open_with(entry()).unwrap();
        let a = &l.offer("demo-agent").unwrap()[0];
        assert_eq!(a.installer.kind(), "npm-global");
        assert_eq!(a.policy, VersionPolicy::Pin);
        assert!(a.verify.matches("demo 1.2.3 (build)"));
        assert!(!a.verify.matches("something else"));
        assert_eq!(l.without_manifest(&["claude-code"]), ["demo-agent"]);
        assert!(l.without_manifest(&["demo-agent"]).is_empty());
        assert_eq!(l.offer("ghost"), Err(ListError::NotOffered("ghost".into())));
    }

    #[test]
    fn the_adr_0044_checks_apply_to_the_list() {
        let (root, other, publisher) = (key(), key(), key());
        let good = payload(5, &publisher, false, vec![entry()]);
        // Not signed by the pinned root.
        assert_eq!(
            verify_list(&sign(&other, &good), &pubhex(&root), 0, 0),
            Err(ListError::Signed(RegistryError::BadSignature))
        );
        // Tampered after signing.
        let tampered = sign(&root, &good).replace("demo-agent", "evil-agent");
        assert!(matches!(
            verify_list(&tampered, &pubhex(&root), 0, 0),
            Err(ListError::Signed(
                RegistryError::BadSignature | RegistryError::Malformed
            ))
        ));
        // Rolled back, expired.
        assert_eq!(
            verify_list(&sign(&root, &good), &pubhex(&root), 10, 6),
            Err(ListError::Signed(RegistryError::Rollback {
                seen: 6,
                offered: 5
            }))
        );
        assert_eq!(
            verify_list(&sign(&root, &good), &pubhex(&root), 1000, 0),
            Err(ListError::Signed(RegistryError::Expired))
        );
        assert!(verify_list(&sign(&root, &good), &pubhex(&root), 10, 5).is_ok());
        // Revoked publisher: the list opens (so the owner can see why) but nothing is offered.
        let revoked = payload(5, &publisher, true, vec![entry()]);
        let l = verify_list(&sign(&root, &revoked), &pubhex(&root), 0, 0).unwrap();
        assert_eq!(
            l.offer("demo-agent"),
            Err(ListError::Revoked("official".into()))
        );
        // An entry that names an undeclared publisher refuses the list.
        let ghost = with(|e| e["publisher"] = json!("ghost"));
        assert_eq!(
            open_with(ghost),
            Err(ListError::UnknownPublisher("ghost".into()))
        );
        // Size limits.
        assert_eq!(
            verify_list(&"x".repeat(MAX_LIST_BYTES + 1), &pubhex(&root), 0, 0),
            Err(ListError::TooLarge)
        );
    }

    #[test]
    fn a_duplicate_name_and_installer_is_refused_but_two_kinds_may_share_a_name() {
        let (root, publisher) = (key(), key());
        let list = |agents| {
            verify_list(
                &sign(&root, &payload(1, &publisher, false, agents)),
                &pubhex(&root),
                0,
                0,
            )
        };
        assert_eq!(
            list(vec![entry(), entry()]),
            Err(ListError::Duplicate {
                name: "demo-agent".into(),
                installer: "npm-global"
            })
        );
        let other = with(|e| {
            e["installer"] =
                json!({"kind": "package-manager", "manager": "brew", "package": "demo"})
        });
        assert_eq!(
            list(vec![entry(), other])
                .unwrap()
                .offer("demo-agent")
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn all_four_installer_kinds_parse() {
        let sha = "a".repeat(64);
        let bin = with(|e| {
            e["installer"] = json!({"kind": "binary-download", "platforms": {
                "linux-x86_64": {"url": "https://example.invalid/d.tar.gz", "sha256": sha, "cmd": "./demo"},
                "windows-x86_64": {"url": "https://example.invalid/d.zip", "sha256": sha, "cmd": ".\\pkg\\demo.exe",
                    "signature_url": "https://example.invalid/d.zip.sig", "signature_key": "release"},
            }})
        });
        assert!(matches!(
            open_with(bin).unwrap().agents[0].installer,
            Installer::BinaryDownload { ref platforms } if platforms.len() == 2
        ));
        let script = with(
            |e| e["installer"] = json!({"kind": "vendor-script", "url": "https://example.invalid/i.sh", "sha256": sha, "interpreter": "sh"}),
        );
        assert_eq!(
            open_with(script).unwrap().agents[0].installer.kind(),
            "vendor-script"
        );
        let pm = with(|e| e["installer"] = json!({"kind": "winget", "x": 1}));
        assert_eq!(refused(pm), Reason::BadInstaller);
    }

    #[test]
    fn version_policies_are_strict() {
        let range = |v: &str, lo: &str, hi: &str| {
            with(|e| {
                e["version_policy"] = json!("range");
                e["version"] = json!(v);
                e["min"] = json!(lo);
                e["max"] = json!(hi);
            })
        };
        assert!(open_with(range("1.5.0", "1.0.0", "2.0.0")).is_ok());
        assert_eq!(refused(range("2.5.0", "1.0.0", "2.0.0")), Reason::BadPolicy);
        assert_eq!(refused(range("1.5.0", "2.0.0", "1.0.0")), Reason::BadPolicy);
        assert_eq!(refused(range("1.5.0", "1.0.0", "2")), Reason::BadVersion);
        // A range with no bounds, and min/max on a pin or a track.
        assert_eq!(
            refused(with(|e| e["version_policy"] = json!("range"))),
            Reason::MissingField
        );
        assert_eq!(
            refused(with(|e| e["min"] = json!("1.0.0"))),
            Reason::BadPolicy
        );
        assert_eq!(
            refused(with(|e| {
                e["version_policy"] = json!("track");
                e["max"] = json!("9.0.0");
            })),
            Reason::BadPolicy
        );
        assert!(open_with(with(|e| e["version_policy"] = json!("track"))).is_ok());
        assert_eq!(
            refused(with(|e| e["version_policy"] = json!("latest"))),
            Reason::BadPolicy
        );
        for bad in [
            "latest", "^1.2.3", "1.2", "1.x", ">=1.0.0", "1.2.3 ", "", "1.2.3;rm",
        ] {
            assert_eq!(
                refused(with(|e| e["version"] = json!(bad))),
                Reason::BadVersion,
                "{bad}"
            );
        }
    }

    #[test]
    fn hostile_entries_are_refused() {
        let sha = "a".repeat(64);
        let bin = |p: Value| {
            with(|e| {
                e["installer"] =
                    json!({"kind": "binary-download", "platforms": {"linux-x86_64": p}})
            })
        };
        let dl = |extra: Value| {
            let mut d = json!({"url": "https://example.invalid/d.tar.gz", "sha256": sha});
            for (k, v) in extra.as_object().unwrap() {
                d[k] = v.clone();
            }
            d
        };
        let cases: Vec<(&str, Value, Reason)> = vec![
            (
                "unknown top field",
                with(|e| e["postinstall"] = json!("rm -rf ~")),
                Reason::UnknownField,
            ),
            (
                "script field",
                with(|e| e["installer"]["script"] = json!("curl x | sh")),
                Reason::UnknownField,
            ),
            (
                "name with a slash",
                with(|e| e["name"] = json!("../evil")),
                Reason::BadName,
            ),
            (
                "upper-case name",
                with(|e| e["name"] = json!("Demo")),
                Reason::BadName,
            ),
            (
                "http homepage",
                with(|e| e["homepage"] = json!("http://example.invalid/")),
                Reason::BadUrl,
            ),
            (
                "userinfo url",
                with(|e| e["changelog"] = json!("https://good.invalid@evil.invalid/")),
                Reason::BadUrl,
            ),
            (
                "url with a space",
                with(|e| e["changelog"] = json!("https://example.invalid/a b")),
                Reason::BadUrl,
            ),
            (
                "url with a quote",
                with(|e| e["changelog"] = json!("https://example.invalid/a\"b")),
                Reason::BadUrl,
            ),
            (
                "file url",
                with(|e| e["changelog"] = json!("file:///etc/passwd")),
                Reason::BadUrl,
            ),
            (
                "rtl display",
                with(|e| e["display"] = json!("Demo\u{202e}evil")),
                Reason::BadText,
            ),
            (
                "empty display",
                with(|e| e["display"] = json!("")),
                Reason::BadText,
            ),
            (
                "shell provides",
                with(|e| e["provides"] = json!(["sh"])),
                Reason::ForbiddenExecutable,
            ),
            (
                "path provides",
                with(|e| e["provides"] = json!(["/usr/bin/demo"])),
                Reason::BadToken,
            ),
            (
                "no provides",
                with(|e| e["provides"] = json!([])),
                Reason::MissingField,
            ),
            (
                "verify arg with shell syntax",
                with(|e| e["verify"]["args"] = json!(["--version; id"])),
                Reason::BadToken,
            ),
            (
                "verify arg with substitution",
                with(|e| e["verify"]["args"] = json!(["$(id)"])),
                Reason::BadToken,
            ),
            (
                "verify arg a path",
                with(|e| e["verify"]["args"] = json!(["/etc/passwd"])),
                Reason::BadToken,
            ),
            (
                "verify has an extra field",
                with(|e| e["verify"]["cmd"] = json!("sh")),
                Reason::UnknownField,
            ),
            (
                "regex matching everything",
                with(|e| e["verify"]["regex"] = json!(".*")),
                Reason::BadRegex,
            ),
            (
                "regex too long",
                with(|e| e["verify"]["regex"] = json!("a".repeat(MAX_REGEX_LEN + 1))),
                Reason::BadRegex,
            ),
            (
                "regex does not compile",
                with(|e| e["verify"]["regex"] = json!("(unclosed")),
                Reason::BadRegex,
            ),
            (
                "regex backreference",
                with(|e| e["verify"]["regex"] = json!("(a)\\1")),
                Reason::BadRegex,
            ),
            (
                "regex too large to compile",
                with(|e| e["verify"]["regex"] = json!("(((a{100}){100}){100}){100}")),
                Reason::BadRegex,
            ),
            (
                "npm url as package",
                with(|e| e["installer"]["package"] = json!("https://evil.invalid/p.tgz")),
                Reason::BadToken,
            ),
            (
                "npm git package",
                with(|e| e["installer"]["package"] = json!("github:evil/p")),
                Reason::BadToken,
            ),
            (
                "npm tag in package",
                with(|e| e["installer"]["package"] = json!("demo@latest")),
                Reason::BadToken,
            ),
            (
                "sha1 integrity",
                with(|e| e["installer"]["integrity"] = json!("sha1-AAAAAAAAAAAAAAAAAAAAAAAAAAA=")),
                Reason::BadHash,
            ),
            (
                "short integrity",
                with(|e| e["installer"]["integrity"] = json!("sha512-AAAA==")),
                Reason::BadHash,
            ),
            (
                "missing integrity",
                with(|e| {
                    e["installer"].as_object_mut().unwrap().remove("integrity");
                }),
                Reason::MissingField,
            ),
            (
                "http download",
                bin(dl(json!({"url": "http://example.invalid/d"}))),
                Reason::BadUrl,
            ),
            (
                "upper-case sha256",
                bin(dl(json!({"sha256": "A".repeat(64)}))),
                Reason::BadHash,
            ),
            (
                "short sha256",
                bin(dl(json!({"sha256": "abc"}))),
                Reason::BadHash,
            ),
            (
                "no sha256",
                bin(json!({"url": "https://example.invalid/d"})),
                Reason::MissingField,
            ),
            (
                "absolute cmd",
                bin(dl(json!({"cmd": "/bin/sh"}))),
                Reason::BadCommand,
            ),
            (
                "dotdot cmd",
                bin(dl(json!({"cmd": "../../bin/demo"}))),
                Reason::BadCommand,
            ),
            (
                "drive cmd",
                bin(dl(json!({"cmd": "C:\\demo.exe"}))),
                Reason::BadCommand,
            ),
            (
                "shell cmd",
                bin(dl(json!({"cmd": "./bash"}))),
                Reason::ForbiddenExecutable,
            ),
            (
                "signature url without key",
                bin(dl(json!({"signature_url": "https://example.invalid/s"}))),
                Reason::MissingField,
            ),
            (
                "odd platform",
                with(
                    |e| e["installer"] = json!({"kind": "binary-download", "platforms": {"../x": {"url": "https://example.invalid/d", "sha256": sha}}}),
                ),
                Reason::BadPlatform,
            ),
            (
                "no platform",
                with(|e| e["installer"] = json!({"kind": "binary-download", "platforms": {}})),
                Reason::BadPlatform,
            ),
            (
                "script without hash",
                with(
                    |e| e["installer"] = json!({"kind": "vendor-script", "url": "https://example.invalid/i.sh", "interpreter": "sh"}),
                ),
                Reason::MissingField,
            ),
            (
                "script bad interpreter",
                with(
                    |e| e["installer"] = json!({"kind": "vendor-script", "url": "https://example.invalid/i.sh", "sha256": sha, "interpreter": "bash -c"}),
                ),
                Reason::BadInstaller,
            ),
            (
                "package manager sudo",
                with(|e| {
                    e["installer"] =
                        json!({"kind": "package-manager", "manager": "apt", "package": "demo"})
                }),
                Reason::BadInstaller,
            ),
            (
                "package manager flag",
                with(
                    |e| e["installer"] = json!({"kind": "package-manager", "manager": "brew", "package": "--cask evil"}),
                ),
                Reason::BadToken,
            ),
            (
                "package id with ..",
                with(|e| {
                    e["installer"] =
                        json!({"kind": "package-manager", "manager": "brew", "package": "a/../b"})
                }),
                Reason::BadToken,
            ),
            (
                "installer not an object",
                with(|e| e["installer"] = json!("npm-global")),
                Reason::BadInstaller,
            ),
        ];
        for (what, e, want) in cases {
            assert_eq!(refused(e), want, "{what}");
        }
        assert_eq!(refused(json!("not an object")), Reason::NotObject);
        assert_eq!(refused(json!({})), Reason::MissingField);
    }

    #[test]
    fn one_bad_entry_refuses_the_whole_list_and_is_named() {
        let (root, publisher) = (key(), key());
        let bad = with(|e| {
            e["name"] = json!("second");
            e["version"] = json!("latest");
        });
        let signed = sign(&root, &payload(1, &publisher, false, vec![entry(), bad]));
        assert_eq!(
            verify_list(&signed, &pubhex(&root), 0, 0),
            Err(ListError::Entry {
                index: 1,
                name: "second".into(),
                reason: Reason::BadVersion
            })
        );
    }

    #[test]
    fn a_pathological_regex_cannot_hang_verification() {
        let l = open_with(with(|e| e["verify"]["regex"] = json!("(a+)+b$"))).unwrap();
        let hostile = format!("{}!", "a".repeat(MAX_VERIFY_OUTPUT * 4));
        let t = std::time::Instant::now();
        assert!(!l.agents[0].verify.matches(&hostile));
        assert!(t.elapsed().as_secs() < 5, "linear-time engine");
        // Output past the limit is not read: a match placed after it is not seen.
        let late = format!("{}demo 1.0.0", " ".repeat(MAX_VERIFY_OUTPUT));
        let l = open_with(entry()).unwrap();
        assert!(!l.agents[0].verify.matches(&late));
    }

    #[test]
    fn unknown_top_level_fields_and_bad_publishers_refuse_the_list() {
        let (root, publisher) = (key(), key());
        let mut p: Value =
            serde_json::from_str(&payload(1, &publisher, false, vec![entry()])).unwrap();
        p["agents_extra"] = json!([]);
        assert_eq!(
            verify_list(&sign(&root, &p.to_string()), &pubhex(&root), 0, 0),
            Err(ListError::Signed(RegistryError::Malformed))
        );
        let mut p: Value =
            serde_json::from_str(&payload(1, &publisher, false, vec![entry()])).unwrap();
        p["publishers"][0]["key"] = json!("abcd");
        assert_eq!(
            verify_list(&sign(&root, &p.to_string()), &pubhex(&root), 0, 0),
            Err(ListError::Signed(RegistryError::Malformed))
        );
    }
}

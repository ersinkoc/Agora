//! ADR-0087 rung 3: installing ONE agent program from a signed list, with the installer kind `binary-download`, on the machine the
//! owner is typing at. The list is already verified (`agent_packages`); this module plans the action, downloads, checks, puts the
//! program in `<AGORA_HOME>/agents/<name>/<version>/`, runs the entry's `verify`, and only then moves the launcher in
//! `<AGORA_HOME>/agents/bin/`. Nothing is written to the live place until every check passed, and a failure leaves the old launcher.
//!
//! What this is not: no hub, no protocol message, no signed device approval (rung 4: the owner types the agent name here), no
//! other installer kind, and no archive: this tree has no inflate code, so an entry with `cmd` (a program inside an archive) is
//! refused with `agent.install.unsupported_format` until a later rung adds a restrictive reader. The network and the signature
//! tool are behind small traits so the tests run the real hash, signature, switch and rollback logic against a fake agent without
//! a network; the command line always passes the strict implementations ([`SystemResolver`], [`CurlFetcher`], [`SshKeygen`]).
//! The address rules apply to whatever resolver is passed, so a fake resolver cannot be used to skip them.

use std::cmp::Ordering;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read, Seek, Write};
use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use agora_platform::{ConfinedDir, EntryKind};
use sha2::{Digest, Sha256};

use crate::agent_packages::{AgentList, Installer, ListError, Verify};
use crate::package::valid_name;

/// Largest download, the largest signature file, and how long `verify` may run.
pub const MAX_DOWNLOAD_BYTES: u64 = 200 * 1024 * 1024;
pub const MAX_SIGNATURE_BYTES: u64 = 64 * 1024;
pub const VERIFY_TIMEOUT: Duration = Duration::from_secs(10);
/// Seconds one download may take.
pub const DOWNLOAD_SECS: u64 = 600;

/// The stable result codes of ADR-0087 item 10 (`agent.install.*`), plus the few this rung needs that the ADR does not list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    IndexInvalid,
    NotApproved,
    DownloadFailed,
    ChecksumMismatch,
    SignatureInvalid,
    DiskFull,
    VerifyFailed,
    Timeout,
    UnsupportedPlatform,
    InProgress,
    Revoked,
    /// The entry's version is lower than the installed one and the owner did not confirm going back (ADR-0087 item 9).
    Downgrade,
    /// The download is an archive (not supported yet) or the entry cannot be installed as one plain program.
    UnsupportedFormat,
    /// The list has no `binary-download` entry of that name.
    NotOffered,
    /// `curl` or `ssh-keygen` is not installed.
    ToolMissing,
    /// Any other file system failure.
    IoFailed,
    /// The agent is named `bin`, or another agent already owns the launcher name.
    Conflict,
}

impl Code {
    pub fn code(self) -> &'static str {
        match self {
            Code::IndexInvalid => "agent.install.index_invalid",
            Code::NotApproved => "agent.install.not_approved",
            Code::DownloadFailed => "agent.install.download_failed",
            Code::ChecksumMismatch => "agent.install.checksum_mismatch",
            Code::SignatureInvalid => "agent.install.signature_invalid",
            Code::DiskFull => "agent.install.disk_full",
            Code::VerifyFailed => "agent.install.verify_failed",
            Code::Timeout => "agent.install.timeout",
            Code::UnsupportedPlatform => "agent.install.unsupported_platform",
            Code::InProgress => "agent.install.in_progress",
            Code::Revoked => "agent.install.revoked",
            Code::Downgrade => "agent.install.downgrade",
            Code::UnsupportedFormat => "agent.install.unsupported_format",
            Code::NotOffered => "agent.install.not_offered",
            Code::ToolMissing => "agent.install.tool_missing",
            Code::IoFailed => "agent.install.io_failed",
            Code::Conflict => "agent.install.conflict",
        }
    }
}

/// A failed install: the code and a short machine token (`blocked_address`, `too_large`, `no_match`, ...), never user text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallError {
    pub code: Code,
    pub detail: String,
}

fn err<T>(code: Code, detail: impl Into<String>) -> Result<T, InstallError> {
    Err(InstallError {
        code,
        detail: detail.into(),
    })
}

fn io_err(e: &io::Error) -> InstallError {
    InstallError {
        code: if e.kind() == io::ErrorKind::StorageFull {
            Code::DiskFull
        } else {
            Code::IoFailed
        },
        detail: io_detail(e),
    }
}

/// The detail of an I/O failure: the kind of error, with the system's error number when the kind alone does not say what it was
/// (for example `uncategorized_32` for a Windows error the standard library gives no kind).
fn io_detail(e: &io::Error) -> String {
    let kind = format!("{:?}", e.kind()).to_ascii_lowercase();
    match e.raw_os_error() {
        Some(n) if kind == "uncategorized" || kind == "other" => format!("{kind}_{n}"),
        _ => kind,
    }
}

// ---- the plan ------------------------------------------------------------------------------------------------------------

/// How the entry's version compares with what is installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Fresh,
    Upgrade,
    /// Already installed: nothing to do.
    Same,
    /// Lower than installed: needs the owner's separate confirmation.
    Downgrade,
}

/// The exact action, from the entry and the node's own state: everything the owner is shown before typing the name.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub name: String,
    pub display: String,
    pub version: String,
    pub platform: String,
    pub url: String,
    pub host: String,
    pub port: u16,
    pub sha256: String,
    pub publisher: String,
    pub publisher_key: String,
    /// The detached signature's address and the name of the key the entry trusts for it.
    pub signature: Option<(String, String)>,
    /// `<AGORA_HOME>/agents/<name>/<version>`.
    pub target_dir: PathBuf,
    /// The file name of the program (the one `provides` name, with `.exe` on Windows).
    pub exe: String,
    pub verify_args: Vec<String>,
    pub verify_pattern: String,
    pub installed: Option<String>,
    pub change: Change,
    verify: Verify,
}

/// `<AGORA_HOME>/agents`.
pub fn agents_dir(home: &Path) -> PathBuf {
    home.join("agents")
}

/// Plans the install of `name` for `platform` (`agora_platform::platform_key()`), refusing what cannot be acted on.
pub fn plan(
    list: &AgentList,
    name: &str,
    platform: &str,
    home: &Path,
) -> Result<Plan, InstallError> {
    let offered = list.offer(name).map_err(|e| match e {
        ListError::Revoked(p) => InstallError {
            code: Code::Revoked,
            detail: p,
        },
        _ => InstallError {
            code: Code::NotOffered,
            detail: "unknown_name".into(),
        },
    })?;
    let Some((entry, platforms)) = offered.iter().find_map(|e| match &e.installer {
        Installer::BinaryDownload { platforms } => Some((*e, platforms)),
        _ => None,
    }) else {
        return err(Code::NotOffered, "no_binary_download");
    };
    let Some(download) = platforms.get(platform) else {
        return err(Code::UnsupportedPlatform, platform);
    };
    // No archive reader yet (module docs): a program inside an archive is not installable here.
    if download.cmd.is_some() {
        return err(Code::UnsupportedFormat, "archive");
    }
    let [program] = entry.provides.as_slice() else {
        return err(Code::UnsupportedFormat, "one_program_per_download");
    };
    let (host, port) = split_url(&download.url)?;
    let exe = exe_name(program);
    check_names(home, &entry.name, &exe)?;
    let publisher_key = list
        .publishers
        .iter()
        .find(|p| p.id == entry.publisher)
        .map(|p| p.key.clone())
        .unwrap_or_default();
    let installed = installed_version(home, name);
    let change = match installed.as_deref() {
        None => Change::Fresh,
        Some(v) => match compare_versions(&entry.version, v) {
            Ordering::Greater => Change::Upgrade,
            Ordering::Equal => Change::Same,
            Ordering::Less => Change::Downgrade,
        },
    };
    Ok(Plan {
        name: entry.name.clone(),
        display: entry.display.clone(),
        version: entry.version.clone(),
        platform: platform.to_string(),
        url: download.url.clone(),
        host,
        port,
        sha256: download.sha256.clone(),
        publisher: entry.publisher.clone(),
        publisher_key,
        signature: download.signature.clone(),
        target_dir: agents_dir(home).join(&entry.name).join(&entry.version),
        exe,
        verify_args: entry.verify.args.clone(),
        verify_pattern: entry.verify.pattern().to_string(),
        installed,
        change,
        verify: entry.verify.clone(),
    })
}

fn exe_name(program: &str) -> String {
    let suffix = agora_platform::EXECUTABLE_SUFFIX;
    if suffix.is_empty() || program.to_ascii_lowercase().ends_with(suffix) {
        program.to_string()
    } else {
        format!("{program}{suffix}")
    }
}

/// Semantic-version order: `major.minor.patch` as numbers; a release is above its own pre-release; pre-release identifiers
/// (split on `.`) compare one by one, numbers as numbers and below words, words as text, and the shorter list is lower when the
/// rest is equal (`rc.2` < `rc.10`, `1.0.0-1` < `1.0.0-alpha`). Build metadata (`+...`) is ignored.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    fn split(v: &str) -> (Vec<u64>, Option<&str>) {
        let v = v.split('+').next().unwrap_or(v);
        let (core, pre) = match v.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (v, None),
        };
        (
            core.split('.').map(|n| n.parse().unwrap_or(0)).collect(),
            pre,
        )
    }
    fn ident(a: &str, b: &str) -> Ordering {
        let num = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
        match (num(a), num(b)) {
            (true, true) => {
                let (x, y) = (a.trim_start_matches('0'), b.trim_start_matches('0'));
                x.len().cmp(&y.len()).then_with(|| x.cmp(y))
            }
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => a.cmp(b),
        }
    }
    let ((na, pa), (nb, pb)) = (split(a), split(b));
    na.cmp(&nb).then_with(|| match (pa, pb) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => {
            let (mut x, mut y) = (x.split('.'), y.split('.'));
            loop {
                match (x.next(), y.next()) {
                    (None, None) => return Ordering::Equal,
                    (None, Some(_)) => return Ordering::Less,
                    (Some(_), None) => return Ordering::Greater,
                    (Some(p), Some(q)) => match ident(p, q) {
                        Ordering::Equal => {}
                        o => return o,
                    },
                }
            }
        }
    })
}

/// The version `agora` installed last for `name` (the `current` file beside its version folders), if any.
pub fn installed_version(home: &Path, name: &str) -> Option<String> {
    let text = std::fs::read_to_string(agents_dir(home).join(name).join("current")).ok()?;
    let v = text.trim();
    (!v.is_empty() && valid_component_text(v)).then(|| v.to_string())
}

fn valid_component_text(v: &str) -> bool {
    !v.is_empty()
        && v != "."
        && v != ".."
        && v.len() <= 64
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
}

// ---- approval ------------------------------------------------------------------------------------------------------------

/// Proof that the owner typed the agent's name on this machine. It can only be made by [`Approval::typed`], and [`install`]
/// takes one, so nothing installs without it.
#[derive(Debug)]
pub struct Approval(());

impl Approval {
    /// `typed` is what the owner typed for the name (only the line ending is ignored: the name must be exact). Going to a lower
    /// version than the installed one also needs `go_back` to be the version, typed separately.
    pub fn typed(plan: &Plan, typed: &str, go_back: Option<&str>) -> Result<Self, InstallError> {
        let typed = typed.trim_end_matches(['\r', '\n']);
        if typed != plan.name {
            return err(Code::NotApproved, "name_differs");
        }
        if plan.change == Change::Downgrade {
            match go_back.map(|g| g.trim_end_matches(['\r', '\n'])) {
                Some(v) if v == plan.version => {}
                Some(_) => return err(Code::NotApproved, "version_differs"),
                None => return err(Code::Downgrade, plan.installed.clone().unwrap_or_default()),
            }
        }
        Ok(Self(()))
    }
}

// ---- the network seams ---------------------------------------------------------------------------------------------------

/// Names to addresses.
pub trait Resolver {
    fn resolve(&self, host: &str, port: u16) -> io::Result<Vec<IpAddr>>;
}

/// The system's resolver.
pub struct SystemResolver;

impl Resolver for SystemResolver {
    fn resolve(&self, host: &str, port: u16) -> io::Result<Vec<IpAddr>> {
        Ok((host, port).to_socket_addrs()?.map(|a| a.ip()).collect())
    }
}

/// One checked request: `addr` is the address the host resolved to and passed the address rules; a fetcher must connect there and
/// nowhere else, and must not follow a redirect.
#[derive(Debug, Clone)]
pub struct FetchRequest<'a> {
    pub url: &'a str,
    pub host: &'a str,
    pub port: u16,
    pub addr: IpAddr,
    pub max_bytes: u64,
    pub timeout_secs: u64,
}

/// Writes the body of the response to `sink`. An `Err` from the sink ends the transfer.
pub trait Fetcher {
    fn fetch(&self, request: &FetchRequest<'_>, sink: &mut dyn Write) -> Result<(), InstallError>;
}

/// Checks a detached signature over `signed`: `key_file` is the owner's file in `<AGORA_HOME>/agent-keys/`.
pub trait SignatureCheck {
    fn check(&self, key_file: &Path, signed: &Path, signature: &Path) -> Result<(), InstallError>;
}

/// Everything [`install`] reaches outside the disk.
pub struct Tools<'a> {
    pub resolver: &'a dyn Resolver,
    pub fetcher: &'a dyn Fetcher,
    pub signatures: &'a dyn SignatureCheck,
    /// The size cap of the download ([`MAX_DOWNLOAD_BYTES`] for the command line; tests lower it).
    pub max_download: u64,
    /// How long `verify` may run ([`VERIFY_TIMEOUT`] for the command line; tests lower it).
    pub verify_timeout: Duration,
    /// A failure to inject after the launcher moved (tests only; `None` for the command line).
    pub fault: Option<Fault>,
}

/// Where a test makes the install fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// Right after the launcher moved, before `current` is written.
    BeforeCurrent,
    /// After `exe` is written, as if `current` could not be, and putting the records back fails too.
    RecordsUnrestorable,
}

/// What a record file (`exe`, `current`) held before the switch, so a failed switch can put it back exactly.
enum Was {
    Held(Vec<u8>),
    Absent,
    /// It could not be read (a folder in its place, say): it is left as it is.
    Unknown,
}

impl Was {
    fn of(path: &Path) -> Self {
        match std::fs::read(path) {
            Ok(b) => Was::Held(b),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Was::Absent,
            Err(_) => Was::Unknown,
        }
    }

    /// Puts the file back as it was; `false` when that failed.
    fn put_back(&self, path: &Path) -> bool {
        match self {
            Was::Held(b) => agora_platform::write_private(path, b).is_ok(),
            Was::Absent => match std::fs::remove_file(path) {
                Ok(()) => true,
                Err(e) => e.kind() == io::ErrorKind::NotFound,
            },
            Was::Unknown => true,
        }
    }
}

impl<'a> Tools<'a> {
    /// The limits of the command line: [`MAX_DOWNLOAD_BYTES`] and [`VERIFY_TIMEOUT`].
    pub fn new(
        resolver: &'a dyn Resolver,
        fetcher: &'a dyn Fetcher,
        signatures: &'a dyn SignatureCheck,
    ) -> Self {
        Self {
            resolver,
            fetcher,
            signatures,
            max_download: MAX_DOWNLOAD_BYTES,
            verify_timeout: VERIFY_TIMEOUT,
            fault: None,
        }
    }
}

/// The tools the command line uses.
pub fn strict_tools() -> (SystemResolver, CurlFetcher, SshKeygen) {
    (SystemResolver, CurlFetcher, SshKeygen)
}

/// Whether the address is one a download must never go to: loopback, private, link-local, unique-local, CGNAT, unspecified,
/// multicast, documentation, benchmarking, reserved, and the IPv6 forms that carry an IPv4 address inside (mapped, NAT64, 6to4,
/// compatible) judged by the address inside.
pub fn blocked_address(ip: IpAddr) -> bool {
    fn v4(a: Ipv4Addr) -> bool {
        let o = a.octets();
        a.is_unspecified()
            || a.is_loopback()
            || a.is_private()
            || a.is_link_local()
            || a.is_broadcast()
            || a.is_multicast()
            || a.is_documentation()
            || o[0] == 0
            || (o[0] == 192 && o[1] == 88 && o[2] == 99)
            || (o[0] == 100 && (o[1] & 0xc0) == 64)
            || (o[0] == 192 && o[1] == 0 && o[2] == 0)
            || (o[0] == 198 && (o[1] & 0xfe) == 18)
            || o[0] >= 240
    }
    match ip {
        IpAddr::V4(a) => v4(a),
        IpAddr::V6(a) => {
            if let Some(m) = a.to_ipv4_mapped() {
                return v4(m);
            }
            let s = a.segments();
            a.is_unspecified()
                || a.is_loopback()
                || a.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xffc0) == 0xfe80
                || (s[0] & 0xffc0) == 0xfec0
                || (s[0] == 0x2001 && (s[1] == 0x0db8 || s[1] == 0 || (s[1] == 2 && s[2] == 0)))
                || (s[0] == 0x100 && s[1..4].iter().all(|x| *x == 0))
                || (s[0] == 0x64
                    && s[1] == 0xff9b
                    && (s[2..6].iter().any(|x| *x != 0) || {
                        let [a, b] = [s[6].to_be_bytes(), s[7].to_be_bytes()];
                        v4(Ipv4Addr::new(a[0], a[1], b[0], b[1]))
                    }))
                || (s[0] == 0x2002 && {
                    let [a, b] = [s[1].to_be_bytes(), s[2].to_be_bytes()];
                    v4(Ipv4Addr::new(a[0], a[1], b[0], b[1]))
                })
                || s[..6].iter().all(|x| *x == 0)
        }
    }
}

/// Host and port of an `https://` address, checked again here (the list's syntax check is not trusted to be the only one).
fn split_url(url: &str) -> Result<(String, u16), InstallError> {
    let bad = || err(Code::DownloadFailed, "bad_url");
    let Some(rest) = url.strip_prefix("https://") else {
        return bad();
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains(['@', '[', ']', ' ', '\\']) {
        return bad();
    }
    let (host, port) = match authority.split_once(':') {
        None => (authority, 443),
        Some((h, p)) => match p.parse::<u16>() {
            Ok(p) if p != 0 => (h, p),
            _ => return bad(),
        },
    };
    let host = host.to_ascii_lowercase();
    // A name, never an address literal: the list refuses those too.
    if host.parse::<IpAddr>().is_ok() || host.is_empty() || !host.contains('.') {
        return err(Code::DownloadFailed, "ip_literal_or_bad_host");
    }
    Ok((host, port))
}

/// Counts and hashes what it writes, stops at `max`, and remembers why it stopped.
struct Sink<W: Write> {
    inner: W,
    hasher: Sha256,
    count: u64,
    max: u64,
    over: bool,
    failure: Option<io::Error>,
}

impl<W: Write> Write for Sink<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.count + buf.len() as u64 > self.max {
            self.over = true;
            return Err(io::Error::other("too large"));
        }
        match self.inner.write(buf) {
            Ok(n) => {
                self.hasher.update(&buf[..n]);
                self.count += n as u64;
                Ok(n)
            }
            Err(e) => {
                self.failure = Some(io::Error::new(e.kind(), "write"));
                Err(e)
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Downloads `url` into `out`: the host must resolve, every address it resolves to must pass [`blocked_address`], and the request
/// goes to the first of them. Returns the byte count and the SHA-256 (lower-case hex).
fn download(
    url: &str,
    tools: &Tools<'_>,
    max: u64,
    out: File,
) -> Result<(u64, String), InstallError> {
    let (host, port) = split_url(url)?;
    let addrs = tools
        .resolver
        .resolve(&host, port)
        .map_err(|_| InstallError {
            code: Code::DownloadFailed,
            detail: "resolve".into(),
        })?;
    if addrs.is_empty() {
        return err(Code::DownloadFailed, "resolve");
    }
    // One private answer among public ones refuses the whole name: a resolver that mixes them is how rebinding starts.
    if addrs.iter().any(|a| blocked_address(*a)) {
        return err(Code::DownloadFailed, "blocked_address");
    }
    let mut sink = Sink {
        inner: out,
        hasher: Sha256::new(),
        count: 0,
        max,
        over: false,
        failure: None,
    };
    let request = FetchRequest {
        url,
        host: &host,
        port,
        addr: addrs[0],
        max_bytes: max,
        timeout_secs: DOWNLOAD_SECS,
    };
    let result = tools.fetcher.fetch(&request, &mut sink);
    if sink.over {
        return err(Code::DownloadFailed, "too_large");
    }
    if let Some(e) = &sink.failure {
        return Err(io_err(e));
    }
    result?;
    sink.inner.sync_all().map_err(|e| io_err(&e))?;
    let sum = sink.hasher.finalize();
    Ok((sink.count, hex(sum.as_ref())))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ---- the strict tools ----------------------------------------------------------------------------------------------------

/// The arguments `curl` gets for a checked request. Ignores any `.curlrc` (`-q`, first), uses no proxy (a proxy would resolve the
/// name itself and defeat the pin), pins the checked address with `--resolve`, allows only https and no redirect at all (a 3xx is
/// a failure), and caps size and time. `%{stderr}%{http_code}` leaves the status where the body is not.
pub fn curl_args(r: &FetchRequest<'_>) -> Vec<String> {
    let addr = match r.addr {
        IpAddr::V4(a) => a.to_string(),
        IpAddr::V6(a) => format!("[{a}]"),
    };
    [
        "-q",
        "--globoff",
        "--silent",
        "--show-error",
        "--fail",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--max-redirs",
        "0",
        "--noproxy",
        "*",
        "--tlsv1.2",
        "--connect-timeout",
        "20",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .chain([
        "--max-filesize".to_string(),
        r.max_bytes.to_string(),
        "--max-time".to_string(),
        r.timeout_secs.to_string(),
        "--resolve".to_string(),
        format!("{}:{}:{addr}", r.host, r.port),
        "--write-out".to_string(),
        "%{stderr}%{http_code}".to_string(),
        "--url".to_string(),
        r.url.to_string(),
    ])
    .collect()
}

/// Downloads with a `curl` subprocess (as `agora update` does), with the arguments of [`curl_args`].
pub struct CurlFetcher;

impl Fetcher for CurlFetcher {
    fn fetch(&self, r: &FetchRequest<'_>, sink: &mut dyn Write) -> Result<(), InstallError> {
        let mut child = Command::new("curl")
            .args(curl_args(r))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                if e.kind() == io::ErrorKind::NotFound {
                    InstallError {
                        code: Code::ToolMissing,
                        detail: "curl".into(),
                    }
                } else {
                    InstallError {
                        code: Code::DownloadFailed,
                        detail: "spawn".into(),
                    }
                }
            })?;
        let mut stdout = child.stdout.take();
        let mut buf = vec![0u8; 64 * 1024];
        let mut stopped = false;
        while let Some(out) = stdout.as_mut() {
            match out.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if sink.write_all(&buf[..n]).is_err() {
                        stopped = true;
                        break;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => {
                    stopped = true;
                    break;
                }
            }
        }
        if stopped {
            let _ = child.kill();
            let _ = child.wait();
            // The sink knows why it refused; this error is only seen when it does not say.
            return err(Code::DownloadFailed, "stopped");
        }
        let mut diag = String::new();
        if let Some(e) = child.stderr.take() {
            let _ = e.take(8192).read_to_string(&mut diag);
        }
        let status = child.wait().map_err(|_| InstallError {
            code: Code::DownloadFailed,
            detail: "wait".into(),
        })?;
        match status.code() {
            Some(0) if diag.trim_end().ends_with("200") => Ok(()),
            Some(0) => err(Code::DownloadFailed, "http_status"),
            Some(28) => err(Code::Timeout, "download"),
            Some(63) => err(Code::DownloadFailed, "too_large"),
            _ => err(Code::DownloadFailed, "curl"),
        }
    }
}

/// Checks the detached signature with `ssh-keygen -Y verify` (as `agora update` does). `key_file` is in `allowed signers` format
/// and names the principal `agora-agent`.
pub struct SshKeygen;

impl SignatureCheck for SshKeygen {
    fn check(&self, key_file: &Path, signed: &Path, signature: &Path) -> Result<(), InstallError> {
        let input = File::open(signed).map_err(|e| io_err(&e))?;
        let status = Command::new("ssh-keygen")
            .args(["-Y", "verify", "-f"])
            .arg(key_file)
            .args(["-I", "agora-agent", "-n", "file", "-s"])
            .arg(signature)
            .stdin(input)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| {
                if e.kind() == io::ErrorKind::NotFound {
                    InstallError {
                        code: Code::ToolMissing,
                        detail: "ssh-keygen".into(),
                    }
                } else {
                    InstallError {
                        code: Code::SignatureInvalid,
                        detail: "spawn".into(),
                    }
                }
            })?;
        if status.success() {
            Ok(())
        } else {
            err(Code::SignatureInvalid, "bad_signature")
        }
    }
}

// ---- finding what was installed -------------------------------------------------------------------------------------------

/// A program `agora agent install` put under `<AGORA_HOME>/agents/bin`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// The real file to start (the version folder's program on Unix, the launcher copy on Windows).
    pub path: PathBuf,
    /// The agent that owns the launcher and the version `current` names.
    pub name: String,
    pub version: String,
}

/// What the installer's own records say about a program name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    /// No agent folder owns this name (or it is not a plain file name).
    NotInstalled,
    /// Owned and every check passed.
    Installed(Installed),
    /// An agent folder owns it, but a check failed (a link, a folder or file others can write, a launcher that is not the
    /// installer's). It is not used.
    Unsafe,
}

/// The launcher file names the installer may have written for a manifest's `program`: the name itself and, on Windows, the name
/// with `.exe` (the installer writes `claude.exe` where the manifest launches `claude`). `windows` is a parameter so the mapping is
/// testable everywhere.
pub fn launcher_names(program: &str, windows: bool) -> Vec<String> {
    let mut names = vec![program.to_string()];
    if windows && Path::new(program).extension().is_none() {
        names.push(format!("{program}.exe"));
    }
    names
}

fn plain_name(program: &str) -> bool {
    let mut parts = Path::new(program).components();
    matches!(
        (parts.next(), parts.next()),
        (Some(std::path::Component::Normal(n)), None) if n == OsStr::new(program)
    ) && !program.is_empty()
}

/// ADR-0087 item 3: the program the installer made for `program`. See [`Lookup`]. Accepted only for a plain file name that an agent
/// folder's `exe` file owns, whose launcher in `<home>/agents/bin` is exactly what the installer writes (Unix: a link that resolves to
/// `agents/<name>/<current version>/<program>`; Windows: a regular file there), and where `agents`, `bin`, the version folder and the
/// file are real (not links), this user's, and not writable by group or others. Only reads the file system.
pub fn lookup_installed(home: &Path, program: &str) -> Lookup {
    if !plain_name(program) {
        return Lookup::NotInstalled;
    }
    let agents = agents_dir(home);
    let Some((exe, name)) = launcher_names(program, cfg!(windows))
        .into_iter()
        .find_map(|n| owner_of(&agents, &n).map(|o| (n, o)))
    else {
        return Lookup::NotInstalled;
    };
    match checked(home, &agents, &exe, name) {
        Some(i) => Lookup::Installed(i),
        None => Lookup::Unsafe,
    }
}

fn checked(home: &Path, agents: &Path, exe: &str, name: String) -> Option<Installed> {
    // A real folder of this user's that others cannot write to (`symlink_metadata`: a link is not a folder here).
    let safe_dir = |p: &Path| {
        std::fs::symlink_metadata(p)
            .is_ok_and(|m| m.is_dir() && !agora_platform::others_can_write(&m))
    };
    let bin = agents.join("bin");
    if !safe_dir(agents) || !safe_dir(&bin) {
        return None;
    }
    let canon_agents = std::fs::canonicalize(agents).ok()?;
    let version = installed_version(home, &name)?;
    let launcher = bin.join(exe);
    let linked = std::fs::symlink_metadata(&launcher).ok()?;
    let wanted = if linked.is_symlink() {
        if !safe_dir(&agents.join(&name).join(&version)) {
            return None;
        }
        canon_agents.join(&name).join(&version).join(exe)
    } else if linked.is_file() {
        canon_agents.join("bin").join(exe)
    } else {
        return None;
    };
    let path = std::fs::canonicalize(&launcher).ok()?;
    if path != wanted {
        return None;
    }
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() || agora_platform::others_can_write(&meta) {
        return None;
    }
    Some(Installed {
        path,
        name,
        version,
    })
}

/// [`lookup_installed`], keeping only an accepted program.
pub fn installed_program(home: &Path, program: &str) -> Option<Installed> {
    match lookup_installed(home, program) {
        Lookup::Installed(i) => Some(i),
        _ => None,
    }
}

// ---- names ---------------------------------------------------------------------------------------------------------------

/// The agent that owns the launcher `exe`, from the `exe` file each agent folder keeps (written when its launcher moved).
fn owner_of(agents: &Path, exe: &str) -> Option<String> {
    for e in std::fs::read_dir(agents).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if n.starts_with('.') || n == "bin" || !e.path().is_dir() {
            continue;
        }
        let owned = std::fs::read_to_string(e.path().join("exe")).unwrap_or_default();
        if owned.trim() == exe {
            return Some(n);
        }
    }
    None
}

/// `bin` is the launcher folder, not an agent; and a launcher name another agent already owns is never taken over.
fn check_names(home: &Path, name: &str, exe: &str) -> Result<(), InstallError> {
    if name == "bin" {
        return err(Code::Conflict, "reserved_name");
    }
    match owner_of(&agents_dir(home), exe) {
        Some(other) if other != name => err(Code::Conflict, "executable_owned"),
        _ => Ok(()),
    }
}

// ---- last seen list version ----------------------------------------------------------------------------------------------

/// The largest `list-state.json` that is read: it holds one number (`{"version":N}`); anything larger was not written by Agora.
pub const MAX_LIST_STATE_BYTES: u64 = 4096;

/// The detail of the refusal when `list-state.json` is there but cannot be read or understood (#923).
pub const LIST_STATE_UNREADABLE: &str = "list_state_unreadable";

/// `<AGORA_HOME>/agents/list-state.json`.
pub fn list_state_path(home: &Path) -> PathBuf {
    agents_dir(home).join(LIST_STATE_NAME)
}

/// The newest list version this machine accepted (`<AGORA_HOME>/agents/list-state.json`): 0 only when the file does not exist (no
/// list accepted yet). A file that is there but cannot be read or understood (no permission, a link or anything but a plain file,
/// larger than [`MAX_LIST_STATE_BYTES`], not `{"version":N}`) is `agent.install.index_invalid` with the detail
/// [`LIST_STATE_UNREADABLE`], never 0: an unknown version would let an older list through (#923). The file is opened without
/// following a link at its name and checked on the opened handle, so a link swapped in after any earlier look is refused, not read.
/// This look is a single read by path; [`remember_list_version`] reads the record again, and replaces it, through the one held
/// `agents` folder (#1000). It is only ever replaced by a rename, so a crash leaves the old one whole; the owner recovers from a
/// damaged one by removing it (docs/specs/agent-packages.md, rung 3).
pub fn last_seen_version(home: &Path) -> Result<u64, InstallError> {
    read_list_state(agora_platform::open_read_no_follow(&list_state_path(home)))
}

/// The record `list-state.json` in the held `agents` folder, read through that handle (see [`last_seen_version`]).
fn read_held_list_state(agents: &ConfinedDir) -> Result<u64, InstallError> {
    read_list_state(agents.open_read(OsStr::new(LIST_STATE_NAME)))
}

const LIST_STATE_NAME: &str = "list-state.json";

/// The version in an opened `list-state.json` (`opened` is the open's result): see [`last_seen_version`].
fn read_list_state(opened: io::Result<File>) -> Result<u64, InstallError> {
    let unreadable = || InstallError {
        code: Code::IndexInvalid,
        detail: LIST_STATE_UNREADABLE.into(),
    };
    let file = match opened {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(_) => return Err(unreadable()),
    };
    if !file.metadata().is_ok_and(|m| m.is_file()) {
        return Err(unreadable());
    }
    // One byte past the cap is read, so a larger file is refused, never read into memory or cut.
    let mut bytes = Vec::new();
    file.take(MAX_LIST_STATE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| unreadable())?;
    if bytes.len() as u64 > MAX_LIST_STATE_BYTES {
        return Err(unreadable());
    }
    // Only the documented object (a derived reader alone would also take a positional array, `[5]`, which no writer emits), then
    // the typed reader, which refuses a repeated `version` where a generic value keeps the last one (Codex's review of #997).
    let is_object =
        serde_json::from_slice::<serde_json::Value>(&bytes).is_ok_and(|v| v.is_object());
    if !is_object {
        return Err(unreadable());
    }
    serde_json::from_slice::<ListState>(&bytes)
        .map(|s| s.version)
        .map_err(|_| unreadable())
}

#[derive(serde::Deserialize)]
struct ListState {
    version: u64,
}

fn rolled_back(seen: u64, version: u64) -> Result<(), InstallError> {
    if version < seen {
        return err(Code::IndexInvalid, "rollback");
    }
    Ok(())
}

/// Records `version`, the list just opened, as the newest accepted, atomically, under the install lock (two processes cannot
/// interleave a read and a write). The record is read again here, so a list that was checked against an older record is refused
/// when a newer one was recorded meanwhile (`agent.install.index_invalid`, detail `rollback`); the same version is already recorded
/// and is accepted. A record that cannot be read is never replaced (#923). The lock, both reads and the replace go through the one
/// `agents` folder held open ([`ConfinedDir`]), never by path, so they all address the same folder even if another is put at its
/// path meanwhile (#1000).
pub fn remember_list_version(home: &Path, version: u64) -> Result<(), InstallError> {
    remember_list_version_with(home, version, &|| {})
}

/// [`remember_list_version`], with `opened` run once the `agents` folder is held, before anything in it is used (tests change the
/// folders there).
fn remember_list_version_with(
    home: &Path,
    version: u64,
    opened: &dyn Fn(),
) -> Result<(), InstallError> {
    let (root, agents) = open_agents(home)?;
    opened();
    let seen = read_held_list_state(&root)?;
    rolled_back(seen, version)?;
    if version == seen {
        return Ok(());
    }
    let _lock = Lock::acquire(&root, &agents)?;
    let seen = read_held_list_state(&root)?;
    rolled_back(seen, version)?;
    if version == seen {
        return Ok(());
    }
    replace_held_list_state(&root, version).map_err(|e| io_err(&e))
}

/// Writes `{"version":N}` beside the record in the held folder (a new file, private from its creation: `0600` on Unix; on Windows it
/// takes the owner-only ACL the `agents` folder passes on) and renames it over the record, so a crash leaves the old one whole.
fn replace_held_list_state(agents: &ConfinedDir, version: u64) -> io::Result<()> {
    let tmp = std::ffi::OsString::from(format!("{LIST_STATE_NAME}.tmp-{}", std::process::id()));
    // One left by a crashed run with this pid (the install lock is held, so no live run uses it).
    match agents.remove_file(&tmp) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let mut file = agents.create_new(&tmp, 0o600)?;
    let result = file
        .write_all(format!("{{\"version\":{version}}}\n").as_bytes())
        .and_then(|()| file.sync_all())
        .and_then(|()| agents.rename_over(&file, &tmp, OsStr::new(LIST_STATE_NAME)));
    if result.is_err() {
        let _ = agents.remove_owned(&file, &tmp);
        return result;
    }
    // The rename itself survives a power loss only once the folder is on disk; failing that does not undo the write.
    let _ = agents.sync();
    Ok(())
}

/// Makes `<home>/agents` (private) and holds it open. The folder itself must not be a link or junction: it is refused, not followed
/// (what is below it is opened through the handle).
fn open_agents(home: &Path) -> Result<(ConfinedDir, PathBuf), InstallError> {
    let agents = agents_dir(home);
    if let Ok(m) = std::fs::symlink_metadata(&agents) {
        if m.file_type().is_symlink() || agora_platform::is_reparse_point(&m) || !m.is_dir() {
            return err(Code::IoFailed, "not_confined");
        }
    }
    std::fs::create_dir_all(&agents)
        .and_then(|()| agora_platform::restrict_to_owner(&agents))
        .map_err(|e| io_err(&e))?;
    let root = ConfinedDir::open_root(&agents).map_err(|e| io_err(&e))?;
    Ok((root, agents))
}

// ---- the install ---------------------------------------------------------------------------------------------------------

/// What an install did.
#[derive(Debug, PartialEq, Eq)]
pub struct Outcome {
    pub version: String,
    pub previous: Option<String>,
    pub dir: PathBuf,
    pub launcher: PathBuf,
    /// The plan said the same version is installed: nothing was changed.
    pub unchanged: bool,
}

/// The install lock (audit F100): `<agents>/.lock`, held with the system's file lock (`File::try_lock`), which the system releases
/// when its owner ends, so a crash never leaves it held and nothing is taken over. While held it names its owner (`pid started boot`)
/// for versions before this one, which judge the lock by that text alone; a lock such a version holds (a live owner named in the file)
/// is respected. It is emptied on release and never removed: a file removed while another process has it open and is about to lock
/// it would let two processes hold locks on two different files. On Windows such a version cannot read a held lock (the system lock
/// is mandatory there) and judges it by its age, but the file is open without delete sharing, so it can neither rename nor remove it.
struct Lock {
    file: File,
}

/// How long [`Lock::acquire`] waits before an install is `in_progress`. A process started by another thread of this process holds a
/// copy of the lock's handle from its fork to its exec (the handle is close-on-exec), and the system counts that copy as holding the
/// lock: the wait outlasts such a copy, not another install.
const LOCK_WAIT: Duration = Duration::from_secs(1);

enum Holder {
    Alive,
    Gone,
    /// Empty or not understood.
    Unreadable,
}

fn own_text() -> String {
    let pid = std::process::id();
    match agora_platform::process_identity(pid) {
        Ok(i) => format!("{pid} {} {}", i.started, i.boot),
        Err(_) => pid.to_string(),
    }
}

fn holder(text: &str) -> Holder {
    let mut f = text.split_whitespace();
    let Some(pid) = f.next().and_then(|p| p.parse::<u32>().ok()) else {
        return Holder::Unreadable;
    };
    let started = f.next().and_then(|s| s.parse::<u64>().ok());
    let boot = f.next().unwrap_or("");
    match (started, agora_platform::process_identity(pid)) {
        // The same pid with another start time or boot is a different process: the owner is gone.
        (Some(s), Ok(now)) => {
            if now.started == s && now.boot == boot {
                Holder::Alive
            } else {
                Holder::Gone
            }
        }
        (Some(_), Err(e)) if e.kind() == io::ErrorKind::NotFound => Holder::Gone,
        _ => {
            if agora_platform::process_exists(pid) {
                Holder::Alive
            } else {
                Holder::Gone
            }
        }
    }
}

impl Lock {
    /// Takes the lock in the held `agents` folder (`root`, whose path is `agents`; the lock file is opened through the handle, #1000).
    fn acquire(root: &ConfinedDir, agents: &Path) -> Result<Self, InstallError> {
        let mut file =
            open_lock(agents, &|| root.open_lock(OsStr::new(".lock"))).map_err(|e| io_err(&e))?;
        let end = Instant::now() + LOCK_WAIT;
        loop {
            match file.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock) if Instant::now() < end => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(std::fs::TryLockError::WouldBlock) => return err(Code::InProgress, "locked"),
                Err(std::fs::TryLockError::Error(e)) => return Err(io_err(&e)),
            }
        }
        // What is left in the file is a crashed owner's (gone) or the lock of a version without the system lock (alive).
        let mut seen = Vec::new();
        file.read_to_end(&mut seen).map_err(|e| io_err(&e))?;
        if matches!(holder(&String::from_utf8_lossy(&seen)), Holder::Alive) {
            return err(Code::InProgress, "locked");
        }
        file.set_len(0)
            .and_then(|()| file.seek(io::SeekFrom::Start(0)))
            .and_then(|_| file.write_all(own_text().as_bytes()))
            .map_err(|e| io_err(&e))?;
        Ok(Self { file })
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        // Emptied while still held; the system lock goes with the handle.
        let _ = self.file.set_len(0);
    }
}

/// Opens `<agents>/.lock` with `open` (the private open, through the held folder). An older version that ended between putting a lock back (`hard_link`) and
/// removing the name it had moved it to left `.lock` with a second name, which the open refuses (`InvalidInput`). That name is only
/// ever a `.lock.stale-*` one: those go and the open is tried once more (review of audit F100), also when this caller removed
/// nothing, since another caller may have removed the name first (Codex's re-review of #934). A second name of any other kind is
/// still refused. Opening is not holding: the system lock taken afterwards still admits one holder.
fn open_lock(agents: &Path, open: &dyn Fn() -> io::Result<File>) -> io::Result<File> {
    match open() {
        Err(e) if e.kind() == io::ErrorKind::InvalidInput => {
            remove_stale_lock_names(agents);
            open()
        }
        r => r,
    }
}

/// Removes the `.lock.stale-*` names older versions left in the agents folder.
fn remove_stale_lock_names(agents: &Path) {
    for e in std::fs::read_dir(agents).into_iter().flatten().flatten() {
        if e.file_name().to_string_lossy().starts_with(".lock.stale-")
            && std::fs::symlink_metadata(e.path()).is_ok_and(|m| m.is_file())
        {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Removes what a crashed install left: `.tmp-*` and `.lock.stale-*` in the agents folder and half-made launchers in `bin`.
fn clean_leftovers(agents: &Path) {
    let remove = |path: &Path| {
        match std::fs::symlink_metadata(path) {
            Ok(m) if m.is_dir() => {
                let _ = std::fs::remove_dir_all(path);
            }
            Ok(_) => {
                let _ = std::fs::remove_file(path);
            }
            Err(_) => {}
        };
    };
    for e in std::fs::read_dir(agents).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if n.starts_with(".tmp-") || n.starts_with(".lock.stale-") {
            remove(&e.path());
        }
    }
    for e in std::fs::read_dir(agents.join("bin"))
        .into_iter()
        .flatten()
        .flatten()
    {
        if e.file_name().to_string_lossy().contains(".tmp-") {
            remove(&e.path());
        }
    }
}

struct TempDir {
    path: PathBuf,
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Whether the launcher in `bin` resolves into the folder `<name>/<version>` (Unix: its link names it; Windows launchers are
/// copies, which nothing in the folder backs, so a folder there is never in use through the launcher).
fn launcher_uses(launcher: &Path, name: &str, version: &str) -> bool {
    let Ok(target) = std::fs::read_link(launcher) else {
        return false;
    };
    let mut parts = target.components().rev().skip(1);
    let v = parts.next().map(|c| c.as_os_str().to_owned());
    let n = parts.next().map(|c| c.as_os_str().to_owned());
    v.as_deref() == Some(OsStr::new(version)) && n.as_deref() == Some(OsStr::new(name))
}

/// After a failed switch, removes a launcher under a name the previous version did not have: the one this attempt `created` (a link,
/// or a copy where links are not made, whose version `launcher_uses` cannot read), or a link into the new version.
fn remove_new_launcher(launcher: &Path, name: &str, version: &str, created: bool) {
    if created || launcher_uses(launcher, name, version) {
        let _ = std::fs::remove_file(launcher);
    }
}

/// Removes a version folder unless the launcher resolves into it.
fn remove_version(launcher: &Path, name: &str, version: &str, path: &Path) {
    if !launcher_uses(launcher, name, version) {
        let _ = std::fs::remove_dir_all(path);
    }
}

/// Installs the planned program. `home` is `<AGORA_HOME>`.
///
/// In order: lock (one install per machine), remove leftovers of a crash, download into a temporary folder (address rules,
/// size cap, SHA-256 equal to the entry's), the detached signature when the entry names a key (the key file must be in
/// `<home>/agent-keys/<key>.pub`), move the folder to `<name>/<version>`, run `verify` (10 seconds, scrubbed environment) and
/// check the program exists, then move the launcher in `bin`, write `current` and the `exe` owner file, then drop all but the new
/// and the previous version. A failure before the launcher moves removes the new folder and leaves the launcher untouched. A
/// failure after it puts the previous launcher back (or removes the new one when there was none) before the new folder is
/// removed; a folder the launcher resolves into is never removed.
pub fn install(
    plan: &Plan,
    _approval: &Approval,
    home: &Path,
    tools: &Tools<'_>,
) -> Result<Outcome, InstallError> {
    if !valid_name(&plan.name) || !valid_component_text(&plan.version) {
        return err(Code::IoFailed, "bad_name");
    }
    let key_file = match &plan.signature {
        None => None,
        Some((_, key)) => {
            let f = home.join("agent-keys").join(format!("{key}.pub"));
            if !std::fs::metadata(&f).is_ok_and(|m| m.is_file()) {
                return err(Code::SignatureInvalid, "key_missing");
            }
            Some(f)
        }
    };
    let (root, agents) = open_agents(home)?;
    let _lock = Lock::acquire(&root, &agents)?;
    // What the plan saw may have changed while waiting for the lock.
    let installed = installed_version(home, &plan.name);
    if installed.as_deref() == Some(plan.version.as_str()) {
        return Ok(Outcome {
            version: plan.version.clone(),
            previous: installed,
            dir: plan.target_dir.clone(),
            launcher: agents.join("bin").join(&plan.exe),
            unchanged: true,
        });
    }
    check_names(home, &plan.name, &plan.exe)?;
    clean_leftovers(&agents);

    let tmp_name = format!(".tmp-{}", std::process::id());
    let tmp = TempDir {
        path: agents.join(&tmp_name),
    };
    let tmp_dir = root
        .subdir(OsStr::new(&tmp_name), true)
        .map_err(|e| io_err(&e))?;
    let stage_dir = tmp_dir
        .subdir(OsStr::new("stage"), true)
        .map_err(|e| io_err(&e))?;
    let stage = tmp.path.join("stage");
    agora_platform::restrict_to_owner(&tmp.path).map_err(|e| io_err(&e))?;
    agora_platform::restrict_to_owner(&stage).map_err(|e| io_err(&e))?;

    // Download and check.
    let program = stage_dir
        .create_new(OsStr::new(&plan.exe), 0o700)
        .map_err(|e| io_err(&e))?;
    let (count, sum) = download(&plan.url, tools, tools.max_download, program)?;
    if count == 0 || sum != plan.sha256 {
        return err(Code::ChecksumMismatch, "sha256");
    }
    if let (Some((sig_url, _)), Some(key_file)) = (&plan.signature, &key_file) {
        let sig = tmp_dir
            .create_new(OsStr::new("download.sig"), 0o600)
            .map_err(|e| io_err(&e))?;
        download(
            sig_url,
            tools,
            MAX_SIGNATURE_BYTES.min(tools.max_download),
            sig,
        )?;
        tools.signatures.check(
            key_file,
            &stage.join(&plan.exe),
            &tmp.path.join("download.sig"),
        )?;
    }
    agora_platform::set_executable(&stage.join(&plan.exe), true).map_err(|e| io_err(&e))?;

    // Put the folder where it will live (not yet linked).
    let name_dir = root
        .subdir(OsStr::new(&plan.name), true)
        .map_err(|e| io_err(&e))?;
    let name_path = agents.join(&plan.name);
    agora_platform::restrict_to_owner(&name_path).map_err(|e| io_err(&e))?;
    let version_path = name_path.join(&plan.version);
    let bin = agents.join("bin");
    let launcher = bin.join(&plan.exe);
    match name_dir
        .entry(OsStr::new(&plan.version))
        .map_err(|e| io_err(&e))?
    {
        None => {}
        // A folder of that version that nothing uses (an earlier failure, a version that was rolled back) is replaced; one the
        // launcher resolves into is live and is left alone.
        Some(e) if e.kind == EntryKind::Folder => {
            if launcher_uses(&launcher, &plan.name, &plan.version) {
                return err(Code::IoFailed, "in_use");
            }
            std::fs::remove_dir_all(&version_path).map_err(|e| io_err(&e))?;
        }
        Some(_) => name_dir
            .remove_file(OsStr::new(&plan.version))
            .map_err(|e| io_err(&e))?,
    }
    std::fs::rename(&stage, &version_path).map_err(|e| io_err(&e))?;

    // Verify before anything live changes.
    let checked = (|| {
        if !std::fs::metadata(version_path.join(&plan.exe)).is_ok_and(|m| m.is_file()) {
            return err(Code::VerifyFailed, "program_missing");
        }
        run_verify(
            &version_path.join(&plan.exe),
            &plan.verify,
            &version_path,
            tools.verify_timeout,
        )
    })();
    if let Err(e) = checked {
        // The launcher was not touched and still resolves to what it did.
        remove_version(&launcher, &plan.name, &plan.version, &version_path);
        return Err(e);
    }

    let old_target = std::fs::read_link(&launcher).ok();
    // Whether this attempt makes the launcher where there was none (a link, or a copy on Windows, which `launcher_uses` cannot read).
    let launcher_was_there = std::fs::symlink_metadata(&launcher).is_ok();
    let mut created = false;
    // The records as they were, put back exactly if the switch fails (Codex review of #1027: the `exe` record too, so a renamed
    // program's new name is not left owned by a version that was rolled back).
    let (exe_was, current_was) = (
        Was::of(&name_path.join("exe")),
        Was::of(&name_path.join("current")),
    );
    let same_exe =
        matches!(&exe_was, Was::Held(b) if String::from_utf8_lossy(b).trim() == plan.exe);
    // Switch, then record. Any failure puts the launcher back before the new folder goes.
    let switched = (|| -> Result<(), InstallError> {
        root.subdir(OsStr::new("bin"), true)
            .map_err(|e| io_err(&e))?;
        agora_platform::restrict_to_owner(&bin).map_err(|e| io_err(&e))?;
        link(plan, &plan.version, &version_path, &launcher)?;
        created = !launcher_was_there;
        if tools.fault == Some(Fault::BeforeCurrent) {
            return err(Code::IoFailed, "injected");
        }
        agora_platform::write_private(&name_path.join("exe"), plan.exe.as_bytes())
            .map_err(|e| io_err(&e))?;
        if tools.fault == Some(Fault::RecordsUnrestorable) {
            return err(Code::IoFailed, "injected");
        }
        agora_platform::write_private(&name_path.join("current"), plan.version.as_bytes())
            .map_err(|e| io_err(&e))
    })();
    if let Err(e) = switched {
        match (&old_target, installed.as_deref()) {
            // Exactly what the launcher pointed at before, even when `current` could not be read.
            (Some(t), _) => {
                let _ = agora_platform::install_launcher(&bin.join(t), t, &launcher);
            }
            // The previous version had this launcher name (its launcher went missing): it points there again.
            (None, Some(old)) if same_exe => {
                let _ = link(plan, old, &name_path.join(old), &launcher);
            }
            // A new launcher name (a first install, or an update that renamed the program): it goes; the old one was not touched.
            (None, _) => remove_new_launcher(&launcher, &plan.name, &plan.version, created),
        }
        let restored = exe_was.put_back(&name_path.join("exe"))
            & current_was.put_back(&name_path.join("current"))
            & (tools.fault != Some(Fault::RecordsUnrestorable));
        remove_version(&launcher, &plan.name, &plan.version, &version_path);
        if !restored {
            // Not a clean rollback: said as such, not as the write failure that started it.
            return err(Code::IoFailed, "rollback_incomplete");
        }
        return Err(e);
    }
    prune(
        &name_path,
        &launcher,
        &plan.name,
        &plan.version,
        installed.as_deref(),
    );
    Ok(Outcome {
        version: plan.version.clone(),
        previous: installed,
        dir: version_path,
        launcher,
        unchanged: false,
    })
}

fn link(
    plan: &Plan,
    version: &str,
    version_path: &Path,
    launcher: &Path,
) -> Result<(), InstallError> {
    let relative = Path::new("..")
        .join(&plan.name)
        .join(version)
        .join(&plan.exe);
    agora_platform::install_launcher(&version_path.join(&plan.exe), &relative, launcher)
        .map_err(|e| io_err(&e))
}

/// Keeps the new version and the one before it, removes every other version folder (only real folders, never a link, and never
/// the one the launcher resolves into).
fn prune(name_path: &Path, launcher: &Path, name: &str, new: &str, previous: Option<&str>) {
    for e in std::fs::read_dir(name_path).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if n.starts_with('.')
            || n == new
            || Some(n.as_str()) == previous
            || n == "current"
            || n == "exe"
        {
            continue;
        }
        if std::fs::symlink_metadata(e.path()).is_ok_and(|m| m.is_dir()) {
            remove_version(launcher, name, &n, &e.path());
        }
    }
}

/// Runs `program args...` with a 10 second limit, an empty environment but a fixed safe `PATH` and a home inside the version
/// folder (no token or secret of the caller reaches it), stdin closed, and matches the first 4096 bytes of its output.
fn run_verify(
    program: &Path,
    verify: &Verify,
    cwd: &Path,
    limit: Duration,
) -> Result<(), InstallError> {
    let mut cmd = Command::new(program);
    cmd.args(&verify.args)
        .current_dir(cwd)
        .env_clear()
        .env("PATH", scrubbed_path())
        .env("HOME", cwd)
        .env("USERPROFILE", cwd)
        .env("LANG", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(root) = std::env::var_os("SystemRoot") {
        // Windows programs do not start without it; it names the system folder, not a secret.
        cmd.env("SystemRoot", root);
    }
    // A program just written can be "busy" for a moment when another thread forked while its file was still open for writing.
    let mut tries = 0;
    let mut child = loop {
        match cmd.spawn() {
            Ok(c) => break c,
            Err(e) if e.kind() == io::ErrorKind::ExecutableFileBusy && tries < 40 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => {
                return err(Code::VerifyFailed, "cannot_run");
            }
        }
    };
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    for stream in [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let mut head = Vec::new();
            let mut s = stream;
            let _ = (&mut s).take(4096).read_to_end(&mut head);
            let _ = tx.send(head);
            // Keep reading (and dropping) so a chatty program is never blocked on a full pipe.
            let _ = io::copy(&mut s, &mut io::sink());
        });
    }
    drop(tx);
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return err(Code::Timeout, "verify");
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return err(Code::VerifyFailed, "wait"),
        }
    };
    let mut output = Vec::new();
    while let Ok(part) = rx.recv_timeout(Duration::from_millis(500)) {
        output.extend_from_slice(&part);
    }
    if !status.success() {
        return err(Code::VerifyFailed, "exit_status");
    }
    if verify.matches(&String::from_utf8_lossy(&output)) {
        Ok(())
    } else {
        err(Code::VerifyFailed, "no_match")
    }
}

fn scrubbed_path() -> String {
    agora_platform::safe_search_path()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod lock_open_tests {
    use super::*;

    /// Codex's re-review of #934: two callers both find `.lock` with a second name; the other one removes the `.lock.stale-*` name
    /// first, so this caller's cleanup removes nothing. It must still open the lock again rather than fail with its first refusal.
    #[test]
    fn a_caller_whose_cleanup_found_the_old_name_already_gone_still_opens_the_lock() {
        let agents = std::env::temp_dir().join(format!("agora-lock-open-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&agents);
        std::fs::create_dir_all(&agents).unwrap();
        // `.lock` has one name again (the other caller removed `.lock.stale-*`); this caller's first open had been refused.
        std::fs::write(agents.join(".lock"), "").unwrap();
        let first = std::cell::Cell::new(true);
        let open = || {
            if first.replace(false) {
                Err(io::Error::new(io::ErrorKind::InvalidInput, "a link"))
            } else {
                agora_platform::open_lock_private(&agents.join(".lock"))
            }
        };
        // The file is closed before the folder goes (Windows cannot remove a lock file while it is open).
        let opened = open_lock(&agents, &open).map(drop);
        let _ = std::fs::remove_dir_all(&agents);
        assert!(opened.is_ok(), "{opened:?}");
    }

    /// An I/O failure whose kind says nothing (a sharing violation on Windows) keeps the system's error number in its detail.
    #[test]
    fn an_io_failure_of_no_known_kind_keeps_its_error_number() {
        assert_eq!(
            io_detail(&io::Error::from_raw_os_error(999_999)),
            "uncategorized_999999"
        );
        assert_eq!(
            io_detail(&io::Error::from(io::ErrorKind::NotFound)),
            "notfound"
        );
        assert_eq!(io_detail(&io::Error::other("x")), "other");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod rollback_launcher_tests {
    use super::*;

    /// Codex review of #1027: a launcher this attempt made is removed by a rollback on every platform, a copy as well as a link.
    /// `launcher_uses` reads only links, so the old criterion left a copy (Windows) under a new name behind. Needs no shell scripts.
    #[test]
    fn a_launcher_copy_this_attempt_made_is_removed_and_one_that_was_there_is_kept() {
        let bin =
            std::env::temp_dir().join(format!("agora-rollback-launcher-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&bin);
        std::fs::create_dir_all(&bin).unwrap();
        let copy = bin.join("fake-agent-two");
        std::fs::write(&copy, b"a copy of the program").unwrap();
        // The old criterion cannot tell a copy's version.
        assert!(!launcher_uses(&copy, "fake-agent", "2.0.0"));
        remove_new_launcher(&copy, "fake-agent", "2.0.0", false);
        assert!(copy.exists(), "a copy this attempt did not make is kept");
        remove_new_launcher(&copy, "fake-agent", "2.0.0", true);
        assert!(!copy.exists(), "a copy this attempt made is removed");
        let _ = std::fs::remove_dir_all(&bin);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod list_state_handle_tests {
    use super::*;

    /// #1000: the lock, the read and the replace of `list-state.json` all go through the one `agents` folder held open, so a folder
    /// put in its place after it was opened is neither locked, read nor written. Here the real record says 2, an empty decoy `agents`
    /// replaces the folder once it is held, and list 3 is recorded: in the held (real) folder, under its lock, never in the decoy.
    #[test]
    fn the_record_is_read_and_replaced_in_the_held_folder_not_one_put_in_its_place() {
        let home =
            std::env::temp_dir().join(format!("agora-list-state-held-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        remember_list_version(&home, 2).unwrap();
        let agents = agents_dir(&home);
        let aside = home.join("agents-held");
        let swapped = std::cell::Cell::new(false);
        let swap = || {
            // A system that does not rename a folder held open: nothing to test there.
            if std::fs::rename(&agents, &aside).is_ok() {
                std::fs::create_dir(&agents).unwrap();
                swapped.set(true);
            }
        };
        let result = remember_list_version_with(&home, 3, &swap);
        if swapped.get() {
            let decoy = agents.join("list-state.json");
            let held = aside.join("list-state.json");
            let outcome = (
                result.map_err(|e| e.detail),
                decoy.exists(),
                agents.join(".lock").exists(),
                std::fs::read_to_string(&held).ok(),
                aside.join(".lock").exists(),
            );
            let _ = std::fs::remove_dir_all(&home);
            assert_eq!(
                outcome,
                (
                    Ok(()),
                    false,
                    false,
                    Some("{\"version\":3}\n".to_string()),
                    true
                )
            );
        } else {
            let _ = std::fs::remove_dir_all(&home);
        }
    }
}

//! A trusted reverse proxy in front of the hub (ADR-0067 rung 4: a tunnel provider, or any proxy the owner runs).
//!
//! Off unless the owner turns it on (`agora remote proxy`, which writes `<hub dir>/remote-proxy`). While it is on, and only for a TCP
//! peer that is loopback or an address the owner named, the hub
//! - takes the real client address from the one header the owner named, never from another and never from any other peer;
//! - never takes a proxied request for the owner at the hub's own machine, whatever the header says (a loopback "client" address,
//!   a missing or repeated header, a list of addresses: all are an unknown address, which is not local);
//! - serves only the names the owner gave (and loopback names): another `Host` is refused (421), and so is a browser `Origin` that
//!   names another host (403), which closes DNS rebinding and cross-site requests to a hub that is on the internet.
//!
//! Pure data and parsing: no sockets. The web layer asks [`ProxyTrust::judge`] for each request head.

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The file in the hub's folder that says the proxy is trusted.
pub const FILE: &str = "remote-proxy";
/// Most public names, and most extra proxy addresses.
pub const MAX_HOSTS: usize = 4;
pub const MAX_PROXIES: usize = 4;
const MAX_FILE: u64 = 4096;
/// How often a running hub looks at the file again.
const RELOAD: Duration = Duration::from_secs(2);

/// Header names that can never carry a client address: they decide how the hub itself reads the request.
const FORBIDDEN_HEADERS: [&str; 14] = [
    "host",
    "origin",
    "referer",
    "authorization",
    "cookie",
    "set-cookie",
    "content-length",
    "content-type",
    "transfer-encoding",
    "connection",
    "upgrade",
    "user-agent",
    "sec-websocket-key",
    "sec-websocket-protocol",
];

/// What the owner turned on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyTrust {
    /// The header (lower case) that carries the real client address.
    header: String,
    /// The public names the hub answers to, lower case.
    hosts: Vec<String>,
    /// Peers that are trusted as the proxy besides loopback.
    proxies: Vec<IpAddr>,
}

/// What the web layer does with a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Answer with this status and stable code, and go no further.
    Refuse { status: u16, code: &'static str },
    /// Not a proxied request (a browser on this machine by its loopback name): nothing changes.
    Direct,
    /// A proxied request: its client address as far as it is known (`None` is unknown, never local).
    Proxied(Option<IpAddr>),
}

/// A header name the owner may name: a plain token, not one the hub itself reads.
pub fn valid_header(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !name.starts_with('-')
        && !FORBIDDEN_HEADERS.contains(&name.to_ascii_lowercase().as_str())
}

/// A public name: letters, digits and hyphens in labels of 1 to 63, at least two labels, ASCII only, at most 253 bytes, one trailing
/// dot at most (removed), not an address and not a loopback name. Returned lower case; anything else is refused whole.
pub fn checked_host(name: &str) -> Option<String> {
    let name = name.strip_suffix('.').unwrap_or(name);
    if name.is_empty() || name.len() > 253 || !name.is_ascii() {
        return None;
    }
    let labels: Vec<&str> = name.split('.').collect();
    let plain = |l: &&str| {
        (1..=63).contains(&l.len())
            && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            && !l.starts_with('-')
            && !l.ends_with('-')
    };
    // A name whose last label is all digits is an address (or a trick that looks like one), not a name.
    let numeric_tld = labels
        .last()
        .is_some_and(|l| l.bytes().all(|b| b.is_ascii_digit()));
    if labels.len() < 2 || !labels.iter().all(plain) || numeric_tld {
        return None;
    }
    let lower = name.to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") {
        return None;
    }
    Some(lower)
}

/// An IPv4 address that arrived mapped into IPv6 (`::ffff:a.b.c.d`) is that IPv4 address: one spelling, so a listed proxy matches a
/// peer however the socket reports it.
pub fn canon(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        IpAddr::V4(_) => ip,
    }
}

fn is_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => {
            v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
        }
    }
}

/// The host of a `Host` header value or of an `Origin` authority, lower case, without port and brackets: `None` when it is not
/// well formed.
fn authority_host(authority: &str) -> Option<String> {
    let a = authority.trim();
    if a.is_empty() || a.len() > 300 || a.contains(['@', '/', ' ', '\\', '?', '#']) {
        return None;
    }
    let host = if let Some(rest) = a.strip_prefix('[') {
        let (h, tail) = rest.split_once(']')?;
        if !(tail.is_empty() || tail.strip_prefix(':').is_some_and(valid_port)) {
            return None;
        }
        h
    } else {
        match a.split_once(':') {
            Some((h, p)) if valid_port(p) => h,
            Some(_) => return None,
            None => a,
        }
    };
    let host = host.strip_suffix('.').unwrap_or(host);
    if host.is_empty() {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

fn valid_port(p: &str) -> bool {
    !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit())
}

/// The names of the machine itself.
fn loopback_name(host: &str) -> bool {
    host == "localhost"
        || host == "127.0.0.1"
        || host == "::1"
        || host.parse::<IpAddr>().is_ok_and(is_loopback)
}

impl ProxyTrust {
    /// A configuration, every part checked. `None` when a part is not valid, or when there is no name or no header.
    pub fn new(header: &str, hosts: &[String], proxies: &[IpAddr]) -> Option<Self> {
        if !valid_header(header) || hosts.is_empty() || hosts.len() > MAX_HOSTS {
            return None;
        }
        let mut names: Vec<String> = Vec::new();
        for h in hosts {
            let h = checked_host(h)?;
            if !names.contains(&h) {
                names.push(h);
            }
        }
        let mut addrs: Vec<IpAddr> = Vec::new();
        for p in proxies {
            let p = canon(*p);
            // An address that nothing connects from cannot be a proxy.
            if p.is_unspecified() || p.is_multicast() {
                return None;
            }
            if !addrs.contains(&p) {
                addrs.push(p);
            }
        }
        if addrs.len() > MAX_PROXIES {
            return None;
        }
        Some(Self {
            header: header.to_ascii_lowercase(),
            hosts: names,
            proxies: addrs,
        })
    }

    pub fn header(&self) -> &str {
        &self.header
    }

    pub fn hosts(&self) -> &[String] {
        &self.hosts
    }

    pub fn proxies(&self) -> &[IpAddr] {
        &self.proxies
    }

    /// The file's text.
    pub fn render(&self) -> String {
        let mut out = format!("header={}\n", self.header);
        for h in &self.hosts {
            out.push_str(&format!("host={h}\n"));
        }
        for p in &self.proxies {
            out.push_str(&format!("proxy={p}\n"));
        }
        out
    }

    /// Reads the file's text again, with every rule of [`ProxyTrust::new`]: a hand-edited file cannot put anything else in. Any line
    /// that is not `header=`, `host=` or `proxy=` makes the whole file damaged (`None`), never half used.
    pub fn parse(text: &str) -> Option<Self> {
        let (mut header, mut hosts, mut proxies) = (None, Vec::new(), Vec::new());
        // A file this long is not one the command wrote: damaged, never half read.
        if text.lines().count() > 32 {
            return None;
        }
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let (k, v) = line.split_once('=')?;
            match k {
                "header" if header.is_none() => header = Some(v.to_string()),
                "host" => hosts.push(v.to_string()),
                "proxy" => proxies.push(v.parse::<IpAddr>().ok()?),
                _ => return None,
            }
        }
        Self::new(&header?, &hosts, &proxies)
    }

    /// Whether a TCP peer may speak for others: loopback (a proxy on this machine) or an address the owner named. Any other peer is
    /// an ordinary client, and a header it sends means nothing.
    pub fn trusts_peer(&self, peer: IpAddr) -> bool {
        is_loopback(peer) || self.proxies.contains(&canon(peer))
    }

    fn host_allowed(&self, host: &str) -> bool {
        loopback_name(host) || self.hosts.iter().any(|h| h == host)
    }

    /// Judges a request head (request line and headers) that arrived from a trusted peer, see [`ProxyTrust::trusts_peer`].
    pub fn judge(&self, head: &str) -> Verdict {
        let headers: Vec<(String, &str)> = head
            .split("\r\n")
            .skip(1)
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim()))
            .collect();
        let values = |name: &str| -> Vec<&str> {
            headers
                .iter()
                .filter(|(k, _)| k == name)
                .map(|(_, v)| *v)
                .collect()
        };
        // Host: exactly one, one of ours. A missing, repeated or foreign one is refused.
        let host = match values("host").as_slice() {
            [one] => authority_host(one),
            _ => None,
        };
        let Some(host) = host.filter(|h| self.host_allowed(h)) else {
            return Verdict::Refuse {
                status: 421,
                code: "proxy.host_not_allowed",
            };
        };
        // Origin (a browser sends it on cross-site requests and on every WebSocket): its host must be one of ours as well. `null`,
        // a repeated one and anything malformed are refused. A client that sends none is not a browser.
        let mut origin_host: Option<String> = None;
        match values("origin").as_slice() {
            [] => {}
            [one] => {
                origin_host = one
                    .split_once("://")
                    .filter(|(scheme, _)| matches!(*scheme, "http" | "https"))
                    .and_then(|(_, rest)| authority_host(rest))
                    .filter(|h| self.host_allowed(h));
                if origin_host.is_none() {
                    return Verdict::Refuse {
                        status: 403,
                        code: "proxy.origin_not_allowed",
                    };
                }
            }
            _ => {
                return Verdict::Refuse {
                    status: 403,
                    code: "proxy.origin_not_allowed",
                }
            }
        }
        let carried = values(&self.header);
        match carried.as_slice() {
            [one] => Verdict::Proxied(real_address(one)),
            [] if !loopback_name(&host) => {
                // The proxy answers to a public name but sent no address: that is a proxy that is set up wrong, not the owner.
                Verdict::Proxied(None)
            }
            // A loopback `Host` with a public `Origin` is not the owner's own browser (that one's Origin is a loopback name): a proxy
            // that kept the target's Host and forgot the header, for a POST or a WebSocket. A bare navigation (GET) has no Origin and
            // stays the gap that T40 names.
            [] if origin_host.as_deref().is_some_and(|o| !loopback_name(o)) => {
                Verdict::Proxied(None)
            }
            [] => Verdict::Direct,
            // A repeated header: which one is real is unknown.
            _ => Verdict::Proxied(None),
        }
    }
}

/// One address, alone. A list, a name, a loopback or unspecified address is unknown (the header is the proxy's word, and even that
/// word must never make a visitor "local").
fn real_address(value: &str) -> Option<IpAddr> {
    let ip: IpAddr = value.trim().parse().ok()?;
    let mapped = match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4),
        IpAddr::V4(_) => None,
    };
    let ip = mapped.unwrap_or(ip);
    (!is_loopback(ip) && !ip.is_unspecified() && !ip.is_multicast()).then_some(ip)
}

/// What a file holds.
#[derive(Debug, PartialEq, Eq)]
pub enum Stored {
    Off,
    On(ProxyTrust),
    /// There is a file and it is not usable (not a regular file, too big, not valid).
    Damaged,
}

/// How the file is opened. The hub's command passes `agora_platform::open_read_no_follow` (never follows a link, never waits on a
/// FIFO); this crate has no platform code of its own.
pub type Opener = fn(&Path) -> std::io::Result<std::fs::File>;

/// [`read_from`] with a plain open (tests). A running hub uses [`read_from`] with the platform opener.
pub fn read_file(path: &Path) -> Stored {
    read_from(path, |p| std::fs::File::open(p))
}

/// Reads the file: a regular file only, at most 4 KiB. It is looked at without following a link first, opened with `open`, and the
/// opened handle must itself be a regular file (so a link or a FIFO swapped in between is refused, never waited on).
pub fn read_from(path: &Path, open: Opener) -> Stored {
    use std::io::Read;
    // Only a file that is not there is off. Any other failure (a path component that is not a folder, no permission, ...) is a file
    // that cannot be read: damaged, so a running hub keeps its last good settings and a starting one trusts nothing.
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Stored::Off,
        Err(_) => return Stored::Damaged,
    };
    if !meta.file_type().is_file() {
        return Stored::Damaged;
    }
    let Ok(file) = open(path) else {
        return Stored::Damaged;
    };
    if !file.metadata().is_ok_and(|m| m.is_file()) {
        return Stored::Damaged;
    }
    let mut bytes = Vec::new();
    if file.take(MAX_FILE + 1).read_to_end(&mut bytes).is_err() || bytes.len() as u64 > MAX_FILE {
        return Stored::Damaged;
    }
    match ProxyTrust::parse(&String::from_utf8_lossy(&bytes)) {
        Some(t) => Stored::On(t),
        None => Stored::Damaged,
    }
}

/// What a running hub consults for each request: the file, looked at again every few seconds, so `agora remote proxy` and
/// `agora remote off` take effect without a restart.
pub struct ProxyHandle {
    file: Option<(PathBuf, Opener)>,
    cache: Mutex<(Option<Instant>, Option<Arc<ProxyTrust>>)>,
    reload: Duration,
}

impl ProxyHandle {
    /// Follows `file`, opened with `open`.
    pub fn watch(file: PathBuf, open: Opener) -> Self {
        Self {
            file: Some((file, open)),
            cache: Mutex::new((None, None)),
            reload: RELOAD,
        }
    }

    /// Looks at the file at every call (tests).
    #[cfg(test)]
    fn always_fresh(mut self) -> Self {
        self.reload = Duration::ZERO;
        self
    }

    /// A fixed answer (tests).
    pub fn fixed(trust: Option<ProxyTrust>) -> Self {
        Self {
            file: None,
            cache: Mutex::new((Some(Instant::now()), trust.map(Arc::new))),
            reload: RELOAD,
        }
    }

    /// The proxy that is trusted now, if any. A file that turns damaged while the hub runs keeps the last good settings (so the Host
    /// and Origin checks do not switch off); a hub that starts with a damaged file trusts nothing. A removed file is off.
    pub fn current(&self) -> Option<Arc<ProxyTrust>> {
        let mut c = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(file) = &self.file {
            if c.0.is_none_or(|t| t.elapsed() >= self.reload) {
                match read_from(&file.0, file.1) {
                    Stored::On(t) => c.1 = Some(Arc::new(t)),
                    Stored::Off => c.1 = None,
                    Stored::Damaged => {}
                }
                c.0 = Some(Instant::now());
            }
        }
        c.1.clone()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn trust() -> ProxyTrust {
        ProxyTrust::new(
            "CF-Connecting-IP",
            &["Agora.Example.com".to_string()],
            &["10.1.2.3".parse().unwrap()],
        )
        .unwrap()
    }

    fn head(lines: &[&str]) -> String {
        format!("GET / HTTP/1.1\r\n{}\r\n\r\n", lines.join("\r\n"))
    }

    #[test]
    fn a_configuration_is_checked_whole_and_round_trips() {
        let t = trust();
        assert_eq!(t.header(), "cf-connecting-ip");
        assert_eq!(t.hosts(), ["agora.example.com"]);
        assert_eq!(ProxyTrust::parse(&t.render()), Some(t.clone()));
        for bad_header in ["", "host", "Origin", "a b", "x:y", "-x", "a\r\nb"] {
            assert!(!valid_header(bad_header), "{bad_header:?}");
        }
        for bad_host in [
            "",
            "localhost",
            "x.localhost",
            "single",
            "127.0.0.1",
            "1.2.3.4",
            "*.example.com",
            "a..b",
            "-a.example.com",
            "ex ample.com",
            "bücher.example.com",
            "a.example.com/",
            "a.example.com:8443",
        ] {
            assert_eq!(checked_host(bad_host), None, "{bad_host:?}");
        }
        assert!(
            ProxyTrust::new("x-ip", &[], &[]).is_none(),
            "a name is needed"
        );
        assert!(ProxyTrust::new("host", &["a.example.com".into()], &[]).is_none());
        assert!(ProxyTrust::new("x-ip", &["*.a.com".into()], &[]).is_none());
        // A file with anything else in it is damaged whole.
        for bad in [
            "header=x-ip\nhost=a.example.com\nextra=1\n",
            "header=x-ip\n",
            "host=a.example.com\n",
            "header=x-ip\nheader=y-ip\nhost=a.example.com\n",
            "header=x-ip\nhost=a.example.com\nproxy=not-an-ip\n",
            "header=x-ip\nhost=a.example.com\nhost\n",
        ] {
            assert_eq!(ProxyTrust::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn only_loopback_and_named_addresses_speak_for_others() {
        let t = trust();
        for yes in ["127.0.0.1", "::1", "::ffff:127.0.0.1", "10.1.2.3"] {
            assert!(t.trusts_peer(yes.parse().unwrap()), "{yes}");
        }
        for no in ["10.1.2.4", "192.168.1.5", "203.0.113.9", "::2"] {
            assert!(!t.trusts_peer(no.parse().unwrap()), "{no}");
        }
    }

    #[test]
    fn the_real_address_comes_from_the_one_header_alone() {
        let t = trust();
        let ok = head(&["Host: agora.example.com", "CF-Connecting-IP: 203.0.113.9"]);
        assert_eq!(
            t.judge(&ok),
            Verdict::Proxied(Some("203.0.113.9".parse().unwrap()))
        );
        // Another header never names the client, even a well known one.
        let other = head(&[
            "Host: agora.example.com",
            "X-Forwarded-For: 203.0.113.9",
            "X-Real-IP: 203.0.113.9",
        ]);
        assert_eq!(t.judge(&other), Verdict::Proxied(None));
        // IPv6, and an IPv4 mapped into one.
        let v6 = head(&["Host: agora.example.com", "cf-connecting-ip: 2001:db8::7"]);
        assert_eq!(
            t.judge(&v6),
            Verdict::Proxied(Some("2001:db8::7".parse().unwrap()))
        );
        let mapped = head(&[
            "Host: agora.example.com",
            "cf-connecting-ip: ::ffff:203.0.113.9",
        ]);
        assert_eq!(
            t.judge(&mapped),
            Verdict::Proxied(Some("203.0.113.9".parse().unwrap()))
        );
    }

    #[test]
    fn an_address_that_could_make_a_visitor_the_owner_is_unknown() {
        let t = trust();
        for value in [
            "127.0.0.1",
            "::1",
            "::ffff:127.0.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "",
            "garbage",
            "203.0.113.9, 127.0.0.1",
            "203.0.113.9,198.51.100.1",
            "203.0.113.9:443",
            "[2001:db8::1]",
        ] {
            let h = head(&[
                "Host: agora.example.com",
                &format!("CF-Connecting-IP: {value}"),
            ]);
            assert_eq!(t.judge(&h), Verdict::Proxied(None), "{value:?}");
        }
        let twice = head(&[
            "Host: agora.example.com",
            "CF-Connecting-IP: 203.0.113.9",
            "cf-connecting-ip: 198.51.100.1",
        ]);
        assert_eq!(t.judge(&twice), Verdict::Proxied(None));
    }

    /// What `judge` alone can say when the trusted header is missing: a public name is a proxy set up wrong (never the owner); a
    /// loopback name is the owner's own browser **or** a proxy that forwards to loopback with its default `Host` and forgot the
    /// header, and those two cannot be told apart from the request (the web layer still refuses the owner's rights to a request
    /// that carries any other proxy header, see `web.rs`). T40 and the guide say this plainly.
    #[test]
    fn without_the_header_a_public_name_is_never_the_owner_and_a_loopback_name_is_the_machines_own_browser(
    ) {
        let t = trust();
        for host in ["agora.example.com:443", "agora.example.com"] {
            assert_eq!(
                t.judge(&head(&[&format!("Host: {host}")])),
                Verdict::Proxied(None),
                "{host}"
            );
        }
        for host in ["127.0.0.1:7443", "localhost", "[::1]:7443", "LOCALHOST."] {
            assert_eq!(
                t.judge(&head(&[&format!("Host: {host}")])),
                Verdict::Direct,
                "{host}"
            );
        }
    }

    /// A loopback `Host` with a public `Origin` is a proxy that kept the target's Host (a POST or a WebSocket from the page of the
    /// public name); the owner's own browser sends a loopback Origin or none.
    #[test]
    fn a_loopback_host_with_a_public_origin_is_not_the_owners_browser() {
        let t = trust();
        for origin in [
            "https://agora.example.com",
            "https://agora.example.com:8443",
        ] {
            let h = head(&["Host: 127.0.0.1:7443", &format!("Origin: {origin}")]);
            assert_eq!(t.judge(&h), Verdict::Proxied(None), "{origin}");
        }
        for origin in [
            "http://127.0.0.1:7443",
            "http://localhost:7443",
            "http://[::1]:7443",
        ] {
            let h = head(&["Host: 127.0.0.1:7443", &format!("Origin: {origin}")]);
            assert_eq!(t.judge(&h), Verdict::Direct, "{origin}");
        }
        // A bare navigation has no Origin: the documented gap.
        assert_eq!(t.judge(&head(&["Host: 127.0.0.1:7443"])), Verdict::Direct);
    }

    /// Only a file that is not there is off; a path that cannot be read is damaged. Unix: a path under a regular file is
    /// `NotADirectory`, which is not `NotFound` there (other platforms map it differently, so this test is Unix only).
    #[cfg(unix)]
    #[test]
    fn an_unreadable_path_is_damaged_and_only_a_missing_file_is_off() {
        let dir = tmp("notfound");
        std::fs::write(dir.join("plain"), "x").unwrap();
        assert_eq!(read_file(&dir.join("absent")), Stored::Off);
        assert_eq!(read_file(&dir.join("plain").join(FILE)), Stored::Damaged);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_listed_proxy_matches_however_the_socket_spells_it_and_nothing_unusable_is_listed() {
        let t = ProxyTrust::new(
            "x-ip",
            &["a.example.com".into()],
            &["::ffff:10.1.2.3".parse().unwrap()],
        )
        .unwrap();
        assert_eq!(t.proxies(), ["10.1.2.3".parse::<IpAddr>().unwrap()]);
        assert!(t.trusts_peer("10.1.2.3".parse().unwrap()));
        assert!(t.trusts_peer("::ffff:10.1.2.3".parse().unwrap()));
        assert!(!t.trusts_peer("::ffff:10.1.2.4".parse().unwrap()));
        let listed_v4 = ProxyTrust::new(
            "x-ip",
            &["a.example.com".into()],
            &["10.1.2.3".parse().unwrap()],
        )
        .unwrap();
        assert!(listed_v4.trusts_peer("::ffff:10.1.2.3".parse().unwrap()));
        for bad in ["0.0.0.0", "::", "224.0.0.1", "ff02::1", "::ffff:0.0.0.0"] {
            assert!(
                ProxyTrust::new("x-ip", &["a.example.com".into()], &[bad.parse().unwrap()])
                    .is_none(),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_file_of_more_than_32_lines_is_damaged_whole() {
        let mut text = String::from("header=x-ip\nhost=a.example.com\n");
        for _ in 0..30 {
            text.push('\n');
        }
        assert!(ProxyTrust::parse(&text).is_some(), "32 lines are fine");
        text.push('\n');
        assert!(ProxyTrust::parse(&text).is_none(), "33 are not");
        // Not even when the extra lines are ones that would be valid.
        let many = format!("header=x-ip\n{}", "host=a.example.com\n".repeat(40));
        assert!(ProxyTrust::parse(&many).is_none());
    }

    #[test]
    fn another_host_is_refused() {
        let t = trust();
        for host in [
            "evil.example.org",
            "agora.example.com.evil.org",
            "evil.org:agora.example.com",
            "agora.example.com@evil.org",
            "agora.example.com:notaport",
            "10.1.2.3",
            "",
        ] {
            let h = head(&[&format!("Host: {host}")]);
            assert_eq!(
                t.judge(&h),
                Verdict::Refuse {
                    status: 421,
                    code: "proxy.host_not_allowed"
                },
                "{host:?}"
            );
        }
        // No Host, and two of them.
        for h in [
            head(&["Accept: */*"]),
            head(&["Host: agora.example.com", "Host: evil.org"]),
            head(&["Host: evil.org", "Host: agora.example.com"]),
        ] {
            assert!(matches!(t.judge(&h), Verdict::Refuse { status: 421, .. }));
        }
    }

    #[test]
    fn a_browser_origin_of_another_site_is_refused() {
        let t = trust();
        for origin in [
            "https://evil.example.org",
            "https://agora.example.com.evil.org",
            "null",
            "",
            "ftp://agora.example.com",
            "agora.example.com",
            "https://agora.example.com@evil.org",
            "https://127.0.0.1.evil.org",
        ] {
            let h = head(&["Host: agora.example.com", &format!("Origin: {origin}")]);
            assert_eq!(
                t.judge(&h),
                Verdict::Refuse {
                    status: 403,
                    code: "proxy.origin_not_allowed"
                },
                "{origin:?}"
            );
        }
        let twice = head(&[
            "Host: agora.example.com",
            "Origin: https://agora.example.com",
            "Origin: https://evil.org",
        ]);
        assert!(matches!(
            t.judge(&twice),
            Verdict::Refuse { status: 403, .. }
        ));
        for origin in [
            "https://agora.example.com",
            "https://agora.example.com:8443",
            "http://localhost:5173",
            "https://127.0.0.1:7443",
        ] {
            let h = head(&[
                "Host: agora.example.com",
                "CF-Connecting-IP: 203.0.113.9",
                &format!("Origin: {origin}"),
            ]);
            assert!(matches!(t.judge(&h), Verdict::Proxied(Some(_))), "{origin}");
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("agora-proxy-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_file_is_read_as_a_small_regular_file_only() {
        let dir = tmp("read");
        let file = dir.join(FILE);
        assert_eq!(read_file(&file), Stored::Off);
        std::fs::write(&file, trust().render()).unwrap();
        assert_eq!(read_file(&file), Stored::On(trust()));
        std::fs::write(&file, "header=x\nbogus\n").unwrap();
        assert_eq!(read_file(&file), Stored::Damaged);
        // Big and not valid.
        std::fs::write(&file, vec![b'a'; 5000]).unwrap();
        assert_eq!(read_file(&file), Stored::Damaged);
        // Big and otherwise valid: the size cap alone refuses it (a long run of spaces after a good line).
        let padded = format!("{}{}\n", trust().render(), " ".repeat(4096));
        assert!(
            ProxyTrust::parse(&padded).is_some(),
            "valid but for its size"
        );
        std::fs::write(&file, &padded).unwrap();
        assert_eq!(read_file(&file), Stored::Damaged);
        // Exactly at the cap is read.
        let at_cap = format!(
            "{}{}",
            trust().render(),
            " ".repeat(4096 - trust().render().len())
        );
        assert_eq!(at_cap.len(), 4096);
        std::fs::write(&file, &at_cap).unwrap();
        assert_eq!(read_file(&file), Stored::On(trust()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_place_of_the_file_is_refused_even_when_it_points_at_a_good_one() {
        let dir = tmp("link");
        std::fs::write(dir.join("real"), trust().render()).unwrap();
        std::os::unix::fs::symlink(dir.join("real"), dir.join(FILE)).unwrap();
        assert_eq!(read_file(&dir.join(FILE)), Stored::Damaged);
        let handle = ProxyHandle::watch(dir.join(FILE), |p| std::fs::File::open(p)).always_fresh();
        assert!(handle.current().is_none(), "a link trusts nothing");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_running_handle_follows_the_file_and_keeps_the_last_good_settings_when_it_turns_damaged() {
        let dir = tmp("handle");
        let file = dir.join(FILE);
        let handle = ProxyHandle::watch(file.clone(), |p| std::fs::File::open(p)).always_fresh();
        assert!(handle.current().is_none(), "no file: off");
        std::fs::write(&file, trust().render()).unwrap();
        assert_eq!(handle.current().as_deref(), Some(&trust()));
        // Damaged while running: the Host and Origin checks do not switch off.
        std::fs::write(&file, "header=x\nbogus\n").unwrap();
        assert_eq!(handle.current().as_deref(), Some(&trust()));
        std::fs::write(&file, vec![b'a'; 5000]).unwrap();
        assert_eq!(handle.current().as_deref(), Some(&trust()));
        // A new good file replaces it; a removed file is off.
        let other = ProxyTrust::new("x-ip", &["b.example.com".into()], &[]).unwrap();
        std::fs::write(&file, other.render()).unwrap();
        assert_eq!(handle.current().as_deref(), Some(&other));
        std::fs::remove_file(&file).unwrap();
        assert!(handle.current().is_none());
        // A hub that starts with a damaged file trusts nothing.
        std::fs::write(&file, "bogus\n").unwrap();
        let fresh = ProxyHandle::watch(file, |p| std::fs::File::open(p)).always_fresh();
        assert!(fresh.current().is_none());
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(ProxyHandle::fixed(Some(trust())).current().is_some());
        assert!(ProxyHandle::fixed(None).current().is_none());
    }
}

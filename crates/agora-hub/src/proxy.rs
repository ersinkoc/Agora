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
            if !addrs.contains(p) {
                addrs.push(*p);
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
        for line in text.lines().take(32) {
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
        is_loopback(peer) || self.proxies.contains(&peer)
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
        match values("origin").as_slice() {
            [] => {}
            [one] => {
                let ok = one
                    .split_once("://")
                    .filter(|(scheme, _)| matches!(*scheme, "http" | "https"))
                    .and_then(|(_, rest)| authority_host(rest))
                    .is_some_and(|h| self.host_allowed(&h));
                if !ok {
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

/// Reads the file: a regular file only (not a link), at most 4 KiB.
pub fn read_file(path: &Path) -> Stored {
    use std::io::Read;
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return Stored::Off;
    };
    if !meta.file_type().is_file() {
        return Stored::Damaged;
    }
    let mut bytes = Vec::new();
    if std::fs::File::open(path)
        .and_then(|f| f.take(MAX_FILE + 1).read_to_end(&mut bytes))
        .is_err()
        || bytes.len() as u64 > MAX_FILE
    {
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
    file: Option<PathBuf>,
    cache: Mutex<(Option<Instant>, Option<Arc<ProxyTrust>>)>,
}

impl ProxyHandle {
    /// Follows `file`.
    pub fn watch(file: PathBuf) -> Self {
        Self {
            file: Some(file),
            cache: Mutex::new((None, None)),
        }
    }

    /// A fixed answer (tests).
    pub fn fixed(trust: Option<ProxyTrust>) -> Self {
        Self {
            file: None,
            cache: Mutex::new((Some(Instant::now()), trust.map(Arc::new))),
        }
    }

    /// The proxy that is trusted now, if any. A damaged file trusts nothing.
    pub fn current(&self) -> Option<Arc<ProxyTrust>> {
        let mut c = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(file) = &self.file {
            if c.0.is_none_or(|t| t.elapsed() >= RELOAD) {
                c.1 = match read_file(file) {
                    Stored::On(t) => Some(Arc::new(t)),
                    _ => None,
                };
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

    #[test]
    fn a_public_name_without_the_header_is_a_proxy_set_up_wrong_never_the_owner() {
        let t = trust();
        assert_eq!(
            t.judge(&head(&["Host: agora.example.com:443"])),
            Verdict::Proxied(None)
        );
        // The owner's own browser, by a loopback name, is direct.
        for host in ["127.0.0.1:7443", "localhost", "[::1]:7443", "LOCALHOST."] {
            assert_eq!(
                t.judge(&head(&[&format!("Host: {host}")])),
                Verdict::Direct,
                "{host}"
            );
        }
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

    #[test]
    fn the_handle_follows_the_file_and_a_damaged_file_trusts_nothing() {
        let dir = std::env::temp_dir().join(format!("agora-proxy-handle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(FILE);
        assert_eq!(read_file(&file), Stored::Off);
        std::fs::write(&file, trust().render()).unwrap();
        assert_eq!(read_file(&file), Stored::On(trust()));
        std::fs::write(&file, "header=x\nbogus\n").unwrap();
        assert_eq!(read_file(&file), Stored::Damaged);
        std::fs::write(&file, vec![b'a'; 5000]).unwrap();
        assert_eq!(read_file(&file), Stored::Damaged);
        let handle = ProxyHandle::watch(file.clone());
        assert!(handle.current().is_none());
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(ProxyHandle::fixed(Some(trust())).current().is_some());
        assert!(ProxyHandle::fixed(None).current().is_none());
    }
}

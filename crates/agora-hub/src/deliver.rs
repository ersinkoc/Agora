//! Delivery of a notification to the owner's phone or chat: `ntfy` (a topic URL) and `webhook` (Slack, Discord and
//! anything that takes a JSON `{"text"}` / `{"content"}` post). Plain HTTP/1.1 over `std` sockets, with TLS from rustls and
//! the system's root certificates for `https`. One short request per notification, bounded by timeouts; a failure is reported to the
//! caller and never stops the hub. The text is already rendered (from `locales/`); nothing here writes prose.
//!
//! Targets come from the owner's file `<hub dir>/notify`: one `ntfy <url>` or `webhook <url>` per line.

use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use crate::notify::Priority;

/// How long connecting, and then each read or write, may take.
const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Ntfy,
    Webhook,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub kind: Kind,
    pub url: Url,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    pub https: bool,
    pub host: String,
    pub port: u16,
    /// Path and query, starting with `/`.
    pub path: String,
}

impl Url {
    /// `http://` or `https://`, a host, an optional port and path. No credentials in the URL.
    pub fn parse(s: &str) -> Option<Url> {
        let (https, rest) = if let Some(r) = s.strip_prefix("https://") {
            (true, r)
        } else {
            (false, s.strip_prefix("http://")?)
        };
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], rest[i..].to_string()),
            None => (rest, "/".to_string()),
        };
        if authority.is_empty()
            || authority.contains('@')
            || path.chars().any(|c| c.is_control() || c == ' ')
        {
            return None;
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) if !p.is_empty() => (h, p.parse().ok()?),
            _ => (authority, if https { 443 } else { 80 }),
        };
        (!host.is_empty()
            && host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')))
        .then(|| Url {
            https,
            host: host.to_string(),
            port,
            path,
        })
    }
}

/// Reads the targets file. Blank lines and `#` comments are skipped; lines that do not parse are counted, not guessed at.
pub fn parse_targets(text: &str) -> (Vec<Target>, usize) {
    let (mut out, mut skipped) = (Vec::new(), 0);
    for line in text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let parsed = line
            .split_once(char::is_whitespace)
            .and_then(|(kind, url)| {
                let kind = match kind {
                    "ntfy" => Kind::Ntfy,
                    "webhook" => Kind::Webhook,
                    _ => return None,
                };
                Some(Target {
                    kind,
                    url: Url::parse(url.trim())?,
                })
            });
        match parsed {
            Some(t) => out.push(t),
            None => skipped += 1,
        }
    }
    (out, skipped)
}

fn header_safe(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// The request bytes for one notification.
pub fn request(target: &Target, title: &str, text: &str, priority: Priority) -> Vec<u8> {
    let (content_type, extra, body) = match target.kind {
        // ntfy: the body is the message; the title and priority are headers.
        Kind::Ntfy => {
            let p = match priority {
                Priority::High => 4,
                Priority::Normal => 3,
                Priority::Low => 2,
            };
            (
                "text/plain; charset=utf-8",
                format!("Title: {}\r\nPriority: {p}\r\n", header_safe(title)),
                text.as_bytes().to_vec(),
            )
        }
        // Slack reads "text", Discord reads "content"; each ignores the other.
        Kind::Webhook => (
            "application/json",
            String::new(),
            serde_json::json!({ "text": text, "content": text })
                .to_string()
                .into_bytes(),
        ),
    };
    let u = &target.url;
    let mut out = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: agora\r\nContent-Type: {content_type}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
        u.path, u.host, body.len()
    )
    .into_bytes();
    out.extend_from_slice(&body);
    out
}

fn status_of(response: &[u8]) -> Option<u16> {
    std::str::from_utf8(response.get(..12)?)
        .ok()?
        .get(9..12)?
        .parse()
        .ok()
}

fn exchange<S: Read + Write>(mut s: S, bytes: &[u8]) -> io::Result<u16> {
    s.write_all(bytes)?;
    let mut head = Vec::new();
    // Only the status line is needed; stop at 256 bytes or the end of the stream.
    let mut buf = [0u8; 256];
    while head.len() < 12 {
        let n = s.read(&mut buf)?;
        if n == 0 {
            break;
        }
        head.extend_from_slice(&buf[..n]);
    }
    status_of(&head).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no HTTP status"))
}

fn tls_config() -> io::Result<Arc<rustls::ClientConfig>> {
    let mut roots = rustls::RootCertStore::empty();
    // Certificates the system lists that rustls cannot read are skipped; having none makes every https target fail.
    roots.add_parsable_certificates(rustls_native_certs::load_native_certs().certs);
    let builder = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
    Ok(Arc::new(
        builder.with_root_certificates(roots).with_no_client_auth(),
    ))
}

/// Sends one prepared request; returns the HTTP status. Blocking: call it from a thread of its own.
pub fn send(url: &Url, bytes: &[u8]) -> io::Result<u16> {
    let addr = (url.host.as_str(), url.port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no address"))?;
    let tcp = TcpStream::connect_timeout(&addr, TIMEOUT)?;
    tcp.set_read_timeout(Some(TIMEOUT))?;
    tcp.set_write_timeout(Some(TIMEOUT))?;
    if !url.https {
        return exchange(tcp, bytes);
    }
    let name = rustls::pki_types::ServerName::try_from(url.host.clone())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
    let conn = rustls::ClientConnection::new(tls_config()?, name)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
    exchange(rustls::StreamOwned::new(conn, tcp), bytes)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_parsed_strictly() {
        let u = Url::parse("https://ntfy.sh/my-topic?x=1").unwrap();
        assert_eq!(
            (u.https, u.host.as_str(), u.port, u.path.as_str()),
            (true, "ntfy.sh", 443, "/my-topic?x=1")
        );
        assert_eq!(Url::parse("http://127.0.0.1:8080").unwrap().port, 8080);
        for bad in [
            "ftp://x",
            "https://",
            "https://u:p@host/",
            "https://ho st/",
            "https://host:abc/",
            "host/path",
            "https://h/a b",
        ] {
            assert_eq!(Url::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_targets_file_skips_what_it_cannot_read() {
        let (t, skipped) = parse_targets("# mine\nntfy https://ntfy.sh/a\n\nwebhook http://localhost:9/x\nsms 123\nntfy not-a-url\n");
        assert_eq!(t.len(), 2);
        assert_eq!(skipped, 2);
        assert_eq!(t[0].kind, Kind::Ntfy);
    }

    #[test]
    fn requests_carry_the_text_and_never_a_line_break_in_a_header() {
        let (t, _) = parse_targets("ntfy https://ntfy.sh/a\nwebhook https://h.example/hook");
        let r = String::from_utf8(request(
            &t[0],
            "Agora\r\nX-Evil: 1",
            "Claude needs you.",
            Priority::High,
        ))
        .unwrap();
        assert!(r.starts_with("POST /a HTTP/1.1\r\nHost: ntfy.sh\r\n"));
        assert!(r.contains("Priority: 4\r\n") && r.contains("Title: Agora  X-Evil: 1\r\n"));
        assert!(r.ends_with("\r\n\r\nClaude needs you."));
        let w = String::from_utf8(request(&t[1], "t", "Hi \"there\"", Priority::Low)).unwrap();
        assert!(
            w.contains("Content-Type: application/json")
                && w.ends_with(r#"{"content":"Hi \"there\"","text":"Hi \"there\""}"#)
        );
    }

    #[test]
    fn a_plain_http_post_reaches_the_server_and_its_status_comes_back() {
        use std::net::TcpListener;
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut got = vec![0u8; 2048];
            let n = s.read(&mut got).unwrap();
            s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
            String::from_utf8_lossy(&got[..n]).to_string()
        });
        let (t, _) = parse_targets(&format!("webhook http://127.0.0.1:{port}/hook"));
        let status = send(&t[0].url, &request(&t[0], "t", "hello", Priority::Normal)).unwrap();
        assert_eq!(status, 200);
        let seen = server.join().unwrap();
        assert!(seen.starts_with("POST /hook HTTP/1.1") && seen.contains("hello"));
    }
}

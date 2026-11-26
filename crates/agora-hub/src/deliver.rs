//! Delivery of a notification to the owner's phone or chat: `ntfy` (a topic URL) and `webhook` (Slack, Discord and
//! anything that takes a JSON `{"text"}` / `{"content"}` post). Plain HTTP/1.1 over `std` sockets, with TLS from rustls and
//! the system's root certificates for `https`. One short request per notification, bounded by timeouts; a failure is reported to the
//! caller and never stops the hub. The text is already rendered (from `locales/`); nothing here writes prose.
//!
//! Targets come from the owner's file `<hub dir>/notify`: one `ntfy <url>` or `webhook <url>` per line.

use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::{Duration, Instant};

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

trait Stream: Read + Write {}
impl<T: Read + Write> Stream for T {}

/// How long a whole notification post may take.
const SEND_DEADLINE: Duration = Duration::from_secs(30);
/// How long a whole [`get`] may take (`agora hub prices update`).
pub const GET_DEADLINE: Duration = Duration::from_secs(120);

fn timed_out() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "deadline")
}

/// The time left for the next step: at most `step`, and never past `deadline` (`TimedOut` once it has passed).
fn left(step: Duration, deadline: Instant) -> io::Result<Duration> {
    let rest = deadline.saturating_duration_since(Instant::now());
    if rest.is_zero() {
        Err(timed_out())
    } else {
        Ok(rest.min(step))
    }
}

/// A socket where every read and write may take at most `step` and none starts or outlasts `deadline`. TLS sits on top of it, so
/// a handshake or a record that arrives a byte at a time is bounded too: rustls reads the socket many times for one record, and each
/// of those reads gets only the time that is left (#1225 review F1).
struct Bounded {
    tcp: TcpStream,
    step: Duration,
    deadline: Instant,
}

impl Bounded {
    /// A timeout that fired because the deadline came says so, not "would block".
    fn after(&self, e: io::Error) -> io::Error {
        if matches!(
            e.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
        ) && Instant::now() >= self.deadline
        {
            timed_out()
        } else {
            e
        }
    }
}

impl Read for Bounded {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.tcp
            .set_read_timeout(Some(left(self.step, self.deadline)?))?;
        self.tcp.read(buf).map_err(|e| self.after(e))
    }
}

impl Write for Bounded {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.tcp
            .set_write_timeout(Some(left(self.step, self.deadline)?))?;
        self.tcp.write(buf).map_err(|e| self.after(e))
    }
    fn flush(&mut self) -> io::Result<()> {
        self.tcp.flush()
    }
}

/// Connects to one of the host's addresses (in the dialler's order), each attempt for at most the time left, and returns the stream,
/// over TLS for `https`, with every later read and write bounded the same way. Resolving the name is the system's and not bounded here.
fn open_within(url: &Url, step: Duration, deadline: Instant) -> io::Result<Box<dyn Stream>> {
    let addrs =
        agora_protocol::dial::order((url.host.as_str(), url.port).to_socket_addrs()?.collect());
    let mut last = io::Error::new(io::ErrorKind::NotFound, "no address");
    let mut tcp = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, left(step, deadline)?) {
            Ok(s) => {
                tcp = Some(s);
                break;
            }
            Err(e) => last = e,
        }
    }
    let Some(tcp) = tcp else {
        return Err(if Instant::now() >= deadline {
            timed_out()
        } else {
            last
        });
    };
    let bounded = Bounded {
        tcp,
        step,
        deadline,
    };
    if !url.https {
        return Ok(Box::new(bounded));
    }
    let name = rustls::pki_types::ServerName::try_from(url.host.clone())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
    let conn = rustls::ClientConnection::new(tls_config()?, name)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
    Ok(Box::new(rustls::StreamOwned::new(conn, bounded)))
}

/// Sends one prepared request; returns the HTTP status. Blocking: call it from a thread of its own. Each step may take
/// [`TIMEOUT`] and the whole post [`SEND_DEADLINE`].
pub fn send(url: &Url, bytes: &[u8]) -> io::Result<u16> {
    exchange(
        open_within(url, TIMEOUT, Instant::now() + SEND_DEADLINE)?,
        bytes,
    )
}

/// `GET` the URL and return the status and the body (at most `max` bytes; more is an error). HTTP/1.0 so the body is not
/// chunked. Blocking. Each step may take [`TIMEOUT`] and the whole exchange [`GET_DEADLINE`].
pub fn get(url: &Url, max: usize) -> io::Result<(u16, Vec<u8>)> {
    get_within(url, max, TIMEOUT, Instant::now() + GET_DEADLINE)
}

/// [`get`] where connecting, and then each read or write (inside TLS too), may take at most `step`, and the whole exchange must
/// end by `deadline` (`TimedOut` otherwise), so a server that sends a byte now and then cannot hold the caller. A redirect is not
/// followed: its status comes back like any other.
pub fn get_within(
    url: &Url,
    max: usize,
    step: Duration,
    deadline: Instant,
) -> io::Result<(u16, Vec<u8>)> {
    let mut s = open_within(url, step, deadline)?;
    let req = format!(
        "GET {} HTTP/1.0\r\nHost: {}\r\nUser-Agent: agora\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
        url.path, url.host
    );
    s.write_all(req.as_bytes())?;
    let mut all = Vec::new();
    let mut buf = [0u8; 16 * 1024];
    loop {
        let n = match s.read(&mut buf) {
            Ok(n) => n,
            // A server that closes without a TLS close_notify has still sent its whole body.
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => 0,
            // rustls may wrap the socket's error; past the deadline it is the deadline whatever it says.
            Err(_) if Instant::now() >= deadline => return Err(timed_out()),
            Err(e) => return Err(e),
        };
        if n == 0 {
            break;
        }
        all.extend_from_slice(&buf[..n]);
        if all.len() > max + 16 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "response too large",
            ));
        }
    }
    let status = status_of(&all)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no HTTP status"))?;
    let body_at = find_body(&all)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no HTTP body"))?;
    let body = all.split_off(body_at);
    if body.len() > max {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "response too large",
        ));
    }
    Ok((status, body))
}

fn find_body(r: &[u8]) -> Option<usize> {
    r.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
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
    fn get_returns_the_status_and_only_the_body_and_refuses_a_body_over_the_limit() {
        use std::net::TcpListener;
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut s, _) = l.accept().unwrap();
                let mut got = [0u8; 512];
                let _ = s.read(&mut got).unwrap();
                s.write_all(b"HTTP/1.0 200 OK\r\nX: y\r\n\r\n0123456789")
                    .unwrap();
            }
        });
        let u = Url::parse(&format!("http://127.0.0.1:{port}/api.json")).unwrap();
        assert_eq!(get(&u, 100).unwrap(), (200, b"0123456789".to_vec()));
        assert!(get(&u, 5).is_err());
    }

    /// #1225 review F1: a server that answers the TLS handshake with a record header and then one byte now and then (each within a
    /// step) is given up at the deadline, not after every step again; so is the same over plain http.
    #[test]
    fn a_server_that_drips_is_given_up_at_the_deadline_over_https_and_http() {
        use std::net::TcpListener;
        for https in [true, false] {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = l.local_addr().unwrap().port();
            std::thread::spawn(move || {
                let (mut s, _) = l.accept().unwrap();
                let mut hello = [0u8; 4096];
                let _ = s.read(&mut hello);
                // A handshake record promising 16 KiB (TLS), or a status line and a body (http), then a byte every 100 ms.
                let head: &[u8] = if https {
                    &[0x16, 0x03, 0x03, 0x40, 0x00]
                } else {
                    b"HTTP/1.0 200 OK\r\n\r\n"
                };
                let _ = s.write_all(head);
                while s.write_all(b"\0").is_ok() {
                    std::thread::sleep(Duration::from_millis(100));
                }
            });
            let scheme = if https { "https" } else { "http" };
            let u = Url::parse(&format!("{scheme}://127.0.0.1:{port}/index.json")).unwrap();
            let started = Instant::now();
            let e = get_within(
                &u,
                1 << 20,
                Duration::from_millis(400),
                started + Duration::from_millis(1200),
            )
            .unwrap_err();
            let took = started.elapsed();
            assert_eq!(e.kind(), io::ErrorKind::TimedOut, "{scheme}: {e}");
            assert!(took < Duration::from_secs(4), "{scheme} took {took:?}");
        }
    }

    /// Nothing starts after the deadline: not even the connection.
    #[test]
    fn nothing_starts_after_the_deadline() {
        let u = Url::parse("http://127.0.0.1:9/x").unwrap();
        let e = get_within(&u, 10, Duration::from_secs(1), Instant::now()).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::TimedOut);
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

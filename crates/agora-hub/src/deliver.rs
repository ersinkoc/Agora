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
const MAX_RESPONSE_HEAD: usize = 16 * 1024;

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
        let split = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let authority = &rest[..split];
        // A fragment belongs to the client and is never part of an HTTP request target. A query without an explicit path uses `/`.
        let target = rest[split..]
            .split_once('#')
            .map_or(&rest[split..], |(p, _)| p);
        let path = if target.is_empty() {
            "/".to_string()
        } else if target.starts_with('?') {
            format!("/{target}")
        } else {
            target.to_string()
        };
        if authority.is_empty()
            || authority.contains('@')
            || path.chars().any(|c| c.is_control() || c == ' ')
            || !valid_percent_encoding(&path)
        {
            return None;
        }
        let default_port = if https { 443 } else { 80 };
        let port = |text: &str| {
            (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
                .then(|| text.parse::<u16>().ok())
                .flatten()
                .filter(|port| *port != 0)
        };
        let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
            let (host, tail) = bracketed.split_once(']')?;
            host.parse::<std::net::Ipv6Addr>().ok()?;
            let port = if tail.is_empty() {
                default_port
            } else {
                port(tail.strip_prefix(':')?)?
            };
            (host, port)
        } else {
            if authority.contains(['[', ']']) || authority.matches(':').count() > 1 {
                return None;
            }
            match authority.rsplit_once(':') {
                Some((h, p)) if !p.is_empty() => (h, port(p)?),
                _ => (authority, default_port),
            }
        };
        (port != 0
            && !host.is_empty()
            && (host.parse::<std::net::IpAddr>().is_ok()
                || host
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))))
        .then(|| Url {
            https,
            host: host.to_string(),
            port,
            path,
        })
    }

    fn authority(&self) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        if self.port == if self.https { 443 } else { 80 } {
            host
        } else {
            format!("{host}:{}", self.port)
        }
    }
}

fn valid_percent_encoding(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            if !bytes
                .get(at + 1..at + 3)
                .is_some_and(|pair| pair.iter().all(u8::is_ascii_hexdigit))
            {
                return false;
            }
            at += 3;
        } else {
            at += 1;
        }
    }
    true
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
    let authority = u.authority();
    let mut out = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: agora\r\nContent-Type: {content_type}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
        u.path, authority, body.len()
    )
    .into_bytes();
    out.extend_from_slice(&body);
    out
}

fn status_of(response: &[u8]) -> Option<u16> {
    let line_end = response
        .windows(2)
        .position(|w| w == b"\r\n")
        .unwrap_or(response.len());
    let line = &response[..line_end];
    let rest = line
        .strip_prefix(b"HTTP/1.0 ")
        .or_else(|| line.strip_prefix(b"HTTP/1.1 "))?;
    if rest.get(3) != Some(&b' ') {
        return None;
    }
    std::str::from_utf8(rest.get(..3)?)
        .ok()?
        .parse()
        .ok()
        .filter(|code| (100..=599).contains(code))
}

fn final_response(response: &[u8]) -> Option<(u16, usize, usize)> {
    let mut head_at = 0;
    loop {
        let body_at = head_at + find_body(response.get(head_at..)?)?;
        let status = status_of(response.get(head_at..body_at)?)?;
        if status >= 200 {
            return Some((status, head_at, body_at));
        }
        head_at = body_at;
    }
}

fn exchange<S: Read + Write>(mut s: S, bytes: &[u8]) -> io::Result<u16> {
    s.write_all(bytes)?;
    let mut head = Vec::new();
    // Read only response headers. Informational responses precede, rather than replace, the final response.
    let mut buf = [0u8; 1024];
    while head.len() < MAX_RESPONSE_HEAD {
        let remaining = MAX_RESPONSE_HEAD - head.len();
        let take = remaining.min(buf.len());
        let n = s.read(&mut buf[..take])?;
        if n == 0 {
            break;
        }
        head.extend_from_slice(&buf[..n]);
        if let Some((status, _, _)) = final_response(&head) {
            return Ok(status);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "no final HTTP response",
    ))
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
    let authority = url.authority();
    let req = format!(
        "GET {} HTTP/1.0\r\nHost: {}\r\nUser-Agent: agora\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
        url.path, authority
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
        match final_response(&all) {
            Some((_, _, body_at)) if body_at > MAX_RESPONSE_HEAD => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "response head too large",
                ));
            }
            None if all.len() >= MAX_RESPONSE_HEAD => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "response head too large",
                ));
            }
            _ => {}
        }
        if all.len() > max.saturating_add(MAX_RESPONSE_HEAD) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "response too large",
            ));
        }
    }
    let (status, head_at, body_at) = final_response(&all)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no final HTTP response"))?;
    let head = &all[head_at..body_at];
    let transfer_coded = validate_response_fields(head)?;
    if transfer_coded && status != 304 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported Transfer-Encoding",
        ));
    }
    // A 304 carries metadata for a selected representation but no message body; its Content-Length can describe that representation.
    let declared = (status != 304)
        .then(|| content_length(head))
        .transpose()?
        .flatten();
    let body = all.split_off(body_at);
    if matches!(status, 204 | 205 | 304) && !body.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "body on a content-free response",
        ));
    }
    if let Some(declared) = declared {
        match body.len().cmp(&declared) {
            std::cmp::Ordering::Less => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "HTTP body shorter than Content-Length",
                ));
            }
            std::cmp::Ordering::Greater => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "HTTP body longer than Content-Length",
                ));
            }
            std::cmp::Ordering::Equal => {}
        }
    }
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

/// Validates the final response's field lines and reports whether transfer coding was declared.
fn validate_response_fields(head: &[u8]) -> io::Result<bool> {
    let mut transfer_coded = false;
    for raw in head.split(|b| *b == b'\n').skip(1) {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        if line.is_empty() {
            break;
        }
        let Some(colon) = line.iter().position(|b| *b == b':') else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "malformed HTTP response field",
            ));
        };
        let (name, value) = (&line[..colon], &line[colon + 1..]);
        let tchar = |b: &u8| {
            b.is_ascii_alphanumeric()
                || matches!(
                    *b,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        };
        if name.is_empty()
            || !name.iter().all(tchar)
            || value
                .iter()
                .any(|b| (*b < b' ' && *b != b'\t') || *b == 0x7f)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "malformed HTTP response field",
            ));
        }
        transfer_coded |= name.eq_ignore_ascii_case(b"transfer-encoding");
    }
    Ok(transfer_coded)
}

fn content_length(head: &[u8]) -> io::Result<Option<usize>> {
    let mut declared = None;
    for raw in head.split(|b| *b == b'\n').skip(1) {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        let Some(colon) = line.iter().position(|b| *b == b':') else {
            continue;
        };
        let (name, value) = (&line[..colon], &line[colon + 1..]);
        if !name.eq_ignore_ascii_case(b"content-length") {
            continue;
        }
        for item in value.split(|b| *b == b',') {
            let item = item.trim_ascii();
            if item.is_empty() || !item.iter().all(u8::is_ascii_digit) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid Content-Length",
                ));
            }
            let value = std::str::from_utf8(item)
                .ok()
                .and_then(|s| s.parse::<usize>().ok())
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid Content-Length")
                })?;
            if declared.is_some_and(|earlier| earlier != value) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "conflicting Content-Length",
                ));
            }
            declared = Some(value);
        }
    }
    Ok(declared)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    struct ScriptedStream {
        response: std::io::Cursor<Vec<u8>>,
        request: Vec<u8>,
        read_cap: usize,
    }

    impl ScriptedStream {
        fn new(response: &[u8]) -> Self {
            Self {
                response: std::io::Cursor::new(response.to_vec()),
                request: Vec::new(),
                read_cap: usize::MAX,
            }
        }

        fn with_cap(response: &[u8], read_cap: usize) -> Self {
            Self {
                read_cap,
                ..Self::new(response)
            }
        }
    }

    impl Read for ScriptedStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let take = buf.len().min(self.read_cap);
            self.response.read(&mut buf[..take])
        }
    }

    impl Write for ScriptedStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.request.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn serve_once(response: &'static [u8]) -> Url {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 512];
            let _ = stream.read(&mut request).unwrap();
            stream.write_all(response).unwrap();
        });
        Url::parse(&format!("http://127.0.0.1:{port}/catalog")).unwrap()
    }

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
    fn f345_a_query_without_an_explicit_path_is_still_a_valid_url() {
        let u = Url::parse("https://ntfy.example?poll=1").expect("query-only URL");
        assert_eq!(
            (u.host.as_str(), u.port, u.path.as_str()),
            ("ntfy.example", 443, "/?poll=1")
        );
    }

    #[test]
    fn a_query_after_an_explicit_path_stays_unchanged() {
        let u = Url::parse("https://ntfy.example/topic?poll=1").unwrap();
        assert_eq!(u.path, "/topic?poll=1");
    }

    #[test]
    fn f346_a_url_fragment_is_never_sent_in_the_http_request_target() {
        let u = Url::parse("https://ntfy.example/topic?x=1#private-view").unwrap();
        assert_eq!(u.path, "/topic?x=1");
        let target = Target {
            kind: Kind::Ntfy,
            url: u,
        };
        let text = String::from_utf8(request(&target, "t", "body", Priority::Normal)).unwrap();
        assert!(text.starts_with("POST /topic?x=1 HTTP/1.1\r\n"));
        assert!(!text.contains("#private-view"));
    }

    #[test]
    fn a_request_target_without_a_fragment_stays_unchanged() {
        let target = Target {
            kind: Kind::Ntfy,
            url: Url::parse("https://ntfy.example/topic?x=1").unwrap(),
        };
        let text = String::from_utf8(request(&target, "t", "body", Priority::Normal)).unwrap();
        assert!(text.starts_with("POST /topic?x=1 HTTP/1.1\r\n"));
    }

    #[test]
    fn f347_a_non_default_port_is_present_in_the_http_host_header() {
        let target = Target {
            kind: Kind::Webhook,
            url: Url::parse("https://hooks.example:8443/in").unwrap(),
        };
        let text = String::from_utf8(request(&target, "t", "body", Priority::Normal)).unwrap();
        assert!(text.contains("\r\nHost: hooks.example:8443\r\n"));
    }

    #[test]
    fn a_default_port_keeps_the_plain_http_host_header() {
        let target = Target {
            kind: Kind::Webhook,
            url: Url::parse("https://hooks.example/in").unwrap(),
        };
        let text = String::from_utf8(request(&target, "t", "body", Priority::Normal)).unwrap();
        assert!(text.contains("\r\nHost: hooks.example\r\n"));
    }

    #[test]
    fn f348_only_an_http_1_status_line_can_supply_a_status() {
        assert_eq!(status_of(b"garbage!!200 everything"), None);
        assert_eq!(status_of(b"HTTP/9.9 200 made-up"), None);
        assert_eq!(status_of(b"HTTP/1.1 999 impossible"), None);
    }

    #[test]
    fn valid_http_1_status_lines_keep_their_status() {
        assert_eq!(status_of(b"HTTP/1.0 200 OK\r\n"), Some(200));
        assert_eq!(status_of(b"HTTP/1.1 503 Busy\r\n"), Some(503));
    }

    #[test]
    fn f350_a_status_code_must_end_at_a_status_line_delimiter() {
        assert_eq!(status_of(b"HTTP/1.1 200X-not-a-delimiter\r\n"), None);
    }

    #[test]
    fn a_status_code_followed_by_a_space_remains_valid() {
        assert_eq!(status_of(b"HTTP/1.1 200 OK\r\n"), Some(200));
        assert_eq!(status_of(b"HTTP/1.1 200 \xff\r\n"), Some(200));
    }

    #[test]
    fn f351_exchange_returns_the_final_status_after_an_informational_response() {
        let response = b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 204 No Content\r\n\r\n";
        assert_eq!(
            exchange(ScriptedStream::new(response), b"GET / HTTP/1.0\r\n\r\n").unwrap(),
            204
        );
    }

    #[test]
    fn exchange_keeps_returning_a_direct_final_status() {
        let response = b"HTTP/1.1 204 No Content\r\n\r\n";
        assert_eq!(
            exchange(ScriptedStream::new(response), b"GET / HTTP/1.0\r\n\r\n").unwrap(),
            204
        );
    }

    #[test]
    fn f352_get_rejects_a_body_shorter_than_its_content_length() {
        let url = serve_once(b"HTTP/1.0 200 OK\r\nContent-Length: 10\r\n\r\n{}");
        assert_eq!(
            get(&url, 100).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn get_keeps_accepting_a_body_that_matches_its_content_length() {
        let url = serve_once(b"HTTP/1.0 200 OK\r\nContent-Length: 2\r\n\r\n{}");
        assert_eq!(get(&url, 100).unwrap(), (200, b"{}".to_vec()));
    }

    #[test]
    fn get_keeps_accepting_a_bodyless_304_with_representation_length_metadata() {
        let url = serve_once(b"HTTP/1.0 304 Not Modified\r\nContent-Length: 20\r\n\r\n");
        assert_eq!(get(&url, 100).unwrap(), (304, Vec::new()));
    }

    #[test]
    fn f353_an_unbounded_get_limit_does_not_overflow() {
        let url = serve_once(b"HTTP/1.0 200 OK\r\nContent-Length: 2\r\n\r\n{}");
        assert_eq!(get(&url, usize::MAX).unwrap(), (200, b"{}".to_vec()));
    }

    #[test]
    fn a_small_get_limit_still_accepts_a_body_that_fits() {
        let url = serve_once(b"HTTP/1.0 200 OK\r\nContent-Length: 2\r\n\r\n{}");
        assert_eq!(get(&url, 2).unwrap(), (200, b"{}".to_vec()));
    }

    #[test]
    fn f354_malformed_percent_encoding_is_not_a_valid_url() {
        for raw in [
            "https://notify.example/%",
            "https://notify.example/%0",
            "https://notify.example/%GG",
        ] {
            assert_eq!(Url::parse(raw), None, "{raw}");
        }
    }

    #[test]
    fn valid_percent_encoding_in_a_url_stays_accepted() {
        for raw in [
            "https://notify.example/a%20b",
            "https://notify.example/%2Ftopic",
            "https://notify.example/?x=%25",
        ] {
            assert!(Url::parse(raw).is_some(), "{raw}");
        }
    }

    #[test]
    fn verifies_f350_status_code_delimiters_across_letters_tabs_and_missing_separators() {
        for bad in [
            b"HTTP/1.1 200X\r\n".as_slice(),
            b"HTTP/1.0 404\tNope\r\n".as_slice(),
            b"HTTP/1.1 503\r\n".as_slice(),
        ] {
            assert_eq!(status_of(bad), None, "{:?}", String::from_utf8_lossy(bad));
        }
        assert_eq!(status_of(b"HTTP/1.1 200 OK\r\n"), Some(200));
        assert_eq!(status_of(b"HTTP/1.0 204 \r\n"), Some(204));
    }

    #[test]
    fn verifies_f351_final_statuses_after_single_multiple_and_fragmented_informationals() {
        for (response, cap, want) in [
            (
                b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 204 Done\r\n\r\n".as_slice(),
                1024,
                204,
            ),
            (
                b"HTTP/1.1 103 Early\r\nLink: </x>\r\n\r\nHTTP/1.1 200 OK\r\n\r\n".as_slice(),
                7,
                200,
            ),
            (
                b"HTTP/1.0 100 One\r\n\r\nHTTP/1.0 102 Two\r\n\r\nHTTP/1.0 201 Made\r\n\r\n"
                    .as_slice(),
                1,
                201,
            ),
        ] {
            assert_eq!(
                exchange(ScriptedStream::with_cap(response, cap), b"request").unwrap(),
                want
            );
        }
    }

    #[test]
    fn verifies_f352_truncated_content_lengths_across_empty_small_and_large_shortfalls() {
        for response in [
            b"HTTP/1.0 200 OK\r\nContent-Length: 1\r\n\r\n".as_slice(),
            b"HTTP/1.0 200 OK\r\nContent-Length: 3\r\n\r\n{}".as_slice(),
            b"HTTP/1.0 200 OK\r\nContent-Length: 999\r\n\r\n{}".as_slice(),
        ] {
            let url = serve_once(response);
            assert_eq!(
                get(&url, 1_000).unwrap_err().kind(),
                io::ErrorKind::UnexpectedEof
            );
        }
    }

    #[test]
    fn verifies_f353_limits_at_each_side_of_the_header_allowance_overflow_boundary() {
        for max in [
            usize::MAX,
            usize::MAX - 1,
            usize::MAX - MAX_RESPONSE_HEAD + 1,
        ] {
            let url = serve_once(b"HTTP/1.0 200 OK\r\nContent-Length: 2\r\n\r\n{}");
            assert_eq!(get(&url, max).unwrap(), (200, b"{}".to_vec()), "{max}");
        }
    }

    #[test]
    fn verifies_f354_percent_triplets_across_path_query_and_hex_classes() {
        for bad in [
            "https://notify.example/topic%",
            "https://notify.example/topic%G0",
            "https://notify.example/topic?next=%0g",
        ] {
            assert_eq!(Url::parse(bad), None, "{bad}");
        }
        for good in [
            "https://notify.example/topic%20name",
            "https://notify.example/topic%2fname",
            "https://notify.example/topic?next=%2Fagent%2f1",
        ] {
            assert!(Url::parse(good).is_some(), "{good}");
        }
    }

    #[test]
    fn f349_a_bracketed_ipv6_literal_is_a_valid_notification_url() {
        let u = Url::parse("http://[2001:db8::5]:8080/topic").expect("IPv6 URL");
        assert_eq!(
            (u.host.as_str(), u.port, u.path.as_str()),
            ("2001:db8::5", 8080, "/topic")
        );
        let target = Target {
            kind: Kind::Ntfy,
            url: u,
        };
        let text = String::from_utf8(request(&target, "t", "body", Priority::Normal)).unwrap();
        assert!(text.contains("\r\nHost: [2001:db8::5]:8080\r\n"));
    }

    #[test]
    fn an_ipv4_literal_notification_url_keeps_working() {
        let u = Url::parse("http://127.0.0.1:8080/topic").unwrap();
        assert_eq!(
            (u.host.as_str(), u.port, u.path.as_str()),
            ("127.0.0.1", 8080, "/topic")
        );
    }

    #[test]
    fn verifies_f345_query_only_urls_across_schemes_authorities_and_reserved_values() {
        for (raw, want) in [
            ("https://notify.example?topic=a", "/?topic=a"),
            (
                "http://127.0.0.1:8080?next=%2Fagent%2Fx",
                "/?next=%2Fagent%2Fx",
            ),
            ("https://notify.example?x=1&y=two#local", "/?x=1&y=two"),
        ] {
            assert_eq!(
                Url::parse(raw).map(|u| u.path),
                Some(want.to_string()),
                "{raw}"
            );
        }
    }

    #[test]
    fn verifies_f346_fragments_are_removed_from_path_query_and_fragment_only_urls() {
        for (raw, want) in [
            ("https://notify.example/topic#phone", "/topic"),
            ("https://notify.example/topic?q=one#phone", "/topic?q=one"),
            ("https://notify.example#phone", "/"),
        ] {
            let u = Url::parse(raw).unwrap();
            assert_eq!(u.path, want, "{raw}");
            assert!(!u.path.contains('#'), "{raw}");
        }
    }

    #[test]
    fn verifies_f347_host_authorities_across_https_http_and_default_ports() {
        for (raw, want) in [
            ("https://hooks.example:8443/in", "hooks.example:8443"),
            ("http://127.0.0.1:8080/in", "127.0.0.1:8080"),
            ("https://hooks.example/in", "hooks.example"),
        ] {
            let target = Target {
                kind: Kind::Webhook,
                url: Url::parse(raw).unwrap(),
            };
            let text = String::from_utf8(request(&target, "t", "body", Priority::Normal)).unwrap();
            assert!(
                text.contains(&format!("\r\nHost: {want}\r\n")),
                "{raw}: {text}"
            );
        }
    }

    #[test]
    fn verifies_f348_status_lines_across_prefix_version_and_code_boundaries() {
        for bad in [
            b"not-http! 200 OK".as_slice(),
            b"HTTP/1.2 200 OK".as_slice(),
            b"HTTP/1.1 099 Low".as_slice(),
            b"HTTP/1.0 600 High".as_slice(),
        ] {
            assert_eq!(status_of(bad), None, "{:?}", String::from_utf8_lossy(bad));
        }
        assert_eq!(status_of(b"HTTP/1.1 100 Continue"), Some(100));
        assert_eq!(status_of(b"HTTP/1.0 599 Edge"), Some(599));
    }

    #[test]
    fn verifies_f349_ipv6_urls_across_schemes_default_ports_and_queries() {
        for (raw, host, port, path, authority) in [
            ("http://[::1]/topic", "::1", 80, "/topic", "[::1]"),
            (
                "https://[2001:db8::7]:9443/hook",
                "2001:db8::7",
                9443,
                "/hook",
                "[2001:db8::7]:9443",
            ),
            (
                "http://[2001:db8::8]?topic=a",
                "2001:db8::8",
                80,
                "/?topic=a",
                "[2001:db8::8]",
            ),
        ] {
            let u = Url::parse(raw).unwrap();
            assert_eq!(
                (u.host.as_str(), u.port, u.path.as_str()),
                (host, port, path),
                "{raw}"
            );
            assert_eq!(u.authority(), authority, "{raw}");
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

    #[test]
    fn proof_f430_signed_url_ports_are_not_decimal_port_syntax() {
        println!("CONTROL OK: an unsigned nonzero decimal port is accepted");
        assert_eq!(
            Url::parse("https://notify.example:443/x").unwrap().port,
            443
        );
        let actual = [
            Url::parse("https://notify.example:+443/x").is_some(),
            Url::parse("http://notify.example:+80/x").is_some(),
            Url::parse("http://[::1]:+8080/x").is_some(),
        ];
        println!("EXPECTED: signed port spellings are rejected");
        println!("ACTUAL: accepted signed ports = {actual:?}");
        if actual.iter().any(|accepted| *accepted) {
            println!("PROBLEM CONFIRMED");
        } else {
            println!("PROBLEM NOT REPRODUCED");
        }
        assert_eq!(actual, [false; 3]);
    }

    #[test]
    fn proof_f431_catalog_get_enforces_its_response_head_limit() {
        println!("CONTROL OK: a small response head and body are accepted");
        let control = serve_once(b"HTTP/1.0 200 OK\r\nX: y\r\n\r\n{}");
        assert_eq!(get(&control, 100).unwrap().1, b"{}");
        let mut response = b"HTTP/1.0 200 OK\r\nX-Pad: ".to_vec();
        response.extend(std::iter::repeat_n(b'a', MAX_RESPONSE_HEAD));
        response.extend_from_slice(b"\r\n\r\n{}");
        let oversized = serve_once(Box::leak(response.into_boxed_slice()));
        let actual = get(&oversized, 100_000);
        println!("EXPECTED: a response head over 16 KiB is rejected");
        println!("ACTUAL: oversized-head result = {actual:?}");
        if actual.is_ok() {
            println!("PROBLEM CONFIRMED");
        } else {
            println!("PROBLEM NOT REPRODUCED");
        }
        assert!(actual.is_err());
    }

    #[test]
    fn proof_f432_catalog_get_rejects_bodies_for_bodyless_statuses() {
        println!("CONTROL OK: a 200 response can carry its declared body");
        let control = serve_once(b"HTTP/1.0 200 OK\r\nContent-Length: 2\r\n\r\n{}");
        assert_eq!(get(&control, 100).unwrap().1, b"{}");
        let response = serve_once(b"HTTP/1.0 304 Not Modified\r\n\r\n{}");
        let actual = get(&response, 100);
        println!("EXPECTED: a 304 response with message content is rejected");
        println!("ACTUAL: bodyless-status result = {actual:?}");
        if actual.is_ok() {
            println!("PROBLEM CONFIRMED");
        } else {
            println!("PROBLEM NOT REPRODUCED");
        }
        assert!(actual.is_err());
    }

    #[test]
    fn proof_f433_catalog_get_never_returns_chunk_framing_as_the_body() {
        println!("CONTROL OK: a content-length framed body is returned without framing");
        let control = serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}");
        assert_eq!(get(&control, 100).unwrap().1, b"{}");
        let response = serve_once(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n",
        );
        let actual = get(&response, 100);
        println!("EXPECTED: unsupported transfer coding is rejected, not returned as content");
        println!("ACTUAL: transfer-coded result = {actual:?}");
        if actual.is_ok() {
            println!("PROBLEM CONFIRMED");
        } else {
            println!("PROBLEM NOT REPRODUCED");
        }
        assert!(actual.is_err());
    }

    #[test]
    fn proof_f434_catalog_get_rejects_malformed_response_fields() {
        println!("CONTROL OK: an exact Content-Length field is accepted");
        let control = serve_once(b"HTTP/1.0 200 OK\r\nContent-Length: 2\r\n\r\n{}");
        assert_eq!(get(&control, 100).unwrap().1, b"{}");
        let response = serve_once(b"HTTP/1.0 200 OK\r\nContent-Length : 2\r\n\r\n{}");
        let actual = get(&response, 100);
        println!("EXPECTED: whitespace before a response-field colon is rejected");
        println!("ACTUAL: malformed-field result = {actual:?}");
        if actual.is_ok() {
            println!("PROBLEM CONFIRMED");
        } else {
            println!("PROBLEM NOT REPRODUCED");
        }
        assert!(actual.is_err());
    }

    #[test]
    fn verify_f430_port_grammar_across_sign_range_and_authority_forms() {
        for bad in [
            "https://notify.example:+1/x",
            "https://notify.example:-443/x",
            "https://[2001:db8::1]:+65535/x",
        ] {
            assert_eq!(Url::parse(bad), None, "{bad}");
        }
        for (good, want) in [
            ("http://notify.example:1/x", 1),
            ("https://notify.example:65535/x", 65535),
            ("https://[2001:db8::1]:8443/x", 8443),
        ] {
            assert_eq!(Url::parse(good).map(|url| url.port), Some(want), "{good}");
        }
        println!("FIX VERIFIED: F430 decimal unsigned nonzero u16 ports only");
    }

    #[test]
    fn verify_f431_response_head_limit_at_both_exact_boundaries() {
        fn response(head_len: usize) -> &'static [u8] {
            let prefix = b"HTTP/1.0 200 OK\r\nX: ";
            let suffix = b"\r\n\r\n";
            assert!(head_len >= prefix.len() + suffix.len());
            let mut bytes = prefix.to_vec();
            bytes.extend(std::iter::repeat_n(
                b'a',
                head_len - prefix.len() - suffix.len(),
            ));
            bytes.extend_from_slice(suffix);
            bytes.extend_from_slice(b"{}");
            Box::leak(bytes.into_boxed_slice())
        }
        for size in [MAX_RESPONSE_HEAD - 1, MAX_RESPONSE_HEAD] {
            let url = serve_once(response(size));
            assert_eq!(get(&url, 100).unwrap().1, b"{}", "{size}");
        }
        let url = serve_once(response(MAX_RESPONSE_HEAD + 1));
        assert_eq!(
            get(&url, 100).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        println!("FIX VERIFIED: F431 exact 16 KiB cumulative response-head limit");
    }

    #[test]
    fn verify_f432_content_free_statuses_across_all_three_codes() {
        for status in [204, 205, 304] {
            let empty = format!("HTTP/1.0 {status} Status\r\n\r\n");
            let url = serve_once(Box::leak(empty.into_bytes().into_boxed_slice()));
            assert_eq!(get(&url, 100).unwrap(), (status, Vec::new()), "{status}");

            let with_body = format!("HTTP/1.0 {status} Status\r\n\r\nx");
            let url = serve_once(Box::leak(with_body.into_bytes().into_boxed_slice()));
            assert_eq!(
                get(&url, 100).unwrap_err().kind(),
                io::ErrorKind::InvalidData,
                "{status}"
            );
        }
        println!("FIX VERIFIED: F432 204, 205 and 304 never return message content");
    }

    #[test]
    fn verify_f433_transfer_codings_across_single_and_combined_values() {
        for field in [
            "Transfer-Encoding: gzip",
            "transfer-encoding: chunked",
            "Transfer-Encoding: gzip, chunked",
        ] {
            let response = format!("HTTP/1.1 200 OK\r\n{field}\r\n\r\ndata");
            let url = serve_once(Box::leak(response.into_bytes().into_boxed_slice()));
            assert_eq!(
                get(&url, 100).unwrap_err().kind(),
                io::ErrorKind::InvalidData,
                "{field}"
            );
        }
        let url = serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\ndata");
        assert_eq!(get(&url, 100).unwrap().1, b"data");
        println!("FIX VERIFIED: F433 transfer-coded framing is never exposed as content");
    }

    #[test]
    fn verify_f434_response_field_grammar_across_distinct_malformed_classes() {
        for field in [
            "Broken field",
            " Content-Length: 2",
            "Content Length: 2",
            "Content-Length\t: 2",
        ] {
            let response = format!("HTTP/1.0 200 OK\r\n{field}\r\n\r\n{{}}");
            let url = serve_once(Box::leak(response.into_bytes().into_boxed_slice()));
            assert_eq!(
                get(&url, 100).unwrap_err().kind(),
                io::ErrorKind::InvalidData,
                "{field:?}"
            );
        }
        let url = serve_once(b"HTTP/1.0 200 OK\r\nX-A_B~C: value\r\n\r\n{}");
        assert_eq!(get(&url, 100).unwrap().1, b"{}");
        println!("FIX VERIFIED: F434 malformed response field lines are rejected");
    }
}

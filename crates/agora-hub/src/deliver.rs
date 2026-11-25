//! Delivery of a notification to the owner's phone or chat: `ntfy` (a topic URL) and `webhook` (Slack, Discord and
//! anything that takes a JSON `{"text"}` / `{"content"}` post). Plain HTTP/1.1 over `std` sockets, with TLS from rustls and
//! the system's root certificates for `https`. One short request per notification, bounded by timeouts; a failure is reported to the
//! caller and never stops the hub. The text is already rendered (from `locales/`); nothing here writes prose.
//!
//! Targets come from the owner's file `<hub dir>/notify`: one `ntfy <url>` or `webhook <url>` per line.

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::time::Duration;

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
        let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
            let (host, tail) = bracketed.split_once(']')?;
            host.parse::<std::net::Ipv6Addr>().ok()?;
            let port = if tail.is_empty() {
                default_port
            } else {
                tail.strip_prefix(':')?.parse().ok()?
            };
            (host, port)
        } else {
            if authority.contains(['[', ']']) || authority.matches(':').count() > 1 {
                return None;
            }
            match authority.rsplit_once(':') {
                Some((h, p)) if !p.is_empty() => (h, p.parse().ok()?),
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

fn open(url: &Url) -> io::Result<Box<dyn Stream>> {
    let tcp = agora_protocol::dial::connect_timeout((url.host.as_str(), url.port), TIMEOUT)?;
    tcp.set_read_timeout(Some(TIMEOUT))?;
    tcp.set_write_timeout(Some(TIMEOUT))?;
    if !url.https {
        return Ok(Box::new(tcp));
    }
    let name = rustls::pki_types::ServerName::try_from(url.host.clone())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
    let conn = rustls::ClientConnection::new(tls_config()?, name)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
    Ok(Box::new(rustls::StreamOwned::new(conn, tcp)))
}

/// Sends one prepared request; returns the HTTP status. Blocking: call it from a thread of its own.
pub fn send(url: &Url, bytes: &[u8]) -> io::Result<u16> {
    exchange(open(url)?, bytes)
}

/// `GET` the URL and return the status and the body (at most `max` bytes; more is an error). HTTP/1.0 so the body is not
/// chunked. Blocking.
pub fn get(url: &Url, max: usize) -> io::Result<(u16, Vec<u8>)> {
    let mut s = open(url)?;
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
            Err(e) => return Err(e),
        };
        if n == 0 {
            break;
        }
        all.extend_from_slice(&buf[..n]);
        if all.len() > max.saturating_add(MAX_RESPONSE_HEAD) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "response too large",
            ));
        }
    }
    let (status, head_at, body_at) = final_response(&all)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no final HTTP response"))?;
    // A 304 carries metadata for a selected representation but no message body; its Content-Length can describe that representation.
    let declared = (status != 304)
        .then(|| content_length(&all[head_at..body_at]))
        .transpose()?
        .flatten();
    let body = all.split_off(body_at);
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

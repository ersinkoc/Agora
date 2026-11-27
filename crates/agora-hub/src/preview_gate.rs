//! ADR-0088 Amendment A: what the hub decides about a preview before any byte reaches a node, as plain data (no sockets, the clock
//! passed in). The listeners and the relay (`preview_relay`, later slices) call into it.
//!
//! - **Origins:** a small pool of the hub's ports, one per open forward while it is carried ([`PortPool`]).
//! - **Entry:** the opener's window gets a single-use ticket (32 random bytes, 60 s); the preview origin trades it for a forward token
//!   (random, bound to the forward and the person, at most 8 hours and never past the forward's end), set as the cookie
//!   `agora_preview_<port>` ([`Gate`]).
//! - **Heads:** each relayed connection carries one request (point 8): the hub reads its head (at most 16 KiB, strict HTTP/1.1),
//!   checks this port's token, strips every `agora_preview_*` cookie, passes on only this forward's partitioned cookies with their
//!   prefix removed, rewrites `Host` to `localhost:<port>`, and asks for `Connection: close` unless it is a WebSocket upgrade
//!   ([`rewrite_request`]). On the way back it reads response heads up to the first that is not `1xx` and renames and caps each
//!   `Set-Cookie` ([`rewrite_response`]). Bodies are never read.
use std::collections::HashMap;

/// The most a request or response head may take, request line or status line included (point 3; more is answered 431).
pub const MAX_HEAD: usize = 16 * 1024;
/// How long an entry ticket is good for (point 2).
pub const TICKET_MS: u64 = 60_000;
/// The longest a forward token lives (point 2), whatever the forward's own end.
pub const TOKEN_MS: u64 = 8 * 3_600_000;
/// The cookie a preview origin's token is carried in, one name per port (point 2).
pub const COOKIE_PREFIX: &str = "agora_preview_";

/// Why a head is not passed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadError {
    /// More than [`MAX_HEAD`] without the blank line that ends it: 431.
    TooLarge,
    /// Not strict HTTP/1.1 (a bare line feed, a folded line, a space before a colon, a control character, another version): 400.
    Malformed,
}

/// Where the head in `buf` ends (the index just past its blank line), `Ok(None)` while more is needed, or too large.
pub fn head_end(buf: &[u8]) -> Result<Option<usize>, HeadError> {
    let window = &buf[..buf.len().min(MAX_HEAD)];
    if let Some(at) = window.windows(4).position(|w| w == b"\r\n\r\n") {
        return Ok(Some(at + 4));
    }
    if buf.len() >= MAX_HEAD {
        return Err(HeadError::TooLarge);
    }
    Ok(None)
}

/// One header as it came, its name kept as sent (compared without case).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub name: String,
    pub value: String,
}

/// A request or response head: its first line's parts and its headers in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    pub first: String,
    pub headers: Vec<Header>,
}

impl Head {
    fn values<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.headers
            .iter()
            .filter(move |h| h.name.eq_ignore_ascii_case(name))
            .map(|h| h.value.as_str())
    }

    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(256);
        out.extend_from_slice(self.first.as_bytes());
        out.extend_from_slice(b"\r\n");
        for h in &self.headers {
            out.extend_from_slice(h.name.as_bytes());
            out.extend_from_slice(b": ");
            out.extend_from_slice(h.value.as_bytes());
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b"\r\n");
        out
    }
}

fn tchar(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

/// Parses a head (everything up to and including its blank line) strictly: CRLF line ends only, no folded lines, a token name with no
/// space before its colon, values without control characters (a tab allowed), no NUL anywhere.
fn parse_head(head: &[u8]) -> Result<Head, HeadError> {
    let text = std::str::from_utf8(head).map_err(|_| HeadError::Malformed)?;
    let body = text.strip_suffix("\r\n\r\n").ok_or(HeadError::Malformed)?;
    let mut lines = body.split("\r\n");
    let first = lines.next().ok_or(HeadError::Malformed)?;
    if first.is_empty() || first.bytes().any(|b| b.is_ascii_control()) {
        return Err(HeadError::Malformed);
    }
    let mut headers = Vec::new();
    for line in lines {
        if line.bytes().any(|b| b == b'\n' || b == b'\r') {
            return Err(HeadError::Malformed); // a bare CR or LF inside a line
        }
        let colon = line.find(':').ok_or(HeadError::Malformed)?;
        let (name, value) = (&line[..colon], &line[colon + 1..]);
        if name.is_empty() || !name.bytes().all(tchar) {
            return Err(HeadError::Malformed); // also a folded line (it starts with a space) and a space before the colon
        }
        if value
            .bytes()
            .any(|b| (b.is_ascii_control() && b != b'\t') || b == 0x7f)
        {
            return Err(HeadError::Malformed);
        }
        headers.push(Header {
            name: name.to_string(),
            value: value.trim_matches(|c| c == ' ' || c == '\t').to_string(),
        });
    }
    Ok(Head {
        first: first.to_string(),
        headers,
    })
}

/// A request head: `METHOD SP target SP HTTP/1.1`, with a token method and a target without spaces.
pub fn parse_request(head: &[u8]) -> Result<Head, HeadError> {
    let h = parse_head(head)?;
    let mut parts = h.first.split(' ');
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(HeadError::Malformed);
    };
    if method.is_empty() || !method.bytes().all(tchar) || target.is_empty() || version != "HTTP/1.1"
    {
        return Err(HeadError::Malformed);
    }
    Ok(h)
}

/// A response head: `HTTP/1.x SP 3DIGIT SP reason`; its status code.
pub fn parse_response(head: &[u8]) -> Result<(u16, Head), HeadError> {
    let h = parse_head(head)?;
    let rest = h
        .first
        .strip_prefix("HTTP/1.1 ")
        .or_else(|| h.first.strip_prefix("HTTP/1.0 "))
        .ok_or(HeadError::Malformed)?;
    let code = rest.get(..3).ok_or(HeadError::Malformed)?;
    if !code.bytes().all(|b| b.is_ascii_digit()) || !(rest.len() == 3 || rest.as_bytes()[3] == b' ')
    {
        return Err(HeadError::Malformed);
    }
    let status: u16 = code.parse().map_err(|_| HeadError::Malformed)?;
    Ok((status, h))
}

/// What one preview port carries: the forward it fronts and how its cookies are partitioned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    /// The hub's preview port (the browser's origin).
    pub port: u16,
    /// Short and random per forward: a forward that gets a port another one used starts with an empty jar (point 8).
    pub tag: String,
    /// The forward's port on the node, for `Host: localhost:<port>`.
    pub node_port: u16,
}

impl Partition {
    /// `p<port><tag>_`: the prefix this forward's cookies carry in the browser.
    pub fn prefix(&self) -> String {
        format!("p{}{}_", self.port, self.tag)
    }
    /// The cookie this port's forward token is carried in.
    pub fn token_cookie(&self) -> String {
        format!("{COOKIE_PREFIX}{}", self.port)
    }
}

/// A request head ready to go on, and what the gate must still check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rewritten {
    /// The head to send to the program.
    pub head: Vec<u8>,
    /// This port's forward token, if the request carried one (another port's is never taken).
    pub token: Option<String>,
    /// The request asks for a WebSocket: after a `101` the connection is relayed as opaque bytes.
    pub upgrade: bool,
    /// The path asked for (the entry path is answered by the hub itself).
    pub target: String,
}

fn has_token(list: &str, token: &str) -> bool {
    list.split(',')
        .any(|t| t.trim().eq_ignore_ascii_case(token))
}

/// Rewrites a request head for the program behind `p` (see the module's header). Hop-by-hop headers about the connection are replaced:
/// a plain request asks for `Connection: close`, an upgrade keeps `Connection: Upgrade` and its `Upgrade`.
pub fn rewrite_request(raw: &[u8], p: &Partition) -> Result<Rewritten, HeadError> {
    let mut h = parse_request(raw)?;
    let target = h.first.split(' ').nth(1).unwrap_or("/").to_string();
    let upgrade = h.values("connection").any(|v| has_token(v, "upgrade"))
        && h.values("upgrade").next().is_some();
    let ours = p.token_cookie();
    let prefix = p.prefix();
    let mut token = None;
    let mut kept = Vec::new();
    for v in h.values("cookie") {
        for pair in v.split(';') {
            let pair = pair.trim();
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            if name == ours {
                // The first wins; a browser sends the one set for this host and path.
                token.get_or_insert_with(|| value.to_string());
            } else if let Some(app) = name.strip_prefix(prefix.as_str()) {
                if !app.is_empty() {
                    kept.push(format!("{app}={value}"));
                }
            }
            // Anything else (another forward's cookies, any `agora_preview_*`) never goes on.
        }
    }
    h.headers.retain(|x| {
        !(x.name.eq_ignore_ascii_case("cookie")
            || x.name.eq_ignore_ascii_case("host")
            || x.name.eq_ignore_ascii_case("connection")
            || x.name.eq_ignore_ascii_case("keep-alive")
            || x.name.eq_ignore_ascii_case("proxy-connection")
            || (!upgrade && x.name.eq_ignore_ascii_case("upgrade")))
    });
    h.headers.insert(
        0,
        Header {
            name: "Host".into(),
            value: format!("localhost:{}", p.node_port),
        },
    );
    if !kept.is_empty() {
        h.headers.push(Header {
            name: "Cookie".into(),
            value: kept.join("; "),
        });
    }
    h.headers.push(Header {
        name: "Connection".into(),
        value: if upgrade { "Upgrade" } else { "close" }.into(),
    });
    Ok(Rewritten {
        head: h.encode(),
        token,
        upgrade,
        target,
    })
}

/// IMF-fixdate (`Sun, 06 Nov 1994 08:49:37 GMT`) as Unix seconds; `None` for anything else.
fn http_date(s: &str) -> Option<i64> {
    let s = s.trim();
    let rest = s.get(5..)?; // "Sun, "
    let mut it = rest.split(' ');
    let (day, mon, year, clock, zone) =
        (it.next()?, it.next()?, it.next()?, it.next()?, it.next()?);
    if zone != "GMT" || it.next().is_some() {
        return None;
    }
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let month = MONTHS.iter().position(|m| *m == mon)? as u8 + 1;
    let mut c = clock.split(':');
    let (hh, mm, ss): (u8, u8, u8) = (
        c.next()?.parse().ok()?,
        c.next()?.parse().ok()?,
        c.next()?.parse().ok()?,
    );
    let date = time::Date::from_calendar_date(
        year.parse().ok()?,
        time::Month::try_from(month).ok()?,
        day.parse().ok()?,
    )
    .ok()?;
    let t = time::Time::from_hms(hh, mm, ss).ok()?;
    Some(
        time::PrimitiveDateTime::new(date, t)
            .assume_utc()
            .unix_timestamp(),
    )
}

/// One `Set-Cookie` renamed into this forward's partition, its lifetime capped at `ends_s` (Unix seconds) seen from `now_s`, and its
/// `Domain` removed (a cookie stays on the preview's own host). `None` for a line without a name.
fn rewrite_set_cookie(v: &str, prefix: &str, now_s: i64, ends_s: i64) -> Option<String> {
    let mut parts = v.split(';');
    let first = parts.next()?.trim();
    let (name, value) = first.split_once('=')?;
    let name = name.trim();
    if name.is_empty() || !name.bytes().all(tchar) {
        return None;
    }
    let left = (ends_s - now_s).max(0);
    let mut max_age: Option<i64> = None;
    let mut expires: Option<i64> = None;
    let mut rest = Vec::new();
    for a in parts {
        let a = a.trim();
        let (k, val) = a
            .split_once('=')
            .map_or((a, ""), |(k, v)| (k.trim(), v.trim()));
        if k.eq_ignore_ascii_case("max-age") {
            max_age = Some(val.parse().unwrap_or(0));
        } else if k.eq_ignore_ascii_case("expires") {
            expires = Some(http_date(val).map_or(0, |at| at - now_s));
        } else if k.eq_ignore_ascii_case("domain") || a.is_empty() {
            // Dropped: the cookie stays host-only.
        } else {
            rest.push(a.to_string());
        }
    }
    // Max-Age wins over Expires, as in a browser; neither is a session cookie, which still ends with the forward.
    let life = max_age.or(expires).map_or(left, |l| l.min(left));
    let mut out = format!("{prefix}{name}={value}; Max-Age={life}");
    for a in rest {
        out.push_str("; ");
        out.push_str(&a);
    }
    Some(out)
}

/// A response head with every `Set-Cookie` moved into this forward's partition (point 6, with point 8's tag and cap); its status code
/// tells the relay whether another head follows (`1xx` other than `101`).
pub fn rewrite_response(
    raw: &[u8],
    p: &Partition,
    now_ms: u64,
    ends_ms: u64,
) -> Result<(u16, Vec<u8>), HeadError> {
    let (status, mut h) = parse_response(raw)?;
    let prefix = p.prefix();
    let (now_s, ends_s) = ((now_ms / 1000) as i64, (ends_ms / 1000) as i64);
    let mut out = Vec::with_capacity(h.headers.len());
    for x in h.headers.drain(..) {
        if x.name.eq_ignore_ascii_case("set-cookie") {
            if let Some(v) = rewrite_set_cookie(&x.value, &prefix, now_s, ends_s) {
                out.push(Header {
                    name: x.name,
                    value: v,
                });
            }
        } else {
            out.push(x);
        }
    }
    h.headers = out;
    Ok((status, h.encode()))
}

/// A forward the hub carries on one of its ports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Carried {
    pub node: String,
    pub forward: String,
    /// The person who opened it: the only one who may enter (point 2).
    pub user: String,
    pub partition: Partition,
    /// When the forward ends (its expiry as the node announced it), Unix ms.
    pub ends_ms: u64,
}

/// The pool of preview ports: one per carried forward, given back when it ends.
#[derive(Debug)]
pub struct PortPool {
    ports: Vec<u16>,
    used: HashMap<u16, (String, String)>,
}

impl PortPool {
    /// The ports `first..=last` (default: the 16 after the hub's own).
    pub fn new(first: u16, last: u16) -> Self {
        Self {
            ports: (first..=last).collect(),
            used: HashMap::new(),
        }
    }
    /// The port already given to `(node, forward)`, else a free one (lowest first) that `usable` accepts (the caller tries to bind it),
    /// else `None` (`preview.unavailable` with `no_origin`).
    pub fn take(
        &mut self,
        node: &str,
        forward: &str,
        mut usable: impl FnMut(u16) -> bool,
    ) -> Option<u16> {
        if let Some((&p, _)) = self
            .used
            .iter()
            .find(|(_, (n, f))| n == node && f == forward)
        {
            return Some(p);
        }
        let free: Vec<u16> = self
            .ports
            .iter()
            .copied()
            .filter(|p| !self.used.contains_key(p))
            .collect();
        let port = free.into_iter().find(|&p| usable(p))?;
        self.used
            .insert(port, (node.to_string(), forward.to_string()));
        Some(port)
    }
    pub fn release(&mut self, port: u16) {
        self.used.remove(&port);
    }
    /// Every port of the pool (written where the node reads the hub's ports, so no forward can point at one).
    pub fn all(&self) -> &[u16] {
        &self.ports
    }
}

/// Entry tickets and forward tokens, with the clock passed in.
#[derive(Debug, Default)]
pub struct Gate {
    /// ticket → (port, user, expires).
    tickets: HashMap<String, (u16, String, u64)>,
    /// token → (port, user, expires).
    tokens: HashMap<String, (u16, String, u64)>,
}

/// 32 random bytes as hex (a ticket or a token); `None` when the system has no randomness to give (nothing is then issued).
pub fn random_hex() -> Option<String> {
    let mut b = [0u8; 32];
    getrandom::getrandom(&mut b).ok()?;
    Some(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// Compares two secrets in time that does not depend on where they differ.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

impl Gate {
    /// A ticket for `user` to enter the forward on `port`: single use, [`TICKET_MS`].
    pub fn ticket(&mut self, port: u16, user: &str, now_ms: u64) -> Option<String> {
        self.tickets.retain(|_, (_, _, until)| *until > now_ms);
        let t = random_hex()?;
        self.tickets
            .insert(t.clone(), (port, user.to_string(), now_ms + TICKET_MS));
        Some(t)
    }

    /// Trades a ticket presented on `port` for a forward token (`None`: unknown, used, late, or for another port). The token lives
    /// [`TOKEN_MS`] and never past `forward_ends_ms`.
    pub fn enter(
        &mut self,
        ticket: &str,
        port: u16,
        now_ms: u64,
        forward_ends_ms: u64,
    ) -> Option<String> {
        let (p, user, until) = self.tickets.remove(ticket)?; // removed whatever follows: single use
        if p != port || until <= now_ms {
            return None;
        }
        let token = random_hex()?;
        self.tokens.insert(
            token.clone(),
            (port, user, (now_ms + TOKEN_MS).min(forward_ends_ms)),
        );
        Some(token)
    }

    /// The person a token presented on `port` belongs to, while it is good.
    pub fn check(&self, token: &str, port: u16, now_ms: u64) -> Option<&str> {
        self.tokens
            .iter()
            .find(|(t, (p, _, until))| *p == port && *until > now_ms && same(t, token))
            .map(|(_, (_, user, _))| user.as_str())
    }

    /// Ends every ticket and token of a port (its forward ended, or the port was given back).
    pub fn end_port(&mut self, port: u16) {
        self.tickets.retain(|_, (p, _, _)| *p != port);
        self.tokens.retain(|_, (p, _, _)| *p != port);
    }

    /// Ends every ticket and token of a person (signed out, removed).
    pub fn end_user(&mut self, user: &str) {
        self.tickets.retain(|_, (_, u, _)| u != user);
        self.tokens.retain(|_, (_, u, _)| u != user);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn p() -> Partition {
        Partition {
            port: 7450,
            tag: "f3a".into(),
            node_port: 5173,
        }
    }
    fn text(b: &[u8]) -> String {
        String::from_utf8(b.to_vec()).unwrap()
    }

    #[test]
    fn a_head_ends_at_its_blank_line_and_never_past_16_kib() {
        assert_eq!(
            head_end(b"GET / HTTP/1.1\r\nHost: a\r\n\r\nbody"),
            Ok(Some(27))
        );
        assert_eq!(head_end(b"GET / HTTP/1.1\r\nHost: a\r\n"), Ok(None));
        let mut big = b"GET / HTTP/1.1\r\nX: ".to_vec();
        big.resize(MAX_HEAD, b'a');
        assert_eq!(head_end(&big), Err(HeadError::TooLarge));
    }

    #[test]
    fn only_strict_http_1_1_heads_are_read() {
        let ok = b"GET /a?b=1 HTTP/1.1\r\nHost: hub:7450\r\nAccept: */*\r\n\r\n";
        assert!(parse_request(ok).is_ok());
        for bad in [
            &b"GET / HTTP/1.1\nHost: a\n\n"[..], // bare line feeds
            b"GET / HTTP/1.1\r\nHost: a\r\n folded\r\n\r\n", // a folded line
            b"GET / HTTP/1.1\r\nHost : a\r\n\r\n", // a space before the colon
            b"GET / HTTP/1.0\r\nHost: a\r\n\r\n", // another version
            b"GET / HTTP/2\r\nHost: a\r\n\r\n",
            b"GET  / HTTP/1.1\r\nHost: a\r\n\r\n", // two spaces
            b"GET / HTTP/1.1\r\nX: a\x00b\r\n\r\n", // a NUL
            b"GET / HTTP/1.1\r\nX: a\rb\r\n\r\n",  // a bare CR
            b"G(T / HTTP/1.1\r\nHost: a\r\n\r\n",  // not a token
            b"GET / HTTP/1.1\r\nNoColon\r\n\r\n",
            b"\xff / HTTP/1.1\r\n\r\n",
        ] {
            assert_eq!(parse_request(bad), Err(HeadError::Malformed), "{bad:?}");
        }
        assert_eq!(parse_response(b"HTTP/1.1 200 OK\r\n\r\n").unwrap().0, 200);
        assert_eq!(parse_response(b"HTTP/1.0 404\r\n\r\n").unwrap().0, 404);
        assert_eq!(
            parse_response(b"HTTP/1.1 2000 OK\r\n\r\n"),
            Err(HeadError::Malformed)
        );
    }

    #[test]
    fn a_request_goes_on_with_only_this_forwards_cookies_and_never_a_token() {
        let raw = b"GET /app HTTP/1.1\r\nHost: hub.example:7450\r\nConnection: keep-alive\r\nKeep-Alive: timeout=5\r\n\
Cookie: agora_preview_7450=tok; agora_preview_7451=other; p7450f3a_session=s1; p7451zzz_session=s2; p7450old_x=1; plain=1\r\n\
Cookie: p7450f3a_theme=dark\r\nAccept: text/html\r\n\r\n";
        let r = rewrite_request(raw, &p()).unwrap();
        let head = text(&r.head);
        assert_eq!(r.token.as_deref(), Some("tok"));
        assert!(!r.upgrade);
        assert_eq!(r.target, "/app");
        assert!(
            head.starts_with("GET /app HTTP/1.1\r\nHost: localhost:5173\r\n"),
            "{head}"
        );
        assert!(
            head.contains("\r\nCookie: session=s1; theme=dark\r\n"),
            "{head}"
        );
        assert!(head.ends_with("Connection: close\r\n\r\n"), "{head}");
        for gone in [
            "agora_preview",
            "other",
            "s2",
            "p7450old",
            "plain=1",
            "keep-alive",
            "Keep-Alive",
            "hub.example",
        ] {
            assert!(!head.contains(gone), "{gone} in {head}");
        }
        assert!(head.contains("Accept: text/html"));
    }

    #[test]
    fn another_ports_token_is_never_taken_and_an_upgrade_stays_an_upgrade() {
        let raw = b"GET /ws HTTP/1.1\r\nHost: h\r\nCookie: agora_preview_7451=x\r\nConnection: keep-alive, Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Key: k\r\n\r\n";
        let r = rewrite_request(raw, &p()).unwrap();
        assert_eq!(r.token, None);
        assert!(r.upgrade);
        let head = text(&r.head);
        assert!(
            head.contains("Upgrade: websocket") && head.ends_with("Connection: Upgrade\r\n\r\n"),
            "{head}"
        );
        assert!(!head.contains("Cookie"));
        // Upgrade without Connection: Upgrade is not an upgrade, and its Upgrade header is dropped.
        let r = rewrite_request(b"GET / HTTP/1.1\r\nUpgrade: websocket\r\n\r\n", &p()).unwrap();
        assert!(!r.upgrade && !text(&r.head).contains("websocket"));
    }

    #[test]
    fn set_cookie_moves_into_the_forwards_partition_and_ends_with_it() {
        let now_ms = 1_000_000_000_000; // 2001-09-09T01:46:40Z
        let ends_ms = now_ms + 3_600_000;
        let raw = b"HTTP/1.1 200 OK\r\nSet-Cookie: session=abc; Path=/; HttpOnly; Domain=example.com\r\n\
Set-Cookie: long=1; Max-Age=999999\r\nSet-Cookie: short=1; Max-Age=60\r\nSet-Cookie: dated=1; Expires=Sun, 09 Sep 2001 01:56:40 GMT\r\n\
Set-Cookie: late=1; Expires=Sun, 06 Nov 2094 08:49:37 GMT\r\nSet-Cookie: =nameless\r\nContent-Type: text/html\r\n\r\n";
        let (status, head) = rewrite_response(raw, &p(), now_ms, ends_ms).unwrap();
        assert_eq!(status, 200);
        let head = text(&head);
        assert!(
            head.contains("Set-Cookie: p7450f3a_session=abc; Max-Age=3600; Path=/; HttpOnly\r\n"),
            "{head}"
        );
        assert!(!head.contains("Domain"));
        assert!(
            head.contains("p7450f3a_long=1; Max-Age=3600\r\n"),
            "capped: {head}"
        );
        assert!(head.contains("p7450f3a_short=1; Max-Age=60\r\n"), "{head}");
        assert!(
            head.contains("p7450f3a_dated=1; Max-Age=600\r\n"),
            "Expires read: {head}"
        );
        assert!(head.contains("p7450f3a_late=1; Max-Age=3600\r\n"), "{head}");
        assert!(!head.contains("nameless"));
        assert!(head.contains("Content-Type: text/html"));
        assert_eq!(
            rewrite_response(
                b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n",
                &p(),
                0,
                1
            )
            .unwrap()
            .0,
            101
        );
    }

    #[test]
    fn a_ticket_is_single_use_short_and_for_its_port_and_a_token_ends_with_its_forward() {
        let mut g = Gate::default();
        let t = g.ticket(7450, "ada", 1_000).unwrap();
        assert_eq!(t.len(), 64);
        assert_eq!(g.enter(&t, 7451, 1_001, u64::MAX), None, "another port");
        assert_eq!(
            g.enter(&t, 7450, 1_002, u64::MAX),
            None,
            "already spent by the wrong port"
        );
        let t = g.ticket(7450, "ada", 1_000).unwrap();
        assert_eq!(g.enter(&t, 7450, 1_000 + TICKET_MS, u64::MAX), None, "late");
        let t = g.ticket(7450, "ada", 1_000).unwrap();
        let tok = g.enter(&t, 7450, 2_000, 10_000).unwrap();
        assert_eq!(g.enter(&t, 7450, 2_001, 10_000), None, "single use");
        assert_eq!(g.check(&tok, 7450, 9_999), Some("ada"));
        assert_eq!(
            g.check(&tok, 7450, 10_000),
            None,
            "never past the forward's end"
        );
        assert_eq!(g.check(&tok, 7451, 3_000), None, "never on another port");
        let t = g.ticket(7450, "ada", 1_000).unwrap();
        let long = g.enter(&t, 7450, 2_000, u64::MAX).unwrap();
        assert_eq!(
            g.check(&long, 7450, 2_000 + TOKEN_MS),
            None,
            "8 hours at most"
        );
        g.end_user("ada");
        assert_eq!(g.check(&long, 7450, 3_000), None);
        let t = g.ticket(7450, "bo", 1_000).unwrap();
        let tok = g.enter(&t, 7450, 2_000, u64::MAX).unwrap();
        g.end_port(7450);
        assert_eq!(g.check(&tok, 7450, 3_000), None);
    }

    #[test]
    fn the_pool_gives_each_forward_one_port_and_takes_it_back() {
        let mut pool = PortPool::new(7450, 7452);
        assert_eq!(pool.take("n", "fwd-a", |_| true), Some(7450));
        assert_eq!(
            pool.take("n", "fwd-a", |_| true),
            Some(7450),
            "the same forward, the same port"
        );
        assert_eq!(
            pool.take("m", "fwd-a", |p| p != 7451),
            Some(7452),
            "keyed by node too; a port that cannot bind is skipped"
        );
        assert_eq!(pool.take("n", "fwd-b", |_| true), Some(7451));
        assert_eq!(pool.take("n", "fwd-c", |_| true), None, "no origin left");
        pool.release(7451);
        assert_eq!(pool.take("n", "fwd-c", |_| true), Some(7451));
        assert_eq!(pool.all(), &[7450, 7451, 7452]);
    }
}

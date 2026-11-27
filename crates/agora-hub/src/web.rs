//! Serves the web app over plain HTTP on the same port as the WebSocket service.
//!
//! The first request head of every connection is read before anything else. A WebSocket upgrade is replayed
//! to the WebSocket handshake unchanged (`Rewind`); any other `GET` is answered from `Assets`.

use std::future::Future;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};

pub use crate::password::Client;

/// A sign-in: the one-time code, the screen password when the client gave one (ADR-0067), and who is asking.
pub struct SignInRequest<'a> {
    pub code: &'a str,
    pub password: Option<&'a str>,
    pub client: &'a Client,
}

pub type SessionReply<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What the sign-in routes need from the hub: `POST /api/session` trades a one-time code (and, from another machine, the screen
/// password) for a session token; `GET /api/session` says whether the password is needed; `DELETE /api/session` signs out.
pub trait SessionApi: Send + Sync {
    fn sign_in<'a>(&'a self, req: &'a SignInRequest<'a>) -> SessionReply<'a, SignInReply>;

    /// `GET /api/session`: what the sign-in screen needs to know before it asks (`passwordRequired`, `passwordSet`). Additive JSON.
    fn info(&self, _client: &Client) -> serde_json::Value {
        serde_json::json!({})
    }

    /// ADR-0067 rung 4: the proxy trust source, consulted for every request.
    fn proxy(&self) -> Option<&crate::proxy::ProxyHandle> {
        None
    }

    /// `DELETE /api/session`: ends the session `bearer` (sign out, "Lock"). Returns the status.
    fn end(&self, _bearer: &str) -> u16 {
        404
    }

    /// `POST /api/password` (`op` = `password`: set or change the screen password). Returns the status and the JSON body.
    fn account<'a>(
        &'a self,
        _op: &'a str,
        _bearer: Option<&'a str>,
        _client: &'a Client,
        _body: &'a serde_json::Value,
    ) -> SessionReply<'a, (u16, String)> {
        Box::pin(async { (404, String::new()) })
    }

    /// `POST /api/passkey/<op>` (WebAuthn, ADR-0015): `bearer` is the session token of the request, `host` its `Host`
    /// header. Returns the status and the JSON body. Hubs without passkeys answer 404.
    fn passkey<'a>(
        &'a self,
        _op: &'a str,
        _bearer: Option<&'a str>,
        _host: &'a str,
        _client: &'a Client,
        _body: &'a serde_json::Value,
    ) -> SessionReply<'a, (u16, String)> {
        Box::pin(async { (404, String::new()) })
    }

    /// `GET /api/audit?since=<ms>&who=<id>&limit=<n>` (ADR-0049): the audit trail as a JSON array, for the owner only. Hubs without
    /// sessions answer 404.
    fn audit(&self, _bearer: Option<&str>, _query: &str) -> (u16, String) {
        (404, String::new())
    }

    /// `GET /api/metrics` (architecture review R8): the hub's own numbers in the Prometheus text format, for the owner or for a
    /// metrics token (ADR-0076), and only when the hub was started with `--metrics`; 404 otherwise. A bearer that is neither counts
    /// toward the address's lockout; a locked-out address gets 429.
    fn metrics(&self, _bearer: Option<&str>, _client: &Client) -> (u16, String) {
        (404, String::new())
    }

    /// Whether `GET /api/metrics` is on (`agora hub run --metrics`). Off, the path is answered as any unknown path.
    fn metrics_on(&self) -> bool {
        false
    }

    /// `POST /api/preview/enter` (ADR-0088 Amendment A, point 2): `{"node", "forward"}` from the session `bearer`, asked through a
    /// request whose `Host` header was `host`; `{"port", "ticket"}` for the forward's opener, else a stable code. Hubs without
    /// previews answer 404.
    fn preview_enter(
        &self,
        _bearer: Option<&str>,
        _host: &str,
        _body: &serde_json::Value,
    ) -> (u16, String) {
        (404, String::new())
    }
}

pub enum SignInReply {
    Token(String),
    /// A wrong, expired or used code, or a wrong or missing password: one answer on purpose.
    Refused,
    /// Too many wrong tries from here (or from everywhere); try later.
    TooMany,
    /// All screen-password workers for this client are occupied.
    Busy,
    /// The hub could not do it (its store failed).
    Failed,
}

/// How long a refused sign-in waits before it is answered: a fixed cost for every wrong guess, the same for each kind.
pub const FAILURE_DELAY: std::time::Duration = std::time::Duration::from_millis(250);

/// Largest body of the sign-in request.
const MAX_BODY: usize = 8 * 1024;

/// Largest request head accepted before the connection is dropped.
const MAX_HEAD: usize = 16 * 1024;

/// One file the hub can serve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

/// Where the web app comes from.
pub trait Assets: Send + Sync {
    /// `path` is the request path without the query, starting with `/`.
    fn get(&self, path: &str) -> Option<Asset>;
}

/// Assets read from a built web app on disk (`apps/web/dist`).
pub struct DirAssets {
    root: PathBuf,
}

impl DirAssets {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn read(&self, rel: &Path) -> Option<Asset> {
        let body = std::fs::read(self.root.join(rel)).ok()?;
        Some(Asset {
            content_type: content_type(rel),
            body,
        })
    }
}

impl Assets for DirAssets {
    fn get(&self, path: &str) -> Option<Asset> {
        resolve(path, |rel| self.read(rel))
    }
}

/// The page's install manifest (ADR-0046): installable to a phone's home screen, no service worker. The name is the locale's
/// product string; colours are the design tokens' dark surface and accent. Besides the mark as SVG, PNG sizes for launchers that
/// need them, and a maskable one with the mark inside the safe zone (ADR-0067 rung 2); the files are the web app's own.
pub fn manifest(name: &str) -> Vec<u8> {
    serde_json::json!({
        "id": "/",
        "name": name,
        "short_name": name,
        "start_url": "/",
        "scope": "/",
        "display": "standalone",
        "background_color": "#09090b",
        "theme_color": "#09090b",
        "icons": [
            { "src": "/icon.svg", "sizes": "any", "type": "image/svg+xml", "purpose": "any" },
            { "src": "/icon-192.png", "sizes": "192x192", "type": "image/png", "purpose": "any" },
            { "src": "/icon-512.png", "sizes": "512x512", "type": "image/png", "purpose": "any" },
            { "src": "/icon-maskable-512.png", "sizes": "512x512", "type": "image/png", "purpose": "maskable" },
        ],
    })
    .to_string()
    .into_bytes()
}

/// `inner` plus `/manifest.webmanifest`, which is answered here and never read from the folder.
pub struct Installable {
    pub inner: Arc<dyn Assets>,
    pub manifest: Vec<u8>,
}

impl Assets for Installable {
    fn get(&self, path: &str) -> Option<Asset> {
        if path == "/manifest.webmanifest" {
            return Some(Asset {
                content_type: content_type(Path::new("manifest.webmanifest")),
                body: self.manifest.clone(),
            });
        }
        self.inner.get(path)
    }
}

/// Assets compiled into the binary: `(path relative to the web root, bytes)`.
pub struct EmbeddedAssets(pub &'static [(&'static str, &'static [u8])]);

impl Assets for EmbeddedAssets {
    fn get(&self, path: &str) -> Option<Asset> {
        resolve(path, |rel| {
            let want = rel.to_str()?.replace('\\', "/");
            let (_, body) = self.0.iter().find(|(name, _)| *name == want)?;
            Some(Asset {
                content_type: content_type(rel),
                body: body.to_vec(),
            })
        })
    }
}

/// The lookup rule both sources share: `/` is the app shell, a missing path without an extension is a client-side
/// route and also gets the shell, and a missing file does not.
fn resolve(path: &str, read: impl Fn(&Path) -> Option<Asset>) -> Option<Asset> {
    let rel = safe_relative(path)?;
    if rel.as_os_str().is_empty() {
        return read(Path::new("index.html"));
    }
    match read(&rel) {
        Some(a) => Some(a),
        None if rel.extension().is_none() => read(Path::new("index.html")),
        None => None,
    }
}

/// Turns a request path into a relative path that cannot leave the root: only plain components survive.
pub fn safe_relative(path: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in path.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part.contains('\\') || part.contains('\0') || part.contains(':') {
            return None;
        }
        let comp = Path::new(part).components().next()?;
        match comp {
            Component::Normal(c) => out.push(c),
            _ => return None,
        }
    }
    Some(out)
}

fn content_type(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("webmanifest") => "application/manifest+json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("woff2") => "font/woff2",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}

/// A stream that first yields bytes already read, then the rest of the underlying stream.
pub struct Rewind<S> {
    prefix: io::Cursor<Vec<u8>>,
    inner: S,
}

impl<S> Rewind<S> {
    pub fn new(prefix: Vec<u8>, inner: S) -> Self {
        Self {
            prefix: io::Cursor::new(prefix),
            inner,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Rewind<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let pos = self.prefix.position() as usize;
        let data = self.prefix.get_ref();
        if pos < data.len() {
            let n = (data.len() - pos).min(buf.remaining());
            buf.put_slice(&data[pos..pos + n]);
            self.prefix.set_position((pos + n) as u64);
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Rewind<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// What the first request on a connection turned out to be.
pub enum First<S> {
    /// A WebSocket upgrade; the stream replays the bytes already read.
    WebSocket(Rewind<S>),
    /// Anything else was answered (or refused); the connection is done.
    Answered,
}

/// Header names (lower case) that a proxy adds and a browser on the hub's own machine never sends, as whole names and as prefixes.
const FORWARDED_NAMES: [&str; 5] = [
    "forwarded",
    "via",
    "x-real-ip",
    "x-client-ip",
    "true-client-ip",
];
const FORWARDED_PREFIXES: [&str; 7] = [
    "x-forwarded-",
    "x-original-forwarded-",
    "x-envoy-",
    "tailscale-",
    "cf-",
    "fastly-",
    "x-real-",
];

/// True when the request head has a header that only a proxy adds. Any value counts, an empty one and a repeated one too: the
/// name is the evidence.
fn forwarded(head: &str) -> bool {
    head.split("\r\n")
        .skip(1)
        .filter_map(|l| l.split_once(':'))
        .map(|(k, _)| k.trim().to_ascii_lowercase())
        .any(|k| {
            FORWARDED_NAMES.contains(&k.as_str())
                || FORWARDED_PREFIXES.iter().any(|p| k.starts_with(p))
        })
}

/// One valid, consistent decimal Content-Length. Repeated and comma-joined copies are allowed only when every value agrees.
fn request_content_length(head: &str) -> Result<Option<usize>, ()> {
    let mut declared = None;
    for line in head.split("\r\n").skip(1).take_while(|l| !l.is_empty()) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.eq_ignore_ascii_case("content-length") {
            continue;
        }
        for item in value.split(',').map(str::trim) {
            if item.is_empty() || !item.bytes().all(|b| b.is_ascii_digit()) {
                return Err(());
            }
            let value = item.parse::<usize>().map_err(|_| ())?;
            if declared.is_some_and(|earlier| earlier != value) {
                return Err(());
            }
            declared = Some(value);
        }
    }
    Ok(declared)
}

fn token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
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
}

/// An HTTP/1 request line with exactly one method, target and supported version.
fn request_parts(head: &str) -> Option<(&str, &str)> {
    let line = head.split("\r\n").next()?;
    let mut parts = line.split(' ');
    let (method, target, version) = (parts.next()?, parts.next()?, parts.next()?);
    (parts.next().is_none()
        && !method.is_empty()
        && method.bytes().all(token_byte)
        && !target.is_empty()
        && matches!(version, "HTTP/1.0" | "HTTP/1.1"))
    .then_some((method, target))
}

/// Field lines this small server can parse without normalizing an invalid name or accepting an obsolete continuation.
fn valid_request_fields(head: &str) -> bool {
    head.split("\r\n")
        .skip(1)
        .take_while(|line| !line.is_empty())
        .all(|line| {
            line.split_once(':').is_some_and(|(name, value)| {
                !name.is_empty()
                    && name.bytes().all(token_byte)
                    && !value.bytes().any(|b| (b < b' ' && b != b'\t') || b == 0x7f)
            })
        })
}

fn header_values<'a>(head: &'a str, wanted: &'a str) -> impl Iterator<Item = &'a str> {
    head.split("\r\n")
        .skip(1)
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_once(':'))
        .filter(move |(name, _)| name.eq_ignore_ascii_case(wanted))
        .map(|(_, value)| value.trim())
}

fn single_header(head: &str, wanted: &str) -> Result<Option<String>, ()> {
    let mut values = header_values(head, wanted);
    let value = values.next().map(str::to_string);
    if values.next().is_some() {
        return Err(());
    }
    Ok(value)
}

/// Reads the request head. A WebSocket upgrade is handed back untouched; other requests are answered here.
pub async fn route<S>(
    stream: S,
    peer: Option<std::net::IpAddr>,
    assets: Option<&dyn Assets>,
    sessions: Option<&dyn SessionApi>,
) -> io::Result<First<S>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    route_within(stream, peer, assets, sessions, crate::server::NODE_SILENCE).await
}

/// [`route`] where the request (its head, and the body of a POST) must have arrived within `read` of the first byte asked for;
/// a peer that is slower, or says nothing, is dropped unanswered (audit F880). Only reading is timed: an answer is written at the
/// pace the peer takes it.
pub async fn route_within<S>(
    mut stream: S,
    peer: Option<std::net::IpAddr>,
    assets: Option<&dyn Assets>,
    sessions: Option<&dyn SessionApi>,
    read: std::time::Duration,
) -> io::Result<First<S>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let until = tokio::time::Instant::now() + read;
    let mut head = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    let end = loop {
        if let Some(i) = find(&head, b"\r\n\r\n") {
            let end = i + 4;
            if end > MAX_HEAD {
                respond(&mut stream, 431, "text/plain", b"", false).await?;
                return Ok(First::Answered);
            }
            break end;
        }
        if head.len() > MAX_HEAD {
            respond(&mut stream, 431, "text/plain", b"", false).await?;
            return Ok(First::Answered);
        }
        let Ok(n) = tokio::time::timeout_at(until, stream.read(&mut chunk)).await else {
            return Ok(First::Answered);
        };
        let n = n?;
        if n == 0 {
            return Ok(First::Answered);
        }
        head.extend_from_slice(&chunk[..n]);
    };
    let text = String::from_utf8_lossy(&head[..end]).to_string();
    // ADR-0067 rung 4: a request from a trusted proxy (loopback, or an address the owner named) is judged before anything else, a
    // WebSocket upgrade included: its Host and Origin must be ours, and its client address is the one the owner's header carries
    // (or unknown). A peer that is not the proxy is an ordinary client: no header it sends counts.
    let mut peer = peer;
    let trust = match sessions.and_then(SessionApi::proxy) {
        Some(proxy) => proxy.current().await?,
        None => None,
    };
    if let (Some(ip), Some(trust)) = (peer, trust) {
        if trust.trusts_peer(ip) {
            match trust.judge(&text) {
                crate::proxy::Verdict::Refuse { status, code } => {
                    let body = format!(r#"{{"error":"{code}"}}"#);
                    respond(
                        &mut stream,
                        status,
                        "application/json",
                        body.as_bytes(),
                        false,
                    )
                    .await?;
                    return Ok(First::Answered);
                }
                crate::proxy::Verdict::Proxied(real) => peer = real,
                crate::proxy::Verdict::Direct => {}
            }
        }
    }
    let Some((method, target)) = request_parts(&text) else {
        respond(&mut stream, 400, "text/plain", b"", false).await?;
        return Ok(First::Answered);
    };
    if !valid_request_fields(&text) || header_values(&text, "transfer-encoding").next().is_some() {
        respond(&mut stream, 400, "text/plain", b"", false).await?;
        return Ok(First::Answered);
    }
    let authorization = match single_header(&text, "authorization") {
        Ok(value) => value,
        Err(()) => {
            respond(&mut stream, 400, "text/plain", b"", false).await?;
            return Ok(First::Answered);
        }
    };
    let upgrade = text.split("\r\n").skip(1).any(|l| {
        l.split_once(':').is_some_and(|(k, v)| {
            k.eq_ignore_ascii_case("upgrade") && v.trim().eq_ignore_ascii_case("websocket")
        })
    });
    if upgrade {
        return Ok(First::WebSocket(Rewind::new(head, stream)));
    }
    let path = target.split(['?', '#']).next().unwrap_or("/");
    let head_only = method == "HEAD";
    let header = |name: &str| {
        text.split("\r\n")
            .skip(1)
            .filter_map(|l| l.split_once(':'))
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.trim().to_string())
    };
    let bearer = || {
        authorization.as_deref().and_then(|value| {
            let (scheme, token) = value.split_once(' ')?;
            scheme
                .eq_ignore_ascii_case("bearer")
                .then(|| token.trim().to_string())
        })
    };
    // A reverse proxy on this machine (`tailscale serve`, a tunnel) connects from loopback for visitors from anywhere. A request that
    // says it was forwarded is never "the owner at the hub's own machine": its address is unknown, so the screen password applies.
    let peer = if peer.is_some_and(|p| Client::new(Some(p), "").is_local()) && forwarded(&text) {
        None
    } else {
        peer
    };
    let client = Client::new(peer, &header("user-agent").unwrap_or_default());
    if path == "/api/session" && (method == "GET" || method == "DELETE") {
        let (status, out) = match (sessions, method) {
            (None, _) => (404, String::new()),
            (Some(api), "GET") => (200, api.info(&client).to_string()),
            (Some(api), _) => match bearer() {
                Some(t) => (api.end(&t), "{}".to_string()),
                None => (401, String::new()),
            },
        };
        respond(
            &mut stream,
            status,
            "application/json",
            out.as_bytes(),
            false,
        )
        .await?;
        return Ok(First::Answered);
    }
    let passkey_op = path.strip_prefix("/api/passkey/");
    let account_op = match path {
        "/api/password" => Some("password"),
        _ => None,
    };
    if method == "POST"
        && (path == "/api/session"
            || path == "/api/preview/enter"
            || passkey_op.is_some()
            || account_op.is_some())
    {
        let length = match request_content_length(&text) {
            Ok(Some(length)) => length,
            Ok(None) => usize::MAX,
            Err(()) => {
                respond(&mut stream, 400, "text/plain", b"", false).await?;
                return Ok(First::Answered);
            }
        };
        if length > MAX_BODY {
            respond(&mut stream, 413, "text/plain", b"", false).await?;
            return Ok(First::Answered);
        }
        let mut body = head[end..].to_vec();
        while body.len() < length {
            let Ok(n) = tokio::time::timeout_at(until, stream.read(&mut chunk)).await else {
                return Ok(First::Answered);
            };
            let n = n?;
            if n == 0 {
                return Ok(First::Answered);
            }
            body.extend_from_slice(&chunk[..n]);
        }
        body.truncate(length);
        let json = serde_json::from_slice::<serde_json::Value>(&body).ok();
        let (status, out) = if path == "/api/preview/enter" {
            match (sessions, json) {
                (Some(api), Some(j)) => {
                    api.preview_enter(bearer().as_deref(), &header("host").unwrap_or_default(), &j)
                }
                (None, _) => (404, String::new()),
                _ => (400, String::new()),
            }
        } else if let Some(op) = passkey_op.or(account_op) {
            match (sessions, json) {
                (Some(api), Some(j)) if account_op.is_some() => {
                    api.account(op, bearer().as_deref(), &client, &j).await
                }
                (Some(api), Some(j)) => {
                    let host = header("host").unwrap_or_default();
                    api.passkey(op, bearer().as_deref(), &host, &client, &j)
                        .await
                }
                (None, _) => (404, String::new()),
                _ => (400, String::new()),
            }
        } else {
            let field = |k: &str| {
                json.as_ref()
                    .and_then(|v| v.get(k))
                    .and_then(|c| c.as_str())
            };
            match (sessions, field("code")) {
                (Some(api), Some(code)) => {
                    let req = SignInRequest {
                        code,
                        password: field("password"),
                        client: &client,
                    };
                    match api.sign_in(&req).await {
                        SignInReply::Token(t) => {
                            (200, serde_json::json!({ "token": t }).to_string())
                        }
                        SignInReply::Refused => (401, String::new()),
                        SignInReply::TooMany => (429, String::new()),
                        SignInReply::Busy => (503, String::new()),
                        SignInReply::Failed => (500, String::new()),
                    }
                }
                (None, _) => (404, String::new()),
                _ => (400, String::new()),
            }
        };
        // Every authentication refusal costs the same short wait, whatever was wrong.
        if status == 401 || status == 429 {
            tokio::time::sleep(FAILURE_DELAY).await;
        }
        respond(
            &mut stream,
            status,
            "application/json",
            out.as_bytes(),
            false,
        )
        .await?;
        return Ok(First::Answered);
    }
    // Only while the hub was started with `--metrics`; otherwise the path is answered as any unknown one (#562 review: an answer of
    // its own would say that it exists).
    if (method == "GET" || head_only)
        && path == "/api/metrics"
        && sessions.is_some_and(|api| api.metrics_on())
    {
        let (status, out) = sessions.map_or((404, String::new()), |api| {
            api.metrics(bearer().as_deref(), &client)
        });
        // As for a wrong sign-in: a refusal costs the same short wait, whatever was wrong.
        if status == 401 || status == 429 {
            tokio::time::sleep(FAILURE_DELAY).await;
        }
        respond(
            &mut stream,
            status,
            "text/plain; version=0.0.4",
            out.as_bytes(),
            head_only,
        )
        .await?;
        return Ok(First::Answered);
    }
    if method == "GET" && path == "/api/audit" {
        let bearer = bearer();
        let query = target
            .split_once('?')
            .map_or("", |(_, q)| q.split('#').next().unwrap_or(""));
        let (status, out) = sessions.map_or((404, String::new()), |api| {
            api.audit(bearer.as_deref(), query)
        });
        respond(
            &mut stream,
            status,
            "application/json",
            out.as_bytes(),
            false,
        )
        .await?;
        return Ok(First::Answered);
    }
    if method != "GET" && !head_only {
        respond(&mut stream, 405, "text/plain", b"", false).await?;
        return Ok(First::Answered);
    }
    match assets.and_then(|a| a.get(path)) {
        Some(a) => {
            // A plugin's frame page (ADR-0114 Amendment A) is the one file framed, by this hub's own window only, under its own
            // policy; every other file keeps the window's headers.
            let page = if path == PLUGIN_FRAME_PATH {
                Page::PluginFrame
            } else {
                Page::App
            };
            // ADR-0088 Amendment A: the window's page may frame previews, each on a port of this same host.
            let host = header("host").unwrap_or_default();
            match (page, preview_frames(&host)) {
                (Page::App, Some(src)) if a.content_type.starts_with("text/html") => {
                    let csp =
                        format!("{APP_CSP_HEAD} frame-src 'self' {src}; frame-ancestors 'none'");
                    respond_with(
                        &mut stream,
                        200,
                        a.content_type,
                        &a.body,
                        head_only,
                        "DENY",
                        &csp,
                    )
                    .await?
                }
                _ => respond_as(&mut stream, 200, a.content_type, &a.body, head_only, page).await?,
            }
        }
        None => respond(&mut stream, 404, "text/plain", b"not found", head_only).await?,
    }
    Ok(First::Answered)
}

/// The page a plugin's sandboxed frame shows (ADR-0114 decision 2 and Amendment A), from the web app's files.
pub const PLUGIN_FRAME_PATH: &str = "/plugin-frame.html";

/// The window's page policy. The only frame it shows is its own plugin frame page (`frame-src 'self'`): every other page of the hub
/// refuses to be framed (`X-Frame-Options: DENY`, `frame-ancestors 'none'`).
const APP_CSP: &str = "default-src 'self'; connect-src 'self' ws: wss:; style-src 'self' 'unsafe-inline'; img-src 'self' data:; \
                       frame-src 'self'; frame-ancestors 'none'";
/// [`APP_CSP`] up to its frame sources, for the window's page that frames previews.
const APP_CSP_HEAD: &str =
    "default-src 'self'; connect-src 'self' ws: wss:; style-src 'self' 'unsafe-inline'; img-src 'self' data:;";

/// The frame source for previews on the host the window was reached by (`https://<host>:*`: each preview is a port of the hub's own,
/// ADR-0088 Amendment A); `None` for a host name with anything a policy could read as more than a host.
fn preview_frames(host: &str) -> Option<String> {
    let name = match host.rfind(']') {
        Some(end) => &host[..=end],
        None => host.rsplit_once(':').map_or(host, |(n, _)| n),
    };
    let ok = !name.is_empty()
        && name.len() <= 253
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'[' | b']' | b':'));
    ok.then(|| format!("https://{name}:*"))
}

/// The plugin frame page's own policy: opaque even when opened on its own (`sandbox allow-scripts`), the window's loader and the
/// blobs it makes inside the frame, no connection to anywhere (WebSocket included; WebRTC too where the engine enforces `webrtc`,
/// and the loader takes it away everywhere), no frames, forms or base, and framed only by this hub's own pages. `apps/web/src/plugins/panel.test.tsx` holds `frameDoc.ts` `FRAME_POLICY` equal to it.
const PLUGIN_FRAME_CSP: &str = "sandbox allow-scripts; default-src 'none'; script-src 'self' blob:; \
                                style-src 'self' blob: 'unsafe-inline'; img-src blob: data:; font-src blob:; connect-src 'none'; \
                                frame-src 'none'; form-action 'none'; base-uri 'none'; webrtc 'block'; frame-ancestors 'self'";

/// Which headers an answer carries.
#[derive(Clone, Copy)]
enum Page {
    App,
    PluginFrame,
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

async fn respond<S: AsyncWrite + Unpin>(
    stream: &mut S,
    status: u16,
    content_type: &str,
    body: &[u8],
    head_only: bool,
) -> io::Result<()> {
    respond_as(stream, status, content_type, body, head_only, Page::App).await
}

async fn respond_with<S: AsyncWrite + Unpin>(
    stream: &mut S,
    status: u16,
    content_type: &str,
    body: &[u8],
    head_only: bool,
    frame_options: &str,
    csp: &str,
) -> io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status} OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         X-Content-Type-Options: nosniff\r\nX-Frame-Options: {frame_options}\r\nReferrer-Policy: no-referrer\r\n\
         Content-Security-Policy: {csp}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    if !head_only {
        stream.write_all(body).await?;
    }
    stream.shutdown().await
}

async fn respond_as<S: AsyncWrite + Unpin>(
    stream: &mut S,
    status: u16,
    content_type: &str,
    body: &[u8],
    head_only: bool,
    page: Page,
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        409 => "Conflict",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        404 => "Not Found",
        405 => "Method Not Allowed",
        421 => "Misdirected Request",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        _ => "Request Header Fields Too Large",
    };
    // The app talks only to its own origin; nothing is sniffed or loaded from elsewhere, and nothing is framed but the plugin frame
    // page, by this hub's own window (ADR-0114 Amendment A).
    let (frame_options, csp) = match page {
        Page::App => ("DENY", APP_CSP),
        Page::PluginFrame => ("SAMEORIGIN", PLUGIN_FRAME_CSP),
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         X-Content-Type-Options: nosniff\r\nX-Frame-Options: {frame_options}\r\nReferrer-Policy: no-referrer\r\n\
         Content-Security-Policy: {csp}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    if !head_only {
        stream.write_all(body).await?;
    }
    stream.shutdown().await
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    struct EmptySessions;

    impl SessionApi for EmptySessions {
        fn sign_in<'a>(&'a self, _req: &'a SignInRequest<'a>) -> SessionReply<'a, SignInReply> {
            Box::pin(async { SignInReply::Failed })
        }
    }

    struct EndSessions;

    impl SessionApi for EndSessions {
        fn sign_in<'a>(&'a self, _req: &'a SignInRequest<'a>) -> SessionReply<'a, SignInReply> {
            Box::pin(async { SignInReply::Failed })
        }

        fn end(&self, bearer: &str) -> u16 {
            if bearer == "good" {
                200
            } else {
                401
            }
        }
    }

    struct SignInSessions;

    impl SessionApi for SignInSessions {
        fn sign_in<'a>(&'a self, _req: &'a SignInRequest<'a>) -> SessionReply<'a, SignInReply> {
            Box::pin(async { SignInReply::Token("token".to_string()) })
        }
    }

    #[test]
    fn paths_cannot_leave_the_root() {
        assert_eq!(safe_relative("/a/b.js"), Some(PathBuf::from("a/b.js")));
        assert_eq!(safe_relative("/"), Some(PathBuf::new()));
        for bad in [
            "/../etc/passwd",
            "/a/../../x",
            "/a\\b",
            "/C:/x",
            "/%2e%2e/x",
        ] {
            let r = safe_relative(bad);
            assert!(
                r.as_ref()
                    .is_none_or(|p| p.components().all(|c| matches!(c, Component::Normal(_)))),
                "{bad}"
            );
        }
        assert_eq!(safe_relative("/../etc/passwd"), None);
        assert_eq!(safe_relative("/a/../../x"), None);
    }

    #[test]
    fn a_directory_serves_files_and_falls_back_to_the_shell_for_routes() {
        let dir = std::env::temp_dir().join(format!("agora-web-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("index.html"), "shell").unwrap();
        std::fs::write(dir.join("assets/app.js"), "js").unwrap();
        let a = DirAssets::new(&dir);
        assert_eq!(a.get("/").unwrap().body, b"shell");
        assert_eq!(
            a.get("/assets/app.js").unwrap().content_type,
            "text/javascript; charset=utf-8"
        );
        assert_eq!(a.get("/agents/42").unwrap().body, b"shell");
        assert!(a.get("/assets/missing.js").is_none());
        assert!(a.get("/../x").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn embedded_assets_follow_the_same_rules() {
        static FILES: &[(&str, &[u8])] = &[("index.html", b"shell"), ("assets/app.js", b"js")];
        let a = EmbeddedAssets(FILES);
        assert_eq!(a.get("/").unwrap().body, b"shell");
        assert_eq!(a.get("/assets/app.js").unwrap().body, b"js");
        assert_eq!(a.get("/agents/7").unwrap().body, b"shell");
        assert!(a.get("/assets/nope.js").is_none());
        assert!(a.get("/../index.html").is_none());
    }

    #[test]
    fn the_manifest_is_answered_by_the_hub_and_everything_else_by_the_folder() {
        static FILES: &[(&str, &[u8])] = &[("index.html", b"shell")];
        let a = Installable {
            inner: Arc::new(EmbeddedAssets(FILES)),
            manifest: manifest("Name"),
        };
        let m = a.get("/manifest.webmanifest").unwrap();
        assert_eq!(m.content_type, "application/manifest+json");
        let v: serde_json::Value = serde_json::from_slice(&m.body).unwrap();
        assert_eq!(
            (v["name"].as_str(), v["display"].as_str()),
            (Some("Name"), Some("standalone"))
        );
        // ADR-0067 rung 2: launchers that cannot use the SVG get PNGs, one of them maskable.
        let icons = v["icons"].as_array().unwrap();
        for (src, sizes, purpose) in [
            ("/icon-192.png", "192x192", "any"),
            ("/icon-512.png", "512x512", "any"),
            ("/icon-maskable-512.png", "512x512", "maskable"),
        ] {
            assert!(
                icons
                    .iter()
                    .any(|i| i["src"] == src && i["sizes"] == sizes && i["purpose"] == purpose),
                "{src}"
            );
        }
        assert_eq!(a.get("/").unwrap().body, b"shell");
    }

    // The window's headers, exactly: framed by no one, framing only the hub's own plugin frame page (ADR-0114 Amendment A).
    const APP_HEADERS: &str = "X-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nReferrer-Policy: no-referrer\r\n\
        Content-Security-Policy: default-src 'self'; connect-src 'self' ws: wss:; style-src 'self' 'unsafe-inline'; \
        img-src 'self' data:; frame-src 'self'; frame-ancestors 'none'\r\nCache-Control: no-cache\r\n";
    // The window's HTML page reached as `Host: h`: the same, and it may frame previews on that host's other ports (ADR-0088 Amendment A).
    const APP_PAGE_HEADERS: &str = "X-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nReferrer-Policy: no-referrer\r\n\
        Content-Security-Policy: default-src 'self'; connect-src 'self' ws: wss:; style-src 'self' 'unsafe-inline'; \
        img-src 'self' data:; frame-src 'self' https://h:*; frame-ancestors 'none'\r\nCache-Control: no-cache\r\n";
    // The plugin frame page's headers, exactly: opaque even on its own, no network, framed only by the hub's own pages.
    const FRAME_HEADERS: &str = "X-Content-Type-Options: nosniff\r\nX-Frame-Options: SAMEORIGIN\r\nReferrer-Policy: no-referrer\r\n\
        Content-Security-Policy: sandbox allow-scripts; default-src 'none'; script-src 'self' blob:; \
        style-src 'self' blob: 'unsafe-inline'; img-src blob: data:; font-src blob:; connect-src 'none'; frame-src 'none'; \
        form-action 'none'; base-uri 'none'; webrtc 'block'; frame-ancestors 'self'\r\nCache-Control: no-cache\r\n";

    async fn get(path: &str, assets: &dyn Assets) -> String {
        let (mut c, s) = tokio::io::duplex(1 << 16);
        let req = format!("GET {path} HTTP/1.1\r\nHost: h\r\n\r\n");
        c.write_all(req.as_bytes()).await.unwrap();
        route(s, None, Some(assets), None).await.unwrap();
        let mut out = String::new();
        c.read_to_string(&mut out).await.unwrap();
        out
    }

    #[tokio::test]
    async fn only_the_plugin_frame_page_has_headers_of_its_own() {
        static FILES: &[(&str, &[u8])] = &[
            ("index.html", b"shell"),
            ("plugin-frame.html", b"frame"),
            ("plugin-frame.js", b"loader"),
            ("assets/app.js", b"js"),
        ];
        let a = EmbeddedAssets(FILES);
        let frame = get("/plugin-frame.html", &a).await;
        assert!(frame.starts_with("HTTP/1.1 200 OK"), "{frame}");
        assert!(frame.contains(FRAME_HEADERS), "{frame}");
        assert!(frame.ends_with("frame"));
        // A query does not change the path.
        assert!(get("/plugin-frame.html?x=1", &a)
            .await
            .contains(FRAME_HEADERS));
        for path in [
            "/",
            "/index.html",
            "/plugin-frame.js",
            "/assets/app.js",
            "/plugin-frame.html/",
            "//plugin-frame.html",
            "/a/../plugin-frame.html",
            "/route",
            "/missing.js",
        ] {
            let out = get(path, &a).await;
            assert!(
                out.contains(APP_HEADERS) || out.contains(APP_PAGE_HEADERS),
                "{path}: {out}"
            );
            assert!(!out.contains("SAMEORIGIN"), "{path}");
        }
        // Without a frame page among the files, the path is a 404 with the window's headers.
        static NONE: &[(&str, &[u8])] = &[("index.html", b"shell")];
        let out = get("/plugin-frame.html", &EmbeddedAssets(NONE)).await;
        assert!(out.starts_with("HTTP/1.1 404"), "{out}");
        assert!(out.contains(APP_HEADERS), "{out}");
    }

    #[tokio::test]
    async fn a_plain_get_is_answered_and_an_upgrade_is_handed_on() {
        struct One;
        impl Assets for One {
            fn get(&self, p: &str) -> Option<Asset> {
                (p == "/").then(|| Asset {
                    content_type: "text/html; charset=utf-8",
                    body: b"hello".to_vec(),
                })
            }
        }
        let (mut c, s) = tokio::io::duplex(4096);
        let task = tokio::spawn(async move {
            route(s, None, Some(&One), None)
                .await
                .map(|f| matches!(f, First::Answered))
        });
        c.write_all(b"GET /?x=1 HTTP/1.1\r\nHost: h\r\n\r\n")
            .await
            .unwrap();
        let mut out = String::new();
        c.read_to_string(&mut out).await.unwrap();
        assert!(out.starts_with("HTTP/1.1 200 OK"), "{out}");
        assert!(out.contains(APP_PAGE_HEADERS), "{out}");
        assert!(out.ends_with("hello"));
        assert!(task.await.unwrap().unwrap());

        let (mut c, s) = tokio::io::duplex(4096);
        let task = tokio::spawn(async move {
            match route(s, None, None, None).await.unwrap() {
                First::WebSocket(mut r) => {
                    let mut buf = vec![0u8; 22];
                    r.read_exact(&mut buf).await.unwrap();
                    Some(buf)
                }
                First::Answered => None,
            }
        });
        c.write_all(b"GET / HTTP/1.1\r\nUpgrade: WebSocket\r\n\r\n")
            .await
            .unwrap();
        assert_eq!(task.await.unwrap().unwrap(), b"GET / HTTP/1.1\r\nUpgrad");
    }

    async fn ask(
        api: &dyn SessionApi,
        peer: &str,
        request: String,
    ) -> (String, std::time::Duration) {
        let (mut c, s) = tokio::io::duplex(64 * 1024);
        c.write_all(request.as_bytes()).await.unwrap();
        let started = std::time::Instant::now();
        route(s, Some(peer.parse().unwrap()), None, Some(api))
            .await
            .unwrap();
        let mut out = String::new();
        c.read_to_string(&mut out).await.unwrap();
        (out, started.elapsed())
    }

    fn post(path: &str, body: &serde_json::Value) -> String {
        let body = body.to_string();
        format!(
            "POST {path} HTTP/1.1\r\nHost: h\r\nUser-Agent: curl/8.5\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
    }

    #[tokio::test]
    async fn f427_conflicting_content_lengths_are_refused_before_sign_in() {
        let hub = Arc::new(
            crate::hub::Hub::open(crate::store::SqliteStore::open_in_memory().unwrap()).unwrap(),
        );
        let api = crate::signin::HubSessions::new(Arc::clone(&hub));
        let code = crate::signin::SignIn::new(hub.store(), Arc::new(crate::enroll::Lockout::new()))
            .create_code(crate::server::now_ms())
            .unwrap();
        let body = serde_json::json!({ "code": code }).to_string();
        let control = format!(
            "POST /api/session HTTP/1.1\r\nHost: h\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let (control, _) = ask(&api, "127.0.0.1", control).await;
        assert!(control.starts_with("HTTP/1.1 200"), "{control}");
        println!("CONTROL OK");

        let code = crate::signin::SignIn::new(hub.store(), Arc::new(crate::enroll::Lockout::new()))
            .create_code(crate::server::now_ms())
            .unwrap();
        let body = serde_json::json!({ "code": code }).to_string();
        let request = format!(
            "POST /api/session HTTP/1.1\r\nHost: h\r\nContent-Length: {}\r\nContent-Length: {}\r\n\r\n{body}",
            body.len(),
            body.len() + 1
        );
        let (actual, _) = ask(&api, "127.0.0.1", request).await;
        println!("EXPECTED: HTTP/1.1 400");
        println!("ACTUAL: {}", actual.lines().next().unwrap_or(""));
        if actual.starts_with("HTTP/1.1 400") {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert!(actual.starts_with("HTTP/1.1 400"), "{actual}");
    }

    #[tokio::test]
    async fn verifies_f427_content_length_boundaries() {
        let hub = Arc::new(
            crate::hub::Hub::open(crate::store::SqliteStore::open_in_memory().unwrap()).unwrap(),
        );
        let api = crate::signin::HubSessions::new(hub);
        let body = r#"{"code":"bad"}"#;
        for fields in [
            format!("Content-Length: {}", body.len()),
            format!("Content-Length: {0}\r\nContent-Length: {0}", body.len()),
            format!("Content-Length: {0}, {0}", body.len()),
        ] {
            let request =
                format!("POST /api/session HTTP/1.1\r\nHost: h\r\n{fields}\r\n\r\n{body}");
            let (out, _) = ask(&api, "127.0.0.1", request).await;
            assert!(!out.starts_with("HTTP/1.1 400"), "{fields}: {out}");
        }
        for fields in [
            format!("Content-Length: +{}", body.len()),
            format!(
                "Content-Length: {}\r\nContent-Length: {}",
                body.len() + 1,
                body.len()
            ),
            "Content-Length: 1x".to_string(),
        ] {
            let request =
                format!("POST /api/session HTTP/1.1\r\nHost: h\r\n{fields}\r\n\r\n{body}");
            let (out, _) = ask(&api, "127.0.0.1", request).await;
            assert!(out.starts_with("HTTP/1.1 400"), "{fields}: {out}");
        }
        println!("FIX VERIFIED");
    }

    #[tokio::test]
    async fn f428_a_request_head_over_the_limit_is_refused() {
        let at_limit_prefix = "GET / HTTP/1.1\r\nX-Pad: ";
        let suffix = "\r\n\r\n";
        let at_limit = format!(
            "{at_limit_prefix}{}{suffix}",
            "a".repeat(MAX_HEAD - at_limit_prefix.len() - suffix.len())
        );
        let (control, _) = ask(&EmptySessions, "127.0.0.1", at_limit).await;
        assert!(!control.starts_with("HTTP/1.1 431"), "{control}");
        println!("CONTROL OK");

        let over = format!(
            "{at_limit_prefix}{}{suffix}",
            "a".repeat(MAX_HEAD + 1 - at_limit_prefix.len() - suffix.len())
        );
        let (actual, _) = ask(&EmptySessions, "127.0.0.1", over).await;
        println!("EXPECTED: HTTP/1.1 431");
        println!("ACTUAL: {}", actual.lines().next().unwrap_or(""));
        if actual.starts_with("HTTP/1.1 431") {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert!(actual.starts_with("HTTP/1.1 431"), "{actual}");
    }

    #[tokio::test]
    async fn verifies_f428_request_head_size_boundaries() {
        let prefix = "GET / HTTP/1.1\r\nX-Pad: ";
        let suffix = "\r\n\r\n";
        for size in [MAX_HEAD - 1, MAX_HEAD] {
            let request = format!(
                "{prefix}{}{suffix}",
                "a".repeat(size - prefix.len() - suffix.len())
            );
            let (out, _) = ask(&EmptySessions, "127.0.0.1", request).await;
            assert!(!out.starts_with("HTTP/1.1 431"), "{size}");
        }
        for size in [MAX_HEAD + 1, MAX_HEAD + 512, MAX_HEAD + 1024] {
            let request = format!(
                "{prefix}{}{suffix}",
                "a".repeat(size - prefix.len() - suffix.len())
            );
            let (out, _) = ask(&EmptySessions, "127.0.0.1", request).await;
            assert!(out.starts_with("HTTP/1.1 431"), "{size}: {out}");
        }
        println!("FIX VERIFIED");
    }

    #[tokio::test]
    async fn the_screen_password_is_asked_of_another_machine_over_http_and_not_of_this_one() {
        let hub = Arc::new(
            crate::hub::Hub::open(crate::store::SqliteStore::open_in_memory().unwrap()).unwrap(),
        );
        let api = crate::signin::HubSessions::new(Arc::clone(&hub)).with_password_rounds(1_000);
        crate::password::ScreenPassword::with_iterations(hub.store(), 1_000)
            .set("long enough")
            .unwrap();
        let code = || {
            crate::signin::SignIn::new(hub.store(), Arc::new(crate::enroll::Lockout::new()))
                .create_code(crate::server::now_ms())
                .unwrap()
        };
        let info = "GET /api/session HTTP/1.1\r\nHost: h\r\n\r\n".to_string();
        let (out, _) = ask(&api, "192.0.2.8", info.clone()).await;
        assert!(
            out.ends_with(r#"{"passwordRequired":true,"passwordSet":true}"#),
            "{out}"
        );
        let (out, _) = ask(&api, "127.0.0.1", info).await;
        assert!(out.contains(r#""passwordRequired":false"#), "{out}");

        let c = code();
        let (out, took) = ask(
            &api,
            "192.0.2.8",
            post("/api/session", &serde_json::json!({ "code": c })),
        )
        .await;
        assert!(out.starts_with("HTTP/1.1 401"), "{out}");
        assert!(took >= FAILURE_DELAY, "a refusal waits");
        let (out, _) = ask(
            &api,
            "192.0.2.8",
            post(
                "/api/session",
                &serde_json::json!({ "code": c, "password": "long enough" }),
            ),
        )
        .await;
        assert!(out.starts_with("HTTP/1.1 200"), "{out}");
        let token =
            serde_json::from_str::<serde_json::Value>(out.split("\r\n\r\n").nth(1).unwrap())
                .unwrap()["token"]
                .as_str()
                .unwrap()
                .to_string();
        let (out, _) = ask(
            &api,
            "127.0.0.1",
            post("/api/session", &serde_json::json!({ "code": code() })),
        )
        .await;
        assert!(
            out.starts_with("HTTP/1.1 200"),
            "the hub's own machine: {out}"
        );

        let end = format!("DELETE /api/session HTTP/1.1\r\nAuthorization: Bearer {token}\r\n\r\n");
        let (out, _) = ask(&api, "192.0.2.8", end.clone()).await;
        assert!(out.starts_with("HTTP/1.1 200"), "{out}");
        let (out, _) = ask(&api, "192.0.2.8", end).await;
        assert!(out.starts_with("HTTP/1.1 401"), "{out}");
    }

    /// A reverse proxy on the hub's machine (`tailscale serve`, a tunnel) connects from loopback: a request that says it was forwarded
    /// is not the owner at the hub's own machine, so the screen password applies to it (ADR-0067).
    #[tokio::test]
    async fn a_forwarded_request_from_loopback_is_asked_for_the_password() {
        let hub = Arc::new(
            crate::hub::Hub::open(crate::store::SqliteStore::open_in_memory().unwrap()).unwrap(),
        );
        let api = crate::signin::HubSessions::new(Arc::clone(&hub)).with_password_rounds(1_000);
        crate::password::ScreenPassword::with_iterations(hub.store(), 1_000)
            .set("long enough")
            .unwrap();
        // Every header name on its own (one iteration each, so a name dropped from the list fails here), in mixed case, with an
        // empty value, and repeated; and one name per prefix.
        for name in [
            "X-Forwarded-For",
            "X-Forwarded-Host",
            "X-Forwarded-Proto",
            "X-Forwarded-Port",
            "X-Forwarded-Server",
            "X-Forwarded-Scheme",
            "X-Original-Forwarded-For",
            "X-Real-IP",
            "Forwarded",
            "Via",
            "X-Client-IP",
            "True-Client-IP",
            "Tailscale-User-Login",
            "Tailscale-User-Name",
            "CF-Connecting-IP",
            "CF-Connecting-IPv6",
            "CF-Ray",
            "Fastly-Client-IP",
            "X-Envoy-External-Address",
            "x-forwarded-anything-new",
            "cf-anything-new",
            "tailscale-anything-new",
            "fastly-anything-new",
            "x-envoy-anything-new",
            "x-original-forwarded-anything",
        ] {
            for header in [
                format!("{name}: 203.0.113.9"),
                format!("{}: ", name.to_ascii_lowercase()),
                format!("{name}: a\r\n{name}: b"),
            ] {
                let info = format!("GET /api/session HTTP/1.1\r\nHost: h\r\n{header}\r\n\r\n");
                let (out, _) = ask(&api, "127.0.0.1", info).await;
                assert!(
                    out.contains(r#""passwordRequired":true"#),
                    "{header:?}: {out}"
                );
            }
        }
        // Names that only look alike are not proxy headers: a browser on this machine still is the owner.
        for header in [
            "Accept: */*",
            "X-Requested-With: x",
            "Cache-Control: no-cache",
            "Referer: http://localhost/",
        ] {
            let info = format!("GET /api/session HTTP/1.1\r\nHost: h\r\n{header}\r\n\r\n");
            let (out, _) = ask(&api, "127.0.0.1", info).await;
            assert!(
                out.contains(r#""passwordRequired":false"#),
                "{header}: {out}"
            );
        }
        let info = "GET /api/session HTTP/1.1\r\nHost: h\r\n\r\n".to_string();
        let (out, _) = ask(&api, "::1", info).await;
        assert!(out.contains(r#""passwordRequired":false"#), "{out}");
    }

    // ---- ADR-0067 rung 4: a trusted reverse proxy ----

    const PUBLIC: &str = "agora.example.com";

    /// A hub with a screen password and the proxy trusted as the owner would turn it on (`None`: never turned on).
    fn proxied(on: bool) -> (Arc<crate::hub::Hub>, crate::signin::HubSessions) {
        let hub = Arc::new(
            crate::hub::Hub::open(crate::store::SqliteStore::open_in_memory().unwrap()).unwrap(),
        );
        crate::password::ScreenPassword::with_iterations(hub.store(), 1_000)
            .set("long enough")
            .unwrap();
        let mut api = crate::signin::HubSessions::new(Arc::clone(&hub)).with_password_rounds(1_000);
        if on {
            let trust = crate::proxy::ProxyTrust::new(
                "CF-Connecting-IP",
                &[PUBLIC.to_string()],
                &["10.1.2.3".parse().unwrap()],
            )
            .unwrap();
            api = api.with_proxy(Arc::new(crate::proxy::ProxyHandle::fixed(Some(trust))));
        }
        (hub, api)
    }

    fn info(headers: &[&str]) -> String {
        format!(
            "GET /api/session HTTP/1.1\r\n{}\r\n\r\n",
            headers.join("\r\n")
        )
    }

    fn required(out: &str) -> bool {
        assert!(out.starts_with("HTTP/1.1 200"), "{out}");
        out.contains(r#""passwordRequired":true"#)
    }

    /// Sign-in with a wrong password; true when the answer is the lockout (429) and not an ordinary refusal (401).
    async fn locked(api: &dyn SessionApi, peer: &str, headers: &[&str]) -> bool {
        let body = serde_json::json!({ "code": "AAAA-AAAA", "password": "wrong one!" }).to_string();
        let request = format!(
            "POST /api/session HTTP/1.1\r\n{}\r\nContent-Length: {}\r\n\r\n{body}",
            headers.join("\r\n"),
            body.len()
        );
        let (out, _) = ask(api, peer, request).await;
        assert!(
            out.starts_with("HTTP/1.1 401") || out.starts_with("HTTP/1.1 429"),
            "{out}"
        );
        out.starts_with("HTTP/1.1 429")
    }

    #[tokio::test]
    async fn off_by_default_a_header_and_a_host_change_nothing() {
        let (_hub, api) = proxied(false);
        // The same request that is judged with the proxy on: from loopback, a foreign Host and an Origin, no proxy header. Nothing is
        // refused and the owner at the machine stays the owner.
        let (out, _) = ask(
            &api,
            "127.0.0.1",
            info(&["Host: evil.example.org", "Origin: https://evil.example.org"]),
        )
        .await;
        assert!(!required(&out), "{out}");
        // A forged header from a remote peer is no address.
        let (out, _) = ask(
            &api,
            "192.0.2.8",
            info(&[&format!("Host: {PUBLIC}"), "CF-Connecting-IP: 127.0.0.1"]),
        )
        .await;
        assert!(required(&out), "{out}");
    }

    #[tokio::test]
    async fn a_proxied_request_is_never_the_owner_at_the_machine() {
        let (_hub, api) = proxied(true);
        let host = format!("Host: {PUBLIC}");
        for header in [
            "CF-Connecting-IP: 203.0.113.9",
            "CF-Connecting-IP: 127.0.0.1",
            "CF-Connecting-IP: ::1",
            "CF-Connecting-IP: ::ffff:127.0.0.1",
            "CF-Connecting-IP: ",
            "CF-Connecting-IP: 203.0.113.9, 127.0.0.1",
            "CF-Connecting-IP: 1.1.1.1\r\ncf-connecting-ip: 127.0.0.1",
            "X-Forwarded-For: 203.0.113.9",
            "Accept: */*",
        ] {
            let (out, _) = ask(&api, "127.0.0.1", info(&[&host, header])).await;
            assert!(required(&out), "{header:?}: {out}");
        }
        // The configured proxy address (not loopback) is the same.
        let (out, _) = ask(
            &api,
            "10.1.2.3",
            info(&[&host, "CF-Connecting-IP: 203.0.113.9"]),
        )
        .await;
        assert!(required(&out), "{out}");
        // The owner's own browser by a loopback name is still the owner.
        for h in [
            "Host: 127.0.0.1:7443",
            "Host: localhost",
            "Host: [::1]:7443",
        ] {
            let (out, _) = ask(&api, "127.0.0.1", info(&[h])).await;
            assert!(!required(&out), "{h}: {out}");
        }
    }

    /// The guard that matters most: a header counts only from the proxy. A remote peer that sends the header (or one with a
    /// loopback address in it) is not believed, so it cannot pick its own rate-limit bucket or become the owner.
    #[tokio::test]
    async fn the_header_counts_only_from_the_proxy_and_not_from_anyone_else() {
        let (_hub, api) = proxied(true);
        // From a stranger, a header naming loopback does not make it local, and different values do not make different buckets: it
        // stays one address, so five wrong tries lock it whatever it claims.
        let (out, _) = ask(
            &api,
            "192.0.2.8",
            info(&[&format!("Host: {PUBLIC}"), "CF-Connecting-IP: 127.0.0.1"]),
        )
        .await;
        assert!(required(&out), "{out}");
        let mut tripped = false;
        for i in 0..8 {
            let claim = format!("CF-Connecting-IP: 198.51.100.{i}");
            tripped |= locked(&api, "192.0.2.8", &["Host: h", &claim]).await;
        }
        assert!(tripped, "a remote peer picked a new bucket with a header");
        // Its own address is what the lockout holds; a different remote peer is untouched.
        assert!(!locked(&api, "192.0.2.77", &["Host: h"]).await);
        // And a stranger's Host is not checked (only the proxy path is): the LAN keeps working by its own address.
        let (out, _) = ask(&api, "192.0.2.9", info(&["Host: 192.0.2.1:7443"])).await;
        assert!(required(&out), "{out}");
    }

    /// A proxy that forwards to loopback with the default `Host` of its target and forgot the address header: with no other sign of a
    /// proxy the request is the owner's own browser (the one case the hub cannot tell apart, stated in T40); with any other proxy
    /// header it is not.
    #[tokio::test]
    async fn a_forgotten_address_header_with_other_proxy_headers_is_never_the_owner() {
        let (_hub, api) = proxied(true);
        for header in [
            "X-Forwarded-For: 203.0.113.9",
            "X-Forwarded-Proto: https",
            "Via: 1.1 proxy",
            "Forwarded: for=203.0.113.9",
            "X-Real-IP: 203.0.113.9",
            "CF-Ray: abc",
        ] {
            let (out, _) = ask(&api, "127.0.0.1", info(&["Host: 127.0.0.1:7443", header])).await;
            assert!(required(&out), "{header}: {out}");
        }
        let (out, _) = ask(&api, "127.0.0.1", info(&["Host: 127.0.0.1:7443"])).await;
        assert!(!required(&out), "the owner's own browser: {out}");
    }

    /// A header name that the generic proxy-header list (T36) does not know: only the configured guard stands between a loopback
    /// "client address" and the owner's rights.
    #[tokio::test]
    async fn a_loopback_client_address_in_a_custom_header_is_never_the_owner() {
        let hub = Arc::new(
            crate::hub::Hub::open(crate::store::SqliteStore::open_in_memory().unwrap()).unwrap(),
        );
        crate::password::ScreenPassword::with_iterations(hub.store(), 1_000)
            .set("long enough")
            .unwrap();
        let trust =
            crate::proxy::ProxyTrust::new("X-Edge-Addr", &[PUBLIC.to_string()], &[]).unwrap();
        let api = crate::signin::HubSessions::new(Arc::clone(&hub))
            .with_proxy(Arc::new(crate::proxy::ProxyHandle::fixed(Some(trust))));
        for value in ["127.0.0.1", "::1", "::ffff:127.0.0.1", "203.0.113.9", ""] {
            let (out, _) = ask(
                &api,
                "127.0.0.1",
                info(&[&format!("Host: {PUBLIC}"), &format!("X-Edge-Addr: {value}")]),
            )
            .await;
            assert!(required(&out), "{value:?}: {out}");
        }
    }

    #[tokio::test]
    async fn each_real_client_address_has_its_own_rate_limit_bucket() {
        let (_hub, api) = proxied(true);
        let host = format!("Host: {PUBLIC}");
        let from = |ip: &str| format!("CF-Connecting-IP: {ip}");
        let mut tripped = false;
        for _ in 0..8 {
            tripped |= locked(&api, "127.0.0.1", &[&host, &from("203.0.113.1")]).await;
        }
        assert!(tripped, "five wrong passwords lock an address");
        assert!(locked(&api, "127.0.0.1", &[&host, &from("203.0.113.1")]).await);
        // Another visitor of the same proxy is not locked by it (the T36 limit is gone for a configured header).
        assert!(!locked(&api, "127.0.0.1", &[&host, &from("203.0.113.2")]).await);
        // Visitors whose address is unknown still share one bucket.
        let mut shared = false;
        for _ in 0..8 {
            shared |= locked(&api, "127.0.0.1", &[&host]).await;
        }
        assert!(shared);
        assert!(locked(&api, "127.0.0.1", &[&host, "CF-Connecting-IP: junk"]).await);
    }

    #[tokio::test]
    async fn a_foreign_host_or_origin_is_refused_with_a_code() {
        let (_hub, api) = proxied(true);
        let ip = "CF-Connecting-IP: 203.0.113.9";
        for peer in ["127.0.0.1", "10.1.2.3"] {
            let (out, _) = ask(&api, peer, info(&["Host: evil.example.org", ip])).await;
            assert!(out.starts_with("HTTP/1.1 421"), "{out}");
            assert!(
                out.ends_with(r#"{"error":"proxy.host_not_allowed"}"#),
                "{out}"
            );
            let (out, _) = ask(&api, peer, info(&["Accept: */*", ip])).await;
            assert!(out.starts_with("HTTP/1.1 421"), "no Host: {out}");
            let (out, _) = ask(
                &api,
                peer,
                info(&[
                    &format!("Host: {PUBLIC}"),
                    "Origin: https://evil.example.org",
                    ip,
                ]),
            )
            .await;
            assert!(out.starts_with("HTTP/1.1 403"), "{out}");
            assert!(
                out.ends_with(r#"{"error":"proxy.origin_not_allowed"}"#),
                "{out}"
            );
        }
        // A POST is judged before its body is read: nothing is signed in by a rebinding page.
        let (hub, api) = proxied(true);
        let c = crate::signin::SignIn::new(hub.store(), Arc::new(crate::enroll::Lockout::new()))
            .create_code(crate::server::now_ms())
            .unwrap();
        let body = serde_json::json!({ "code": c, "password": "long enough" }).to_string();
        let request = format!(
            "POST /api/session HTTP/1.1\r\nHost: evil.example.org\r\n{ip}\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let (out, _) = ask(&api, "127.0.0.1", request).await;
        assert!(out.starts_with("HTTP/1.1 421"), "{out}");
        // The right names pass, with an Origin of the public name too.
        let good = info(&[
            &format!("Host: {PUBLIC}"),
            &format!("Origin: https://{PUBLIC}"),
            ip,
        ]);
        let (out, _) = ask(&api, "127.0.0.1", good).await;
        assert!(required(&out), "{out}");
    }

    #[tokio::test]
    async fn a_websocket_upgrade_through_the_proxy_has_its_host_and_origin_checked() {
        let (_hub, api) = proxied(true);
        let upgrade = |headers: &[&str]| {
            format!(
                "GET /ws HTTP/1.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n{}\r\n\r\n",
                headers.join("\r\n")
            )
        };
        let run = |request: String| {
            let api = &api;
            async move {
                let (mut c, s) = tokio::io::duplex(4096);
                c.write_all(request.as_bytes()).await.unwrap();
                match route(s, Some("127.0.0.1".parse().unwrap()), None, Some(api))
                    .await
                    .unwrap()
                {
                    First::WebSocket(_) => "upgrade".to_string(),
                    First::Answered => {
                        let mut out = String::new();
                        c.read_to_string(&mut out).await.unwrap();
                        out
                    }
                }
            }
        };
        let ip = "CF-Connecting-IP: 203.0.113.9";
        assert_eq!(
            run(upgrade(&[
                &format!("Host: {PUBLIC}"),
                &format!("Origin: https://{PUBLIC}"),
                ip
            ]))
            .await,
            "upgrade"
        );
        let out = run(upgrade(&["Host: evil.example.org", ip])).await;
        assert!(out.starts_with("HTTP/1.1 421"), "{out}");
        let out = run(upgrade(&[
            &format!("Host: {PUBLIC}"),
            "Origin: https://evil.example.org",
            ip,
        ]))
        .await;
        assert!(out.starts_with("HTTP/1.1 403"), "{out}");
        // A page of another site opening a socket to the hub by its loopback name from the owner's own browser is refused too.
        let out = run(upgrade(&[
            "Host: 127.0.0.1:7443",
            "Origin: https://evil.example.org",
        ]))
        .await;
        assert!(out.starts_with("HTTP/1.1 403"), "{out}");
    }

    #[tokio::test]
    async fn proof_f435_transfer_coding_cannot_reframe_a_sign_in_body() {
        let body = r#"{"code":"ok"}"#;
        let request = |extra: &str| {
            format!(
                "POST /api/session HTTP/1.1\r\nHost: h\r\n{extra}Content-Length: {}\r\n\r\n{body}",
                body.len()
            )
        };
        let (control, _) = ask(&SignInSessions, "127.0.0.1", request("")).await;
        assert!(control.starts_with("HTTP/1.1 200"), "{control}");
        println!("CONTROL OK");

        let (actual, _) = ask(
            &SignInSessions,
            "127.0.0.1",
            request("Transfer-Encoding: chunked\r\n"),
        )
        .await;
        println!("EXPECTED: HTTP/1.1 400 before sign-in");
        println!("ACTUAL: {}", actual.lines().next().unwrap_or(""));
        if actual.starts_with("HTTP/1.1 400") {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert!(actual.starts_with("HTTP/1.1 400"), "{actual}");
    }

    #[tokio::test]
    async fn proof_f436_invalid_http_versions_cannot_reach_sign_in() {
        let body = r#"{"code":"ok"}"#;
        let request = |version: &str| {
            format!(
                "POST /api/session {version}\r\nHost: h\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
        };
        let (control, _) = ask(&SignInSessions, "127.0.0.1", request("HTTP/1.1")).await;
        assert!(control.starts_with("HTTP/1.1 200"), "{control}");
        println!("CONTROL OK");

        let (actual, _) = ask(&SignInSessions, "127.0.0.1", request("HTTP/9.9")).await;
        println!("EXPECTED: HTTP/1.1 400 before sign-in");
        println!("ACTUAL: {}", actual.lines().next().unwrap_or(""));
        if actual.starts_with("HTTP/1.1 400") {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert!(actual.starts_with("HTTP/1.1 400"), "{actual}");
    }

    #[tokio::test]
    async fn proof_f437_whitespace_cannot_normalize_an_authorization_field_name() {
        let control =
            "DELETE /api/session HTTP/1.1\r\nHost: h\r\nAuthorization: Bearer good\r\n\r\n";
        let (control, _) = ask(&EndSessions, "127.0.0.1", control.to_string()).await;
        assert!(control.starts_with("HTTP/1.1 200"), "{control}");
        println!("CONTROL OK");

        let malformed =
            "DELETE /api/session HTTP/1.1\r\nHost: h\r\nAuthorization : Bearer good\r\n\r\n";
        let (actual, _) = ask(&EndSessions, "127.0.0.1", malformed.to_string()).await;
        println!("EXPECTED: HTTP/1.1 400 before sign-out");
        println!("ACTUAL: {}", actual.lines().next().unwrap_or(""));
        if actual.starts_with("HTTP/1.1 400") {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert!(actual.starts_with("HTTP/1.1 400"), "{actual}");
    }

    #[tokio::test]
    async fn proof_f438_conflicting_authorization_fields_are_not_first_wins() {
        let control =
            "DELETE /api/session HTTP/1.1\r\nHost: h\r\nAuthorization: Bearer good\r\n\r\n";
        let (control, _) = ask(&EndSessions, "127.0.0.1", control.to_string()).await;
        assert!(control.starts_with("HTTP/1.1 200"), "{control}");
        println!("CONTROL OK");

        let repeated = "DELETE /api/session HTTP/1.1\r\nHost: h\r\nAuthorization: Bearer good\r\nAuthorization: Bearer bad\r\n\r\n";
        let (actual, _) = ask(&EndSessions, "127.0.0.1", repeated.to_string()).await;
        println!("EXPECTED: HTTP/1.1 400 before sign-out");
        println!("ACTUAL: {}", actual.lines().next().unwrap_or(""));
        if actual.starts_with("HTTP/1.1 400") {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert!(actual.starts_with("HTTP/1.1 400"), "{actual}");
    }

    #[tokio::test]
    async fn proof_f439_bearer_auth_scheme_is_ascii_case_insensitive() {
        let control =
            "DELETE /api/session HTTP/1.1\r\nHost: h\r\nAuthorization: Bearer good\r\n\r\n";
        let (control, _) = ask(&EndSessions, "127.0.0.1", control.to_string()).await;
        assert!(control.starts_with("HTTP/1.1 200"), "{control}");
        println!("CONTROL OK");

        let lower = "DELETE /api/session HTTP/1.1\r\nHost: h\r\nAuthorization: bearer good\r\n\r\n";
        let (actual, _) = ask(&EndSessions, "127.0.0.1", lower.to_string()).await;
        println!("EXPECTED: HTTP/1.1 200 with a case-insensitive auth scheme");
        println!("ACTUAL: {}", actual.lines().next().unwrap_or(""));
        if actual.starts_with("HTTP/1.1 200") {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert!(actual.starts_with("HTTP/1.1 200"), "{actual}");
    }

    #[tokio::test]
    async fn verify_f435_transfer_codings_are_refused_before_authentication() {
        let body = r#"{"code":"ok"}"#;
        for field in [
            "Transfer-Encoding: gzip",
            "transfer-encoding: chunked",
            "Transfer-Encoding: gzip, chunked",
        ] {
            let request = format!(
                "POST /api/session HTTP/1.1\r\nHost: h\r\n{field}\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            let (out, _) = ask(&SignInSessions, "127.0.0.1", request).await;
            assert!(out.starts_with("HTTP/1.1 400"), "{field}: {out}");
        }
        let (out, _) = ask(
            &SignInSessions,
            "127.0.0.1",
            post("/api/session", &serde_json::json!({ "code": "ok" })),
        )
        .await;
        assert!(out.starts_with("HTTP/1.1 200"), "{out}");
        println!("FIX VERIFIED: F435 unsupported transfer coding is refused before auth");
    }

    #[tokio::test]
    async fn verify_f436_request_lines_across_version_spacing_and_arity() {
        let body = r#"{"code":"ok"}"#;
        for line in [
            "POST /api/session HTTP/0.9",
            "POST  /api/session HTTP/1.1",
            "POST /api/session HTTP/1.1 extra",
        ] {
            let request = format!(
                "{line}\r\nHost: h\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            let (out, _) = ask(&SignInSessions, "127.0.0.1", request).await;
            assert!(out.starts_with("HTTP/1.1 400"), "{line:?}: {out}");
        }
        for version in ["HTTP/1.0", "HTTP/1.1"] {
            let request = format!(
                "POST /api/session {version}\r\nHost: h\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            let (out, _) = ask(&SignInSessions, "127.0.0.1", request).await;
            assert!(out.starts_with("HTTP/1.1 200"), "{version}: {out}");
        }
        println!("FIX VERIFIED: F436 only exact supported HTTP/1 request lines reach auth");
    }

    #[tokio::test]
    async fn verify_f437_field_names_across_leading_internal_and_precolon_whitespace() {
        for field in [
            " Authorization: Bearer good",
            "Authori zation: Bearer good",
            "Authorization\t: Bearer good",
        ] {
            let request = format!("DELETE /api/session HTTP/1.1\r\nHost: h\r\n{field}\r\n\r\n");
            let (out, _) = ask(&EndSessions, "127.0.0.1", request).await;
            assert!(out.starts_with("HTTP/1.1 400"), "{field:?}: {out}");
        }
        let valid = "DELETE /api/session HTTP/1.1\r\nHost: h\r\nauthorization: Bearer good\r\n\r\n";
        let (out, _) = ask(&EndSessions, "127.0.0.1", valid.to_string()).await;
        assert!(out.starts_with("HTTP/1.1 200"), "{out}");
        println!("FIX VERIFIED: F437 invalid field names are never normalized into auth");
    }

    #[tokio::test]
    async fn verify_f438_authorization_is_single_across_order_and_equal_values() {
        for fields in [
            "Authorization: Bearer good\r\nAuthorization: Bearer bad",
            "Authorization: Bearer bad\r\nAuthorization: Bearer good",
            "Authorization: Bearer good\r\nAuthorization: Bearer good",
        ] {
            let request = format!("DELETE /api/session HTTP/1.1\r\nHost: h\r\n{fields}\r\n\r\n");
            let (out, _) = ask(&EndSessions, "127.0.0.1", request).await;
            assert!(out.starts_with("HTTP/1.1 400"), "{fields:?}: {out}");
        }
        let single =
            "DELETE /api/session HTTP/1.1\r\nHost: h\r\nAuthorization: Bearer good\r\n\r\n";
        let (out, _) = ask(&EndSessions, "127.0.0.1", single.to_string()).await;
        assert!(out.starts_with("HTTP/1.1 200"), "{out}");
        println!("FIX VERIFIED: F438 Authorization is never resolved first-wins");
    }

    #[tokio::test]
    async fn verify_f439_bearer_scheme_case_boundaries_and_other_scheme() {
        for scheme in ["bearer", "BEARER", "BeArEr"] {
            let request = format!(
                "DELETE /api/session HTTP/1.1\r\nHost: h\r\nAuthorization: {scheme} good\r\n\r\n"
            );
            let (out, _) = ask(&EndSessions, "127.0.0.1", request).await;
            assert!(out.starts_with("HTTP/1.1 200"), "{scheme}: {out}");
        }
        let other = "DELETE /api/session HTTP/1.1\r\nHost: h\r\nAuthorization: Basic good\r\n\r\n";
        let (out, _) = ask(&EndSessions, "127.0.0.1", other.to_string()).await;
        assert!(out.starts_with("HTTP/1.1 401"), "{out}");
        println!("FIX VERIFIED: F439 Bearer is case-insensitive without accepting other schemes");
    }

    #[test]
    fn the_window_may_frame_previews_on_its_own_host_only() {
        assert_eq!(
            preview_frames("hub.example:7443").as_deref(),
            Some("https://hub.example:*")
        );
        assert_eq!(
            preview_frames("hub.example").as_deref(),
            Some("https://hub.example:*")
        );
        assert_eq!(
            preview_frames("[::1]:7443").as_deref(),
            Some("https://[::1]:*")
        );
        assert_eq!(
            preview_frames("10.0.0.2:7443").as_deref(),
            Some("https://10.0.0.2:*")
        );
        assert_eq!(preview_frames("evil; script-src *"), None);
        assert_eq!(preview_frames("a b:1"), None);
        assert_eq!(preview_frames(""), None);
    }
}

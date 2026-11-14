//! Serves the web app over plain HTTP on the same port as the WebSocket service.
//!
//! The first request head of every connection is read before anything else. A WebSocket upgrade is replayed
//! to the WebSocket handshake unchanged (`Rewind`); any other `GET` is answered from `Assets`.

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

/// What the sign-in routes need from the hub: `POST /api/session` trades a one-time code (and, from another machine, the screen
/// password) for a session token; `GET /api/session` says whether the password is needed; `DELETE /api/session` signs out.
pub trait SessionApi: Send + Sync {
    fn sign_in(&self, req: &SignInRequest<'_>) -> SignInReply;

    /// `GET /api/session`: what the sign-in screen needs to know before it asks (`passwordRequired`, `passwordSet`). Additive JSON.
    fn info(&self, _client: &Client) -> serde_json::Value {
        serde_json::json!({})
    }

    /// `DELETE /api/session`: ends the session `bearer` (sign out, "Lock"). Returns the status.
    fn end(&self, _bearer: &str) -> u16 {
        404
    }

    /// `POST /api/password` (`op` = `password`: set or change the screen password). Returns the status and the JSON body.
    fn account(
        &self,
        _op: &str,
        _bearer: Option<&str>,
        _client: &Client,
        _body: &serde_json::Value,
    ) -> (u16, String) {
        (404, String::new())
    }

    /// `POST /api/passkey/<op>` (WebAuthn, ADR-0015): `bearer` is the session token of the request, `host` its `Host`
    /// header. Returns the status and the JSON body. Hubs without passkeys answer 404.
    fn passkey(
        &self,
        _op: &str,
        _bearer: Option<&str>,
        _host: &str,
        _client: &Client,
        _body: &serde_json::Value,
    ) -> (u16, String) {
        (404, String::new())
    }

    /// `GET /api/audit?since=<ms>&who=<id>&limit=<n>` (ADR-0049): the audit trail as a JSON array, for the owner only. Hubs without
    /// sessions answer 404.
    fn audit(&self, _bearer: Option<&str>, _query: &str) -> (u16, String) {
        (404, String::new())
    }
}

pub enum SignInReply {
    Token(String),
    /// A wrong, expired or used code, or a wrong or missing password: one answer on purpose.
    Refused,
    /// Too many wrong tries from here (or from everywhere); try later.
    TooMany,
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

/// Reads the request head. A WebSocket upgrade is handed back untouched; other requests are answered here.
pub async fn route<S>(
    mut stream: S,
    peer: Option<std::net::IpAddr>,
    assets: Option<&dyn Assets>,
    sessions: Option<&dyn SessionApi>,
) -> io::Result<First<S>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut head = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    let end = loop {
        if let Some(i) = find(&head, b"\r\n\r\n") {
            break i + 4;
        }
        if head.len() > MAX_HEAD {
            respond(&mut stream, 431, "text/plain", b"", false).await?;
            return Ok(First::Answered);
        }
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(First::Answered);
        }
        head.extend_from_slice(&chunk[..n]);
    };
    let text = String::from_utf8_lossy(&head[..end]).to_string();
    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let upgrade = lines.any(|l| {
        l.split_once(':').is_some_and(|(k, v)| {
            k.trim().eq_ignore_ascii_case("upgrade") && v.trim().eq_ignore_ascii_case("websocket")
        })
    });
    if upgrade {
        return Ok(First::WebSocket(Rewind::new(head, stream)));
    }
    let mut parts = request_line.split(' ');
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let path = target.split(['?', '#']).next().unwrap_or("/");
    let head_only = method == "HEAD";
    let header = |name: &str| {
        text.split("\r\n")
            .skip(1)
            .filter_map(|l| l.split_once(':'))
            .find(|(k, _)| k.trim().eq_ignore_ascii_case(name))
            .map(|(_, v)| v.trim().to_string())
    };
    let bearer = || {
        header("authorization")
            .and_then(|v| v.strip_prefix("Bearer ").map(|t| t.trim().to_string()))
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
    if method == "POST" && (path == "/api/session" || passkey_op.is_some() || account_op.is_some())
    {
        let length = header("content-length")
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(usize::MAX);
        if length > MAX_BODY {
            respond(&mut stream, 413, "text/plain", b"", false).await?;
            return Ok(First::Answered);
        }
        let mut body = head[end..].to_vec();
        while body.len() < length {
            let n = stream.read(&mut chunk).await?;
            if n == 0 {
                return Ok(First::Answered);
            }
            body.extend_from_slice(&chunk[..n]);
        }
        body.truncate(length);
        let json = serde_json::from_slice::<serde_json::Value>(&body).ok();
        let (status, out) = if let Some(op) = passkey_op.or(account_op) {
            match (sessions, json) {
                (Some(api), Some(j)) if account_op.is_some() => {
                    api.account(op, bearer().as_deref(), &client, &j)
                }
                (Some(api), Some(j)) => {
                    let host = header("host").unwrap_or_default();
                    api.passkey(op, bearer().as_deref(), &host, &client, &j)
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
                    match api.sign_in(&req) {
                        SignInReply::Token(t) => {
                            (200, serde_json::json!({ "token": t }).to_string())
                        }
                        SignInReply::Refused => (401, String::new()),
                        SignInReply::TooMany => (429, String::new()),
                        SignInReply::Failed => (500, String::new()),
                    }
                }
                (None, _) => (404, String::new()),
                _ => (400, String::new()),
            }
        };
        // Every refusal costs the same short wait, whatever was wrong.
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
        Some(a) => respond(&mut stream, 200, a.content_type, &a.body, head_only).await?,
        None => respond(&mut stream, 404, "text/plain", b"not found", head_only).await?,
    }
    Ok(First::Answered)
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
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        409 => "Conflict",
        500 => "Internal Server Error",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        _ => "Request Header Fields Too Large",
    };
    // The app talks only to its own origin; nothing is framed, sniffed or loaded from elsewhere.
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         X-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nReferrer-Policy: no-referrer\r\n\
         Content-Security-Policy: default-src 'self'; connect-src 'self' ws: wss:; style-src 'self' 'unsafe-inline'; \
         img-src 'self' data:; frame-ancestors 'none'\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
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
        assert!(out.contains("Content-Security-Policy"));
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
}

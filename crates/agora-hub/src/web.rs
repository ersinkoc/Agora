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

/// What `POST /api/session` needs from the hub: trade a one-time sign-in code for a session token.
pub trait SessionApi: Send + Sync {
    fn sign_in(&self, code: &str) -> SignInReply;

    /// `POST /api/passkey/<op>` (WebAuthn, ADR-0015): `bearer` is the session token of the request, `host` its `Host`
    /// header. Returns the status and the JSON body. Hubs without passkeys answer 404.
    fn passkey(
        &self,
        _op: &str,
        _bearer: Option<&str>,
        _host: &str,
        _body: &serde_json::Value,
    ) -> (u16, String) {
        (404, String::new())
    }
}

pub enum SignInReply {
    Token(String),
    /// A wrong, expired or used code.
    Refused,
    /// Too many wrong guesses; try later.
    TooMany,
}

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
/// product string; colours are the design tokens' dark surface and accent.
pub fn manifest(name: &str) -> Vec<u8> {
    serde_json::json!({
        "name": name,
        "short_name": name,
        "start_url": "/",
        "display": "standalone",
        "background_color": "#09090b",
        "theme_color": "#09090b",
        "icons": [{ "src": "/icon.svg", "sizes": "any", "type": "image/svg+xml", "purpose": "any" }],
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
    let passkey_op = path.strip_prefix("/api/passkey/");
    if method == "POST" && (path == "/api/session" || passkey_op.is_some()) {
        let header = |name: &str| {
            text.split("\r\n")
                .filter_map(|l| l.split_once(':'))
                .find(|(k, _)| k.trim().eq_ignore_ascii_case(name))
                .map(|(_, v)| v.trim().to_string())
        };
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
        if let Some(op) = passkey_op {
            let bearer = header("authorization")
                .and_then(|v| v.strip_prefix("Bearer ").map(|t| t.trim().to_string()));
            let host = header("host").unwrap_or_default();
            let (status, out) = match (sessions, json) {
                (Some(api), Some(j)) => api.passkey(op, bearer.as_deref(), &host, &j),
                (None, _) => (404, String::new()),
                _ => (400, String::new()),
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
        let code = json.and_then(|v| v.get("code").and_then(|c| c.as_str()).map(str::to_string));
        match (sessions, code) {
            (Some(api), Some(code)) => match api.sign_in(&code) {
                SignInReply::Token(t) => {
                    let json = serde_json::json!({ "token": t }).to_string();
                    respond(&mut stream, 200, "application/json", json.as_bytes(), false).await?
                }
                SignInReply::Refused => respond(&mut stream, 401, "text/plain", b"", false).await?,
                SignInReply::TooMany => respond(&mut stream, 429, "text/plain", b"", false).await?,
            },
            (None, _) => respond(&mut stream, 404, "text/plain", b"", false).await?,
            _ => respond(&mut stream, 400, "text/plain", b"", false).await?,
        }
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
            route(s, Some(&One), None)
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
            match route(s, None, None).await.unwrap() {
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
}

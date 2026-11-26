//! `agora panel` (ADR-0056 Amendment A): opens the hub's web window on this machine without importing any certificate.
//!
//! The command runs the desktop app's loopback tunnel (`agora-desktop-tunnel`, ADR-0048) from the files enrolment left on this
//! machine: the hub's address and the CA the node pinned. The browser loads `http://127.0.0.1:<port>/`; the tunnel carries the
//! connection to the hub over TLS that trusts only that CA, and it refuses any request whose `Host` or `Origin` is not its own
//! (DNS rebinding, threat model T27). It reads no credential, holds no session and widens nothing: the person signs in to the hub
//! as from any browser, and the hub still sees an ordinary TLS client.
//!
//! The port is fixed per hub: the first run takes [`PREFERRED_PORT`] (or any free one when that is taken) and keeps it in
//! `<link dir>/panel-port`, because a device key and a sign-in belong to the page's origin, which includes the port. A later run
//! that finds its port taken by something else refuses to start rather than moving, since a new port would be a new device.
//! The listener exists only while the command runs, bound exclusively (no other program can bind the same port beside it, T27).
//!
//! With a person at the terminal on a machine with a screen, the command also asks the system to open the address in the browser
//! (`crate::frontdoor::may_open_browser`: not without a terminal, over SSH, on Linux without a display, in CI, or with
//! `AGORA_NO_BROWSER` set); it prints the address in any case.

use std::path::Path;
use std::process::ExitCode;

use agora_desktop_tunnel::{check, open, Tunnel, TunnelError};
use agora_i18n::key;

use crate::hubcli::link_dir;
use crate::Say;

/// The port the first run asks for. Not the hub's own ports (7443, 7444).
const PREFERRED_PORT: u16 = 7480;

/// Why the forwarder did not start.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PanelError {
    /// This machine holds no enrolment (no hub address or no CA).
    NotEnrolled,
    /// The stored hub address or CA is unusable.
    BrokenLink,
    /// The port kept for this hub is taken by something else.
    PortTaken(u16),
    /// No loopback port could be opened at all.
    NoPort,
    /// The hub does not answer.
    HubUnreachable,
    /// The hub's certificate is not from the pinned CA.
    HubUntrusted,
}

/// A running forwarder: dropping it closes the port.
pub(crate) struct Panel {
    _tunnel: Tunnel,
    pub(crate) port: u16,
    /// Whether the port is kept for the next run; when not, the next run may get another port, which is another origin.
    pub(crate) port_kept: bool,
}

/// `host:port` of the hub from the address a node was enrolled with (`wss://host:port` or `wss://host/path`); 443 when none.
fn hub_host_port(url: &str) -> Option<String> {
    let rest = url.trim().strip_prefix("wss://")?;
    let host = rest.split('/').next().filter(|h| !h.is_empty())?;
    let has_port = host
        .rsplit_once(':')
        .is_some_and(|(_, p)| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    Some(if has_port {
        host.to_string()
    } else {
        format!("{host}:443")
    })
}

fn stored_port(dir: &Path) -> Option<u16> {
    let p = std::fs::read_to_string(dir.join("panel-port")).ok()?;
    p.trim().parse().ok().filter(|p| *p != 0)
}

/// Starts the forwarder for the enrolment in `dir`. `preferred` is the port a first run asks for (0 = any free one).
/// Must run inside a Tokio runtime.
pub(crate) async fn start(dir: &Path, preferred: u16) -> Result<Panel, PanelError> {
    let read = |n: &str| std::fs::read_to_string(dir.join(n)).ok();
    let url = read("url").ok_or(PanelError::NotEnrolled)?;
    let ca = read("ca.pem").ok_or(PanelError::NotEnrolled)?;
    let hub = hub_host_port(&url).ok_or(PanelError::BrokenLink)?;
    // Without the hub the panel cannot work (ADR-0056 decision 5): say so now, not through a browser error page later.
    check(&hub, &ca).await.map_err(|e| match e {
        TunnelError::Untrusted => PanelError::HubUntrusted,
        TunnelError::BadCa | TunnelError::BadAddress => PanelError::BrokenLink,
        TunnelError::Unreachable | TunnelError::Listen => PanelError::HubUnreachable,
    })?;
    let tunnel = match stored_port(dir) {
        Some(p) => open(&hub, &ca, p)
            .await
            .map_err(|_| PanelError::PortTaken(p))?,
        None => match open(&hub, &ca, preferred).await {
            Ok(t) => t,
            Err(_) => open(&hub, &ca, 0).await.map_err(|_| PanelError::NoPort)?,
        },
    };
    let port = tunnel.local.port();
    // Owner-only like the rest of `link/`. A port that cannot be kept is reported, not hidden: the window works now, but the
    // next run may get another port, which is a new origin (a new device and sign-in).
    let port_kept = stored_port(dir) == Some(port)
        || agora_platform::write_private(&dir.join("panel-port"), port.to_string().as_bytes())
            .is_ok();
    Ok(Panel {
        _tunnel: tunnel,
        port,
        port_kept,
    })
}

/// `agora panel`: serves the hub's window on this machine until Ctrl+C.
pub fn panel(say: &Say) -> ExitCode {
    let Ok(rt) = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    else {
        say.err(key!("cli-panel-no-port"), &[]);
        return ExitCode::from(1);
    };
    run_to_end(rt, async {
        match start(&link_dir(), PREFERRED_PORT).await {
            Ok(p) => {
                let browser: Option<&Opener<'_>> = if crate::frontdoor::may_open_browser_here(true)
                {
                    Some(&agora_platform::open_url)
                } else {
                    None
                };
                // The system's opener may wait a few seconds for the browser; the forwarder runs on the other workers meanwhile.
                tokio::task::block_in_place(|| announce(say, p.port, browser));
                if !p.port_kept {
                    say.err(key!("cli-panel-port-unsaved"), &[]);
                }
                // Dropping `p` after this closes the port; connections already open end with the process.
                let _ = tokio::signal::ctrl_c().await;
                ExitCode::SUCCESS
            }
            Err(e) => {
                let (id, args): (&str, Vec<(&str, crate::Arg)>) = match e {
                    PanelError::NotEnrolled => (key!("cli-panel-none"), vec![]),
                    PanelError::BrokenLink => (key!("cli-panel-link"), vec![]),
                    PanelError::PortTaken(p) => (
                        key!("cli-panel-port-taken"),
                        vec![("port", p.to_string().into())],
                    ),
                    PanelError::NoPort => (key!("cli-panel-no-port"), vec![]),
                    PanelError::HubUnreachable => (key!("cli-panel-unreachable"), vec![]),
                    PanelError::HubUntrusted => (key!("cli-panel-untrusted"), vec![]),
                };
                say.err(id, &args);
                ExitCode::from(1)
            }
        }
    })
}

/// What asks the system to show an address in the browser ([`agora_platform::open_url`]; a recorder in tests).
type Opener<'a> = dyn Fn(&str) -> std::io::Result<()> + 'a;

/// Says where the panel is and, when `browser` is given (a person at a terminal with a screen, see
/// [`crate::frontdoor::may_open_browser`]), asks it to open the address (ADR-0056 Amendment A). The address is printed in any case,
/// so a browser that does not open costs nothing. Returns whether a browser was asked and said yes.
fn announce(say: &Say, port: u16, browser: Option<&Opener<'_>>) -> bool {
    let url = format!("http://127.0.0.1:{port}/");
    say.out(key!("cli-panel"), &[("url", url.clone().into())]);
    let opened = browser.is_some_and(|open| open(&url).is_ok());
    if opened {
        say.out(key!("cli-panel-opened"), &[]);
    }
    opened
}

/// Runs `fut` on `rt` and returns as soon as it ends. Dropping the runtime would wait, with no limit, for its blocking pool: a hub
/// that did not answer within its 10 s while its name was still being looked up (`lookup_host` runs there and cannot be stopped) would
/// keep `agora panel` from ending, after it said so, until the resolver gave up (#1150). That lookup is left to end on its own.
fn run_to_end<F: std::future::Future>(rt: tokio::runtime::Runtime, fut: F) -> F::Output {
    let out = rt.block_on(fut);
    rt.shutdown_background();
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::net::SocketAddr;
    use std::time::Duration;

    use agora_hub::ca::Ca;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    use super::*;

    /// A TLS hub for `localhost` signed by `ca`: answers a plain request with `hello <request line>`; on a WebSocket upgrade it
    /// answers 101 and echoes what follows. Counts the connections it accepts.
    async fn fake_hub(ca: &Ca) -> (SocketAddr, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let (cert, key) = ca.issue_server(&["localhost".to_string()]).unwrap();
        let acceptor = agora_hub::tls::acceptor_with(&cert, &key, ca.cert_pem(), true).unwrap();
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        let seen = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = seen.clone();
        tokio::spawn(async move {
            while let Ok((s, _)) = l.accept().await {
                let a = acceptor.clone();
                let counter = counter.clone();
                tokio::spawn(async move {
                    let Ok(mut s) = a.accept(s).await else { return };
                    counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let mut head = Vec::new();
                    let mut b = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        if s.read_exact(&mut b).await.is_err() {
                            return;
                        }
                        head.push(b[0]);
                    }
                    let text = String::from_utf8_lossy(&head).to_string();
                    if text.contains("Upgrade: websocket") {
                        let _ = s
                            .write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n")
                            .await;
                        let mut buf = [0u8; 64];
                        while let Ok(n) = s.read(&mut buf).await {
                            if n == 0 || s.write_all(&buf[..n]).await.is_err() {
                                break;
                            }
                        }
                    } else {
                        let line = text.split("\r\n").next().unwrap_or_default();
                        let _ = s.write_all(format!("hello {line}").as_bytes()).await;
                        let _ = s.shutdown().await;
                    }
                });
            }
        });
        (addr, seen)
    }

    /// An enrolment folder for a hub at `hub`.
    fn enrolment(tag: &str, hub: SocketAddr, ca: &Ca) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("agora-panel-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("url"), format!("wss://localhost:{}\n", hub.port())).unwrap();
        std::fs::write(dir.join("ca.pem"), ca.cert_pem()).unwrap();
        dir
    }

    async fn send(port: u16, request: &str) -> String {
        let mut c = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        c.write_all(request.as_bytes()).await.unwrap();
        let mut out = String::new();
        let _ = tokio::time::timeout(Duration::from_secs(5), c.read_to_string(&mut out)).await;
        out
    }

    const REFUSED: &str =
        "HTTP/1.1 421 Misdirected Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

    /// #1150: `agora panel` ends when its wait for the hub ran out, also while a blocking task (a host name lookup) is still running;
    /// the runtime used to wait for the task on its way out, so the command outlasted its 10 s wait by the resolver's own timeout.
    #[test]
    fn a_wait_that_ran_out_does_not_wait_for_a_lookup_still_running() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let start = std::time::Instant::now();
        let got = run_to_end(rt, async {
            let slow = tokio::task::spawn_blocking(|| std::thread::sleep(Duration::from_secs(5)));
            tokio::time::timeout(Duration::from_millis(200), slow).await
        });
        assert!(got.is_err());
        let took = start.elapsed();
        assert!(took < Duration::from_secs(3), "took {took:?}");
    }

    /// ADR-0056 Amendment A: the panel's own loopback address is what the browser is asked to open, and only when a browser may be
    /// opened at all; the address is printed either way. Never a real browser: the opener is a recorder.
    #[test]
    fn the_browser_is_asked_for_the_panels_address_only_when_one_may_be_opened() {
        let say = Say(agora_i18n::Catalog::english());
        let asked = std::sync::Mutex::new(Vec::<String>::new());
        let record = |u: &str| -> std::io::Result<()> {
            asked.lock().unwrap().push(u.to_string());
            Ok(())
        };
        assert!(announce(&say, 7480, Some(&record)));
        assert_eq!(*asked.lock().unwrap(), vec!["http://127.0.0.1:7480/"]);
        // The address is one the system's opener accepts.
        assert!(agora_platform::is_openable_url("http://127.0.0.1:7480/"));
        // Negative control: no browser allowed, nothing is asked.
        assert!(!announce(&say, 7480, None));
        assert_eq!(asked.lock().unwrap().len(), 1);
        // A browser that cannot be opened is not an error: the address was printed.
        let refuse = |_: &str| -> std::io::Result<()> { Err(std::io::ErrorKind::NotFound.into()) };
        assert!(!announce(&say, 7480, Some(&refuse)));
    }

    #[test]
    fn the_hub_address_gives_host_and_port() {
        assert_eq!(
            hub_host_port("wss://hub.lan:7444\n").as_deref(),
            Some("hub.lan:7444")
        );
        assert_eq!(
            hub_host_port("wss://localhost:7443/x").as_deref(),
            Some("localhost:7443")
        );
        assert_eq!(
            hub_host_port("wss://hub.lan").as_deref(),
            Some("hub.lan:443")
        );
        assert_eq!(hub_host_port("wss://[::1]").as_deref(), Some("[::1]:443"));
        assert_eq!(
            hub_host_port("wss://[::1]:7443").as_deref(),
            Some("[::1]:7443")
        );
        assert_eq!(hub_host_port("ws://hub:1"), None);
        assert_eq!(hub_host_port("wss://"), None);
    }

    #[tokio::test]
    async fn the_page_and_its_websocket_pass_through_to_the_hub_and_only_to_the_right_names() {
        let ca = Ca::generate("hub CA").unwrap();
        let (hub, seen) = fake_hub(&ca).await;
        let dir = enrolment("pass", hub, &ca);
        let p = start(&dir, 0).await.unwrap();
        let port = p.port;
        // The forwarder is on loopback only.
        let probe = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
        if probe.connect("192.0.2.1:9").is_ok() {
            let ip = probe.local_addr().unwrap().ip();
            if !ip.is_loopback() && !ip.is_unspecified() {
                let r =
                    tokio::time::timeout(Duration::from_secs(2), TcpStream::connect((ip, port)))
                        .await;
                assert!(!matches!(r, Ok(Ok(_))), "reachable on {ip}");
            }
        }
        // A plain request, under both names.
        for host in [format!("127.0.0.1:{port}"), format!("localhost:{port}")] {
            let got = send(port, &format!("GET /app HTTP/1.1\r\nHost: {host}\r\n\r\n")).await;
            assert_eq!(got, "hello GET /app HTTP/1.1");
        }
        // A WebSocket upgrade from the panel's own origin, then bytes both ways.
        let mut c = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let up = format!(
            "GET /ws HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n"
        );
        c.write_all(up.as_bytes()).await.unwrap();
        let mut got = vec![0u8; 12];
        c.read_exact(&mut got).await.unwrap();
        assert!(got.starts_with(b"HTTP/1.1 101"));
        c.write_all(b"ping").await.unwrap();
        let mut rest = Vec::new();
        let mut b = [0u8; 1];
        while !rest.ends_with(b"\r\n\r\nping") {
            c.read_exact(&mut b).await.unwrap();
            rest.push(b[0]);
        }
        let before = seen.load(std::sync::atomic::Ordering::SeqCst);
        // DNS rebinding: the attacker's name, with and without the right port; and a foreign Origin on the upgrade.
        for req in [
            format!("GET /app HTTP/1.1\r\nHost: evil.example:{port}\r\n\r\n"),
            "GET /app HTTP/1.1\r\nHost: evil.example\r\n\r\n".to_string(),
            format!("GET /app HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n", port.wrapping_add(1)),
            format!(
                "GET /ws HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://evil.example:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n"
            ),
            format!(
                "POST /x HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: null\r\nContent-Length: 0\r\n\r\n"
            ),
        ] {
            assert_eq!(send(port, &req).await, REFUSED, "{req}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            seen.load(std::sync::atomic::Ordering::SeqCst),
            before,
            "a refused request reached the hub"
        );
        // The port is kept for the next run.
        assert_eq!(stored_port(&dir), Some(port));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_port_that_cannot_be_kept_is_reported_and_a_kept_one_is_owner_only() {
        let ca = Ca::generate("hub CA").unwrap();
        let (hub, _) = fake_hub(&ca).await;
        let dir = enrolment("unsaved", hub, &ca);
        let p = start(&dir, 0).await.unwrap();
        assert!(p.port_kept);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("panel-port"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0);
        }
        drop(p);
        // Negative control above; now a folder where the file should be: the write fails and the run says so.
        std::fs::remove_file(dir.join("panel-port")).unwrap();
        std::fs::create_dir(dir.join("panel-port")).unwrap();
        let p = start(&dir, 0).await.unwrap();
        assert!(!p.port_kept);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn closing_the_panel_closes_the_port_and_the_next_run_gets_the_same_one() {
        let ca = Ca::generate("hub CA").unwrap();
        let (hub, _) = fake_hub(&ca).await;
        let dir = enrolment("again", hub, &ca);
        let p = start(&dir, 0).await.unwrap();
        let port = p.port;
        drop(p);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
        let p = start(&dir, 0).await.unwrap();
        assert_eq!(p.port, port);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_port_taken_by_something_else_refuses_and_keeps_the_stored_port() {
        let ca = Ca::generate("hub CA").unwrap();
        let (hub, _) = fake_hub(&ca).await;
        let dir = enrolment("taken", hub, &ca);
        let other = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let taken = other.local_addr().unwrap().port();
        std::fs::write(dir.join("panel-port"), taken.to_string()).unwrap();
        assert_eq!(
            start(&dir, 0).await.err(),
            Some(PanelError::PortTaken(taken))
        );
        assert_eq!(stored_port(&dir), Some(taken));
        // A first run that finds its preferred port taken takes another free one instead.
        std::fs::remove_file(dir.join("panel-port")).unwrap();
        let p = start(&dir, taken).await.unwrap();
        assert_ne!(p.port, taken);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_missing_enrolment_an_unreachable_hub_and_a_foreign_ca_start_nothing() {
        let ca = Ca::generate("hub CA").unwrap();
        let (hub, _) = fake_hub(&ca).await;
        let empty = std::env::temp_dir().join(format!("agora-panel-none-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&empty);
        assert_eq!(start(&empty, 0).await.err(), Some(PanelError::NotEnrolled));
        // Negative control: the same folder with a hub that answers and the right CA starts.
        let dir = enrolment("neg", hub, &ca);
        assert!(start(&dir, 0).await.is_ok());
        // Another CA than the hub's: refused, and no port file is written.
        let other = Ca::generate("another CA").unwrap();
        let wrong = enrolment("wrongca", hub, &other);
        assert_eq!(start(&wrong, 0).await.err(), Some(PanelError::HubUntrusted));
        assert_eq!(stored_port(&wrong), None);
        // Nobody listens at the hub's address.
        let gone = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let gone_addr = gone.local_addr().unwrap();
        drop(gone);
        let down = enrolment("down", gone_addr, &ca);
        assert_eq!(
            start(&down, 0).await.err(),
            Some(PanelError::HubUnreachable)
        );
        for d in [&empty, &dir, &wrong, &down] {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}

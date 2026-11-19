//! `agora hub run|code` and `agora node enroll`: the commands that connect machines to a hub. The hub keeps its state in
//! `<hub dir>` (default `$AGORA_HOME/hub`, a folder only the user can enter): the database, its certificate authority (key
//! readable only by the user) and the client token. A node keeps what enrollment gave it in `<AGORA_HOME>/link`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use agora_hub::ca::{new_device_request, Ca};
use agora_hub::enroll::{Enrollment, Lockout};
use agora_hub::hub::Hub;
use agora_hub::server::{self, ClientAuth, ServerState};
use agora_hub::signin::{HubSessions, SignIn};
use agora_hub::store::SqliteStore;
use agora_hub::tls;
use agora_i18n::key;
use agora_node::uplink_client::{enroll, HubLink, LinkError, SILENCE_DEADLINE};
use agora_protocol::handshake::Support;

use crate::{home, Opts, Say};

pub(crate) const DEFAULT_LISTEN: &str = "127.0.0.1:7443";

/// What this hub agrees on with a node that offers it. Every capability a node offers
/// (`agora_node::uplink_client::CAPABILITIES`) must be here, or the hub refuses each command that needs it (ADR-0053's
/// `directory.list` was missing once, #378); `the_hub_agrees_on_every_capability_a_node_offers` checks it. A capability added here
/// that no node offers (only the hub or clients use it) is not covered by that check: give it a test of its own.
const HUB_CAPABILITIES: &[&str] = &[
    "terminal.raw",
    "workspace.directory",
    "directory.list",
    "board.join",
    "mesh.spawn",
    // ADR-0058
    "agent.set_mode",
    "agent.set_model",
    // ADR-0090: chat entries that change in place (tool calls and plans, with status, output, thinking, permissions and queued
    // prompts), `AgentPromptResult`, `AgentSetConfig`, `AgentCancelTurn` and `AgentCommands`: the hub relays and gates each (a node offers them).
    "chat.rich",
    "agent.set_config",
    "agent.cancel_turn",
    "agent.commands",
    // ADR-0060
    "agent.resume",
    // ADR-0059
    "agent.launch_options",
    // ADR-0066
    "agent.launch_settings",
    // ADR-0061: the hub carries endorsements and revocations to nodes that verify them (`device.trust`), and answers a node's
    // `DeviceList` on a connection of its own (`device.list`).
    "device.trust",
    "device.list",
    // ADR-0085: answers a machine's `RenewRequest` on the connection it makes with its certificate.
    "enroll.renew",
    // ADR-0069: the hub keeps `NodeOnline.binary` in its snapshot (`NodeSnapshot.binary`) and passes `AgentKind.program` through.
    "kinds.program",
    // ADR-0065: speaks the pairing messages (`pairing.v1`, docs/specs/pairing.md). Nothing sends them yet.
    "pairing.v1",
];

/// `<hub dir>/listen`: the address the running hub bound, written at every start.
pub(crate) const LISTEN_FILE: &str = "listen";

/// The address a client on the hub's own machine dials, from `<hub dir>/listen`. A hub listening on every address
/// (`0.0.0.0` or `::`) is reached on loopback. `None` when the file is missing or unreadable.
pub(crate) fn own_hub_addr(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(LISTEN_FILE)).ok()?;
    local_dial_addr(text.trim())
}

fn local_dial_addr(listen: &str) -> Option<String> {
    let addr: std::net::SocketAddr = listen.parse().ok()?;
    let ip = match addr.ip() {
        std::net::IpAddr::V4(ip) if ip.is_unspecified() => std::net::Ipv4Addr::LOCALHOST.into(),
        std::net::IpAddr::V6(ip) if ip.is_unspecified() => std::net::Ipv6Addr::LOCALHOST.into(),
        ip => ip,
    };
    Some(std::net::SocketAddr::new(ip, addr.port()).to_string())
}

pub(crate) fn hub_dir(o: &Opts) -> PathBuf {
    o.hub_dir.clone().unwrap_or_else(|| home().join("hub"))
}

/// `agora hub <command>` on a machine that does not run the hub: there is no hub folder, so say that the command belongs on the hub's
/// machine instead of showing the database error it would hit.
pub fn not_here(say: &Say, o: &Opts) -> ExitCode {
    say.err(
        key!("cli-hub-not-here"),
        &[("dir", hub_dir(o).display().to_string().into())],
    );
    ExitCode::from(1)
}

pub fn link_dir() -> PathBuf {
    home().join("link")
}

/// Makes the hub's folder, closed to every other user (audit F1001): `hub.db` and its `-wal`/`-shm` are created with the default mode,
/// and they hold the hashes of the live one-time codes (short enough to search for offline within their 10 minutes), of the sessions
/// and of the screen password, and the event history. An existing folder is closed too, so a hub made by an older version is closed at
/// its next start.
fn private_hub_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    agora_platform::restrict_to_owner(dir).map_err(|e| format!("{}: {e}", dir.display()))
}

fn random_token() -> Option<String> {
    let mut b = [0u8; 16];
    getrandom::getrandom(&mut b).ok()?;
    Some(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// Loads the hub's CA from `dir`, creating it the first time.
fn load_or_create_ca(dir: &Path) -> Result<Ca, String> {
    let (cert, key) = (dir.join("ca.pem"), dir.join("ca.key"));
    match (cert.exists(), key.exists()) {
        (true, true) => {
            let c = std::fs::read_to_string(&cert).map_err(|e| format!("{}: {e}", cert.display()))?;
            let k = std::fs::read_to_string(&key).map_err(|e| format!("{}: {e}", key.display()))?;
            return Ca::from_pem(&c, &k).map_err(|e| e.to_string());
        }
        // One half alone is damage, not a fresh start: making a new CA would cut off every enrolled node.
        (true, false) | (false, true) => {
            return Err(format!(
                "{} and {} must both exist; restore the missing one from a backup (a new CA would disconnect every node)",
                cert.display(),
                key.display()
            ))
        }
        (false, false) => {}
    }
    let ca = Ca::generate("Agora hub CA").map_err(|e| e.to_string())?;
    std::fs::write(&cert, ca.cert_pem()).map_err(|e| e.to_string())?;
    agora_platform::write_private(&key, ca.key_pem().as_bytes()).map_err(|e| e.to_string())?;
    Ok(ca)
}

/// Who may connect as a client: a live session (from the web sign-in) or the long-lived token in `<hub dir>/token`
/// that scripts use.
struct ClientTokens {
    file_token: String,
    sessions: Arc<HubSessions>,
}

impl ClientAuth for ClientTokens {
    fn check(&self, token: &str) -> bool {
        // Not constant time, but the token is 128 random bits compared once per connection; nothing to learn from timing.
        token == self.file_token || self.sessions.check(token)
    }

    fn access(&self, token: &str) -> Option<agora_hub::users::Access> {
        if token == self.file_token {
            return Some(agora_hub::users::Access {
                role: agora_hub::users::Role::Owner,
                user: "owner".into(),
                projects: None,
                session: String::new(),
            });
        }
        agora_hub::server::ClientAuth::access(&*self.sessions, token)
    }
}

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/web_files.rs"));
}

/// The web app the hub serves: `--web DIR` wins, else the copy compiled into this binary (when the build had one).
fn web_assets(o: &Opts) -> Option<Arc<dyn agora_hub::web::Assets>> {
    let inner: Arc<dyn agora_hub::web::Assets> = if let Some(dir) = &o.web {
        Arc::new(agora_hub::web::DirAssets::new(dir))
    } else if embedded::WEB_FILES.is_empty() {
        return None;
    } else {
        Arc::new(agora_hub::web::EmbeddedAssets(embedded::WEB_FILES))
    };
    // ADR-0046: the page can be installed to a phone's home screen; its name is the locale's product string.
    let name = agora_i18n::Catalog::for_locale(&agora_i18n::catalog::locale_from_env())
        .render("notify-title", &[])
        .unwrap_or_default();
    Some(Arc::new(agora_hub::web::Installable {
        inner,
        manifest: agora_hub::web::manifest(&name),
    }))
}

pub fn hub_run(say: &Say, o: &Opts) -> ExitCode {
    let dir = hub_dir(o);
    // ADR-0067: a hub is never opened to the internet without the screen password.
    if let Err(code) =
        crate::remotecmd::check_listen(say, &dir, o.listen.as_deref().unwrap_or(DEFAULT_LISTEN))
    {
        return code;
    }
    let run = || -> Result<(), String> {
        private_hub_dir(&dir)?;
        let ca = load_or_create_ca(&dir)?;
        let token_path = dir.join("token");
        let token = match std::fs::read_to_string(&token_path) {
            Ok(t) if !t.trim().is_empty() => t.trim().to_string(),
            _ => {
                let t = random_token().ok_or("no system randomness")?;
                agora_platform::write_private(&token_path, t.as_bytes())
                    .map_err(|e| e.to_string())?;
                t
            }
        };
        let has_web = web_assets(o).is_some();
        let store = SqliteStore::open(&dir.join("hub.db")).map_err(|e| e.to_string())?;
        let hub = Arc::new(Hub::open(store).map_err(|e| e.to_string())?);
        // R8: `GET /api/metrics` answers (owner only) when asked for at start.
        if o.metrics {
            hub.enable_metrics();
        }
        // Where notifications go, if the owner listed any (docs/design/notifications.md).
        let notify_file = dir.join("notify");
        if let Ok(text) = std::fs::read_to_string(&notify_file) {
            let (targets, skipped) = agora_hub::deliver::parse_targets(&text);
            let file = notify_file.display().to_string();
            if skipped > 0 {
                say.err(
                    key!("cli-hub-notify-skipped"),
                    &[
                        ("count", (skipped as i64).into()),
                        ("file", file.clone().into()),
                    ],
                );
            }
            if !targets.is_empty() {
                say.out(
                    key!("cli-hub-notify"),
                    &[
                        ("count", (targets.len() as i64).into()),
                        ("file", file.into()),
                    ],
                );
            }
            agora_hub::notifier::spawn(
                Arc::clone(&hub),
                targets,
                agora_i18n::Catalog::for_locale(&agora_i18n::catalog::locale_from_env()),
                std::fs::read_to_string(dir.join("public-url"))
                    .ok()
                    .map(|u| u.trim().to_string())
                    .filter(|u| u.starts_with("https://") || u.starts_with("http://")),
            );
        }
        let (prices, prices_at, bad_overrides) = load_prices(&dir);
        hub.set_prices(prices, prices_at);
        if bad_overrides > 0 {
            say.err(
                key!("cli-hub-prices-skipped"),
                &[("count", (bad_overrides as i64).into())],
            );
        }
        // ADR-0067 rung 4: a reverse proxy is trusted only while `<hub dir>/remote-proxy` says so (read again every few seconds).
        let sessions = Arc::new(HubSessions::new(Arc::clone(&hub)).with_proxy(Arc::new(
            agora_hub::proxy::ProxyHandle::watch(
                dir.join(agora_hub::proxy::FILE),
                agora_platform::open_read_no_follow,
            ),
        )));
        if let agora_hub::proxy::Stored::On(t) = crate::remotecmd::read_proxy(&dir) {
            say.out(
                key!("cli-hub-proxy-on"),
                &[("hosts", t.hosts().join(", ").into())],
            );
        }
        // A fresh one-time code for the first sign-in; `agora hub login` makes more.
        let web_code = SignIn::new(hub.store(), Arc::new(Lockout::new()))
            .create_code(server::now_ms())
            .map_err(|_| "could not create a sign-in code".to_string())?;
        let state = Arc::new(ServerState {
            hub,
            support: Support {
                generations: vec![1],
                capabilities: HUB_CAPABILITIES.iter().map(|c| (*c).to_string()).collect(),
                required: vec![],
            },
            ca: Some(ca),
            lockout: Arc::new(Lockout::new()),
            auth: Box::new(ClientTokens {
                file_token: token,
                sessions: Arc::clone(&sessions),
            }),
            assets: web_assets(o),
            sessions: Some(sessions),
        });
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        rt.block_on(async {
            let listener =
                tokio::net::TcpListener::bind(o.listen.as_deref().unwrap_or(DEFAULT_LISTEN))
                    .await
                    .map_err(|e| e.to_string())?;
            let addr = listener.local_addr().map_err(|e| e.to_string())?;
            // ADR-0065, spec section 5b: the hub's certificate names the addresses the hub is reachable at (the `--name` values,
            // `localhost`, the host name, the addresses of its interfaces), from the same CA. It is made again at every
            // `agora pair open` when an address is missing; the listener takes the new one for new connections.
            let leaf = start_leaf(say, &state, &dir, addr.ip(), &o.names)?;
            let acceptor_slot = leaf.slot();
            watch_remote_names(&dir, &leaf);
            keep_certificates_fresh(&state, &leaf);
            // The hub can pair once it knows the hash of the certificate machines will see; the switch is closed until the owner
            // opens it.
            state.hub.pairing().enable(leaf.hash());
            state
                .hub
                .pairing()
                .provision_names(Arc::clone(&leaf) as Arc<dyn agora_hub::leaf::Provision>);
            let _discovery = start_discovery(&state, &leaf, addr.port());
            // Where this hub really listens (a port of its own, or `:0`), for a client on this machine that only has the folder
            // (`agora tui --hub-dir`). Best effort: a client without it falls back to the default address, as before.
            let _ =
                agora_platform::write_private(&dir.join(LISTEN_FILE), addr.to_string().as_bytes());
            say.out(
                key!("cli-hub-listening"),
                &[
                    ("addr", addr.to_string().into()),
                    ("dir", dir.display().to_string().into()),
                ],
            );
            say.out(
                key!("cli-hub-names"),
                &[("names", leaf.names().join(", ").into())],
            );
            if crate::remotecmd::exposure(addr.ip()).public {
                say.out(key!("remote-public-warning"), &[("cause", "listen".into())]);
            }
            let _tunnels = start_tunnels(say, &dir, addr);
            if has_web {
                say.out(
                    key!("cli-hub-web"),
                    &[("addr", addr.to_string().into()), ("code", web_code.into())],
                );
            }
            server::run_slot(listener, state, acceptor_slot).await;
            Ok(())
        })
    };
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            say.err(key!("cli-hub-failed"), &[("reason", e.into())]);
            ExitCode::from(1)
        }
    }
}

/// The hub's server certificate and the means to make it again with other names (docs/specs/pairing.md section 5b).
fn start_leaf(
    say: &Say,
    state: &ServerState,
    dir: &Path,
    listen: std::net::IpAddr,
    names: &[String],
) -> Result<Arc<agora_hub::leaf::Leaf>, String> {
    use agora_hub::names::Iface;
    let ca = state.ca.as_ref().ok_or("no certificate authority")?;
    // The leaf signs with its own copy of the CA key, loaded from the same PEM.
    let ca = Ca::from_pem(ca.cert_pem(), &ca.key_pem()).map_err(|e| e.to_string())?;
    // The names `agora remote tailscale` added (ADR-0067 rung 3) are in the first certificate, and are read again at every issue.
    let mut all_names = names.to_vec();
    for n in crate::remotecmd::saved_names(dir) {
        if !all_names.contains(&n) && all_names.len() < agora_hub::names::MAX_NAMES {
            all_names.push(n);
        }
    }
    let (leaf, failure) = agora_hub::leaf::Leaf::start(
        ca,
        all_names,
        listen,
        Box::new(agora_platform::hostname),
        Box::new(|| {
            agora_platform::interface_addresses()
                .map(|v| {
                    v.into_iter()
                        .map(|a| Iface {
                            ip: a.ip,
                            up: a.up,
                            loopback: a.loopback,
                            transient: a.transient,
                        })
                        .collect()
                })
                .map_err(|e| e.to_string())
        }),
        Some(Box::new({
            let file = dir.join("server-cert.pem");
            // Atomic and owner-only, like the other files of the hub's folder.
            move |pem| agora_platform::write_private(&file, pem.as_bytes())
        })),
    )?;
    if failure.is_some() {
        say.err(key!("cli-hub-names-unlisted"), &[]);
    }
    let remote_dir = dir.to_path_buf();
    let leaf = leaf.with_extra(Box::new(move || crate::remotecmd::saved_names(&remote_dir)));
    Ok(Arc::new(leaf))
}

/// ADR-0085: once at start and then every day, replaces the hub's own server certificate before it ends (when fewer than
/// [`agora_hub::ca::EXPIRY_WARN_DAYS`] days remain: new connections use the new one, open ones are untouched, `server-cert.pem` is
/// rewritten) and revokes the older certificates of machines that connected with a newer one a week ago. The thread lives as long as
/// the process and survives a failure or a panic of one round: it says so on the log at warn level and tries again after a wait that
/// doubles from 5 minutes up to an hour, then returns to daily when a round works.
fn keep_certificates_fresh(state: &Arc<ServerState>, leaf: &Arc<agora_hub::leaf::Leaf>) {
    const DAY: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
    const FIRST_RETRY: std::time::Duration = std::time::Duration::from_secs(5 * 60);
    const LAST_RETRY: std::time::Duration = std::time::Duration::from_secs(60 * 60);
    let (state, leaf) = (Arc::clone(state), Arc::clone(leaf));
    let _ = std::thread::Builder::new()
        .name("certificates".into())
        .spawn(move || {
            let mut retry = FIRST_RETRY;
            loop {
                let round = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    certificate_round(&state, &leaf)
                }));
                let wait = match round {
                    Ok(true) => {
                        retry = FIRST_RETRY;
                        DAY
                    }
                    Ok(false) => {
                        let w = retry;
                        retry = (retry * 2).min(LAST_RETRY);
                        w
                    }
                    Err(_) => {
                        eprintln!("warn: the certificate round panicked; trying again later");
                        let w = retry;
                        retry = (retry * 2).min(LAST_RETRY);
                        w
                    }
                };
                std::thread::sleep(wait);
            }
        });
}

/// One round of [`keep_certificates_fresh`]: `false` when something failed (and was logged).
fn certificate_round(state: &ServerState, leaf: &agora_hub::leaf::Leaf) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    let mut ok = true;
    match leaf.renew_if_ending(now, agora_hub::ca::EXPIRY_WARN_DAYS * 24 * 60 * 60) {
        // Pairing in flight is not disturbed: a session froze the hash of the certificate its own connection was shown.
        Ok(true) => state.hub.pairing().enable(leaf.hash()),
        Ok(false) => {}
        Err(e) => {
            eprintln!("warn: the hub's certificate could not be renewed: {e}");
            ok = false;
        }
    }
    if let Err(e) = Enrollment::new(state.hub.store()).revoke_superseded(server::now_ms()) {
        eprintln!("warn: older machine certificates could not be revoked: {e:?}");
        ok = false;
    }
    ok
}

/// Gives the certificate the names `agora remote tailscale` writes while the hub runs: the file is looked at every few seconds, and a
/// change makes the hub issue a new certificate from its own CA (names only ever come from that file, which checks every one). The
/// thread lives as long as the process.
fn watch_remote_names(dir: &Path, leaf: &Arc<agora_hub::leaf::Leaf>) {
    use agora_hub::leaf::Provision;
    let (file, leaf) = (dir.join(crate::remotecmd::NAMES_FILE), Arc::clone(leaf));
    let _ = std::thread::Builder::new()
        .name("remote-names".into())
        .spawn(move || {
            let read = || std::fs::read(&file).ok();
            let mut seen = read();
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3));
                let now = read();
                if now != seen {
                    seen = now;
                    let _ = leaf.provision();
                }
            }
        });
}

/// Answers "is there a hub here?" on the local network while, and only while, the hub is open for pairing (spec section 3). The
/// thread lives as long as the process: `hub_run` returns, and the process ends, only when the hub's listener ends.
fn start_discovery(
    state: &Arc<ServerState>,
    leaf: &Arc<agora_hub::leaf::Leaf>,
    hub_port: u16,
) -> Option<std::thread::JoinHandle<()>> {
    use agora_hub::discovery::{Config, Responder, DISCOVERY_PORT};
    let fingerprint = tls::leaf_hash(state.ca.as_ref()?.cert_pem()).ok()?;
    let leaf = Arc::clone(leaf);
    let responder = Responder::new(Config {
        port: DISCOVERY_PORT,
        bind: std::net::Ipv4Addr::UNSPECIFIED,
        name: Box::new(|| agora_platform::hostname().unwrap_or_else(|| "agora".to_string())),
        hub_port,
        ca_fingerprint: fingerprint,
        version: env!("CARGO_PKG_VERSION"),
        addresses: Box::new(move || leaf.addresses()),
    });
    let state = Arc::clone(state);
    Some(std::thread::spawn(move || loop {
        // A port another hub on this machine holds is not an error worth stopping for: this one stays silent and tries again.
        let _ = responder.sync(state.hub.pairing().is_open());
        std::thread::sleep(std::time::Duration::from_millis(500));
    }))
}

/// A fresh one-time sign-in code from the hub's own store (valid 10 minutes).
pub fn new_login_code(dir: &Path) -> Result<String, String> {
    private_hub_dir(dir)?;
    let store = SqliteStore::open(&dir.join("hub.db")).map_err(|e| e.to_string())?;
    SignIn::new(&store, Arc::new(Lockout::new()))
        .create_code(server::now_ms())
        .map_err(|_| "could not create a sign-in code".to_string())
}

/// `agora hub fingerprint`: the SHA-256 of the hub's CA certificate, for the desktop app's first-use check (ADR-0048).
pub fn hub_fingerprint(say: &Say, o: &Opts) -> ExitCode {
    let path = hub_dir(o).join("ca.pem");
    let result = std::fs::read_to_string(&path)
        .map_err(|_| path.display().to_string())
        .and_then(|pem| {
            agora_desktop_tunnel::fingerprint(&pem).map_err(|_| path.display().to_string())
        });
    match result {
        Ok(f) => {
            say.out(key!("cli-hub-fingerprint"), &[("fingerprint", f.into())]);
            ExitCode::SUCCESS
        }
        Err(p) => {
            say.err(key!("cli-hub-failed"), &[("reason", p.into())]);
            ExitCode::from(1)
        }
    }
}

/// `agora hub login`: a one-time code to sign in to the web app (valid 10 minutes).
pub fn hub_login(say: &Say, o: &Opts) -> ExitCode {
    let result = new_login_code(&hub_dir(o));
    match result {
        Ok(code) => {
            say.out(key!("cli-hub-login"), &[("code", code.into())]);
            ExitCode::SUCCESS
        }
        Err(e) => {
            say.err(key!("cli-hub-failed"), &[("reason", e.into())]);
            ExitCode::from(1)
        }
    }
}

/// `agora password set|clear|status` (ADR-0067): the screen password a sign-in from another machine must give. Run on the hub's
/// machine, so no old password is asked. Setting reads the password twice from the terminal with echo off and refuses to read one
/// from a pipe; setting or clearing it signs out every session, and both are in the audit trail.
pub fn password(say: &Say, o: &Opts, op: &str) -> ExitCode {
    use agora_hub::password::{check_new, PasswordError, ScreenPassword};
    let fail = |e: String| {
        say.err(key!("cli-hub-failed"), &[("reason", e.into())]);
        ExitCode::from(1)
    };
    let dir = hub_dir(o);
    if op == "set" && !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        say.err(key!("cli-password-needs-terminal"), &[]);
        return ExitCode::from(2);
    }
    if private_hub_dir(&dir).is_err() {
        return fail(dir.display().to_string());
    }
    let store = match SqliteStore::open(&dir.join("hub.db")) {
        Ok(s) => s,
        Err(e) => return fail(e.to_string()),
    };
    let screen = ScreenPassword::new(&store);
    let now = server::now_ms();
    // Every session ends, and so does every passkey added from another machine without the password (ADR-0067).
    let sign_out_all = || {
        let _ = agora_hub::password::drop_unvouched_passkeys(&store);
        SignIn::new(&store, Arc::new(Lockout::new()))
            .revoke_all()
            .unwrap_or(0)
    };
    let audit = |action: &str| {
        let _ = agora_hub::audit::Audit::new(&store).record(now, "owner", action, "cli", "ok");
    };
    match op {
        "status" => {
            say.out(
                if screen.is_set() {
                    key!("cli-password-on")
                } else {
                    key!("cli-password-off")
                },
                &[],
            );
            ExitCode::SUCCESS
        }
        "clear" => {
            // A path from outside is on: the password stays until `agora remote off` (ADR-0067).
            if let Some(code) = crate::remotecmd::clear_refused(say, &dir) {
                return code;
            }
            match screen.clear() {
                Ok(true) => {
                    let revoked = sign_out_all();
                    audit("password_clear");
                    say.out(
                        key!("cli-password-cleared"),
                        &[("count", (revoked as i64).into())],
                    );
                    ExitCode::SUCCESS
                }
                Ok(false) => {
                    say.out(key!("cli-password-off"), &[]);
                    ExitCode::SUCCESS
                }
                Err(e) => fail(format!("{e:?}")),
            }
        }
        "set" => {
            // The prompt appears only once echo is off: an answer typed or pasted right after it is never shown.
            let read =
                |prompt: &str| agora_platform::read_secret_line_with_prompt(&say.text(prompt, &[]));
            // Ctrl-C: the terminal is already back to normal; stop as an interrupted program does.
            let stopped = |e: std::io::Error| {
                if e.kind() == std::io::ErrorKind::Interrupted {
                    ExitCode::from(130)
                } else {
                    say.err(key!("cli-password-needs-terminal"), &[]);
                    ExitCode::from(2)
                }
            };
            let first = match read(key!("cli-password-prompt")) {
                Ok(p) => p,
                Err(e) => return stopped(e),
            };
            match check_new(&first) {
                Ok(()) => {}
                Err(PasswordError::TooLong) => {
                    say.err(key!("cli-password-too-long"), &[]);
                    return ExitCode::from(2);
                }
                Err(_) => {
                    say.err(
                        key!("cli-password-too-short"),
                        &[("min", (agora_hub::password::MIN_CHARS as i64).into())],
                    );
                    return ExitCode::from(2);
                }
            }
            let again = match read(key!("cli-password-repeat")) {
                Ok(p) => p,
                Err(e) => return stopped(e),
            };
            if again != first {
                say.err(key!("cli-password-mismatch"), &[]);
                return ExitCode::from(1);
            }
            match screen.set(&first) {
                Ok(()) => {
                    let revoked = sign_out_all();
                    audit("password_set");
                    say.out(
                        key!("cli-password-set"),
                        &[("count", (revoked as i64).into())],
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => fail(format!("{e:?}")),
            }
        }
        _ => {
            say.err(key!("cli-usage"), &[]);
            ExitCode::from(2)
        }
    }
}

/// `agora hub user add|list|role|remove|login` (ADR-0049): the people of a team hub and what each may do.
pub fn hub_user(say: &Say, o: &Opts) -> ExitCode {
    use agora_hub::users::{Role, Users};
    let fail = |e: String| {
        say.err(key!("cli-hub-failed"), &[("reason", e.into())]);
        ExitCode::from(1)
    };
    let bad = || {
        say.err(key!("cli-bad-arguments"), &[]);
        ExitCode::from(2)
    };
    let dir = hub_dir(o);
    if private_hub_dir(&dir).is_err() {
        return fail(dir.display().to_string());
    }
    let store = match SqliteStore::open(&dir.join("hub.db")) {
        Ok(s) => s,
        Err(e) => return fail(e.to_string()),
    };
    let users = Users::new(&store);
    let code_for = |id: &str| {
        SignIn::new(&store, Arc::new(Lockout::new()))
            .create_code_for(Some(id), server::now_ms())
            .map_err(|_| "could not create a sign-in code".to_string())
    };
    let words: Vec<&str> = o.positional.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["add", name] => {
            let Some(role) = o.role.as_deref().map_or(Some(Role::Member), Role::parse) else {
                return bad();
            };
            let user = match users.add(name, role, server::now_ms()) {
                Ok(u) => u,
                Err(e) => return fail(e.to_string()),
            };
            match code_for(&user.id) {
                Ok(code) => {
                    say.out(
                        key!("cli-hub-user-added"),
                        &[
                            ("id", user.id.into()),
                            ("role", user.role.as_str().into()),
                            ("code", code.into()),
                        ],
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => fail(e),
            }
        }
        ["list"] => match users.list() {
            Ok(list) if list.is_empty() => {
                say.out(key!("cli-hub-user-none"), &[]);
                ExitCode::SUCCESS
            }
            Ok(list) => {
                for u in list {
                    say.out(
                        key!("cli-hub-user-line"),
                        &[
                            ("id", u.id.clone().into()),
                            ("name", u.name.into()),
                            ("role", u.role.as_str().into()),
                        ],
                    );
                    for project in users.grants_of(&u.id).unwrap_or_default() {
                        say.out(key!("cli-hub-user-project"), &[("project", project.into())]);
                    }
                }
                ExitCode::SUCCESS
            }
            Err(e) => fail(e.to_string()),
        },
        ["role", id, role] => {
            let Some(role) = Role::parse(role) else {
                return bad();
            };
            match users.set_role(id, role) {
                Ok(true) => ExitCode::SUCCESS,
                Ok(false) => {
                    say.err(key!("cli-hub-user-unknown"), &[("id", (*id).into())]);
                    ExitCode::from(1)
                }
                Err(e) => fail(e.to_string()),
            }
        }
        [verb @ ("grant" | "revoke"), id, project] => {
            let Some((node, project)) = project
                .split_once('/')
                .filter(|(n, p)| !n.is_empty() && !p.is_empty())
            else {
                return bad();
            };
            let done = if *verb == "grant" {
                users.grant(id, node, project)
            } else {
                users.revoke(id, node, project)
            };
            match done {
                Ok(true) => ExitCode::SUCCESS,
                Ok(false) => {
                    say.err(key!("cli-hub-user-unknown"), &[("id", (*id).into())]);
                    ExitCode::from(1)
                }
                Err(e) => fail(e.to_string()),
            }
        }
        ["remove", id] => match users.remove(id) {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => {
                say.err(key!("cli-hub-user-unknown"), &[("id", (*id).into())]);
                ExitCode::from(1)
            }
            Err(e) => fail(e.to_string()),
        },
        ["login", id] => match users.list() {
            Ok(list) if list.iter().any(|u| u.id == *id) => match code_for(id) {
                Ok(code) => {
                    say.out(key!("cli-hub-login"), &[("code", code.into())]);
                    ExitCode::SUCCESS
                }
                Err(e) => fail(e),
            },
            Ok(_) => {
                say.err(key!("cli-hub-user-unknown"), &[("id", (*id).into())]);
                ExitCode::from(1)
            }
            Err(e) => fail(e.to_string()),
        },
        _ => bad(),
    }
}

/// `agora hub metrics-token create NAME [--expires DAYS] | list | revoke NAME` (ADR-0076): tokens that can only read `GET /api/metrics`.
/// The token is shown once, by `create`; `list` never shows one. The hub looks a token up at every scrape, so `revoke` ends it at its
/// next use without a restart.
pub fn hub_metrics_token(say: &Say, o: &Opts) -> ExitCode {
    use agora_hub::metrics_token::{MetricsTokens, TokenError};
    let fail = |e: String| {
        say.err(key!("cli-hub-failed"), &[("reason", e.into())]);
        ExitCode::from(1)
    };
    let bad = || {
        say.err(key!("cli-bad-arguments"), &[]);
        ExitCode::from(2)
    };
    let words: Vec<&str> = o.positional.iter().map(String::as_str).collect();
    // Arguments first, so a wrong command line never creates the hub folder.
    if !matches!(words.as_slice(), ["create", _] | ["list"] | ["revoke", _])
        || (o.expires.is_some() && words.first() != Some(&"create"))
        || o.expires == Some(0)
    {
        return bad();
    }
    let dir = hub_dir(o);
    if private_hub_dir(&dir).is_err() {
        return fail(dir.display().to_string());
    }
    let store = match SqliteStore::open(&dir.join("hub.db")) {
        Ok(s) => s,
        Err(e) => return fail(e.to_string()),
    };
    let tokens = MetricsTokens::new(&store);
    let now = server::now_ms();
    match words.as_slice() {
        ["create", name] => {
            let expires = o
                .expires
                .map(|days| now.saturating_add(days.saturating_mul(86_400_000)));
            match tokens.create(name, now, expires) {
                Ok(token) => {
                    say.out(
                        key!("cli-metrics-token-created"),
                        &[("name", (*name).into()), ("token", token.into())],
                    );
                    ExitCode::SUCCESS
                }
                Err(TokenError::BadName) => {
                    say.err(key!("cli-metrics-token-bad-name"), &[]);
                    ExitCode::from(2)
                }
                Err(TokenError::Exists) => {
                    say.err(
                        key!("cli-metrics-token-exists"),
                        &[("name", (*name).into())],
                    );
                    ExitCode::from(1)
                }
                Err(TokenError::Store(e)) => fail(e),
            }
        }
        ["list"] => match tokens.list() {
            Ok(list) if list.is_empty() => {
                say.out(key!("cli-metrics-token-none"), &[]);
                ExitCode::SUCCESS
            }
            Ok(list) => {
                for t in list {
                    let used = if t.last_used_ms == 0 {
                        say.text(key!("cli-metrics-token-never-used"), &[])
                    } else {
                        utc(t.last_used_ms)
                    };
                    let expires = if t.expires_ms == 0 {
                        say.text(key!("cli-metrics-token-no-expiry"), &[])
                    } else {
                        utc(t.expires_ms)
                    };
                    let state = if t.revoked_ms > 0 {
                        say.text(key!("cli-metrics-token-state-revoked"), &[])
                    } else if t.expires_ms > 0 && t.expires_ms <= now {
                        say.text(key!("cli-metrics-token-state-expired"), &[])
                    } else {
                        String::new()
                    };
                    say.out(
                        key!("cli-metrics-token-line"),
                        &[
                            ("name", t.name.clone().into()),
                            ("created", utc(t.created_ms).into()),
                            ("used", used.into()),
                            ("expires", expires.into()),
                            ("state", state.into()),
                        ],
                    );
                }
                ExitCode::SUCCESS
            }
            Err(TokenError::Store(e)) => fail(e),
            Err(_) => bad(),
        },
        ["revoke", name] => match tokens.revoke(name, now) {
            Ok(true) => {
                say.out(
                    key!("cli-metrics-token-revoked"),
                    &[("name", (*name).into())],
                );
                ExitCode::SUCCESS
            }
            Ok(false) => {
                say.err(
                    key!("cli-metrics-token-unknown"),
                    &[("name", (*name).into())],
                );
                ExitCode::from(1)
            }
            Err(_) => fail("store".into()),
        },
        _ => bad(),
    }
}

/// A moment as people read it in a terminal: `2026-11-13 14:02 UTC` (the hub's and the node's clocks are both UTC-agnostic).
pub(crate) fn utc(ms: u64) -> String {
    let secs = ms / 1000;
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let (h, m) = ((secs % 86_400) / 3600, (secs % 3600) / 60);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(mo <= 2);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02} UTC")
}

/// `agora hub devices`: every registered device (ADR-0061) with its fingerprint, the person it belongs to, and when it was registered
/// and last used. These are candidates: a machine trusts one only when its owner says so there, or a trusted device endorses it.
pub fn hub_devices(say: &Say, o: &Opts) -> ExitCode {
    let dir = hub_dir(o);
    // `agora hub devices forget ID`: a revoked device leaves the list (its revocations stay, so machines keep refusing it).
    if let [verb, id] = o.positional.as_slice() {
        if verb != "forget" {
            say.err(key!("cli-bad-arguments"), &[]);
            return ExitCode::from(2);
        }
        let done = SqliteStore::open(&dir.join("hub.db"))
            .map_err(|e| e.to_string())
            .and_then(|store| {
                agora_hub::devices::Devices::new(&store)
                    .forget(id)
                    .map_err(|e| e.code.as_str().to_string())
            });
        return match done {
            Ok(true) => {
                say.out(
                    key!("cli-hub-device-forgotten"),
                    &[("id", id.clone().into())],
                );
                ExitCode::SUCCESS
            }
            Ok(false) => {
                say.err(
                    key!("cli-hub-device-not-forgotten"),
                    &[("id", id.clone().into())],
                );
                ExitCode::from(1)
            }
            Err(e) => {
                say.err(key!("cli-hub-failed"), &[("reason", e.into())]);
                ExitCode::from(1)
            }
        };
    }
    if !o.positional.is_empty() {
        say.err(key!("cli-bad-arguments"), &[]);
        return ExitCode::from(2);
    }
    let list = SqliteStore::open(&dir.join("hub.db"))
        .map_err(|e| e.to_string())
        .and_then(|store| {
            agora_hub::devices::Devices::new(&store)
                .list_all()
                .map_err(|e| e.code.as_str().to_string())
        });
    match list {
        Ok(list) if list.is_empty() => {
            say.out(key!("cli-hub-devices-none"), &[]);
            ExitCode::SUCCESS
        }
        Ok(list) => {
            for d in list {
                let state = if d.revoked_ms > 0 {
                    say.text(key!("cli-device-state-revoked"), &[])
                } else if d.approval_asked_ms > 0 {
                    say.text(key!("cli-device-state-waiting"), &[])
                } else {
                    String::new()
                };
                say.out(
                    key!("cli-hub-device-line"),
                    &[
                        ("name", d.name.clone().into()),
                        (
                            "fingerprint",
                            agora_hub::devices::fingerprint(&d.public_key).into(),
                        ),
                        ("id", d.device_id.clone().into()),
                        ("user", d.user_id.clone().into()),
                        ("created", utc(d.created_ms).into()),
                        ("used", utc(d.last_used_ms).into()),
                        ("state", state.into()),
                    ],
                );
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            say.err(key!("cli-hub-failed"), &[("reason", e.into())]);
            ExitCode::from(1)
        }
    }
}

/// `agora hub audit export [--since MS] [--who ID]`: the audit trail (ADR-0049) as JSON lines on standard output, oldest first.
pub fn hub_audit(say: &Say, o: &Opts) -> ExitCode {
    if o.positional.iter().map(String::as_str).collect::<Vec<_>>() != ["export"] {
        say.err(key!("cli-bad-arguments"), &[]);
        return ExitCode::from(2);
    }
    let dir = hub_dir(o);
    let rows = SqliteStore::open(&dir.join("hub.db"))
        .map_err(|e| e.to_string())
        .and_then(|store| {
            agora_hub::audit::Audit::new(&store)
                .list(o.since.unwrap_or(0), o.who.as_deref(), usize::MAX / 2)
                .map_err(|e| e.to_string())
        });
    match rows {
        Ok(rows) => {
            for e in rows {
                println!("{}", agora_hub::audit::json_line(&e));
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            say.err(key!("cli-hub-failed"), &[("reason", e.into())]);
            ExitCode::from(1)
        }
    }
}

/// The catalog is a refreshed copy in the hub dir if there is one, else the table bundled with the release; the owner's
/// `prices.json` overrides sit on top (ADR-0033). Returns the table, when it was refreshed (0 = bundled) and how many
/// override entries could not be read.
fn load_prices(dir: &Path) -> (agora_hub::usage::Catalog, u64, usize) {
    use agora_hub::usage::Catalog;
    let refreshed = std::fs::read_to_string(dir.join("prices-catalog.json"))
        .ok()
        .and_then(|t| Catalog::from_models_dev(&t).ok())
        .filter(|c| !c.is_empty());
    let at_ms = refreshed
        .as_ref()
        .and_then(|_| std::fs::metadata(dir.join("prices-catalog.json")).ok())
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(0));
    let mut catalog = refreshed.unwrap_or_else(|| (*Catalog::bundled()).clone());
    let skipped = std::fs::read_to_string(dir.join("prices.json"))
        .ok()
        .and_then(|t| catalog.load_overrides(&t).ok())
        .unwrap_or(0);
    (catalog, at_ms, skipped)
}

/// `agora hub prices update [--url URL]`: fetches the live models.dev catalog (only when asked) and keeps it in the hub dir.
pub fn hub_prices(say: &Say, o: &Opts) -> ExitCode {
    use agora_hub::deliver::{get, Url};
    if o.positional.first().map(String::as_str) != Some("update") {
        say.err(key!("cli-bad-arguments"), &[]);
        say.err(key!("cli-usage"), &[]);
        return ExitCode::from(2);
    }
    let dir = hub_dir(o);
    let fetched = (|| -> Result<usize, String> {
        let url = Url::parse(o.url.as_deref().unwrap_or("https://models.dev/api.json"))
            .ok_or("not an http:// or https:// address")?;
        let (status, body) = get(&url, 64 * 1024 * 1024).map_err(|e| e.to_string())?;
        if status != 200 {
            return Err(format!("status {status}"));
        }
        let text = String::from_utf8(body).map_err(|e| e.to_string())?;
        let count = agora_hub::usage::Catalog::from_models_dev(&text)
            .map_err(|e| e.to_string())?
            .len();
        if count == 0 {
            return Err("no prices in the answer".into());
        }
        private_hub_dir(&dir)?;
        std::fs::write(dir.join("prices-catalog.json"), text).map_err(|e| e.to_string())?;
        Ok(count)
    })();
    match fetched {
        Ok(n) => {
            say.out(
                key!("cli-hub-prices-updated"),
                &[("count", (n as i64).into())],
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            say.err(key!("cli-hub-prices-failed"), &[("reason", e.into())]);
            ExitCode::from(1)
        }
    }
}

/// Starts the SSH tunnels listed in `<hub dir>/ssh-machines` (ADR-0032) and keeps `<hub dir>/ssh-status` up to date.
fn start_tunnels(
    say: &Say,
    dir: &Path,
    listen: std::net::SocketAddr,
) -> Option<agora_hub::ssh::Dialer> {
    use agora_hub::ssh::{parse_machines, Backoff, Dialer, State};
    let file = dir.join("ssh-machines");
    let text = std::fs::read_to_string(&file).ok()?;
    let (machines, skipped) = parse_machines(&text);
    let name = file.display().to_string();
    if skipped > 0 {
        say.err(
            key!("cli-hub-ssh-skipped"),
            &[
                ("count", (skipped as i64).into()),
                ("file", name.clone().into()),
            ],
        );
    }
    if machines.is_empty() {
        return None;
    }
    say.out(
        key!("cli-hub-ssh-started"),
        &[
            ("count", (machines.len() as i64).into()),
            ("file", name.into()),
        ],
    );
    // The hub dials itself on loopback when it listens on every address.
    let ip = if listen.ip().is_unspecified() {
        std::net::Ipv4Addr::LOCALHOST.into()
    } else {
        listen.ip()
    };
    let hub = std::net::SocketAddr::new(ip, listen.port()).to_string();
    let status = dir.join("ssh-status");
    let states: std::sync::Mutex<std::collections::BTreeMap<String, String>> = Default::default();
    let program = std::env::var("AGORA_SSH").unwrap_or_else(|_| "ssh".into());
    Some(Dialer::start(
        machines,
        &hub,
        &program,
        Backoff::default(),
        Arc::new(move |name, st| {
            let text = match st {
                State::Connecting => "connecting".to_string(),
                State::Up => "up".to_string(),
                State::Down { reason, retry_ms } => {
                    format!("down, retry in {} s: {reason}", retry_ms / 1000)
                }
            };
            if let Ok(mut m) = states.lock() {
                m.insert(name.to_string(), text);
                let all: String = m.iter().map(|(k, v)| format!("{k} {v}\n")).collect();
                let _ = std::fs::write(&status, all);
            }
        }),
    ))
}

/// `agora hub ssh` lists the machines the hub keeps a tunnel to, with the last state it wrote; `agora hub ssh add NAME
/// DESTINATION [--port N]` adds one and says what to run on it (ADR-0032).
pub fn hub_ssh(say: &Say, o: &Opts) -> ExitCode {
    use agora_hub::ssh::{parse_machines, Machine, DEFAULT_PORT};
    let dir = hub_dir(o);
    let file = dir.join("ssh-machines");
    let name = file.display().to_string();
    let mut text = std::fs::read_to_string(&file).unwrap_or_default();
    match o.positional.as_slice() {
        [] => {
            let (machines, _) = parse_machines(&text);
            if machines.is_empty() {
                say.out(key!("cli-hub-ssh-none"), &[]);
                return ExitCode::SUCCESS;
            }
            let status = std::fs::read_to_string(dir.join("ssh-status")).unwrap_or_default();
            for m in machines {
                let state = status
                    .lines()
                    .find_map(|l| l.strip_prefix(&format!("{} ", m.name)))
                    .unwrap_or("not running");
                println!("{} {} {} {state}", m.name, m.destination, m.port);
            }
            ExitCode::SUCCESS
        }
        [verb, machine, destination] if verb == "add" => {
            let port = o.port.unwrap_or(DEFAULT_PORT);
            let Some(m) = Machine::new(machine, destination, port) else {
                say.err(key!("cli-hub-ssh-bad"), &[]);
                return ExitCode::from(2);
            };
            if parse_machines(&text).0.iter().any(|x| x.name == m.name) {
                say.err(key!("cli-hub-ssh-bad"), &[]);
                return ExitCode::from(2);
            }
            let result = (|| -> Result<String, String> {
                private_hub_dir(&dir)?;
                let _ = load_or_create_ca(&dir)?;
                let store = SqliteStore::open(&dir.join("hub.db")).map_err(|e| e.to_string())?;
                let code = Enrollment::new(&store)
                    .create_code(server::now_ms())
                    .map_err(|_| "could not create a code".to_string())?;
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&m.line());
                text.push('\n');
                std::fs::write(&file, &text).map_err(|e| e.to_string())?;
                Ok(code)
            })();
            match result {
                Ok(code) => {
                    say.out(
                        key!("cli-hub-ssh-added"),
                        &[
                            ("name", m.name.clone().into()),
                            ("file", name.into()),
                            ("ca", dir.join("ca.pem").display().to_string().into()),
                            ("port", i64::from(m.port).into()),
                            ("code", code.into()),
                        ],
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    say.err(key!("cli-hub-failed"), &[("reason", e.into())]);
                    ExitCode::from(1)
                }
            }
        }
        [verb, machine] if verb == "install" => hub_ssh_install(say, o, &dir, machine, &text),
        _ => {
            say.err(key!("cli-bad-arguments"), &[]);
            say.err(key!("cli-usage"), &[]);
            ExitCode::from(2)
        }
    }
}

/// Where release archives are published, unless `--release-url` or `<hub dir>/release-url` says otherwise.
const RELEASE_BASE: &str = "https://github.com/ersinkoc/Agora/releases/download";

/// `agora hub ssh install NAME` (ADR-0034): shows the plan, asks once, then runs check / binary / enroll / run on the machine.
fn hub_ssh_install(say: &Say, o: &Opts, dir: &Path, name: &str, machines: &str) -> ExitCode {
    use agora_hub::ssh::parse_machines;
    use agora_hub::ssh_install::{install, Finish, Options, SshRemote};
    let Some(machine) = parse_machines(machines)
        .0
        .into_iter()
        .find(|m| m.name == name)
    else {
        say.err(key!("cli-hub-ssh-unknown"), &[("name", name.into())]);
        return ExitCode::from(2);
    };
    let read = |f: &str| {
        std::fs::read_to_string(dir.join(f))
            .ok()
            .map(|t| t.trim().to_string())
    };
    let base = o
        .release_url
        .clone()
        .or_else(|| read("release-url"))
        .unwrap_or_else(|| RELEASE_BASE.to_string());
    let version = env!("CARGO_PKG_VERSION");
    say.out(
        key!("cli-hub-ssh-install-plan"),
        &[
            ("name", name.into()),
            ("destination", machine.destination.clone().into()),
            ("version", version.into()),
            ("base", base.clone().into()),
        ],
    );
    if !o.yes {
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        if !matches!(
            line.trim().to_lowercase().as_str(),
            "y" | "yes" | "e" | "evet"
        ) {
            say.out(key!("cli-hub-ssh-install-cancelled"), &[]);
            return ExitCode::from(1);
        }
    }
    let prepared = (|| -> Result<(String, String), String> {
        let ca = load_or_create_ca(dir)?;
        let store = SqliteStore::open(&dir.join("hub.db")).map_err(|e| e.to_string())?;
        let code = Enrollment::new(&store)
            .create_code(server::now_ms())
            .map_err(|_| "could not create a code".to_string())?;
        Ok((ca.cert_pem().to_string(), code))
    })();
    let (ca_pem, code) = match prepared {
        Ok(v) => v,
        Err(e) => {
            say.err(key!("cli-hub-failed"), &[("reason", e.into())]);
            return ExitCode::from(1);
        }
    };
    let opts = Options {
        version: version.to_string(),
        release_base: base,
        ca_pem,
        code,
        port: machine.port,
        release_key: read("release-key"),
        unsigned: o.unsigned,
    };
    let mut step = |s: &'static str| match s {
        "check" => say.out(key!("cli-hub-ssh-install-check"), &[]),
        "binary" => say.out(key!("cli-hub-ssh-install-binary"), &[]),
        "enroll" => say.out(key!("cli-hub-ssh-install-enroll"), &[]),
        _ => say.out(key!("cli-hub-ssh-install-run"), &[]),
    };
    match install(&SshRemote, &machine, &opts, &mut step) {
        Ok(Finish::Service(service)) => {
            say.out(
                key!("cli-hub-ssh-install-done"),
                &[("name", name.into()), ("service", service.into())],
            );
            ExitCode::SUCCESS
        }
        Ok(Finish::Manual(command)) => {
            say.out(
                key!("cli-hub-ssh-install-manual"),
                &[("name", name.into()), ("command", command.into())],
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            say.err(
                key!("cli-hub-ssh-install-failed"),
                &[
                    ("name", name.into()),
                    ("step", e.step.into()),
                    ("detail", e.detail.into()),
                    ("done", e.done.join(", ").into()),
                ],
            );
            ExitCode::from(1)
        }
    }
}

/// `agora hub notify [--ntfy URL | --webhook URL | --test]`: lists where notifications go, adds a target, or sends a test
/// message to every target (docs/design/notifications.md).
pub fn hub_notify(say: &Say, o: &Opts) -> ExitCode {
    use agora_hub::deliver::{parse_targets, request, send, Kind};
    let file = hub_dir(o).join("notify");
    let name = file.display().to_string();
    let fail = |e: String| {
        say.err(key!("cli-hub-failed"), &[("reason", e.into())]);
        ExitCode::from(1)
    };
    let mut text = std::fs::read_to_string(&file).unwrap_or_default();
    let add = match (&o.ntfy, &o.webhook) {
        (Some(u), None) => Some(("ntfy", u)),
        (None, Some(u)) => Some(("webhook", u)),
        (None, None) => None,
        _ => {
            say.err(key!("cli-bad-arguments"), &[]);
            return ExitCode::from(2);
        }
    };
    if let Some((kind, url)) = add {
        let line = format!("{kind} {url}");
        if parse_targets(&line).0.is_empty() {
            say.err(key!("cli-hub-notify-bad-url"), &[]);
            return ExitCode::from(2);
        }
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&line);
        text.push('\n');
        // The URL of a topic is the only secret ntfy has, so the file is private to the owner.
        if let Err(e) = private_hub_dir(&hub_dir(o))
            .map_err(std::io::Error::other)
            .and_then(|()| agora_platform::write_private(&file, text.as_bytes()))
        {
            return fail(e.to_string());
        }
        say.out(
            key!("cli-hub-notify-added"),
            &[("file", name.clone().into())],
        );
    }
    let (targets, _) = parse_targets(&text);
    if targets.is_empty() {
        say.out(key!("cli-hub-notify-none"), &[("file", name.into())]);
        return ExitCode::SUCCESS;
    }
    let message = say.text(key!("notify-test"), &[]);
    let mut all_ok = true;
    for t in &targets {
        let kind = if t.kind == Kind::Ntfy {
            "ntfy"
        } else {
            "webhook"
        };
        let target = format!("{kind} {}:{}", t.url.host, t.url.port);
        if !o.test {
            println!("{target}");
            continue;
        }
        let title = say.text(key!("notify-title"), &[]);
        let sent = send(
            &t.url,
            &request(t, &title, &message, agora_hub::notify::Priority::Normal),
        );
        match sent {
            Ok(status) if (200..300).contains(&status) => {
                say.out(key!("cli-hub-notify-sent"), &[("target", target.into())]);
            }
            Ok(status) => {
                all_ok = false;
                say.err(
                    key!("cli-hub-notify-refused"),
                    &[
                        ("target", target.into()),
                        ("status", i64::from(status).into()),
                    ],
                );
            }
            Err(e) => {
                all_ok = false;
                say.err(
                    key!("cli-hub-notify-unreachable"),
                    &[("target", target.into()), ("reason", e.to_string().into())],
                );
            }
        }
    }
    if all_ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// `agora hub code`: a one-time code for enrolling a node, and where the CA to pin is.
pub fn hub_code(say: &Say, o: &Opts) -> ExitCode {
    let dir = hub_dir(o);
    match new_enrollment_code(&dir) {
        Ok(code) => {
            say.out(
                key!("cli-hub-code"),
                &[
                    ("code", code.into()),
                    ("ca", dir.join("ca.pem").display().to_string().into()),
                ],
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            say.err(key!("cli-hub-failed"), &[("reason", e.into())]);
            ExitCode::from(1)
        }
    }
}

/// A one-time enrollment code from the hub's own store (valid 10 minutes), creating the hub's CA the first time.
pub(crate) fn new_enrollment_code(dir: &Path) -> Result<String, String> {
    private_hub_dir(dir)?;
    let _ = load_or_create_ca(dir)?;
    let store = SqliteStore::open(&dir.join("hub.db")).map_err(|e| e.to_string())?;
    Enrollment::new(&store)
        .create_code(server::now_ms())
        .map_err(|_| "could not create a code".to_string())
}

const BACKUP_PREFIX: &str = "hub-";
const DEFAULT_KEEP: usize = 7;

/// `agora hub backup [FILE] [--keep N]`: an online, consistent copy of the hub database. Without FILE it goes to
/// `<hub dir>/backups/hub-<unix ms>.db` and only the newest `--keep` (default 7) of those are kept.
pub fn hub_backup(say: &Say, o: &Opts) -> ExitCode {
    let dir = hub_dir(o);
    let result = (|| -> Result<PathBuf, String> {
        let store = SqliteStore::open(&dir.join("hub.db")).map_err(|e| e.to_string())?;
        if let Some(file) = o.positional.first() {
            store
                .backup_to(Path::new(file))
                .map_err(|e| e.to_string())?;
            return Ok(PathBuf::from(file));
        }
        let backups = dir.join("backups");
        std::fs::create_dir_all(&backups).map_err(|e| e.to_string())?;
        let dest = backups.join(format!("{BACKUP_PREFIX}{:013}.db", server::now_ms()));
        store.backup_to(&dest).map_err(|e| e.to_string())?;
        prune_backups(&backups, o.keep.unwrap_or(DEFAULT_KEEP));
        Ok(dest)
    })();
    match result {
        Ok(path) => {
            say.out(
                key!("cli-hub-backup"),
                &[("path", path.display().to_string().into())],
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            say.err(key!("cli-hub-failed"), &[("reason", e.into())]);
            ExitCode::from(1)
        }
    }
}

/// Deletes all but the newest `keep` automatic backups (the zero-padded timestamp makes names sort by age).
fn prune_backups(dir: &Path, keep: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<_> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with(BACKUP_PREFIX) && n.ends_with(".db"))
        .collect();
    names.sort();
    let drop = names.len().saturating_sub(keep.max(1));
    for n in names.into_iter().take(drop) {
        let _ = std::fs::remove_file(dir.join(n));
    }
}

/// `agora hub restore FILE`: replaces the hub database with a verified backup. Stop the hub first; the replaced
/// database stays next to it as `hub.db.before-restore`.
pub fn hub_restore(say: &Say, o: &Opts) -> ExitCode {
    let Some(file) = o.positional.first() else {
        say.err(key!("cli-bad-arguments"), &[]);
        say.err(key!("cli-usage"), &[]);
        return ExitCode::from(2);
    };
    let dest = hub_dir(o).join("hub.db");
    match agora_hub::store::restore(Path::new(file), &dest) {
        Ok(()) => {
            say.out(
                key!("cli-hub-restored"),
                &[("path", dest.display().to_string().into())],
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            say.err(key!("cli-hub-failed"), &[("reason", e.to_string().into())]);
            ExitCode::from(1)
        }
    }
}

/// `agora node enroll --hub URL --ca FILE --code CODE --id NAME`
pub fn node_enroll(say: &Say, o: &Opts) -> ExitCode {
    let (Some(url), Some(ca_file), Some(code), Some(id)) = (&o.hub, &o.ca, &o.code, &o.id) else {
        say.err(key!("cli-bad-arguments"), &[]);
        say.err(key!("cli-usage"), &[]);
        return ExitCode::from(2);
    };
    // ADR-0061: `--yes` covers exactly the devices whose fingerprints are given; checked before anything is enrolled.
    if o.yes != !o.fingerprints.is_empty() || (o.include_others && !o.yes) {
        say.err(key!("cli-bad-arguments"), &[]);
        say.err(key!("cli-usage"), &[]);
        return ExitCode::from(2);
    }
    // `--ca -` reads the certificate from standard input (how `hub ssh install` hands it over, ADR-0034).
    let ca_text = if ca_file.as_os_str() == "-" {
        let mut t = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut t).map(|_| t)
    } else {
        std::fs::read_to_string(ca_file)
    };
    let Ok(ca_pem) = ca_text else {
        say.err(
            key!("cli-hub-failed"),
            &[("reason", format!("{}", ca_file.display()).into())],
        );
        return ExitCode::from(1);
    };
    match enroll_and_save_with_devices(url, &ca_pem, code, id) {
        Ok((dir, devices)) => {
            say.out(
                key!("cli-enrolled"),
                &[
                    ("id", id.as_str().into()),
                    ("dir", dir.display().to_string().into()),
                ],
            );
            // ADR-0061: the owner is at this machine now; the devices they registered can be trusted here in one step. An old hub
            // sends none, and nothing is asked.
            if devices.is_empty() {
                ExitCode::SUCCESS
            } else {
                crate::trustcli::offer(say, o, &devices)
            }
        }
        Err(EnrollFailure::Refused(code)) => {
            say.err(key!("cli-enroll-refused"), &[("code", code.into())]);
            ExitCode::from(1)
        }
        Err(EnrollFailure::Other(reason)) => {
            say.err(key!("cli-hub-failed"), &[("reason", reason.into())]);
            ExitCode::from(1)
        }
    }
}

/// Why an enrollment ([`enroll_unless_linked`], [`enroll_and_save_with_devices`]) did not enroll this machine.
pub(crate) enum EnrollFailure {
    /// The hub refused, with this error code.
    Refused(String),
    Other(String),
}

/// Enrolls this machine with the hub at `url` (pinning `ca_pem`) under `id` with a one-time `code`, and keeps what the hub gave it in
/// the link folder, unless the folder already holds a link: then the hub is not asked and this is `Ok(false)`. The look, the enrollment
/// and the save hold the folder's guard together (audit F1030): two `agora` runs that start together enroll once, the second finding the
/// link of the first. Without it both enrolled under the same node id, the hub revoked the first certificate when it issued the second,
/// and the save that came last could keep the revoked one, which `node-id` then kept from ever being enrolled again.
pub(crate) fn enroll_unless_linked(
    url: &str,
    ca_pem: &str,
    code: &str,
    id: &str,
) -> Result<bool, EnrollFailure> {
    enroll_into(&link_dir(), LINK_GUARD_WAIT, url, id, false, &|csr| {
        enroll(url, ca_pem, code, id, csr)
    })
    .map(|devices| devices.is_some())
}

/// Enrolls this machine into the link folder `dir`, asking the hub with `enroll` (given the certificate request) and waiting at most
/// `wait` for the folder's guard, which is held from the look at `node-id` to the end of the save. Without `replace` a folder that holds
/// a link is left as it is and the hub is not asked (`Ok(None)`); with it the link is replaced. Returns the devices the hub registered.
///
/// Every enrollment holds the guard across its whole look, enrollment and save (`agora node enroll` too, Codex review of F1030): the hub
/// keeps one live certificate per node id and revokes the earlier ones when it issues a new one, so of two enrollments of one machine
/// the one that asked the hub last has the certificate that works, and it must also be the one that saves last. With the guard held
/// from before the hub is asked to after the save, the enrollments of this machine ask and save in the same order.
fn enroll_into(
    dir: &Path,
    wait: std::time::Duration,
    url: &str,
    id: &str,
    replace: bool,
    enroll: &dyn Fn(&str) -> Result<agora_protocol::v1::EnrollResponse, LinkError>,
) -> Result<Option<Vec<agora_protocol::v1::DeviceInfo>>, EnrollFailure> {
    let held = hold_link_within(dir, wait).map_err(|e| EnrollFailure::Other(e.to_string()))?;
    if !replace && held.dir.join("node-id").exists() {
        return Ok(None);
    }
    let (key_pem, csr) =
        new_device_request().map_err(|_| EnrollFailure::Other("key generation".into()))?;
    match enroll(&csr) {
        Ok(r) => {
            held.store(url, id, &r.ca_pem, &r.cert_pem, &key_pem)
                .map_err(|e| EnrollFailure::Other(e.to_string()))?;
            Ok(Some(r.devices))
        }
        Err(LinkError::Refused(code)) => Err(EnrollFailure::Refused(code)),
        Err(e) => Err(EnrollFailure::Other(format!("{e:?}"))),
    }
}

/// Enrolls this machine with the hub at `url` (pinning `ca_pem`) under `id` with a one-time `code`, keeps what the hub gave it in the
/// link folder (replacing any link there: `agora node enroll` is asked for by name), and returns the folder and the devices the hub
/// registered (ADR-0061: candidates the owner may trust here). The folder's guard is held from before the hub is asked to the end of the
/// save ([`enroll_into`]).
pub(crate) fn enroll_and_save_with_devices(
    url: &str,
    ca_pem: &str,
    code: &str,
    id: &str,
) -> Result<(PathBuf, Vec<agora_protocol::v1::DeviceInfo>), EnrollFailure> {
    let dir = link_dir();
    let devices = enroll_into(&dir, LINK_GUARD_WAIT, url, id, true, &|csr| {
        enroll(url, ca_pem, code, id, csr)
    })?;
    Ok((dir, devices.unwrap_or_default()))
}

/// The writes of [`HeldLink::store`] into `dir`, writing every file with `write` (the private-file helper: a temporary file made new and
/// renamed over the target, so a link planted at the target is replaced, never followed, and a stop never leaves half a file), waiting
/// at most `wait` for the folder's guard. For the tests of the order of the writes.
#[cfg(test)]
fn store_into(
    dir: &Path,
    wait: std::time::Duration,
    files: [&str; 5],
    write: &dyn Fn(&Path, &[u8]) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let held = hold_link_within(dir, wait)?;
    write_link(&held.dir, files, write)
}

/// The writes of a save into `dir`, whose guard the caller holds. The order keeps the folder consistent at every point: `node-id` is
/// what says this machine is joined, so it is removed **first** (a machine being joined again looks un-joined while the new material
/// goes in) and written **last** (only a complete set looks joined). A failure at any step leaves a folder without `node-id`. The guard
/// held across the whole save means two saves that overlap (two `agora` runs enrolling at once, `agora join --replace` beside them)
/// never mix their files: one save ends whole before the other starts (audit F1000).
fn write_link(
    dir: &Path,
    [url, id, ca_pem, cert_pem, key_pem]: [&str; 5],
    write: &dyn Fn(&Path, &[u8]) -> std::io::Result<()>,
) -> std::io::Result<()> {
    match std::fs::remove_file(dir.join("node-id")) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    write(&dir.join("key.pem"), key_pem.as_bytes())?;
    write(&dir.join("ca.pem"), ca_pem.as_bytes())?;
    write(&dir.join("cert.pem"), cert_pem.as_bytes())?;
    write(&dir.join("url"), url.as_bytes())?;
    write(&dir.join("node-id"), id.as_bytes())
}

/// The link folder, made if missing and resolved once, with its guard ([`link_guard`]) held until this is dropped: what is looked at in
/// it and saved into it meanwhile is never changed by another save, and a look and the save it decides are one step (audit F1030,
/// F1032). Every save of the link folder goes through one of these.
pub(crate) struct HeldLink {
    dir: PathBuf,
    _guard: std::fs::File,
}

impl HeldLink {
    /// The resolved folder.
    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    /// Keeps what enrollment (or pairing) gave this machine in the held folder: the hub's address, the node id, the CA, the certificate
    /// and the private key, every file readable by this user only ([`write_link`] for the order).
    pub(crate) fn store(
        &self,
        url: &str,
        id: &str,
        ca_pem: &str,
        cert_pem: &str,
        key_pem: &str,
    ) -> std::io::Result<()> {
        write_link(
            &self.dir,
            [url, id, ca_pem, cert_pem, key_pem],
            &agora_platform::write_private,
        )
    }
}

/// Holds the guard of the link folder `dir`, waiting as long as any holder may hold it ([`LINK_GUARD_WAIT`]); see [`HeldLink`].
pub(crate) fn hold_link(dir: &Path) -> std::io::Result<HeldLink> {
    hold_link_within(dir, LINK_GUARD_WAIT)
}

fn hold_link_within(dir: &Path, wait: std::time::Duration) -> std::io::Result<HeldLink> {
    std::fs::create_dir_all(dir)?;
    // Resolved once: the guard and every write name the same folder, also when `link` is a link that is pointed elsewhere meanwhile.
    let dir = std::fs::canonicalize(dir)?;
    let guard = link_guard(&dir, wait)?;
    Ok(HeldLink { dir, _guard: guard })
}

/// How long the link folder's guard is waited for: the longest a holder that is alive and working holds it (Codex review of F1030). An
/// enrollment holds it from its look at `node-id` to the end of its save, and `uplink_client::enroll` gives up on the hub after
/// [`SILENCE_DEADLINE`] for the connection and 15 s for the answer; the key, the request and the save take far less than the 10 s
/// margin. A save, a join's look and save and a read hold it for milliseconds. So a second `agora` that waits while the first enrolls is
/// never given up before the first ends; the wait runs out only when the holder is stopped or hangs (a holder that dies lets go at
/// once: the lock is the operating system's), and then the waiter fails with `TimedOut`, changing nothing.
const LINK_GUARD_WAIT: std::time::Duration =
    std::time::Duration::from_secs(SILENCE_DEADLINE.as_secs() + 15 + 10);

/// Holds `<dir>/lock` exclusively (an OS file lock, released when the returned file is dropped or the process ends), waiting at most
/// `wait` for another holder; `dir` is the resolved folder. The lock file is opened with `agora_platform::open_lock_private`: private
/// from its creation, never through a link, so another local user can neither hold it nor make it land on another file.
///
/// The error says which of two things happened. `TimedOut`: someone else held it (or kept the file from being opened) for the whole
/// wait. Any other: the guard cannot be had here at all, returned at once without waiting ([`lasting`]: no folder, a folder this user
/// cannot write in, a lock file that is a link or another user's); no save of this user can take it there either.
fn link_guard(dir: &Path, wait: std::time::Duration) -> std::io::Result<std::fs::File> {
    let lock = dir.join("lock");
    let end = std::time::Instant::now() + wait;
    let busy = || std::io::Error::from(std::io::ErrorKind::TimedOut);
    let pause = || std::thread::sleep(std::time::Duration::from_millis(20));
    loop {
        // On Windows a first save creates the file unshared for a moment (a sharing violation for anyone opening it then), so an open
        // that fails for a reason that can clear is tried again within the wait (Codex review of F1031: only those).
        let file = match agora_platform::open_lock_private(&lock) {
            Ok(f) => f,
            Err(e) if lasting(&e) => return Err(e),
            Err(_) if std::time::Instant::now() < end => {
                pause();
                continue;
            }
            Err(_) => return Err(busy()),
        };
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < end => pause(),
            Err(std::fs::TryLockError::WouldBlock) => return Err(busy()),
            Err(std::fs::TryLockError::Error(e)) => return Err(e),
        }
    }
}

/// Whether an error opening the lock file is one that waiting does not clear: the folder or the file is missing, this user may not
/// write there (a read-only folder or file system, another user's file), the file is a link (refused as `InvalidInput`), or the system
/// does not do it. Anything else (a Windows sharing violation, which has no kind of its own, a busy or interrupted call) can clear.
fn lasting(e: &std::io::Error) -> bool {
    use std::io::ErrorKind as K;
    matches!(
        e.kind(),
        K::NotFound
            | K::PermissionDenied
            | K::ReadOnlyFilesystem
            | K::InvalidInput
            | K::NotADirectory
            | K::IsADirectory
            | K::Unsupported
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod store_tests {
    use super::*;
    use std::cell::Cell;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("agora-store-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    const NEW: [&str; 5] = ["wss://h:1", "new", "NEWCA", "NEWCERT", "NEWKEY"];

    #[test]
    fn a_complete_store_has_every_file_and_node_id_last() {
        let dir = tmp("ok");
        let order = std::cell::RefCell::new(Vec::new());
        store_into(&dir, LINK_GUARD_WAIT, NEW, &|p, c| {
            order
                .borrow_mut()
                .push(p.file_name().unwrap().to_string_lossy().into_owned());
            agora_platform::write_private(p, c)
        })
        .unwrap();
        assert_eq!(order.borrow().last().unwrap(), "node-id");
        assert_eq!(std::fs::read_to_string(dir.join("node-id")).unwrap(), "new");
        assert_eq!(
            std::fs::read_to_string(dir.join("key.pem")).unwrap(),
            "NEWKEY"
        );
    }

    #[test]
    fn a_failure_between_any_two_writes_leaves_a_folder_that_does_not_look_joined() {
        for fail_at in 0..5 {
            let dir = tmp(&format!("fail{fail_at}"));
            // An older join is already there (the `--replace` case).
            std::fs::create_dir_all(&dir).unwrap();
            for (f, c) in [
                ("node-id", "old"),
                ("key.pem", "OLDKEY"),
                ("ca.pem", "OLDCA"),
                ("cert.pem", "OLDCERT"),
                ("url", "wss://old:1"),
            ] {
                std::fs::write(dir.join(f), c).unwrap();
            }
            let n = Cell::new(0);
            let r = store_into(&dir, LINK_GUARD_WAIT, NEW, &|p, c| {
                if n.replace(n.get() + 1) == fail_at {
                    return Err(std::io::Error::other("injected"));
                }
                agora_platform::write_private(p, c)
            });
            assert!(r.is_err());
            assert!(
                !dir.join("node-id").exists(),
                "a stop after {fail_at} writes left a machine that looks joined"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_link_planted_at_a_target_is_replaced_not_followed() {
        let dir = tmp("link");
        std::fs::create_dir_all(&dir).unwrap();
        let victim = dir.join("victim");
        std::fs::write(&victim, "keep").unwrap();
        for f in ["ca.pem", "cert.pem", "url", "node-id", "key.pem"] {
            std::os::unix::fs::symlink(&victim, dir.join(f)).unwrap();
        }
        store_into(&dir, LINK_GUARD_WAIT, NEW, &agora_platform::write_private).unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
        assert_eq!(
            std::fs::read_to_string(dir.join("ca.pem")).unwrap(),
            "NEWCA"
        );
        assert!(!std::fs::symlink_metadata(dir.join("url"))
            .unwrap()
            .file_type()
            .is_symlink());
    }

    /// A link folder that is itself a link (to another disk, say) is saved into as before; only the lock file is never reached
    /// through a link (review of the F1000 guard).
    #[test]
    fn a_link_folder_that_is_itself_a_link_is_still_saved_into() {
        let base = tmp("linked-folder");
        let real = base.join("real");
        std::fs::create_dir_all(&real).unwrap();
        let dir = base.join("link");
        // Only where the platform makes links without privileges.
        if agora_platform::symlink(&real, &dir).is_err() {
            return;
        }
        store_into(&dir, LINK_GUARD_WAIT, NEW, &agora_platform::write_private).unwrap();
        assert_eq!(
            std::fs::read_to_string(real.join("node-id")).unwrap(),
            "new"
        );
        let _ = std::fs::remove_dir_all(base);
    }

    /// Audit F1000: a second save that starts while the first is between its key and its certificate (two `agora` runs enrolling at
    /// once) used to run to the end and leave `node-id` over the key of one enrollment and the certificate of the other. The order is
    /// forced: the first save's writer starts the second one. It is kept out (told not to wait, `TimedOut`, nothing written) and
    /// the first ends whole; the guard is free again after a save that failed.
    #[test]
    fn overlapping_saves_never_mix_two_enrollments() {
        let dir = tmp("overlap");
        let other = ["wss://o:1", "other", "OTHERCA", "OTHERCERT", "OTHERKEY"];
        let second = std::cell::RefCell::new(None);
        store_into(&dir, LINK_GUARD_WAIT, NEW, &|p, c| {
            agora_platform::write_private(p, c)?;
            if p.file_name().is_some_and(|n| n == "key.pem") && second.borrow().is_none() {
                let r = store_into(
                    &dir,
                    std::time::Duration::ZERO,
                    other,
                    &agora_platform::write_private,
                );
                *second.borrow_mut() = Some(r.map_err(|e| e.kind()));
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(*second.borrow(), Some(Err(std::io::ErrorKind::TimedOut)));
        for (f, want) in [
            ("node-id", "new"),
            ("key.pem", "NEWKEY"),
            ("cert.pem", "NEWCERT"),
            ("ca.pem", "NEWCA"),
            ("url", "wss://h:1"),
        ] {
            assert_eq!(std::fs::read_to_string(dir.join(f)).unwrap(), want, "{f}");
        }

        let failed = store_into(&dir, LINK_GUARD_WAIT, other, &|p, c| {
            if p.file_name().is_some_and(|n| n == "cert.pem") {
                return Err(std::io::Error::other("injected"));
            }
            agora_platform::write_private(p, c)
        });
        assert!(failed.is_err());
        assert!(link_guard(&dir, std::time::Duration::ZERO).is_ok());
        store_into(&dir, LINK_GUARD_WAIT, other, &agora_platform::write_private).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("key.pem")).unwrap(),
            "OTHERKEY"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("cert.pem")).unwrap(),
            "OTHERCERT"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    fn answer(cert: &str) -> agora_protocol::v1::EnrollResponse {
        agora_protocol::v1::EnrollResponse {
            cert_pem: cert.into(),
            ca_pem: "CA".into(),
            ..Default::default()
        }
    }

    /// Audit F1030: two `agora` runs that start together on a machine with no link used to both pass `up`'s look at `node-id` and both
    /// enroll under the same node id; the hub revokes the first certificate when it issues the second, and the save that came last
    /// could keep the revoked one. The order is forced: a second run starts while the first is asking the hub. It is kept out by the
    /// guard (told not to wait, it fails and asks nothing); a run after the first finds the link and asks nothing either.
    #[test]
    fn two_runs_enrolling_at_once_ask_the_hub_once() {
        let dir = tmp("enroll-once");
        let second = std::cell::RefCell::new(None);
        let first = enroll_into(&dir, LINK_GUARD_WAIT, "wss://h:1", "me", false, &|_| {
            let asked = Cell::new(false);
            let r = enroll_into(
                &dir,
                std::time::Duration::ZERO,
                "wss://h:1",
                "me",
                false,
                &|_| {
                    asked.set(true);
                    Ok(answer("CERT-2"))
                },
            );
            *second.borrow_mut() = Some((matches!(r, Err(EnrollFailure::Other(_))), asked.get()));
            Ok(answer("CERT-1"))
        });
        assert!(matches!(first, Ok(Some(_))));
        assert_eq!(*second.borrow(), Some((true, false)));
        let later = enroll_into(&dir, LINK_GUARD_WAIT, "wss://h:1", "me", false, &|_| {
            panic!("a machine with a link asked the hub again")
        });
        assert!(matches!(later, Ok(None)));
        assert_eq!(std::fs::read_to_string(dir.join("node-id")).unwrap(), "me");
        assert_eq!(
            std::fs::read_to_string(dir.join("cert.pem")).unwrap(),
            "CERT-1"
        );
        // A refusal saves nothing and leaves the guard free for the next run.
        let refused = tmp("enroll-refused");
        let r = enroll_into(&refused, LINK_GUARD_WAIT, "wss://h:1", "me", false, &|_| {
            Err(LinkError::Refused("enroll.bad_code".into()))
        });
        assert!(matches!(r, Err(EnrollFailure::Refused(c)) if c == "enroll.bad_code"));
        assert!(!refused.join("node-id").exists());
        assert!(hold_link_within(&refused, std::time::Duration::ZERO).is_ok());
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(refused);
    }

    /// Audit F1031: the node's start read the five files one by one with no guard, so a save between two reads (`agora join
    /// --replace` while the node starts) gave it one link's `node-id` and another's key and certificate. The order is forced: right
    /// after `node-id` is read, a save tries to start. It is kept out until the read is over (told not to wait, it fails), and what
    /// was read is one link.
    #[test]
    fn a_read_of_the_link_never_sees_half_a_save() {
        let dir = tmp("read-whole");
        store_into(&dir, LINK_GUARD_WAIT, NEW, &agora_platform::write_private).unwrap();
        let other = ["wss://o:1", "other", "OTHERCA", "OTHERCERT", "OTHERKEY"];
        let save = std::cell::RefCell::new(None);
        let got = load_from(&dir, LINK_GUARD_WAIT, PathBuf::from("socket"), &|p| {
            let text = std::fs::read_to_string(p);
            if p.file_name().is_some_and(|n| n == "node-id") && save.borrow().is_none() {
                let r = hold_link_within(&dir, std::time::Duration::ZERO)
                    .and_then(|h| write_link(&h.dir, other, &agora_platform::write_private));
                *save.borrow_mut() = Some(r.map_err(|e| e.kind()));
            }
            text
        })
        .unwrap()
        .unwrap();
        assert_eq!(*save.borrow(), Some(Err(std::io::ErrorKind::TimedOut)));
        assert_eq!(
            (got.0.as_str(), got.1.url.as_str(), got.1.ca_pem.as_str()),
            ("new", "wss://h:1", "NEWCA")
        );
        assert_eq!(
            (got.1.cert_pem.as_str(), got.1.key_pem.as_str()),
            ("NEWCERT", "NEWKEY")
        );
        // The guard is free once the read is over.
        assert!(hold_link_within(&dir, std::time::Duration::ZERO).is_ok());
        // No folder: nothing, and reading makes none.
        let missing = tmp("read-missing");
        assert!(matches!(
            load_from(&missing, LINK_GUARD_WAIT, PathBuf::from("socket"), &|p| {
                std::fs::read_to_string(p)
            }),
            Ok(None)
        ));
        assert!(!missing.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Codex review of F1030: `agora node enroll` asked the hub outside the guard, so it could get its certificate while `agora`'s own
    /// enrollment held the guard and had not asked yet; that one then got a newer certificate for the same node id (the hub revoked the
    /// first), saved it, and the command by name, once it had the guard, saved the revoked one over it. The order is forced: the command
    /// by name starts while the other enrollment holds the guard and before it asks, and the "hub" numbers what it issues. The command
    /// by name asks only after the other saved, so the folder ends with the certificate issued last, the one the hub keeps.
    #[test]
    fn an_enrollment_by_name_asks_the_hub_only_after_one_under_way_has_saved() {
        let dir = tmp("enroll-by-name");
        let issued = std::sync::Mutex::new(Vec::<&str>::new());
        let hub = |who: &'static str| -> Result<agora_protocol::v1::EnrollResponse, LinkError> {
            let mut v = issued.lock().unwrap();
            v.push(who);
            Ok(answer(&format!("CERT-{}", v.len())))
        };
        let by_name = std::sync::Mutex::new(None);
        let (d, h, b) = (&dir, &hub, &by_name);
        std::thread::scope(|s| {
            let first = enroll_into(d, LINK_GUARD_WAIT, "wss://h:1", "me", false, &|_| {
                s.spawn(move || {
                    let r = enroll_into(d, LINK_GUARD_WAIT, "wss://h:1", "me", true, &|_| {
                        h("by name")
                    });
                    *b.lock().unwrap() = Some(r.is_ok());
                });
                // Time for the command by name to ask the hub, were it not kept out.
                std::thread::sleep(std::time::Duration::from_millis(300));
                h("agora")
            });
            assert!(matches!(first, Ok(Some(_))));
        });
        assert_eq!(*by_name.lock().unwrap(), Some(true));
        assert_eq!(*issued.lock().unwrap(), ["agora", "by name"]);
        assert_eq!(
            std::fs::read_to_string(dir.join("cert.pem")).unwrap(),
            "CERT-2"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Codex re-review of F1031: a link folder that is there but cannot be resolved or entered is an error, not "no link" (the node would
    /// start unlinked). Here: a path through a file, a link to a missing folder, and a folder this user may not enter.
    #[cfg(unix)]
    #[test]
    fn a_folder_that_cannot_be_resolved_is_an_error_not_no_link() {
        let base = tmp("read-unresolvable");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("file"), "x").unwrap();
        let got = load_from(
            &base.join("file").join("link"),
            LINK_GUARD_WAIT,
            PathBuf::from("socket"),
            &|p| std::fs::read_to_string(p),
        );
        assert!(got.is_err_and(|e| e.kind() != std::io::ErrorKind::TimedOut));
        // A link folder that is a link to something missing (a volume not mounted): an error too.
        let dangling = base.join("dangling");
        std::os::unix::fs::symlink(base.join("unmounted"), &dangling).unwrap();
        let got = load_from(&dangling, LINK_GUARD_WAIT, PathBuf::from("socket"), &|p| {
            std::fs::read_to_string(p)
        });
        assert!(got.is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound));
        // A link folder this user may not enter: an error, not "no link" (unless the system lets this user in anyway, as for root).
        let shut = base.join("shut");
        store_into(&shut, LINK_GUARD_WAIT, NEW, &agora_platform::write_private).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shut, std::fs::Permissions::from_mode(0o000)).unwrap();
        let entered = std::fs::read_to_string(shut.join("node-id")).is_ok();
        let got = load_from(&shut, LINK_GUARD_WAIT, PathBuf::from("socket"), &|p| {
            std::fs::read_to_string(p)
        });
        std::fs::set_permissions(&shut, std::fs::Permissions::from_mode(0o700)).unwrap();
        if !entered {
            assert!(got.is_err_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied));
        }
        let _ = std::fs::remove_dir_all(base);
    }

    /// Codex review of F1031: a read that waited out the guard used to read anyway, so a save held up past the wait could again be
    /// read half. Now nothing is read and the read says `TimedOut`.
    #[test]
    fn a_read_never_goes_past_a_guard_another_holds() {
        let dir = tmp("read-busy");
        store_into(&dir, LINK_GUARD_WAIT, NEW, &agora_platform::write_private).unwrap();
        let held = hold_link_within(&dir, LINK_GUARD_WAIT).unwrap();
        let read_any = Cell::new(false);
        let got = load_from(
            &dir,
            std::time::Duration::ZERO,
            PathBuf::from("socket"),
            &|p| {
                read_any.set(true);
                std::fs::read_to_string(p)
            },
        );
        assert_eq!(
            got.map(|l| l.is_some()).map_err(|e| e.kind()),
            Err(std::io::ErrorKind::TimedOut)
        );
        assert!(!read_any.get());
        drop(held);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Codex review of F1031: an error opening the lock file that waiting cannot clear used to be tried again for the whole wait, so a
    /// link folder where the guard cannot be had added that wait to every start of the node. Such a folder (here: `lock` is a folder,
    /// which no save can lock either) is read at once, without the guard. Errors that can clear are still waited out.
    #[test]
    fn a_folder_where_the_guard_cannot_be_had_is_read_at_once() {
        let dir = tmp("read-no-guard");
        store_into(&dir, LINK_GUARD_WAIT, NEW, &agora_platform::write_private).unwrap();
        std::fs::remove_file(dir.join("lock")).unwrap();
        std::fs::create_dir(dir.join("lock")).unwrap();
        let start = std::time::Instant::now();
        let got = load_from(&dir, LINK_GUARD_WAIT, PathBuf::from("socket"), &|p| {
            std::fs::read_to_string(p)
        })
        .unwrap()
        .unwrap();
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        assert_eq!((got.0.as_str(), got.1.key_pem.as_str()), ("new", "NEWKEY"));
        for k in [
            std::io::ErrorKind::NotFound,
            std::io::ErrorKind::PermissionDenied,
            std::io::ErrorKind::ReadOnlyFilesystem,
            std::io::ErrorKind::InvalidInput,
        ] {
            assert!(lasting(&std::io::Error::from(k)), "{k:?}");
        }
        for k in [
            std::io::ErrorKind::WouldBlock,
            std::io::ErrorKind::Interrupted,
            std::io::ErrorKind::Other,
        ] {
            assert!(!lasting(&std::io::Error::from(k)), "{k:?}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Codex review of F1031: the guard was taken in the resolved folder but the files were read by the path given, so a link folder
    /// that is itself a link, pointed elsewhere between the two, was read in another folder than the one held. Every read now goes
    /// through the resolved folder.
    #[test]
    fn a_read_goes_through_the_folder_whose_guard_it_holds() {
        let base = tmp("read-resolved");
        let real = base.join("real");
        store_into(&real, LINK_GUARD_WAIT, NEW, &agora_platform::write_private).unwrap();
        let dir = base.join("link");
        // Only where the platform makes links without privileges.
        if agora_platform::symlink(&real, &dir).is_err() {
            return;
        }
        let resolved = std::fs::canonicalize(&real).unwrap();
        let outside = Cell::new(false);
        let got = load_from(&dir, LINK_GUARD_WAIT, PathBuf::from("socket"), &|p| {
            outside.set(outside.get() || !p.starts_with(&resolved));
            std::fs::read_to_string(p)
        })
        .unwrap();
        assert!(got.is_some());
        assert!(!outside.get());
        let _ = std::fs::remove_dir_all(base);
    }
}

/// What enrollment left on this machine, if anything: the node id and how to dial the hub. `TimedOut` when another command held the link
/// folder for longer than any of them holds it working ([`LINK_GUARD_WAIT`]): the folder is then not read at all (it may be half
/// saved), and the caller fails rather than start with no link or half of one. Any other error: the folder is there but could not be
/// resolved, and the caller fails as well.
pub fn load_link(socket: PathBuf) -> std::io::Result<Option<(String, HubLink)>> {
    load_from(&link_dir(), LINK_GUARD_WAIT, socket, &|p| {
        std::fs::read_to_string(p)
    })
}

/// [`load_link`] from `dir`, waiting at most `wait` for its guard and reading each file with `read`. The five files are read under the
/// folder's guard (audit F1031): a save that runs meanwhile (`agora join --replace`, an `agora` enrolling while the node starts) is seen
/// whole or not at all, never one link's `node-id` with another's key and certificate. The folder is resolved once and the files are
/// read through that same folder the guard is held in (Codex review of F1031).
///
/// No folder is no link. Where the guard cannot be had at all (a folder this user cannot write in, a read-only one say) the files are read without it,
/// at once: no save of this user can take the guard there either, so none runs to be seen half. Where another holder kept it for the
/// whole wait, nothing is read: that is `TimedOut` (Codex review of F1031: a read past a busy guard could again be half a save).
fn load_from(
    dir: &Path,
    wait: std::time::Duration,
    socket: PathBuf,
    read: &dyn Fn(&Path) -> std::io::Result<String>,
) -> std::io::Result<Option<(String, HubLink)>> {
    let resolved = resolve_link_folder(dir)?;
    let Some(dir) = resolved else {
        return Ok(None);
    };
    let _guard = match link_guard(&dir, wait) {
        Ok(g) => Some(g),
        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => return Err(e),
        Err(_) => None,
    };
    // A stop in the middle of a certificate renewal (ADR-0085) left the certificate and key as two halves: finish it, or go back to the
    // pair that was in use, before anything reads them. Only under the guard: without it a renewal may be running.
    if _guard.is_some() {
        agora_node::link_renewal::recover(&dir, &agora_node::link_renewal::Real);
    }
    link_files(&dir, socket, read)
}

/// The link folder `dir`, resolved, or `None` when there is none. Never the path given instead: a folder made after the look could be
/// mid-save (Codex re-review of F1031). A folder that is there but cannot be resolved is an error, not "no link", so the node does not
/// start as an unlinked one: a link to something missing (a volume that is not mounted) too. Whatever appeared between the two looks
/// (a folder an enrollment just made, a link whose target just came back) is resolved again, a few times, so the read waits for that
/// enrollment's guard; what still cannot be resolved then is the error.
fn resolve_link_folder(dir: &Path) -> std::io::Result<Option<PathBuf>> {
    let mut tries = 0;
    loop {
        match std::fs::canonicalize(dir) {
            Ok(resolved) => return Ok(Some(resolved)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                match std::fs::symlink_metadata(dir) {
                    Err(_) => return Ok(None),
                    Ok(_) if tries >= 3 => return Err(e),
                    Ok(_) => tries += 1,
                }
            }
            Err(e) => return Err(e),
        }
    }
}

/// The five files of the link folder `dir`, if `node-id` and the others are all there. A file that is not there is no link (a save
/// that stopped part way leaves no `node-id`, audit F1000); any other failure to read one (a folder this user may not enter) is an
/// error, not "no link" (Codex re-review of F1031).
fn link_files(
    dir: &Path,
    socket: PathBuf,
    read: &dyn Fn(&Path) -> std::io::Result<String>,
) -> std::io::Result<Option<(String, HubLink)>> {
    let read = |n: &str| match read(&dir.join(n)) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    };
    let Some(id) = read("node-id")? else {
        return Ok(None);
    };
    let (Some(url), Some(ca_pem), Some(cert_pem), Some(key_pem)) = (
        read("url")?,
        read("ca.pem")?,
        read("cert.pem")?,
        read("key.pem")?,
    ) else {
        return Ok(None);
    };
    Ok(Some((
        id.trim().to_string(),
        HubLink {
            url: url.trim().to_string(),
            ca_pem,
            cert_pem,
            key_pem,
            commands: Some(socket),
            mesh_out: None,
            grants: None,
            mesh: None,
            devices: None,
            state_dir: None,
            certs: None,
        },
    )))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn the_hub_agrees_on_every_capability_a_node_offers() {
        // Including the ones no client can send (`board.join` goes only from the hub to nodes), which the real hub-and-node test
        // in `tests/hub_cli.rs` cannot reach.
        for c in agora_node::uplink_client::CAPABILITIES {
            assert!(HUB_CAPABILITIES.contains(c), "the hub does not support {c}");
        }
    }

    #[test]
    fn moments_are_written_as_utc_dates() {
        assert_eq!(utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(utc(1_700_000_000_000), "2023-11-14 22:13 UTC");
        assert_eq!(utc(951_782_400_000), "2000-02-29 00:00 UTC");
    }

    #[test]
    fn only_the_newest_automatic_backups_are_kept() {
        let dir = std::env::temp_dir().join(format!("agora-prune-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for i in 1..=5u32 {
            std::fs::write(dir.join(format!("hub-{i:013}.db")), "x").unwrap();
        }
        std::fs::write(dir.join("notes.txt"), "mine").unwrap();
        prune_backups(&dir, 2);
        let mut left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        left.sort();
        assert_eq!(
            left,
            ["hub-0000000000004.db", "hub-0000000000005.db", "notes.txt"]
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod listen_file_tests {
    use super::local_dial_addr;

    #[test]
    fn a_client_on_the_hubs_machine_dials_the_port_the_hub_bound() {
        assert_eq!(
            local_dial_addr("127.0.0.1:7445").as_deref(),
            Some("127.0.0.1:7445")
        );
        assert_eq!(
            local_dial_addr("0.0.0.0:7446").as_deref(),
            Some("127.0.0.1:7446")
        );
        assert_eq!(local_dial_addr("[::]:7447").as_deref(), Some("[::1]:7447"));
        assert_eq!(
            local_dial_addr("192.168.1.5:7448").as_deref(),
            Some("192.168.1.5:7448")
        );
        assert_eq!(local_dial_addr("not an address"), None);
        assert_eq!(local_dial_addr(""), None);
    }
}

/// ADR-0073: a machine not seen for this long is shown as old (the window and the TUI use the same threshold).
pub(crate) const OLD_AFTER_MS: u64 = 24 * 60 * 60 * 1000;

/// A machine's state as people see it: `online`, `offline`, or `old` (offline and not seen for [`OLD_AFTER_MS`]).
pub(crate) fn machine_state(online: bool, last_seen_ms: u64, now_ms: u64) -> &'static str {
    if online {
        "online"
    } else if now_ms.saturating_sub(last_seen_ms) >= OLD_AFTER_MS {
        "old"
    } else {
        "offline"
    }
}

/// Says so under a machine's line when its certificate ends within 30 days or has ended. The hub does not store a certificate's end:
/// it is when the machine's current certificate was issued plus the 90 days every certificate lasts. Nothing renews it yet.
fn certificate_note(say: &Say, dir: &Path, id: &str, now_ms: u64) {
    use agora_hub::ca::{expiry, Expiry, DEVICE_CERT_DAYS};
    let Ok(store) = SqliteStore::open(&dir.join("hub.db")) else {
        return;
    };
    let Ok(issued) = agora_hub::enroll::Enrollment::new(&store).issued_ms_by_node() else {
        return;
    };
    let Some(&at) = issued.get(id) else {
        return;
    };
    let end = i64::try_from(at / 1000).unwrap_or(0) + DEVICE_CERT_DAYS * 86_400;
    let date = utc(u64::try_from(end).unwrap_or(0) * 1000);
    let date = date.split(' ').next().unwrap_or("").to_string();
    match expiry(end, i64::try_from(now_ms / 1000).unwrap_or(0)) {
        Expiry::Fine(_) => {}
        Expiry::Soon(days) => say.out(
            key!("cli-machine-cert-soon"),
            &[
                ("id", id.to_string().into()),
                ("date", date.into()),
                ("days", days.into()),
            ],
        ),
        Expiry::Expired(_) => say.out(
            key!("cli-machine-cert-expired"),
            &[("id", id.to_string().into()), ("date", date.into())],
        ),
    }
}

/// `agora hub machines`: every machine the hub shows, with its state and when it was last seen. `forget NAME` and `forget --old`
/// (every old machine) ask the running hub to forget them (ADR-0073), after the owner's yes on this terminal (`--yes` for a
/// script); `--refuse-return` also revokes their certificates. The list is read from the hub's file; forgetting goes through the
/// running hub, signed in from its folder, so its view and every window hear of it.
pub fn hub_machines(say: &Say, o: &Opts) -> ExitCode {
    let dir = hub_dir(o);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    let view = match SqliteStore::open(&dir.join("hub.db"))
        .and_then(|s| agora_hub::projection::View::rebuild(&s))
    {
        Ok((view, _)) => view,
        Err(e) => {
            say.err(key!("cli-hub-failed"), &[("reason", e.to_string().into())]);
            return ExitCode::from(1);
        }
    };
    let shown = view.shown();
    let machines: Vec<(&String, &agora_hub::projection::NodeView, &str)> = shown
        .nodes
        .iter()
        .map(|(id, n)| (id, n, machine_state(n.online, n.last_event_ms, now)))
        .collect();
    let picked: Vec<&String> = match o.positional.as_slice() {
        [] if !o.old => {
            if machines.is_empty() {
                say.out(key!("cli-machines-none"), &[]);
            }
            for (id, n, state) in &machines {
                say.out(
                    key!("cli-machine-line"),
                    &[
                        ("id", (*id).clone().into()),
                        ("state", (*state).into()),
                        ("seen", utc(n.last_event_ms).into()),
                    ],
                );
                certificate_note(say, &dir, id, now);
            }
            return ExitCode::SUCCESS;
        }
        [verb] if verb == "forget" && o.old => machines
            .iter()
            .filter(|(_, _, s)| *s == "old")
            .map(|(id, _, _)| *id)
            .collect(),
        [verb, name] if verb == "forget" && !o.old => {
            match machines.iter().find(|(id, _, _)| *id == name) {
                Some((id, _, _)) => vec![*id],
                None => {
                    say.err(key!("cli-machine-unknown"), &[("id", name.clone().into())]);
                    return ExitCode::from(1);
                }
            }
        }
        _ => {
            say.err(key!("cli-bad-arguments"), &[]);
            return ExitCode::from(2);
        }
    };
    if picked.is_empty() {
        say.out(key!("cli-machines-none-old"), &[]);
        return ExitCode::SUCCESS;
    }
    for (id, n, state) in machines.iter().filter(|(id, _, _)| picked.contains(id)) {
        say.out(
            key!("cli-machine-line"),
            &[
                ("id", (*id).clone().into()),
                ("state", (*state).into()),
                ("seen", utc(n.last_event_ms).into()),
            ],
        );
    }
    if !o.yes {
        use std::io::{BufRead, IsTerminal, Write};
        if !std::io::stdin().is_terminal() {
            say.out(key!("cli-machines-not-asked"), &[]);
            return ExitCode::SUCCESS;
        }
        let question = if o.refuse_return {
            key!("cli-machines-question-refuse")
        } else {
            key!("cli-machines-question")
        };
        let mut out = std::io::stdout();
        let _ = write!(out, "{} ", say.text(question, &[]));
        let _ = out.flush();
        let mut answer = String::new();
        let _ = std::io::stdin().lock().read_line(&mut answer);
        if !crate::trustcli::is_yes(&answer, &say.text(key!("cli-devices-yes"), &[])) {
            say.out(key!("cli-machines-declined"), &[]);
            return ExitCode::SUCCESS;
        }
    }
    let session = (|| {
        let hub = agora_tui::client::Hub {
            addr: own_hub_addr(&dir).unwrap_or_else(|| DEFAULT_LISTEN.to_string()),
            ca_pem: std::fs::read_to_string(dir.join("ca.pem")).map_err(|e| e.to_string())?,
        };
        let code = new_login_code(&dir)?;
        let token = agora_tui::client::sign_in(&hub, &code).map_err(|e| format!("{e:?}"))?;
        agora_tui::client::Session::connect(&hub, &token).map_err(|e| format!("{e:?}"))
    })();
    let mut session = match session {
        Ok(s) => s,
        Err(reason) => {
            say.err(
                key!("cli-machines-hub-unreachable"),
                &[("reason", reason.into())],
            );
            return ExitCode::from(1);
        }
    };
    let mut code = ExitCode::SUCCESS;
    for id in picked {
        let forget = agora_protocol::v1::command_request::Command::NodeForget(
            agora_protocol::v1::NodeForget {
                node_id: id.clone(),
                refuse_return: o.refuse_return,
            },
        );
        match session.command("", forget, std::time::Duration::from_secs(15)) {
            Ok(()) => say.out(key!("cli-machine-forgotten"), &[("id", id.clone().into())]),
            Err(e) => {
                say.err(
                    key!("cli-machine-not-forgotten"),
                    &[("id", id.clone().into()), ("code", e.into())],
                );
                code = ExitCode::from(1);
            }
        }
    }
    code
}

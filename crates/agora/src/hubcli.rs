//! `agora hub run|code` and `agora node enroll`: the commands that connect machines to a hub. The hub keeps its state in
//! `<hub dir>` (default `$AGORA_HOME/hub`): the database, its certificate authority (key readable only by the user)
//! and the client token. A node keeps what enrollment gave it in `<AGORA_HOME>/link`.

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
use agora_node::uplink_client::{enroll, HubLink, LinkError};
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
    // ADR-0061: the hub carries endorsements and revocations to nodes that verify them (`device.trust`), and answers a node's
    // `DeviceList` on a connection of its own (`device.list`).
    "device.trust",
    "device.list",
];

pub(crate) fn hub_dir(o: &Opts) -> PathBuf {
    o.hub_dir.clone().unwrap_or_else(|| home().join("hub"))
}

pub fn link_dir() -> PathBuf {
    home().join("link")
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
    let run = || -> Result<(), String> {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let ca = load_or_create_ca(&dir)?;
        let names = if o.names.is_empty() {
            vec!["localhost".to_string()]
        } else {
            o.names.clone()
        };
        let (cert, key) = ca.issue_server(&names).map_err(|e| e.to_string())?;
        let acceptor =
            tls::acceptor_with(&cert, &key, ca.cert_pem(), true).map_err(|e| e.to_string())?;
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
        let sessions = Arc::new(HubSessions::new(Arc::clone(&hub)));
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
            say.out(
                key!("cli-hub-listening"),
                &[
                    ("addr", addr.to_string().into()),
                    ("dir", dir.display().to_string().into()),
                ],
            );
            let _tunnels = start_tunnels(say, &dir, addr);
            if has_web {
                say.out(
                    key!("cli-hub-web"),
                    &[("addr", addr.to_string().into()), ("code", web_code.into())],
                );
            }
            server::run(listener, state, Some(acceptor)).await;
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

/// A fresh one-time sign-in code from the hub's own store (valid 10 minutes).
pub fn new_login_code(dir: &Path) -> Result<String, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
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
    if std::fs::create_dir_all(&dir).is_err() {
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
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
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
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
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
        if let Err(e) = std::fs::create_dir_all(hub_dir(o))
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
    let result = (|| -> Result<String, String> {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let _ = load_or_create_ca(&dir)?;
        let store = SqliteStore::open(&dir.join("hub.db")).map_err(|e| e.to_string())?;
        Enrollment::new(&store)
            .create_code(server::now_ms())
            .map_err(|_| "could not create a code".to_string())
    })();
    match result {
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
    let Ok((key_pem, csr)) = new_device_request() else {
        say.err(
            key!("cli-hub-failed"),
            &[("reason", "key generation".into())],
        );
        return ExitCode::from(1);
    };
    match enroll(url, &ca_pem, code, id, &csr) {
        Ok(r) => {
            let dir = link_dir();
            let write = || -> std::io::Result<()> {
                std::fs::create_dir_all(&dir)?;
                std::fs::write(dir.join("url"), url)?;
                std::fs::write(dir.join("node-id"), id)?;
                std::fs::write(dir.join("ca.pem"), &r.ca_pem)?;
                std::fs::write(dir.join("cert.pem"), &r.cert_pem)?;
                agora_platform::write_private(&dir.join("key.pem"), key_pem.as_bytes())
            };
            if let Err(e) = write() {
                say.err(key!("cli-hub-failed"), &[("reason", e.to_string().into())]);
                return ExitCode::from(1);
            }
            say.out(
                key!("cli-enrolled"),
                &[
                    ("id", id.as_str().into()),
                    ("dir", dir.display().to_string().into()),
                ],
            );
            ExitCode::SUCCESS
        }
        Err(LinkError::Refused(code)) => {
            say.err(key!("cli-enroll-refused"), &[("code", code.into())]);
            ExitCode::from(1)
        }
        Err(e) => {
            say.err(
                key!("cli-hub-failed"),
                &[("reason", format!("{e:?}").into())],
            );
            ExitCode::from(1)
        }
    }
}

/// The address of the hub's web app, from the hub address a node was enrolled with (`wss://host:port` -> `https://host:port/`).
fn panel_address(hub: &str) -> Option<String> {
    let rest = hub.trim().strip_prefix("wss://")?;
    let host = rest.split('/').next().filter(|h| !h.is_empty())?;
    Some(format!("https://{host}/"))
}

/// `agora panel`: where to open the hub's panel from a machine that runs a node (ADR-0056). The person still signs in with a code
/// from `agora hub login` on the hub; nothing here holds a session or opens a port.
pub fn panel(say: &Say) -> ExitCode {
    let dir = link_dir();
    let address = std::fs::read_to_string(dir.join("url"))
        .ok()
        .and_then(|u| panel_address(&u));
    match address {
        Some(url) => {
            say.out(
                key!("cli-panel"),
                &[
                    ("url", url.into()),
                    ("ca", dir.join("ca.pem").display().to_string().into()),
                ],
            );
            ExitCode::SUCCESS
        }
        None => {
            say.err(key!("cli-panel-none"), &[]);
            ExitCode::from(1)
        }
    }
}

/// What enrollment left on this machine, if anything: the node id and how to dial the hub.
pub fn load_link(socket: PathBuf) -> Option<(String, HubLink)> {
    let dir = link_dir();
    let read = |n: &str| std::fs::read_to_string(dir.join(n)).ok();
    let id = read("node-id")?.trim().to_string();
    Some((
        id,
        HubLink {
            url: read("url")?.trim().to_string(),
            ca_pem: read("ca.pem")?,
            cert_pem: read("cert.pem")?,
            key_pem: read("key.pem")?,
            commands: Some(socket),
            mesh_out: None,
            grants: None,
            mesh: None,
        },
    ))
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
    fn the_panel_address_comes_from_the_hub_address() {
        assert_eq!(
            panel_address("wss://hub.lan:7444\n").as_deref(),
            Some("https://hub.lan:7444/")
        );
        assert_eq!(
            panel_address("wss://localhost:7443/x").as_deref(),
            Some("https://localhost:7443/")
        );
        assert_eq!(panel_address("ws://hub:1"), None);
        assert_eq!(panel_address("wss://"), None);
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

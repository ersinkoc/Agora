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

const DEFAULT_LISTEN: &str = "127.0.0.1:7443";

fn hub_dir(o: &Opts) -> PathBuf {
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
    if let (Ok(c), Ok(k)) = (
        std::fs::read_to_string(&cert),
        std::fs::read_to_string(&key),
    ) {
        return Ca::from_pem(&c, &k).map_err(|e| e.to_string());
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
}

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/web_files.rs"));
}

/// The web app the hub serves: `--web DIR` wins, else the copy compiled into this binary (when the build had one).
fn web_assets(o: &Opts) -> Option<Arc<dyn agora_hub::web::Assets>> {
    if let Some(dir) = &o.web {
        return Some(Arc::new(agora_hub::web::DirAssets::new(dir)));
    }
    if embedded::WEB_FILES.is_empty() {
        return None;
    }
    Some(Arc::new(agora_hub::web::EmbeddedAssets(
        embedded::WEB_FILES,
    )))
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
        let sessions = Arc::new(HubSessions::new(Arc::clone(&hub)));
        // A fresh one-time code for the first sign-in; `agora hub login` makes more.
        let web_code = SignIn::new(hub.store(), Arc::new(Lockout::new()))
            .create_code(server::now_ms())
            .map_err(|_| "could not create a sign-in code".to_string())?;
        let state = Arc::new(ServerState {
            hub,
            support: Support {
                generations: vec![1],
                capabilities: vec!["terminal.raw".into()],
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

/// `agora hub login`: a one-time code to sign in to the web app (valid 10 minutes).
pub fn hub_login(say: &Say, o: &Opts) -> ExitCode {
    let dir = hub_dir(o);
    let result = (|| -> Result<String, String> {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let store = SqliteStore::open(&dir.join("hub.db")).map_err(|e| e.to_string())?;
        SignIn::new(&store, Arc::new(Lockout::new()))
            .create_code(server::now_ms())
            .map_err(|_| "could not create a sign-in code".to_string())
    })();
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
    let Ok(ca_pem) = std::fs::read_to_string(ca_file) else {
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
        },
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

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

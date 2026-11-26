//! ADR-0114 decision 6 at the hub, over real sockets: the owner installs on the hub's machine (no message installs or removes a
//! plugin); owner, member and viewer each list the installed plugins, fetch a package in pieces and set and read their own grants,
//! which are kept per person; a request naming a node is refused; a removed plugin is gone at the next list and fetch.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use agora_hub::enroll::Lockout;
use agora_hub::hub::Hub;
use agora_hub::plugins::{Plugins, PIECE};
use agora_hub::server::{self, ClientAuth, ServerState};
use agora_hub::store::SqliteStore;
use agora_hub::users::{Access, Role};
use agora_protocol::frame::{kinds, STREAM_COMMANDS, STREAM_CONTROL, STREAM_EVENTS};
use agora_protocol::handshake::Support;
use agora_protocol::v1::{self, command_ok, command_request as cr, command_response};
use agora_protocol::ws::{self, Conn};
use agora_protocol::Frame;
use prost::Message;
use tokio::net::{TcpListener, TcpStream};

/// Three people: the owner, a member granted no project, and a viewer granted none.
struct People;

impl ClientAuth for People {
    fn check(&self, token: &str) -> bool {
        self.access(token).is_some()
    }
    fn access(&self, token: &str) -> Option<Access> {
        let (role, user, projects) = match token {
            "owner-token" => (Role::Owner, "owner", None),
            "member-token" => (Role::Member, "eda", Some(Default::default())),
            "viewer-token" => (Role::Viewer, "bo", Some(Default::default())),
            _ => return None,
        };
        Some(Access {
            role,
            user: user.into(),
            projects,
            session: String::new(),
        })
    }
}

fn f(stream: u32, kind: u8, payload: Vec<u8>) -> Frame {
    Frame {
        stream_id: stream,
        kind,
        seq: None,
        payload,
    }
}

async fn start() -> (Arc<ServerState>, std::net::SocketAddr) {
    let state = Arc::new(ServerState {
        hub: Arc::new(Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap()),
        support: Support {
            generations: vec![1],
            capabilities: vec![],
            required: vec![],
        },
        ca: None,
        lockout: Arc::new(Lockout::new()),
        auth: Box::new(People),
        assets: None,
        sessions: None,
    });
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(server::run(l, Arc::clone(&state), None));
    (state, addr)
}

async fn next(c: &mut Conn<TcpStream>) -> Frame {
    tokio::time::timeout(Duration::from_secs(10), c.recv())
        .await
        .expect("timeout")
        .unwrap()
        .expect("closed")
}

/// A signed-in window: subscribed, past its `Welcome` and snapshot.
async fn window(addr: std::net::SocketAddr, token: &str) -> Conn<TcpStream> {
    let s = TcpStream::connect(addr).await.unwrap();
    let mut c = ws::connect_with_token(s, &format!("ws://{addr}/"), token)
        .await
        .unwrap();
    c.send(&f(
        STREAM_EVENTS,
        kinds::SUBSCRIBE,
        v1::Subscribe::default().encode_to_vec(),
    ))
    .await
    .unwrap();
    let w = next(&mut c).await;
    assert_eq!((w.stream_id, w.kind), (STREAM_CONTROL, kinds::WELCOME));
    assert_eq!(next(&mut c).await.kind, kinds::SNAPSHOT);
    c
}

async fn ask(c: &mut Conn<TcpStream>, target: &str, command: cr::Command) -> v1::CommandResponse {
    let req = v1::CommandRequest {
        request_id: "r".into(),
        idempotency_key: String::new(),
        target_node: target.into(),
        command: Some(command),
    };
    c.send(&f(STREAM_COMMANDS, 0, req.encode_to_vec()))
        .await
        .unwrap();
    loop {
        let fr = next(c).await;
        if (fr.stream_id, fr.kind) == (STREAM_COMMANDS, 1) {
            return v1::CommandResponse::decode(fr.payload.as_slice()).unwrap();
        }
    }
}

fn ok(r: v1::CommandResponse) -> command_ok::Result {
    match r.result {
        Some(command_response::Result::Ok(v1::CommandOk { result: Some(x) })) => x,
        other => panic!("not an answer: {other:?}"),
    }
}

fn err(r: &v1::CommandResponse) -> &str {
    match &r.result {
        Some(command_response::Result::Err(e)) => &e.code,
        _ => "",
    }
}

fn package(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/plugin-packages")
            .join(name),
    )
    .unwrap()
}

async fn list(c: &mut Conn<TcpStream>) -> Vec<v1::PluginInfo> {
    match ok(ask(c, "", cr::Command::PluginList(v1::PluginList {})).await) {
        command_ok::Result::PluginList(l) => l.plugins,
        other => panic!("{other:?}"),
    }
}

async fn grants(c: &mut Conn<TcpStream>) -> Vec<v1::PluginGrant> {
    match ok(ask(c, "", cr::Command::PluginGrantList(v1::PluginGrantList {})).await) {
        command_ok::Result::PluginGrantList(l) => l.grants,
        other => panic!("{other:?}"),
    }
}

async fn fetch(c: &mut Conn<TcpStream>, id: &str, sha: &str) -> Result<Vec<u8>, String> {
    let mut got = Vec::new();
    loop {
        let r = ask(
            c,
            "",
            cr::Command::PluginFetch(v1::PluginFetch {
                plugin_id: id.into(),
                sha256: sha.into(),
                offset: got.len() as u64,
            }),
        )
        .await;
        if !err(&r).is_empty() {
            return Err(err(&r).to_string());
        }
        let command_ok::Result::PluginFetch(p) = ok(r) else {
            panic!("not a piece")
        };
        assert!(p.data.len() <= PIECE);
        if p.data.is_empty() {
            return Ok(got);
        }
        got.extend(p.data);
    }
}

fn grant(id: &str, version: &str, granted: &[&str], sha: &str) -> cr::Command {
    cr::Command::PluginGrantSet(v1::PluginGrantSet {
        plugin_id: id.into(),
        version: version.into(),
        granted: granted.iter().map(|c| (*c).to_string()).collect(),
        sha256: sha.into(),
    })
}

#[tokio::test]
async fn every_person_reads_the_installed_plugins_and_keeps_grants_of_their_own() {
    let (st, addr) = start().await;
    let bytes = package("ok.bundle");
    Plugins::new(st.hub.store())
        .add(&bytes, "0.3.0", 1)
        .unwrap();
    let mut owner = window(addr, "owner-token").await;
    let mut member = window(addr, "member-token").await;
    let mut viewer = window(addr, "viewer-token").await;

    for c in [&mut owner, &mut member, &mut viewer] {
        let l = list(c).await;
        assert_eq!(l.len(), 1);
        assert_eq!(
            (l[0].plugin_id.as_str(), l[0].version.as_str()),
            ("demo", "1.0.0")
        );
        assert_eq!(l[0].capabilities, ["agents.read"]);
        // The package comes back byte for byte, whoever asks: a member with no project and a viewer included.
        assert_eq!(fetch(c, "demo", &l[0].sha256).await.unwrap(), bytes);
        assert!(grants(c).await.is_empty());
    }
    let sha = list(&mut owner).await[0].sha256.clone();

    // Only the owner installs, and only on the hub's machine: no message adds a plugin, and a grant for one that is not installed
    // installs nothing (whoever sends it).
    for c in [&mut member, &mut viewer, &mut owner] {
        assert_eq!(
            err(&ask(c, "", grant("evil", "1.0.0", &[], &sha)).await),
            "plugin.not_found"
        );
        assert_eq!(list(c).await.len(), 1);
    }

    // Each sets their own grant; a viewer too (choosing what a plugin may see changes nothing on any machine).
    ok(ask(
        &mut member,
        "",
        grant("demo", "1.0.0", &["agents.read"], &sha),
    )
    .await);
    ok(ask(&mut viewer, "", grant("demo", "1.0.0", &[], &sha)).await);
    let m = grants(&mut member).await;
    assert_eq!(
        (m.len(), m[0].granted.as_slice()),
        (1, &["agents.read".to_string()][..])
    );
    assert_eq!(
        m[0].seen,
        ["agents.read"],
        "seen is the installed manifest's own"
    );
    let v = grants(&mut viewer).await;
    assert_eq!((v.len(), v[0].granted.len()), (1, 0));
    assert!(
        grants(&mut owner).await.is_empty(),
        "one person's grant is theirs alone"
    );

    // Not the installed version, or a capability it does not ask for, or a node named.
    assert_eq!(
        err(&ask(&mut member, "", grant("demo", "0.9.0", &[], &sha)).await),
        "plugin.not_found"
    );
    assert_eq!(
        err(&ask(
            &mut member,
            "",
            grant("demo", "1.0.0", &["chat.read"], &sha)
        )
        .await),
        "protocol.invalid_message"
    );
    assert_eq!(
        err(&ask(
            &mut member,
            "n1",
            cr::Command::PluginList(v1::PluginList {})
        )
        .await),
        "protocol.invalid_message"
    );
    assert_eq!(
        err(&ask(&mut viewer, "n1", grant("demo", "1.0.0", &[], &sha)).await),
        "protocol.invalid_message"
    );

    // Stop showing it: that person's grant is forgotten, nobody else's.
    ok(ask(
        &mut viewer,
        "",
        cr::Command::PluginGrantRemove(v1::PluginGrantRemove {
            plugin_id: "demo".into(),
        }),
    )
    .await);
    assert!(grants(&mut viewer).await.is_empty());
    assert_eq!(grants(&mut member).await.len(), 1);

    // The owner removes it on the hub's machine: the next list and fetch no longer have it, and its grants are gone.
    assert!(Plugins::new(st.hub.store()).remove("demo", 2).unwrap());
    assert!(list(&mut member).await.is_empty());
    assert_eq!(
        fetch(&mut member, "demo", &sha).await.unwrap_err(),
        "plugin.not_found"
    );
    assert!(grants(&mut member).await.is_empty());

    // The grants were in the audit trail, each as its person's.
    let rows = agora_hub::audit::Audit::new(st.hub.store())
        .list(0, None, 100)
        .unwrap();
    let mine: Vec<(String, String, String, String)> = rows
        .into_iter()
        .filter(|r| r.action.starts_with("plugin.") && r.who != "owner")
        .map(|r| (r.who, r.action, r.node, r.outcome))
        .collect();
    let row = |w: &str, a: &str, n: &str, o: &str| {
        (w.to_string(), a.to_string(), n.to_string(), o.to_string())
    };
    for want in [
        row("eda", "plugin.grant", "demo", "ok"),
        row("bo", "plugin.grant", "demo", "ok"),
        row("eda", "plugin.grant", "demo", "plugin.not_found"),
        row("eda", "plugin.grant", "evil", "plugin.not_found"),
        row("bo", "plugin.ungrant", "demo", "ok"),
    ] {
        assert!(mine.contains(&want), "{want:?} in {mine:?}");
    }
}

/// A package bigger than one frame comes in pieces, and a replaced package is not served by its old hash.
#[tokio::test]
async fn a_large_package_is_fetched_in_pieces_and_a_replaced_one_by_its_new_hash_only() {
    let (st, addr) = start().await;
    // A package of about 1.5 MiB: the shared fixture with a big text file added and listed.
    let entry = b"agora.watch('agents.read', function () {});";
    let text = vec![b'a'; 1_500_000];
    let sha = |b: &[u8]| -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(b)
            .iter()
            .map(|x| format!("{x:02x}"))
            .collect()
    };
    let b64 = |b: &[u8]| -> String {
        const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for c in b.chunks(3) {
            let n = (u32::from(c[0]) << 16)
                | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
                | u32::from(*c.get(2).unwrap_or(&0));
            for i in 0..4 {
                out.push(if i <= c.len() {
                    A[((n >> (18 - 6 * i)) & 63) as usize] as char
                } else {
                    '='
                });
            }
        }
        out
    };
    let make = |version: &str| -> Vec<u8> {
        let manifest = format!(
            r#"{{"id":"big","name":"Big","version":"{version}","author":"A","licence":"MIT","min_agora":"0.1.0","panels":[{{"id":"main"}}],"capabilities":[],"entry":"panel.js","files":{{"panel.js":"{}","big.txt":"{}"}}}}"#,
            sha(entry),
            sha(&text)
        );
        format!(
            r#"{{"agora-plugin-bundle":1,"files":{{"agora-plugin.json":"{}","panel.js":"{}","big.txt":"{}"}}}}"#,
            b64(manifest.as_bytes()),
            b64(entry),
            b64(&text)
        )
        .into_bytes()
    };
    let one = make("1.0.0");
    assert!(
        one.len() > agora_protocol::frame::DEFAULT_MAX_PAYLOAD,
        "bigger than a frame"
    );
    Plugins::new(st.hub.store()).add(&one, "0.3.0", 1).unwrap();
    let mut c = window(addr, "viewer-token").await;
    let old = list(&mut c).await[0].sha256.clone();
    assert_eq!(fetch(&mut c, "big", &old).await.unwrap(), one);
    let two = make("1.1.0");
    Plugins::new(st.hub.store()).add(&two, "0.3.0", 2).unwrap();
    assert_eq!(
        fetch(&mut c, "big", &old).await.unwrap_err(),
        "plugin.not_found"
    );
    let new = list(&mut c).await[0].sha256.clone();
    assert_eq!(fetch(&mut c, "big", &new).await.unwrap(), two);
}

/// F3: one person's grant changes are limited (30 a minute); past that nothing changes and a loop of refusals leaves one audit row.
#[tokio::test]
async fn grant_changes_past_the_limit_are_refused_with_one_row() {
    let (st, addr) = start().await;
    let mut member = window(addr, "member-token").await;
    let remove = || {
        cr::Command::PluginGrantRemove(v1::PluginGrantRemove {
            plugin_id: "demo".into(),
        })
    };
    for _ in 0..30 {
        ok(ask(&mut member, "", remove()).await);
    }
    for _ in 0..4 {
        assert_eq!(err(&ask(&mut member, "", remove()).await), "rate.limited");
    }
    // Another person is counted apart.
    let mut viewer = window(addr, "viewer-token").await;
    ok(ask(&mut viewer, "", remove()).await);
    let rows = agora_hub::audit::Audit::new(st.hub.store())
        .list(0, None, 1000)
        .unwrap();
    let limited = rows
        .iter()
        .filter(|r| r.who == "eda" && r.outcome == "rate.limited")
        .count();
    assert_eq!(limited, 1);
}

/// ADR-0114 decision 7: every signed-in person may list the catalog's entries, and only while the owner has it on; a request naming a
/// node is refused. The list says what is available; nothing in it installs anything.
#[tokio::test]
async fn the_catalog_is_listed_to_everyone_only_while_it_is_on() {
    use agora_hub::plugin_catalog::{self, Entry, Fetched};
    let (st, addr) = start().await;
    let store = st.hub.store();
    // A public key made of chosen bytes, for the test only: no private key exists for it.
    let mut blob = Vec::new();
    for part in [b"ssh-ed25519".as_slice(), &[7u8; 32]] {
        blob.extend_from_slice(&(part.len() as u32).to_be_bytes());
        blob.extend_from_slice(part);
    }
    let line = format!("ssh-ed25519 {}", agora_hub::sshsig::base64_encode(&blob));
    let key = plugin_catalog::set_key(store, &line, 1).unwrap();
    let fetched = Fetched {
        serial: 3,
        entries: vec![Entry {
            id: "demo".into(),
            name: "Demo".into(),
            version: "1.2.0".into(),
            author: "Someone".into(),
            licence: "MIT".into(),
            description: "Lists the agents.".into(),
            capabilities: vec!["agents.read".into()],
            sha256: "a".repeat(64),
            size: 1000,
            file: "plugins/demo.json".into(),
        }],
        refused: Vec::new(),
    };
    plugin_catalog::save(store, &fetched, "http://127.0.0.1:9/index.json", &key, 50).unwrap();
    let list = || cr::Command::PluginCatalogList(v1::PluginCatalogList {});
    let catalog = |r: v1::CommandResponse| match ok(r) {
        command_ok::Result::PluginCatalogList(l) => l,
        other => panic!("{other:?}"),
    };
    let mut people = Vec::new();
    for token in ["owner-token", "member-token", "viewer-token"] {
        people.push(window(addr, token).await);
    }
    for c in &mut people {
        assert_eq!(
            catalog(ask(c, "", list()).await),
            v1::PluginCatalogListResult::default(),
            "off: nothing"
        );
    }
    plugin_catalog::set_on(store, true, 60).unwrap();
    for c in &mut people {
        let l = catalog(ask(c, "", list()).await);
        assert!(l.on);
        assert_eq!(l.updated_at_ms, 50);
        assert_eq!(l.entries.len(), 1);
        assert_eq!(
            (
                l.entries[0].plugin_id.as_str(),
                l.entries[0].version.as_str()
            ),
            ("demo", "1.2.0")
        );
        assert_eq!(
            err(&ask(c, "n1", list()).await),
            "protocol.invalid_message",
            "it names no node"
        );
    }
    // Listing installs nothing.
    assert!(Plugins::new(store).list().unwrap().is_empty());
}

//! ADR-0113 decision 6 (build step 4) at the hub, over real sockets: owner, member and viewer each read and write only their own settings
//! value (keyed by the session's person, never by the request); a write based on an older version is refused and changes nothing; a
//! value over 256 KiB is refused whole; a request naming a node or an unknown key is refused; writes are audit rows without the value.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use agora_hub::enroll::Lockout;
use agora_hub::hub::Hub;
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

/// Three people: the owner, a member granted no project, and a viewer granted none. Eda signs in from two windows (one token).
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

fn get(key: &str) -> cr::Command {
    cr::Command::UserSettingsGet(v1::UserSettingsGet { key: key.into() })
}

fn set(key: &str, value: &[u8], base: u64) -> cr::Command {
    cr::Command::UserSettingsSet(v1::UserSettingsSet {
        key: key.into(),
        value: value.to_vec(),
        base_version: base,
    })
}

async fn value(c: &mut Conn<TcpStream>) -> v1::UserSettingsValue {
    match ok(ask(c, "", get("themes")).await) {
        command_ok::Result::UserSettings(v) => v,
        other => panic!("{other:?}"),
    }
}

async fn write(c: &mut Conn<TcpStream>, v: &[u8], base: u64) -> v1::UserSettingsValue {
    match ok(ask(c, "", set("themes", v, base)).await) {
        command_ok::Result::UserSettings(v) => v,
        other => panic!("{other:?}"),
    }
}

fn param(r: &v1::CommandResponse, name: &str) -> Option<v1::ParamValue> {
    match &r.result {
        Some(command_response::Result::Err(e)) => e.params.get(name).cloned(),
        _ => None,
    }
}

#[tokio::test]
async fn every_person_reads_and_writes_only_their_own_value() {
    let (st, addr) = start().await;
    let mut owner = window(addr, "owner-token").await;
    let mut eda = window(addr, "member-token").await;
    let mut bo = window(addr, "viewer-token").await;

    for c in [&mut owner, &mut eda, &mut bo] {
        let v = value(c).await;
        assert_eq!(
            (v.key.as_str(), v.version, v.value.len(), v.updated_ms),
            ("themes", 0, 0, 0)
        );
    }
    // A viewer keeps their own too: it changes nothing on any machine.
    assert_eq!(write(&mut eda, b"eda's themes", 0).await.version, 1);
    assert_eq!(write(&mut bo, b"bo's themes", 0).await.version, 1);
    assert_eq!(value(&mut eda).await.value, b"eda's themes");
    assert_eq!(value(&mut bo).await.value, b"bo's themes");
    // Person A cannot read B's: the owner reads their own (none yet), whatever anyone else kept.
    let o = value(&mut owner).await;
    assert_eq!((o.version, o.value.len()), (0, 0));
    // Nor write over it: the owner's write lands in the owner's row, with the owner's own versions.
    assert_eq!(write(&mut owner, b"owner's themes", 0).await.version, 1);
    assert_eq!(value(&mut eda).await.value, b"eda's themes");
    assert_eq!(value(&mut bo).await.value, b"bo's themes");

    // A request that names a machine, or a key the hub does not keep, is refused.
    for (c, target) in [
        (get("themes"), "n1"),
        (set("themes", b"x", 1), "n1"),
        (get("layout"), ""),
        (set("layout", b"x", 0), ""),
    ] {
        let r = ask(&mut eda, target, c).await;
        assert_eq!(err(&r), "protocol.invalid_message");
    }
    assert_eq!(value(&mut eda).await.version, 1);
    drop(st);
}

#[tokio::test]
async fn a_stale_write_and_a_value_over_the_cap_change_nothing_and_writes_are_audited() {
    let (st, addr) = start().await;
    let mut laptop = window(addr, "member-token").await;
    let mut phone = window(addr, "member-token").await;
    assert_eq!(write(&mut laptop, b"first", 0).await.version, 1);
    // Both windows read version 1; the laptop writes first; the phone's write, based on version 1, is refused and names version 2.
    assert_eq!(value(&mut phone).await.version, 1);
    assert_eq!(write(&mut laptop, b"laptop", 1).await.version, 2);
    let r = ask(&mut phone, "", set("themes", b"phone", 1)).await;
    assert_eq!(err(&r), "settings.stale");
    assert_eq!(
        param(&r, "version"),
        Some(v1::ParamValue {
            value: Some(v1::param_value::Value::Int(2))
        })
    );
    // The phone reads again, merges and writes on version 2.
    let now = value(&mut phone).await;
    assert_eq!((now.value.as_slice(), now.version), (&b"laptop"[..], 2));
    assert_eq!(write(&mut phone, b"laptop+phone", 2).await.version, 3);
    // Over 256 KiB is refused whole and names the cap; the value stays.
    let r = ask(
        &mut laptop,
        "",
        set("themes", &vec![b'x'; 256 * 1024 + 1], 3),
    )
    .await;
    assert_eq!(err(&r), "settings.too_large");
    assert_eq!(
        param(&r, "max"),
        Some(v1::ParamValue {
            value: Some(v1::param_value::Value::Int(262_144))
        })
    );
    assert_eq!(value(&mut laptop).await.value, b"laptop+phone");
    assert_eq!(
        write(&mut laptop, &vec![b'y'; 256 * 1024], 3).await.version,
        4
    );

    // Each write is an audit row of its person naming the key and the outcome; nothing of the value; a read and a stale write (which
    // changed nothing) write none.
    let rows = agora_hub::audit::Audit::new(st.hub.store())
        .list(0, None, 100)
        .unwrap();
    let got: Vec<(String, String, String, String)> = rows
        .iter()
        .filter(|r| r.action.starts_with("settings."))
        .map(|r| {
            (
                r.who.clone(),
                r.action.clone(),
                r.node.clone(),
                r.outcome.clone(),
            )
        })
        .collect();
    let row = |o: &str| {
        (
            "eda".to_string(),
            "settings.set".to_string(),
            "themes".to_string(),
            o.to_string(),
        )
    };
    let mut want = vec![
        row("ok"),
        row("ok"),
        row("ok"),
        row("settings.too_large"),
        row("ok"),
    ];
    let mut sorted = got.clone();
    sorted.sort();
    want.sort();
    assert_eq!(sorted, want, "{got:?}");
    for r in &rows {
        for text in [&r.action, &r.node, &r.outcome] {
            assert!(
                !text.contains("laptop") && !text.contains("phone") && !text.contains("first"),
                "{r:?}"
            );
        }
    }
}

/// What an older hub answers a window's settings request: it decodes one as a request with no command (an arm it does not know), which
/// its routing refuses first as `auth.forbidden` naming the action `None`, whoever asks. The window takes exactly that as "this hub keeps
/// nothing" (`apps/web/src/themes/sync.ts` `isOlderHub`). This hub answers such a request the same way.
#[tokio::test]
async fn a_request_with_no_command_is_refused_as_an_older_hub_refuses_an_unknown_one() {
    let (_st, addr) = start().await;
    for token in ["owner-token", "member-token", "viewer-token"] {
        let mut c = window(addr, token).await;
        let req = v1::CommandRequest {
            request_id: "r".into(),
            idempotency_key: String::new(),
            target_node: String::new(),
            command: None,
        };
        c.send(&f(STREAM_COMMANDS, 0, req.encode_to_vec()))
            .await
            .unwrap();
        let r = loop {
            let fr = next(&mut c).await;
            if (fr.stream_id, fr.kind) == (STREAM_COMMANDS, 1) {
                break v1::CommandResponse::decode(fr.payload.as_slice()).unwrap();
            }
        };
        assert_eq!(err(&r), "auth.forbidden", "{token}");
        assert_eq!(
            param(&r, "action"),
            Some(v1::ParamValue {
                value: Some(v1::param_value::Value::Str("None".into()))
            }),
            "{token}"
        );
    }
}

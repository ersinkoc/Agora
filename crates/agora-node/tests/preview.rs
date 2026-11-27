//! ADR-0088 build step 2: a node's preview forwards. The owner opens, lists and closes them on the control socket; through the hub an
//! open needs a device's `preview-action-v1` signature for exactly that node, port and lifetime, once. The node's own ports and ports
//! nothing listens on are refused, whoever asks. Driven against a real node, the hub played over plain WebSocket.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agora_node::control::Client;
use agora_node::daemon::{Node, NodeConfig};
use agora_node::uplink_client::{spawn, HubLink};
use agora_protocol::frame::{kinds, STREAM_COMMANDS, STREAM_CONTROL};
use agora_protocol::v1::{
    self, command_ok::Result as Ok_, command_request::Command as Cmd, command_response,
    CommandRequest, CommandResponse,
};
use agora_protocol::Frame;
use prost::Message;
use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};

struct Env {
    dir: PathBuf,
    key: EcdsaKeyPair,
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A node home with one trusted device, `phone`, and `policy` as the node's policy file.
fn env(name: &str, policy: &str) -> Env {
    let dir = std::env::temp_dir().join(format!("agora-preview-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let data = dir.join("data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(dir.join("manifests")).unwrap();
    std::fs::write(data.join("policy"), policy).unwrap();
    let rng = ring::rand::SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
    let key =
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng).unwrap();
    let hex: String = key
        .public_key()
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    std::fs::write(
        data.join(agora_node::approvals::TRUSTED_DEVICES_FILE),
        format!("phone {hex}\n"),
    )
    .unwrap();
    Env { dir, key }
}

fn node(e: &Env) -> Node {
    Node::start(&NodeConfig {
        resume_all: false,
        node_id: "laptop".into(),
        socket: e.dir.join("c.sock"),
        data_dir: e.dir.join("data"),
        worktree_root: e.dir.join("wt"),
        manifests_dir: e.dir.join("manifests"),
        projects: Vec::new(),
        limits: Default::default(),
    })
    .expect("start node")
}

/// A port something listens on (the "dev server"), kept open while the listener lives.
fn server() -> (TcpListener, u32) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let p = l.local_addr().unwrap().port();
    (l, u32::from(p))
}

fn open(port: u32, seconds: u32) -> Cmd {
    Cmd::PreviewOpen(v1::PreviewOpen {
        port,
        expires_seconds: seconds,
        authorization: None,
    })
}

/// The open signed by `phone` for `node`, `port` and `seconds`, with nonce number `n`, valid for a minute.
fn signed(e: &Env, node: &str, port: u32, seconds: u32, n: u8) -> Cmd {
    let nonce = format!("{n:032x}");
    let expires = agora_node::grants::now_ms() + 60_000;
    let msg = agora_node::preview::signed_message(node, port, seconds, &nonce, expires);
    let signature = e
        .key
        .sign(&ring::rand::SystemRandom::new(), &msg)
        .unwrap()
        .as_ref()
        .to_vec();
    Cmd::PreviewOpen(v1::PreviewOpen {
        port,
        expires_seconds: seconds,
        authorization: Some(v1::PreviewAuthorization {
            device_id: "phone".into(),
            nonce,
            expires_at_ms: expires,
            signature,
        }),
    })
}

fn code(r: &CommandResponse) -> String {
    match &r.result {
        Some(command_response::Result::Ok(_)) => "ok".into(),
        Some(command_response::Result::Err(e)) => e.code.clone(),
        None => "none".into(),
    }
}

fn reason(r: &CommandResponse) -> String {
    match &r.result {
        Some(command_response::Result::Err(e)) => {
            match e.params.get("reason").and_then(|p| p.value.as_ref()) {
                Some(v1::param_value::Value::Str(s)) => s.clone(),
                _ => String::new(),
            }
        }
        _ => String::new(),
    }
}

fn forward(r: &CommandResponse) -> v1::PreviewForward {
    match &r.result {
        Some(command_response::Result::Ok(v1::CommandOk {
            result: Some(Ok_::PreviewOpen(o)),
        })) => o.forward.clone().unwrap(),
        other => panic!("not an open: {other:?}"),
    }
}

fn local(e: &Env, cmd: Cmd) -> CommandResponse {
    let mut c = Client::connect(&e.dir.join("c.sock")).unwrap();
    let req = CommandRequest {
        request_id: "r".into(),
        command: Some(cmd),
        ..Default::default()
    };
    c.request_on(STREAM_COMMANDS, &req, Duration::from_secs(10))
        .unwrap()
}

fn listed(e: &Env) -> Vec<v1::PreviewForward> {
    match local(e, Cmd::PreviewList(v1::PreviewList {})).result {
        Some(command_response::Result::Ok(v1::CommandOk {
            result: Some(Ok_::PreviewList(l)),
        })) => l.forwards,
        other => panic!("not a list: {other:?}"),
    }
}

fn changed(n: &Node) -> Vec<v1::PreviewForward> {
    n.events
        .events_after(0)
        .into_iter()
        .filter_map(|e| match e.body {
            Some(v1::event::Body::PreviewChanged(c)) => c.forward,
            _ => None,
        })
        .collect()
}

fn frame(stream_id: u32, kind: u8, payload: Vec<u8>) -> Frame {
    Frame {
        stream_id,
        kind,
        seq: None,
        payload,
    }
}

/// Plays the hub: sends each command on stream 1 once the previous one was answered, and returns the answers in order.
fn through_hub(n: &Node, e: &Env, cmds: Vec<Cmd>) -> Vec<CommandResponse> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let l = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let port = l.local_addr().unwrap().port();
    let uplink = spawn(
        Arc::clone(&n.events),
        HubLink {
            url: format!("ws://127.0.0.1:{port}"),
            ca_pem: String::new(),
            cert_pem: String::new(),
            key_pem: String::new(),
            commands: Some(e.dir.join("c.sock")),
            mesh_out: None,
            grants: Some(Arc::clone(&n.grants)),
            mesh: None,
            devices: None,
            state_dir: None,
            certs: None,
            spawner: None,
        },
    )
    .unwrap();
    let answers = rt.block_on(async move {
        let (s, _) = l.accept().await.unwrap();
        let mut c = agora_protocol::ws::accept(s).await.unwrap();
        assert_eq!(c.recv().await.unwrap().unwrap().kind, kinds::HELLO);
        let welcome = v1::Welcome {
            chosen_generation: 1,
            ..Default::default()
        };
        c.send(&frame(
            STREAM_CONTROL,
            kinds::WELCOME,
            welcome.encode_to_vec(),
        ))
        .await
        .unwrap();
        while c.recv().await.unwrap().unwrap().kind != kinds::RESUME {}
        let ok = v1::ResumeOk { from_seq: 1 };
        c.send(&frame(STREAM_CONTROL, kinds::RESUME_OK, ok.encode_to_vec()))
            .await
            .unwrap();
        let mut answers = Vec::new();
        for (i, cmd) in cmds.into_iter().enumerate() {
            let req = CommandRequest {
                request_id: format!("h{i}"),
                command: Some(cmd),
                ..Default::default()
            };
            c.send(&frame(STREAM_COMMANDS, 0, req.encode_to_vec()))
                .await
                .unwrap();
            let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
            loop {
                let f = tokio::time::timeout_at(deadline, c.recv())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                if (f.stream_id, f.kind) == (STREAM_CONTROL, kinds::PING) {
                    let _ = c.send(&frame(STREAM_CONTROL, kinds::PONG, f.payload)).await;
                } else if (f.stream_id, f.kind) == (STREAM_COMMANDS, 1) {
                    answers.push(CommandResponse::decode(f.payload.as_slice()).unwrap());
                    break;
                }
            }
        }
        answers
    });
    drop(uplink);
    answers
}

#[test]
fn the_owner_opens_lists_and_closes_a_forward_and_each_change_is_recorded() {
    let e = env("owner", "");
    let n = node(&e);
    let (_app, port) = server();
    let r = local(&e, open(port, 0));
    assert_eq!(code(&r), "ok");
    let f = forward(&r);
    assert_eq!((f.port, f.creator.as_str()), (port, "owner"));
    assert!(
        f.forward_id.starts_with("fwd-") && f.forward_id.len() == 36,
        "unguessable: {}",
        f.forward_id
    );
    assert_eq!(f.expires_at_ms - f.opened_at_ms, 8 * 3600 * 1000);
    assert_eq!(listed(&e), vec![f.clone()]);
    let closed = local(
        &e,
        Cmd::PreviewClose(v1::PreviewClose {
            forward_id: f.forward_id.clone(),
            all: false,
        }),
    );
    assert_eq!(code(&closed), "ok");
    assert!(listed(&e).is_empty());
    let seen = changed(&n);
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert_eq!(seen[0].state, v1::PreviewState::Open as i32);
    assert_eq!(seen[1].close_reason, v1::PreviewCloseReason::Closed as i32);
    // An id that is no longer open, and a close that names both or neither.
    let again = local(
        &e,
        Cmd::PreviewClose(v1::PreviewClose {
            forward_id: f.forward_id,
            all: false,
        }),
    );
    assert_eq!(code(&again), "preview.unknown");
    let both = local(
        &e,
        Cmd::PreviewClose(v1::PreviewClose {
            forward_id: "fwd-any".into(),
            all: true,
        }),
    );
    assert_eq!(code(&both), "protocol.invalid_message");
}

#[test]
fn the_nodes_own_ports_privileged_and_silent_ports_are_refused_for_the_owner_too() {
    let e = env("ports", "");
    // A hub on this machine (its `listen` file next to the data folder) and the panel the window uses.
    let (_hub, hub) = server();
    let (_panel, panel) = server();
    std::fs::create_dir_all(e.dir.join("hub")).unwrap();
    std::fs::create_dir_all(e.dir.join("link")).unwrap();
    std::fs::write(e.dir.join("hub/listen"), format!("0.0.0.0:{hub}")).unwrap();
    std::fs::write(e.dir.join("link/panel-port"), panel.to_string()).unwrap();
    let _n = node(&e);
    for port in [hub, panel] {
        let r = local(&e, open(port, 0));
        assert_eq!(
            (code(&r).as_str(), reason(&r).as_str()),
            ("preview.port_refused", "node_port"),
            "{port}"
        );
    }
    let r = local(&e, open(80, 0));
    assert_eq!(reason(&r), "privileged");
    let (gone, silent) = server();
    drop(gone);
    assert_eq!(reason(&local(&e, open(silent, 0))), "not_loopback");
    assert!(listed(&e).is_empty());
}

#[test]
fn through_the_hub_an_open_needs_the_devices_signature_for_exactly_that_open_once() {
    let e = env("hub", "");
    let n = node(&e);
    let (_app, port) = server();
    let (_other, other_port) = server();
    let replay = signed(&e, "laptop", port, 0, 9);
    let Cmd::PreviewOpen(mut moved) = signed(&e, "laptop", port, 0, 3) else {
        unreachable!()
    };
    moved.port = other_port;
    let Cmd::PreviewOpen(mut longer) = signed(&e, "laptop", port, 600, 4) else {
        unreachable!()
    };
    longer.expires_seconds = 86_400;
    let answers = through_hub(
        &n,
        &e,
        vec![
            open(port, 0),
            signed(&e, "desktop", port, 0, 2),
            Cmd::PreviewOpen(moved),
            Cmd::PreviewOpen(longer),
            replay.clone(),
            replay,
            Cmd::PreviewList(v1::PreviewList {}),
        ],
    );
    let codes: Vec<String> = answers.iter().map(code).collect();
    assert_eq!(
        codes,
        [
            "approval.signature_invalid", // unsigned
            "approval.signature_invalid", // signed for another node
            "approval.signature_invalid", // the hub changed the port
            "approval.signature_invalid", // the hub changed the lifetime
            "ok",
            "approval.signature_invalid", // the same nonce again
            "ok",
        ]
    );
    let f = forward(&answers[4]);
    assert_eq!((f.port, f.creator.as_str()), (port, "phone"));
    match &answers[6].result {
        Some(command_response::Result::Ok(v1::CommandOk {
            result: Some(Ok_::PreviewList(l)),
        })) => assert_eq!(l.forwards, vec![f]),
        other => panic!("not a list: {other:?}"),
    }
}

#[test]
fn a_signed_open_still_meets_every_port_rule() {
    let e = env("hub-ports", "");
    let (_hub, hub) = server();
    std::fs::create_dir_all(e.dir.join("hub")).unwrap();
    std::fs::write(e.dir.join("hub/listen"), format!("127.0.0.1:{hub}")).unwrap();
    let n = node(&e);
    let answers = through_hub(
        &n,
        &e,
        vec![
            signed(&e, "laptop", hub, 0, 1),
            signed(&e, "laptop", 22, 0, 2),
        ],
    );
    assert_eq!(reason(&answers[0]), "node_port");
    assert_eq!(reason(&answers[1]), "privileged");
}

#[test]
fn a_device_limited_to_some_projects_opens_no_preview() {
    let e = env("limited", "device.phone.projects = id:somewhere\n");
    let n = node(&e);
    let (_app, port) = server();
    let answers = through_hub(&n, &e, vec![signed(&e, "laptop", port, 0, 1)]);
    assert_eq!(code(&answers[0]), "auth.forbidden");
    assert!(listed(&e).is_empty());
}

#[test]
fn a_stopping_node_ends_its_forwards() {
    let e = env("stop", "");
    let n = node(&e);
    let (_app, port) = server();
    assert_eq!(code(&local(&e, open(port, 0))), "ok");
    let events = Arc::clone(&n.events);
    drop(n);
    let last = events
        .events_after(0)
        .into_iter()
        .filter_map(|e| match e.body {
            Some(v1::event::Body::PreviewChanged(c)) => c.forward,
            _ => None,
        })
        .next_back()
        .unwrap();
    assert_eq!(
        last.close_reason,
        v1::PreviewCloseReason::NodeStopped as i32
    );
}

#[test]
fn a_used_signature_stays_used_after_the_node_restarts() {
    let e = env("restart", "");
    let (_app, port) = server();
    let once = signed(&e, "laptop", port, 0, 7);
    let n = node(&e);
    assert_eq!(code(&through_hub(&n, &e, vec![once.clone()])[0]), "ok");
    drop(n);
    let n = node(&e);
    assert!(listed(&e).is_empty(), "forwards are never restored");
    assert_eq!(
        code(&through_hub(&n, &e, vec![once])[0]),
        "approval.signature_invalid"
    );
}

// ---- relayed connections (step 2b) ----

use agora_protocol::frame::STREAM_CLOSE;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A server that echoes what it reads and counts the connections it accepted.
fn echo() -> (u32, Arc<AtomicUsize>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = u32::from(l.local_addr().unwrap().port());
    let accepted = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&accepted);
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { return };
            count.fetch_add(1, Ordering::SeqCst);
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = s.read(&mut buf) {
                    if n == 0 || s.write_all(&buf[..n]).is_err() {
                        return;
                    }
                }
            });
        }
    });
    (port, accepted)
}

fn opened(e: &Env, port: u32) -> String {
    forward(&local(e, open(port, 0))).forward_id
}

fn client(e: &Env) -> Client {
    Client::connect(&e.dir.join("c.sock")).unwrap()
}

fn connect(c: &mut Client, stream: u32, forward_id: &str) -> CommandResponse {
    let req = CommandRequest {
        request_id: format!("connect-{stream}"),
        command: Some(Cmd::PreviewConnect(v1::PreviewConnect {
            forward_id: forward_id.into(),
        })),
        ..Default::default()
    };
    c.request_on(stream, &req, Duration::from_secs(10)).unwrap()
}

/// Frames on `stream` until `until` holds for the last one or 10 s pass.
fn frames(c: &mut Client, stream: u32, until: impl Fn(&Frame) -> bool) -> Vec<Frame> {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut out = Vec::new();
    while std::time::Instant::now() < deadline {
        if let Ok(Some(f)) = c.read_frame(Some(Duration::from_millis(100))) {
            if f.stream_id == stream {
                let stop = until(&f);
                out.push(f);
                if stop {
                    break;
                }
            }
        }
    }
    out
}

fn data(fs: &[Frame]) -> Vec<u8> {
    fs.iter()
        .filter(|f| f.kind == kinds::PREVIEW_DATA)
        .flat_map(|f| f.payload.clone())
        .collect()
}

fn end_of(fs: &[Frame]) -> v1::PreviewEnd {
    let f = fs
        .iter()
        .find(|f| f.kind == kinds::PREVIEW_END)
        .expect("a PREVIEW_END");
    v1::PreviewEnd::decode(f.payload.as_slice()).unwrap()
}

fn is_close(f: &Frame) -> bool {
    f.kind == STREAM_CLOSE
}

#[test]
fn a_connection_carries_bytes_both_ways_unchanged_and_reaches_only_its_port() {
    let e = env("relay", "");
    let _n = node(&e);
    let (port, accepted) = echo();
    let (other, others) = echo();
    let id = opened(&e, port);
    // The open looked once whether the port answers; that connection is not the relayed one.
    let probed = accepted.load(Ordering::SeqCst);
    let mut c = client(&e);
    let r = connect(&mut c, 3, &id);
    match &r.result {
        Some(command_response::Result::Ok(v1::CommandOk {
            result: Some(Ok_::PreviewConnect(p)),
        })) => assert_eq!((p.max_chunk_bytes, p.window_bytes), (16_384, 262_144)),
        other => panic!("not a connect: {other:?}"),
    }
    // A request naming another host and port, even absolutely: the bytes go to the forward's port as they are.
    let request = format!("GET http://127.0.0.1:{other}/ HTTP/1.1\r\nHost: 127.0.0.1:{other}\r\nOrigin: http://127.0.0.1:{other}\r\n\r\n");
    c.send_frame(3, kinds::PREVIEW_DATA, request.clone().into_bytes())
        .unwrap();
    let want = request.len();
    let fs = {
        let mut all = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while data(&all).len() < want && std::time::Instant::now() < deadline {
            all.extend(frames(&mut c, 3, |f| f.kind == kinds::PREVIEW_DATA));
        }
        all
    };
    assert_eq!(data(&fs), request.as_bytes(), "echoed unchanged");
    assert_eq!(accepted.load(Ordering::SeqCst), probed + 1);
    assert_eq!(
        others.load(Ordering::SeqCst),
        0,
        "the other port saw nothing"
    );
    // The window ends it; the node says nothing back and the program's connection closes.
    c.send_frame(
        3,
        kinds::PREVIEW_END,
        v1::PreviewEnd::default().encode_to_vec(),
    )
    .unwrap();
    // The stream can carry a new connection once the old one is over.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(code(&connect(&mut c, 3, &id)), "ok");
}

#[test]
fn the_program_closing_delivers_its_last_bytes_then_says_target_gone() {
    let e = env("gone", "");
    let _n = node(&e);
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = u32::from(l.local_addr().unwrap().port());
    // Every connection hears "bye" and is closed (the open's own look at the port included).
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { return };
            let _ = s.write_all(b"bye");
        }
    });
    let id = opened(&e, port);
    let mut c = client(&e);
    assert_eq!(code(&connect(&mut c, 3, &id)), "ok");
    let fs = frames(&mut c, 3, is_close);
    assert_eq!(data(&fs), b"bye");
    let end = end_of(&fs);
    assert_eq!(end.reason, v1::PreviewCloseReason::TargetGone as i32);
    assert_eq!((end.bytes_from_target, end.bytes_to_target), (3, 0));
    assert!(is_close(fs.last().unwrap()), "STREAM_CLOSE comes last");
}

#[test]
fn a_frame_over_the_chunk_ends_the_connection_for_breaking_the_rules() {
    let e = env("chunk", "");
    let _n = node(&e);
    let (port, _) = echo();
    let id = opened(&e, port);
    let mut c = client(&e);
    assert_eq!(code(&connect(&mut c, 3, &id)), "ok");
    c.send_frame(3, kinds::PREVIEW_DATA, vec![b'x'; 16_385])
        .unwrap();
    let fs = frames(&mut c, 3, is_close);
    assert_eq!(end_of(&fs).reason, v1::PreviewCloseReason::Protocol as i32);
}

#[test]
fn closing_the_forward_ends_its_live_connections() {
    let e = env("close-live", "");
    let _n = node(&e);
    let (port, _) = echo();
    let id = opened(&e, port);
    let mut c = client(&e);
    assert_eq!(code(&connect(&mut c, 3, &id)), "ok");
    let closed = local(
        &e,
        Cmd::PreviewClose(v1::PreviewClose {
            forward_id: id.clone(),
            all: false,
        }),
    );
    assert_eq!(code(&closed), "ok");
    let fs = frames(&mut c, 3, is_close);
    assert_eq!(end_of(&fs).reason, v1::PreviewCloseReason::Closed as i32);
    assert_eq!(code(&connect(&mut c, 5, &id)), "preview.unknown");
}

#[test]
fn over_the_forwards_total_the_connection_ends_with_budget() {
    let e = env("total", "");
    let n = node(&e);
    n.previews.set_budgets(agora_node::preview::Budgets {
        total: 10,
        ..Default::default()
    });
    let (port, _) = echo();
    let id = opened(&e, port);
    let mut c = client(&e);
    assert_eq!(code(&connect(&mut c, 3, &id)), "ok");
    c.send_frame(3, kinds::PREVIEW_DATA, b"0123456789A".to_vec())
        .unwrap();
    let fs = frames(&mut c, 3, is_close);
    assert_eq!(end_of(&fs).reason, v1::PreviewCloseReason::Budget as i32);
}

#[test]
fn one_connection_over_the_cap_is_answered_then_ended_with_budget() {
    let e = env("cap", "");
    let _n = node(&e);
    let (port, accepted) = echo();
    let id = opened(&e, port);
    let probed = accepted.load(Ordering::SeqCst);
    let mut c = client(&e);
    let cap = agora_node::preview::MAX_CONNECTIONS_PER_FORWARD as u32;
    for i in 0..cap {
        assert_eq!(code(&connect(&mut c, 3 + i, &id)), "ok", "{i}");
    }
    let over = 3 + cap;
    assert_eq!(code(&connect(&mut c, over, &id)), "ok");
    let fs = frames(&mut c, over, is_close);
    let end = end_of(&fs);
    assert_eq!(end.reason, v1::PreviewCloseReason::Budget as i32);
    assert_eq!((end.bytes_to_target, end.bytes_from_target), (0, 0));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        accepted.load(Ordering::SeqCst),
        probed + cap as usize,
        "the one over the cap never reached the program"
    );
}

#[test]
fn a_port_that_became_the_nodes_own_ends_the_forward_at_the_next_connect() {
    let e = env("became-own", "");
    let n = node(&e);
    let (port, _) = echo();
    let id = opened(&e, port);
    std::fs::create_dir_all(e.dir.join("hub")).unwrap();
    std::fs::write(e.dir.join("hub/listen"), format!("127.0.0.1:{port}")).unwrap();
    let mut c = client(&e);
    let r = connect(&mut c, 3, &id);
    assert_eq!(
        (code(&r).as_str(), reason(&r).as_str()),
        ("preview.port_refused", "node_port")
    );
    assert!(listed(&e).is_empty());
    let last = changed(&n).pop().unwrap();
    assert_eq!(last.close_reason, v1::PreviewCloseReason::Policy as i32);
}

#[test]
fn an_unknown_forward_takes_no_connection() {
    let e = env("unknown", "");
    let _n = node(&e);
    let mut c = client(&e);
    assert_eq!(code(&connect(&mut c, 3, "fwd-9")), "preview.unknown");
}

#[test]
fn the_window_going_over_the_byte_rate_ends_the_connection_with_budget() {
    let e = env("rate", "");
    let n = node(&e);
    n.previews.set_budgets(agora_node::preview::Budgets {
        rate: 8,
        ..Default::default()
    });
    let (port, _) = echo();
    let id = opened(&e, port);
    let mut c = client(&e);
    let r = connect(&mut c, 3, &id);
    match &r.result {
        Some(command_response::Result::Ok(v1::CommandOk {
            result: Some(Ok_::PreviewConnect(p)),
        })) => assert_eq!(p.rate_bytes_per_second, 8),
        other => panic!("not a connect: {other:?}"),
    }
    c.send_frame(3, kinds::PREVIEW_DATA, b"123456789".to_vec())
        .unwrap();
    let fs = frames(&mut c, 3, is_close);
    assert_eq!(end_of(&fs).reason, v1::PreviewCloseReason::Budget as i32);
}

/// A server that writes one byte value without end on each connection, a new value per connection (`a`, `b`, ...).
fn streamer() -> u32 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = u32::from(l.local_addr().unwrap().port());
    std::thread::spawn(move || {
        for (i, s) in l.incoming().enumerate() {
            let Ok(mut s) = s else { return };
            let byte = b'a' + u8::try_from(i % 26).unwrap();
            std::thread::spawn(move || {
                let block = [byte; 4096];
                while s.write_all(&block).is_ok() {}
            });
        }
    });
    port
}

#[test]
fn a_stream_used_again_right_after_a_close_carries_nothing_of_the_old_connection() {
    let e = env("reuse", "");
    let _n = node(&e);
    let port = streamer();
    let id = opened(&e, port);
    let mut c = client(&e);
    for round in 0..40 {
        let req = CommandRequest {
            request_id: format!("reuse-{round}"),
            command: Some(Cmd::PreviewConnect(v1::PreviewConnect {
                forward_id: id.clone(),
            })),
            ..Default::default()
        };
        if round > 0 {
            // The old connection is still sending: end it and use the stream again at once.
            c.send_frame(3, STREAM_CLOSE, Vec::new()).unwrap();
        }
        c.send_frame(3, 0, req.encode_to_vec()).unwrap();
        // Frames in the order they came: everything after the answer must be the new connection's.
        let mut answered = false;
        let mut after: Vec<u8> = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while after.len() < 4 * 1024 && std::time::Instant::now() < deadline {
            let Ok(Some(f)) = c.read_frame(Some(Duration::from_millis(100))) else {
                continue;
            };
            if (f.stream_id, f.kind) == (STREAM_COMMANDS, 1) {
                let r = CommandResponse::decode(f.payload.as_slice()).unwrap();
                if r.request_id == req.request_id {
                    assert_eq!(code(&r), "ok", "round {round}");
                    answered = true;
                }
            } else if answered && f.stream_id == 3 && f.kind == kinds::PREVIEW_DATA {
                after.extend(f.payload);
            }
        }
        assert!(answered, "round {round}");
        assert!(
            !after.is_empty(),
            "round {round}: the new connection sent nothing"
        );
        assert!(
            after.iter().all(|b| *b == after[0]),
            "round {round}: bytes of two connections on one stream"
        );
    }
}

// ---- through the hub link (step 2b-ii) ----

type Ws = agora_protocol::ws::Conn<tokio::net::TcpStream>;

/// The node's link to a fake hub: the runtime, the hub's side once the handshake is done, and the link (dropped to stop it).
fn hub_link(
    n: &Node,
    e: &Env,
) -> (
    tokio::runtime::Runtime,
    Ws,
    agora_node::uplink_client::UplinkHandle,
) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let l = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let port = l.local_addr().unwrap().port();
    let uplink = spawn(
        Arc::clone(&n.events),
        HubLink {
            url: format!("ws://127.0.0.1:{port}"),
            ca_pem: String::new(),
            cert_pem: String::new(),
            key_pem: String::new(),
            commands: Some(e.dir.join("c.sock")),
            mesh_out: None,
            grants: Some(Arc::clone(&n.grants)),
            mesh: None,
            devices: None,
            state_dir: None,
            certs: None,
            spawner: None,
        },
    )
    .unwrap();
    let ws = rt.block_on(async move {
        let (s, _) = l.accept().await.unwrap();
        let mut c = agora_protocol::ws::accept(s).await.unwrap();
        let hello = c.recv().await.unwrap().unwrap();
        assert_eq!(hello.kind, kinds::HELLO);
        let hello = v1::Hello::decode(hello.payload.as_slice()).unwrap();
        assert!(
            hello.capabilities.iter().any(|c| c == "preview.v1"),
            "the node offers previews"
        );
        let welcome = v1::Welcome {
            chosen_generation: 1,
            ..Default::default()
        };
        c.send(&frame(
            STREAM_CONTROL,
            kinds::WELCOME,
            welcome.encode_to_vec(),
        ))
        .await
        .unwrap();
        while c.recv().await.unwrap().unwrap().kind != kinds::RESUME {}
        let ok = v1::ResumeOk { from_seq: 1 };
        c.send(&frame(STREAM_CONTROL, kinds::RESUME_OK, ok.encode_to_vec()))
            .await
            .unwrap();
        c
    });
    (rt, ws, uplink)
}

/// Frames from the node on `stream` (and the answers on stream 1) until `until` holds or 10 s pass; pings are answered.
async fn hub_frames(c: &mut Ws, stream: u32, until: impl Fn(&[Frame]) -> bool) -> Vec<Frame> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut out = Vec::new();
    while !until(&out) {
        let Ok(Ok(Some(f))) = tokio::time::timeout_at(deadline, c.recv()).await else {
            break;
        };
        if (f.stream_id, f.kind) == (STREAM_CONTROL, kinds::PING) {
            let _ = c.send(&frame(STREAM_CONTROL, kinds::PONG, f.payload)).await;
        } else if f.stream_id == stream || (f.stream_id, f.kind) == (STREAM_COMMANDS, 1) {
            out.push(f);
        }
    }
    out
}

fn connect_req(id: &str, forward_id: &str) -> Vec<u8> {
    CommandRequest {
        request_id: id.into(),
        command: Some(Cmd::PreviewConnect(v1::PreviewConnect {
            forward_id: forward_id.into(),
        })),
        ..Default::default()
    }
    .encode_to_vec()
}

fn answered(fs: &[Frame], id: &str) -> Option<String> {
    fs.iter()
        .filter(|f| (f.stream_id, f.kind) == (STREAM_COMMANDS, 1))
        .map(|f| CommandResponse::decode(f.payload.as_slice()).unwrap())
        .find(|r| r.request_id == id)
        .map(|r| code(&r))
}

#[test]
fn through_the_hub_a_connection_carries_bytes_both_ways_and_a_reused_stream_gets_only_its_own() {
    let e = env("hub-relay", "");
    let n = node(&e);
    let (port, _) = echo();
    let id = opened(&e, port);
    let (rt, mut c, uplink) = hub_link(&n, &e);
    rt.block_on(async {
        c.send(&frame(3, 0, connect_req("c1", &id))).await.unwrap();
        let fs = hub_frames(&mut c, 3, |fs| answered(fs, "c1").is_some()).await;
        assert_eq!(answered(&fs, "c1").as_deref(), Some("ok"));
        c.send(&frame(3, kinds::PREVIEW_DATA, b"hello".to_vec()))
            .await
            .unwrap();
        let fs = hub_frames(&mut c, 3, |fs| data(fs).len() >= 5).await;
        assert_eq!(data(&fs), b"hello");
        // The hub ends it and gives the stream a new connection at once.
        c.send(&frame(3, STREAM_CLOSE, Vec::new())).await.unwrap();
        c.send(&frame(3, 0, connect_req("c2", &id))).await.unwrap();
        let fs = hub_frames(&mut c, 3, |fs| answered(fs, "c2").is_some()).await;
        assert_eq!(answered(&fs, "c2").as_deref(), Some("ok"));
        c.send(&frame(3, kinds::PREVIEW_DATA, b"again".to_vec()))
            .await
            .unwrap();
        let fs = hub_frames(&mut c, 3, |fs| data(fs).len() >= 5).await;
        assert_eq!(
            data(&fs),
            b"again",
            "only the new connection's bytes after its answer"
        );
    });
    drop(uplink);
}

#[test]
fn through_the_hub_data_beyond_the_credit_ends_the_connection() {
    let e = env("hub-credit", "");
    let n = node(&e);
    // A program that never reads: the node's credit is not given back.
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = u32::from(l.local_addr().unwrap().port());
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for s in l.incoming().flatten() {
            held.push(s);
        }
    });
    let id = opened(&e, port);
    let (rt, mut c, uplink) = hub_link(&n, &e);
    rt.block_on(async {
        c.send(&frame(3, 0, connect_req("c1", &id))).await.unwrap();
        let fs = hub_frames(&mut c, 3, |fs| answered(fs, "c1").is_some()).await;
        assert_eq!(answered(&fs, "c1").as_deref(), Some("ok"));
        // Far beyond the window, without waiting for credit.
        for _ in 0..200 {
            c.send(&frame(3, kinds::PREVIEW_DATA, vec![b'x'; 16_384]))
                .await
                .unwrap();
        }
        let fs = hub_frames(&mut c, 3, |fs| fs.iter().any(|f| f.kind == STREAM_CLOSE)).await;
        let end = end_of(&fs);
        assert!(
            end.reason == v1::PreviewCloseReason::Protocol as i32,
            "ended for breaking the credit rules: {end:?}"
        );
    });
    drop(uplink);
}

#[test]
fn a_program_that_never_reads_does_not_hold_a_connection_the_window_ended() {
    let e = env("never-reads", "");
    let n = node(&e);
    // The program neither reads nor writes: only the node's own drain time can end the connection.
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = u32::from(l.local_addr().unwrap().port());
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for s in l.incoming().flatten() {
            held.push(s);
        }
    });
    let id = opened(&e, port);
    let mut c = client(&e);
    assert_eq!(code(&connect(&mut c, 3, &id)), "ok");
    // Send within the credit until the node stops giving it back (it is held in a write to the program), then end it.
    let mut credit: u64 = 262_144;
    let mut quiet_since = std::time::Instant::now();
    while quiet_since.elapsed() < Duration::from_secs(1) {
        if credit >= 16_384 {
            c.send_frame(3, kinds::PREVIEW_DATA, vec![b'x'; 16_384])
                .unwrap();
            credit -= 16_384;
            continue;
        }
        if let Ok(Some(f)) = c.read_frame(Some(Duration::from_millis(50))) {
            if f.stream_id == 3 && f.kind == kinds::PREVIEW_CREDIT {
                credit += u64::from(
                    v1::PreviewCredit::decode(f.payload.as_slice())
                        .unwrap()
                        .bytes,
                );
                quiet_since = std::time::Instant::now();
            }
        }
    }
    assert_eq!(n.previews.relayed(), 1);
    let ended = std::time::Instant::now();
    c.send_frame(3, STREAM_CLOSE, Vec::new()).unwrap();
    while n.previews.relayed() > 0 && ended.elapsed() < Duration::from_secs(15) {
        std::thread::sleep(Duration::from_millis(50));
    }
    let took = ended.elapsed();
    assert_eq!(n.previews.relayed(), 0, "the connection was let go");
    assert!(
        took >= Duration::from_secs(4),
        "what the window sent had its drain time first: {took:?}"
    );
    assert!(took < Duration::from_secs(10), "and not longer: {took:?}");
}

/// ADR-0022 measurement for step 2b-ii (run with `--ignored --nocapture`): the round trip of a command through the hub link, alone
/// and while a preview connection floods the link from a program that writes without end (the hub grants credit as fast as it reads).
/// Preview frames go only when no answer and no terminal frame waits, so the round trip must stay where it is.
#[test]
#[ignore = "a measurement, printed; run by hand"]
fn measure_answers_through_the_hub_while_a_preview_floods_the_link() {
    let e = env("bench", "");
    let n = node(&e);
    let port = streamer();
    let id = opened(&e, port);
    let (rt, mut c, uplink) = hub_link(&n, &e);
    rt.block_on(async {
        async fn round_trips(c: &mut Ws, count: usize, flood: bool) -> (Vec<Duration>, u64) {
            let mut times = Vec::new();
            let mut bytes = 0u64;
            for i in 0..count {
                let id = format!("q{i}-{flood}");
                let req = CommandRequest {
                    request_id: id.clone(),
                    command: Some(Cmd::PreviewList(v1::PreviewList {})),
                    ..Default::default()
                };
                let at = tokio::time::Instant::now();
                c.send(&frame(STREAM_COMMANDS, 0, req.encode_to_vec())).await.unwrap();
                loop {
                    let f = c.recv().await.unwrap().unwrap();
                    if f.stream_id == 3 && f.kind == kinds::PREVIEW_DATA {
                        bytes += f.payload.len() as u64;
                        let credit = v1::PreviewCredit { bytes: u32::try_from(f.payload.len()).unwrap() };
                        c.send(&frame(3, kinds::PREVIEW_CREDIT, credit.encode_to_vec())).await.unwrap();
                    } else if (f.stream_id, f.kind) == (STREAM_CONTROL, kinds::PING) {
                        let _ = c.send(&frame(STREAM_CONTROL, kinds::PONG, f.payload)).await;
                    } else if (f.stream_id, f.kind) == (STREAM_COMMANDS, 1)
                        && CommandResponse::decode(f.payload.as_slice()).unwrap().request_id == id
                    {
                        times.push(at.elapsed());
                        break;
                    }
                }
            }
            (times, bytes)
        }
        fn p(times: &mut [Duration], q: f64) -> f64 {
            times.sort();
            let i = ((times.len() as f64 - 1.0) * q).round() as usize;
            times[i].as_secs_f64() * 1e6
        }
        let (mut alone, _) = round_trips(&mut c, 300, false).await;
        c.send(&frame(3, 0, connect_req("flood", &id))).await.unwrap();
        let started = std::time::Instant::now();
        let (mut flooded, bytes) = round_trips(&mut c, 300, true).await;
        let secs = started.elapsed().as_secs_f64();
        println!(
            "answers alone:      p50 {:.0} us, p95 {:.0} us, max {:.0} us",
            p(&mut alone, 0.5),
            p(&mut alone, 0.95),
            p(&mut alone, 1.0)
        );
        println!(
            "answers in a flood: p50 {:.0} us, p95 {:.0} us, max {:.0} us; preview {:.1} MB/s meanwhile",
            p(&mut flooded, 0.5),
            p(&mut flooded, 0.95),
            p(&mut flooded, 1.0),
            bytes as f64 / secs / 1e6
        );
    });
    drop(uplink);
}

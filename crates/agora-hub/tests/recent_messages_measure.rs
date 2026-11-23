//! Measurement only (ADR-0103, ADR-0022): what the recent-message rings cost a window's subscribe (the view's clone under the hub
//! lock) and its snapshot (built outside the lock), with no messages, with every ring full by count and with all rings at the 8 MiB cap.
//! Run with `cargo test --release -p agora-hub --test recent_messages_measure -- --ignored --nocapture`.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use agora_hub::{
    hub::Hub, scope::Scope, server::snapshot_of, session::NodeSession, store::SqliteStore,
};
use agora_protocol::handshake::Support;
use agora_protocol::v1::{self, event::Body};
use prost::Message;

const PROJECTS: usize = 50;
const RUNS: usize = 300;

fn hub_with(projects: usize, per_project: usize, body_len: usize) -> Hub {
    let hub = Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap();
    let support = Support {
        generations: vec![1],
        capabilities: vec![],
        required: vec![],
    };
    let mut session = NodeSession::new(hub.store(), "n", &support);
    session
        .hello(
            &v1::Hello {
                role: v1::Role::Node as i32,
                protocol_generations: vec![1],
                ..Default::default()
            },
            0,
        )
        .unwrap();
    session.resume(&v1::Resume::default()).unwrap();
    let mut seq = 0;
    let mut send = |body: Body| {
        seq += 1;
        let payload = v1::Event {
            producer_id: "n".into(),
            seq,
            at_ms: seq,
            body: Some(body),
        }
        .encode_to_vec();
        hub.ingest(&mut session, &payload).unwrap();
    };
    for p in 0..projects {
        send(Body::AgentStarted(v1::AgentStarted {
            agent_id: format!("a{p}"),
            project_id: format!("p{p:02}"),
            node_id: "n".into(),
            kind: "k".into(),
            ..Default::default()
        }));
    }
    let body = "x".repeat(body_len);
    for i in 0..per_project {
        for p in 0..projects {
            send(Body::MessagePosted(v1::MessagePosted {
                message_id: format!("m-{i}"),
                from_agent_id: format!("a{p}"),
                to: format!("a{}", (p + 1) % projects),
                body: body.clone(),
            }));
        }
    }
    hub
}

fn pct(samples: &mut [Duration], p: f64) -> f64 {
    samples.sort_unstable();
    let i = ((samples.len() as f64 - 1.0) * p).round() as usize;
    samples[i].as_secs_f64() * 1e6
}

fn measure(name: &str, hub: &Hub) {
    let (mut subscribe, mut owner, mut member) = (Vec::new(), Vec::new(), Vec::new());
    let (mut owner_bytes, mut member_bytes, mut owner_n) = (0, 0, 0);
    for _ in 0..RUNS {
        let t = Instant::now();
        let sub = hub.subscribe(None).unwrap();
        subscribe.push(t.elapsed());
        let (view, pos) = sub.snapshot.unwrap();
        let t = Instant::now();
        let snap = snapshot_of(&view, pos);
        let bytes = snap.encode_to_vec();
        owner.push(t.elapsed());
        owner_bytes = bytes.len();
        owner_n = snap.recent_messages.len();
        let t = Instant::now();
        let granted: BTreeSet<String> = ["n/p07".to_string()].into();
        let s = Scope::from_view(granted, &view);
        let cut = snapshot_of(&s.filter_view(&view), pos).encode_to_vec();
        member.push(t.elapsed());
        member_bytes = cut.len();
    }
    let (view, _) = hub.view();
    println!(
        "MEASURE {name}: rings {} KiB in {} projects; subscribe (clone under the lock) p50 {:.1} us p95 {:.1} us; \
         owner snapshot_of+encode p50 {:.1} us p95 {:.1} us ({owner_n} messages, {owner_bytes} bytes); \
         member cut+snapshot_of+encode p50 {:.1} us p95 {:.1} us ({member_bytes} bytes)",
        view.messages.bytes / 1024,
        view.messages.projects.len(),
        pct(&mut subscribe, 0.5),
        pct(&mut subscribe, 0.95),
        pct(&mut owner, 0.5),
        pct(&mut owner, 0.95),
        pct(&mut member, 0.5),
        pct(&mut member, 0.95),
    );
}

#[test]
#[ignore = "measurement"]
fn subscribe_and_snapshot_with_empty_and_full_rings() {
    measure("no messages", &hub_with(PROJECTS, 0, 0));
    measure(
        "50 projects x 50 messages of 200 bytes",
        &hub_with(PROJECTS, 50, 200),
    );
    measure(
        "40 projects x 15 messages of 16 KiB (8 MiB cap)",
        &hub_with(40, 15, 16 * 1024),
    );
}

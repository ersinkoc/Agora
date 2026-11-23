//! Measurement only (ADR-0103, ADR-0022): what the recent-message rings cost the hub. Under the hub lock: the view's clone (what a
//! subscribe takes, `Hub::view`), and the ingest of one more message with every ring full, with no snapshot held and with one held
//! (then the rings are copied once, as pointers). Outside the lock: the owner's and a member's snapshot, built and encoded. With no
//! messages, with every ring full by count, and with all rings at the 8 MiB cap.
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

const RUNS: usize = 300;

fn pct(samples: &mut [Duration], p: f64) -> f64 {
    samples.sort_unstable();
    let i = ((samples.len() as f64 - 1.0) * p).round() as usize;
    samples[i].as_secs_f64() * 1e6
}

fn line(samples: &mut [Duration]) -> String {
    format!(
        "p50 {:.1} us p95 {:.1} us",
        pct(samples, 0.5),
        pct(samples, 0.95)
    )
}

fn measure(name: &str, projects: usize, per_project: usize, body_len: usize) {
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
    let body = "x".repeat(body_len.max(1));
    let mut payload = |body: Body| {
        seq += 1;
        v1::Event {
            producer_id: "n".into(),
            seq,
            at_ms: seq,
            body: Some(body),
        }
        .encode_to_vec()
    };
    let message = |p: usize, i: usize| {
        Body::MessagePosted(v1::MessagePosted {
            message_id: format!("m-{i}"),
            from_agent_id: format!("a{p}"),
            to: format!("a{}", (p + 1) % projects),
            body: body.clone(),
        })
    };
    for p in 0..projects {
        let e = payload(Body::AgentStarted(v1::AgentStarted {
            agent_id: format!("a{p}"),
            project_id: format!("p{p:02}"),
            node_id: "n".into(),
            kind: "k".into(),
            ..Default::default()
        }));
        hub.ingest(&mut session, &e).unwrap();
    }
    for i in 0..per_project {
        for p in 0..projects {
            let e = payload(message(p, i));
            hub.ingest(&mut session, &e).unwrap();
        }
    }

    let (mut clone, mut owner, mut member) = (Vec::new(), Vec::new(), Vec::new());
    let (mut owner_bytes, mut member_bytes, mut owner_n) = (0, 0, 0);
    for _ in 0..RUNS {
        let t = Instant::now();
        let (view, pos) = hub.view();
        clone.push(t.elapsed());
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
    // One more message per run (the stored payload is prepared first); `held` keeps a snapshot's view alive meanwhile, as a window
    // did before it dropped the snapshot once sent.
    let (mut free, mut held) = (Vec::new(), Vec::new());
    for i in 0..RUNS {
        let e = payload(message(i % projects.max(1), i));
        let t = Instant::now();
        hub.ingest(&mut session, &e).unwrap();
        free.push(t.elapsed());
        let kept = hub.view();
        let e = payload(message(i % projects.max(1), i));
        let t = Instant::now();
        hub.ingest(&mut session, &e).unwrap();
        held.push(t.elapsed());
        drop(kept);
    }
    let (view, _) = hub.view();
    println!(
        "MEASURE {name}: rings {} KiB in {} projects\n  view clone under the lock (subscribe): {}\n  ingest one message: {}; with a \
         snapshot held: {}\n  owner snapshot_of+encode: {} ({owner_n} messages, {owner_bytes} bytes)\n  member cut+snapshot_of+encode: {} \
         ({member_bytes} bytes)",
        view.messages.bytes / 1024,
        view.messages.projects.len(),
        line(&mut clone),
        line(&mut free),
        line(&mut held),
        line(&mut owner),
        line(&mut member),
    );
}

#[test]
#[ignore = "measurement"]
fn subscribe_and_snapshot_with_empty_and_full_rings() {
    measure("no messages (50 agents)", 50, 0, 0);
    measure("50 projects x 50 messages of 200 bytes", 50, 50, 200);
    measure(
        "40 projects x 15 messages of 16 KiB (8 MiB cap)",
        40,
        15,
        16 * 1024,
    );
}

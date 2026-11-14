#![allow(clippy::unwrap_used, clippy::expect_used)]
use agora_hub::{
    hub::{Hub, MAX_REPLAY_BYTES, MAX_REPLAY_EVENTS},
    session::NodeSession,
    store::SqliteStore,
};
use agora_protocol::{
    handshake::Support,
    v1::{self as v1, event::Body},
};
use prost::Message;
fn session<'a>(hub: &'a Hub, support: &'a Support) -> NodeSession<'a, SqliteStore> {
    let mut s = NodeSession::new(hub.store(), "n", support);
    s.hello(
        &v1::Hello {
            role: v1::Role::Node as i32,
            protocol_generations: vec![1],
            ..Default::default()
        },
        0,
    )
    .unwrap();
    s.resume(&v1::Resume::default()).unwrap();
    s
}
fn sized(seq: u64, target: usize) -> Vec<u8> {
    let mut paths: Vec<String> = (0..256)
        .map(|i| format!("src/{i:04}/{}.rs", "x".repeat(240)))
        .collect();
    // Keep all components legal. Extra padding is a directory separator and a short component.
    paths.last_mut().unwrap().push('/');
    loop {
        let event = v1::Event {
            producer_id: "n".into(),
            seq,
            at_ms: seq,
            body: Some(Body::FilesChanged(v1::FilesChanged {
                worktree_id: "w".into(),
                paths: paths.clone(),
            })),
        }
        .encode_to_vec();
        if event.len() == target {
            return event;
        }
        assert!(event.len() < target);
        paths
            .last_mut()
            .unwrap()
            .extend(std::iter::repeat_n('y', target - event.len()));
    }
}
fn small(seq: u64) -> Vec<u8> {
    v1::Event {
        producer_id: "n".into(),
        seq,
        at_ms: seq,
        body: Some(Body::FilesChanged(v1::FilesChanged::default())),
    }
    .encode_to_vec()
}
#[test]
fn review_exact_byte_boundary_suffix_and_live_delivery() {
    for extra in [0, 1] {
        let hub = Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap();
        let support = Support {
            generations: vec![1],
            capabilities: vec![],
            required: vec![],
        };
        let mut s = session(&hub, &support);
        for seq in 1..=128 {
            let size = 65536 + if seq == 128 { extra } else { 0 };
            hub.ingest(&mut s, &sized(seq, size)).unwrap();
        }
        assert_eq!(
            hub.store().payload_bytes_after(0).unwrap(),
            (MAX_REPLAY_BYTES + extra) as u64
        );
        let sub = hub.subscribe(Some(0)).unwrap();
        if extra == 0 {
            assert!(sub.snapshot.is_none());
            assert_eq!(sub.backlog.len(), 128);
        } else {
            assert_eq!(sub.snapshot.as_ref().map(|(_, p)| *p), Some(128));
            assert!(sub.backlog.is_empty());
        }
        // Prefix payload must not count against a later resume position.
        let suffix = hub.subscribe(Some(1)).unwrap();
        assert!(suffix.snapshot.is_none());
        assert_eq!(suffix.backlog.len(), 127);
        assert_eq!(suffix.backlog.first().unwrap().pos, 2);
        // Both replay and snapshot subscriptions transition to the same next live position exactly once.
        hub.ingest(&mut s, &small(129)).unwrap();
        assert_eq!(sub.live.try_recv().unwrap().pos, 129);
        assert!(sub.live.try_recv().is_err());
        assert_eq!(suffix.live.try_recv().unwrap().pos, 129);
        assert!(suffix.live.try_recv().is_err());
    }
}
#[test]
fn review_exact_count_boundary_and_ahead_of_head() {
    let hub = Hub::open(SqliteStore::open_in_memory().unwrap()).unwrap();
    let support = Support {
        generations: vec![1],
        capabilities: vec![],
        required: vec![],
    };
    let mut s = session(&hub, &support);
    for seq in 1..=MAX_REPLAY_EVENTS {
        hub.ingest(&mut s, &small(seq)).unwrap();
    }
    let exact = hub.subscribe(Some(0)).unwrap();
    assert!(exact.snapshot.is_none());
    assert_eq!(exact.backlog.len(), MAX_REPLAY_EVENTS as usize);
    assert!(hub.subscribe(Some(u64::MAX)).unwrap().snapshot.is_some());
    let tail = hub.subscribe(Some(MAX_REPLAY_EVENTS)).unwrap();
    assert!(tail.snapshot.is_none() && tail.backlog.is_empty());
    hub.ingest(&mut s, &small(MAX_REPLAY_EVENTS + 1)).unwrap();
    let far = hub.subscribe(Some(0)).unwrap();
    assert!(far.snapshot.is_some() && far.backlog.is_empty());
    assert_eq!(exact.live.try_recv().unwrap().pos, MAX_REPLAY_EVENTS + 1);
    assert_eq!(tail.live.try_recv().unwrap().pos, MAX_REPLAY_EVENTS + 1);
}

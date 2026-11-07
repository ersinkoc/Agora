//! A node's events reach a hub's store and view through the two state machines (the node's `Uplink` and the hub's
//! `NodeSession`), with the messages passed in memory. Transport and framing are tested where they live; this is
//! the protocol between them: resume, replay, no duplicates, no gaps, and two machines in one view.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::Arc;

use agora_core::AgentId;
use agora_hub::projection::View;
use agora_hub::session::{Ingest, NodeSession};
use agora_hub::store::{EventStore, SqliteStore};
use agora_node::daemon::EventSink;
use agora_node::uplink::{Uplink, UplinkError};
use agora_protocol::handshake::Support;
use agora_protocol::v1::AgentState;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("agora-link-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn support() -> Support {
    Support {
        generations: vec![1],
        capabilities: vec!["terminal.raw".into()],
        required: vec![],
    }
}

/// A node with a checkout and one running agent: 6 events.
fn node(dir: &std::path::Path, id: &str) -> Arc<EventSink> {
    let sink = Arc::new(EventSink::open(dir, id).unwrap());
    sink.node_online(id);
    sink.checkout_discovered(id, "proj", std::path::Path::new("/work/proj"));
    sink.agent_launched(
        &AgentId::new(format!("agent-{id}")),
        "proj",
        id,
        std::path::Path::new("/work/proj"),
        std::path::Path::new("/work/wt"),
        "agent/x",
        "shell",
    );
    sink
}

/// One connection: handshake, then send up to `limit` events (None = everything), acking as the hub does.
/// Returns how many events the hub applied.
fn connect(
    store: &SqliteStore,
    sink: &Arc<EventSink>,
    node_id: &str,
    limit: Option<usize>,
) -> Result<usize, UplinkError> {
    let sup = support();
    let mut hub = NodeSession::new(store, node_id, &sup);
    let mut up = Uplink::new(Arc::clone(sink), "test", &["terminal.raw"]);

    let hello = up.hello()?;
    let welcome = hub.hello(&hello, 0).unwrap();
    up.on_welcome(&welcome)?;
    let ok = hub.resume(&up.resume()?).unwrap();
    up.on_resume_ok(&ok)?;

    let mut applied = 0;
    'send: loop {
        let batch = up.next_batch(2)?;
        if batch.is_empty() {
            break;
        }
        for payload in batch {
            if limit.is_some_and(|l| applied >= l) {
                break 'send; // the connection drops here
            }
            match hub.event(&payload).unwrap() {
                Ingest::Applied { acked, .. } => {
                    applied += 1;
                    up.on_ack(acked)?;
                }
                Ingest::Duplicate { acked } => up.on_ack(acked)?,
                Ingest::Gap { expected } => up.on_gap(expected)?,
            }
        }
    }
    Ok(applied)
}

#[test]
fn all_events_arrive_in_order_and_the_node_trims_its_buffer() {
    let store = SqliteStore::open_in_memory().unwrap();
    let sink = node(&tmp("all"), "n1");
    let total = sink.next_seq() - 1;
    assert_eq!(connect(&store, &sink, "n1", None).unwrap() as u64, total);
    assert_eq!(store.acked("n1").unwrap(), total);
    assert_eq!(sink.acked(), total, "the node learned what the hub holds");
    // A second connection has nothing left to send.
    assert_eq!(connect(&store, &sink, "n1", None).unwrap(), 0);
}

#[test]
fn a_dropped_connection_resumes_without_duplicates_or_gaps() {
    let store = SqliteStore::open_in_memory().unwrap();
    let sink = node(&tmp("drop"), "n1");
    let total = sink.next_seq() - 1;
    assert_eq!(connect(&store, &sink, "n1", Some(3)).unwrap(), 3);
    // More happens while disconnected: nodes keep working without the hub.
    sink.node_resources("n1", 1, 1);
    let rest = connect(&store, &sink, "n1", None).unwrap() as u64;
    assert_eq!(rest, total + 1 - 3);
    let seqs: Vec<u64> = store
        .events_after(0, 100)
        .unwrap()
        .iter()
        .map(|e| e.seq)
        .collect();
    assert_eq!(seqs, (1..=total + 1).collect::<Vec<_>>());
}

#[test]
fn a_hub_restored_from_an_older_backup_gets_the_gap_replayed() {
    let dir = tmp("restore");
    std::fs::create_dir_all(&dir).unwrap();
    let live = dir.join("hub.db");
    let sink = node(&dir.join("n"), "n1");
    let total = sink.next_seq() - 1;
    {
        let store = SqliteStore::open(&live).unwrap();
        connect(&store, &sink, "n1", Some(2)).unwrap();
        store.backup_to(&dir.join("backup.db")).unwrap();
        connect(&store, &sink, "n1", None).unwrap();
        assert_eq!(store.acked("n1").unwrap(), total);
    }
    // Disaster: the hub comes back from the backup, which only has 2 events.
    agora_hub::store::restore(&dir.join("backup.db"), &live).unwrap();
    let store = SqliteStore::open(&live).unwrap();
    assert_eq!(store.acked("n1").unwrap(), 2);
    // The node had trimmed its buffer past 2 when the hub acked everything, so the old events are gone: the
    // uplink must say so rather than send a gap (the hub then resynchronizes from a snapshot).
    match connect(&store, &sink, "n1", None) {
        Err(UplinkError::Stale { oldest_retained }) => assert!(oldest_retained > 3),
        Ok(n) => assert_eq!(n as u64, total - 2, "buffer still held the gap"),
        Err(e) => panic!("{e:?}"),
    }
}

#[test]
fn a_node_that_lost_old_events_to_its_size_cap_says_stale() {
    let dir = tmp("stale");
    // A cap so small that early events are evicted before the hub ever saw them.
    let sink = Arc::new(EventSink::open_with_cap(&dir, "n1", 600).unwrap());
    for i in 0..40 {
        sink.node_resources("n1", i, i);
    }
    assert!(sink.oldest_retained() > 1);
    let store = SqliteStore::open_in_memory().unwrap();
    match connect(&store, &sink, "n1", None) {
        Err(UplinkError::Stale { oldest_retained }) => {
            assert_eq!(oldest_retained, sink.oldest_retained());
        }
        other => panic!("expected Stale, got {other:?}"),
    }
}

#[test]
fn two_machines_show_up_in_one_view() {
    let store = SqliteStore::open_in_memory().unwrap();
    let (a, b) = (node(&tmp("ma"), "laptop"), node(&tmp("mb"), "server"));
    connect(&store, &a, "laptop", None).unwrap();
    connect(&store, &b, "server", None).unwrap();
    let (view, _) = View::rebuild(&store).unwrap();
    assert_eq!(view.nodes.len(), 2);
    assert!(view.nodes.values().all(|n| n.online));
    assert_eq!(view.agents.len(), 2);
    assert_eq!(view.worktrees.len(), 2);
    assert_eq!(view.terminals.len(), 2);
    assert_eq!(view.agents["laptop/agent-laptop"].node_id, "laptop");
    assert_eq!(view.agents["server/agent-server"].state, AgentState::Idle);
}

#[test]
fn messages_out_of_order_are_refused_on_the_node_side_too() {
    let sink = node(&tmp("order"), "n1");
    let mut up = Uplink::new(sink, "t", &[]);
    assert_eq!(up.next_batch(1).unwrap_err(), UplinkError::OutOfOrder);
    assert_eq!(up.resume().unwrap_err(), UplinkError::OutOfOrder);
    up.hello().unwrap();
    assert_eq!(up.hello().unwrap_err(), UplinkError::OutOfOrder);
    let bad = agora_protocol::v1::Welcome {
        chosen_generation: 9,
        ..Default::default()
    };
    assert_eq!(
        up.on_welcome(&bad).unwrap_err(),
        UplinkError::BadGeneration(9)
    );
}

//! A node's events reach a hub's store and view through the two state machines (the node's `Uplink` and the hub's
//! `NodeSession`), with the messages passed in memory. Transport and framing are tested where they live; this is
//! the protocol between them: resume, replay, no duplicates, no gaps, a resync after lost events (ADR-0054), and two machines
//! in one view.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::Arc;

use agora_core::AgentId;
use agora_hub::projection::View;
use agora_hub::session::{Ingest, NodeSession};
use agora_hub::store::{EventStore, SqliteStore};
use agora_node::daemon::EventSink;
use agora_node::uplink::{Replay, Uplink, UplinkError};
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
        true,
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
    // A node that no longer has what the hub asks for says so before sending more (ADR-0054), as `uplink_client` does.
    if let Replay::Resync { .. } = up.on_resume_ok(&ok)? {
        hub.snapshot_required(0).unwrap();
    }

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
                Ingest::Applied { acked, .. } | Ingest::Resynced { acked, .. } => {
                    applied += 1;
                    up.on_ack(acked)?;
                }
                Ingest::Duplicate { acked } => up.on_ack(acked)?,
                Ingest::Gap { expected } => {
                    if let Replay::Resync { .. } = up.on_gap(expected)? {
                        hub.snapshot_required(0).unwrap();
                    }
                }
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
    sink.node_resources("n1", 1, 1, 0);
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
    {
        let store = SqliteStore::open(&live).unwrap();
        connect(&store, &sink, "n1", Some(2)).unwrap();
        store.backup_to(&dir.join("backup.db")).unwrap();
        // Enough happens after the backup that, once acknowledged, the node's buffer is compacted past it.
        let long = std::path::PathBuf::from(format!("/work/{}", "x".repeat(1000)));
        for _ in 0..80 {
            sink.checkout_discovered("n1", "proj", &long);
        }
        connect(&store, &sink, "n1", None).unwrap();
        assert_eq!(store.acked("n1").unwrap(), sink.next_seq() - 1);
    }
    // Disaster: the hub comes back from the backup, which only has 2 events.
    agora_hub::store::restore(&dir.join("backup.db"), &live).unwrap();
    let store = SqliteStore::open(&live).unwrap();
    assert_eq!(store.acked("n1").unwrap(), 2);
    let oldest = sink.oldest_retained();
    assert!(
        oldest > 3,
        "the node trimmed its buffer past 2 when the hub acked everything"
    );
    // The old events are gone from the node too: it resynchronizes (ADR-0054). Its present goes at the tail of its buffer, the hub
    // takes the oldest event it still has as the base and records the numbers before it as lost, and from then on everything
    // arrives. (This bare buffer has no announcer; agora-node's `a_resync_announces_the_nodes_present` covers what a real node says.)
    let delivered = connect(&store, &sink, "n1", None).unwrap() as u64;
    assert_eq!(delivered, sink.next_seq() - oldest);
    assert_eq!(store.acked("n1").unwrap(), sink.next_seq() - 1);
    assert_eq!(
        store.resyncs().unwrap(),
        [("n1".to_string(), 3, oldest - 1, 0)]
    );
    sink.node_resources("n1", 1, 1, 0);
    assert_eq!(connect(&store, &sink, "n1", None).unwrap(), 1);
    assert_eq!(store.resyncs().unwrap().len(), 1, "once");
}

/// F080 / ADR-0054: once the size cap dropped events the hub never acknowledged, the node used to end every connection in `Stale`
/// and the hub never heard from it again. Now it resynchronizes: the hub gets everything the node still has and all newer events,
/// and records the lost range.
#[test]
fn a_node_whose_cap_dropped_unacked_events_resyncs_and_delivers_its_newer_events() {
    let sink = Arc::new(EventSink::open_with_cap(&tmp("f080"), "n1", 1200).unwrap());
    for i in 0..5 {
        sink.node_resources("n1", i, i, 0);
    }
    let store = SqliteStore::open_in_memory().unwrap();
    assert_eq!(connect(&store, &sink, "n1", Some(3)).unwrap(), 3);
    // Offline for a while: the node keeps recording, and the cap drops events the hub never saw.
    for i in 0..60 {
        sink.node_resources("n1", 100 + i, i, 0);
    }
    let oldest = sink.oldest_retained();
    assert!(oldest > 4, "events 4.. were dropped unacknowledged");
    let delivered = connect(&store, &sink, "n1", None).unwrap();
    assert_eq!(delivered as u64, sink.next_seq() - oldest);
    assert_eq!(store.acked("n1").unwrap(), sink.next_seq() - 1);
    assert_eq!(
        store.resyncs().unwrap(),
        [("n1".to_string(), 4, oldest - 1, 0)]
    );
    // From then on the link is ordinary: newer events arrive, nothing more is skipped.
    sink.node_resources("n1", 1000, 0, 0);
    assert_eq!(connect(&store, &sink, "n1", None).unwrap(), 1);
    assert_eq!(store.acked("n1").unwrap(), sink.next_seq() - 1);
    assert_eq!(store.resyncs().unwrap().len(), 1);
}

/// A gap answer that arrives mid-session and asks for what the node no longer has resynchronizes too; a late second answer for
/// the same point (sent by the hub before it read `SnapshotRequired`) does not announce again.
#[test]
fn a_gap_answer_for_lost_events_resyncs_once_mid_session() {
    let dir = tmp("gap-resync");
    let sink = Arc::new(EventSink::open_with_cap(&dir, "n1", 600).unwrap());
    sink.set_announcer(Box::new(|ev| ev.node_online("n1")));
    sink.node_resources("n1", 0, 0, 0);
    let store = SqliteStore::open_in_memory().unwrap();
    connect(&store, &sink, "n1", None).unwrap();
    for i in 0..40 {
        sink.node_resources("n1", i, i, 0);
    }
    let oldest = sink.oldest_retained();
    assert!(oldest > 2);
    let sup = support();
    let mut hub = NodeSession::new(&store, "n1", &sup);
    let mut up = Uplink::new(Arc::clone(&sink), "test", &[]);
    let hello = up.hello().unwrap();
    up.on_welcome(&hub.hello(&hello, 0).unwrap()).unwrap();
    // The hub says 2 (its ack + 1); the node starts from what it has, as if it believed the hub were further on.
    let ok = hub.resume(&up.resume().unwrap()).unwrap();
    assert_eq!(ok.from_seq, 2);
    let start = agora_protocol::v1::ResumeOk { from_seq: oldest };
    assert_eq!(up.on_resume_ok(&start).unwrap(), Replay::Resumed);
    let first = up.next_batch(1).unwrap();
    assert_eq!(
        hub.event(&first[0]).unwrap(),
        Ingest::Gap { expected: 2 },
        "the hub wants 2"
    );
    // That gap answer asks for what is gone: the node resynchronizes, once.
    let before = sink.next_seq();
    assert_eq!(
        up.on_gap(2).unwrap(),
        Replay::Resync {
            from_seq: 2,
            // Read after the announcement, which may push more out under the cap.
            oldest_retained: sink.oldest_retained()
        }
    );
    let after_first = sink.next_seq();
    assert!(
        after_first > before,
        "the present was announced at the tail"
    );
    assert_eq!(up.on_gap(2).unwrap(), Replay::Resumed, "a late answer");
    assert_eq!(sink.next_seq(), after_first, "announced once");
    hub.snapshot_required(0).unwrap();
    for p in up.next_batch(1000).unwrap() {
        hub.event(&p).unwrap();
    }
    assert_eq!(store.acked("n1").unwrap(), sink.next_seq() - 1);
    // Only one event was in flight before the resync, so only one late answer could come; another means the hub ignored
    // `SnapshotRequired` (a hub from before ADR-0054), and the connection ends instead of looping.
    assert!(matches!(up.on_gap(2), Err(UplinkError::Stale { .. })));
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

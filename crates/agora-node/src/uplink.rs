//! The node's side of its connection to the hub, as a state machine over decoded messages (the mirror of the hub's
//! `NodeSession`). No sockets and no framing: the transport (WSS with the node's client certificate, or SSH stdio)
//! carries the messages. Nodes are autonomous (AGENTS.md): nothing here is needed for agents to run, and everything
//! the node reports is already safe in the durable event buffer before it is ever sent.
//!
//! Order: `hello` → (peer's `Welcome`) → `resume` → (peer's `ResumeOk`) → `next_batch`* while the connection lasts.
//! The hub decides where replay starts (`ResumeOk.from_seq`); the node only checks it still has those events. When it no longer
//! does (the size cap dropped them, or the hub was restored from an older backup), the node resynchronizes (ADR-0054): it
//! re-states its present at the tail of its buffer, tells the hub with `SnapshotRequired` and sends what it still has.

use std::sync::Arc;

use agora_protocol::v1::{Hello, Resume, ResumeOk, Role, Welcome};

use crate::daemon::EventSink;
use crate::event_buffer::ReplayError;

/// What this build speaks (docs/specs/protocol.md section 1).
pub const GENERATION: u32 = 1;

#[derive(Debug, PartialEq, Eq)]
pub enum UplinkError {
    /// A message arrived that does not fit the current step.
    OutOfOrder,
    /// The hub chose a protocol generation this node did not offer.
    BadGeneration(u32),
    /// The node cannot send what the hub needs and cannot resynchronize on this connection: events it was about to send were
    /// dropped by the size cap while it was sending, or the hub keeps asking for lost events after `SnapshotRequired` (a hub from
    /// before ADR-0054 ignores it). The connection ends; the next one resynchronizes ([`Replay::Resync`]). `oldest_retained` is
    /// the first event the node still has.
    Stale {
        oldest_retained: u64,
    },
    Io(String),
}

/// How sending continues after the hub said where to start (`ResumeOk`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum Replay {
    /// The node still has everything from there: send as usual.
    Resumed,
    /// The node no longer has `from_seq..oldest_retained` (ADR-0054). It re-stated its present at the tail of its buffer; the
    /// caller sends `SnapshotRequired` (control stream, kind 4, empty) before any further event, then sends from `oldest_retained`.
    Resync { from_seq: u64, oldest_retained: u64 },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    New,
    HelloSent,
    Negotiated,
    Streaming,
}

pub struct Uplink {
    sink: Arc<EventSink>,
    build: String,
    capabilities: Vec<String>,
    phase: Phase,
    /// Last sequence number handed out for sending on this connection.
    cursor: u64,
    /// Events handed out for sending on this connection, rewinds included.
    handed_out: u64,
    /// The `from_seq` this connection last resynchronized for. A later answer at or below it was sent by the hub before it read
    /// our `SnapshotRequired` (its numbers only move forward), so it needs no second announcement.
    resynced_from: Option<u64>,
    /// How many such late answers can still come: at most one per event handed out before the resync. A hub that sends more has
    /// ignored `SnapshotRequired`, and the connection ends instead of replaying the same events forever.
    late_answers: u64,
}

impl Uplink {
    pub fn new(sink: Arc<EventSink>, build: &str, capabilities: &[&str]) -> Self {
        Self {
            sink,
            build: build.to_string(),
            capabilities: capabilities.iter().map(|c| (*c).to_string()).collect(),
            phase: Phase::New,
            cursor: 0,
            handed_out: 0,
            resynced_from: None,
            late_answers: 0,
        }
    }

    pub fn hello(&mut self) -> Result<Hello, UplinkError> {
        if self.phase != Phase::New {
            return Err(UplinkError::OutOfOrder);
        }
        self.phase = Phase::HelloSent;
        Ok(Hello {
            role: Role::Node as i32,
            protocol_generations: vec![GENERATION],
            capabilities: self.capabilities.clone(),
            build: self.build.clone(),
            locale: None,
        })
    }

    pub fn on_welcome(&mut self, welcome: &Welcome) -> Result<(), UplinkError> {
        if self.phase != Phase::HelloSent {
            return Err(UplinkError::OutOfOrder);
        }
        if welcome.chosen_generation != GENERATION {
            return Err(UplinkError::BadGeneration(welcome.chosen_generation));
        }
        self.phase = Phase::Negotiated;
        Ok(())
    }

    /// What the node believes the hub has acknowledged; the hub's own record decides, this is informational.
    pub fn resume(&self) -> Result<Resume, UplinkError> {
        if self.phase != Phase::Negotiated {
            return Err(UplinkError::OutOfOrder);
        }
        Ok(Resume {
            last_acked_seq: Some(self.sink.acked()),
        })
    }

    /// The hub says where to start. Everything before `from_seq` is, by the hub's own record, safely stored.
    pub fn on_resume_ok(&mut self, ok: &ResumeOk) -> Result<Replay, UplinkError> {
        if self.phase != Phase::Negotiated {
            return Err(UplinkError::OutOfOrder);
        }
        // The hub is ahead of everything this node has (it acknowledged those numbers for an earlier life of this producer):
        // events below `from_seq` would be dropped as duplicates, so number on from there and announce the node again.
        if ok.from_seq > self.sink.next_seq() {
            self.sink
                .catch_up_with_hub(ok.from_seq)
                .map_err(|e| UplinkError::Io(e.to_string()))?;
        }
        let replay = self.rewind_to(ok.from_seq)?;
        self.phase = Phase::Streaming;
        Ok(replay)
    }

    /// The hub reported a gap (it expected `expected`, answered with `ResumeOk`): send again from there.
    pub fn on_gap(&mut self, expected: u64) -> Result<Replay, UplinkError> {
        if self.phase != Phase::Streaming {
            return Err(UplinkError::OutOfOrder);
        }
        self.rewind_to(expected)
    }

    fn rewind_to(&mut self, from_seq: u64) -> Result<Replay, UplinkError> {
        if from_seq >= self.sink.oldest_retained() {
            self.cursor = from_seq.saturating_sub(1);
            return Ok(Replay::Resumed);
        }
        if self.resynced_from.is_some_and(|r| from_seq <= r) {
            let oldest = self.sink.oldest_retained();
            if self.late_answers == 0 {
                return Err(UplinkError::Stale {
                    oldest_retained: oldest,
                });
            }
            // A late answer to what we sent before resynchronizing: send again from the oldest event we still have.
            self.late_answers -= 1;
            self.cursor = oldest.saturating_sub(1);
            return Ok(Replay::Resumed);
        }
        // Gone: say what is true now at the tail, then send everything still held. The announcement may push older events
        // out under the cap, so the oldest is read after it.
        self.sink.announce();
        let oldest = self.sink.oldest_retained();
        self.cursor = oldest.saturating_sub(1);
        self.resynced_from = Some(from_seq);
        self.late_answers = self.handed_out;
        Ok(Replay::Resync {
            from_seq,
            oldest_retained: oldest,
        })
    }

    /// Up to `max` encoded events not yet sent on this connection, oldest first. Empty when caught up.
    pub fn next_batch(&mut self, max: usize) -> Result<Vec<Vec<u8>>, UplinkError> {
        if self.phase != Phase::Streaming {
            return Err(UplinkError::OutOfOrder);
        }
        let mut batch = match self.sink.payloads_after(self.cursor) {
            Ok(b) => b,
            Err(ReplayError::Stale { oldest_retained }) => {
                return Err(UplinkError::Stale { oldest_retained })
            }
            Err(ReplayError::Io(e)) => return Err(UplinkError::Io(e.to_string())),
        };
        batch.truncate(max);
        self.cursor += batch.len() as u64;
        self.handed_out += batch.len() as u64;
        Ok(batch)
    }

    /// The hub acknowledged everything up to `up_to_seq` (ADR-0026 `EventAck`): the buffer may drop it.
    pub fn on_ack(&self, up_to_seq: u64) -> Result<(), UplinkError> {
        self.sink
            .ack(up_to_seq)
            .map_err(|e| UplinkError::Io(e.to_string()))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use agora_protocol::v1::{event::Body, Event};
    use prost::Message;

    fn sink(name: &str, cap: u64) -> Arc<EventSink> {
        let dir = std::env::temp_dir().join(format!("agora-uplink-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Arc::new(EventSink::open_with_cap(&dir, "n1", cap).unwrap());
        s.set_announcer(Box::new(|ev| ev.node_online("n1")));
        s
    }

    fn streaming(sink: &Arc<EventSink>, from_seq: u64) -> (Uplink, Replay) {
        let mut up = Uplink::new(Arc::clone(sink), "t", &[]);
        up.hello().unwrap();
        up.on_welcome(&Welcome {
            chosen_generation: GENERATION,
            ..Default::default()
        })
        .unwrap();
        let replay = up.on_resume_ok(&ResumeOk { from_seq }).unwrap();
        (up, replay)
    }

    fn decoded(batch: &[Vec<u8>]) -> Vec<Event> {
        batch
            .iter()
            .map(|b| Event::decode(b.as_slice()).unwrap())
            .collect()
    }

    #[test]
    fn a_resume_the_node_can_serve_is_unchanged() {
        let s = sink("resume", 1 << 20);
        for i in 0..5 {
            s.node_resources("n1", i, i, 0);
        }
        let (mut up, replay) = streaming(&s, 3);
        assert_eq!(replay, Replay::Resumed);
        let sent = decoded(&up.next_batch(100).unwrap());
        assert_eq!(sent.iter().map(|e| e.seq).collect::<Vec<_>>(), [3, 4, 5]);
        assert_eq!(s.next_seq(), 6, "nothing was announced");
        assert_eq!(up.on_gap(4).unwrap(), Replay::Resumed);
        assert_eq!(decoded(&up.next_batch(100).unwrap())[0].seq, 4);
    }

    /// ADR-0054: what used to end every connection in `Stale` now resynchronizes: the present is announced at the tail and
    /// sending starts at the oldest event still held.
    #[test]
    fn a_resume_from_dropped_events_resyncs_with_the_announcement_at_the_tail() {
        let s = sink("resync", 600);
        for i in 0..40 {
            s.node_resources("n1", i, i, 0);
        }
        let before = s.next_seq();
        assert!(s.oldest_retained() > 2);
        let (mut up, replay) = streaming(&s, 2);
        let oldest = s.oldest_retained();
        assert_eq!(
            replay,
            Replay::Resync {
                from_seq: 2,
                oldest_retained: oldest
            }
        );
        assert_eq!(s.next_seq(), before + 1, "one announcement, new numbers");
        let sent = decoded(&up.next_batch(1000).unwrap());
        assert_eq!(sent[0].seq, oldest, "the cursor sat at oldest_retained - 1");
        let last = sent.last().unwrap();
        assert_eq!(last.seq, before);
        assert!(matches!(last.body, Some(Body::NodeOnline(_))));
        assert!(sent.windows(2).all(|w| w[1].seq == w[0].seq + 1));
    }

    /// A hub from before ADR-0054 ignores `SnapshotRequired` and keeps asking for the lost events: the connection ends as `Stale`
    /// (and is tried again later) instead of replaying the same events forever.
    #[test]
    fn a_hub_that_ignores_snapshot_required_ends_the_connection() {
        let s = sink("oldhub", 600);
        for i in 0..40 {
            s.node_resources("n1", i, i, 0);
        }
        let (mut up, replay) = streaming(&s, 2);
        assert!(matches!(replay, Replay::Resync { .. }));
        let next = s.next_seq();
        assert!(!up.next_batch(1).unwrap().is_empty());
        // The old hub answers that event with a gap for 2 again; nothing was in flight before the resync.
        assert!(matches!(up.on_gap(2), Err(UplinkError::Stale { .. })));
        assert_eq!(s.next_seq(), next, "no second announcement");
    }

    #[test]
    fn without_an_announcer_a_resync_still_sends_what_is_held() {
        let dir = std::env::temp_dir().join(format!("agora-uplink-{}-bare", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Arc::new(EventSink::open_with_cap(&dir, "n1", 600).unwrap());
        for i in 0..40 {
            s.node_resources("n1", i, i, 0);
        }
        let next = s.next_seq();
        let (mut up, replay) = streaming(&s, 1);
        assert!(matches!(replay, Replay::Resync { from_seq: 1, .. }));
        assert_eq!(s.next_seq(), next);
        assert_eq!(
            decoded(&up.next_batch(1000).unwrap())[0].seq,
            s.oldest_retained()
        );
    }
}

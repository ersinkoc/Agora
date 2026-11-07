//! Event sequencing and replay semantics (docs/specs/protocol.md section 3).
//!
//! Every producer numbers its events 1, 2, 3, ... A consumer applies each `(producer, seq)` at most once, in
//! order, and acknowledges the highest contiguous `seq`. Delivery is at-least-once: after a reconnect the
//! producer replays everything after the consumer's ack. These are the in-memory reference implementations;
//! persistence (node disk buffer, hub event store) wraps the same rules.

use std::collections::HashMap;

/// Identifies who numbers a stream of events (a node, or the hub for its own facts).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProducerId(pub String);

/// Producer side: numbers events and keeps unacknowledged ones for replay.
#[derive(Debug)]
pub struct EventLog<E> {
    /// Sequence number of the first retained event.
    first_retained: u64,
    retained: Vec<E>,
    acked: u64,
}

impl<E: Clone> Default for EventLog<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: Clone> EventLog<E> {
    pub fn new() -> Self {
        Self {
            first_retained: 1,
            retained: Vec::new(),
            acked: 0,
        }
    }

    /// Record an event and return its sequence number.
    pub fn append(&mut self, event: E) -> u64 {
        self.retained.push(event);
        self.next_seq() - 1
    }

    /// The number the next appended event will get.
    pub fn next_seq(&self) -> u64 {
        self.first_retained + self.retained.len() as u64
    }

    /// Lowest sequence number still retained (equals `next_seq` when everything is acked).
    pub fn oldest_retained(&self) -> u64 {
        self.first_retained
    }

    pub fn acked(&self) -> u64 {
        self.acked
    }

    /// Consumer acknowledged everything up to and including `seq`. Acks never move backwards and are
    /// clamped to what was actually produced.
    pub fn ack(&mut self, seq: u64) {
        let seq = seq.min(self.next_seq() - 1);
        if seq <= self.acked {
            return;
        }
        self.acked = seq;
        let drop = (seq + 1 - self.first_retained) as usize;
        self.retained.drain(..drop);
        self.first_retained = seq + 1;
    }

    /// Events from `from_seq` onward, with their sequence numbers; `None` if `from_seq` was already
    /// discarded (the consumer needs a snapshot instead).
    pub fn replay_from(&self, from_seq: u64) -> Option<Vec<(u64, E)>> {
        if from_seq < self.first_retained {
            return None;
        }
        let skip = (from_seq - self.first_retained) as usize;
        Some(
            self.retained
                .iter()
                .skip(skip)
                .enumerate()
                .map(|(i, e)| (from_seq + i as u64, e.clone()))
                .collect(),
        )
    }
}

/// What happened to one received event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Receive {
    /// Applied; the consumer should ack `acked`.
    Applied { acked: u64 },
    /// Already applied earlier; ignored.
    Duplicate { acked: u64 },
    /// An earlier event is missing. The event was not applied; the producer must replay from `expected`.
    Gap { expected: u64 },
}

/// Consumer side: tracks the highest contiguous `seq` per producer and applies each event once.
#[derive(Debug, Default)]
pub struct Consumer {
    acked: HashMap<ProducerId, u64>,
}

impl Consumer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Highest contiguous sequence number applied for `producer` (0 if none). Send this in `Resume`.
    pub fn acked(&self, producer: &ProducerId) -> u64 {
        self.acked.get(producer).copied().unwrap_or(0)
    }

    /// Offer an event. `apply` runs only when the event is exactly the next one expected.
    pub fn receive(&mut self, producer: &ProducerId, seq: u64, apply: impl FnOnce()) -> Receive {
        let acked = self.acked(producer);
        if seq <= acked {
            Receive::Duplicate { acked }
        } else if seq == acked + 1 {
            apply();
            self.acked.insert(producer.clone(), seq);
            Receive::Applied { acked: seq }
        } else {
            Receive::Gap {
                expected: acked + 1,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(n: &str) -> ProducerId {
        ProducerId(n.into())
    }

    fn deliver(c: &mut Consumer, who: &ProducerId, seq: u64, applied: &mut Vec<u64>) -> Receive {
        c.receive(who, seq, || applied.push(seq))
    }

    #[test]
    fn sequence_numbers_start_at_one() {
        let mut log = EventLog::new();
        assert_eq!(log.append("a"), 1);
        assert_eq!(log.append("b"), 2);
        assert_eq!(log.next_seq(), 3);
    }

    #[test]
    fn in_order_delivery_applies_each_event_once() {
        let (mut c, who, mut applied) = (Consumer::new(), p("n"), Vec::new());
        assert_eq!(
            deliver(&mut c, &who, 1, &mut applied),
            Receive::Applied { acked: 1 }
        );
        assert_eq!(
            deliver(&mut c, &who, 2, &mut applied),
            Receive::Applied { acked: 2 }
        );
        assert_eq!(applied, vec![1, 2]);
    }

    #[test]
    fn duplicates_are_ignored() {
        let (mut c, who, mut applied) = (Consumer::new(), p("n"), Vec::new());
        deliver(&mut c, &who, 1, &mut applied);
        assert_eq!(
            deliver(&mut c, &who, 1, &mut applied),
            Receive::Duplicate { acked: 1 }
        );
        assert_eq!(applied, vec![1]);
    }

    #[test]
    fn gaps_and_reordering_are_refused_until_the_missing_event_arrives() {
        let (mut c, who, mut applied) = (Consumer::new(), p("n"), Vec::new());
        assert_eq!(
            deliver(&mut c, &who, 2, &mut applied),
            Receive::Gap { expected: 1 }
        );
        assert!(applied.is_empty());
        deliver(&mut c, &who, 1, &mut applied);
        // 2 arrives again (at-least-once) and is now applied
        assert_eq!(
            deliver(&mut c, &who, 2, &mut applied),
            Receive::Applied { acked: 2 }
        );
        assert_eq!(applied, vec![1, 2]);
    }

    #[test]
    fn producers_are_independent() {
        let (mut c, mut applied) = (Consumer::new(), Vec::new());
        deliver(&mut c, &p("a"), 1, &mut applied);
        assert_eq!(c.acked(&p("b")), 0);
        assert_eq!(
            deliver(&mut c, &p("b"), 1, &mut applied),
            Receive::Applied { acked: 1 }
        );
    }

    #[test]
    fn reconnect_replays_exactly_what_was_missed() {
        let (mut log, mut c, who) = (EventLog::new(), Consumer::new(), p("n"));
        let mut applied = Vec::new();
        for e in 1..=5u64 {
            log.append(e);
        }
        // Connection delivers 1..=3, then drops (4 and 5 were lost)
        for s in 1..=3 {
            deliver(&mut c, &who, s, &mut applied);
        }
        log.ack(c.acked(&who));
        // Reconnect: Resume { last_acked = 3 } -> replay from 4. Replay may overlap what already arrived.
        let replay = log.replay_from(c.acked(&who) + 1);
        assert_eq!(replay.as_ref().map(Vec::len), Some(2));
        for (s, _) in replay.unwrap_or_default() {
            deliver(&mut c, &who, s, &mut applied);
        }
        assert_eq!(applied, vec![1, 2, 3, 4, 5]);
        assert_eq!(c.acked(&who), 5);
    }

    #[test]
    fn acked_events_are_discarded_and_stale_resumes_need_a_snapshot() {
        let mut log = EventLog::new();
        for e in 0..10 {
            log.append(e);
        }
        log.ack(6);
        assert_eq!(log.oldest_retained(), 7);
        assert_eq!(log.replay_from(7).map(|v| v.len()), Some(4));
        assert_eq!(log.replay_from(3), None);
    }

    #[test]
    fn acks_never_regress_or_exceed_what_exists() {
        let mut log = EventLog::new();
        log.append(());
        log.append(());
        log.ack(2);
        log.ack(1);
        assert_eq!(log.acked(), 2);
        log.ack(99);
        assert_eq!(log.acked(), 2);
        assert_eq!(log.next_seq(), 3);
        assert_eq!(log.append(()), 3);
    }
}

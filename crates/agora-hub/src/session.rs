//! The hub's side of one node connection, as a state machine over decoded messages. No I/O and no framing:
//! the transport (WSS + mTLS, ADR-0011) authenticates the node, decodes frames and feeds this, so the same
//! logic is testable without sockets. The producer id comes from the authenticated identity, never from the
//! node's own messages.
//!
//! Order: `hello` → `resume` → (`event` | `snapshot_required`)*. Anything out of order is `protocol.invalid_message`; an event that
//! names another producer than the certificate is `protocol.producer_mismatch`.
//!
//! `SnapshotRequired` from the node (ADR-0054) says it cannot replay from where the hub asked: the next event above what the
//! hub holds becomes the new base of this producer's numbering, once, and the skipped numbers are recorded as lost.

use agora_protocol::handshake::{negotiate, Support};
use agora_protocol::v1::{Event, Hello, Resume, ResumeOk, Role, Welcome};
use agora_protocol::{Error, ErrorCode};
use prost::Message;

use crate::store::{Append, EventStore, StoreError};

/// What happened to one received event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ingest {
    /// Stored at global log position `pos`; `acked` is the highest contiguous seq now held for this node.
    Applied { acked: u64, pos: u64 },
    /// Already held; `acked` is unchanged and may be re-sent.
    Duplicate { acked: u64 },
    /// Not the next event. Nothing was stored; the node must replay from `expected` (answer with `ResumeOk`).
    Gap { expected: u64 },
    /// Stored as the new base after the node's `SnapshotRequired` (ADR-0054): `from_seq..=to_seq` were skipped and are recorded as
    /// lost. Otherwise like `Applied`.
    Resynced {
        acked: u64,
        pos: u64,
        from_seq: u64,
        to_seq: u64,
    },
}

#[derive(Debug)]
pub enum SessionError {
    /// The peer broke the protocol; send this error and close.
    Protocol(Error),
    Store(StoreError),
}

impl From<Error> for SessionError {
    fn from(e: Error) -> Self {
        Self::Protocol(e)
    }
}

impl From<StoreError> for SessionError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

fn invalid(field: &str) -> SessionError {
    Error::new(ErrorCode::ProtocolInvalidMessage)
        .with("field", field)
        .into()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    New,
    Negotiated,
    Ready,
}

pub struct NodeSession<'a, S: EventStore> {
    store: &'a S,
    producer: String,
    support: &'a Support,
    phase: Phase,
    /// When the node said it cannot replay (ADR-0054) and no event above the hub's ack has come since.
    resync_at_ms: Option<u64>,
}

impl<'a, S: EventStore> NodeSession<'a, S> {
    /// `producer` is the node id from its client certificate.
    pub fn new(store: &'a S, producer: &str, support: &'a Support) -> Self {
        Self {
            store,
            producer: producer.to_string(),
            support,
            phase: Phase::New,
            resync_at_ms: None,
        }
    }

    pub fn hello(&mut self, hello: &Hello, now_ms: u64) -> Result<Welcome, SessionError> {
        if self.phase != Phase::New {
            return Err(invalid("hello"));
        }
        if hello.role != Role::Node as i32 {
            return Err(invalid("role"));
        }
        let welcome = negotiate(hello, self.support, &self.producer, now_ms)?;
        self.phase = Phase::Negotiated;
        Ok(welcome)
    }

    /// The hub's own record decides where replay starts: whatever the node believes, `from_seq` is the first
    /// event the hub does not hold. (A node that lost its buffer is caught by `Gap` on the first event.)
    pub fn resume(&mut self, _request: &Resume) -> Result<ResumeOk, SessionError> {
        if self.phase != Phase::Negotiated {
            return Err(invalid("resume"));
        }
        let acked = self.store.acked(&self.producer)?;
        self.phase = Phase::Ready;
        Ok(ResumeOk {
            from_seq: acked + 1,
        })
    }

    /// The node cannot replay from where the hub asked (ADR-0054): its next event above the hub's ack becomes the new base. Only
    /// after the handshake; a second one before such an event changes nothing more.
    pub fn snapshot_required(&mut self, now_ms: u64) -> Result<(), SessionError> {
        if self.phase != Phase::Ready {
            return Err(invalid("snapshot_required"));
        }
        self.resync_at_ms.get_or_insert(now_ms);
        Ok(())
    }

    /// Decode and store one event payload from this node.
    pub fn event(&mut self, payload: &[u8]) -> Result<Ingest, SessionError> {
        if self.phase != Phase::Ready {
            return Err(invalid("event"));
        }
        let ev = Event::decode(payload).map_err(|_| invalid("event"))?;
        // Its own name or nothing: a node's buffer from before enrollment (`local`) or under another name is refused with a code
        // that says so, which the transport sends as its close reason.
        if ev.producer_id != self.producer {
            let shown: String = ev.producer_id.chars().take(64).collect();
            return Err(Error::new(ErrorCode::ProtocolProducerMismatch)
                .with("node_id", self.producer.as_str())
                .with("producer_id", shown.as_str())
                .into());
        }
        if ev.seq == 0 {
            return Err(invalid("seq"));
        }
        // A node speaks for itself only (threat model T19). A fact naming another node would end that node's agents, say it is
        // online or gone, overwrite its numbers, or list this node's folders, agents and terminals on it (and in its projects).
        // Presence (ADR-0070) and forgetting are the hub's own facts, written from the certificate-authenticated connection.
        match ev.body.as_ref().map(crate::projection::named_node) {
            Some(Err(field)) => return Err(invalid(field)),
            Some(Ok(Some(node))) if node != self.producer => return Err(invalid("node_id")),
            _ => {}
        }
        let appended = match self.resync_at_ms {
            Some(at) => self
                .store
                .append_resync(&self.producer, ev.seq, ev.at_ms, payload, at)?,
            None => self
                .store
                .append(&self.producer, ev.seq, ev.at_ms, payload)?,
        };
        Ok(match appended {
            Append::Applied { pos } => {
                // The first event above the ack ends the resync, jump or not: it is allowed once.
                self.resync_at_ms = None;
                Ingest::Applied {
                    acked: self.store.acked(&self.producer)?,
                    pos,
                }
            }
            Append::Resynced {
                pos,
                from_seq,
                to_seq,
            } => {
                self.resync_at_ms = None;
                Ingest::Resynced {
                    acked: self.store.acked(&self.producer)?,
                    pos,
                    from_seq,
                    to_seq,
                }
            }
            Append::Duplicate => Ingest::Duplicate {
                acked: self.store.acked(&self.producer)?,
            },
            Append::Gap { expected } => Ingest::Gap { expected },
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub(crate) mod tests {
    use super::*;
    use crate::store::SqliteStore;
    use agora_protocol::v1::event::Body;
    use agora_protocol::v1::NodeOnline;

    fn support() -> Support {
        Support {
            generations: vec![1],
            capabilities: vec!["terminal.raw".into()],
            required: vec![],
        }
    }

    fn node_hello() -> Hello {
        Hello {
            role: Role::Node as i32,
            protocol_generations: vec![1],
            capabilities: vec!["terminal.raw".into(), "unknown.thing".into()],
            build: "t".into(),
            locale: None,
        }
    }

    fn event(producer: &str, seq: u64) -> Vec<u8> {
        Event {
            producer_id: producer.into(),
            seq,
            at_ms: 1,
            body: Some(agora_protocol::v1::event::Body::NodeOnline(NodeOnline {
                node_id: producer.into(),
                ..NodeOnline::default()
            })),
        }
        .encode_to_vec()
    }

    fn ready<'a>(
        store: &'a SqliteStore,
        support: &'a Support,
        id: &str,
    ) -> NodeSession<'a, SqliteStore> {
        let mut s = NodeSession::new(store, id, support);
        s.hello(&node_hello(), 0).unwrap();
        s.resume(&Resume::default()).unwrap();
        s
    }

    fn code(e: SessionError) -> ErrorCode {
        match e {
            SessionError::Protocol(e) => e.code,
            SessionError::Store(e) => panic!("{e}"),
        }
    }

    #[test]
    fn hello_negotiates_and_names_the_peer_by_identity() {
        let (store, sup) = (SqliteStore::open_in_memory().unwrap(), support());
        let mut s = NodeSession::new(&store, "node-a", &sup);
        let w = s.hello(&node_hello(), 7).unwrap();
        assert_eq!(w.peer_id, "node-a");
        assert_eq!(w.capabilities, ["terminal.raw"]);
        assert_eq!(w.server_time_ms, 7);
    }

    #[test]
    fn only_nodes_may_use_a_node_session() {
        let (store, sup) = (SqliteStore::open_in_memory().unwrap(), support());
        let mut s = NodeSession::new(&store, "n", &sup);
        let mut h = node_hello();
        h.role = Role::Client as i32;
        assert_eq!(
            code(s.hello(&h, 0).unwrap_err()),
            ErrorCode::ProtocolInvalidMessage
        );
    }

    /// ADR-0070: presence is the hub's fact; a node that sends a `NodeLink` (for itself or another node) is refused.
    #[test]
    fn a_node_cannot_send_presence() {
        let store = SqliteStore::open_in_memory().unwrap();
        let sup = support();
        let mut s = NodeSession::new(&store, "n1", &sup);
        s.hello(&node_hello(), 0).unwrap();
        s.resume(&Resume::default()).unwrap();
        let ev = Event {
            producer_id: "n1".into(),
            seq: 1,
            at_ms: 1,
            body: Some(agora_protocol::v1::event::Body::NodeLink(
                agora_protocol::v1::NodeLink {
                    node_id: "n2".into(),
                    up: true,
                    ..Default::default()
                },
            )),
        }
        .encode_to_vec();
        assert!(s.event(&ev).is_err());
        assert_eq!(store.acked("n1").unwrap(), 0);
    }

    /// Every body that carries a node id, naming `node` (local ids `c`, `a`, `t`, `w`).
    pub(crate) fn naming(node: &str) -> Vec<Body> {
        use agora_protocol::v1;
        let node = node.to_string();
        vec![
            Body::NodeOnline(v1::NodeOnline {
                node_id: node.clone(),
                ..Default::default()
            }),
            Body::NodeOffline(v1::NodeOffline {
                node_id: node.clone(),
            }),
            Body::NodeResources(v1::NodeResources {
                node_id: node.clone(),
                agents_running: 7,
                ..Default::default()
            }),
            Body::CheckoutDiscovered(v1::CheckoutDiscovered {
                checkout_id: "c".into(),
                node_id: node.clone(),
                ..Default::default()
            }),
            Body::AgentStarted(v1::AgentStarted {
                agent_id: "a".into(),
                node_id: node.clone(),
                ..Default::default()
            }),
            Body::TerminalOpened(v1::TerminalOpened {
                terminal_id: "t".into(),
                node_id: node.clone(),
                ..Default::default()
            }),
            Body::WorkspaceRegistered(v1::WorkspaceRegistered {
                workspace_id: "w".into(),
                node_id: node,
                ..Default::default()
            }),
        ]
    }

    /// Threat model T19: a node speaks for itself only. A fact that names another node, nobody, or this node spelled otherwise is
    /// refused (`field = node_id`) and not stored; the same fact naming the sender is stored.
    #[test]
    fn a_fact_that_names_a_node_must_name_the_sender() {
        let (store, sup) = (SqliteStore::open_in_memory().unwrap(), support());
        let mut s = ready(&store, &sup, "n1");
        let at = |seq: u64, body| {
            Event {
                producer_id: "n1".into(),
                seq,
                at_ms: 1,
                body: Some(body),
            }
            .encode_to_vec()
        };
        let refused = Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "node_id");
        let mut seq = 0;
        for (i, own) in naming("n1").into_iter().enumerate() {
            for other in ["n2", "", "N1"] {
                match s.event(&at(seq + 1, naming(other).swap_remove(i))) {
                    Err(SessionError::Protocol(e)) => {
                        assert_eq!(e, refused, "{own:?} naming {other:?}")
                    }
                    got => panic!("{own:?} naming {other:?}: {got:?}"),
                }
                assert_eq!(store.acked("n1").unwrap(), seq);
            }
            seq += 1;
            assert_eq!(
                s.event(&at(seq, own)).unwrap(),
                Ingest::Applied {
                    acked: seq,
                    pos: seq
                }
            );
        }
    }

    /// ADR-0073, ADR-0053: forgetting a machine or an agent is the hub's own fact, like presence; a node that sends one (about another
    /// machine or itself) is refused and nothing is stored.
    #[test]
    fn a_node_cannot_forget_a_machine_or_an_agent() {
        use agora_protocol::v1::{AgentForgotten, NodeForgotten};
        let (store, sup) = (SqliteStore::open_in_memory().unwrap(), support());
        let mut s = ready(&store, &sup, "n1");
        let facts = [
            (
                Body::NodeForgotten(NodeForgotten {
                    node_id: "n2".into(),
                    refused_return: true,
                }),
                "node_forgotten",
            ),
            (
                Body::NodeForgotten(NodeForgotten {
                    node_id: "n1".into(),
                    refused_return: false,
                }),
                "node_forgotten",
            ),
            (
                Body::AgentForgotten(AgentForgotten {
                    agent_id: "n2/a".into(),
                }),
                "agent_forgotten",
            ),
            (
                Body::AgentForgotten(AgentForgotten {
                    agent_id: "n1/a".into(),
                }),
                "agent_forgotten",
            ),
        ];
        for (body, field) in facts {
            let ev = Event {
                producer_id: "n1".into(),
                seq: 1,
                at_ms: 1,
                body: Some(body.clone()),
            }
            .encode_to_vec();
            match s.event(&ev) {
                Err(SessionError::Protocol(e)) => assert_eq!(
                    e,
                    Error::new(ErrorCode::ProtocolInvalidMessage).with("field", field)
                ),
                got => panic!("{body:?}: {got:?}"),
            }
        }
        assert_eq!(store.acked("n1").unwrap(), 0);
    }

    #[test]
    fn events_before_the_handshake_are_refused() {
        let (store, sup) = (SqliteStore::open_in_memory().unwrap(), support());
        let mut s = NodeSession::new(&store, "n", &sup);
        assert_eq!(
            code(s.event(&event("n", 1)).unwrap_err()),
            ErrorCode::ProtocolInvalidMessage
        );
        s.hello(&node_hello(), 0).unwrap();
        assert_eq!(
            code(s.event(&event("n", 1)).unwrap_err()),
            ErrorCode::ProtocolInvalidMessage
        );
        assert_eq!(
            code(s.hello(&node_hello(), 0).unwrap_err()),
            ErrorCode::ProtocolInvalidMessage
        );
    }

    #[test]
    fn resume_starts_after_what_the_hub_holds() {
        let (store, sup) = (SqliteStore::open_in_memory().unwrap(), support());
        {
            let mut s = ready(&store, &sup, "n");
            s.event(&event("n", 1)).unwrap();
            s.event(&event("n", 2)).unwrap();
        }
        let mut again = NodeSession::new(&store, "n", &sup);
        again.hello(&node_hello(), 0).unwrap();
        // The node claims it only acked 0; the hub still says replay starts at 3 (it already holds 1 and 2).
        let ok = again
            .resume(&Resume {
                last_acked_seq: Some(0),
            })
            .unwrap();
        assert_eq!(ok.from_seq, 3);
    }

    #[test]
    fn ingest_reports_applied_duplicate_and_gap() {
        let (store, sup) = (SqliteStore::open_in_memory().unwrap(), support());
        let mut s = ready(&store, &sup, "n");
        assert_eq!(
            s.event(&event("n", 1)).unwrap(),
            Ingest::Applied { acked: 1, pos: 1 }
        );
        assert_eq!(
            s.event(&event("n", 1)).unwrap(),
            Ingest::Duplicate { acked: 1 }
        );
        assert_eq!(
            s.event(&event("n", 3)).unwrap(),
            Ingest::Gap { expected: 2 }
        );
        assert_eq!(
            s.event(&event("n", 2)).unwrap(),
            Ingest::Applied { acked: 2, pos: 2 }
        );
    }

    #[test]
    fn a_node_cannot_write_as_another_producer() {
        let (store, sup) = (SqliteStore::open_in_memory().unwrap(), support());
        let mut s = ready(&store, &sup, "n");
        assert_eq!(
            code(s.event(&event("other", 1)).unwrap_err()),
            ErrorCode::ProtocolProducerMismatch
        );
        assert_eq!(store.acked("other").unwrap(), 0);
    }

    #[test]
    fn garbage_and_zero_seq_are_invalid() {
        let (store, sup) = (SqliteStore::open_in_memory().unwrap(), support());
        let mut s = ready(&store, &sup, "n");
        assert_eq!(
            code(s.event(&[0xff, 0xff, 0xff]).unwrap_err()),
            ErrorCode::ProtocolInvalidMessage
        );
        assert_eq!(
            code(s.event(&event("n", 0)).unwrap_err()),
            ErrorCode::ProtocolInvalidMessage
        );
    }

    #[test]
    fn snapshot_required_before_the_handshake_is_invalid() {
        let (store, sup) = (SqliteStore::open_in_memory().unwrap(), support());
        let mut s = NodeSession::new(&store, "n", &sup);
        assert_eq!(
            code(s.snapshot_required(1).unwrap_err()),
            ErrorCode::ProtocolInvalidMessage
        );
        s.hello(&node_hello(), 0).unwrap();
        assert_eq!(
            code(s.snapshot_required(1).unwrap_err()),
            ErrorCode::ProtocolInvalidMessage
        );
    }

    /// ADR-0054: a jump is accepted only right after `SnapshotRequired`, only forwards, and only once.
    #[test]
    fn a_jump_is_accepted_only_after_snapshot_required_only_forwards_and_once() {
        let (store, sup) = (SqliteStore::open_in_memory().unwrap(), support());
        let mut s = ready(&store, &sup, "n");
        for i in 1..=3 {
            s.event(&event("n", i)).unwrap();
        }
        // Without it, a jump is a gap and nothing is stored.
        assert_eq!(
            s.event(&event("n", 9)).unwrap(),
            Ingest::Gap { expected: 4 }
        );
        s.snapshot_required(77).unwrap();
        s.snapshot_required(78).unwrap(); // a second one changes nothing more
                                          // Never backwards: what the hub holds stays a duplicate and does not use the resync up.
        assert_eq!(
            s.event(&event("n", 2)).unwrap(),
            Ingest::Duplicate { acked: 3 }
        );
        assert_eq!(
            s.event(&event("n", 9)).unwrap(),
            Ingest::Resynced {
                acked: 9,
                pos: 4,
                from_seq: 4,
                to_seq: 8
            }
        );
        assert_eq!(store.resyncs().unwrap(), [("n".to_string(), 4, 8, 77)]);
        // Once: the next jump is a gap again.
        assert_eq!(
            s.event(&event("n", 20)).unwrap(),
            Ingest::Gap { expected: 10 }
        );
        assert_eq!(
            s.event(&event("n", 10)).unwrap(),
            Ingest::Applied { acked: 10, pos: 5 }
        );
    }

    /// A `SnapshotRequired` followed by the very next number moves nothing and records nothing, and it is used up.
    #[test]
    fn snapshot_required_followed_by_the_next_number_changes_nothing() {
        let (store, sup) = (SqliteStore::open_in_memory().unwrap(), support());
        let mut s = ready(&store, &sup, "n");
        s.event(&event("n", 1)).unwrap();
        s.snapshot_required(5).unwrap();
        assert_eq!(
            s.event(&event("n", 2)).unwrap(),
            Ingest::Applied { acked: 2, pos: 2 }
        );
        assert!(store.resyncs().unwrap().is_empty());
        assert_eq!(
            s.event(&event("n", 7)).unwrap(),
            Ingest::Gap { expected: 3 }
        );
        // And one never followed by an event (the connection ends) leaves the store as it was.
        let mut s = ready(&store, &sup, "n");
        s.snapshot_required(6).unwrap();
        drop(s);
        assert_eq!(store.acked("n").unwrap(), 2);
        assert!(store.resyncs().unwrap().is_empty());
        let mut s = ready(&store, &sup, "n");
        assert_eq!(
            s.event(&event("n", 7)).unwrap(),
            Ingest::Gap { expected: 3 },
            "a resync does not carry over to the next connection"
        );
    }

    /// T19: the stored-form rule (`projection::is_forged`, used by `View::rebuild` and the replay barrier) and the live rule in
    /// `NodeSession::event` must say the same about every body, or a history could hold what live ingestion would have refused (or the
    /// reverse). A new body that names a node must be added to `named_node`, and this table says what is expected of it.
    #[test]
    fn the_stored_form_rule_and_the_live_ingestion_rule_agree() {
        use agora_protocol::v1 as v;
        let other = "n2";
        let table: Vec<(&str, Option<Body>, bool)> = vec![
            ("no body", None, false),
            (
                "own online",
                Some(Body::NodeOnline(v::NodeOnline {
                    node_id: "n1".into(),
                    ..Default::default()
                })),
                false,
            ),
            (
                "other online",
                Some(Body::NodeOnline(v::NodeOnline {
                    node_id: other.into(),
                    ..Default::default()
                })),
                true,
            ),
            (
                "own offline",
                Some(Body::NodeOffline(v::NodeOffline {
                    node_id: "n1".into(),
                })),
                false,
            ),
            (
                "other offline",
                Some(Body::NodeOffline(v::NodeOffline {
                    node_id: other.into(),
                })),
                true,
            ),
            (
                "own resources",
                Some(Body::NodeResources(v::NodeResources {
                    node_id: "n1".into(),
                    ..Default::default()
                })),
                false,
            ),
            (
                "other resources",
                Some(Body::NodeResources(v::NodeResources {
                    node_id: other.into(),
                    ..Default::default()
                })),
                true,
            ),
            (
                "own checkout",
                Some(Body::CheckoutDiscovered(v::CheckoutDiscovered {
                    node_id: "n1".into(),
                    ..Default::default()
                })),
                false,
            ),
            (
                "other checkout",
                Some(Body::CheckoutDiscovered(v::CheckoutDiscovered {
                    node_id: other.into(),
                    ..Default::default()
                })),
                true,
            ),
            (
                "own agent start",
                Some(Body::AgentStarted(v::AgentStarted {
                    node_id: "n1".into(),
                    ..Default::default()
                })),
                false,
            ),
            (
                "other agent start",
                Some(Body::AgentStarted(v::AgentStarted {
                    node_id: other.into(),
                    ..Default::default()
                })),
                true,
            ),
            (
                "own terminal",
                Some(Body::TerminalOpened(v::TerminalOpened {
                    node_id: "n1".into(),
                    ..Default::default()
                })),
                false,
            ),
            (
                "other terminal",
                Some(Body::TerminalOpened(v::TerminalOpened {
                    node_id: other.into(),
                    ..Default::default()
                })),
                true,
            ),
            (
                "own workspace",
                Some(Body::WorkspaceRegistered(v::WorkspaceRegistered {
                    node_id: "n1".into(),
                    ..Default::default()
                })),
                false,
            ),
            (
                "other workspace",
                Some(Body::WorkspaceRegistered(v::WorkspaceRegistered {
                    node_id: other.into(),
                    ..Default::default()
                })),
                true,
            ),
            (
                "a link (the hub's fact)",
                Some(Body::NodeLink(v::NodeLink {
                    node_id: "n1".into(),
                    ..Default::default()
                })),
                true,
            ),
            (
                "a forgotten machine (the hub's fact)",
                Some(Body::NodeForgotten(v::NodeForgotten {
                    node_id: "n1".into(),
                    ..Default::default()
                })),
                true,
            ),
            (
                "a forgotten agent (the hub's fact)",
                Some(Body::AgentForgotten(v::AgentForgotten {
                    agent_id: "n1/a".into(),
                })),
                true,
            ),
            (
                "a state change names no node",
                Some(Body::AgentStateChanged(v::AgentStateChanged::default())),
                false,
            ),
        ];
        for (what, body, forged) in table {
            let ev = Event {
                producer_id: "n1".into(),
                seq: 1,
                at_ms: 1,
                body,
            };
            let payload = ev.encode_to_vec();
            // The stored form: what `rebuild` decodes from the log.
            let stored = Event::decode(payload.as_slice()).unwrap();
            assert_eq!(
                crate::projection::is_forged(&stored),
                forged,
                "stored rule: {what}"
            );
            // The live rule: a fresh session, one event.
            let (store, sup) = (SqliteStore::open_in_memory().unwrap(), support());
            let mut s = ready(&store, &sup, "n1");
            match s.event(&payload) {
                Ok(_) => assert!(
                    !forged,
                    "live ingestion accepted a body the stored rule calls forged: {what}"
                ),
                Err(e) => {
                    assert_eq!(code(e), ErrorCode::ProtocolInvalidMessage, "{what}");
                    assert!(
                        forged,
                        "live ingestion refused a body the stored rule accepts: {what}"
                    );
                }
            }
        }
        // The hub's own events are never forged in storage, whatever node they name (a node cannot produce them live: the producer
        // would not match its certificate).
        let hub_event = Event {
            producer_id: crate::ca::RESERVED_HUB_ID.into(),
            seq: 1,
            at_ms: 1,
            body: Some(Body::NodeLink(agora_protocol::v1::NodeLink {
                node_id: "n2".into(),
                ..Default::default()
            })),
        };
        assert!(!crate::projection::is_forged(&hub_event));
    }
}

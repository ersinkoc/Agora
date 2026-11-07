//! The hub's side of one node connection, as a state machine over decoded messages. No I/O and no framing:
//! the transport (WSS + mTLS, ADR-0011) authenticates the node, decodes frames and feeds this, so the same
//! logic is testable without sockets. The producer id comes from the authenticated identity, never from the
//! node's own messages.
//!
//! Order: `hello` → `resume` → `event`*. Anything out of order is `protocol.invalid_message`.

use agora_protocol::handshake::{negotiate, Support};
use agora_protocol::v1::{Event, Hello, Resume, ResumeOk, Role, Welcome};
use agora_protocol::{Error, ErrorCode};
use prost::Message;

use crate::store::{Append, EventStore, StoreError};

/// What happened to one received event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ingest {
    /// Stored; `acked` is the highest contiguous seq now held for this node.
    Applied { acked: u64 },
    /// Already held; `acked` is unchanged and may be re-sent.
    Duplicate { acked: u64 },
    /// Not the next event. Nothing was stored; the node must replay from `expected` (answer with `ResumeOk`).
    Gap { expected: u64 },
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
}

impl<'a, S: EventStore> NodeSession<'a, S> {
    /// `producer` is the node id from its client certificate.
    pub fn new(store: &'a S, producer: &str, support: &'a Support) -> Self {
        Self {
            store,
            producer: producer.to_string(),
            support,
            phase: Phase::New,
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

    /// Decode and store one event payload from this node.
    pub fn event(&mut self, payload: &[u8]) -> Result<Ingest, SessionError> {
        if self.phase != Phase::Ready {
            return Err(invalid("event"));
        }
        let ev = Event::decode(payload).map_err(|_| invalid("event"))?;
        if ev.producer_id != self.producer {
            return Err(invalid("producer_id"));
        }
        if ev.seq == 0 {
            return Err(invalid("seq"));
        }
        Ok(
            match self
                .store
                .append(&self.producer, ev.seq, ev.at_ms, payload)?
            {
                Append::Applied { .. } => Ingest::Applied {
                    acked: self.store.acked(&self.producer)?,
                },
                Append::Duplicate => Ingest::Duplicate {
                    acked: self.store.acked(&self.producer)?,
                },
                Append::Gap { expected } => Ingest::Gap { expected },
            },
        )
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::store::SqliteStore;
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
            body: Some(agora_protocol::v1::event::Body::NodeOnline(
                NodeOnline::default(),
            )),
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
            Ingest::Applied { acked: 1 }
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
            Ingest::Applied { acked: 2 }
        );
    }

    #[test]
    fn a_node_cannot_write_as_another_producer() {
        let (store, sup) = (SqliteStore::open_in_memory().unwrap(), support());
        let mut s = ready(&store, &sup, "n");
        assert_eq!(
            code(s.event(&event("other", 1)).unwrap_err()),
            ErrorCode::ProtocolInvalidMessage
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
}

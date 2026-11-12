//! The node's side of its connection to the hub, as a state machine over decoded messages (the mirror of the hub's
//! `NodeSession`). No sockets and no framing: the transport (WSS with the node's client certificate, or SSH stdio)
//! carries the messages. Nodes are autonomous (AGENTS.md): nothing here is needed for agents to run, and everything
//! the node reports is already safe in the durable event buffer before it is ever sent.
//!
//! Order: `hello` → (peer's `Welcome`) → `resume` → (peer's `ResumeOk`) → `next_batch`* while the connection lasts.
//! The hub decides where replay starts (`ResumeOk.from_seq`); the node only checks it still has those events.

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
    /// The hub needs events the node no longer has (the size cap dropped them): the hub must resynchronize from a
    /// snapshot instead of replaying. `oldest_retained` is the first event the node still has.
    Stale {
        oldest_retained: u64,
    },
    Io(String),
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
}

impl Uplink {
    pub fn new(sink: Arc<EventSink>, build: &str, capabilities: &[&str]) -> Self {
        Self {
            sink,
            build: build.to_string(),
            capabilities: capabilities.iter().map(|c| (*c).to_string()).collect(),
            phase: Phase::New,
            cursor: 0,
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
    pub fn on_resume_ok(&mut self, ok: &ResumeOk) -> Result<(), UplinkError> {
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
        self.rewind_to(ok.from_seq)?;
        self.phase = Phase::Streaming;
        Ok(())
    }

    /// The hub reported a gap (it expected `expected`): send again from there.
    pub fn on_gap(&mut self, expected: u64) -> Result<(), UplinkError> {
        if self.phase != Phase::Streaming {
            return Err(UplinkError::OutOfOrder);
        }
        self.rewind_to(expected)
    }

    fn rewind_to(&mut self, from_seq: u64) -> Result<(), UplinkError> {
        let oldest = self.sink.oldest_retained();
        if from_seq < oldest {
            return Err(UplinkError::Stale {
                oldest_retained: oldest,
            });
        }
        self.cursor = from_seq.saturating_sub(1);
        Ok(())
    }

    /// Up to `max` encoded events not yet sent on this connection, oldest first. Empty when caught up.
    pub fn next_batch(&mut self, max: usize) -> Result<Vec<Vec<u8>>, UplinkError> {
        if self.phase != Phase::Streaming {
            return Err(UplinkError::OutOfOrder);
        }
        let batch = match self.sink.payloads_after_max(self.cursor, max) {
            Ok(b) => b,
            Err(ReplayError::Stale { oldest_retained }) => {
                return Err(UplinkError::Stale { oldest_retained })
            }
            Err(ReplayError::Io(e)) => return Err(UplinkError::Io(e.to_string())),
        };
        self.cursor += batch.len() as u64;
        Ok(batch)
    }

    /// The hub acknowledged everything up to `up_to_seq` (ADR-0026 `EventAck`): the buffer may drop it.
    pub fn on_ack(&self, up_to_seq: u64) -> Result<(), UplinkError> {
        self.sink
            .ack(up_to_seq)
            .map_err(|e| UplinkError::Io(e.to_string()))
    }
}

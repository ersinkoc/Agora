//! Handshake and capability negotiation (docs/specs/protocol.md section 1). Pure functions over the
//! generated messages; no I/O.

use crate::error::{Error, ErrorCode};
use crate::v1::{Hello, Resume, ResumeOk, Welcome};

/// What one side supports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Support {
    /// Generations this build speaks. A hub supports the current and the previous one (ADR-0019).
    pub generations: Vec<u32>,
    pub capabilities: Vec<String>,
    /// Capabilities without which this side refuses to continue.
    pub required: Vec<String>,
}

fn join(list: &[u32]) -> String {
    list.iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

/// Pick the highest common generation and intersect capabilities. Unknown capabilities are ignored.
/// `peer_id` and `now_ms` are supplied by the caller so this stays deterministic.
pub fn negotiate(
    hello: &Hello,
    ours: &Support,
    peer_id: &str,
    now_ms: u64,
) -> Result<Welcome, Error> {
    let chosen = hello
        .protocol_generations
        .iter()
        .filter(|g| ours.generations.contains(g))
        .max()
        .copied()
        .ok_or_else(|| {
            Error::new(ErrorCode::ProtocolUnsupportedGeneration)
                .with("offered", join(&hello.protocol_generations).as_str())
                .with("supported", join(&ours.generations).as_str())
        })?;
    let mut caps: Vec<String> = ours
        .capabilities
        .iter()
        .filter(|c| hello.capabilities.contains(c))
        .cloned()
        .collect();
    caps.sort();
    caps.dedup();
    if let Some(missing) = ours.required.iter().find(|r| !caps.contains(r)) {
        return Err(
            Error::new(ErrorCode::ProtocolCapabilityMissing).with("capability", missing.as_str())
        );
    }
    Ok(Welcome {
        chosen_generation: chosen,
        capabilities: caps,
        peer_id: peer_id.to_string(),
        server_time_ms: now_ms,
    })
}

/// Answer to a `Resume`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeAnswer {
    /// Replay events starting at `from_seq`.
    Replay(ResumeOk),
    /// Nothing usable to replay from; the peer must take a fresh snapshot.
    SnapshotRequired,
}

/// Decide resume vs snapshot. `oldest_retained` is the lowest sequence number still stored and `next_seq`
/// the number the next new event will get. A peer that has acked `n` needs `n + 1` onward.
pub fn resume(request: &Resume, oldest_retained: u64, next_seq: u64) -> ResumeAnswer {
    let from = match request.last_acked_seq {
        Some(n) => n.saturating_add(1),
        None => return ResumeAnswer::SnapshotRequired,
    };
    if from > next_seq || from < oldest_retained {
        // Peer is ahead of us (we lost state) or behind what we kept: replay would be wrong or incomplete.
        return ResumeAnswer::SnapshotRequired;
    }
    ResumeAnswer::Replay(ResumeOk { from_seq: from })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1::Role;

    fn hello(gens: &[u32], caps: &[&str]) -> Hello {
        Hello {
            role: Role::Client as i32,
            protocol_generations: gens.to_vec(),
            capabilities: caps.iter().map(|s| s.to_string()).collect(),
            build: "test".into(),
            locale: None,
        }
    }

    fn ours(gens: &[u32], caps: &[&str], required: &[&str]) -> Support {
        Support {
            generations: gens.to_vec(),
            capabilities: caps.iter().map(|s| s.to_string()).collect(),
            required: required.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn picks_highest_common_generation() {
        let w = negotiate(&hello(&[1, 2, 3], &[]), &ours(&[1, 2], &[], &[]), "p", 5);
        assert_eq!(
            w.map(|w| (w.chosen_generation, w.peer_id, w.server_time_ms)),
            Ok((2, "p".into(), 5))
        );
    }

    #[test]
    fn no_common_generation_reports_both_lists() {
        let e = negotiate(&hello(&[3, 4], &[]), &ours(&[1, 2], &[], &[]), "p", 0).err();
        let e = e.map(|e| (e.code.clone(), e.params));
        assert_eq!(
            e.as_ref().map(|e| &e.0),
            Some(&ErrorCode::ProtocolUnsupportedGeneration)
        );
        let params = e.map(|e| e.1).unwrap_or_default();
        assert_eq!(params.get("offered"), Some(&"3,4".into()));
        assert_eq!(params.get("supported"), Some(&"1,2".into()));
    }

    #[test]
    fn capabilities_are_intersected_and_unknown_ones_ignored() {
        let w = negotiate(
            &hello(&[1], &["terminal.raw", "from.the.future", "mesh"]),
            &ours(&[1], &["mesh", "terminal.raw", "terminal.cells"], &[]),
            "p",
            0,
        );
        assert_eq!(
            w.map(|w| w.capabilities),
            Ok(vec!["mesh".to_string(), "terminal.raw".to_string()])
        );
    }

    #[test]
    fn missing_required_capability_is_an_error() {
        let e = negotiate(
            &hello(&[1], &["mesh"]),
            &ours(&[1], &["mesh", "acp"], &["acp"]),
            "p",
            0,
        );
        assert_eq!(
            e.err()
                .map(|e| (e.code, e.params.get("capability").cloned())),
            Some((ErrorCode::ProtocolCapabilityMissing, Some("acp".into())))
        );
    }

    #[test]
    fn resume_replays_when_the_peer_is_within_retained_history() {
        let r = |n| resume(&Resume { last_acked_seq: n }, 10, 20);
        assert_eq!(r(Some(14)), ResumeAnswer::Replay(ResumeOk { from_seq: 15 }));
        assert_eq!(r(Some(9)), ResumeAnswer::Replay(ResumeOk { from_seq: 10 })); // exactly the oldest retained
        assert_eq!(r(Some(19)), ResumeAnswer::Replay(ResumeOk { from_seq: 20 }));
        // nothing missed
    }

    #[test]
    fn resume_needs_a_snapshot_when_history_is_missing_or_inconsistent() {
        let r = |n| resume(&Resume { last_acked_seq: n }, 10, 20);
        assert_eq!(r(None), ResumeAnswer::SnapshotRequired);
        assert_eq!(r(Some(3)), ResumeAnswer::SnapshotRequired); // older than retained
        assert_eq!(r(Some(25)), ResumeAnswer::SnapshotRequired); // ahead of us
        assert_eq!(r(Some(u64::MAX)), ResumeAnswer::SnapshotRequired);
    }

    #[test]
    fn messages_round_trip_on_the_wire() {
        use prost::Message;
        let h = Hello {
            locale: Some("tr".into()),
            ..hello(&[1], &["mesh"])
        };
        assert_eq!(Hello::decode(h.encode_to_vec().as_slice()), Ok(h));
        // proto3 optional: absent and present-but-zero are different
        let a = Resume {
            last_acked_seq: None,
        }
        .encode_to_vec();
        let b = Resume {
            last_acked_seq: Some(0),
        }
        .encode_to_vec();
        assert_ne!(a, b);
    }
}

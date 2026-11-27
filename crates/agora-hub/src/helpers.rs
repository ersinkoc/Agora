//! ADR-0119 decision 2: what the hub keeps to carry helpers started on another machine. The hub vouches for the asking machine
//! (`origin`, from the connection's certificate) and for a chain's depth and root: it writes them from its own record of the starts it
//! carried, so no machine can claim a place in another machine's chain. An asker it carried no start for is depth 0 and its own root.
//! Everything here lives in memory: after a hub restart every asker is its own root again, so chain grants stop applying (they
//! narrow, never widen). Pure data, tested without sockets.

use std::collections::{HashMap, VecDeque};

/// Starts carried per asking machine in any minute, and starts per asking machine whose first answer has not come back.
pub const STARTS_PER_MINUTE: usize = 12;
pub const OPEN_STARTS: usize = 8;
/// How long a carried start is remembered for its result (the card's time to decide, and the re-sends after a reconnection).
pub const PENDING_MS: u64 = 15 * 60 * 1000 + 10 * 60 * 1000;
/// The most carried starts remembered, and the most helpers whose chain is remembered.
const MAX_PENDING: usize = 4096;
const MAX_LINEAGE: usize = 16_384;

/// A chain's place: how deep an agent is and the person-started agent (and its run) the chain began with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chain {
    pub depth: u32,
    pub root_node: String,
    pub root_agent: String,
    pub root_run: Vec<u8>,
}

#[derive(Debug, Clone)]
struct Pending {
    chain: Chain,
    at_ms: u64,
}

/// Why the hub refuses to carry a start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// More than [`STARTS_PER_MINUTE`] in a minute.
    Rate,
    /// More than [`OPEN_STARTS`] waiting for their first answer.
    Open,
}

#[derive(Debug, Default)]
pub struct HelperBroker {
    /// `(machine, agent)` → its chain, for helpers this hub carried a start for.
    lineage: HashMap<(String, String), Chain>,
    /// `(target machine, start id)` → the asker's chain, until the start's result comes back.
    pending: HashMap<(String, Vec<u8>), Pending>,
    /// Per asking machine: when its recent starts were carried, and how many wait for their first answer.
    recent: HashMap<String, VecDeque<u64>>,
    open: HashMap<String, usize>,
}

impl HelperBroker {
    /// The chain of `agent` on `node` as this hub knows it: a helper it carried a start for, else a person-started agent (depth 0,
    /// its own root with the run it gave).
    pub fn chain_of(&self, node: &str, agent: &str, run: &[u8]) -> Chain {
        self.lineage
            .get(&(node.to_string(), agent.to_string()))
            .cloned()
            .unwrap_or_else(|| Chain {
                depth: 0,
                root_node: node.to_string(),
                root_agent: agent.to_string(),
                root_run: run.to_vec(),
            })
    }

    /// A start from `origin` is about to be carried to `target`: checks the per-machine limits and remembers the asker's chain for
    /// the result. Call [`Self::first_answer`] when the target's first answer (or its failure) is back.
    pub fn carry(
        &mut self,
        origin: &str,
        target: &str,
        start_id: &[u8],
        chain: Chain,
        now_ms: u64,
    ) -> Result<(), Refusal> {
        let recent = self.recent.entry(origin.to_string()).or_default();
        while recent
            .front()
            .is_some_and(|t| now_ms.saturating_sub(*t) >= 60_000)
        {
            recent.pop_front();
        }
        if recent.len() >= STARTS_PER_MINUTE {
            return Err(Refusal::Rate);
        }
        let open = self.open.entry(origin.to_string()).or_insert(0);
        if *open >= OPEN_STARTS {
            return Err(Refusal::Open);
        }
        *open += 1;
        recent.push_back(now_ms);
        self.pending
            .retain(|_, p| now_ms.saturating_sub(p.at_ms) < PENDING_MS);
        if self.pending.len() >= MAX_PENDING {
            // The oldest goes: its result, if it ever comes, then finds no chain and its helper is a root of its own (narrowing).
            if let Some(oldest) = self
                .pending
                .iter()
                .min_by_key(|(_, p)| p.at_ms)
                .map(|(k, _)| k.clone())
            {
                self.pending.remove(&oldest);
            }
        }
        self.pending.insert(
            (target.to_string(), start_id.to_vec()),
            Pending {
                chain,
                at_ms: now_ms,
            },
        );
        Ok(())
    }

    /// The target's first answer to a start from `origin` came back (or the forward failed).
    pub fn first_answer(&mut self, origin: &str) {
        if let Some(n) = self.open.get_mut(origin) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                self.open.remove(origin);
            }
        }
    }

    /// The result of a carried start, from `target`: when it started `agent`, that helper's chain is the asker's one level deeper.
    /// A result for a start this hub did not carry (or no longer remembers) records nothing.
    pub fn result(&mut self, target: &str, start_id: &[u8], agent: &str) {
        let Some(p) = self
            .pending
            .remove(&(target.to_string(), start_id.to_vec()))
        else {
            return;
        };
        if agent.is_empty() {
            return;
        }
        if self.lineage.len() >= MAX_LINEAGE {
            // Bounded: a helper forgotten here is its own root when it asks (narrowing, never widening).
            if let Some(k) = self.lineage.keys().next().cloned() {
                self.lineage.remove(&k);
            }
        }
        self.lineage.insert(
            (target.to_string(), agent.to_string()),
            Chain {
                depth: p.chain.depth.saturating_add(1),
                ..p.chain
            },
        );
    }

    /// Whether a start from `origin` to `target` with `start_id` is one this hub carried and still waits for (a result for anything
    /// else is not forwarded).
    pub fn awaits(&self, target: &str, start_id: &[u8]) -> bool {
        self.pending
            .contains_key(&(target.to_string(), start_id.to_vec()))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_chain_is_the_hubs_record_never_the_askers_word() {
        let mut b = HelperBroker::default();
        // A person-started agent on mac is its own root.
        let root = b.chain_of("mac", "agent-3", b"run-1");
        assert_eq!(
            root,
            Chain {
                depth: 0,
                root_node: "mac".into(),
                root_agent: "agent-3".into(),
                root_run: b"run-1".to_vec()
            }
        );
        b.carry("mac", "blackview", b"s1", root.clone(), 0).unwrap();
        b.first_answer("mac");
        b.result("blackview", b"s1", "agent-7");
        // The helper on blackview is depth 1 in mac's chain, whatever run it says it has.
        let helper = b.chain_of("blackview", "agent-7", b"anything");
        assert_eq!(
            (
                helper.depth,
                helper.root_node.as_str(),
                helper.root_agent.as_str()
            ),
            (1, "mac", "agent-3")
        );
        assert_eq!(helper.root_run, b"run-1".to_vec());
        // A result for a start the hub did not carry records nothing.
        b.result("win", b"s9", "agent-1");
        assert_eq!(b.chain_of("win", "agent-1", b"r").depth, 0);
        assert!(!b.awaits("blackview", b"s1"));
    }

    #[test]
    fn each_asking_machine_has_its_rate_and_its_open_starts() {
        let mut b = HelperBroker::default();
        for i in 0..OPEN_STARTS {
            b.carry("mac", "t", &[i as u8], b_chain(), 0).unwrap();
        }
        assert_eq!(b.carry("mac", "t", b"x", b_chain(), 0), Err(Refusal::Open));
        // Another machine is counted apart.
        assert!(b.carry("win", "t", b"x", b_chain(), 0).is_ok());
        for _ in 0..OPEN_STARTS {
            b.first_answer("mac");
        }
        for i in OPEN_STARTS..STARTS_PER_MINUTE {
            b.carry("mac", "t", &[i as u8], b_chain(), 1).unwrap();
            b.first_answer("mac");
        }
        assert_eq!(b.carry("mac", "t", b"y", b_chain(), 2), Err(Refusal::Rate));
        // A minute later the rate allows again.
        assert!(b.carry("mac", "t", b"y", b_chain(), 60_001).is_ok());
    }

    fn b_chain() -> Chain {
        Chain {
            depth: 0,
            root_node: "mac".into(),
            root_agent: "a".into(),
            root_run: vec![1],
        }
    }
}

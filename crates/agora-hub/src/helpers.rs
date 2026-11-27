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
    /// The asking agent on the origin machine (the origin is part of the key).
    asker: String,
    chain: Chain,
    at_ms: u64,
}

/// A helper a target said it started for a carried start, waiting for its own `AgentStarted` to confirm it (review of step 3, finding
/// 2): only an agent the hub sees start new, naming exactly that parent, joins the asker's chain.
#[derive(Debug, Clone)]
struct Expected {
    origin: String,
    asker: String,
    chain: Chain,
    at_ms: u64,
}

/// What a target's `AgentStarted` said of a helper with a parent on another machine, kept briefly in case its result comes after it.
#[derive(Debug, Clone)]
struct Seen {
    parent_node: String,
    parent_agent: String,
    new: bool,
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
    /// `(target machine, origin machine, start id)` → the asker and its chain, until the start's result comes back.
    pending: HashMap<(String, String, Vec<u8>), Pending>,
    /// `(target machine, agent)` → a helper a result named, until its `AgentStarted` confirms it.
    expected: HashMap<(String, String), Expected>,
    /// `(target machine, agent)` → an `AgentStarted` with a remote parent seen before its result.
    seen: HashMap<(String, String), Seen>,
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
        asker: &str,
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
            (target.to_string(), origin.to_string(), start_id.to_vec()),
            Pending {
                asker: asker.to_string(),
                chain,
                at_ms: now_ms,
            },
        );
        Ok(())
    }

    /// A carried start that never reached its target (unknown, not agreed, the forward failed): its first answer is in, and nothing
    /// waits for a result.
    pub fn abandon(&mut self, origin: &str, target: &str, start_id: &[u8]) {
        self.pending
            .remove(&(target.to_string(), origin.to_string(), start_id.to_vec()));
        self.first_answer(origin);
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

    /// The result of a start `origin` asked `target` for. Nothing joins a chain here: the helper it names joins the asker's chain only
    /// once its own `AgentStarted` confirms it ([`Self::agent_started`]), whichever comes first. A result for a start this hub did not
    /// carry from `origin` to `target` records nothing.
    pub fn result(
        &mut self,
        target: &str,
        origin: &str,
        start_id: &[u8],
        agent: &str,
        now_ms: u64,
    ) {
        let Some(p) =
            self.pending
                .remove(&(target.to_string(), origin.to_string(), start_id.to_vec()))
        else {
            return;
        };
        if agent.is_empty() {
            return;
        }
        let key = (target.to_string(), agent.to_string());
        let expected = Expected {
            origin: origin.to_string(),
            asker: p.asker,
            chain: Chain {
                depth: p.chain.depth.saturating_add(1),
                ..p.chain
            },
            at_ms: now_ms,
        };
        match self.seen.remove(&key) {
            Some(seen) => self.confirm(key, &expected, &seen),
            None => {
                self.expected
                    .retain(|_, e| now_ms.saturating_sub(e.at_ms) < PENDING_MS);
                if self.expected.len() < MAX_PENDING {
                    self.expected.insert(key, expected);
                }
            }
        }
    }

    /// A target's `AgentStarted` for `agent` naming a parent on another machine; `new`: the hub had not seen that agent before.
    pub fn agent_started(
        &mut self,
        target: &str,
        agent: &str,
        parent_node: &str,
        parent_agent: &str,
        new: bool,
        now_ms: u64,
    ) {
        let key = (target.to_string(), agent.to_string());
        let seen = Seen {
            parent_node: parent_node.to_string(),
            parent_agent: parent_agent.to_string(),
            new,
            at_ms: now_ms,
        };
        match self.expected.remove(&key) {
            Some(expected) => self.confirm(key, &expected, &seen),
            None => {
                self.seen
                    .retain(|_, s| now_ms.saturating_sub(s.at_ms) < PENDING_MS);
                if self.seen.len() < MAX_PENDING {
                    self.seen.insert(key, seen);
                }
            }
        }
    }

    /// The helper joins the asker's chain only when the hub saw it start new under exactly that parent.
    fn confirm(&mut self, key: (String, String), expected: &Expected, seen: &Seen) {
        if !seen.new || seen.parent_node != expected.origin || seen.parent_agent != expected.asker {
            return;
        }
        if self.lineage.len() >= MAX_LINEAGE {
            // Bounded: a helper forgotten here is its own root when it asks (narrowing, never widening).
            if let Some(k) = self.lineage.keys().next().cloned() {
                self.lineage.remove(&k);
            }
        }
        self.lineage.insert(key, expected.chain.clone());
    }

    /// Whether a start from `origin` to `target` with `start_id` is one this hub carried and still waits for (a result for anything
    /// else is not forwarded).
    pub fn awaits(&self, target: &str, origin: &str, start_id: &[u8]) -> bool {
        self.pending
            .contains_key(&(target.to_string(), origin.to_string(), start_id.to_vec()))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn root() -> Chain {
        Chain {
            depth: 0,
            root_node: "mac".into(),
            root_agent: "agent-3".into(),
            root_run: b"run-1".to_vec(),
        }
    }

    #[test]
    fn a_chain_is_the_hubs_record_never_the_askers_word() {
        let mut b = HelperBroker::default();
        // A person-started agent on mac is its own root.
        assert_eq!(b.chain_of("mac", "agent-3", b"run-1"), root());
        b.carry("mac", "agent-3", "blackview", b"s1", root(), 0)
            .unwrap();
        b.first_answer("mac");
        // The result alone records nothing; the helper's own start confirms it.
        b.result("blackview", "mac", b"s1", "agent-7", 1);
        assert_eq!(b.chain_of("blackview", "agent-7", b"r").depth, 0);
        b.agent_started("blackview", "agent-7", "mac", "agent-3", true, 2);
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
        // The other order: its `AgentStarted` first, then the result.
        b.carry("mac", "agent-3", "win", b"s2", root(), 3).unwrap();
        b.agent_started("win", "agent-2", "mac", "agent-3", true, 4);
        b.result("win", "mac", b"s2", "agent-2", 5);
        assert_eq!(b.chain_of("win", "agent-2", b"r").depth, 1);
        // A result for a start the hub did not carry records nothing.
        b.result("win", "mac", b"s9", "agent-1", 6);
        b.agent_started("win", "agent-1", "mac", "agent-3", true, 6);
        assert_eq!(b.chain_of("win", "agent-1", b"r").depth, 0);
    }

    /// Review of step 3, findings 1 and 2: a target can aim a result only at the machine that asked, and cannot launder its own
    /// agents into another machine's chain.
    #[test]
    fn a_target_cannot_aim_a_result_elsewhere_or_launder_its_own_agents() {
        let mut b = HelperBroker::default();
        b.carry("mac", "agent-3", "t", b"s1", root(), 0).unwrap();
        assert!(b.awaits("t", "mac", b"s1"));
        assert!(
            !b.awaits("t", "win", b"s1"),
            "only the asking machine is the result's destination"
        );
        // An agent the hub already knew (not new) is not taken into the chain.
        b.result("t", "mac", b"s1", "agent-old", 1);
        b.agent_started("t", "agent-old", "mac", "agent-3", false, 2);
        assert_eq!(b.chain_of("t", "agent-old", b"r").depth, 0);
        // One whose own start names another parent neither.
        b.carry("mac", "agent-3", "t", b"s2", root(), 3).unwrap();
        b.result("t", "mac", b"s2", "agent-9", 4);
        b.agent_started("t", "agent-9", "mac", "agent-4", true, 5);
        assert_eq!(b.chain_of("t", "agent-9", b"r").depth, 0);
        b.carry("mac", "agent-3", "t", b"s3", root(), 6).unwrap();
        b.result("t", "mac", b"s3", "agent-10", 7);
        b.agent_started("t", "agent-10", "win", "agent-3", true, 8);
        assert_eq!(b.chain_of("t", "agent-10", b"r").depth, 0);
        // A start that never reached its target leaves nothing waiting.
        b.carry("mac", "agent-3", "gone", b"s4", root(), 9).unwrap();
        b.abandon("mac", "gone", b"s4");
        assert!(!b.awaits("gone", "mac", b"s4"));
    }

    #[test]
    fn each_asking_machine_has_its_rate_and_its_open_starts() {
        let mut b = HelperBroker::default();
        for i in 0..OPEN_STARTS {
            b.carry("mac", "a", "t", &[i as u8], root(), 0).unwrap();
        }
        assert_eq!(
            b.carry("mac", "a", "t", b"x", root(), 0),
            Err(Refusal::Open)
        );
        // Another machine is counted apart.
        assert!(b.carry("win", "a", "t", b"x", root(), 0).is_ok());
        for _ in 0..OPEN_STARTS {
            b.first_answer("mac");
        }
        for i in OPEN_STARTS..STARTS_PER_MINUTE {
            b.carry("mac", "a", "t", &[i as u8], root(), 1).unwrap();
            b.first_answer("mac");
        }
        assert_eq!(
            b.carry("mac", "a", "t", b"y", root(), 2),
            Err(Refusal::Rate)
        );
        // A minute later the rate allows again.
        assert!(b.carry("mac", "a", "t", b"y", root(), 60_001).is_ok());
    }
}

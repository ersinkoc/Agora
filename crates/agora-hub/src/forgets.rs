//! ADR-0108: the forgets the hub still owes each node. A person who forgets a closed agent in the window (`agent.forget`, ADR-0053)
//! makes the hub pass the forget on to the agent's node, which removes the agent's saved conversation and its Resume offer. The agent's
//! machine is often offline then, so the hub keeps, per node, the node's own ids of the agents forgotten and not yet answered, in its
//! settings (`forget.<node>`, a JSON list of ids, oldest first), so a hub restart keeps them too. Sent to a node that offers
//! `agent.forget.local` at once and again after each of its connections opens (`server.rs`); an entry goes when the node answers,
//! whatever the answer. At most [`MAX_PER_NODE`] per node, the oldest dropped first (retention still removes those files on the
//! node). Forgetting the machine (ADR-0073) drops its list.

use std::sync::{Arc, Mutex};

use crate::store::{EventStore, SqliteStore, StoreError};

/// The most forgets kept for one node.
pub const MAX_PER_NODE: usize = 1000;

/// The pending forgets of every node, in the hub's settings.
pub struct Forgets {
    store: Arc<SqliteStore>,
    /// One read-modify-write at a time, so two forgets of the same node's agents never lose one.
    lock: Mutex<()>,
}

fn key(node: &str) -> String {
    format!("forget.{node}")
}

impl Forgets {
    pub fn new(store: Arc<SqliteStore>) -> Self {
        Self {
            store,
            lock: Mutex::new(()),
        }
    }

    /// What is stored for `node`; a value the hub did not write reads as nothing.
    fn read(&self, node: &str) -> Vec<String> {
        self.store
            .setting(&key(node))
            .ok()
            .flatten()
            .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
            .unwrap_or_default()
    }

    fn write(&self, node: &str, ids: &[String]) -> Result<(), StoreError> {
        let bytes = serde_json::to_vec(ids).map_err(|e| StoreError(e.to_string()))?;
        self.store.set_setting(&key(node), &bytes)
    }

    fn change(
        &self,
        node: &str,
        f: impl FnOnce(&mut Vec<String>) -> bool,
    ) -> Result<(), StoreError> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut ids = self.read(node);
        if f(&mut ids) {
            self.write(node, &ids)?;
        }
        Ok(())
    }

    /// The person forgot `agent` (the node's own id) of `node`: owed until the node answers. Once only; the oldest go past
    /// [`MAX_PER_NODE`].
    pub fn add(&self, node: &str, agent: &str) -> Result<(), StoreError> {
        self.change(node, |ids| {
            if ids.iter().any(|a| a == agent) {
                return false;
            }
            ids.push(agent.to_string());
            let over = ids.len().saturating_sub(MAX_PER_NODE);
            ids.drain(..over);
            true
        })
    }

    /// The forgets owed to `node`, oldest first.
    pub fn pending(&self, node: &str) -> Vec<String> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        self.read(node)
    }

    /// The node answered the forget of `agent`, whatever the answer: it is no longer owed.
    pub fn answered(&self, node: &str, agent: &str) -> Result<(), StoreError> {
        self.change(node, |ids| {
            let before = ids.len();
            ids.retain(|a| a != agent);
            ids.len() != before
        })
    }

    /// ADR-0073: the machine was forgotten: nothing is owed to it any more.
    pub fn drop_node(&self, node: &str) -> Result<(), StoreError> {
        self.change(node, |ids| {
            let had = !ids.is_empty();
            ids.clear();
            had
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn forgets() -> Forgets {
        Forgets::new(Arc::new(SqliteStore::open_in_memory().unwrap()))
    }

    #[test]
    fn each_node_keeps_its_own_list_once_each_oldest_first_and_an_answer_removes_one() {
        let f = forgets();
        f.add("laptop", "agent-1").unwrap();
        f.add("laptop", "agent-2").unwrap();
        f.add("laptop", "agent-1").unwrap();
        f.add("vps", "agent-1").unwrap();
        assert_eq!(f.pending("laptop"), vec!["agent-1", "agent-2"]);
        f.answered("laptop", "agent-1").unwrap();
        assert_eq!(f.pending("laptop"), vec!["agent-2"]);
        assert_eq!(
            f.pending("vps"),
            vec!["agent-1"],
            "another node's list stays"
        );
        f.answered("laptop", "agent-9").unwrap();
        f.drop_node("laptop").unwrap();
        assert!(f.pending("laptop").is_empty());
        assert_eq!(f.pending("vps"), vec!["agent-1"]);
    }

    #[test]
    fn past_the_bound_the_oldest_go_first() {
        let f = forgets();
        for i in 0..MAX_PER_NODE + 5 {
            f.add("laptop", &format!("agent-{i}")).unwrap();
        }
        let p = f.pending("laptop");
        assert_eq!(p.len(), MAX_PER_NODE);
        assert_eq!(p[0], "agent-5");
        assert_eq!(p[MAX_PER_NODE - 1], format!("agent-{}", MAX_PER_NODE + 4));
    }

    #[test]
    fn a_value_the_hub_did_not_write_reads_as_nothing() {
        let f = forgets();
        f.store.set_setting("forget.laptop", b"not json").unwrap();
        assert!(f.pending("laptop").is_empty());
        f.add("laptop", "agent-1").unwrap();
        assert_eq!(f.pending("laptop"), vec!["agent-1"]);
    }
}

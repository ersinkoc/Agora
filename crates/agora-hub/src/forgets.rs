//! ADR-0108: the forgets the hub still owes each node. A person who forgets a closed agent in the window (`agent.forget`, ADR-0053)
//! makes the hub pass the forget on to the agent's node, which removes the agent's saved conversation and its Resume offer. The agent's
//! machine is often offline then, so the hub keeps, per node, the node's own ids of the agents forgotten and not yet answered, in its
//! settings (`forget.<node>`, a JSON list, oldest first), so a hub restart keeps them too. Sent to a node that offers
//! `agent.forget.local` at once and again after each of its connections opens (`server.rs`). An entry goes when the node answers with
//! a decision; an answer that says it could not decide now (`node.offline`, `node.timeout`, `internal`) keeps it for the next
//! connection, at most [`MAX_TRANSIENT`] times (ADR-0108 amendment A). At most [`MAX_PER_NODE`] per node, the oldest dropped first
//! (retention still removes those files on the node). Forgetting the machine (ADR-0073) drops its list.
//!
//! Each list item is the agent's id as a string, or `{"agent": id, "transient": n}` once it has had `n` such answers: a list written
//! before amendment A (strings only) reads as it was.

use std::sync::{Arc, Mutex};

use crate::store::{EventStore, SqliteStore, StoreError};

/// The most forgets kept for one node.
pub const MAX_PER_NODE: usize = 1000;

/// ADR-0108 amendment A: the most answers that say the node could not decide now (`node.offline`, `node.timeout`, `internal`) one
/// forget gets before it is dropped, so a node that always fails it is not sent it for ever. The hub sends a kept forget again after
/// the node's next `Welcome`, or when the person forgets the agent again; every such answer counts.
pub const MAX_TRANSIENT: u32 = 5;

/// One forget owed: the node's own id of the agent and how many passing failures it has had.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Owed {
    agent: String,
    transient: u32,
}

impl Owed {
    fn from_json(v: &serde_json::Value) -> Option<Self> {
        match v {
            serde_json::Value::String(agent) => Some(Self {
                agent: agent.clone(),
                transient: 0,
            }),
            serde_json::Value::Object(o) => Some(Self {
                agent: o.get("agent")?.as_str()?.to_string(),
                transient: u32::try_from(o.get("transient")?.as_u64()?).ok()?,
            }),
            _ => None,
        }
    }

    /// A plain string while it has had no passing failure, as the list was written before amendment A.
    fn to_json(&self) -> serde_json::Value {
        if self.transient == 0 {
            serde_json::Value::String(self.agent.clone())
        } else {
            serde_json::json!({ "agent": self.agent, "transient": self.transient })
        }
    }
}

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
    fn read(&self, node: &str) -> Vec<Owed> {
        let Some(items) = self
            .store
            .setting(&key(node))
            .ok()
            .flatten()
            .and_then(|b| serde_json::from_slice::<Vec<serde_json::Value>>(&b).ok())
        else {
            return Vec::new();
        };
        items
            .iter()
            .map(Owed::from_json)
            .collect::<Option<Vec<_>>>()
            .unwrap_or_default()
    }

    fn write(&self, node: &str, owed: &[Owed]) -> Result<(), StoreError> {
        let items: Vec<serde_json::Value> = owed.iter().map(Owed::to_json).collect();
        let bytes = serde_json::to_vec(&items).map_err(|e| StoreError(e.to_string()))?;
        self.store.set_setting(&key(node), &bytes)
    }

    fn change(&self, node: &str, f: impl FnOnce(&mut Vec<Owed>) -> bool) -> Result<(), StoreError> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut owed = self.read(node);
        if f(&mut owed) {
            self.write(node, &owed)?;
        }
        Ok(())
    }

    /// The person forgot `agent` (the node's own id) of `node`: owed until the node answers. Once only; the oldest go past
    /// [`MAX_PER_NODE`].
    pub fn add(&self, node: &str, agent: &str) -> Result<(), StoreError> {
        self.change(node, |owed| {
            if owed.iter().any(|o| o.agent == agent) {
                return false;
            }
            owed.push(Owed {
                agent: agent.to_string(),
                transient: 0,
            });
            let over = owed.len().saturating_sub(MAX_PER_NODE);
            owed.drain(..over);
            true
        })
    }

    /// The forgets owed to `node`, oldest first.
    pub fn pending(&self, node: &str) -> Vec<String> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        self.read(node).into_iter().map(|o| o.agent).collect()
    }

    /// The node answered the forget of `agent` with a decision (removed, unknown, refused while the agent runs there, refused by its
    /// grants or policy, an invalid id): it is no longer owed.
    pub fn answered(&self, node: &str, agent: &str) -> Result<(), StoreError> {
        self.change(node, |owed| {
            let before = owed.len();
            owed.retain(|o| o.agent != agent);
            owed.len() != before
        })
    }

    /// ADR-0108 amendment A: the node answered the forget of `agent` with a passing failure (`node.offline`, `node.timeout`,
    /// `internal`): it stays owed, to be sent again after the node's next `Welcome`, unless this was its [`MAX_TRANSIENT`]th such
    /// answer; then it is dropped. True when it is still owed.
    pub fn failed_for_now(&self, node: &str, agent: &str) -> Result<bool, StoreError> {
        let mut kept = false;
        self.change(node, |owed| {
            let Some(i) = owed.iter().position(|o| o.agent == agent) else {
                return false;
            };
            owed[i].transient = owed[i].transient.saturating_add(1);
            if owed[i].transient >= MAX_TRANSIENT {
                owed.remove(i);
            } else {
                kept = true;
            }
            true
        })?;
        Ok(kept)
    }

    /// ADR-0073: the machine was forgotten: nothing is owed to it any more.
    pub fn drop_node(&self, node: &str) -> Result<(), StoreError> {
        self.change(node, |owed| {
            let had = !owed.is_empty();
            owed.clear();
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

    /// Amendment A: a passing failure keeps the forget, counted; the [`MAX_TRANSIENT`]th drops it; a decision drops it at any time.
    #[test]
    fn a_passing_failure_keeps_a_forget_until_the_bound() {
        let f = forgets();
        f.add("laptop", "agent-1").unwrap();
        f.add("laptop", "agent-2").unwrap();
        for i in 1..MAX_TRANSIENT {
            assert!(f.failed_for_now("laptop", "agent-1").unwrap(), "after {i}");
            assert_eq!(f.pending("laptop"), vec!["agent-1", "agent-2"]);
        }
        // Forgetting it again while owed neither adds it twice nor starts its count again.
        f.add("laptop", "agent-1").unwrap();
        assert!(!f.failed_for_now("laptop", "agent-1").unwrap());
        assert_eq!(f.pending("laptop"), vec!["agent-2"]);
        // Not owed: nothing to count.
        assert!(!f.failed_for_now("laptop", "agent-1").unwrap());
        assert!(f.failed_for_now("laptop", "agent-2").unwrap());
        f.answered("laptop", "agent-2").unwrap();
        assert!(f.pending("laptop").is_empty());
        // Forgotten again after it was dropped: owed afresh, with a fresh count.
        f.add("laptop", "agent-1").unwrap();
        assert!(f.failed_for_now("laptop", "agent-1").unwrap());
        f.drop_node("laptop").unwrap();
        assert!(f.pending("laptop").is_empty());
    }

    /// A list written before amendment A (plain ids) reads as it was, and a counted entry is written so that it reads back the same.
    #[test]
    fn a_list_of_plain_ids_reads_as_before_and_counts_survive_a_write() {
        let f = forgets();
        f.store
            .set_setting("forget.laptop", br#"["agent-1","agent-2"]"#)
            .unwrap();
        assert_eq!(f.pending("laptop"), vec!["agent-1", "agent-2"]);
        assert!(f.failed_for_now("laptop", "agent-2").unwrap());
        let stored = f.store.setting("forget.laptop").unwrap().unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&stored).unwrap(),
            serde_json::json!(["agent-1", {"agent": "agent-2", "transient": 1}])
        );
        let again = Forgets::new(Arc::clone(&f.store));
        for _ in 1..MAX_TRANSIENT - 1 {
            assert!(again.failed_for_now("laptop", "agent-2").unwrap());
        }
        assert!(!again.failed_for_now("laptop", "agent-2").unwrap());
        assert_eq!(again.pending("laptop"), vec!["agent-1"]);
    }

    #[test]
    fn a_value_the_hub_did_not_write_reads_as_nothing() {
        let f = forgets();
        f.store.set_setting("forget.laptop", b"not json").unwrap();
        assert!(f.pending("laptop").is_empty());
        f.store
            .set_setting("forget.laptop", br#"["agent-1", {"agent": 7}]"#)
            .unwrap();
        assert!(f.pending("laptop").is_empty());
        f.add("laptop", "agent-1").unwrap();
        assert_eq!(f.pending("laptop"), vec!["agent-1"]);
    }
}

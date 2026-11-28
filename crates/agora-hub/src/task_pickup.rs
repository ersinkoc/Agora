//! ADR-0131 (with Amendment A): an idle agent's automatic pickup. The owner's per-project settings (on or off, and the allow-list of
//! node ids and agent kinds) kept in the hub's settings store, unsigned since a node keeps its own opt-in; the limits on requests; and
//! the outcome row each run's latest request leaves (its sequence and result), keyed by the project, the node of the link and the run,
//! so a node speaks only for its own runs.

use std::collections::BTreeMap;

use agora_protocol::v1;
use prost::Message;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::store::{EventStore, SqliteStore, StoreError};

const SETTINGS_PREFIX: &str = "task_pickup.settings.";
const OUTCOME_PREFIX: &str = "task_pickup.outcome.";
/// The board tool an idle agent calls.
pub const TOOL: &str = "task_take_next";
/// The rule a pickup names (`TaskPickedUp.rule`).
pub const RULE: &str = "take_next";
/// Requests taken per second: per run, per node, per project.
pub const PER_RUN: u32 = 1;
pub const PER_NODE: u32 = 5;
pub const PER_PROJECT: u32 = 20;
/// The most entries of each allow-list, and the longest entry.
pub const LIST_MAX: usize = 32;
pub const ENTRY_MAX: usize = 64;

/// An allow-list entry (a node id or an agent kind): 1 to 64 of `[A-Za-z0-9._-]`.
pub fn valid_entry(entry: &str) -> bool {
    (1..=ENTRY_MAX).contains(&entry.len())
        && entry
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn settings_key(project: &str) -> String {
    format!("{SETTINGS_PREFIX}{project}")
}

/// The owner's settings for `project`; off with empty lists when none were saved. `active` and `inactive_reason` are the caller's.
pub fn settings(store: &SqliteStore, project: &str) -> Result<v1::TaskPickupSettings, StoreError> {
    let saved = store
        .setting(&settings_key(project))?
        .and_then(|b| v1::TaskPickupSettings::decode(b.as_slice()).ok())
        .unwrap_or_default();
    Ok(v1::TaskPickupSettings {
        project_id: project.to_string(),
        enabled: saved.enabled,
        node_ids: saved.node_ids,
        agent_kinds: saved.agent_kinds,
        active: false,
        inactive_reason: String::new(),
    })
}

/// A list as kept: each entry checked, duplicates dropped, sorted. `None` when the list is too long or holds an entry that is not one.
fn kept(list: &[String]) -> Option<Vec<String>> {
    if list.len() > LIST_MAX || !list.iter().all(|e| valid_entry(e)) {
        return None;
    }
    let mut out = list.to_vec();
    out.sort();
    out.dedup();
    Some(out)
}

/// Saves the owner's settings for `project`. `Err(field)` for a list that is too long or holds an entry that is not a node id or kind.
pub fn set_settings(
    store: &SqliteStore,
    set: &v1::TaskPickupSettingsSet,
    project: &str,
) -> Result<Result<(), &'static str>, StoreError> {
    let Some(node_ids) = kept(&set.node_ids) else {
        return Ok(Err("node_ids"));
    };
    let Some(agent_kinds) = kept(&set.agent_kinds) else {
        return Ok(Err("agent_kinds"));
    };
    let saved = v1::TaskPickupSettings {
        project_id: project.to_string(),
        enabled: set.enabled,
        node_ids,
        agent_kinds,
        active: false,
        inactive_reason: String::new(),
    };
    store.set_setting(&settings_key(project), &saved.encode_to_vec())?;
    Ok(Ok(()))
}

/// The requests of the current second, per run, node and project. A request over any limit is refused and not counted.
#[derive(Debug, Default)]
pub struct Rate {
    second: u64,
    runs: BTreeMap<String, u32>,
    nodes: BTreeMap<String, u32>,
    projects: BTreeMap<String, u32>,
}

impl Rate {
    /// Takes one request, or says the limit it would pass (per second).
    pub fn admit(&mut self, now_ms: u64, project: &str, node: &str, run: &str) -> Result<(), u32> {
        let second = now_ms / 1000;
        if second != self.second {
            self.second = second;
            self.runs.clear();
            self.nodes.clear();
            self.projects.clear();
        }
        let run_key = format!("{node}\u{0}{run}");
        for (map, key, limit) in [
            (&self.runs, run_key.as_str(), PER_RUN),
            (&self.nodes, node, PER_NODE),
            (&self.projects, project, PER_PROJECT),
        ] {
            if map.get(key).copied().unwrap_or(0) >= limit {
                return Err(limit);
            }
        }
        *self.runs.entry(run_key).or_default() += 1;
        *self.nodes.entry(node.to_string()).or_default() += 1;
        *self.projects.entry(project.to_string()).or_default() += 1;
        Ok(())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The start of the SHA-256 of length-prefixed parts, as hex: no part's text can run into another's.
fn digest(parts: &[&[u8]]) -> String {
    let mut h = Sha256::new();
    for part in parts {
        h.update((part.len() as u64).to_be_bytes());
        h.update(part);
    }
    hex(&h.finalize()[..16])
}

/// The prefix of one project's outcome rows.
fn outcomes_prefix(project: &str) -> String {
    format!("{OUTCOME_PREFIX}{}.", digest(&[project.as_bytes()]))
}

/// The settings key of the outcome row of `run` of `node` in `project`.
pub fn outcome_key(project: &str, node: &str, run: &[u8; 16]) -> String {
    format!(
        "{}{}",
        outcomes_prefix(project),
        digest(&[node.as_bytes(), run])
    )
}

/// A run's latest committed request: its sequence and the result it got (the tool's JSON, without the MCP wrapping).
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub node: String,
    pub agent: String,
    pub run: [u8; 16],
    pub seq: u64,
    pub result: Value,
}

impl Outcome {
    pub fn encode(&self) -> Vec<u8> {
        json!({"node": self.node, "agent": self.agent, "run": hex(&self.run), "seq": self.seq, "result": self.result})
            .to_string()
            .into_bytes()
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let v: Value = serde_json::from_slice(bytes).ok()?;
        let run_hex = v["run"].as_str()?;
        let mut run = [0u8; 16];
        if run_hex.len() != 32 {
            return None;
        }
        for (i, b) in run.iter_mut().enumerate() {
            *b = u8::from_str_radix(run_hex.get(2 * i..2 * i + 2)?, 16).ok()?;
        }
        Some(Self {
            node: v["node"].as_str()?.to_string(),
            agent: v["agent"].as_str()?.to_string(),
            run,
            seq: v["seq"].as_u64()?,
            result: v["result"].clone(),
        })
    }
}

/// The outcome row at `key`; none when there is none or it is not one the hub wrote.
pub fn outcome(store: &SqliteStore, key: &str) -> Result<Option<Outcome>, StoreError> {
    Ok(store.setting(key)?.and_then(|b| Outcome::decode(&b)))
}

/// Drops the outcome rows of `project` whose run is not running now (`running(node, agent, run)` says), so the rows are bounded by
/// the project's running runs.
pub fn prune(
    store: &SqliteStore,
    project: &str,
    running: impl Fn(&str, &str, &[u8; 16]) -> bool,
) -> Result<(), StoreError> {
    for bytes in store.settings_with_prefix(&outcomes_prefix(project))? {
        let Some(o) = Outcome::decode(&bytes) else {
            continue;
        };
        if !running(&o.node, &o.agent, &o.run) {
            store.remove_setting(&outcome_key(project, &o.node, &o.run))?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_owners_lists_are_checked_kept_sorted_and_read_back() {
        let path = std::env::temp_dir().join(format!("agora-pickup-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let store = SqliteStore::open(&path).unwrap();
        assert!(
            !settings(&store, "p").unwrap().enabled,
            "off until the owner turns it on"
        );
        let set = v1::TaskPickupSettingsSet {
            project_id: "p".into(),
            enabled: true,
            node_ids: vec!["n2".into(), "n1".into(), "n2".into()],
            agent_kinds: vec!["claude-code".into()],
        };
        assert_eq!(set_settings(&store, &set, "p").unwrap(), Ok(()));
        let s = settings(&store, "p").unwrap();
        assert!(s.enabled);
        assert_eq!(s.node_ids, ["n1", "n2"]);
        assert_eq!(s.agent_kinds, ["claude-code"]);
        assert!(!settings(&store, "q").unwrap().enabled, "per project");

        for (field, bad) in [
            (
                "node_ids",
                v1::TaskPickupSettingsSet {
                    node_ids: vec!["a b".into()],
                    ..set.clone()
                },
            ),
            (
                "node_ids",
                v1::TaskPickupSettingsSet {
                    node_ids: vec![String::new()],
                    ..set.clone()
                },
            ),
            (
                "agent_kinds",
                v1::TaskPickupSettingsSet {
                    agent_kinds: vec!["k".repeat(65)],
                    ..set.clone()
                },
            ),
            (
                "agent_kinds",
                v1::TaskPickupSettingsSet {
                    agent_kinds: (0..33).map(|i| format!("k{i}")).collect(),
                    ..set.clone()
                },
            ),
        ] {
            assert_eq!(set_settings(&store, &bad, "p").unwrap(), Err(field));
        }
        assert_eq!(
            settings(&store, "p").unwrap().node_ids,
            ["n1", "n2"],
            "a refused write changes nothing"
        );
        drop(store);
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod more_tests {
    use super::*;

    #[test]
    fn requests_are_limited_per_run_node_and_project_each_second() {
        let mut r = Rate::default();
        assert_eq!(r.admit(1_000, "p", "n1", "r1"), Ok(()));
        assert_eq!(
            r.admit(1_500, "p", "n1", "r1"),
            Err(PER_RUN),
            "one per run per second"
        );
        for i in 2..=5 {
            assert_eq!(r.admit(1_500, "p", "n1", &format!("r{i}")), Ok(()));
        }
        assert_eq!(
            r.admit(1_600, "p", "n1", "r6"),
            Err(PER_NODE),
            "minting runs does not get round the node's limit"
        );
        for n in 2..=16 {
            let _ = r.admit(1_700, "p", &format!("n{n}"), "r");
        }
        assert_eq!(r.admit(1_800, "p", "n17", "r"), Err(PER_PROJECT));
        assert_eq!(r.admit(1_800, "q", "n17", "r"), Ok(()), "per project");
        assert_eq!(r.admit(2_000, "p", "n1", "r1"), Ok(()), "a new second");
    }

    #[test]
    fn outcome_rows_are_the_runs_own_and_go_with_the_run() {
        let path =
            std::env::temp_dir().join(format!("agora-pickup-rows-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let store = SqliteStore::open(&path).unwrap();
        let row = |node: &str, run: u8| Outcome {
            node: node.into(),
            agent: "a".into(),
            run: [run; 16],
            seq: 2,
            result: json!({"outcome": "empty", "request_seq": 2}),
        };
        assert_ne!(
            outcome_key("p", "n1", &[1; 16]),
            outcome_key("p", "n2", &[1; 16]),
            "another node's run is another row"
        );
        assert_ne!(
            outcome_key("p", "n1", &[1; 16]),
            outcome_key("p/x", "n1", &[1; 16])
        );
        for o in [row("n1", 1), row("n2", 1)] {
            store
                .set_setting(&outcome_key("p", &o.node, &o.run), &o.encode())
                .unwrap();
        }
        assert_eq!(
            outcome(&store, &outcome_key("p", "n1", &[1; 16])).unwrap(),
            Some(row("n1", 1))
        );
        prune(&store, "p", |node, _, _| node == "n1").unwrap();
        assert!(outcome(&store, &outcome_key("p", "n1", &[1; 16]))
            .unwrap()
            .is_some());
        assert!(
            outcome(&store, &outcome_key("p", "n2", &[1; 16]))
                .unwrap()
                .is_none(),
            "an ended run's row is dropped"
        );
        drop(store);
        let _ = std::fs::remove_file(&path);
    }
}

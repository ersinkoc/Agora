//! ADR-0131 (with Amendment A): an idle agent's automatic pickup. This part is the owner's: the per-project settings (on or off, and the
//! allow-list of node ids and agent kinds) kept in the hub's settings store, unsigned since a node keeps its own opt-in.

use agora_protocol::v1;
use prost::Message;

use crate::store::{EventStore, SqliteStore, StoreError};

const SETTINGS_PREFIX: &str = "task_pickup.settings.";
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

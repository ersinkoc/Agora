//! What each trusted device may sign on this node (ADR-0049). A device the node's `policy` file names with
//! `device.<id>.projects = a, b` can sign grants and approvals only for those projects; a device it does not name is limited by
//! nothing, as before. The hub cannot change the file, so a compromised hub can neither promote a member's device nor sign for it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

type Resolver = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

#[derive(Default)]
pub struct DeviceLimits {
    /// Device id to the projects it may act in. An entry with no projects is a device that may act in none.
    map: BTreeMap<String, BTreeSet<String>>,
    /// Agent id to project id.
    project_of: Mutex<Option<Resolver>>,
    /// ADR-0061: the trusted-devices file, what was last read from it, and the limits devices there inherited from a limited endorser
    /// (`projects=`). A device the policy names keeps the policy's limits.
    inherited: Mutex<Option<Inherited>>,
}

type Inherited = (
    std::path::PathBuf,
    Option<Vec<u8>>,
    Vec<crate::device_trust::Entry>,
);

impl DeviceLimits {
    pub fn new(map: BTreeMap<String, BTreeSet<String>>) -> Self {
        Self {
            map,
            project_of: Mutex::new(None),
            inherited: Mutex::new(None),
        }
    }

    /// Follows the trusted-devices file for limits inherited through endorsements (ADR-0061), read again whenever it changes.
    pub fn follow_trust_file(&self, path: &std::path::Path) {
        *self.inherited.lock().unwrap_or_else(|e| e.into_inner()) =
            Some((path.to_path_buf(), None, Vec::new()));
    }

    /// What `device` may act in: its policy line, what it inherited through an endorsement, and what its endorser may act in now, up
    /// the chain ([`crate::device_trust::effective_limit`]); `None`: not limited.
    fn limit(&self, device: &str) -> Option<BTreeSet<String>> {
        let mut guard = self.inherited.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_mut() {
            None => self.map.get(device).cloned(),
            Some((path, seen, entries)) => {
                let now = std::fs::read(&*path).ok();
                if now != *seen {
                    *entries = crate::device_trust::read_entries(path).0;
                    *seen = now;
                }
                crate::device_trust::effective_limit(entries, &self.map, device)
            }
        }
    }

    /// Is `device` limited to some projects (by the policy file or through an endorsement)?
    pub fn is_limited(&self, device: &str) -> bool {
        self.limit(device).is_some()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// How to find the project of an agent, for grants and approvals that name an agent.
    pub fn set_project_resolver(&self, f: Resolver) {
        *self.project_of.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
    }

    /// May `device` act in `project`? A device that is not limited may.
    pub fn allows_project(&self, device: &str, project: &str) -> bool {
        self.limit(device).is_none_or(|p| p.contains(project))
    }

    /// May `device` act for `agent`? A limited device may only when the agent's project is known and its own.
    pub fn allows_agent(&self, device: &str, agent: &str) -> bool {
        let Some(projects) = self.limit(device) else {
            return true;
        };
        let guard = self.project_of.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .as_ref()
            .and_then(|f| f(agent))
            .is_some_and(|p| projects.contains(&p))
    }

    /// May `device` sign a grant of this scope (`node`, `agent:ID`, `project:ID`)? A limited device never signs a whole-machine grant.
    pub fn allows_scope(&self, device: &str, scope: &str) -> bool {
        if self.limit(device).is_none() {
            return true;
        }
        match scope.split_once(':') {
            Some(("project", id)) => self.allows_project(device, id),
            Some(("agent", id)) => self.allows_agent(device, id),
            _ => false,
        }
    }
}

/// A device's limit from the policy file and the one it inherited: both apply (their intersection); either alone applies as is.
pub fn combine(
    policy: Option<BTreeSet<String>>,
    inherited: Option<BTreeSet<String>>,
) -> Option<BTreeSet<String>> {
    match (policy, inherited) {
        (Some(p), Some(i)) => Some(p.intersection(&i).cloned().collect()),
        (p, i) => p.or(i),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn limits() -> DeviceLimits {
        let l = DeviceLimits::new(BTreeMap::from([
            ("eda".to_string(), BTreeSet::from(["web".to_string()])),
            ("nobody".to_string(), BTreeSet::new()),
        ]));
        l.set_project_resolver(Box::new(|a| match a {
            "a-web" => Some("web".into()),
            "a-api" => Some("api".into()),
            _ => None,
        }));
        l
    }

    #[test]
    fn a_limited_device_acts_only_in_its_projects_and_others_are_unchanged() {
        let l = limits();
        assert!(l.allows_scope("eda", "project:web") && !l.allows_scope("eda", "project:api"));
        assert!(l.allows_scope("eda", "agent:a-web") && !l.allows_scope("eda", "agent:a-api"));
        assert!(
            !l.allows_scope("eda", "agent:unknown"),
            "an agent it cannot place is refused"
        );
        assert!(
            !l.allows_scope("eda", "node"),
            "a whole-machine grant is the owner's"
        );
        assert!(!l.allows_scope("nobody", "project:web"));
        let _ = &l;
        assert!(l.allows_scope("owner-phone", "node") && l.allows_agent("owner-phone", "a-api"));
        assert!(!l.is_empty());
    }

    #[test]
    fn the_policy_file_names_the_limited_devices_and_a_typo_never_widens() {
        let dir = std::env::temp_dir().join(format!("agora-limits-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("policy");
        std::fs::write(
            &file,
            "device.eda.projects = web, docs\ndevice.empty.projects =\ndevice..projects = x\ndevice.eda.project = api\n",
        )
        .unwrap();
        let p = crate::daemon::read_policy(&file).device_projects;
        assert_eq!(p.len(), 2, "{p:?}");
        assert_eq!(
            p["eda"],
            BTreeSet::from(["web".to_string(), "docs".to_string()])
        );
        assert!(
            p["empty"].is_empty(),
            "a named device with no projects acts in none"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}

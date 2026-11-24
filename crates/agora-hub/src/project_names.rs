//! The projects a person can be granted (ADR-0049, #904), by the names the window shows: the machine's name and the project's folder
//! name. A grant is stored by the ids `Scope` compares (`node/project`), which the window never shows, so the owner names a project the
//! way the window does and this module finds its ids in the hub's view. Pure: the caller hands it the view (`View::rebuild(..).shown()`).
//!
//! The window names a project by the last part of its folder: a workspace's repository root (or its folder when it is no repository),
//! a checkout's folder (`apps/web/src/net/view.ts`). A machine is named by its host name; where two machines share one, by its id, as
//! the window does. One project may have several folders (two clones of one repository), and two projects may share a name: a name that
//! matches more than one project is refused with the candidates, and a folder's full path names one.

use std::collections::{BTreeMap, BTreeSet};

use crate::projection::{split_key, View};

/// One folder of a project, as the hub knows it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Folder {
    /// What the owner types for the machine: its host name, or its id where the host name does not tell it apart.
    pub machine: String,
    /// The project's name: the last part of `path`.
    pub name: String,
    /// The folder.
    pub path: String,
    /// The machine's host name (empty when it never said).
    pub host: String,
    /// The node id and project id a grant stores.
    pub node: String,
    pub project: String,
}

impl Folder {
    /// `machine/name`, what the owner types (and reads in `agora hub user list`).
    pub fn shown(&self) -> String {
        format!("{}/{}", self.machine, self.name)
    }
}

/// What a name the owner typed stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// Exactly one project: the node id and project id to store.
    One { node: String, project: String },
    /// No machine of that name or id.
    UnknownMachine,
    /// The machine is known but has no such project.
    UnknownProject,
    /// More than one project matches; every matching folder, to choose from.
    Ambiguous(Vec<Folder>),
}

/// The last part of a path, `/` or `\` separated (the window's `lastSegment`).
fn last_segment(path: &str) -> &str {
    path.split(['/', '\\'])
        .rfind(|p| !p.is_empty())
        .unwrap_or_default()
}

/// A path without the separators at its end, so `/srv/api/` and `/srv/api` are one folder.
fn trimmed(path: &str) -> &str {
    let t = path.trim_end_matches(['/', '\\']);
    if t.is_empty() {
        path
    } else {
        t
    }
}

/// Every node the view knows of → its host name (empty when it never said).
fn hosts(view: &View) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = view
        .nodes
        .iter()
        .map(|(id, n)| (id.clone(), n.hostname.clone()))
        .collect();
    for (key, (node, _, _)) in &view.checkouts {
        let node = split_key(key).map_or(node.as_str(), |(n, _)| n);
        out.entry(node.to_string()).or_default();
    }
    for w in view.workspaces.values() {
        out.entry(w.node_id.clone()).or_default();
    }
    for key in view.agents.keys() {
        if let Some((node, _)) = split_key(key) {
            out.entry(node.to_string()).or_default();
        }
    }
    out
}

/// What the owner types for each machine: the host name when no other machine has it as its name or id, otherwise the id.
fn machine_labels(hosts: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut count: BTreeMap<&str, usize> = BTreeMap::new();
    for h in hosts.values().filter(|h| !h.is_empty()) {
        *count.entry(h.as_str()).or_default() += 1;
    }
    hosts
        .iter()
        .map(|(id, h)| {
            let unique = !h.is_empty()
                && count.get(h.as_str()) == Some(&1)
                && hosts.keys().all(|other| other == id || other != h);
            (id.clone(), if unique { h.clone() } else { id.clone() })
        })
        .collect()
}

/// The folder a workspace stands for as its project's folder: its repository root, or its own folder when it is no repository (the
/// window's choice too). `Scope` places a member's folder commands by the same folders (ADR-0049 Amendment A).
pub fn workspace_root(w: &agora_protocol::v1::WorkspaceRegistered) -> &str {
    if w.git_root.is_empty() {
        &w.path
    } else {
        &w.git_root
    }
}

/// Every folder of every project the hub knows, sorted by machine, name and path. A folder reported twice is listed once.
pub fn folders(view: &View) -> Vec<Folder> {
    let hosts = hosts(view);
    let labels = machine_labels(&hosts);
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    let mut add = |node: &str, project: &str, path: &str| {
        if project.is_empty() || path.is_empty() || last_segment(path).is_empty() {
            return;
        }
        if !seen.insert((
            node.to_string(),
            project.to_string(),
            trimmed(path).to_string(),
        )) {
            return;
        }
        out.push(Folder {
            machine: labels.get(node).cloned().unwrap_or_else(|| node.into()),
            name: last_segment(path).into(),
            path: path.into(),
            host: hosts.get(node).cloned().unwrap_or_default(),
            node: node.into(),
            project: project.into(),
        });
    };
    // The machine is the one that reported it (the key's producer), as `Scope` places it.
    for (key, (node, project, path)) in &view.checkouts {
        let node = split_key(key).map_or(node.as_str(), |(n, _)| n);
        add(node, project, path);
    }
    for w in view.workspaces.values() {
        add(&w.node_id, &w.project_id, workspace_root(w));
    }
    out.sort();
    out
}

/// Finds the project `project` on `machine`, as the owner typed them. `machine` is a node id or a host name (an id first, since ids are
/// unique). `project` is the project's id, its name, or the full path of one of its folders; an id wins over a name.
pub fn resolve(view: &View, machine: &str, project: &str) -> Resolved {
    let all = folders(view);
    let hosts = hosts(view);
    let nodes: BTreeSet<&str> = if hosts.contains_key(machine) {
        BTreeSet::from([machine])
    } else {
        hosts
            .iter()
            .filter(|(_, h)| h.as_str() == machine)
            .map(|(id, _)| id.as_str())
            .collect()
    };
    if nodes.is_empty() {
        return Resolved::UnknownMachine;
    }
    let on: Vec<&Folder> = all
        .iter()
        .filter(|f| nodes.contains(f.node.as_str()))
        .collect();
    let by_id: Vec<&Folder> = on
        .iter()
        .copied()
        .filter(|f| f.project == project)
        .collect();
    // ADR-0060: a project the hub knows only from an agent (one it never saw start, its folder never reported) is still a project
    // `Scope` places, so its exact id is accepted; it has no folder, hence no name.
    if by_id.is_empty() {
        let from_agents: BTreeSet<&str> = view
            .agents
            .iter()
            .filter_map(|(k, a)| {
                let (node, _) = split_key(k)?;
                (nodes.contains(node) && a.project_id == project).then_some(node)
            })
            .collect();
        if from_agents.len() == 1 {
            let node = from_agents.into_iter().next().unwrap_or_default();
            return Resolved::One {
                node: node.into(),
                project: project.into(),
            };
        }
        if from_agents.len() > 1 {
            let labels = machine_labels(&hosts);
            return Resolved::Ambiguous(
                from_agents
                    .into_iter()
                    .map(|node| Folder {
                        machine: labels.get(node).cloned().unwrap_or_else(|| node.into()),
                        name: project.into(),
                        path: String::new(),
                        host: hosts.get(node).cloned().unwrap_or_default(),
                        node: node.into(),
                        project: project.into(),
                    })
                    .collect(),
            );
        }
    }
    let matching: Vec<&Folder> = if by_id.is_empty() {
        on.iter()
            .copied()
            .filter(|f| f.name == project || trimmed(&f.path) == trimmed(project))
            .collect()
    } else {
        by_id
    };
    let distinct: BTreeSet<(&str, &str)> = matching
        .iter()
        .map(|f| (f.node.as_str(), f.project.as_str()))
        .collect();
    match distinct.len() {
        0 => Resolved::UnknownProject,
        1 => {
            let (node, project) = distinct.into_iter().next().unwrap_or_default();
            Resolved::One {
                node: node.into(),
                project: project.into(),
            }
        }
        _ => Resolved::Ambiguous(matching.into_iter().cloned().collect()),
    }
}

/// How `agora hub user list` names a stored grant: `machine/name` of its first folder, with that folder's path when another project
/// has the same name there, or the stored `node/project` when the hub knows no folder of it (a project no machine reports any more, or a
/// row an older build stored without checking).
pub fn shown_grant(folders: &[Folder], node: &str, project: &str) -> (String, Option<String>) {
    let Some(f) = folders
        .iter()
        .find(|f| f.node == node && f.project == project)
    else {
        return (format!("{node}/{project}"), None);
    };
    let shared = folders.iter().any(|o| {
        o.machine == f.machine && o.name == f.name && (o.node != f.node || o.project != f.project)
    });
    (f.shown(), shared.then(|| f.path.clone()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use agora_protocol::v1::{self, event::Body, Event};

    fn ev(node: &str, body: Body) -> Event {
        Event {
            producer_id: node.into(),
            seq: 1,
            at_ms: 1,
            body: Some(body),
        }
    }
    fn online(node: &str, host: &str) -> Event {
        ev(
            node,
            Body::NodeOnline(v1::NodeOnline {
                node_id: node.into(),
                hostname: host.into(),
                ..Default::default()
            }),
        )
    }
    fn checkout(node: &str, id: &str, project: &str, path: &str) -> Event {
        ev(
            node,
            Body::CheckoutDiscovered(v1::CheckoutDiscovered {
                checkout_id: id.into(),
                node_id: node.into(),
                project_id: project.into(),
                path: path.into(),
            }),
        )
    }
    fn workspace(node: &str, id: &str, project: &str, path: &str, root: &str) -> Event {
        ev(
            node,
            Body::WorkspaceRegistered(v1::WorkspaceRegistered {
                workspace_id: id.into(),
                node_id: node.into(),
                project_id: project.into(),
                path: path.into(),
                git_root: root.into(),
                ..Default::default()
            }),
        )
    }
    fn view(events: &[Event]) -> View {
        let mut v = View::default();
        for e in events {
            v.apply(e);
        }
        v
    }
    fn one(node: &str, project: &str) -> Resolved {
        Resolved::One {
            node: node.into(),
            project: project.into(),
        }
    }

    #[test]
    fn a_project_is_found_by_the_names_the_window_shows() {
        let v = view(&[
            online("n1", "laptop"),
            checkout("n1", "c1", "#root", "/home/a/web"),
            // A folder inside a repository is named by the repository, a folder that is none by itself (the window's rule).
            workspace("n1", "w1", "#root2", "/home/a/site/docs", "/home/a/site"),
            workspace("n1", "w2", "id:u", r"C:\work\notes\", ""),
        ]);
        assert_eq!(resolve(&v, "laptop", "web"), one("n1", "#root"));
        assert_eq!(resolve(&v, "laptop", "site"), one("n1", "#root2"));
        assert_eq!(resolve(&v, "laptop", "notes"), one("n1", "id:u"));
        // The machine's id, the project's id and a folder's full path name it too.
        assert_eq!(resolve(&v, "n1", "web"), one("n1", "#root"));
        assert_eq!(resolve(&v, "laptop", "#root"), one("n1", "#root"));
        assert_eq!(resolve(&v, "laptop", "/home/a/web/"), one("n1", "#root"));
        assert_eq!(resolve(&v, "laptop", r"C:\work\notes"), one("n1", "id:u"));
        assert_eq!(
            folders(&v).iter().map(Folder::shown).collect::<Vec<_>>(),
            ["laptop/notes", "laptop/site", "laptop/web"]
        );
    }

    #[test]
    fn an_unknown_name_is_refused_and_a_name_for_two_projects_lists_both() {
        let v = view(&[
            online("n1", "laptop"),
            checkout("n1", "c1", "#a", "/home/a/api"),
            checkout("n1", "c2", "#b", "/srv/api"),
            // Two clones of one repository are one project: not ambiguous.
            checkout("n1", "c3", "#w", "/home/a/web"),
            checkout("n1", "c4", "#w", "/tmp/web"),
        ]);
        assert_eq!(resolve(&v, "laptop", "proj"), Resolved::UnknownProject);
        assert_eq!(resolve(&v, "desk", "web"), Resolved::UnknownMachine);
        // An id the hub has never seen is no project either (it used to be stored and showed nothing).
        assert_eq!(
            resolve(&v, "laptop", "directory:/x"),
            Resolved::UnknownProject
        );
        assert_eq!(resolve(&v, "laptop", "web"), one("n1", "#w"));
        let Resolved::Ambiguous(c) = resolve(&v, "laptop", "api") else {
            panic!("api names two projects")
        };
        assert_eq!(
            c.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            ["/home/a/api", "/srv/api"]
        );
        assert_eq!(resolve(&v, "laptop", "/srv/api"), one("n1", "#b"));
    }

    #[test]
    fn machines_that_share_a_host_name_are_told_apart_by_their_id() {
        let v = view(&[
            online("n2", "box"),
            checkout("n2", "c1", "#o2", "/opt/ops"),
            online("n3", "box"),
            checkout("n3", "c1", "#o3", "/opt/ops"),
            // A machine whose host name is another machine's id is named by its own id, so the label always resolves back to it.
            online("n4", "n2"),
            checkout("n4", "c1", "#x", "/x/tools"),
        ]);
        let shown: Vec<String> = folders(&v).iter().map(Folder::shown).collect();
        assert_eq!(shown, ["n2/ops", "n3/ops", "n4/tools"]);
        let Resolved::Ambiguous(c) = resolve(&v, "box", "ops") else {
            panic!("two machines are called box")
        };
        assert_eq!(c.len(), 2);
        assert_eq!(resolve(&v, "n3", "ops"), one("n3", "#o3"));
        assert_eq!(resolve(&v, "n2", "ops"), one("n2", "#o2"));
        for f in folders(&v) {
            assert_eq!(resolve(&v, &f.machine, &f.name), one(&f.node, &f.project));
        }
    }

    #[test]
    fn a_checkout_is_placed_on_the_machine_that_reported_it() {
        // T19: the body names n9, but n1 said it; the grant must be the one `Scope` compares.
        let v = view(&[online("n1", "laptop"), checkout("n9", "c1", "#p", "/p/app")]);
        let mut forged = checkout("n9", "c2", "#q", "/q/lib");
        forged.producer_id = "n1".into();
        let v = {
            let mut v = v;
            v.apply(&forged);
            v
        };
        assert_eq!(resolve(&v, "laptop", "lib"), one("n1", "#q"));
        assert_eq!(resolve(&v, "n9", "lib"), Resolved::UnknownProject);
    }

    #[test]
    fn a_project_known_only_from_an_agent_is_granted_by_its_exact_id() {
        let started = |node: &str, project: &str| {
            ev(
                node,
                Body::AgentStarted(v1::AgentStarted {
                    agent_id: "a1".into(),
                    project_id: project.into(),
                    node_id: node.into(),
                    kind: "codex".into(),
                    ..Default::default()
                }),
            )
        };
        let v = view(&[online("n1", "laptop"), started("n1", "#lost")]);
        assert_eq!(resolve(&v, "laptop", "#lost"), one("n1", "#lost"));
        assert_eq!(resolve(&v, "laptop", "lost"), Resolved::UnknownProject);
        let v = view(&[
            online("n2", "box"),
            started("n2", "#p"),
            online("n3", "box"),
            started("n3", "#p"),
        ]);
        let Resolved::Ambiguous(c) = resolve(&v, "box", "#p") else {
            panic!("two machines are called box")
        };
        assert_eq!(
            c.iter().map(|f| f.machine.as_str()).collect::<Vec<_>>(),
            ["n2", "n3"]
        );
    }

    #[test]
    fn a_stored_grant_is_shown_by_its_name_or_as_stored() {
        let v = view(&[
            online("n1", "laptop"),
            checkout("n1", "c1", "#r", "/a/web"),
            checkout("n1", "c2", "#a", "/a/api"),
            checkout("n1", "c3", "#b", "/b/api"),
        ]);
        let f = folders(&v);
        assert_eq!(shown_grant(&f, "n1", "#r"), ("laptop/web".into(), None));
        assert_eq!(
            shown_grant(&f, "laptop", "proj"),
            ("laptop/proj".into(), None)
        );
        // Two projects called `api` there: the folder tells which.
        assert_eq!(
            shown_grant(&f, "n1", "#b"),
            ("laptop/api".into(), Some("/b/api".into()))
        );
    }
}

//! The folder picker of "start an agent in a folder" (ADR-0053): one folder of a machine at a time, its subfolders to choose from.
//! A pure function of what the machine answered and where the keyboard points (render never mutates state, AGENTS.md); it returns
//! the bytes that paint one whole frame.
//!
//! The owner browses the whole machine from its home folder. A member may look only inside the projects granted to them (ADR-0049
//! Amendment A, the hub refuses anything else), so their picker opens at their project's folder on that machine, or offers their
//! projects there when there are several, and never goes above one ([`Nav`], #1086). When the client does not know the role, the
//! picker opens there too, as the window does, and goes up only as the machine says (#1099).

use std::fmt::Write as _;

use agora_i18n::{key, Arg};
use agora_protocol::v1::{DirectoryEntries, DirectoryEntry, Snapshot};

use crate::render::{plain, Text};

/// What one frame of the picker shows.
pub struct Picker<'a> {
    /// The machine's name, as the tree shows it.
    pub machine: &'a str,
    pub folder: &'a DirectoryEntries,
    /// The subfolder the keyboard points at.
    pub selected: usize,
    pub notice: &'a str,
    /// The folder is the choice among a member's projects ([`Nav::projects_listing`]): a heading instead of a path, and no "start
    /// here".
    pub projects: bool,
    pub color: bool,
    pub cols: u16,
    pub rows: u16,
}

/// The folders of the projects a machine reported, as the hub places a member's folders (ADR-0049 Amendment A): each checkout's
/// folder and each workspace's repository root (its own folder when it is no repository), sorted, each once. A member's snapshot
/// holds only their granted projects, so for them these are the folders they may open.
pub fn project_roots(snapshot: &Snapshot, node: &str) -> Vec<String> {
    let checkouts = snapshot
        .checkouts
        .iter()
        .filter(|c| c.node_id == node)
        .map(|c| c.path.clone());
    let workspaces = snapshot
        .workspaces
        .iter()
        .filter(|w| w.node_id == node)
        .map(|w| {
            if w.git_root.is_empty() {
                w.path.clone()
            } else {
                w.git_root.clone()
            }
        });
    let mut roots: Vec<String> = checkouts
        .chain(workspaces)
        .filter(|p| !p.is_empty())
        .collect();
    roots.sort();
    roots.dedup();
    roots
}

/// What the picker shows next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum At {
    /// A folder on the machine, to ask it for (`""`: its home folder).
    Folder(String),
    /// The choice among a member's projects on the machine.
    Projects,
}

/// What this client knows of the person's role on the current connection (#1086, #1099). The hub ends a connection when a role or a
/// grant changes, so it holds for one connection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Role {
    /// Not known: the plain list, which does not ask, or a console whose question went unanswered. The picker opens where the window
    /// does, at the machine's project folder (or the choice of them), and goes up as the machine says: the hub leaves a member's
    /// project folder without a way up, and gives the owner one. With no project folder there, the home folder.
    #[default]
    Unknown,
    /// The owner (the hub answered `PairStatus`): the whole machine, from its home folder.
    Owner,
    /// Not the owner (the hub refused `PairStatus`, or a folder, for the role): only inside their projects.
    Limited,
}

/// Where the picker may go on one machine. Pure state: the caller asks the machine for each [`At::Folder`] it answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Nav {
    /// `None`: the whole machine (the owner). `Some`: these project folders first (a member, #1086, or a role not known, #1099).
    roots: Option<Vec<String>>,
    /// Never above a project folder, whatever the machine says (a member, #1086). False: up as the machine says.
    confined: bool,
    /// How many folders below the project folder the picker is (a member's only). Counted by the steps taken, not by comparing
    /// paths: the machine answers a folder in its own spelling (links followed, a trailing separator dropped).
    depth: usize,
}

impl Nav {
    /// The owner's picker: the whole machine.
    pub fn machine() -> Self {
        Self {
            roots: None,
            confined: false,
            depth: 0,
        }
    }

    /// A member's picker: only inside `roots` (see [`project_roots`]).
    pub fn projects(roots: Vec<String>) -> Self {
        Self {
            roots: Some(roots),
            confined: true,
            depth: 0,
        }
    }

    /// The picker for `role` on a machine whose project folders (see [`project_roots`]) are `roots`.
    pub fn for_role(role: Role, roots: Vec<String>) -> Self {
        match role {
            Role::Owner => Self::machine(),
            Role::Limited => Self::projects(roots),
            Role::Unknown => Self {
                roots: Some(roots),
                confined: false,
                depth: 0,
            },
        }
    }

    /// The role this picker was made for.
    pub fn role(&self) -> Role {
        match (&self.roots, self.confined) {
            (None, _) => Role::Owner,
            (Some(_), true) => Role::Limited,
            (Some(_), false) => Role::Unknown,
        }
    }

    /// Whether this picker stays inside a member's projects.
    pub fn confined(&self) -> bool {
        self.confined
    }

    /// The machine's answer for the folder shown. A member's project folder that the hub gave a way up from tells that the hub no
    /// longer keeps this person inside their projects: made the owner, on a connection that began after the change, which the client
    /// did not see as a lost link (#1099). From there the picker goes up as the machine says, as with the role not known; true when
    /// it changed so.
    pub fn answered(&mut self, folder: &DirectoryEntries) -> bool {
        if !self.confined || self.depth > 0 || folder.parent.is_empty() {
            return false;
        }
        self.confined = false;
        true
    }

    /// The folder to ask the machine `node` for before the choice of projects is offered (#1099): the hub refuses a person whose role
    /// may not look into folders (a viewer) at once, so they are told before they choose rather than after. The first project folder
    /// that `snapshot` places in one project only: the hub refuses a member a folder two projects were reported in (a project they were
    /// not granted is not in their snapshot, so that cannot always be seen here). `None` without a choice.
    pub fn probe(&self, snapshot: &Snapshot, node: &str) -> Option<String> {
        let roots = self.roots.as_deref().filter(|r| r.len() > 1)?;
        let projects_of = |root: &str| {
            let checkouts = snapshot
                .checkouts
                .iter()
                .filter(|c| c.node_id == node && c.path == root)
                .map(|c| c.project_id.as_str());
            let workspaces = snapshot
                .workspaces
                .iter()
                .filter(|w| {
                    let folder = if w.git_root.is_empty() {
                        &w.path
                    } else {
                        &w.git_root
                    };
                    w.node_id == node && folder == root
                })
                .map(|w| w.project_id.as_str());
            let mut ids: Vec<&str> = checkouts.chain(workspaces).collect();
            ids.sort_unstable();
            ids.dedup();
            ids.len()
        };
        roots
            .iter()
            .find(|r| projects_of(r) == 1)
            .or_else(|| roots.first())
            .cloned()
    }

    /// Where the picker opens: the home folder for the whole machine; a member's project folder, or the choice when they have several
    /// there (with none, the home folder, which the hub refuses in words for their role).
    pub fn start(&mut self) -> At {
        self.depth = 0;
        match self.roots.as_deref() {
            None | Some([]) => At::Folder(String::new()),
            Some([root]) => At::Folder(root.clone()),
            Some(_) => At::Projects,
        }
    }

    /// The choice among a member's projects, as a listing the picker draws: each project's folder by its whole path.
    pub fn projects_listing(&self) -> DirectoryEntries {
        DirectoryEntries {
            entries: self
                .roots
                .iter()
                .flatten()
                .map(|r| DirectoryEntry {
                    name: r.clone(),
                    is_git: false,
                })
                .collect(),
            ..DirectoryEntries::default()
        }
    }

    /// Into the `i`th project of the choice.
    pub fn open_project(&mut self, i: usize) -> Option<At> {
        let root = self.roots.as_ref()?.get(i)?.clone();
        self.depth = 0;
        Some(At::Folder(root))
    }

    /// Into the subfolder `name` of the folder shown.
    pub fn open(&mut self, folder: &DirectoryEntries, name: &str) -> At {
        self.depth += 1;
        At::Folder(child(&folder.path, name))
    }

    /// Up from the folder shown: its parent, as the machine named it. A member stops at their project's folder: from there up is the
    /// choice of their projects when they have several, else nowhere (`None`). With the role not known, up goes as the machine says
    /// (the hub names no way up out of a member's project), and a folder without one leads back to the choice.
    pub fn up(&mut self, folder: &DirectoryEntries) -> Option<At> {
        let Some(roots) = &self.roots else {
            return (!folder.parent.is_empty()).then(|| At::Folder(folder.parent.clone()));
        };
        if (self.depth > 0 || !self.confined) && !folder.parent.is_empty() {
            self.depth = self.depth.saturating_sub(1);
            return Some(At::Folder(folder.parent.clone()));
        }
        self.depth = 0;
        (roots.len() > 1).then_some(At::Projects)
    }
}

/// A subfolder's path on the machine: the machine's own separator (a Windows machine answers `C:\…`), whatever this terminal runs on.
pub fn child(folder: &str, name: &str) -> String {
    let sep = if folder.contains('\\') && !folder.contains('/') {
        '\\'
    } else {
        '/'
    };
    if folder.ends_with(sep) {
        format!("{folder}{name}")
    } else {
        format!("{folder}{sep}{name}")
    }
}

/// Text cut to `width` characters (folder names are shown as plain text: hidden characters are escaped and controls cannot reach the terminal).
fn fit(text: &str, width: usize) -> String {
    let without_controls: String = text.chars().filter(|c| !c.is_control()).collect();
    let clean = plain(&without_controls);
    if clean.chars().count() <= width {
        return clean;
    }
    let mut out: String = clean.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub fn frame(p: &Picker<'_>, t: Text<'_>) -> String {
    let (cols, rows) = (usize::from(p.cols.max(20)), usize::from(p.rows.max(6)));
    let (bold, dim, rev, reset) = if p.color {
        ("\x1b[1m", "\x1b[2m", "\x1b[7m", "\x1b[0m")
    } else {
        ("", "", "", "")
    };
    let mut lines = vec![
        format!(
            "{bold}{}{reset}",
            fit(
                &t(
                    key!("tui-folder-title"),
                    &[("machine", Arg::Str(p.machine.to_string()))]
                ),
                cols
            )
        ),
        format!(
            "{dim}{}{reset}",
            if p.projects {
                fit(&t(key!("tui-folder-projects"), &[]), cols)
            } else {
                fit(&p.folder.path, cols)
            }
        ),
        String::new(),
    ];
    // The list fills what the title, the path, a blank line and the status line leave; the selected folder stays in view.
    // A truncated listing keeps one row for its "more" note, so the cut is never silent.
    let room = rows
        .saturating_sub(4 + usize::from(p.folder.truncated))
        .max(1);
    let entries = &p.folder.entries;
    if entries.is_empty() {
        lines.push(format!(
            "  {dim}{}{reset}",
            t(key!("tui-folder-empty"), &[])
        ));
    }
    let first = p
        .selected
        .saturating_sub(room.saturating_sub(1))
        .min(entries.len().saturating_sub(room));
    for (i, e) in entries.iter().enumerate().skip(first).take(room) {
        let git = if e.is_git {
            format!("  {dim}{}{reset}", t(key!("tui-folder-git"), &[]))
        } else {
            String::new()
        };
        let name = fit(&e.name, cols.saturating_sub(10));
        if i == p.selected {
            if p.color {
                lines.push(format!("{rev} ▸ {name} {reset}{git}"));
            } else {
                lines.push(format!(">  {name}{git}"));
            }
        } else {
            lines.push(format!("   {name}{git}"));
        }
    }
    if p.folder.truncated {
        lines.push(format!(
            "  {dim}{}{reset}",
            t(
                key!("tui-folder-more"),
                &[("n", Arg::Num(entries.len() as i64))]
            )
        ));
    }
    let status = if !p.notice.is_empty() {
        p.notice.to_string()
    } else if p.projects {
        t(key!("tui-folder-projects-keys"), &[])
    } else {
        t(key!("tui-folder-keys"), &[])
    };
    let mut out = String::from("\x1b[?25l\x1b[H\x1b[2J");
    for (r, l) in lines.iter().take(rows - 1).enumerate() {
        let _ = write!(out, "\x1b[{};1H{l}", r + 1);
    }
    let _ = write!(
        out,
        "\x1b[{rows};1H{rev}{}{reset}\x1b[K",
        fit(&format!(" {status}"), cols)
    );
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use agora_protocol::v1::DirectoryEntry;
    use agora_vt::{AlacrittyTerminal, Size, Terminal};

    fn text(id: &str, args: &[(&str, Arg)]) -> String {
        let mut s = id.to_string();
        for (k, v) in args {
            let v = match v {
                Arg::Num(n) => n.to_string(),
                Arg::Str(s) => s.clone(),
                #[allow(unreachable_patterns)]
                _ => String::new(),
            };
            let _ = write!(s, " {k}={v}");
        }
        s
    }

    fn folder(n: usize, truncated: bool) -> DirectoryEntries {
        DirectoryEntries {
            path: "/Users/me/code".into(),
            parent: "/Users/me".into(),
            entries: (0..n)
                .map(|i| DirectoryEntry {
                    name: format!("p{i:02}"),
                    is_git: i % 2 == 0,
                })
                .collect(),
            truncated,
        }
    }

    fn rows(f: &str, rows: u16) -> Vec<String> {
        let mut t = AlacrittyTerminal::new(Size { cols: 80, rows });
        t.feed(f.as_bytes());
        (0..rows).map(|r| t.line(r)).collect()
    }

    #[test]
    fn a_child_uses_the_machines_own_separator() {
        assert_eq!(child("/Users/me", "code"), "/Users/me/code");
        assert_eq!(child("/", "Users"), "/Users");
        assert_eq!(child(r"C:\Users\me", "code"), r"C:\Users\me\code");
        assert_eq!(child(r"C:\", "Users"), r"C:\Users");
    }

    #[test]
    fn the_picker_shows_the_folder_its_subfolders_and_keeps_the_selection_in_view() {
        let f = folder(30, true);
        let p = Picker {
            machine: "mac2",
            folder: &f,
            selected: 25,
            notice: "",
            projects: false,
            color: false,
            cols: 80,
            rows: 12,
        };
        let r = rows(&frame(&p, &text), 12);
        assert!(r[0].contains("tui-folder-title machine=mac2"), "{r:#?}");
        assert_eq!(r[1], "/Users/me/code");
        // Eight list rows (12 minus title, path, blank and status): the selected one is in them, marked, git folders tagged.
        assert!(r.iter().any(|l| l.starts_with(">  p25")), "{r:#?}");
        assert!(r.iter().any(|l| l == "   p24  tui-folder-git"), "{r:#?}");
        assert!(!r.iter().any(|l| l.contains("p00")), "scrolled: {r:#?}");
        assert!(r[11].contains("tui-folder-keys"), "{r:#?}");
    }

    #[test]
    fn folder_names_make_hidden_characters_visible() {
        let mut f = folder(1, false);
        f.entries[0].name = "a\u{202e}\u{2800}b".into();
        let p = Picker {
            machine: "mac2",
            folder: &f,
            selected: 0,
            notice: "",
            projects: false,
            color: false,
            cols: 80,
            rows: 8,
        };
        let drawn = frame(&p, &text);
        assert!(!drawn.chars().any(agora_core::text::is_hidden), "{drawn:?}");
        assert!(drawn.contains("a\\u{2800}b"), "{drawn:?}");
    }

    #[test]
    fn a_truncated_folder_that_fills_the_list_still_says_there_are_more() {
        // The list fills the room exactly when a listing was cut (directory.list's cap): the notice
        // must not be the one line the status row squeezes out.
        let f = folder(30, true);
        let p = Picker {
            machine: "mac2",
            folder: &f,
            selected: 25,
            notice: "",
            projects: false,
            color: false,
            cols: 80,
            rows: 12,
        };
        assert!(
            rows(&frame(&p, &text), 12)
                .iter()
                .any(|l| l.contains("tui-folder-more n=30")),
            "the truncation notice is not shown: {r:#?}",
            r = rows(&frame(&p, &text), 12)
        );
    }

    #[test]
    fn an_empty_folder_says_so_and_a_long_one_says_there_are_more() {
        let f = folder(0, false);
        let p = Picker {
            machine: "m",
            folder: &f,
            selected: 0,
            notice: "",
            projects: false,
            color: true,
            cols: 80,
            rows: 10,
        };
        assert!(rows(&frame(&p, &text), 10)
            .iter()
            .any(|l| l.contains("tui-folder-empty")));
        let f = folder(3, true);
        let p = Picker {
            machine: "m",
            folder: &f,
            selected: 0,
            notice: "",
            projects: false,
            color: true,
            cols: 80,
            rows: 10,
        };
        assert!(rows(&frame(&p, &text), 10)
            .iter()
            .any(|l| l.contains("tui-folder-more n=3")));
    }

    fn listing(path: &str, parent: &str) -> DirectoryEntries {
        DirectoryEntries {
            path: path.into(),
            parent: parent.into(),
            entries: vec![DirectoryEntry {
                name: "api".into(),
                is_git: false,
            }],
            truncated: false,
        }
    }

    /// A machine `n1` with a checkout `/srv/mine` and a workspace in `/home/eda/web` (started in its `src`), each reported twice; `n2`
    /// has a project of its own.
    fn snapshot() -> Snapshot {
        use agora_protocol::v1::{CheckoutSnapshot, WorkspaceRegistered};
        let checkout = |node: &str, id: &str, path: &str| CheckoutSnapshot {
            checkout_id: id.into(),
            node_id: node.into(),
            project_id: "p".into(),
            path: path.into(),
        };
        let workspace = |id: &str, path: &str, git_root: &str| WorkspaceRegistered {
            workspace_id: id.into(),
            node_id: "n1".into(),
            project_id: "w".into(),
            path: path.into(),
            git_root: git_root.into(),
            ..Default::default()
        };
        Snapshot {
            checkouts: vec![
                checkout("n1", "c1", "/srv/mine"),
                checkout("n2", "c1", "/srv/theirs"),
                checkout("n1", "c2", "/srv/mine"),
            ],
            workspaces: vec![
                workspace("w1", "/home/eda/web/src", "/home/eda/web"),
                workspace("w2", "/home/eda/web", ""),
                workspace("w3", "/home/eda/notes", ""),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn a_members_picker_opens_at_their_project_folder_on_that_machine() {
        // #1086: the folders the hub lets a member open (ADR-0049 Amendment A): checkouts, and workspaces by their repository root.
        let s = snapshot();
        assert_eq!(
            project_roots(&s, "n1"),
            ["/home/eda/notes", "/home/eda/web", "/srv/mine"]
        );
        assert_eq!(project_roots(&s, "n2"), ["/srv/theirs"]);
        assert!(project_roots(&s, "n3").is_empty());
        // One project there: the picker opens in its folder, never at the home folder the hub refuses a member.
        let mut nav = Nav::projects(project_roots(&s, "n2"));
        assert!(nav.confined());
        assert_eq!(nav.start(), At::Folder("/srv/theirs".into()));
        // Several: their projects are the choice, each opened by its folder.
        let mut nav = Nav::projects(project_roots(&s, "n1"));
        assert_eq!(nav.start(), At::Projects);
        let names: Vec<String> = nav
            .projects_listing()
            .entries
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, project_roots(&s, "n1"));
        assert_eq!(nav.open_project(2), Some(At::Folder("/srv/mine".into())));
        assert_eq!(nav.open_project(3), None);
        // None there (no project the hub knows of): the home folder, which the hub refuses in words for the role.
        assert_eq!(Nav::projects(Vec::new()).start(), At::Folder(String::new()));
    }

    #[test]
    fn the_owners_picker_still_opens_at_the_home_folder_and_goes_up_as_the_machine_says() {
        let mut nav = Nav::machine();
        assert!(!nav.confined());
        assert_eq!(nav.start(), At::Folder(String::new()));
        let home = listing("/Users/me", "/Users");
        assert_eq!(nav.up(&home), Some(At::Folder("/Users".into())));
        assert_eq!(nav.up(&listing("/", "")), None);
        assert_eq!(nav.open(&home, "code"), At::Folder("/Users/me/code".into()));
        // Up from a folder it went into is the machine's parent, however far below it is.
        assert_eq!(
            nav.up(&listing("/Users/me/code", "/Users/me")),
            Some(At::Folder("/Users/me".into()))
        );
        assert_eq!(nav.up(&home), Some(At::Folder("/Users".into())));
    }

    #[test]
    fn a_member_never_goes_above_a_project_folder() {
        // One project: up from its folder goes nowhere, even when the machine (an older hub) names a parent.
        let mut nav = Nav::projects(vec!["/srv/mine".into()]);
        assert_eq!(nav.start(), At::Folder("/srv/mine".into()));
        let root = listing("/srv/mine", "/srv");
        assert_eq!(nav.up(&root), None);
        // Down and back up again, then no further.
        assert_eq!(nav.open(&root, "api"), At::Folder("/srv/mine/api".into()));
        let api = listing("/srv/mine/api", "/srv/mine");
        assert_eq!(nav.open(&api, "v2"), At::Folder("/srv/mine/api/v2".into()));
        assert_eq!(
            nav.up(&listing("/srv/mine/api/v2", "/srv/mine/api")),
            Some(At::Folder("/srv/mine/api".into()))
        );
        assert_eq!(nav.up(&api), Some(At::Folder("/srv/mine".into())));
        assert_eq!(nav.up(&root), None);
        // A listing with no way up (the hub left it out) stops there too.
        assert_eq!(nav.open(&root, "api"), At::Folder("/srv/mine/api".into()));
        assert_eq!(nav.up(&listing("/srv/mine/api", "")), None);
        // Several projects: up from one's folder is the choice of them, and up from the choice is nowhere.
        let mut nav = Nav::projects(vec!["/a".into(), "/b".into()]);
        assert_eq!(nav.start(), At::Projects);
        assert_eq!(nav.open_project(1), Some(At::Folder("/b".into())));
        assert_eq!(nav.up(&listing("/b", "/")), Some(At::Projects));
    }

    #[test]
    fn with_the_role_not_known_the_picker_opens_where_the_window_does_and_never_at_the_home_folder()
    {
        // #1099: the plain list does not ask for the role. Its picker opens at the machine's project folder, as the window does, so a
        // member's first `f` asks the hub for no folder it refuses (no refused row in the owner's Activity).
        let s = snapshot();
        let mut nav = Nav::for_role(Role::Unknown, project_roots(&s, "n2"));
        assert_eq!(nav.role(), Role::Unknown);
        assert!(!nav.confined());
        assert_eq!(nav.start(), At::Folder("/srv/theirs".into()));
        // Up as the machine says: the hub gives the owner a way up and a member none.
        let root = listing("/srv/theirs", "/srv");
        assert_eq!(nav.up(&root), Some(At::Folder("/srv".into())));
        assert_eq!(nav.up(&listing("/", "")), None);
        let mut nav = Nav::for_role(Role::Unknown, project_roots(&s, "n2"));
        nav.start();
        assert_eq!(nav.up(&listing("/srv/theirs", "")), None);
        // Several: the choice first; a project folder without a way up leads back to it.
        let mut nav = Nav::for_role(Role::Unknown, project_roots(&s, "n1"));
        assert_eq!(nav.start(), At::Projects);
        assert_eq!(
            nav.open_project(0),
            Some(At::Folder("/home/eda/notes".into()))
        );
        assert_eq!(nav.up(&listing("/home/eda/notes", "")), Some(At::Projects));
        // No project folder there: the home folder, as before.
        assert_eq!(
            Nav::for_role(Role::Unknown, Vec::new()).start(),
            At::Folder(String::new())
        );
        // A known role keeps its picker.
        assert_eq!(
            Nav::for_role(Role::Owner, project_roots(&s, "n1")),
            Nav::machine()
        );
        assert_eq!(
            Nav::for_role(Role::Limited, project_roots(&s, "n1")),
            Nav::projects(project_roots(&s, "n1"))
        );
        assert_eq!(Nav::machine().role(), Role::Owner);
        assert_eq!(Nav::projects(Vec::new()).role(), Role::Limited);
    }

    #[test]
    fn a_member_made_the_owner_goes_up_from_the_project_folder_once_the_hub_gives_a_way_up() {
        // #1099: the hub names no way up out of a member's project folder and one for the owner. A picker made for a member that hears
        // one goes up as the machine says from then on, without waiting for the client to ask the role again.
        let mut nav = Nav::for_role(Role::Limited, vec!["/srv/mine".into()]);
        assert_eq!(nav.start(), At::Folder("/srv/mine".into()));
        assert!(
            !nav.answered(&listing("/srv/mine", "")),
            "a member's: no way up"
        );
        assert_eq!(nav.role(), Role::Limited);
        // Below the project folder a way up is the member's too: it tells nothing.
        assert_eq!(
            nav.open(&listing("/srv/mine", ""), "api"),
            At::Folder("/srv/mine/api".into())
        );
        assert!(!nav.answered(&listing("/srv/mine/api", "/srv/mine")));
        assert_eq!(nav.role(), Role::Limited);
        assert_eq!(
            nav.up(&listing("/srv/mine/api", "/srv/mine")),
            Some(At::Folder("/srv/mine".into()))
        );
        let root = listing("/srv/mine", "/srv");
        assert!(nav.answered(&root));
        assert_eq!(nav.role(), Role::Unknown);
        assert_eq!(nav.up(&root), Some(At::Folder("/srv".into())));
        // The owner's picker hears nothing new.
        let mut nav = Nav::machine();
        assert!(!nav.answered(&listing("/Users/me", "/Users")));
    }

    #[test]
    fn the_choice_of_projects_is_tried_on_the_machine_before_it_is_offered() {
        // #1099: a viewer is refused every folder; the first project's folder is asked for before the choice is shown, so they are
        // told at once instead of after Enter. One project or the whole machine opens a folder at once: nothing to try first.
        let s = snapshot();
        for role in [Role::Limited, Role::Unknown] {
            assert_eq!(
                Nav::for_role(role, project_roots(&s, "n1")).probe(&s, "n1"),
                Some("/home/eda/notes".into())
            );
            assert_eq!(
                Nav::for_role(role, project_roots(&s, "n2")).probe(&s, "n2"),
                None
            );
        }
        assert_eq!(
            Nav::for_role(Role::Owner, project_roots(&s, "n1")).probe(&s, "n1"),
            None
        );
        // A folder two projects were reported in is refused to a member: the next folder that is one project's is tried instead.
        let mut two = snapshot();
        two.checkouts.push(agora_protocol::v1::CheckoutSnapshot {
            checkout_id: "c3".into(),
            node_id: "n1".into(),
            project_id: "q".into(),
            path: "/home/eda/notes".into(),
        });
        let nav = Nav::for_role(Role::Limited, project_roots(&two, "n1"));
        assert_eq!(nav.probe(&two, "n1"), Some("/home/eda/web".into()));
    }

    #[test]
    fn the_choice_of_projects_has_a_heading_and_no_start_here() {
        let nav = Nav::projects(vec!["/srv/mine".into(), "/home/eda/web".into()]);
        let f = nav.projects_listing();
        let p = Picker {
            machine: "n1",
            folder: &f,
            selected: 1,
            notice: "",
            projects: true,
            color: false,
            cols: 80,
            rows: 10,
        };
        let r = rows(&frame(&p, &text), 10);
        assert_eq!(r[1], "tui-folder-projects");
        assert!(r.iter().any(|l| l == "   /srv/mine"), "{r:#?}");
        assert!(
            r.iter().any(|l| l.starts_with(">  /home/eda/web")),
            "{r:#?}"
        );
        assert!(r[9].contains("tui-folder-projects-keys"), "{r:#?}");
    }
}

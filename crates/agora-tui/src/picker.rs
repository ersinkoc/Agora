//! The folder picker of "start an agent in a folder" (ADR-0053): one folder of a machine at a time, its subfolders to choose from.
//! A pure function of what the machine answered and where the keyboard points (render never mutates state, AGENTS.md); it returns
//! the bytes that paint one whole frame.
//!
//! The owner browses the whole machine from its home folder. A member may look only inside the projects granted to them (ADR-0049
//! Amendment A, the hub refuses anything else), so their picker opens at their project's folder on that machine, or offers their
//! projects there when there are several, and never goes above one ([`Nav`], #1086).

use std::fmt::Write as _;

use agora_i18n::{key, Arg};
use agora_protocol::v1::{DirectoryEntries, DirectoryEntry, Snapshot};

use crate::render::Text;

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

/// Where the picker may go on one machine. Pure state: the caller asks the machine for each [`At::Folder`] it answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Nav {
    /// `None`: the whole machine (the owner). `Some`: only inside these project folders (a member, #1086).
    roots: Option<Vec<String>>,
    /// How many folders below the project folder the picker is (a member's only). Counted by the steps taken, not by comparing
    /// paths: the machine answers a folder in its own spelling (links followed, a trailing separator dropped).
    depth: usize,
}

impl Nav {
    /// The owner's picker: the whole machine.
    pub fn machine() -> Self {
        Self {
            roots: None,
            depth: 0,
        }
    }

    /// A member's picker: only inside `roots` (see [`project_roots`]).
    pub fn projects(roots: Vec<String>) -> Self {
        Self {
            roots: Some(roots),
            depth: 0,
        }
    }

    /// Whether this picker stays inside a member's projects.
    pub fn confined(&self) -> bool {
        self.roots.is_some()
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
    /// choice of their projects when they have several, else nowhere (`None`).
    pub fn up(&mut self, folder: &DirectoryEntries) -> Option<At> {
        let Some(roots) = &self.roots else {
            return (!folder.parent.is_empty()).then(|| At::Folder(folder.parent.clone()));
        };
        if self.depth > 0 && !folder.parent.is_empty() {
            self.depth -= 1;
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

/// Text cut to `width` characters (folder names are shown as plain text: no control characters reach the terminal).
fn fit(text: &str, width: usize) -> String {
    let clean: String = text.chars().filter(|c| !c.is_control()).collect();
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

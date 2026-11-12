//! What an agent's process may touch (ADR-0047): the owner's `sandbox` setting turned into the paths handed to
//! `agora_platform::Sandbox`. Pure and testable; enforcement lives in `agora-platform`.

use std::path::{Path, PathBuf};

/// How strictly agents are confined. The default is off; a value that is not understood is the strictest one that exists, because a
/// typo must never widen what agents may touch (the rule `read_policy` follows).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Level {
    #[default]
    Off,
    /// Writes only in the agent's worktree, its git folders and a temp folder; reads the system and what the owner listed.
    Worktree,
    /// As `Worktree`, and no network. Not enforceable yet (Landlock's network rules arrive with a newer kernel ABI), so a node
    /// set to it refuses to start agents instead of pretending.
    WorktreeOffline,
}

impl Level {
    pub fn parse(s: &str) -> Level {
        match s.trim() {
            "off" => Level::Off,
            "worktree" => Level::Worktree,
            _ => Level::WorktreeOffline,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Level::Off => "off",
            Level::Worktree => "worktree",
            Level::WorktreeOffline => "worktree-offline",
        }
    }
}

/// The node's confinement settings: the level and the extra paths the owner allows every agent (`sandbox.read`, `sandbox.write`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SandboxPolicy {
    pub level: Level,
    pub read: Vec<PathBuf>,
    pub write: Vec<PathBuf>,
    /// Kinds whose manifests may ask for more (`sandbox.trust_manifest = kind, kind`). Empty: no manifest widens anything, so a
    /// community manifest can never reach `~/.ssh` by asking.
    pub trusted_kinds: Vec<String>,
}

/// The system folders every program needs to run.
const SYSTEM_READ: &[&str] = &[
    "/usr", "/lib", "/lib32", "/lib64", "/bin", "/sbin", "/etc", "/opt", "/proc", "/sys", "/run",
    "/var",
];

/// A path from a manifest or from the node's `policy`: absolute, or `~/` (`~\` too) below the home folder. Anything else (relative
/// paths, `..`) is ignored, never guessed at.
pub fn expand(p: &str) -> Option<PathBuf> {
    let p = p.trim();
    let path = match p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\")) {
        Some(rest) => crate::usage_files::home_dir()?.join(rest),
        None => PathBuf::from(p),
    };
    (path.is_absolute()
        && !path
            .components()
            .any(|c| c == std::path::Component::ParentDir))
    .then_some(path)
}

/// `<common>/packed-refs`, made empty if it is missing (the platform skips paths that do not exist; git reads an empty file as no
/// packed refs). Git rewrites it to delete a branch and tries to on a commit or checkout, printing an error when it cannot; it holds
/// only refs, which a confined agent can already move. `None` if it cannot be made.
fn packed_refs(common: &Path) -> Option<PathBuf> {
    let path = common.join("packed-refs");
    if !path.is_file() {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .ok()?;
    }
    Some(path)
}

/// The parts of the shared Git folder a commit in `worktree` needs, as `(write, read)`: its own admin folder (index, HEAD) and the
/// common `objects`, `refs` and `logs/refs` are written; the common folder itself (`config`, `hooks`) is only read, so nothing planted
/// there runs in the user's or the node's git.
fn narrow_git(worktree: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut write = Vec::new();
    let mut read = Vec::new();
    if let Some((admin, common)) = git_dirs(worktree) {
        write.push(admin);
        write.push(common.join("objects"));
        write.push(common.join("refs"));
        // A commit appends to the branch's reflog, which `git worktree add` already created under `logs/refs/heads`.
        write.push(common.join("logs").join("refs"));
        read.push(common);
    }
    (write, read)
}

/// The worktree's own admin folder and its checkout's shared Git folder, from the worktree's `.git` file, but only when they check
/// out: the agent can rewrite that file, so its `gitdir:` alone must never decide what the profile opens. The admin folder must be
/// `<common>/worktrees/<name>`, its `gitdir` file must point back at this worktree, and `<common>` must hold `HEAD`, `objects` and
/// `refs`. A relative `gitdir:` is read from the worktree, as git does. `None` (no Git access) for anything else.
pub fn git_dirs(worktree: &Path) -> Option<(PathBuf, PathBuf)> {
    let real = |p: &Path| std::fs::canonicalize(p).ok();
    let text = std::fs::read_to_string(worktree.join(".git")).ok()?;
    let named = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
    let admin = real(&if named.is_absolute() {
        named
    } else {
        worktree.join(named)
    })?;
    if admin.parent()?.file_name()? != "worktrees" {
        return None;
    }
    let common = admin.parent()?.parent()?.to_path_buf();
    if !(common.join("HEAD").is_file()
        && common.join("objects").is_dir()
        && common.join("refs").is_dir())
    {
        return None;
    }
    // The back-pointer git keeps in the admin folder: it must name this worktree's `.git`.
    let back = std::fs::read_to_string(admin.join("gitdir")).ok()?;
    let back = PathBuf::from(back.trim());
    let back = if back.is_absolute() {
        back
    } else {
        admin.join(back)
    };
    (real(&back)? == real(&worktree.join(".git"))?).then_some((admin, common))
}

/// A worktree's shared Git folder (`<main>/.git`), checked as [`git_dirs`] does; git needs to write there (and to the object store
/// and refs) where the profile is not narrowed.
pub fn common_git_dir(worktree: &Path) -> Option<PathBuf> {
    git_dirs(worktree).map(|(_, common)| common)
}

/// Folders below the home folder that hold secrets, never opened to an agent through a selected directory.
const SECRET_HOME_FOLDERS: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".aws",
    ".kube",
    ".docker",
    ".azure",
    ".config/gcloud",
    ".config/gh",
];

/// True for a directory a confined agent may not be launched in (ADR-0052 with ADR-0047): a filesystem root, the home folder or a
/// folder that contains it, a secret folder below it, and the node's own folders (`node_home`, the folder that holds its data and
/// keys, other than the `worktree_root` inside it) or any folder that contains them. Without this, selecting `/` or the home folder
/// would quietly turn `sandbox = worktree` into no confinement. All paths are canonical.
pub fn refused_directory(
    selected: &Path,
    home: Option<&Path>,
    node_home: &Path,
    worktree_root: &Path,
) -> bool {
    if selected.parent().is_none() {
        return true;
    }
    if let Some(home) = home {
        if home.starts_with(selected)
            || SECRET_HOME_FOLDERS
                .iter()
                .any(|f| selected.starts_with(home.join(f)))
        {
            return true;
        }
    }
    // Inside the node's folder only the worktrees it made are fine.
    node_home.starts_with(selected)
        || (selected.starts_with(node_home) && !selected.starts_with(worktree_root))
}

/// The private temp folder of the agent that works in `worktree`; the node points `TMPDIR`, `TEMP` and `TMP` at it where
/// [`agora_platform::PRIVATE_TEMP`] says the system's temp folder is out of the agent's reach.
pub fn agent_temp(worktree: &Path) -> PathBuf {
    let name = worktree
        .file_name()
        .map_or_else(|| "agent".into(), |n| n.to_string_lossy().into_owned());
    std::env::temp_dir().join(format!("agora-agent-{name}"))
}

impl SandboxPolicy {
    /// The paths for one agent, or `None` when agents are not confined.
    pub fn profile(
        &self,
        kind: &str,
        needs: Option<&agora_adapters::manifest::SandboxNeeds>,
        worktree: &Path,
        program: &str,
    ) -> Option<agora_platform::Sandbox> {
        if self.level == Level::Off {
            return None;
        }
        let mut sb = self.files(kind, needs, worktree, program);
        // ADR-0047 `worktree-offline`: as `worktree`, and no network but this machine; the supervisor adds the agent's own sockets.
        if self.level == Level::WorktreeOffline {
            sb.offline = true;
        }
        Some(sb)
    }

    /// The file part of an agent's profile (both confined levels).
    fn files(
        &self,
        kind: &str,
        needs: Option<&agora_adapters::manifest::SandboxNeeds>,
        worktree: &Path,
        program: &str,
    ) -> agora_platform::Sandbox {
        let mut read: Vec<PathBuf> = SYSTEM_READ.iter().map(PathBuf::from).collect();
        // The agent's own program folder, when it is named by path.
        if program.contains(['/', std::path::MAIN_SEPARATOR]) {
            if let Some(dir) = Path::new(program).parent() {
                read.push(dir.to_path_buf());
            }
        }
        read.extend(self.read.iter().cloned());
        if agora_platform::NARROW_GIT_WRITES {
            return self.narrow_profile(kind, needs, worktree, read);
        }
        let mut write = vec![
            worktree.to_path_buf(),
            PathBuf::from("/tmp"),
            PathBuf::from("/dev"),
        ];
        if agora_platform::PRIVATE_TEMP {
            // It must exist before the profile is made: the platform skips folders that do not.
            let temp = agent_temp(worktree);
            let _ = std::fs::create_dir_all(&temp);
            write.push(temp);
        }
        if agora_platform::GIT_CONFIG_READ_ONLY {
            let (git_write, git_read) = narrow_git(worktree);
            write.extend(git_write);
            // Only reached on macOS, whose profile also opens a listed file's `<name>.*` siblings (here `packed-refs.lock`).
            write.extend(git_read.iter().filter_map(|common| packed_refs(common)));
            read.extend(git_read);
        } else {
            write.extend(common_git_dir(worktree));
        }
        write.extend(self.write.iter().cloned());
        if let Some(needs) = needs.filter(|_| self.trusted_kinds.iter().any(|k| k == kind)) {
            read.extend(needs.read.iter().filter_map(|p| expand(p)));
            write.extend(needs.write.iter().filter_map(|p| expand(p)));
        }
        agora_platform::Sandbox::new(read, write)
    }

    /// The profile where permissions are granted folder by folder (ADR-0050): the shared Git folder is readable, and only the parts a
    /// commit needs are writable, so a planted hook or config cannot be left behind.
    fn narrow_profile(
        &self,
        kind: &str,
        needs: Option<&agora_adapters::manifest::SandboxNeeds>,
        worktree: &Path,
        mut read: Vec<PathBuf>,
    ) -> agora_platform::Sandbox {
        let temp = agent_temp(worktree);
        let _ = std::fs::create_dir_all(&temp);
        let mut write = vec![worktree.to_path_buf(), temp];
        let (git_write, git_read) = narrow_git(worktree);
        write.extend(git_write);
        read.extend(git_read);
        write.extend(self.write.iter().cloned());
        if let Some(needs) = needs.filter(|_| self.trusted_kinds.iter().any(|k| k == kind)) {
            read.extend(needs.read.iter().filter_map(|p| expand(p)));
            write.extend(needs.write.iter().filter_map(|p| expand(p)));
        }
        agora_platform::Sandbox::new(read, write)
    }

    /// Whether agents can be started under this policy on this machine.
    pub fn enforceable(&self) -> bool {
        match self.level {
            Level::Off => true,
            Level::Worktree => agora_platform::sandbox_supported(),
            Level::WorktreeOffline => {
                agora_platform::sandbox_supported() && agora_platform::offline_supported()
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// A linked worktree as `git worktree add` leaves it: `<root>/wt/.git` names `<root>/main/.git/worktrees/wt`, whose
    /// `gitdir` file points back. Paths are canonical, as the profile's are. Returns (worktree, common git folder, admin folder).
    fn linked_worktree(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
        let _ = std::fs::remove_dir_all(root);
        std::fs::create_dir_all(root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let (wt, common) = (root.join("wt"), root.join("main").join(".git"));
        let admin = common.join("worktrees").join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::create_dir_all(&admin).unwrap();
        std::fs::create_dir_all(common.join("objects")).unwrap();
        std::fs::create_dir_all(common.join("refs")).unwrap();
        std::fs::write(common.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
        std::fs::write(
            admin.join("gitdir"),
            format!("{}\n", wt.join(".git").display()),
        )
        .unwrap();
        (wt, common, admin)
    }

    #[test]
    fn a_worktrees_git_file_alone_never_decides_what_becomes_writable() {
        let root = std::env::temp_dir().join(format!("agora-gitdir-{}", std::process::id()));
        let (wt, common, admin) = linked_worktree(&root);
        assert_eq!(git_dirs(&wt), Some((admin.clone(), common.clone())));

        // A relative `gitdir:` is read from the worktree, as git does.
        std::fs::write(wt.join(".git"), "gitdir: ../main/.git/worktrees/wt\n").unwrap();
        assert_eq!(git_dirs(&wt), Some((admin.clone(), common.clone())));

        // The agent points its `.git` file somewhere else that looks like a repository's worktree folder: the back-pointer
        // there does not name this worktree, so nothing opens.
        let elsewhere = root.join("elsewhere").join(".git");
        let decoy = elsewhere.join("worktrees").join("x");
        std::fs::create_dir_all(&decoy).unwrap();
        std::fs::create_dir_all(elsewhere.join("objects")).unwrap();
        std::fs::create_dir_all(elsewhere.join("refs")).unwrap();
        std::fs::write(elsewhere.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(decoy.join("gitdir"), "/somewhere/else/.git\n").unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", decoy.display())).unwrap();
        assert_eq!(git_dirs(&wt), None);
        let policy = SandboxPolicy {
            level: Level::Worktree,
            ..SandboxPolicy::default()
        };
        let sb = policy.narrow_profile("k", None, &wt, vec![]);
        assert!(!sb.write.iter().any(|w| w.starts_with(&elsewhere)));
        assert!(!sb.read.iter().any(|r| r.starts_with(&elsewhere)));

        // Nor does a folder that is not under a `worktrees` folder, or one with no repository around it.
        let plain = root.join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        std::fs::write(
            plain.join("gitdir"),
            format!("{}\n", wt.join(".git").display()),
        )
        .unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", plain.display())).unwrap();
        assert_eq!(git_dirs(&wt), None);
        let bare = root.join("bare").join("worktrees").join("wt");
        std::fs::create_dir_all(&bare).unwrap();
        std::fs::write(
            bare.join("gitdir"),
            format!("{}\n", wt.join(".git").display()),
        )
        .unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", bare.display())).unwrap();
        assert_eq!(git_dirs(&wt), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_narrow_profile_never_makes_the_shared_git_folder_itself_writable() {
        let root = std::env::temp_dir().join(format!("agora-narrow-{}", std::process::id()));
        let (wt, common, admin) = linked_worktree(&root);
        let policy = SandboxPolicy {
            level: Level::Worktree,
            ..SandboxPolicy::default()
        };
        let sb = policy.narrow_profile("k", None, &wt, vec![]);
        assert!(sb.write.contains(&wt) && sb.write.contains(&admin));
        assert!(
            sb.write.contains(&common.join("objects")) && sb.write.contains(&common.join("refs"))
        );
        // The branch reflog a commit appends to, and nothing else under `logs`.
        assert!(sb.write.contains(&common.join("logs").join("refs")));
        assert!(!sb.write.contains(&common.join("logs")));
        assert!(!sb.write.contains(&common) && !sb.write.contains(&common.join("hooks")));
        assert!(sb.read.contains(&common));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_confined_launch_refuses_roots_the_home_folder_secret_folders_and_the_nodes_own_folder() {
        let (home, node) = (Path::new("/home/u"), Path::new("/home/u/.agora"));
        let wt = Path::new("/home/u/.agora/worktrees");
        let no = |p: &str| refused_directory(Path::new(p), Some(home), node, wt);
        assert!(no("/") && no("/home") && no("/home/u"));
        assert!(no("/home/u/.ssh") && no("/home/u/.ssh/keys") && no("/home/u/.config/gcloud"));
        assert!(no("/home/u/.agora") && no("/home/u/.agora/data"));
        // A project, a subfolder of one, and a worktree the node made are fine.
        assert!(!no("/home/u/code/app") && !no("/home/u/code/app/src"));
        assert!(!no("/home/u/.agora/worktrees/shell-1"));
        assert!(!no("/srv/shop"));
    }

    #[test]
    fn an_unknown_level_is_the_strictest_never_off() {
        assert_eq!(Level::parse("off"), Level::Off);
        assert_eq!(Level::parse(" worktree "), Level::Worktree);
        assert_eq!(Level::parse("wortree"), Level::WorktreeOffline);
        assert_eq!(Level::parse(""), Level::WorktreeOffline);
    }

    #[test]
    fn the_profile_lets_an_agent_write_in_its_worktree_and_nowhere_listed_by_default() {
        let dir = std::env::temp_dir().join(format!("agora-sbx-{}", std::process::id()));
        let (wt, _, _) = linked_worktree(&dir);
        let (dir, main) = (
            wt.parent().unwrap().to_path_buf(),
            wt.parent().unwrap().join("main"),
        );
        let p = SandboxPolicy {
            level: Level::Worktree,
            read: vec!["/home/x/.cfg".into()],
            write: vec!["/home/x/.cache/agent".into()],
            trusted_kinds: vec![],
        };
        let sb = p.profile("k", None, &wt, "/home/x/bin/agent").unwrap();
        // `worktree` leaves the network alone; `worktree-offline` asks for it to be cut, keeping the node's sockets.
        assert!(!sb.offline);
        let offline = SandboxPolicy {
            level: Level::WorktreeOffline,
            ..p.clone()
        }
        .profile("k", None, &wt, "/home/x/bin/agent")
        .unwrap();
        assert!(offline.offline && offline.sockets.is_empty());
        assert_eq!(offline.write, sb.write);
        assert!(sb.write.contains(&wt));
        assert!(sb.write.contains(&PathBuf::from("/home/x/.cache/agent")));
        if agora_platform::GIT_CONFIG_READ_ONLY {
            // The shared Git folder is read-only apart from `objects`, `refs` and `logs/refs`: no planted `config` or `hooks`.
            assert!(!sb.write.contains(&main.join(".git")) && sb.read.contains(&main.join(".git")));
            assert!(sb.write.contains(&main.join(".git").join("objects")));
            assert!(sb.write.contains(&main.join(".git").join("refs")));
            if !agora_platform::NARROW_GIT_WRITES {
                // macOS: made if missing, so a branch delete and a commit's ref update can rewrite it.
                assert!(sb.write.contains(&main.join(".git").join("packed-refs")));
                assert!(main.join(".git").join("packed-refs").is_file());
            }
        } else {
            assert!(sb.write.contains(&main.join(".git")));
        }
        if !agora_platform::NARROW_GIT_WRITES {
            assert!(
                sb.read.contains(&PathBuf::from("/home/x/bin")) && sb.read.contains(&"/usr".into())
            );
        }
        assert!(!sb.write.iter().any(|w| w == Path::new("/home/x")));
        assert!(SandboxPolicy::default()
            .profile("k", None, &wt, "agent")
            .is_none());

        // A manifest's request is honoured only for a kind the owner trusts, and only for sane paths.
        // Absolute on every system (a drive letter on Windows).
        let (data, out) = (dir.join("data"), dir.join("out"));
        let needs = agora_adapters::manifest::SandboxNeeds {
            read: vec![
                data.to_string_lossy().into_owned(),
                "relative".into(),
                format!("{}/../etc", dir.display()),
            ],
            write: vec![out.to_string_lossy().into_owned()],
        };
        let untrusted = p.profile("k", Some(&needs), &wt, "agent").unwrap();
        assert!(!untrusted.read.contains(&data));
        let trusting = SandboxPolicy {
            trusted_kinds: vec!["k".into()],
            ..p
        };
        let sb = trusting.profile("k", Some(&needs), &wt, "agent").unwrap();
        assert!(sb.read.contains(&data) && sb.write.contains(&out));
        assert!(!sb.read.iter().any(|r| r == Path::new("relative")
            || r.components().any(|c| c == std::path::Component::ParentDir)));
        assert!(trusting
            .profile("other", Some(&needs), &wt, "agent")
            .unwrap()
            .write
            .iter()
            .all(|w| *w != out));
        let _ = std::fs::remove_dir_all(dir);
    }
}

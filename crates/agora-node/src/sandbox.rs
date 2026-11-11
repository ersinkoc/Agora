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

/// A manifest path: absolute, or `~/` below the home folder. Anything else (relative paths, `..`) is ignored, never guessed at.
fn expand(p: &str) -> Option<PathBuf> {
    let path = match p.strip_prefix("~/") {
        Some(rest) => PathBuf::from(std::env::var_os("HOME")?).join(rest),
        None => PathBuf::from(p),
    };
    (path.is_absolute()
        && !path
            .components()
            .any(|c| c == std::path::Component::ParentDir))
    .then_some(path)
}

/// A worktree's `.git` is a file naming its folder inside the main checkout's `.git`; git needs to write there (and to the shared
/// object store and refs next to it). Returns the main `.git` folder.
pub fn common_git_dir(worktree: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(worktree.join(".git")).ok()?;
    let gitdir = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
    // `<main>/.git/worktrees/<name>` -> `<main>/.git`
    gitdir.parent()?.parent().map(Path::to_path_buf)
}

/// The private temp folder of the agent that works in `worktree`; the node points `TEMP` and `TMP` at it where permissions are
/// granted folder by folder.
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
        let mut read: Vec<PathBuf> = SYSTEM_READ.iter().map(PathBuf::from).collect();
        // The agent's own program folder, when it is named by path.
        if program.contains(['/', std::path::MAIN_SEPARATOR]) {
            if let Some(dir) = Path::new(program).parent() {
                read.push(dir.to_path_buf());
            }
        }
        read.extend(self.read.iter().cloned());
        if agora_platform::NARROW_GIT_WRITES {
            return Some(self.narrow_profile(kind, needs, worktree, read));
        }
        let mut write = vec![
            worktree.to_path_buf(),
            PathBuf::from("/tmp"),
            PathBuf::from("/dev"),
        ];
        write.extend(common_git_dir(worktree));
        write.extend(self.write.iter().cloned());
        if let Some(needs) = needs.filter(|_| self.trusted_kinds.iter().any(|k| k == kind)) {
            read.extend(needs.read.iter().filter_map(|p| expand(p)));
            write.extend(needs.write.iter().filter_map(|p| expand(p)));
        }
        Some(agora_platform::Sandbox { read, write })
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
        if let Some(text) = std::fs::read_to_string(worktree.join(".git"))
            .ok()
            .and_then(|t| {
                t.trim()
                    .strip_prefix("gitdir:")
                    .map(|g| g.trim().to_owned())
            })
        {
            // Its own admin folder (index, HEAD) is writable; the common folder is read, with `objects` and `refs` writable.
            write.push(PathBuf::from(text));
        }
        if let Some(common) = common_git_dir(worktree) {
            write.push(common.join("objects"));
            write.push(common.join("refs"));
            read.push(common);
        }
        write.extend(self.write.iter().cloned());
        if let Some(needs) = needs.filter(|_| self.trusted_kinds.iter().any(|k| k == kind)) {
            read.extend(needs.read.iter().filter_map(|p| expand(p)));
            write.extend(needs.write.iter().filter_map(|p| expand(p)));
        }
        agora_platform::Sandbox { read, write }
    }

    /// Whether agents can be started under this policy on this machine.
    pub fn enforceable(&self) -> bool {
        match self.level {
            Level::Off => true,
            Level::Worktree => agora_platform::sandbox_supported(),
            Level::WorktreeOffline => false,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn the_narrow_profile_never_makes_the_shared_git_folder_itself_writable() {
        let root = std::env::temp_dir().join(format!("agora-narrow-{}", std::process::id()));
        let (wt, common) = (root.join("wt"), root.join("main").join(".git"));
        let admin = common.join("worktrees").join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::create_dir_all(&admin).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
        let policy = SandboxPolicy {
            level: Level::Worktree,
            ..SandboxPolicy::default()
        };
        let sb = policy.narrow_profile("k", None, &wt, vec![]);
        assert!(sb.write.contains(&wt) && sb.write.contains(&admin));
        assert!(
            sb.write.contains(&common.join("objects")) && sb.write.contains(&common.join("refs"))
        );
        assert!(!sb.write.contains(&common) && !sb.write.contains(&common.join("hooks")));
        assert!(sb.read.contains(&common));
        let _ = std::fs::remove_dir_all(&root);
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
        let (main, wt) = (dir.join("main"), dir.join("wt"));
        std::fs::create_dir_all(main.join(".git/worktrees/wt")).unwrap();
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(
            wt.join(".git"),
            format!("gitdir: {}\n", main.join(".git/worktrees/wt").display()),
        )
        .unwrap();
        let p = SandboxPolicy {
            level: Level::Worktree,
            read: vec!["/home/x/.cfg".into()],
            write: vec!["/home/x/.cache/agent".into()],
            trusted_kinds: vec![],
        };
        let sb = p.profile("k", None, &wt, "/home/x/bin/agent").unwrap();
        assert!(sb.write.contains(&wt));
        assert!(sb.write.contains(&PathBuf::from("/home/x/.cache/agent")));
        if agora_platform::NARROW_GIT_WRITES {
            // Folder by folder: the shared Git folder is read-only apart from `objects` and `refs`.
            assert!(!sb.write.contains(&main.join(".git")) && sb.read.contains(&main.join(".git")));
        } else {
            assert!(sb.write.contains(&main.join(".git")));
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

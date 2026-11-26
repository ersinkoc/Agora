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
    /// The node's own folders and sockets, closed to every confined agent whatever else is open (set by the node, not the policy
    /// file): its data (trusted devices, policy, ledger), identity, logs, hub folder, control socket and the agents' mesh sockets.
    pub protect: Vec<PathBuf>,
    /// ADR-0098: the node's configured data directory owns generated Git files.
    pub data_dir: Option<PathBuf>,
    /// ADR-0082: the control token file, set by the node where it requires the token (Linux, confinement on). A profile whose read or
    /// write grants contain it is refused at the start (`control_token_reachable`).
    pub control_token: Option<PathBuf>,
    /// ADR-0083: names of environment variables the owner lets through to confined agents although the node scrubs them
    /// (`sandbox.pass_env = NAME, ...`). Names only, never values; any malformed entry empties the list.
    pub pass_env: Vec<String>,
    /// ADR-0083: unix sockets the owner adds to every confined agent's allow-list (`sandbox.sockets = <path>, ...`), each of which
    /// hands the agent whatever the socket's service can do for the user.
    pub sockets: Vec<PathBuf>,
}

/// ADR-0083: variables that point at a socket, and through it at something the user's session can do: the ssh-agent (signs with every
/// key), gpg-agent, the session bus (starts programs outside the sandbox), the docker daemon (root on the machine), a tmux server
/// (keystrokes into the owner's shells). A confined agent does not inherit them from the node; `sandbox.pass_env` lets one through.
/// The scrub is the only layer where the system cannot refuse the connection itself (`agora_platform::socket_confinement`).
pub const SOCKET_VARIABLES: &[&str] = &[
    "SSH_AUTH_SOCK",
    "GPG_AGENT_INFO",
    "DBUS_SESSION_BUS_ADDRESS",
    "DOCKER_HOST",
    "TMUX",
    "TMUX_PANE",
];

/// `p` with its symbolic links resolved, as far as it exists: the nearest existing parent is resolved and the rest kept as written,
/// so a socket that is not there yet (or a link to the node's folder) is judged by where it would really be.
fn canonical(p: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut at = p;
    loop {
        if let Ok(real) = std::fs::canonicalize(at) {
            return rest.iter().rev().fold(real, |acc, c| acc.join(c));
        }
        match (at.parent(), at.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                at = parent;
            }
            _ => return p.to_path_buf(),
        }
    }
}

/// Whether `name` can be an environment variable name the policy passes through: letters, digits and `_`, not starting with a digit.
/// Nothing else: no `=`, no pattern, no empty name.
fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `sandbox.pass_env`: a comma-separated list of variable names. One entry that is not a plain name (a `*`, an `=`, a value) makes the
/// whole list empty: a typo never widens what an agent receives, and it is never read as a value.
pub fn parse_pass_env(v: &str) -> Vec<String> {
    let names: Vec<&str> = v
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if names.iter().all(|n| is_env_name(n)) {
        names.into_iter().map(str::to_string).collect()
    } else {
        Vec::new()
    }
}

impl SandboxPolicy {
    /// The inherited variables a confined agent's environment leaves out: [`SOCKET_VARIABLES`] but the ones the owner passed. Empty
    /// when agents are not confined at all (the caller applies it only to confined starts).
    pub fn scrubbed_env(&self) -> Vec<String> {
        SOCKET_VARIABLES
            .iter()
            .filter(|n| !self.pass_env.iter().any(|p| p == *n))
            .map(|n| (*n).to_string())
            .collect()
    }
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
/// `refs`. A relative `gitdir:` is read from the worktree, as git does. The `.git` file must be a plain file, not a link: through a
/// link to another worktree's `.git`, that worktree's `gitdir:` and back-pointer agree and its repository would open. `None` (no
/// Git access) for anything else.
pub fn git_dirs(worktree: &Path) -> Option<(PathBuf, PathBuf)> {
    let real = |p: &Path| std::fs::canonicalize(p).ok();
    let own = worktree.join(".git");
    if !std::fs::symlink_metadata(&own).ok()?.file_type().is_file() {
        return None;
    }
    // This worktree's `.git` by its real folder, never by following a link at `.git` itself.
    let own = real(worktree)?.join(".git");
    // Read once, bounded, without following a link or waiting on a FIFO swapped in after the look above.
    let read = |p: &Path| {
        crate::git::read_link_file(p)
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
    };
    let text = read(&own)?;
    let named = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
    let admin = real(&if named.is_absolute() {
        named
    } else {
        own.with_file_name(named)
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
    let back = read(&admin.join("gitdir"))?;
    let back = PathBuf::from(back.trim());
    let back = if back.is_absolute() {
        back
    } else {
        admin.join(back)
    };
    (real(&back)? == own).then_some((admin, common))
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
/// folder that contains it, a secret folder below it or a folder that contains one, and the node's own folders (`node_home`, the folder that holds its data and
/// keys, other than the `worktree_root` inside it) or any folder that contains them. Without this, selecting `/` or the home folder
/// would quietly turn `sandbox = worktree` into no confinement. All paths are canonical.
pub fn refused_directory(
    selected: &Path,
    home: Option<&Path>,
    node_home: &Path,
    worktree_root: &Path,
) -> bool {
    selected.parent().is_none()
        || home.is_some_and(|h| {
            // The home folder or a folder that holds it, and a folder that holds a secret one (`~/.config` holds
            // `~/.config/gh`): fine to look into, not to launch in.
            h.starts_with(selected)
                || SECRET_HOME_FOLDERS
                    .iter()
                    .any(|f| h.join(f).starts_with(selected))
        })
        || node_home.starts_with(selected)
        || private_folder(selected, home, node_home, worktree_root)
}

/// A folder kept from agents and from folder listings (ADR-0052, ADR-0053): a secret folder below the home folder or a folder below
/// one, or inside the node's own folder anything but the worktrees it made. All paths are canonical.
pub fn private_folder(
    dir: &Path,
    home: Option<&Path>,
    node_home: &Path,
    worktree_root: &Path,
) -> bool {
    home.is_some_and(|h| {
        SECRET_HOME_FOLDERS
            .iter()
            .any(|f| dir.starts_with(h.join(f)))
    }) || (dir.starts_with(node_home) && !dir.starts_with(worktree_root))
}

/// The private temp folder of the agent that works in `worktree`; the node points `TMPDIR`, `TEMP` and `TMP` at it where
/// [`agora_platform::PRIVATE_TEMP`] says the system's temp folder is out of the agent's reach. The place is part of the name: two
/// worktrees called `web` of different projects must not share one. Only the first 48 characters of the name are used (at most 192
/// bytes; the place keeps it unique), so the folder's name stays within the 255-byte limit whatever the worktree is called.
pub fn agent_temp(worktree: &Path) -> PathBuf {
    let name: String = worktree.file_name().map_or_else(
        || "agent".into(),
        |n| n.to_string_lossy().chars().take(48).collect(),
    );
    let place = crate::gitconfig::place_hash(worktree);
    std::env::temp_dir().join(format!("agora-agent-{name}-{place:08x}"))
}

/// Whether `file` lies below any of the `write` folders, by name and by where the names lead (a folder that is a link into a writable
/// one). Fails closed: a folder whose place cannot be resolved counts as under a grant, and a grant that cannot be resolved is compared
/// by name only.
fn under_a_grant(file: &Path, write: &[PathBuf]) -> bool {
    if write.iter().any(|w| file.starts_with(w)) {
        return true;
    }
    let (Some(parent), Some(name)) = (file.parent(), file.file_name()) else {
        return true;
    };
    let Ok(parent) = std::fs::canonicalize(parent) else {
        return true;
    };
    let real = parent.join(name);
    write
        .iter()
        .any(|w| std::fs::canonicalize(w).is_ok_and(|w| real.starts_with(w)))
}

/// #1114: whether one of the node's own paths lies at or below a grant: by name; by its name in its real parent folder (a link there
/// that an agent may write could be swapped); or by where it really leads, a folder that is itself a link included (`--data` naming a
/// link into `/tmp`). A path that does not exist yet is judged by its nearest existing folder ([`canonical`]) instead of failing closed
/// as [`under_a_grant`] does: below a folder the agent cannot write it cannot make the rest of the name lead anywhere else. Except a
/// link on the way whose target is missing (`--manifests` naming a link into `/tmp` after a reboot cleared it): where it leads cannot
/// be resolved, and an agent that may write there could make it, so it fails closed.
fn reaches(path: &Path, grants: &[PathBuf]) -> bool {
    if grants.iter().any(|g| path.starts_with(g)) {
        return true;
    }
    if path.ancestors().any(|a| {
        std::fs::symlink_metadata(a).is_ok_and(|m| m.file_type().is_symlink())
            && std::fs::canonicalize(a).is_err()
    }) {
        return true;
    }
    let named = match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => canonical(parent).join(name),
        _ => canonical(path),
    };
    let real = canonical(path);
    grants
        .iter()
        .filter_map(|g| std::fs::canonicalize(g).ok())
        .any(|g| named.starts_with(&g) || real.starts_with(&g))
}

impl SandboxPolicy {
    /// Whether the owner's own `sandbox.read` already grants the global git configuration (`~/.gitconfig`): then the node does not give
    /// the agent a file of its own (ADR-0078, decision 4) and the explicit choice stands as it is.
    pub fn grants_gitconfig(&self) -> bool {
        crate::usage_files::home_dir()
            .map(|h| h.join(".gitconfig"))
            .is_some_and(|g| self.read.contains(&g))
    }

    /// Prepares the generated Git file when its directory can be kept private except for the file's own read grant.
    pub fn git_config(&self, profile: &agora_platform::Sandbox) -> Option<PathBuf> {
        if self.level == Level::Off || self.grants_gitconfig() {
            return None;
        }
        let data_dir = std::fs::canonicalize(self.data_dir.as_ref()?).ok()?;
        data_dir.to_str()?;
        let directory = crate::gitconfig::directory(&data_dir);
        // An existing directory can have a different spelling on a case-insensitive volume.
        let resolved = std::fs::canonicalize(&directory).unwrap_or(directory);
        if under_a_grant(&resolved, &profile.read) || under_a_grant(&resolved, &profile.write) {
            return None;
        }
        crate::gitconfig::write_for(&data_dir)
    }

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
        // ADR-0083: unix sockets only where the agent was given them (what the system can refuse, `socket_confinement`): its worktree,
        // its private temp folder and what the owner listed, outside the node's own folders; the supervisor adds its mesh socket.
        sb.confine_sockets = true;
        sb.sockets.push(worktree.to_path_buf());
        if agora_platform::PRIVATE_TEMP {
            sb.sockets.push(agent_temp(worktree));
        }
        let protect: Vec<PathBuf> = self.protect.iter().map(|p| canonical(p)).collect();
        sb.sockets.extend(
            self.sockets
                .iter()
                .filter(|s| {
                    let real = canonical(s);
                    !self.protect.iter().any(|p| s.starts_with(p))
                        && !protect.iter().any(|p| real.starts_with(p))
                })
                .cloned(),
        );
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
        // The agent's own program folder, when it is named by path (never a filesystem root).
        if program.contains(['/', std::path::MAIN_SEPARATOR]) {
            if let Some(dir) = Path::new(program).parent() {
                if !dir.as_os_str().is_empty() && dir.parent().is_some() {
                    read.push(dir.to_path_buf());
                }
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
        let mut sb = agora_platform::Sandbox::new(read, write);
        sb.deny = self.protect.clone();
        // The worktree the node made is the one folder of its home the agent gets back; checked again at every start, so a worktree
        // the agent turned into a link stays closed.
        sb.reopen = vec![worktree.to_path_buf()];
        sb
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
        let mut sb = agora_platform::Sandbox::new(read, write);
        sb.deny = self.protect.clone();
        // The worktree the node made is the one folder of its home the agent gets back; checked again at every start, so a worktree
        // the agent turned into a link stays closed.
        sb.reopen = vec![worktree.to_path_buf()];
        sb
    }

    /// ADR-0082, decision 4: whether `profile` lets the agent read or write the control token file (a grant that is the file, or any
    /// folder above it, by name or by where the names lead). Always false when the node does not require the token.
    pub fn token_reachable(&self, profile: &agora_platform::Sandbox) -> bool {
        let Some(file) = &self.control_token else {
            return false;
        };
        self.path_reachable(file, profile)
    }

    /// Whether `profile` lets the agent read or write `path` (a grant that is the path, or any folder above it, by name or by where the
    /// names lead). Landlock cannot say "except", so such a path must lie outside every grant (ADR-0082, ADR-0084).
    pub fn path_reachable(&self, path: &Path, profile: &agora_platform::Sandbox) -> bool {
        let grants: Vec<PathBuf> = profile
            .read
            .iter()
            .chain(profile.write.iter())
            .cloned()
            .collect();
        under_a_grant(path, &grants)
    }

    /// #1114: the first of the node's own paths (`protect`, and its data folder) that `profile` lets the agent read or write, where only
    /// a refusal keeps them closed (`agora_platform::PROTECTED_PATHS_BY_REFUSAL`: Linux, whose Landlock cannot say "except", so
    /// `Sandbox::deny` is ignored there). `None` where the system enforces `deny` itself, and when agents are not confined.
    pub fn protected_reachable(&self, profile: &agora_platform::Sandbox) -> Option<&Path> {
        if !agora_platform::PROTECTED_PATHS_BY_REFUSAL || self.level == Level::Off {
            return None;
        }
        self.first_protected_reachable(profile)
    }

    /// #1150: the paths [`Self::token_reachable`] and [`Self::protected_reachable`] keep out of every grant, for the platform to check
    /// again on what it opens (`agora_platform::Sandbox::closed`): the control token file, and where only a refusal keeps them closed
    /// (Linux) the node's own paths and its data folder. Empty elsewhere and when agents are not confined.
    pub fn closed_paths(&self) -> Vec<PathBuf> {
        if !agora_platform::PROTECTED_PATHS_BY_REFUSAL || self.level == Level::Off {
            return Vec::new();
        }
        self.control_token
            .iter()
            .chain(self.protect.iter())
            .chain(self.data_dir.iter())
            .cloned()
            .collect()
    }

    /// [`Self::protected_reachable`] on every system (the unit tests' entry). Only a grant at or above a protected path counts: one below
    /// it (the agent's own worktree in the node's folder, a program `agora agent install` keeps there) is the opening the node meant.
    fn first_protected_reachable(&self, profile: &agora_platform::Sandbox) -> Option<&Path> {
        let grants: Vec<PathBuf> = profile
            .read
            .iter()
            .chain(profile.write.iter())
            .cloned()
            .collect();
        self.protect
            .iter()
            .chain(self.data_dir.iter())
            .find(|p| reaches(p, &grants))
            .map(PathBuf::as_path)
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

    /// #897: whether an agent started as a plain process (an adapter or ACP bridge, not a terminal) can be confined at this level here.
    pub fn enforceable_for_commands(&self) -> bool {
        matches!(self.level, Level::Off)
            || (self.enforceable() && agora_platform::COMMAND_CONFINEMENT_SUPPORTED)
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
    fn a_git_file_linked_to_another_worktrees_opens_nothing() {
        let root = std::env::temp_dir().join(format!("agora-gitlink-{}", std::process::id()));
        let (mine, _, _) = linked_worktree(&root.join("mine"));
        let (theirs, their_common, their_admin) = linked_worktree(&root.join("theirs"));
        assert!(git_dirs(&theirs).is_some());
        // The agent swaps its `.git` file for a link to another worktree's: read through the link, that file's `gitdir:` and the
        // back-pointer agree, so only refusing the link keeps the other repository closed.
        std::fs::remove_file(mine.join(".git")).unwrap();
        if agora_platform::symlink(&theirs.join(".git"), &mine.join(".git")).is_err() {
            let _ = std::fs::remove_dir_all(&root);
            return; // No links on this system.
        }
        assert_eq!(git_dirs(&mine), None);
        let policy = SandboxPolicy {
            level: Level::Worktree,
            ..SandboxPolicy::default()
        };
        let sb = policy.profile("k", None, &mine, "agent").unwrap();
        assert!(!sb
            .write
            .iter()
            .any(|w| w.starts_with(&their_common) || w == &their_admin));
        let _ = std::fs::remove_dir_all(&root);
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
    fn two_worktrees_of_the_same_name_never_share_a_private_temp_folder() {
        // Audit F841: two projects' `web` folders (a directory launch uses the folder itself as the worktree).
        let t = |p: &str| agent_temp(Path::new(p));
        assert_ne!(t("/work/client-a/web"), t("/work/client-b/web"));
        // The same worktree keeps its folder across restarts, still named after it.
        assert_eq!(t("/work/client-a/web"), t("/work/client-a/web"));
        let name = t("/work/client-a/web");
        assert!(name
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("agora-agent-web-"));
        // A name at the 255-byte limit, in ASCII and in four-byte characters, still gives a folder name within it (Codex review of
        // F841: the place made a 240-character name too long); two such names that differ only past the cut stay apart.
        for long in ["x".repeat(255), "𝔵".repeat(63)] {
            assert!(t(&format!("/work/a/{long}")).file_name().unwrap().len() <= 255);
        }
        let (a, b) = (
            t(&format!("/work/{}1", "y".repeat(100))),
            t(&format!("/work/{}2", "y".repeat(100))),
        );
        assert_ne!(a, b);
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
            protect: vec!["/node/home".into()],
            data_dir: None,
            control_token: None,
            pass_env: vec![],
            sockets: vec!["/run/user/1/keep.sock".into(), "/node/home/m/x".into()],
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
        assert!(offline.offline);
        // ADR-0083: the allow-list is the worktree and what the owner listed (not a socket below the node's own folders, also through a link).
        assert!(sb.confine_sockets && offline.confine_sockets);
        assert!(sb.sockets.contains(&wt) && !sb.sockets.contains(&PathBuf::from("/tmp")));
        assert!(sb.sockets.contains(&PathBuf::from("/run/user/1/keep.sock")));
        assert!(!sb.sockets.contains(&PathBuf::from("/node/home/m/x")));
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

    #[test]
    fn a_grant_that_contains_the_control_token_makes_it_reachable() {
        let base = std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!("agora-adr82-reach-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("home/worktrees/a")).unwrap();
        std::fs::create_dir_all(base.join("home2")).unwrap();
        let tok = base.join("home/control.token");
        let policy = SandboxPolicy {
            level: Level::Worktree,
            control_token: Some(tok.clone()),
            ..SandboxPolicy::default()
        };
        let grant = |read: &[&Path], write: &[&Path]| {
            agora_platform::Sandbox::new(
                read.iter().map(|p| p.to_path_buf()).collect(),
                write.iter().map(|p| p.to_path_buf()).collect(),
            )
        };
        // Outside every grant, also beside it and in a folder of the same name prefix.
        let (wt, home2) = (base.join("home/worktrees/a"), base.join("home2"));
        assert!(!policy.token_reachable(&grant(&[Path::new("/usr")], &[&wt, &home2])));
        // The folder above, the home itself and the file itself, as read or as write grants.
        for g in [base.clone(), base.join("home"), tok.clone()] {
            assert!(policy.token_reachable(&grant(&[&g], &[])), "{g:?}");
            assert!(policy.token_reachable(&grant(&[], &[&g])), "{g:?}");
        }
        // Where a link leads counts too: a grant that is a link to the home.
        #[cfg(unix)]
        {
            let link = base.join("link");
            std::os::unix::fs::symlink(base.join("home"), &link).unwrap();
            assert!(policy.token_reachable(&grant(&[&link], &[])));
        }
        // A node that does not require the token refuses nothing.
        let off = SandboxPolicy::default();
        assert!(!off.token_reachable(&grant(&[Path::new("/")], &[Path::new("/")])));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_link_to_a_missing_place_on_the_way_to_one_of_the_nodes_paths_counts_as_reached() {
        let base = std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!("agora-1114-dangling-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (home, open) = (base.join("home"), base.join("open"));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&open).unwrap();
        // `--manifests ~/m/manifests` where `~/m` leads into a granted folder whose target is gone (a reboot cleared `/tmp`): an agent
        // that may write there makes the target and puts manifests in it. Also the protected path itself as such a link.
        let (mlink, self_link) = (home.join("mlink"), home.join("self-link"));
        if agora_platform::symlink(&open.join("later"), &mlink).is_err()
            || agora_platform::symlink(&open.join("gone"), &self_link).is_err()
        {
            let _ = std::fs::remove_dir_all(&base);
            return; // No links on this system.
        }
        let grants = vec![open.clone()];
        assert!(reaches(&mlink.join("manifests"), &grants));
        assert!(reaches(&self_link, &grants));
        // Fail closed: wherever the missing target would be, the place cannot be known.
        assert!(reaches(
            &mlink.join("manifests"),
            &[Path::new("/usr").to_path_buf()]
        ));
        // A missing path with no link on its way is still judged by its nearest existing folder.
        assert!(!reaches(&home.join("missing/manifests"), &grants));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_grant_at_or_above_one_of_the_nodes_paths_makes_it_reachable() {
        let base = std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!("agora-1114-reach-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (home, data, open) = (base.join("home"), base.join("data"), base.join("open"));
        std::fs::create_dir_all(home.join("worktrees/a")).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(&open).unwrap();
        let policy = SandboxPolicy {
            level: Level::Worktree,
            protect: vec![home.clone(), base.join("missing/deeper/manifests")],
            data_dir: Some(data.clone()),
            ..SandboxPolicy::default()
        };
        let grant = |read: &[&Path], write: &[&Path]| {
            agora_platform::Sandbox::new(
                read.iter().map(|p| p.to_path_buf()).collect(),
                write.iter().map(|p| p.to_path_buf()).collect(),
            )
        };
        let wt = home.join("worktrees/a");
        // Outside every grant; a grant below the node's folder (its own worktree) is the opening the node meant; a path that does not
        // exist yet below no grant is not refused.
        assert_eq!(
            policy.first_protected_reachable(&grant(&[Path::new("/usr"), &open], &[&wt])),
            None
        );
        // The folder above, the node's folder, the data folder (in `data_dir` though not in `protect`), as read or write grants.
        for (g, hit) in [(&base, &home), (&home, &home), (&data, &data)] {
            assert_eq!(
                policy.first_protected_reachable(&grant(&[g], &[])),
                Some(hit.as_path())
            );
            assert_eq!(
                policy.first_protected_reachable(&grant(&[], &[g])),
                Some(hit.as_path())
            );
        }
        // A path that does not exist yet, below a grant.
        assert!(policy
            .first_protected_reachable(&grant(&[&base.join("missing")], &[]))
            .is_some());
        // Where links lead counts: a grant that is a link to the node's folder, and a data folder that is a link into a grant.
        let link = base.join("link");
        let linked_data = base.join("linked-data");
        if agora_platform::symlink(&home, &link).is_ok()
            && agora_platform::symlink(&open.join("d"), &linked_data).is_ok()
        {
            std::fs::create_dir_all(open.join("d")).unwrap();
            assert!(policy
                .first_protected_reachable(&grant(&[&link], &[]))
                .is_some());
            let linked = SandboxPolicy {
                protect: vec![linked_data.clone()],
                data_dir: None,
                ..policy.clone()
            };
            assert_eq!(
                linked.first_protected_reachable(&grant(&[&open], &[])),
                Some(linked_data.as_path())
            );
        }
        // Only where the system cannot keep them closed itself, and only when agents are confined.
        let wide = grant(&[&base], &[]);
        assert_eq!(
            policy.protected_reachable(&wide).is_some(),
            agora_platform::PROTECTED_PATHS_BY_REFUSAL
        );
        let off = SandboxPolicy {
            level: Level::Off,
            ..policy.clone()
        };
        assert_eq!(off.protected_reachable(&wide), None);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn socket_variables_are_scrubbed_unless_the_owner_names_them() {
        let mut p = SandboxPolicy::default();
        for n in [
            "SSH_AUTH_SOCK",
            "GPG_AGENT_INFO",
            "DBUS_SESSION_BUS_ADDRESS",
            "DOCKER_HOST",
            "TMUX",
            "TMUX_PANE",
        ] {
            assert!(p.scrubbed_env().iter().any(|s| s == n), "{n}");
        }
        p.pass_env = parse_pass_env("SSH_AUTH_SOCK, DOCKER_HOST");
        let left = p.scrubbed_env();
        assert!(!left
            .iter()
            .any(|s| s == "SSH_AUTH_SOCK" || s == "DOCKER_HOST"));
        assert!(left.iter().any(|s| s == "TMUX"));
    }

    #[test]
    fn a_malformed_pass_env_passes_nothing() {
        assert_eq!(parse_pass_env("A_B, c1"), ["A_B", "c1"]);
        assert!(parse_pass_env("").is_empty());
        for bad in [
            "SSH_AUTH_SOCK=/tmp/x",
            "SSH_*",
            "1ABC",
            "A B",
            "SSH_AUTH_SOCK, ",
            "OK, $HOME",
            "OK,é",
        ] {
            let got = parse_pass_env(bad);
            // A trailing comma is not malformed; every other bad entry empties the whole list.
            if bad == "SSH_AUTH_SOCK, " {
                assert_eq!(got, ["SSH_AUTH_SOCK"]);
            } else {
                assert!(got.is_empty(), "{bad}: {got:?}");
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn an_owner_socket_that_leads_into_the_nodes_folder_through_a_link_is_dropped() {
        let dir = std::env::temp_dir().join(format!("agora-sock-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (home, wt) = (dir.join("home"), dir.join("wt"));
        std::fs::create_dir_all(home.join("m")).unwrap();
        std::fs::create_dir_all(&wt).unwrap();
        std::os::unix::fs::symlink(&home, dir.join("alias")).unwrap();
        let p = SandboxPolicy {
            level: Level::Worktree,
            protect: vec![home.clone()],
            sockets: vec![
                dir.join("alias/m/agent.1"),
                home.join("m/agent.1"),
                dir.join("ok.sock"),
            ],
            ..Default::default()
        };
        let sb = p.profile("k", None, &wt, "/bin/sh").unwrap();
        assert!(sb.sockets.contains(&dir.join("ok.sock")));
        assert!(
            !sb.sockets.iter().any(|s| s.ends_with("agent.1")),
            "{:?}",
            sb.sockets
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn root_program_parent_is_not_added_to_sandbox_read() {
        let wt = std::env::temp_dir().join(format!("agora-sb-root-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&wt);
        let p = SandboxPolicy {
            level: Level::Worktree,
            ..Default::default()
        };
        let sb = p.profile("k", None, &wt, "/agent").unwrap();
        assert!(!sb.read.contains(&PathBuf::from("/")));
        let ctrl = p.profile("k", None, &wt, "/usr/local/bin/agent").unwrap();
        assert!(ctrl.read.contains(&PathBuf::from("/usr/local/bin")));
        let _ = std::fs::remove_dir_all(&wt);
    }
}

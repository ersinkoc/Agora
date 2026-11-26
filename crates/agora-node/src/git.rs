//! Git operations on a node: checkout discovery, worktrees, status and diff (ADR-0003).
//!
//! Uses the `git` command line (stable, ubiquitous, no extra dependency). Every call runs with a clean locale,
//! no prompts, and paths/branches after `--` or validated first, so repository content cannot inject options.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, PoisonError};

use agora_core::{ProjectId, Worktree, WorktreeError};

mod utf16;
use agora_protocol::{Error, ErrorCode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    NotARepository(PathBuf),
    /// Uncommitted changes block the operation.
    Dirty(PathBuf),
    BranchExists(String),
    /// The branch has commits that exist on no other branch or remote.
    Unmerged(String),
    InvalidBranch(String),
    /// A worktree's Git links (`commondir`) no longer name the Git folders the node pinned it to (T8b): refused, never answered from
    /// whatever they name now.
    LinksMoved(PathBuf),
    /// git itself failed; `stderr` is for logs, never for users.
    Command {
        args: String,
        stderr: String,
    },
    Io(String),
}

impl GitError {
    /// Map to the protocol error `{ code, params }` at the boundary.
    pub fn to_protocol(&self) -> Error {
        let path = |p: &Path| p.display().to_string();
        match self {
            Self::NotARepository(p) => {
                Error::new(ErrorCode::ProtocolInvalidMessage).with("field", path(p).as_str())
            }
            Self::Dirty(p) => Error::new(ErrorCode::WorktreeDirty).with("path", path(p).as_str()),
            Self::BranchExists(b) => {
                Error::new(ErrorCode::WorktreeExists).with("branch", b.as_str())
            }
            Self::Unmerged(b) => Error::new(ErrorCode::WorktreeUnmerged).with("branch", b.as_str()),
            Self::InvalidBranch(_) => {
                Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "branch")
            }
            Self::LinksMoved(_) | Self::Command { .. } | Self::Io(_) => {
                Error::new(ErrorCode::Internal).with("incident_id", "unassigned")
            }
        }
    }
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_protocol())
    }
}

impl std::error::Error for GitError {}

/// A project's working copy found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckoutInfo {
    pub root: PathBuf,
    pub project: ProjectId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStatus {
    /// Changed paths including untracked files.
    pub files_changed: u32,
    /// Line counts of tracked changes against HEAD.
    pub insertions: u32,
    pub deletions: u32,
    pub head_commit: String,
}

/// Settings that would let a repository run a program when the node reads it, switched off for every git the node runs. The node's
/// git is not confined, and an agent may write the git config and hooks a worktree uses (its checkout's `.git`): a planted
/// `core.fsmonitor` runs on `status`, a `post-checkout` hook on `worktree add`, an external diff or a textconv on `diff`.
/// Filter drivers (`.gitattributes` + `filter.<name>.clean`/`smudge`/`process`) have no single switch; [`hardened`] blanks each
/// one the repository's own config defines.
const HARDENED: [&str; 8] = [
    "-c",
    "core.fsmonitor=false",
    "-c",
    "core.hooksPath=/dev/null",
    "-c",
    "diff.external=",
    "-c",
    "core.attributesFile=/dev/null",
];

/// `git -C dir` with [`HARDENED`] and the node's environment (no prompts, no optional locks, plain English output), and with every
/// filter driver defined in the repository's own config (see [`repository_filters`]) blanked.
pub(crate) fn hardened(dir: &Path) -> Command {
    hardened_with(dir, &Filters::of(dir))
}

/// The filter drivers to blank for one repository, asked right before each command. An agent that can write the config could still
/// add one between the question and the command; [`plain_hardened`] pins a node-made worktree's Git folders so that, where the shared
/// config is read-only to agents (macOS, Windows), there is no config it can write (T8b).
#[derive(Debug, PartialEq, Eq)]
enum Filters {
    Blank(Vec<String>),
    /// One of them cannot be named back to git (not UTF-8), or git could not answer: nothing runs.
    Refuse,
}

impl Filters {
    fn of(dir: &Path) -> Self {
        let mut command = plain_hardened(dir);
        let out = command
            .args([
                "config",
                "--show-scope",
                "--name-only",
                "-z",
                "--get-regexp",
                r"^filter\..*\.(clean|smudge|process)$",
            ])
            .output();
        match out {
            Ok(_) if moved_links(&command).is_some() => Self::Refuse,
            Ok(o) => Self::from_answer(o.status.code(), &o.stdout),
            Err(_) => Self::Refuse,
        }
    }

    /// 0 is a list, and 1 "no such key" only with nothing printed; anything else (129 for a git before 2.26 that does not know
    /// `--show-scope`, 128 for a config it cannot read, a signal) is a refusal, not "nothing to blank".
    fn from_answer(code: Option<i32>, out: &[u8]) -> Self {
        match code {
            Some(0) => repository_filters(out).map_or(Self::Refuse, Self::Blank),
            Some(1) if out.is_empty() => Self::Blank(vec![]),
            _ => Self::Refuse,
        }
    }
}

fn hardened_with(dir: &Path, filters: &Filters) -> Command {
    let mut c = plain_hardened(dir);
    match filters {
        Filters::Blank(keys) if keys.is_empty() => {}
        // Through the environment, not `-c`: a filter's name may hold a `=`, which `-c name=value` splits at, leaving it on.
        Filters::Blank(keys) if config_env_supported() => {
            c.env("GIT_CONFIG_COUNT", keys.len().to_string());
            for (i, key) in keys.iter().enumerate() {
                c.env(format!("GIT_CONFIG_KEY_{i}"), key)
                    .env(format!("GIT_CONFIG_VALUE_{i}"), "");
            }
        }
        // A filter that cannot be blanked (a name that is not UTF-8, or a git older than 2.31 that ignores `GIT_CONFIG_KEY_n`):
        // a `-c` key with no section makes every git refuse to start, so nothing runs.
        Filters::Blank(_) | Filters::Refuse => {
            c.args(REFUSED);
        }
    }
    c
}

/// A `-c` key with no section: every git refuses to start on it ("key does not contain a section"), so nothing runs.
const REFUSED: [&str; 2] = ["-c", "refused-filter-driver"];

/// Whether this machine's git reads `GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_n` (git 2.31 and later). Asked once per node run.
fn config_env_supported() -> bool {
    static SUPPORTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        agora_platform::no_console_window(&mut Command::new("git"))
            .args(["config", "--get", "probe.configenv"])
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "probe.configenv")
            .env("GIT_CONFIG_VALUE_0", "yes")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .is_ok_and(|o| o.stdout.trim_ascii() == b"yes")
    })
}

/// One worktree this node made, recorded by [`create_worktree`] as soon as `git worktree add` returns, before any agent runs there:
/// its Git folders (plain canonical spellings), what git wrote into the admin folder's `commondir` and `gitdir`, and the project of
/// the checkout it was made from.
#[derive(Clone)]
struct Made {
    work: PathBuf,
    admin: PathBuf,
    common: PathBuf,
    commondir: Vec<u8>,
    gitdir: Vec<u8>,
    project: Option<ProjectId>,
}

/// Every worktree this node made and has not removed. Its `.git` file and everything it names can be rewritten by the agent working
/// there, which can also build a whole repository inside the worktree whose `worktrees/<name>/gitdir` points back at it, so a check
/// of the file at the time of a command proves nothing; what git said when the node made the worktree does. The node's git there is
/// pinned to these folders ([`plain_hardened`]), but git still reads branch refs through the admin folder's `commondir`, which the
/// agent can also rewrite: every command checks that it still names the recorded shared folder ([`links_hold`]), and removal puts
/// back the recorded `.git`, `commondir` and `gitdir` first. The project comes from here too, not from the worktree's HEAD, which
/// the agent can move. A node that restarts starts with none of these and falls back to [`crate::sandbox::git_dirs`] (with the
/// same `commondir` check) and the identity git computes; an agent the holder kept is taken up in the project its ledger entry
/// recorded.
static MADE: Mutex<Vec<Made>> = Mutex::new(Vec::new());

fn made() -> std::sync::MutexGuard<'static, Vec<Made>> {
    // A panic while holding the lock leaves a plain list behind, still valid.
    MADE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Records `work`'s Git folders, links and project (see [`MADE`]); nothing when they do not check out.
fn record_made(work: &Path, project: Option<ProjectId>) {
    let plain = agora_platform::canonicalize_plain;
    let Some((Ok(work), Ok(admin), Ok(common))) = crate::sandbox::git_dirs(work)
        .map(|(admin, common)| (plain(work), plain(&admin), plain(&common)))
    else {
        return;
    };
    let (Ok(commondir), Ok(gitdir)) = (
        read_link_file(&admin.join("commondir")),
        read_link_file(&admin.join("gitdir")),
    ) else {
        return;
    };
    if !links_hold(&admin, &common) {
        return;
    }
    let mut list = made();
    list.retain(|m| m.work != work);
    list.push(Made {
        work,
        admin,
        common,
        commondir,
        gitdir,
        project,
    });
}

/// Test hook: forgets the record of the worktree at `work`, as a node that restarts has none ([`MADE`]).
#[doc(hidden)]
pub fn forget_made_worktree(work: &Path) {
    if let Ok(work) = agora_platform::canonicalize_plain(work) {
        made().retain(|m| m.work != work);
    }
}

/// The record of the worktree this node made that `dir` is, or is inside.
fn made_dirs(dir: &Path) -> Option<Made> {
    let dir = agora_platform::canonicalize_plain(dir).ok()?;
    made()
        .iter()
        .filter(|m| dir.starts_with(&m.work))
        // The innermost one, should one worktree have been made inside another.
        .max_by_key(|m| m.work.components().count())
        .cloned()
}

/// Where the node's git for one folder looks for its repository.
enum Pin {
    /// Not a linked worktree's root: git finds the repository as it always does.
    Unpinned,
    /// A `.git` that is a link: nothing runs.
    Refused,
    /// `(GIT_DIR, GIT_COMMON_DIR, GIT_WORK_TREE)`.
    Dirs(PathBuf, PathBuf, PathBuf),
}

/// In a worktree this node made (or a folder below it) the folders recorded when it was made ([`MADE`]). In any other linked worktree
/// the ones [`crate::sandbox::git_dirs`] verifies, and a `.git` that is a link is refused. Any other `.git` file (a submodule's, a
/// separate git dir) is followed as git does.
fn pin(dir: &Path) -> Pin {
    if let Some(m) = made_dirs(dir) {
        return Pin::Dirs(m.admin, m.common, m.work);
    }
    let Ok(meta) = std::fs::symlink_metadata(dir.join(".git")) else {
        return Pin::Unpinned;
    };
    if meta.is_symlink() {
        return Pin::Refused;
    }
    if !meta.is_file() {
        return Pin::Unpinned;
    }
    // Plain spellings, as for any path a process is given (no verbatim `\\?\` prefix on Windows).
    let plain = agora_platform::canonicalize_plain;
    match crate::sandbox::git_dirs(dir)
        .map(|(admin, common)| (plain(&admin), plain(&common), plain(dir)))
    {
        Some((Ok(admin), Ok(common), Ok(work))) => Pin::Dirs(admin, common, work),
        _ => Pin::Unpinned,
    }
}

/// Whether the admin folder's `commondir` (a plain file, as git writes it) still names `common`. git's ref store reads it even when
/// `GIT_COMMON_DIR` is set (measured with git 2.53, unlike what `git help repository-layout` says): config and objects stay pinned, but HEAD's
/// branch and every other shared ref would come from the repository it names, and a git process the node's git starts without the
/// pinning (`git worktree remove` runs a `status` in the worktree) would read that repository's config. Read as git reads it:
/// trailing line ends dropped, a relative path taken from the admin folder. Read once with [`read_link_file`]: a link, a FIFO or a
/// long file there is a refusal, never followed, waited on or read whole.
fn links_hold(admin: &Path, common: &Path) -> bool {
    let Some(text) = read_link_file(&admin.join("commondir"))
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
    else {
        return false;
    };
    let named = text.trim_end_matches(['\n', '\r']);
    if named.is_empty() {
        return false;
    }
    let canonical = agora_platform::canonicalize_plain;
    match (canonical(&admin.join(named)), canonical(common)) {
        (Ok(named), Ok(common)) => named == common,
        _ => false,
    }
}

/// The most a git link file (`.git`, `commondir`, `gitdir`) is read: a path (at most 4096 bytes on Linux) and `gitdir: `. An agent may
/// write these files; anything longer is not one git wrote.
pub(crate) const LINK_FILE_MAX: u64 = 4096 + 64;

/// A git link file an agent may have rewritten, read as [`agora_platform::read_small_regular_file_no_follow`] reads it: only a plain
/// file of at most [`LINK_FILE_MAX`] bytes, opened once without following a link or waiting on a FIFO.
pub(crate) fn read_link_file(path: &Path) -> std::io::Result<Vec<u8>> {
    agora_platform::read_small_regular_file_no_follow(path, LINK_FILE_MAX)
}

/// `Err` when `dir` is pinned to Git folders whose `commondir` no longer names the shared one ([`links_hold`]).
fn check_links(dir: &Path) -> Result<(), GitError> {
    match pin(dir) {
        Pin::Dirs(admin, common, _) if !links_hold(&admin, &common) => {
            Err(GitError::LinksMoved(dir.to_path_buf()))
        }
        _ => Ok(()),
    }
}

/// `f` between two [`check_links`]: an answer read while the links named another repository is a refusal, not data. A rewrite that
/// is undone again within `f` is not seen (it can only change the refs read, never the config or objects).
fn with_links<T>(dir: &Path, f: impl FnOnce() -> Result<T, GitError>) -> Result<T, GitError> {
    check_links(dir)?;
    let out = f();
    check_links(dir)?;
    out
}

/// [`GitError::LinksMoved`] when `command` was pinned ([`plain_hardened`]) to Git folders whose `commondir` no longer names the shared
/// one: asked after it ran, so a rewrite made while it ran turns its answer into a refusal.
fn moved_links(command: &Command) -> Option<GitError> {
    let (mut admin, mut common, mut work) = (None, None, None);
    for (key, value) in command.get_envs() {
        match key.to_str() {
            Some("GIT_DIR") => admin = value,
            Some("GIT_COMMON_DIR") => common = value,
            Some("GIT_WORK_TREE") => work = value,
            _ => {}
        }
    }
    let (admin, common) = (Path::new(admin?), Path::new(common?));
    (!links_hold(admin, common))
        .then(|| GitError::LinksMoved(PathBuf::from(work.unwrap_or(admin.as_os_str()))))
}

/// `git -C dir` with [`HARDENED`] and the node's environment, its Git folders pinned (`GIT_DIR`, `GIT_COMMON_DIR`, `GIT_WORK_TREE`)
/// as [`pin`] says, so nothing the agent writes into its `.git` file points the node's git at a config the agent wrote. git still
/// reads the branch refs through the admin folder's `commondir` (see [`links_hold`]): a pinned command whose `commondir` does not
/// name the pinned shared folder is refused here, and [`run`] refuses its answer when it no longer does once it ran.
fn plain_hardened(dir: &Path) -> Command {
    let mut c = Command::new("git");
    // The node has no console: on Windows each git would otherwise open a window and take the focus (a diff runs several).
    agora_platform::no_console_window(&mut c);
    c.args(HARDENED)
        .arg("-C")
        .arg(dir)
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_DIR")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_WORK_TREE");
    match pin(dir) {
        Pin::Unpinned => {}
        Pin::Refused => {
            c.args(REFUSED);
        }
        Pin::Dirs(admin, common, work) => {
            if !links_hold(&admin, &common) {
                c.args(REFUSED);
            }
            c.env("GIT_DIR", admin)
                .env("GIT_COMMON_DIR", common)
                .env("GIT_WORK_TREE", work);
        }
    }
    c
}

/// The filter-driver keys to blank, from `git config --show-scope --name-only -z` output (`scope NUL key NUL` pairs): those set in the
/// repository's own config (`local`, `worktree`, and what they include), which an agent can write. The user's global and system
/// filters (Git LFS) are kept. `None` when one of them is not UTF-8 and so cannot be named back to git.
fn repository_filters(out: &[u8]) -> Option<Vec<String>> {
    let fields: Vec<&[u8]> = out.split(|b| *b == 0).collect();
    fields
        .as_chunks::<2>()
        .0
        .iter()
        .filter(|[scope, _]| matches!(*scope, b"local" | b"worktree"))
        .map(|[_, key]| std::str::from_utf8(key).ok().map(str::to_owned))
        .collect()
}

/// [`plain_hardened`] for a command that only reads or moves refs and never reads a file's contents (`rev-parse`, `rev-list`,
/// `log` without a patch, `show-ref`, `symbolic-ref`, `update-ref`, `remote`, `worktree list`): a filter driver runs only when git
/// hashes or checks out a file, so these skip the filter question and cost one git process instead of two (about a second each on
/// Windows). Anything that can touch a file's contents (`status`, `diff`, `add`, `worktree add`/`remove`, `ls-files`) goes
/// through [`hardened`]. A `log` format must not ask for signatures (`%G?` and friends; measured: with them, or with the default
/// format, `log.showSignature` + `gpg.program` from the repository's config runs that program).
pub(crate) fn refs_only(dir: &Path) -> Command {
    plain_hardened(dir)
}

/// [`git`] for the commands [`refs_only`] names.
fn git_refs(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    run(refs_only(dir), args)
}

fn git(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    run(hardened(dir), args)
}

fn run(mut command: Command, args: &[&str]) -> Result<String, GitError> {
    let out = command
        .args(args)
        .output()
        .map_err(|e| GitError::Io(e.to_string()))?;
    if let Some(moved) = moved_links(&command) {
        return Err(moved);
    }
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(GitError::Command {
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

/// Find the checkout containing `path` and compute its machine-independent project identity. In a worktree this node made, the root is
/// that worktree and the project the one of the checkout it was made from, recorded before its agent ran ([`MADE`]): its agent can
/// move its branch to a root commit of its own or change the remote, and neither makes it another project.
pub fn discover_checkout(path: &Path) -> Result<CheckoutInfo, GitError> {
    if let Some(Made {
        work,
        project: Some(project),
        ..
    }) = made_dirs(path)
    {
        return Ok(CheckoutInfo {
            root: work,
            project,
        });
    }
    with_links(path, || compute_checkout(path))
}

fn compute_checkout(path: &Path) -> Result<CheckoutInfo, GitError> {
    let root = git_refs(path, &["rev-parse", "--show-toplevel"])
        .map_err(|_| GitError::NotARepository(path.to_path_buf()))?;
    // Rebuilt from its parts so it uses this system's separator: git for Windows answers `C:/...`.
    let root = root.strip_suffix('\n').unwrap_or(&root);
    let root: PathBuf = Path::new(root).components().collect();
    // The two identity queries run at the resolved root, so the `.git` checks in `git_command` see the checkout's own `.git`
    // (a linked `.git` is refused even when asked from a subfolder). They run side by side: one serial git spawn less, which
    // shows on Windows. A thread that cannot be started is no error: that query then runs in order instead.
    let query_remote = || git_refs(&root, &["remote", "get-url", "origin"]).ok();
    let (roots, remote) = std::thread::scope(|s| {
        let remote = std::thread::Builder::new()
            .spawn_scoped(s, query_remote)
            .ok();
        let roots = git_refs(&root, &["rev-list", "--max-parents=0", "HEAD"]);
        let remote = match remote {
            Some(handle) => handle.join().unwrap_or_else(|_| query_remote()),
            None => query_remote(),
        };
        (roots, remote)
    });
    // A repository can have several root commits; the lexicographically smallest is a stable choice.
    let roots = roots?;
    let root_commit = roots
        .lines()
        .min()
        .ok_or_else(|| GitError::NotARepository(root.clone()))?
        .to_string();
    let project = ProjectId::new(remote.as_deref().map(str::trim), &root_commit)
        .or_else(|_| ProjectId::new(None, &root_commit))
        .map_err(|_| GitError::NotARepository(root.clone()))?;
    Ok(CheckoutInfo { root, project })
}

/// Create a worktree at `path` on a new branch `branch`, starting from `base` (default: HEAD).
pub fn create_worktree(
    checkout: &Path,
    path: &Path,
    branch: &str,
    base: Option<&str>,
) -> Result<(), GitError> {
    // Reuse the domain rules for branch names before they reach git.
    let probe = agora_core::Checkout {
        id: agora_core::CheckoutId::new("probe"),
        project: ProjectId::new(None, &"0".repeat(40))
            .map_err(|_| GitError::InvalidBranch(branch.into()))?,
        node: agora_core::NodeId::new("probe"),
        path: String::new(),
    };
    Worktree::new(agora_core::WorktreeId::new("probe"), &probe, "", branch)
        .map_err(|WorktreeError::InvalidBranch| GitError::InvalidBranch(branch.into()))?;
    if branch_exists(checkout, branch)? {
        return Err(GitError::BranchExists(branch.into()));
    }
    let path = path.to_string_lossy();
    let mut args = vec!["worktree", "add", "-b", branch, "--", path.as_ref()];
    if let Some(base) = base {
        if base.starts_with('-') {
            return Err(GitError::InvalidBranch(base.into()));
        }
        args.push(base);
    }
    git(checkout, &args)?;
    // The checkout's project, asked once the worktree exists and before any agent runs in it: what a start records for its agent.
    let project = discover_checkout(checkout).ok().map(|c| c.project);
    record_made(Path::new(path.as_ref()), project);
    Ok(())
}

/// Receipt for a worktree made for one pending start. Ordinary worktree removal never deletes a branch.
pub(crate) struct StartWorktree {
    checkout: PathBuf,
    worktree: PathBuf,
    branch: String,
    initial: String,
}

/// Snapshot the base before creation, so later commits cannot be mistaken for disposable initial state.
/// Git still receives the original base expression to preserve its upstream setup; a moving base conservatively keeps the branch.
pub(crate) fn create_start_worktree(
    checkout: &Path,
    worktree: &Path,
    branch: &str,
    base: Option<&str>,
) -> Result<StartWorktree, GitError> {
    let expression = base.unwrap_or("HEAD");
    if expression.starts_with('-') {
        return Err(GitError::InvalidBranch(expression.into()));
    }
    let initial = git_refs(
        checkout,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{expression}^{{commit}}"),
        ],
    )?
    .trim()
    .to_owned();
    create_worktree(checkout, worktree, branch, base)?;
    Ok(StartWorktree {
        checkout: checkout.to_path_buf(),
        worktree: worktree.to_path_buf(),
        branch: branch.to_owned(),
        initial,
    })
}

impl StartWorktree {
    /// Remove this failed start's worktree first, then its branch only if still unused and at the initial commit.
    /// Changed, symbolic or checked-out branches are kept; a ref lock makes the commit comparison and deletion atomic.
    pub(crate) fn rollback(self) -> Result<(), GitError> {
        remove_worktree(&self.checkout, &self.worktree, true)?;
        let reference = format!("refs/heads/{}", self.branch);
        let checked_out = format!("branch {reference}");
        if git_refs(&self.checkout, &["worktree", "list", "--porcelain", "-z"])?
            .split('\0')
            .any(|field| field == checked_out)
        {
            return Ok(());
        }
        // Exit 1 means a direct ref. A symbolic ref or an inspection error must not authorize deletion.
        let symbolic = refs_only(&self.checkout)
            .args(["symbolic-ref", "--quiet", &reference])
            .output()
            .map_err(|e| GitError::Io(e.to_string()))?;
        if symbolic.status.code() != Some(1) {
            return Ok(());
        }
        // Never follow a symbolic ref even if another process rewrites it after the check above.
        git_refs(
            &self.checkout,
            &["update-ref", "--no-deref", "-d", &reference, &self.initial],
        )
        .map(|_| ())
    }
}

fn branch_exists(dir: &Path, branch: &str) -> Result<bool, GitError> {
    match git_refs(
        dir,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    ) {
        Ok(_) => Ok(true),
        Err(GitError::Command { .. }) => Ok(false),
        Err(e) => Err(e),
    }
}

/// Passed to every `status` and `diff` the node runs: git looks inside a submodule with that submodule's own config, whose filter
/// drivers [`hardened`] does not see (measured: a clean filter planted in `.git/modules/<name>/config` ran on the node's `status`).
/// A submodule moved to another commit still shows; edits inside it do not. [`remove_worktree`] refuses a worktree with a checked-out
/// submodule unless forced ([`has_submodules`], what git itself refuses without `--force`), so it loses nothing.
const SUBMODULE_COMMITS_ONLY: &str = "--ignore-submodules=dirty";

fn is_dirty(worktree: &Path) -> Result<bool, GitError> {
    Ok(
        !git(worktree, &["status", "--porcelain", SUBMODULE_COMMITS_ONLY])?
            .trim()
            .is_empty(),
    )
}

/// The branch checked out in a worktree (`None` when HEAD is detached).
fn current_branch(worktree: &Path) -> Result<Option<String>, GitError> {
    let b = git_refs(worktree, &["symbolic-ref", "--quiet", "--short", "HEAD"]).ok();
    Ok(b.map(|s| s.trim().to_string()))
}

/// Commits on `branch` that no other branch or remote has.
fn unmerged_commits(dir: &Path, branch: &str) -> Result<u32, GitError> {
    let tip = format!("refs/heads/{branch}");
    // For `--branches` the pattern is relative to refs/heads/.
    let exclude = format!("--exclude={branch}");
    let n = git_refs(
        dir,
        &[
            "rev-list",
            "--count",
            &tip,
            "--not",
            &exclude,
            "--branches",
            "--remotes",
        ],
    )?;
    Ok(n.trim().parse().unwrap_or(0))
}

/// Commits of the worktree's HEAD that no branch or remote has. An answer that is not a number counts as some (keep the work).
fn detached_commits(worktree: &Path) -> Result<u32, GitError> {
    let n = git(
        worktree,
        &[
            "rev-list",
            "--count",
            "HEAD",
            "--not",
            "--branches",
            "--remotes",
        ],
    )?;
    Ok(n.trim().parse().unwrap_or(u32::MAX))
}

/// Whether removing `worktree` without `--force` would be refused by git for its submodules (`validate_no_submodules`): its admin
/// folder holds a `modules` folder, or a submodule in its index is checked out (has a `.git`).
fn has_submodules(worktree: &Path) -> Result<bool, GitError> {
    let modules = git_refs(worktree, &["rev-parse", "--git-path", "modules"])?;
    let modules = Path::new(modules.trim());
    if worktree.join(modules).is_dir() {
        return Ok(true);
    }
    // `-z` entries are `mode oid stage<TAB>name`; a submodule's mode is 160000.
    let listed = git(worktree, &["ls-files", "--stage", "-z"])?;
    Ok(listed
        .split('\0')
        .filter_map(|entry| entry.strip_prefix("160000 "))
        .filter_map(|rest| rest.split_once('\t').map(|(_, name)| name))
        .any(|name| std::fs::symlink_metadata(worktree.join(name).join(".git")).is_ok()))
}

/// Remove a worktree. Refuses to discard uncommitted changes, unmerged commits or a checked-out submodule unless `force` is set.
/// The branch itself is never deleted.
///
/// Those checks are the node's own, pinned and link-checked like every read ([`with_links`]), and git is then always told `--force`:
/// without it git runs its own `status` in the worktree, a process the node cannot pin, which follows the admin folder's links to
/// whatever config they name (T8b). In a worktree this node made, its `.git` file and admin `commondir` and `gitdir` are put back
/// first as recorded ([`MADE`]); in any other, links that moved make the checks, and so the removal, refuse.
pub fn remove_worktree(checkout: &Path, worktree: &Path, force: bool) -> Result<(), GitError> {
    // Before git removes it: the folder must still exist to be spelled canonically.
    let removing = agora_platform::canonicalize_plain(worktree);
    if let Some(m) = made_dirs(worktree).filter(|m| removing.as_ref().is_ok_and(|r| *r == m.work)) {
        restore_git_file(&m.work, &m.admin);
        put_back(&m.admin.join("commondir"), &m.commondir);
        put_back(&m.admin.join("gitdir"), &m.gitdir);
    }
    if !force {
        with_links(worktree, || {
            if is_dirty(worktree)? || has_submodules(worktree)? {
                return Err(GitError::Dirty(worktree.to_path_buf()));
            }
            match current_branch(worktree)? {
                Some(branch) => {
                    if unmerged_commits(checkout, &branch)? > 0 {
                        return Err(GitError::Unmerged(branch));
                    }
                }
                // A detached HEAD: commits made there are on no branch, and removing the worktree drops the only HEAD that has them.
                None => {
                    if detached_commits(worktree)? > 0 {
                        return Err(GitError::Unmerged("HEAD".into()));
                    }
                }
            }
            Ok(())
        })?;
    }
    let path = worktree.to_string_lossy();
    git(
        checkout,
        &["worktree", "remove", "--force", "--", path.as_ref()],
    )?;
    if let Ok(work) = removing {
        made().retain(|m| m.work != work);
    }
    Ok(())
}

/// Puts back the `.git` file of a worktree this node made, which its agent may have rewritten: git refuses to remove a worktree whose
/// `.git` does not point back at its admin folder.
fn restore_git_file(work: &Path, admin: &Path) {
    put_back(
        &work.join(".git"),
        format!("gitdir: {}\n", admin.display()).as_bytes(),
    );
}

/// Writes `contents` as a new file at `file`, which an agent may have rewritten. The old entry is unlinked, not written through (it
/// may be a link to anywhere), and the new file is created only if nothing took its place; a folder there is left for git to refuse.
fn put_back(file: &Path, contents: &[u8]) {
    use std::io::Write;
    if std::fs::symlink_metadata(file).is_ok_and(|m| !m.is_dir()) {
        let _ = std::fs::remove_file(file);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(file)
    {
        let _ = f.write_all(contents);
    }
}

fn parse_shortstat(s: &str) -> (u32, u32) {
    let num_before = |word: &str| {
        s.split(',')
            .find(|p| p.contains(word))
            .and_then(|p| p.split_whitespace().next())
            .and_then(|n| n.parse().ok())
    };
    (
        num_before("insertion").unwrap_or(0),
        num_before("deletion").unwrap_or(0),
    )
}

pub fn status(worktree: &Path) -> Result<GitStatus, GitError> {
    with_links(worktree, || status_unchecked(worktree))
}

fn status_unchecked(worktree: &Path) -> Result<GitStatus, GitError> {
    // A git process costs about a second on a busy Windows machine. The filter question is asked once for both commands that read
    // files, right before the first; the commit comes from the same `status` as the changed paths (`# branch.oid`) instead of a
    // `rev-parse` of its own.
    // Both commands take the pathspec `.`, like [`diff`]: without one git reports the whole checkout even when run in a folder
    // inside it (a workspace selected inside a repository, ADR-0053), and the count would include files the diff never lists. At a
    // checkout's root `.` is everything, so a worktree the node made is counted as before.
    // Every untracked file is one entry, as [`diff`] lists each: by default git shows a new folder as one `? dir/` line, and the
    // repository's own `status.showUntrackedFiles=no` would hide them all.
    let filters = Filters::of(worktree);
    let git = |args: &[&str]| run(hardened_with(worktree, &filters), args);
    let (files_changed, head_commit) = parse_status_v2(&git(&[
        "status",
        "--porcelain=v2",
        "--branch",
        "--untracked-files=all",
        SUBMODULE_COMMITS_ONLY,
        "--",
        ".",
    ])?);
    // #941: on a branch with no commit yet there is no HEAD to count against; every tracked file is new (git's empty tree).
    let base = if head_commit.is_empty() {
        empty_tree(worktree)
    } else {
        "HEAD"
    };
    let (insertions, deletions) = parse_shortstat(&git(&[
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        SUBMODULE_COMMITS_ONLY,
        "--shortstat",
        base,
        "--",
        ".",
    ])?);
    Ok(GitStatus {
        files_changed,
        insertions,
        deletions,
        head_commit,
    })
}

/// Changed paths (every entry line; renames are one line, paths are quoted) and the commit (`# branch.oid`, empty on a branch with no
/// commit yet) from `git status --porcelain=v2 --branch`.
fn parse_status_v2(out: &str) -> (u32, String) {
    let mut changed = 0;
    let mut head = String::new();
    for line in out.lines() {
        if let Some(oid) = line.strip_prefix("# branch.oid ") {
            if oid != "(initial)" {
                head = oid.to_string();
            }
        } else if !line.starts_with('#') && !line.is_empty() {
            changed += 1;
        }
    }
    (changed, head)
}

/// git's empty tree, what a branch with no commit yet is compared with (#941): every file is new. Its id depends on the repository's
/// hash; a git too old to answer `--show-object-format` knows only SHA-1.
fn empty_tree(dir: &Path) -> &'static str {
    match git_refs(dir, &["rev-parse", "--show-object-format"]) {
        Ok(format) if format.trim() == "sha256" => {
            "6ef19b41225c5369f1c104d45d8d85efa9b057b53b14b4b9b939dd74decc5321"
        }
        _ => "4b825dc642cb6eb9a060e54bf8d69288fbee4904",
    }
}

/// Whether HEAD names no commit: a repository (or a branch) with no commit yet.
fn no_commit_yet(dir: &Path) -> bool {
    git_refs(dir, &["rev-parse", "--verify", "-q", "HEAD"]).is_err()
}

/// Most untracked files a diff shows, and the most bytes of one it reads.
const MAX_UNTRACKED: usize = 50;
const MAX_UNTRACKED_BYTES: u64 = 64 * 1024;

/// The most diff output ever held in memory; what is beyond it is dropped as it is read.
const MAX_DIFF_READ: u64 = 1024 * 1024;

/// Like [`git`], keeping at most `cap` bytes of stdout (the process is stopped when it has produced that much).
fn git_capped(dir: &Path, args: &[&str], cap: u64) -> Result<String, GitError> {
    use std::io::Read;
    let mut command = hardened(dir);
    let mut child = command
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| GitError::Io(e.to_string()))?;
    let mut buf = Vec::new();
    if let Some(out) = child.stdout.take() {
        let _ = out.take(cap).read_to_end(&mut buf);
    }
    // Past the cap git may still be writing; closing its pipe and killing it is how it stops.
    let _ = child.kill();
    let status = child.wait().map_err(|e| GitError::Io(e.to_string()))?;
    if let Some(moved) = moved_links(&command) {
        return Err(moved);
    }
    if buf.len() as u64 >= cap || status.success() {
        Ok(String::from_utf8_lossy(&buf).into_owned())
    } else {
        Err(GitError::Command {
            args: args.join(" "),
            stderr: String::new(),
        })
    }
}

/// A changed secret file (ADR-0080): named, counted, never shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretChange {
    pub path: String,
    pub added: u32,
    pub removed: u32,
}

/// [`diff_with`] hiding secret files (the default; ADR-0080), without the list of them.
pub fn diff(worktree: &Path, path: Option<&str>) -> Result<String, GitError> {
    diff_with(worktree, path, false).map(|(text, _)| text)
}

/// Unified diff of the changes against HEAD, optionally for one path. A new file that git does not track yet is shown as
/// all-added (what a reviewer would see), without touching the index. With no commit yet (#941) every file is new: the diff is
/// against git's empty tree.
///
/// ADR-0080: unless `show_secret` (the owner's policy), a file whose name is secret appears as its `diff --git` header (and its
/// mode and rename lines, which hold names only) without hunks or blob ids; its line counts are returned beside the text.
pub fn diff_with(
    worktree: &Path,
    path: Option<&str>,
    show_secret: bool,
) -> Result<(String, Vec<SecretChange>), GitError> {
    with_links(worktree, || diff_unchecked(worktree, path, show_secret))
}

fn diff_unchecked(
    worktree: &Path,
    path: Option<&str>,
    show_secret: bool,
) -> Result<(String, Vec<SecretChange>), GitError> {
    // Always a pathspec: without one git diffs the whole checkout even when run in a folder inside it (a selected workspace),
    // while `ls-files` below lists only that folder. A named path is a name, not a pattern: a file name may hold `*`, `?` or `[`,
    // which git would otherwise match against other files too (`a[1].txt` would show `a1.txt`).
    let literal = path.map(|p| format!(":(literal){p}"));
    let tracked_against = |base: &str| {
        git_capped(
            worktree,
            &[
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                SUBMODULE_COMMITS_ONLY,
                // Names from the folder the diff runs in, like `ls-files` below and like the paths `git.diff` and `files.read`
                // take: without it a workspace that is a folder inside a repository (ADR-0053) gets tracked files named from the
                // repository's root (`web/a.txt`) and untracked ones from the folder (`b.txt`). At a checkout's root it changes
                // nothing.
                "--relative",
                base,
                "--",
                literal.as_deref().unwrap_or("."),
            ],
            MAX_DIFF_READ,
        )
    };
    // HEAD is asked about only when the diff against it fails: a commit is the usual case, and each git costs time on Windows.
    let tracked = match tracked_against("HEAD") {
        Ok(text) => text,
        Err(_) if no_commit_yet(worktree) => tracked_against(empty_tree(worktree))?,
        Err(e) => return Err(e),
    };
    let (mut out, mut secret) = if show_secret {
        (tracked, Vec::new())
    } else {
        hide_secret_sections(&tracked)
    };
    // #940: a UTF-16 text file git called binary, as text. After the secret files were cut: no hidden one is decoded.
    out = utf16::decode_sections(worktree, std::mem::take(&mut out), show_secret);
    let mut ls = vec!["ls-files", "--others", "--exclude-standard", "-z"];
    if let Some(p) = &literal {
        ls.extend(["--", p]);
    }
    let listed = git(worktree, &ls)?;
    let names: Vec<&str> = listed
        .split('\0')
        .filter(|n| !n.is_empty())
        .take(MAX_UNTRACKED)
        .collect();
    if !names.is_empty() {
        out.push_str(&untracked_diff(worktree, &names, show_secret, &mut secret)?);
    }
    Ok((out, secret))
}

/// Lines starting with these name a change without showing any content (a mode, a rename).
const HEADER_KEEP: &[&str] = &[
    "old mode ",
    "new mode ",
    "new file mode ",
    "deleted file mode ",
    "rename from ",
    "rename to ",
    "copy from ",
    "copy to ",
];

/// Git's C-style quoting undone (`"a/\303\251"` as the path it names); an unquoted name is returned as is.
fn unquote(name: &str) -> String {
    let Some(inner) = name.strip_prefix('"').and_then(|n| n.strip_suffix('"')) else {
        return name.to_string();
    };
    let mut out = Vec::new();
    let b = inner.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' || i + 1 >= b.len() {
            out.push(b[i]);
            i += 1;
            continue;
        }
        i += 1;
        let c = b[i];
        if c.is_ascii_digit() {
            let end = (i + 3).min(b.len());
            let v = std::str::from_utf8(&b[i..end])
                .ok()
                .and_then(|o| u8::from_str_radix(o, 8).ok());
            out.push(v.unwrap_or(c));
            i = end;
        } else {
            out.push(match c {
                b'a' => 0x07,
                b'b' => 0x08,
                b't' => b'\t',
                b'n' => b'\n',
                b'v' => 0x0b,
                b'f' => 0x0c,
                b'r' => b'\r',
                other => other,
            });
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The paths a `diff --git` section names: from its extra header lines (rename, copy, `---`, `+++`), and, since a section without
/// hunks has no others, every way its first line can be split into an `a/` and a `b/` name. Every one of them is checked
/// (a file counts as secret when any is); the last element's first value is the name shown for the file (its new name).
fn section_paths(section: &[&str]) -> (Vec<String>, String) {
    let strip = |p: &str| -> String {
        let p = unquote(p.trim_end_matches(['\n', '\r']).trim_end_matches('\t'));
        p.strip_prefix("a/")
            .or_else(|| p.strip_prefix("b/"))
            .unwrap_or(&p)
            .to_string()
    };
    let mut all = Vec::new();
    let mut shown = None;
    for line in &section[1..] {
        if let Some(r) = line
            .strip_prefix("rename to ")
            .or_else(|| line.strip_prefix("copy to "))
        {
            let p = unquote(r.trim_end());
            shown.get_or_insert(p.clone());
            all.push(p);
        } else if let Some(r) = line
            .strip_prefix("rename from ")
            .or_else(|| line.strip_prefix("copy from "))
        {
            all.push(unquote(r.trim_end()));
        } else if let Some(r) = line.strip_prefix("+++ ") {
            if r.trim_end() != "/dev/null" {
                let p = strip(r);
                shown.get_or_insert(p.clone());
                all.push(p);
            }
        } else if let Some(r) = line.strip_prefix("--- ") {
            if r.trim_end() != "/dev/null" {
                all.push(strip(r));
            }
        }
        if line.starts_with("@@") {
            break;
        }
    }
    let rest = section[0]
        .trim_end()
        .strip_prefix("diff --git ")
        .unwrap_or("");
    let mut guess = None;
    for (i, _) in rest.match_indices(' ') {
        let (a, b) = (&rest[..i], rest[i + 1..].trim_start());
        if !(b.starts_with("b/") || b.starts_with("\"b/")) {
            continue;
        }
        let (pa, pb) = (strip(a), strip(b));
        if pa == pb {
            guess.get_or_insert(pb.clone());
        }
        all.push(pa);
        all.push(pb);
    }
    let shown = shown
        .or(guess)
        .unwrap_or_else(|| all.last().cloned().unwrap_or_default());
    (all, shown)
}

/// ADR-0080: `diff` with each secret file's section cut to its header lines that hold names only; the line counts of what was
/// cut (hunk lines starting with `+` and `-`) come back beside it.
fn hide_secret_sections(diff: &str) -> (String, Vec<SecretChange>) {
    let lines: Vec<&str> = diff.split_inclusive('\n').collect();
    let mut out = String::with_capacity(diff.len());
    let mut secret = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if !lines[i].starts_with("diff --git ") {
            out.push_str(lines[i]);
            i += 1;
            continue;
        }
        let mut end = i + 1;
        while end < lines.len() && !lines[end].starts_with("diff --git ") {
            end += 1;
        }
        let section = &lines[i..end];
        i = end;
        let (paths, shown) = section_paths(section);
        if !paths.iter().any(|p| agora_core::secrets::is_secret_path(p)) {
            section.iter().for_each(|l| out.push_str(l));
            continue;
        }
        let (mut added, mut removed, mut in_hunk) = (0u32, 0u32, false);
        for line in section {
            if line.starts_with("@@") {
                in_hunk = true;
            } else if in_hunk {
                match line.as_bytes().first() {
                    Some(b'+') => added = added.saturating_add(1),
                    Some(b'-') => removed = removed.saturating_add(1),
                    _ => {}
                }
            }
        }
        out.push_str(section[0]);
        for line in &section[1..] {
            if HEADER_KEEP.iter().any(|k| line.starts_with(k)) {
                out.push_str(line);
            }
        }
        secret.push(SecretChange {
            path: shown,
            added,
            removed,
        });
    }
    (out, secret)
}

/// The diff of each untracked file against nothing, built here rather than by `git diff --no-index`. git given a path in the
/// worktree opens it itself, and the agent working there can swap a folder above it for a link between `ls-files` and that open
/// (measured: git follows a linked folder, though not a link in the file's own place), so the node would show a file from
/// anywhere; a copy on disk to diff instead is one more place to race (a shared temporary folder is the agent's to write on
/// Linux). Each file is read the way `files.read` reads it ([`crate::files::Files::untracked`]: checked on the open handle,
/// regular files only, at most [`MAX_UNTRACKED_BYTES`]; a link shows as a link), and the text is git's own for a new file
/// ([`new_file_diff`]). Files that are not plain, too large, or lead out of the worktree are left out, as before.
fn untracked_diff(
    worktree: &Path,
    names: &[&str],
    show_secret: bool,
    secret: &mut Vec<SecretChange>,
) -> Result<String, GitError> {
    // Read with secrets allowed so that a secret file can be counted; what it holds never reaches the text below (ADR-0080).
    let files = crate::files::Files::open(worktree, MAX_UNTRACKED_BYTES)
        .map_err(|e| GitError::Io(format!("{e:?}")))?
        .with_secret_shown(true);
    let mut out = Vec::new();
    for name in names {
        let (mode, data) = match files.untracked(name) {
            Ok(crate::files::Untracked::File { data, executable }) => {
                (if executable { "100755" } else { "100644" }, data)
            }
            Ok(crate::files::Untracked::Link(target)) => ("120000", target),
            Err(_) => continue,
        };
        // A file is secret by its own name or by where its folder leads (a folder linked to `.ssh`).
        if !show_secret && files.resolves_secret(name) {
            let lines =
                u32::try_from(data.split_inclusive(|c| *c == b'\n').count()).unwrap_or(u32::MAX);
            let binary = data[..data.len().min(8000)].contains(&0);
            secret.push(SecretChange {
                path: (*name).to_string(),
                added: if binary || mode == "120000" { 0 } else { lines },
                removed: 0,
            });
            let q = |p: &str| quote_path(&format!("{p}/{name}"));
            out.extend(
                format!("diff --git {} {}\nnew file mode {mode}\n", q("a"), q("b")).into_bytes(),
            );
            continue;
        }
        out.extend(new_file_diff(name, mode, &data));
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// What `git diff --no-index /dev/null <name>` prints for a new file of `mode` holding `data` (checked against git in
/// `new_file_diffs_match_git`): the blob id abbreviated to 7, `Binary files ... differ` when a NUL is in the first 8000 bytes,
/// names quoted as git quotes them by default, and a tab after a `+++` name that holds a space. The bytes are shown as they are on
/// disk: no `core.autocrlf`/`eol` conversion (git for Windows, `autocrlf=true` by default, would show CRLF lines without the CR).
fn new_file_diff(name: &str, mode: &str, data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut blob = format!("blob {}\0", data.len()).into_bytes();
    blob.extend_from_slice(data);
    let id = ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, &blob);
    let abbrev: String = id.as_ref()[..4]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()[..7]
        .to_string();
    let (a, b) = (
        quote_path(&format!("a/{name}")),
        quote_path(&format!("b/{name}")),
    );
    let mut out = Vec::new();
    // Writing to a Vec cannot fail.
    let _ = write!(
        out,
        "diff --git {a} {b}\nnew file mode {mode}\nindex 0000000..{abbrev}\n"
    );
    if data.is_empty() {
        return out;
    }
    // #940: UTF-16 text with a byte-order mark (Windows PowerShell's `>`) is shown as its text, with or without a zero byte.
    let decoded;
    let data = if let Some(text) = agora_core::text::utf16_text(data) {
        decoded = text;
        if decoded.is_empty() {
            return out;
        }
        decoded.as_bytes()
    } else if data[..data.len().min(8000)].contains(&0) {
        let _ = writeln!(out, "Binary files /dev/null and {b} differ");
        return out;
    } else {
        data
    };
    let tab = if b.contains(' ') { "\t" } else { "" };
    let lines: Vec<&[u8]> = data.split_inclusive(|c| *c == b'\n').collect();
    let range = if lines.len() == 1 {
        "1".to_string()
    } else {
        format!("1,{}", lines.len())
    };
    let _ = write!(out, "--- /dev/null\n+++ {b}{tab}\n@@ -0,0 +{range} @@\n");
    for line in &lines {
        out.push(b'+');
        out.extend_from_slice(line);
    }
    if !data.ends_with(b"\n") {
        out.extend_from_slice(b"\n\\ No newline at end of file\n");
    }
    out
}

/// A path as git prints it with `core.quotePath` at its default: in double quotes with C escapes (octal for other control bytes,
/// DEL and every byte of a non-ASCII character) when it holds any of those, a double quote or a backslash; as is otherwise.
fn quote_path(path: &str) -> String {
    let needs = |c: u8| c < 0x20 || c == b'"' || c == b'\\' || c >= 0x7f;
    if !path.bytes().any(needs) {
        return path.to_string();
    }
    let mut out = String::from("\"");
    for c in path.bytes() {
        match c {
            0x07 => out.push_str("\\a"),
            0x08 => out.push_str("\\b"),
            b'\t' => out.push_str("\\t"),
            b'\n' => out.push_str("\\n"),
            0x0b => out.push_str("\\v"),
            0x0c => out.push_str("\\f"),
            b'\r' => out.push_str("\\r"),
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            c if needs(c) => out.push_str(&format!("\\{c:03o}")),
            c => out.push(char::from(c)),
        }
    }
    out.push('"');
    out
}

/// One commit of [`log`].
pub struct Commit {
    pub id: String,
    pub subject: String,
    pub author: String,
    pub at_ms: u64,
}

/// The newest `limit` commits reachable from HEAD. A workspace with no commit yet has an empty log.
pub fn log(worktree: &Path, limit: usize) -> Result<Vec<Commit>, GitError> {
    with_links(worktree, || log_unchecked(worktree, limit))
}

fn log_unchecked(worktree: &Path, limit: usize) -> Result<Vec<Commit>, GitError> {
    if git_refs(worktree, &["rev-parse", "--verify", "-q", "HEAD"]).is_err() {
        return Ok(Vec::new());
    }
    let n = format!("-n{limit}");
    // Git terminates each commit with NUL under -z. Use NUL between its four fields too:
    // a commit's author and subject can contain other ASCII control characters.
    let out = git_refs(
        worktree,
        &[
            "log",
            &n,
            "--no-color",
            "-z",
            "--format=%H%x00%an%x00%ct%x00%s",
        ],
    )?;
    let mut fields = out.split_terminator('\0');
    let records = std::iter::from_fn(|| {
        Some((
            fields.next()?,
            fields.next()?,
            fields.next()?,
            fields.next()?,
        ))
    });
    Ok(records
        .filter_map(|(id, author, ct, subject)| {
            Some(Commit {
                id: id.to_string(),
                author: author.to_string(),
                at_ms: ct.parse::<u64>().ok()?.saturating_mul(1000),
                subject: subject.to_string(),
            })
        })
        .collect())
}

/// Adds `rel` to the worktree's private ignore list (`info/exclude` of its git dir), so a file the node wrote stays
/// out of `git status` and `git add .` without touching a tracked `.gitignore`. Best effort: failure only means the
/// file shows up as untracked.
pub fn exclude_locally(worktree: &Path, rel: &Path) {
    let Ok(p) = git_refs(worktree, &["rev-parse", "--git-path", "info/exclude"]) else {
        return;
    };
    let p = p.trim().to_string();
    let exclude = if Path::new(&p).is_absolute() {
        PathBuf::from(p)
    } else {
        worktree.join(p)
    };
    if let Some(dir) = exclude.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let line = format!("/{}", rel.to_string_lossy().replace('\\', "/"));
    let existing = std::fs::read_to_string(&exclude).unwrap_or_default();
    if !existing.lines().any(|l| l == line) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&exclude)
        {
            let _ = writeln!(f, "{line}");
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)] // expect is allowed in tests
mod tests {
    #[test]
    fn secret_sections_are_cut_to_their_names_and_counted() {
        let diff = "diff --git a/.env b/.env\nindex 1111111..2222222 100644\n--- a/.env\n+++ b/.env\n@@ -1,2 +1,3 @@\n A=1\n-B=2\n+B=3\n+C=4\n\
diff --git a/x.txt b/x.txt\nindex 1..2 100644\n--- a/x.txt\n+++ b/x.txt\n@@ -1 +1 @@\n-one\n+two\n\
diff --git a/old.txt b/keys/server.pem\nsimilarity index 80%\nrename from old.txt\nrename to keys/server.pem\nindex 1..2 100644\n--- a/old.txt\n+++ b/keys/server.pem\n@@ -1 +1 @@\n-aaa\n+bbb\n\
diff --git \"a/\\303\\251.pem\" \"b/\\303\\251.pem\"\nindex 1..2 100644\n--- \"a/\\303\\251.pem\"\n+++ \"b/\\303\\251.pem\"\n@@ -1 +1 @@\n-q\n+r\n\
diff --git a/.ssh/known b/.ssh/known\nold mode 100644\nnew mode 100755\n";
        let (out, secret) = hide_secret_sections(diff);
        for leaked in [
            "A=1",
            "B=2",
            "B=3",
            "C=4",
            "aaa",
            "bbb",
            "index 1111111",
            "@@ -1,2",
        ] {
            assert!(!out.contains(leaked), "{leaked}\n{out}");
        }
        // The ordinary file is untouched, byte for byte.
        assert!(out.contains("diff --git a/x.txt b/x.txt\nindex 1..2 100644\n--- a/x.txt\n+++ b/x.txt\n@@ -1 +1 @@\n-one\n+two\n"));
        assert!(out.contains("rename from old.txt\nrename to keys/server.pem\n"));
        assert!(out.contains("old mode 100644\nnew mode 100755\n"));
        let got: Vec<(&str, u32, u32)> = secret
            .iter()
            .map(|c| (c.path.as_str(), c.added, c.removed))
            .collect();
        assert_eq!(
            got,
            vec![
                (".env", 2, 1),
                ("keys/server.pem", 1, 1),
                ("\u{e9}.pem", 1, 1),
                (".ssh/known", 0, 0),
            ]
        );
    }

    #[test]
    fn a_diff_without_secret_files_is_returned_unchanged() {
        let diff = "diff --git a/b.txt b/b.txt\nindex 1..2 100644\n--- a/b.txt\n+++ b/b.txt\n@@ -1 +1 @@\n-x\n+y\n";
        assert_eq!(hide_secret_sections(diff), (diff.to_string(), Vec::new()));
    }

    use std::fs;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    /// Plain git for the controls, with none of the repository-location variables a calling git (a hook) may have set, which would
    /// make it ignore the `.git` a test planted.
    fn raw_git() -> Command {
        let mut c = Command::new("git");
        for v in [
            "GIT_DIR",
            "GIT_COMMON_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_CONFIG_COUNT",
        ] {
            c.env_remove(v);
        }
        c
    }

    /// Windows canonicalizes to `\\?\C:\...`, which git cannot use; drop the verbatim prefix.
    fn plain(p: PathBuf) -> PathBuf {
        match p.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
            Some(rest) => PathBuf::from(rest),
            None => p,
        }
    }

    /// A temporary repository with one commit on `main`, removed on drop.
    struct Repo {
        dir: PathBuf,
    }

    impl Repo {
        fn new(remote: Option<&str>) -> Self {
            let r = Self::unborn();
            if let Some(url) = remote {
                r.git(&["remote", "add", "origin", url]);
            }
            fs::write(r.dir.join("a.txt"), "one\n").expect("write");
            r.commit("first");
            r
        }

        /// A temporary repository on `main` with no commit yet.
        fn unborn() -> Self {
            static N: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "agora-git-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).expect("mkdir");
            // Canonicalize: macOS temp dirs are symlinks and git reports real paths.
            let dir = plain(dir.canonicalize().expect("canonicalize"));
            let r = Self { dir };
            r.git(&["init", "-q", "-b", "main"]);
            r
        }

        fn git(&self, args: &[&str]) -> String {
            git(
                &self.dir,
                &[
                    &[
                        "-c",
                        "user.name=t",
                        "-c",
                        "user.email=t@example.com",
                        "-c",
                        "commit.gpgsign=false",
                    ],
                    args,
                ]
                .concat(),
            )
            .unwrap_or_else(|e| panic!("git {args:?}: {e:?}"))
        }

        fn commit(&self, msg: &str) {
            self.git(&["add", "-A"]);
            self.git(&["commit", "-q", "-m", msg]);
        }

        fn wt(&self, name: &str) -> PathBuf {
            self.dir.with_file_name(format!(
                "{}-{name}",
                self.dir.file_name().expect("name").to_string_lossy()
            ))
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
            for suffix in ["w1", "w2"] {
                let _ = fs::remove_dir_all(self.wt(suffix));
            }
        }
    }

    /// #941 (RM-35): a repository with no commit yet is a normal state, not a failure. Its files are all new: what is staged and
    /// what git does not track yet both show as added (secret files by name only), the counts are what `git status` shows, and
    /// there is no commit.
    #[test]
    fn a_repository_with_no_commit_yet_shows_its_files_as_new() {
        let r = Repo::unborn();
        fs::write(r.dir.join("b.txt"), "hello\n").expect("write");
        fs::write(r.dir.join("s.txt"), "staged\nfile\n").expect("write");
        fs::write(r.dir.join(".env"), "KEY=fake\n").expect("write");
        r.git(&["add", "s.txt"]);

        let (d, secret) = diff_with(&r.dir, None, false).expect("diff");
        assert!(
            d.contains("diff --git a/b.txt b/b.txt\nnew file mode 100644\n"),
            "{d}"
        );
        assert!(d.contains("+hello\n"), "{d}");
        assert!(
            d.contains("diff --git a/s.txt b/s.txt\nnew file mode 100644\n"),
            "{d}"
        );
        assert!(d.contains("+staged\n+file\n"), "{d}");
        assert!(!d.contains("KEY=fake"), "{d}");
        let got: Vec<(&str, u32, u32)> = secret
            .iter()
            .map(|c| (c.path.as_str(), c.added, c.removed))
            .collect();
        assert_eq!(got, vec![(".env", 1, 0)]);
        // One path, as `git.diff` with a path asks.
        let one = diff(&r.dir, Some("s.txt")).expect("diff one");
        assert!(one.contains("+staged\n") && !one.contains("b.txt"), "{one}");

        let st = status(&r.dir).expect("status");
        assert_eq!(st.files_changed, 3);
        // Line counts are of tracked changes: the staged file's two lines.
        assert_eq!((st.insertions, st.deletions), (2, 0));
        assert_eq!(st.head_commit, "");
        assert!(log(&r.dir, 5).expect("log").is_empty());
    }

    /// #941: the empty tree is the repository's own; a SHA-256 repository has another id for it.
    #[test]
    fn a_sha256_repository_with_no_commit_yet_shows_its_files_as_new() {
        let r = Repo::unborn();
        fs::remove_dir_all(r.dir.join(".git")).expect("rm .git");
        if git(&r.dir, &["init", "-q", "--object-format=sha256"]).is_err() {
            return; // a git without SHA-256 repositories
        }
        fs::write(r.dir.join("s.txt"), "staged\n").expect("write");
        r.git(&["add", "s.txt"]);
        assert!(diff(&r.dir, None).expect("diff").contains("+staged\n"));
        let st = status(&r.dir).expect("status");
        assert_eq!((st.files_changed, st.insertions), (1, 1));
    }

    #[test]
    fn discovers_checkout_and_project_identity() {
        let r = Repo::new(Some("git@github.com:Owner/Repo.git"));
        let sub = r.dir.join("sub");
        fs::create_dir_all(&sub).expect("mkdir");
        let info = discover_checkout(&sub).expect("discover");
        assert_eq!(info.root, r.dir);
        // Spelled as this system spells it, not only equal as a path: the web compares the text, and on Windows git answers
        // `C:/...` where every other path the node reports is `C:\...`, so one folder showed up twice.
        assert_eq!(info.root.as_os_str(), r.dir.as_os_str());
        assert_eq!(info.project.remote(), Some("github.com/owner/repo"));
        assert_eq!(info.project.root_commit().len(), 40);
    }

    /// A folder whose `.git` links to another repository's `.git` is refused, also when asked from a subfolder: git itself
    /// finds the link from there, so the identity queries must run at the resolved root, where the link is seen.
    #[cfg(unix)]
    #[test]
    fn a_linked_dot_git_is_refused_from_a_subfolder_too() {
        let other = Repo::new(Some("git@github.com:Other/Repo.git"));
        let linked = other.wt("linked");
        fs::create_dir_all(linked.join("sub")).expect("mkdir");
        agora_platform::symlink(&other.dir.join(".git"), &linked.join(".git")).expect("symlink");
        let from_sub = discover_checkout(&linked.join("sub"));
        let from_root = discover_checkout(&linked);
        let _ = fs::remove_dir_all(&linked);
        assert!(from_sub.is_err(), "linked/sub discovered: {from_sub:?}");
        assert!(from_root.is_err(), "linked discovered: {from_root:?}");
    }

    #[test]
    fn repository_without_a_remote_uses_the_root_commit_only() {
        let r = Repo::new(None);
        let info = discover_checkout(&r.dir).expect("discover");
        assert_eq!(info.project.remote(), None);
    }

    #[test]
    fn not_a_repository_is_an_error() {
        let dir = std::env::temp_dir().join(format!("agora-not-git-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("mkdir");
        assert!(matches!(
            discover_checkout(&dir),
            Err(GitError::NotARepository(_))
        ));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn creates_a_worktree_on_a_new_branch_and_rejects_an_existing_one() {
        let r = Repo::new(None);
        create_worktree(&r.dir, &r.wt("w1"), "agent/one", None).expect("create");
        assert_eq!(
            current_branch(&r.wt("w1")).expect("branch").as_deref(),
            Some("agent/one")
        );
        let again = create_worktree(&r.dir, &r.wt("w2"), "agent/one", None);
        assert_eq!(again, Err(GitError::BranchExists("agent/one".into())));
        assert_eq!(
            again.err().map(|e| e.to_protocol().code),
            Some(ErrorCode::WorktreeExists)
        );
    }

    #[test]
    fn rejects_invalid_branch_names_and_option_like_bases() {
        let r = Repo::new(None);
        // Each is a name git refuses: it must come back as InvalidBranch (protocol.invalid_message, field=branch),
        // never as the Internal error a failed git command maps to.
        for b in [
            "",
            "-x",
            "a..b",
            "a b",
            "agent/.hidden",
            "a.lock/b",
            "a/b.lock",
            "a/",
            "a.",
            "a@{1}",
            "a:b",
            "HEAD",
        ] {
            let got = create_worktree(&r.dir, &r.wt("w1"), b, None);
            assert!(matches!(got, Err(GitError::InvalidBranch(_))), "{b:?}");
            let err = got.err().map(|e| e.to_protocol());
            assert_eq!(
                err.as_ref().map(|e| &e.code),
                Some(&ErrorCode::ProtocolInvalidMessage),
                "{b:?}"
            );
            assert_eq!(
                err.as_ref().and_then(|e| e.params.get("field")),
                Some(&agora_protocol::ParamValue::Str("branch".into())),
                "{b:?}"
            );
            assert!(!r.wt("w1").exists(), "{b:?}");
        }
        assert!(matches!(
            create_worktree(&r.dir, &r.wt("w1"), "ok", Some("--force")),
            Err(GitError::InvalidBranch(_))
        ));
    }

    #[test]
    fn removing_a_clean_worktree_with_no_extra_commits_works_and_keeps_the_branch() {
        let r = Repo::new(None);
        create_worktree(&r.dir, &r.wt("w1"), "agent/one", None).expect("create");
        remove_worktree(&r.dir, &r.wt("w1"), false).expect("remove");
        assert!(!r.wt("w1").exists());
        assert!(
            branch_exists(&r.dir, "agent/one").expect("exists"),
            "work is never deleted with the worktree"
        );
    }

    #[test]
    fn dirty_worktree_is_refused_unless_forced() {
        let r = Repo::new(None);
        create_worktree(&r.dir, &r.wt("w1"), "agent/one", None).expect("create");
        fs::write(r.wt("w1").join("new.txt"), "x").expect("write");
        let err = remove_worktree(&r.dir, &r.wt("w1"), false);
        assert_eq!(err, Err(GitError::Dirty(r.wt("w1"))));
        assert_eq!(
            err.err().map(|e| e.to_protocol().code),
            Some(ErrorCode::WorktreeDirty)
        );
        assert!(r.wt("w1").exists());
        remove_worktree(&r.dir, &r.wt("w1"), true).expect("forced");
        assert!(!r.wt("w1").exists());
    }

    #[test]
    fn unmerged_commits_block_removal_until_merged_or_forced() {
        let r = Repo::new(None);
        create_worktree(&r.dir, &r.wt("w1"), "agent/one", None).expect("create");
        let w = r.wt("w1");
        fs::write(w.join("work.txt"), "work").expect("write");
        git(&w, &["add", "-A"]).expect("add");
        git(
            &w,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "commit",
                "-q",
                "-m",
                "work",
            ],
        )
        .expect("commit");
        assert_eq!(
            remove_worktree(&r.dir, &w, false),
            Err(GitError::Unmerged("agent/one".into()))
        );
        // Merge it into main: now the commits exist on another branch.
        r.git(&["merge", "-q", "--ff-only", "agent/one"]);
        remove_worktree(&r.dir, &w, false).expect("removable after merge");
    }

    #[test]
    fn status_and_diff_report_changes() {
        let r = Repo::new(None);
        let clean = status(&r.dir).expect("status");
        assert_eq!(
            (clean.files_changed, clean.insertions, clean.deletions),
            (0, 0, 0)
        );
        assert_eq!(clean.head_commit, r.git(&["rev-parse", "HEAD"]).trim());
        fs::write(r.dir.join("a.txt"), "one\ntwo\nthree\n").expect("write");
        fs::write(r.dir.join("untracked.txt"), "u").expect("write");
        let s = status(&r.dir).expect("status");
        assert_eq!((s.files_changed, s.insertions, s.deletions), (2, 2, 0));
        let d = diff(&r.dir, None).expect("diff");
        assert!(d.contains("+two") && d.contains("+three"));
        assert!(diff(&r.dir, Some("nothing-here.txt"))
            .expect("diff")
            .is_empty());
    }

    #[test]
    fn a_workspace_inside_a_checkout_diffs_only_its_own_folder() {
        let r = Repo::new(None);
        fs::create_dir_all(r.dir.join("web")).expect("dir");
        fs::write(r.dir.join("web/w.txt"), "web\n").expect("write");
        r.commit("web");
        fs::write(r.dir.join("a.txt"), "one\nOUTSIDE\n").expect("write");
        fs::write(r.dir.join("web/w.txt"), "web\nINSIDE\n").expect("write");
        let d = diff(&r.dir.join("web"), None).expect("diff");
        assert!(d.contains("+INSIDE") && !d.contains("OUTSIDE"), "{d}");
        // At the checkout's root everything still shows.
        assert!(diff(&r.dir, None).expect("diff").contains("+OUTSIDE"));
    }

    /// The files one diff names: the new name of each `diff --git` entry (`rename to` for a rename, the second half of `a/P b/P`
    /// otherwise, split in the middle so a folder named `x b` is not cut at its own " b/", as the web's `parseDiff` does), and
    /// whether it was deleted.
    fn diff_paths(d: &str) -> Vec<(String, bool)> {
        let mut out: Vec<(String, bool)> = Vec::new();
        for line in d.lines() {
            if let Some(rest) = line.strip_prefix("diff --git ") {
                let half = (rest.len() - 5) / 2;
                let name = if rest.get(2 + half..5 + half) == Some(" b/") {
                    &rest[5 + half..]
                } else {
                    // Two different names: a rename, whose `rename to` follows.
                    &rest[rest.rfind(" b/").expect("b/") + 3..]
                };
                out.push((name.to_string(), false));
            } else if let Some(to) = line.strip_prefix("rename to ") {
                out.last_mut().expect("entry").0 = to.to_string();
            } else if line.starts_with("deleted file mode") {
                out.last_mut().expect("entry").1 = true;
            }
        }
        out
    }

    /// A repository whose `web/` folder is the workspace, with changes inside it, outside it and across its edge.
    fn folder_workspace() -> Repo {
        let r = Repo::new(None);
        for (name, body) in [
            ("web/w.txt", &b"web\n"[..]),
            ("web/old.txt", b"old one\nstays\n"),
            ("web/m.txt", b"move me\n"),
            ("web/img.bin", b"\0\x01\x02"),
            ("web/x b/f.txt", b"f\n"),
            ("in.txt", b"in\n"),
        ] {
            fs::create_dir_all(r.dir.join(name).parent().expect("parent")).expect("dir");
            fs::write(r.dir.join(name), body).expect("write");
        }
        r.commit("layout");
        fs::write(r.dir.join("a.txt"), "one\nOUTSIDE\n").expect("write");
        fs::write(r.dir.join("web/w.txt"), "web\nINSIDE\n").expect("write");
        fs::write(r.dir.join("web/new.txt"), "new\n").expect("write");
        fs::write(r.dir.join("web/new.bin"), b"\0\x05").expect("write");
        fs::write(r.dir.join("web/img.bin"), b"\0\x09\x09").expect("write");
        fs::write(r.dir.join("web/x b/f.txt"), "f\ng\n").expect("write");
        fs::create_dir_all(r.dir.join("web/in b")).expect("dir");
        r.git(&["mv", "web/old.txt", "web/renamed.txt"]);
        r.git(&["mv", "in.txt", "web/in b/in.txt"]);
        r.git(&["mv", "web/m.txt", "moved.txt"]);
        r
    }

    #[test]
    fn a_folder_workspace_names_every_file_from_the_folder() {
        let r = folder_workspace();
        let web = r.dir.join("web");
        let d = diff(&web, None).expect("diff");
        let mut paths = diff_paths(&d);
        paths.sort();
        let want = |p: &str, deleted| (p.to_string(), deleted);
        assert_eq!(
            paths,
            vec![
                want("img.bin", false),
                want("in b/in.txt", false), // renamed in across the edge: a new file here
                want("m.txt", true),        // renamed out across the edge: gone from here
                want("new.bin", false),
                want("new.txt", false),
                want("renamed.txt", false), // a rename inside the folder
                want("w.txt", false),
                want("x b/f.txt", false),
            ],
            "{d}"
        );
        assert!(d.contains("rename from old.txt\n"), "{d}");
        assert!(
            d.contains("Binary files a/img.bin and b/img.bin differ"),
            "{d}"
        );
        assert!(
            !d.contains("web/") && !d.contains("OUTSIDE") && !d.contains("moved.txt"),
            "{d}"
        );
        // Each name works as the path of `git.diff` and `files.read` (a deleted file has nothing left to read).
        let files = crate::files::Files::open(&web, 1 << 20).expect("files");
        for (p, deleted) in &paths {
            let one = diff(&web, Some(p)).expect("diff of one file");
            assert_eq!(diff_paths(&one).len(), 1, "{p}: {one}");
            assert!(one.contains(&format!(" b/{p}")), "{p}: {one}");
            assert_eq!(files.read(p).is_ok(), !deleted, "{p}");
        }
    }

    #[test]
    fn a_root_workspace_diff_is_what_git_prints() {
        let r = folder_workspace();
        // Without `--relative`, as before: at a checkout's root the flag changes nothing.
        let mut want = git(
            &r.dir,
            &[
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                SUBMODULE_COMMITS_ONLY,
                "HEAD",
                "--",
                ".",
            ],
        )
        .expect("git diff");
        let listed = git(
            &r.dir,
            &["ls-files", "--others", "--exclude-standard", "-z"],
        )
        .expect("ls");
        let names: Vec<&str> = listed.split('\0').filter(|n| !n.is_empty()).collect();
        want.push_str(&untracked_diff(&r.dir, &names, true, &mut Vec::new()).expect("untracked"));
        let d = diff(&r.dir, None).expect("diff");
        assert_eq!(d, want);
        assert!(
            d.contains("diff --git a/web/w.txt b/web/w.txt") && d.contains("+OUTSIDE"),
            "{d}"
        );
    }

    #[test]
    fn a_workspace_inside_a_checkout_counts_only_its_own_folder() {
        let r = Repo::new(None);
        fs::create_dir_all(r.dir.join("web")).expect("dir");
        fs::write(r.dir.join("web/w.txt"), "web\n").expect("write");
        fs::write(r.dir.join("web/m.txt"), "move me\n").expect("write");
        fs::write(r.dir.join("gone.txt"), "bye\n").expect("write");
        fs::write(r.dir.join("in.txt"), "in\n").expect("write");
        r.commit("layout");
        // Outside the folder: a tracked change, an untracked file, a deleted file.
        fs::write(r.dir.join("a.txt"), "one\nOUTSIDE\n").expect("write");
        fs::write(r.dir.join("root.txt"), "r\n").expect("write");
        fs::remove_file(r.dir.join("gone.txt")).expect("rm");
        // Renames across the boundary, both ways: inside the folder one is a new file, the other a deleted one.
        fs::create_dir_all(r.dir.join("web/in b")).expect("dir");
        r.git(&["mv", "in.txt", "web/in b/in.txt"]);
        r.git(&["mv", "web/m.txt", "moved.txt"]);
        // Inside: a tracked change and an untracked file whose name has a space.
        fs::write(r.dir.join("web/w.txt"), "web\nINSIDE\n").expect("write");
        fs::write(r.dir.join("web/new file.txt"), "n\n").expect("write");

        let web = status(&r.dir.join("web")).expect("status");
        // web/w.txt, web/new file.txt, web/in b/in.txt (added), web/m.txt (deleted).
        assert_eq!(
            (web.files_changed, web.insertions, web.deletions),
            (4, 2, 1)
        );
        assert_eq!(web.head_commit, r.git(&["rev-parse", "HEAD"]).trim());
        // The diff of the folder lists the same files the count describes, each named from the folder.
        let d = diff(&r.dir.join("web"), None).expect("diff");
        for inside in [
            "diff --git a/w.txt b/w.txt\n",
            "diff --git a/new file.txt b/new file.txt\n",
            "diff --git a/in b/in.txt b/in b/in.txt\n",
            "diff --git a/m.txt b/m.txt\n",
        ] {
            assert!(d.contains(inside), "{inside} missing from {d}");
        }
        for outside in ["web/", "root.txt", "gone.txt", "OUTSIDE", "moved.txt"] {
            assert!(!d.contains(outside), "{outside} in {d}");
        }

        // At the checkout's root (a worktree the node made) everything still counts: the four above plus a.txt, root.txt and
        // gone.txt; each rename is one entry.
        let root = status(&r.dir).expect("status");
        let porcelain = r.git(&["status", "--porcelain"]);
        assert_eq!(root.files_changed as usize, porcelain.lines().count());
        assert_eq!(root.files_changed, 7);
    }

    #[test]
    fn a_path_is_a_name_not_a_pattern() {
        let r = Repo::new(None);
        for f in ["a1.txt", "a[1].txt"] {
            fs::write(r.dir.join(f), "one\n").expect("write");
        }
        r.commit("names");
        for f in ["a1.txt", "a[1].txt"] {
            fs::write(r.dir.join(f), "one\ntwo\n").expect("write");
        }
        for f in ["b1.txt", "b[1].txt"] {
            fs::write(r.dir.join(f), "new\n").expect("write");
        }
        let files = |p: &str| -> Vec<String> {
            diff(&r.dir, Some(p))
                .expect("diff")
                .lines()
                .filter(|l| l.starts_with("diff --git "))
                .map(str::to_owned)
                .collect()
        };
        assert_eq!(files("a[1].txt"), ["diff --git a/a[1].txt b/a[1].txt"]);
        assert_eq!(files("b[1].txt"), ["diff --git a/b[1].txt b/b[1].txt"]);
        // `.` still selects everything.
        assert_eq!(files(".").len(), 4);
    }

    #[test]
    fn each_new_file_counts_as_the_diff_lists_it() {
        let r = Repo::new(None);
        // A new folder (with a folder of its own) is three files, not one `? new/` line.
        for f in ["new/a.txt", "new/b.txt", "new/deep/c.txt"] {
            fs::create_dir_all(r.dir.join(f).parent().expect("parent")).expect("dir");
            fs::write(r.dir.join(f), "x\n").expect("write");
        }
        let listed = |d: &str| d.lines().filter(|l| l.starts_with("diff --git ")).count();
        assert_eq!(status(&r.dir).expect("status").files_changed, 3);
        assert_eq!(listed(&diff(&r.dir, None).expect("diff")), 3);
        // The repository's own config cannot hide them from the count.
        r.git(&["config", "status.showUntrackedFiles", "no"]);
        assert_eq!(status(&r.dir).expect("status").files_changed, 3);
    }

    #[test]
    fn untracked_files_show_as_new_files_with_their_mode() {
        let r = Repo::new(None);
        fs::create_dir_all(r.dir.join("sub")).expect("dir");
        fs::write(r.dir.join("sub/new.txt"), "fresh\n").expect("write");
        fs::write(r.dir.join("run.sh"), "echo\n").expect("write");
        agora_platform::set_executable(&r.dir.join("run.sh"), true).expect("chmod");
        let d = diff(&r.dir, None).expect("diff");
        assert!(d.contains("diff --git a/sub/new.txt b/sub/new.txt"), "{d}");
        assert!(d.contains("+fresh"), "{d}");
        let meta = fs::metadata(r.dir.join("run.sh")).expect("meta");
        let mode = if agora_platform::is_executable(&meta) {
            "new file mode 100755"
        } else {
            "new file mode 100644"
        };
        assert!(d.contains(mode), "{d}");
        // Only this one when asked for it.
        let one = diff(&r.dir, Some("sub/new.txt")).expect("diff");
        assert!(one.contains("+fresh") && !one.contains("run.sh"), "{one}");
    }

    #[test]
    fn an_untracked_file_behind_a_folder_swapped_for_a_link_is_not_shown() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let r = Repo::new(None);
        let outside = r.dir.with_file_name(format!(
            "{}-outside",
            r.dir.file_name().expect("name").to_string_lossy()
        ));
        fs::create_dir_all(&outside).expect("dir");
        fs::write(outside.join("f.txt"), "TOPSECRET\n").expect("secret");
        // The agent flips `d` between a folder of its own and a link to a folder outside its worktree, both holding `f.txt`. The link
        // is made once (a junction on Windows, which needs no privilege) and the flipping is renames only; a link that cannot be made
        // fails the test rather than skipping it.
        let (name, link, spare) = (r.dir.join("d"), r.dir.join("d.link"), r.dir.join("d.spare"));
        fs::create_dir_all(&name).expect("dir");
        fs::write(name.join("f.txt"), "mine\n").expect("mine");
        agora_platform::link_dir(&outside, &link).expect("a folder link (a junction on Windows)");
        let stop = Arc::new(AtomicBool::new(false));
        let flipper = {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                let mut flips = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    let _ = fs::rename(&name, &spare);
                    if fs::rename(&link, &name).is_ok() {
                        flips += 1;
                    }
                    let _ = fs::rename(&name, &link);
                    let _ = fs::rename(&spare, &name);
                }
                flips
            })
        };
        let end = std::time::Instant::now() + std::time::Duration::from_secs(6);
        let mut leaked = 0;
        while std::time::Instant::now() < end {
            if diff(&r.dir, None).is_ok_and(|d| d.contains("TOPSECRET")) {
                leaked += 1;
            }
        }
        stop.store(true, Ordering::Relaxed);
        let flips = flipper.join().expect("flipper");
        let _ = fs::remove_dir_all(&outside);
        assert!(flips > 0, "the folder was never swapped for the link");
        assert_eq!(
            leaked, 0,
            "the node showed a file from outside the worktree"
        );
    }

    #[test]
    fn status_v2_parsing_counts_entries_and_reads_the_commit() {
        let out = "# branch.oid 0123456789abcdef0123456789abcdef01234567\n# branch.head main\n\
                   1 .M N... 100644 100644 100644 aaa bbb a.txt\n\
                   2 R. N... 100644 100644 100644 aaa aaa R100 new.txt\told.txt\n\
                   ? \"line\\nbreak.txt\"\n? u.txt\n";
        assert_eq!(
            parse_status_v2(out),
            (4, "0123456789abcdef0123456789abcdef01234567".to_string())
        );
        assert_eq!(
            parse_status_v2("# branch.oid (initial)\n# branch.head main\n? a\n"),
            (1, String::new())
        );
    }

    #[test]
    fn new_file_diffs_match_git() {
        let d = std::env::temp_dir().join(format!("agora-newfile-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).expect("dir");
        let cases: [(&str, &[u8]); 11] = [
            ("two", b"a\nb\n"),
            ("nonl", b"one"),
            ("empty", b""),
            ("bin", b"x\0y\n"),
            ("solo", b"solo\n"),
            ("\u{e9}t\u{e9}", "caf\u{e9}\n".as_bytes()),
            ("sp ace", b"q\n"),
            ("crlf", b"r\r\nn\r\n"),
            ("qu\"ote", b"z\n"),
            ("back\\slash", b"w\n"),
            ("run.sh", b"echo\n"),
        ];
        for (name, data) in cases {
            // Names Windows does not allow are only checked where they can be made.
            if fs::write(d.join(name), data).is_err() {
                continue;
            }
            let mut mode = "100644";
            if name == "run.sh" {
                agora_platform::set_executable(&d.join(name), true).expect("chmod");
                if agora_platform::is_executable(&fs::metadata(d.join(name)).expect("meta")) {
                    mode = "100755";
                }
            }
            let git_says = raw_git()
                .arg("-C")
                .arg(&d)
                .args([
                    "-c",
                    "core.quotePath=true",
                    "-c",
                    "core.abbrev=7",
                    "-c",
                    "core.autocrlf=false",
                ])
                .args([
                    "diff",
                    "--no-index",
                    "--no-color",
                    "--no-ext-diff",
                    "--no-textconv",
                ])
                .args(["--", "/dev/null", name])
                .env("GIT_CEILING_DIRECTORIES", d.parent().expect("parent"))
                .output()
                .expect("git")
                .stdout;
            assert_eq!(
                String::from_utf8_lossy(&new_file_diff(name, mode, data)),
                String::from_utf8_lossy(&git_says),
                "{name:?}"
            );
        }
        if agora_platform::symlink(Path::new("target-text"), &d.join("lnk")).is_ok() {
            let git_says = raw_git()
                .arg("-C")
                .arg(&d)
                .args([
                    "-c",
                    "core.quotePath=true",
                    "-c",
                    "core.abbrev=7",
                    "-c",
                    "core.autocrlf=false",
                ])
                .args(["diff", "--no-index", "--no-color", "--", "/dev/null", "lnk"])
                .env("GIT_CEILING_DIRECTORIES", d.parent().expect("parent"))
                .output()
                .expect("git")
                .stdout;
            assert_eq!(
                String::from_utf8_lossy(&new_file_diff("lnk", "120000", b"target-text")),
                String::from_utf8_lossy(&git_says)
            );
        }
        let _ = fs::remove_dir_all(&d);
    }

    /// `text` as UTF-16 with a byte-order mark, little-endian (what Windows PowerShell 5.1's `>` writes) or big-endian.
    fn utf16(text: &str, little: bool) -> Vec<u8> {
        let mut out = if little {
            vec![0xff, 0xfe]
        } else {
            vec![0xfe, 0xff]
        };
        for u in text.encode_utf16() {
            out.extend(if little {
                u.to_le_bytes()
            } else {
                u.to_be_bytes()
            });
        }
        out
    }

    #[test]
    fn a_utf16_text_file_shows_its_changes_as_text_and_a_binary_file_stays_binary() {
        // #940: git calls a UTF-16 file binary (it has NUL bytes); the node shows its text, decoded, as a reviewer would read it.
        let r = Repo::new(None);
        fs::write(r.dir.join("ps.txt"), utf16("hello\r\n", true)).expect("write");
        let d = diff(&r.dir, Some("ps.txt")).expect("diff");
        assert!(
            d.contains("--- /dev/null\n+++ b/ps.txt\n@@ -0,0 +1 @@\n+hello\r\n"),
            "{d}"
        );
        assert!(!d.contains("Binary files"), "{d}");
        // The blob id is still that of the bytes on disk, as git names it.
        assert!(
            d.contains(&format!(
                "index 0000000..{}\n",
                &r.git(&["hash-object", "ps.txt"]).trim()[..7]
            )),
            "{d}"
        );

        fs::write(r.dir.join("be.txt"), utf16("one\ntwo\n", false)).expect("write");
        fs::write(r.dir.join("gone.txt"), utf16("bye\n", true)).expect("write");
        fs::write(r.dir.join("img.bin"), b"x\0y\n").expect("write");
        let lines: String = (1..=20).map(|n| format!("line {n}\n")).collect();
        fs::write(r.dir.join("long.txt"), utf16(&lines, true)).expect("write");
        fs::write(r.dir.join("enc.txt"), "same\n").expect("write");
        fs::write(r.dir.join("odd.bin"), [0xff, 0xfe, b'h', 0, b'i']).expect("write");
        fs::write(r.dir.join(".env"), utf16("TOKEN=old-secret\n", true)).expect("write");
        r.commit("utf16");
        // Only the encoding changed: git's own line says so better than an empty text diff.
        fs::write(r.dir.join("enc.txt"), utf16("same\n", true)).expect("write");
        // A byte-order mark on what is not UTF-16 (an odd length): binary, as before.
        fs::write(r.dir.join("odd.bin"), [0xff, 0xfe, b'h', 0, b'o']).expect("write");
        fs::write(r.dir.join(".env"), utf16("TOKEN=new-secret\n", true)).expect("write");
        fs::write(r.dir.join("ps.txt"), utf16("hello\r\nworld\r\n", true)).expect("write");
        fs::write(r.dir.join("be.txt"), utf16("one\nTWO\n", false)).expect("write");
        fs::remove_file(r.dir.join("gone.txt")).expect("rm");
        fs::write(r.dir.join("img.bin"), b"x\0z\n").expect("write");
        let changed = lines
            .replace("line 2\n", "line two\n")
            .replace("line 18\n", "");
        fs::write(r.dir.join("long.txt"), utf16(&changed, true)).expect("write");
        let d = diff(&r.dir, None).expect("diff");
        assert!(
            d.contains("--- a/ps.txt\n+++ b/ps.txt\n@@ -1 +1,2 @@\n hello\r\n+world\r\n"),
            "{d}"
        );
        assert!(
            d.contains("--- a/be.txt\n+++ b/be.txt\n@@ -1,2 +1,2 @@\n one\n-two\n+TWO\n"),
            "{d}"
        );
        assert!(
            d.contains("deleted file mode 100644\n")
                && d.contains("--- a/gone.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-bye\n"),
            "{d}"
        );
        // Two changes far apart are two hunks, each with three lines of context, as git prints them.
        assert!(
            d.contains(
                "@@ -1,5 +1,5 @@\n line 1\n-line 2\n+line two\n line 3\n line 4\n line 5\n\
                 @@ -15,6 +15,5 @@\n line 15\n line 16\n line 17\n-line 18\n line 19\n line 20\n"
            ),
            "{d}"
        );
        assert!(
            d.contains("Binary files a/img.bin and b/img.bin differ"),
            "{d}"
        );
        assert!(
            d.contains("Binary files a/enc.txt and b/enc.txt differ")
                && d.contains("Binary files a/odd.bin and b/odd.bin differ"),
            "{d}"
        );
        assert_eq!(d.matches("Binary files").count(), 3, "{d}");
        // A secret file is still cut to its name (ADR-0080), never decoded into the text.
        assert!(
            d.contains("diff --git a/.env b/.env\n") && !d.contains("secret"),
            "{d}"
        );
        // The bytes on disk are never rewritten.
        assert_eq!(
            fs::read(r.dir.join("ps.txt")).expect("read"),
            utf16("hello\r\nworld\r\n", true)
        );
    }

    #[test]
    fn a_utf16_file_without_a_zero_byte_shows_as_its_text_too() {
        // One line of letters above U+00FF and no line break: no zero byte, so git calls it text and prints the raw bytes.
        let r = Repo::new(None);
        fs::write(
            r.dir.join("greek.txt"),
            utf16("\u{3b1}\u{3b2}\u{3b3}", true),
        )
        .expect("write");
        let d = diff(&r.dir, Some("greek.txt")).expect("diff");
        assert!(
            d.contains("+++ b/greek.txt\n@@ -0,0 +1 @@\n+\u{3b1}\u{3b2}\u{3b3}\n\\ No newline at end of file\n"),
            "{d}"
        );
        r.commit("greek");
        fs::write(
            r.dir.join("greek.txt"),
            utf16("\u{3b1}\u{3b2}\u{3b3}\u{3b4}", true),
        )
        .expect("write");
        let d = diff(&r.dir, None).expect("diff");
        assert!(
            d.contains(
                "--- a/greek.txt\n+++ b/greek.txt\n@@ -1 +1 @@\n-\u{3b1}\u{3b2}\u{3b3}\n\\ No newline at end of file\n\
                 +\u{3b1}\u{3b2}\u{3b3}\u{3b4}\n\\ No newline at end of file\n"
            ),
            "{d}"
        );
        assert!(!d.contains('\u{fffd}'), "{d}");
    }

    #[test]
    fn a_utf16_file_that_leads_to_a_secret_file_is_not_read_for_the_diff() {
        // ADR-0080: the name git printed is not secret, but the file opened is (a link swapped in after git ran). Unless the owner
        // shows secret files, the read is refused where the opened file really is, and the section stays git's.
        let r = Repo::new(None);
        fs::write(r.dir.join("notes.txt"), utf16("TOKEN=old\n", true)).expect("write");
        fs::write(r.dir.join(".env"), utf16("TOKEN=new-secret\n", true)).expect("write");
        r.commit("notes");
        fs::remove_file(r.dir.join("notes.txt")).expect("rm");
        if agora_platform::symlink(Path::new(".env"), &r.dir.join("notes.txt")).is_err() {
            return;
        }
        let git_says = "diff --git a/notes.txt b/notes.txt\nindex 1111111..2222222 100644\n\
                        Binary files a/notes.txt and b/notes.txt differ\n";
        assert_eq!(
            utf16::decode_sections(&r.dir, git_says.to_string(), false),
            git_says
        );
        let shown = utf16::decode_sections(&r.dir, git_says.to_string(), true);
        assert!(shown.contains("-TOKEN=old\n+TOKEN=new-secret\n"), "{shown}");
        // And a UTF-16 secret file itself, through the whole diff: named only.
        fs::write(r.dir.join(".env"), utf16("TOKEN=newer-secret\n", true)).expect("write");
        let (d, secret) = diff_with(&r.dir, Some(".env"), false).expect("diff");
        assert!(!d.contains("TOKEN"), "{d}");
        assert_eq!(secret.len(), 1, "{d}");
    }

    #[test]
    fn an_untracked_link_shows_as_a_link_and_a_fifo_does_not_stop_the_diff() {
        let r = Repo::new(None);
        fs::write(r.dir.join("inside.txt"), "INSIDE CONTENT\n").expect("write");
        if agora_platform::symlink(Path::new("inside.txt"), &r.dir.join("lnk")).is_ok() {
            let one = diff(&r.dir, Some("lnk")).expect("diff");
            assert!(one.contains("new file mode 120000"), "{one}");
            assert!(
                one.contains("+inside.txt") && !one.contains("INSIDE CONTENT"),
                "{one}"
            );
        }
        // A FIFO (where `mkfifo` exists): left out, and nothing waits on it.
        let fifo = r.dir.join("pipe");
        // Git for Windows' `mkfifo` makes a plain file: only a real FIFO is checked.
        if Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .is_ok_and(|s| s.success())
            && fs::metadata(&fifo).is_ok_and(|m| !m.is_file())
        {
            let dir = r.dir.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let d = diff(&dir, None);
                let read = crate::files::Files::open(&dir, 1024).map(|f| f.read("pipe"));
                let _ = tx.send((d, read));
            });
            let (d, read) = rx
                .recv_timeout(std::time::Duration::from_secs(20))
                .expect("git.diff or files.read waited on a FIFO");
            let d = d.expect("diff");
            assert!(!d.contains("b/pipe"), "{d}");
            assert!(read.expect("open").is_err());
        }
    }

    #[test]
    fn shortstat_parsing() {
        assert_eq!(
            parse_shortstat(" 2 files changed, 3 insertions(+), 1 deletion(-)\n"),
            (3, 1)
        );
        assert_eq!(parse_shortstat(" 1 file changed, 1 deletion(-)\n"), (0, 1));
        assert_eq!(parse_shortstat(""), (0, 0));
    }

    #[test]
    fn a_capped_read_keeps_only_that_much_output_and_still_reports_real_failures() {
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        let out = git_capped(here, &["log", "--format=%H%n%s%n%b"], 100).expect("log");
        assert!(out.len() <= 100);
        assert!(git_capped(here, &["rev-parse", "no-such-ref-xyz"], 100).is_err());
    }

    /// Planted by an agent that can write the checkout's `.git` (config, hooks) and its worktree (`.gitattributes`): each one makes
    /// plain git run a program. The node's git must run none of them.
    #[test]
    fn only_the_repositorys_own_filters_are_blanked_and_an_unnamable_one_refuses() {
        let out = b"global\0filter.lfs.clean\0system\0filter.x.process\0local\0filter.a=b.clean\0worktree\0filter.my name.smudge\0";
        assert_eq!(
            repository_filters(out),
            Some(vec![
                "filter.a=b.clean".to_owned(),
                "filter.my name.smudge".to_owned()
            ])
        );
        assert_eq!(repository_filters(b""), Some(vec![]));
        assert_eq!(repository_filters(b"local\0filter.\xff.clean\0"), None);
        // A non-UTF-8 name in the user's global config is theirs, not refused.
        assert_eq!(
            repository_filters(b"global\0filter.\xff.clean\0"),
            Some(vec![])
        );
    }

    #[test]
    fn the_nodes_git_runs_no_filter_planted_in_a_submodules_config() {
        let r = Repo::new(None);
        let lib = r.dir.with_file_name(format!(
            "{}-lib",
            r.dir.file_name().expect("name").to_string_lossy()
        ));
        let run = |dir: &Path, args: &[&str]| {
            let ok = raw_git()
                .args(["-c", "user.name=t", "-c", "user.email=t@e"])
                .args(["-c", "protocol.file.allow=always"])
                .arg("-C")
                .arg(dir)
                .args(args)
                .output()
                .expect("git")
                .status
                .success();
            assert!(ok, "git {args:?}");
        };
        fs::create_dir_all(&lib).expect("lib");
        run(&lib, &["init", "-q"]);
        fs::write(lib.join("a.txt"), "one\n").expect("a");
        run(&lib, &["add", "a.txt"]);
        run(&lib, &["commit", "-q", "-m", "lib"]);
        run(
            &r.dir,
            &["submodule", "add", "-q", &lib.to_string_lossy(), "lib"],
        );
        run(&r.dir, &["commit", "-q", "-m", "sub"]);
        let mark = r.dir.join("ran-sub-clean");
        let modules = r.dir.join(".git/modules/lib");
        run(
            &r.dir,
            &[
                "--git-dir",
                &modules.to_string_lossy(),
                "config",
                "filter.ev.clean",
                &format!("sh -c 'touch \"{}\"; cat'", mark.display()),
            ],
        );
        fs::write(r.dir.join("lib/.gitattributes"), "*.txt filter=ev\n").expect("attributes");
        // Same size, later mtime: git must hash the file, which runs the clean filter.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        // Keep the checkout's own line ending (`core.autocrlf` writes `one\r\n` on Windows): a size change lets git call the
        // file modified without hashing it, and the clean filter would never run.
        let checked_out = fs::read_to_string(r.dir.join("lib/a.txt")).expect("read");
        fs::write(r.dir.join("lib/a.txt"), checked_out.replace("one", "uno")).expect("edit");

        // The node's git.
        status(&r.dir).expect("status");
        diff(&r.dir, None).expect("diff");
        assert!(is_dirty(&r.dir).is_ok());
        assert!(!mark.exists(), "the node's git ran a submodule's filter");
        // Control: plain git runs it, so the plant works on this system.
        let _ = raw_git()
            .arg("-C")
            .arg(&r.dir)
            .args(["status", "--porcelain"])
            .output();
        assert!(mark.exists(), "the plant itself did not work");
        let _ = fs::remove_dir_all(&lib);
    }

    #[test]
    fn only_a_clean_answer_from_git_means_nothing_to_blank() {
        let keys = b"local\0filter.x.clean\0";
        assert_eq!(
            Filters::from_answer(Some(0), keys),
            Filters::Blank(vec!["filter.x.clean".into()])
        );
        assert_eq!(Filters::from_answer(Some(1), b""), Filters::Blank(vec![]));
        assert_eq!(Filters::from_answer(Some(1), keys), Filters::Refuse);
        // A git before 2.26 does not know `--show-scope`; a broken config; a signal.
        assert_eq!(Filters::from_answer(Some(129), b""), Filters::Refuse);
        assert_eq!(Filters::from_answer(Some(128), b""), Filters::Refuse);
        assert_eq!(Filters::from_answer(None, b""), Filters::Refuse);
    }

    #[test]
    fn the_nodes_git_in_a_worktree_reads_only_the_verified_git_folders() {
        let r = Repo::new(None);
        let wt = r.dir.with_file_name(format!(
            "{}-pinned-wt",
            r.dir.file_name().expect("name").to_string_lossy()
        ));
        create_worktree(&r.dir, &wt, "pinned", None).expect("worktree");
        let (admin, common) = crate::sandbox::git_dirs(&wt).expect("verified");
        // The agent points its admin folder's `commondir` at a repository it built, with a filter in its config.
        let fake = wt.with_file_name(format!(
            "{}-fake",
            wt.file_name().expect("name").to_string_lossy()
        ));
        let ok = raw_git()
            .args(["clone", "-q", "--bare"])
            .arg(&r.dir)
            .arg(&fake)
            .status()
            .expect("clone")
            .success();
        assert!(ok);
        let mark = wt.join("ran-fake-clean");
        let set = raw_git()
            .arg("--git-dir")
            .arg(&fake)
            .args(["config", "filter.ev.clean"])
            .arg(format!("sh -c 'touch \"{}\"; cat'", mark.display()))
            .status()
            .expect("config")
            .success();
        assert!(set);
        // Pinned: the node's git uses the real shared folder.
        let used = hardened(&wt)
            .args(["rev-parse", "--git-common-dir"])
            .output()
            .expect("rev-parse");
        let used = PathBuf::from(String::from_utf8_lossy(&used.stdout).trim());
        // git prints forward slashes on Windows; compare the folders, not the spellings.
        assert_eq!(
            agora_platform::canonicalize_plain(&used).expect("used"),
            agora_platform::canonicalize_plain(&common).expect("common")
        );
        fs::write(admin.join("commondir"), format!("{}\n", fake.display())).expect("commondir");
        fs::write(wt.join(".gitattributes"), "*.txt filter=ev\n").expect("attributes");
        std::thread::sleep(std::time::Duration::from_millis(1100));
        // Keep the checkout's own line ending (`core.autocrlf` writes `one\r\n` on Windows): a size change lets git call the
        // file modified without hashing it, and the clean filter would never run.
        let checked_out = fs::read_to_string(wt.join("a.txt")).expect("read");
        fs::write(wt.join("a.txt"), checked_out.replace("one", "uno")).expect("edit");

        // Once `commondir` moved it refuses: git would read the branch refs through `commondir` even when pinned, so nothing is answered from the
        // repository it names. The filter blanking alone would also stop this plant, but not one added between its question and
        // the command.
        let refused = hardened(&wt)
            .args(["rev-parse", "--git-common-dir"])
            .output()
            .expect("rev-parse");
        assert!(!refused.status.success(), "the node's git ran");
        assert!(matches!(status(&wt), Err(GitError::LinksMoved(_))));
        assert!(matches!(diff(&wt, None), Err(GitError::LinksMoved(_))));
        assert!(
            !mark.exists(),
            "the node's git followed the agent's commondir"
        );
        // Control: plain git follows it.
        let _ = raw_git()
            .arg("-C")
            .arg(&wt)
            .args(["status", "--porcelain"])
            .output();
        assert!(mark.exists(), "the plant itself did not work");

        // A `.git` that is a link is refused outright.
        let linked = wt.with_file_name(format!(
            "{}-linked",
            wt.file_name().expect("name").to_string_lossy()
        ));
        fs::create_dir_all(&linked).expect("dir");
        if agora_platform::symlink(&wt.join(".git"), &linked.join(".git")).is_ok() {
            assert!(status(&linked).is_err());
        }
        for d in [&wt, &fake, &linked] {
            let _ = fs::remove_dir_all(d);
        }
    }

    #[test]
    fn a_folder_uses_the_innermost_worktree_recorded_around_it() {
        let r = Repo::new(None);
        let outer = agora_platform::canonicalize_plain(&r.dir).expect("dir");
        let inner = outer.join("inner");
        fs::create_dir_all(inner.join("deep")).expect("dirs");
        let entry = |work: &Path, tag: &str| Made {
            work: work.to_path_buf(),
            admin: work.join(format!("{tag}-admin")),
            common: work.join(format!("{tag}-common")),
            commondir: Vec::new(),
            gitdir: Vec::new(),
            project: None,
        };
        made().extend([entry(&outer, "outer"), entry(&inner, "inner")]);
        let found = made_dirs(&inner.join("deep")).map(|m| m.admin);
        let at_outer = made_dirs(&outer).map(|m| m.admin);
        made().retain(|m| !m.work.starts_with(&outer));
        assert_eq!(found, Some(inner.join("inner-admin")));
        assert_eq!(at_outer, Some(outer.join("outer-admin")));
    }

    #[test]
    fn the_nodes_git_uses_the_git_folders_recorded_when_it_made_the_worktree() {
        let r = Repo::new(None);
        let wt = r.dir.with_file_name(format!(
            "{}-recorded-wt",
            r.dir.file_name().expect("name").to_string_lossy()
        ));
        create_worktree(&r.dir, &wt, "recorded", None).expect("worktree");
        let (admin, common) = crate::sandbox::git_dirs(&wt).expect("verified");
        // The agent builds a repository inside its worktree, with an admin folder whose back-pointer names this worktree and a
        // filter in its config, and points its `.git` file there: every check of the files on disk now agrees with it.
        let fake = wt.join("fake");
        let ok = raw_git()
            .args(["clone", "-q", "--bare"])
            .arg(&r.dir)
            .arg(&fake)
            .status()
            .expect("clone")
            .success();
        assert!(ok);
        let fake_admin = fake.join("worktrees").join("x");
        fs::create_dir_all(&fake_admin).expect("admin");
        for f in ["HEAD", "index"] {
            fs::copy(admin.join(f), fake_admin.join(f)).expect("copy");
        }
        let own = agora_platform::canonicalize_plain(&wt)
            .expect("wt")
            .join(".git");
        fs::write(fake_admin.join("gitdir"), format!("{}\n", own.display())).expect("gitdir");
        fs::write(fake_admin.join("commondir"), "../..\n").expect("commondir");
        let mark = wt.join("ran-fake-clean");
        let set = raw_git()
            .arg("--git-dir")
            .arg(&fake)
            .args(["config", "filter.ev.clean"])
            .arg(format!("sh -c 'touch \"{}\"; cat'", mark.display()))
            .status()
            .expect("config")
            .success();
        assert!(set);
        fs::write(
            wt.join(".git"),
            format!("gitdir: {}\n", fake_admin.display()),
        )
        .expect(".git");
        assert!(
            crate::sandbox::git_dirs(&wt).is_some_and(|(_, c)| c != common),
            "the fake repository does not pass the check on disk"
        );
        fs::write(wt.join(".gitattributes"), "*.txt filter=ev\n").expect("attributes");
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let checked_out = fs::read_to_string(wt.join("a.txt")).expect("read");
        fs::write(wt.join("a.txt"), checked_out.replace("one", "uno")).expect("edit");

        let used = hardened(&wt)
            .args(["rev-parse", "--git-common-dir"])
            .output()
            .expect("rev-parse");
        let used = PathBuf::from(String::from_utf8_lossy(&used.stdout).trim());
        assert_eq!(
            agora_platform::canonicalize_plain(&used).expect("used"),
            agora_platform::canonicalize_plain(&common).expect("common")
        );
        status(&wt).expect("pinned status");
        diff(&wt, None).expect("pinned diff");
        assert!(
            !mark.exists(),
            "the node's git followed the agent's .git file"
        );
        // Control: plain git follows it.
        let _ = raw_git()
            .arg("-C")
            .arg(&wt)
            .args(["status", "--porcelain"])
            .output();
        assert!(mark.exists(), "the plant itself did not work");

        // Removing it still works: the node puts its own `.git` back first (git refuses a worktree that does not point back), and
        // does not write through a `.git` the agent made a link to something outside.
        let outside = wt.with_file_name(format!(
            "{}-outside",
            wt.file_name().expect("name").to_string_lossy()
        ));
        fs::write(&outside, "keep\n").expect("outside");
        fs::remove_file(wt.join(".git")).expect("unlink");
        if agora_platform::symlink(&outside, &wt.join(".git")).is_err() {
            fs::write(
                wt.join(".git"),
                format!("gitdir: {}\n", fake_admin.display()),
            )
            .expect(".git");
        }
        let work = agora_platform::canonicalize_plain(&wt).expect("wt");
        remove_worktree(&r.dir, &wt, true).expect("remove");
        assert!(!wt.exists(), "the worktree is still there");
        assert_eq!(fs::read_to_string(&outside).expect("outside"), "keep\n");
        assert!(made().iter().all(|m| m.work != work), "still recorded");
        let _ = fs::remove_file(&outside);
        let _ = fs::remove_dir_all(&wt);
    }

    #[test]
    fn a_filter_that_cannot_be_blanked_stops_the_nodes_git() {
        let r = Repo::new(None);
        let ok = |f: &Filters| {
            hardened_with(&r.dir, f)
                .args(["status", "--porcelain"])
                .output()
                .expect("git")
                .status
                .success()
        };
        assert!(ok(&Filters::Blank(vec![])));
        assert!(!ok(&Filters::Refuse));
        // Blanking works with this machine's git (CI runs a recent one), so a named filter does not stop it.
        assert!(config_env_supported());
        assert!(ok(&Filters::Blank(vec!["filter.a=b.clean".into()])));
    }

    #[test]
    fn the_nodes_git_runs_no_program_a_repository_plants() {
        let r = Repo::new(None);
        let mark = |n: &str| r.dir.join(format!("ran-{n}"));
        let touch = |n: &str| {
            format!(
                "sh -c 'touch \"{}\"; cat \"$0\" 2>/dev/null' ",
                mark(n).display()
            )
        };
        r.git(&["config", "core.fsmonitor", &touch("fsmonitor")]);
        r.git(&["config", "diff.external", &touch("external")]);
        r.git(&["config", "diff.conv.textconv", &touch("textconv")]);
        // A clean filter runs when git hashes a changed tracked file (status, diff); a `=` in the name defeats `-c name=`.
        r.git(&["config", "filter.a=b.clean", &touch("clean")]);
        fs::write(r.dir.join(".gitattributes"), "*.txt diff=conv filter=a=b\n")
            .expect("attributes");
        let hook = r.dir.join(".git/hooks/post-checkout");
        fs::write(
            &hook,
            format!("#!/bin/sh\ntouch \"{}\"\n", mark("hook").display()),
        )
        .expect("hook");
        let _ = Command::new("chmod").arg("+x").arg(&hook).status();
        fs::write(r.dir.join("a.txt"), "two\n").expect("change");
        let planted = ["fsmonitor", "external", "textconv", "hook", "clean"];

        // Plain git, as an unhardened node would run it: which plants work on this system.
        let plain = |args: &[&str]| {
            let _ = raw_git().arg("-C").arg(&r.dir).args(args).output();
        };
        plain(&["status", "--porcelain"]);
        plain(&["diff", "HEAD"]);
        plain(&[
            "worktree",
            "add",
            "-q",
            "-b",
            "plain-wt",
            "--",
            "../plain-wt-probe",
        ]);
        let effective: Vec<&str> = planted
            .iter()
            .copied()
            .filter(|n| mark(n).exists())
            .collect();
        assert!(
            effective.contains(&"fsmonitor") && effective.contains(&"clean"),
            "the plant itself did not work: {effective:?}"
        );
        for n in planted {
            let _ = fs::remove_file(mark(n));
        }

        // The node's git.
        status(&r.dir).expect("status");
        diff(&r.dir, None).expect("diff");
        let wt = r.dir.with_file_name(format!(
            "{}-hardened-wt",
            r.dir.file_name().expect("name").to_string_lossy()
        ));
        create_worktree(&r.dir, &wt, "hardened-wt", None).expect("worktree");
        log(&r.dir, 5).expect("log");
        for n in &effective {
            assert!(!mark(n).exists(), "the node's git ran the planted {n}");
        }
        let _ = fs::remove_dir_all(wt);
        let _ = fs::remove_dir_all(r.dir.with_file_name("plain-wt-probe"));
    }

    /// The agent's own git in `dir` (plain, unconfined, following whatever the worktree's links say), with an identity; its output.
    fn agent_git(dir: &Path, args: &[&str]) -> String {
        let out = raw_git()
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "protocol.file.allow=always",
            ])
            .args(args)
            .output()
            .expect("git");
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A worktree whose admin folder's `commondir` names another bare repository (T8b: the agent may write its admin folder and the
    /// shared object store). That repository sits inside the admin folder and shares the real objects. Its branches are `refs`, and with
    /// `mark` its config defines a clean filter `probe` that touches `mark`. Returns its path.
    fn common_dir_elsewhere(
        admin: &Path,
        common: &Path,
        refs: &[(&str, &str)],
        mark: Option<&Path>,
    ) -> PathBuf {
        // `git_dirs` answers canonical paths, verbatim (`\\?\C:\...`) on Windows, which git cannot use.
        let (admin, common) = (plain(admin.to_path_buf()), plain(common.to_path_buf()));
        let other = admin.join("other.git");
        let ok = raw_git()
            .args(["init", "-q", "--bare"])
            .arg(&other)
            .status()
            .expect("init")
            .success();
        assert!(ok);
        let objects = common
            .join("objects")
            .display()
            .to_string()
            .replace('\\', "/");
        fs::write(
            other.join("objects/info/alternates"),
            format!("{objects}\n"),
        )
        .expect("alternates");
        for (branch, oid) in refs {
            fs::write(other.join("refs/heads").join(branch), format!("{oid}\n")).expect("ref");
        }
        if let Some(mark) = mark {
            let set = raw_git()
                .arg("--git-dir")
                .arg(&other)
                .args(["config", "filter.probe.clean"])
                .arg(format!("sh -c 'touch \"{}\"; cat'", mark.display()))
                .status()
                .expect("config")
                .success();
            assert!(set);
        }
        fs::write(admin.join("commondir"), format!("{}\n", other.display())).expect("commondir");
        other
    }

    /// The same contents with a newer modification time, so the next `status` hashes the file again (through its clean filter).
    fn restat(file: &Path) {
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let text = fs::read(file).expect("read");
        fs::write(file, text).expect("write");
    }

    /// T8b: removing a worktree runs no filter driver from a config its admin folder's `commondir` was pointed at; the node's own
    /// checks decide, on the links it recorded.
    #[test]
    fn removing_a_worktree_runs_no_filter_from_a_common_dir_its_agent_named() {
        removal_with_a_moved_common_dir("links-rm", true);
    }

    /// The same with no record of the worktree (a restarted node): the moved link makes the node's checks refuse.
    #[test]
    fn removing_an_unrecorded_worktree_runs_no_filter_from_a_common_dir_its_agent_named() {
        removal_with_a_moved_common_dir("links-rm-unrec", false);
    }

    fn removal_with_a_moved_common_dir(name: &str, recorded: bool) {
        let r = Repo::new(None);
        let wt = r.wt(name);
        create_worktree(&r.dir, &wt, name, None).expect("worktree");
        if !recorded {
            forget_made_worktree(&wt);
        }
        let (admin, common) = crate::sandbox::git_dirs(&wt).expect("verified");
        // The agent's own commit, kept on a second branch too, so nothing on its branch is unmerged.
        fs::write(wt.join(".gitattributes"), "a.txt filter=probe\n").expect("attributes");
        agent_git(&wt, &["add", "-A"]);
        agent_git(&wt, &["commit", "-q", "-m", "attributes"]);
        agent_git(&wt, &["branch", &format!("{name}-kept")]);
        let tip = agent_git(&wt, &["rev-parse", "HEAD"]);
        let mark = r.wt(&format!("{name}-ran"));
        common_dir_elsewhere(&admin, &common, &[(name, &tip)], Some(&mark));
        restat(&wt.join("a.txt"));
        // Control: git that follows the admin folder's links runs the filter.
        let _ = raw_git()
            .env("GIT_OPTIONAL_LOCKS", "0")
            .arg("-C")
            .arg(&wt)
            .args(["status", "--porcelain"])
            .output();
        assert!(mark.exists(), "the plant itself did not work");
        fs::remove_file(&mark).expect("reset");
        restat(&wt.join("a.txt"));

        let removed = remove_worktree(&r.dir, &wt, false);
        assert!(!mark.exists(), "the node's removal ran the filter");
        if recorded {
            assert_eq!(removed, Ok(()));
            assert!(!wt.exists(), "not removed");
        } else {
            assert!(
                matches!(removed, Err(GitError::LinksMoved(_))),
                "{removed:?}"
            );
        }
        let _ = fs::remove_file(&mark);
        let _ = remove_worktree(&r.dir, &wt, true);
        let _ = fs::remove_dir_all(&wt);
    }

    /// T8b: once a worktree's `commondir` names another repository, the node's reads there are refused rather than answered from
    /// refs it did not record; a worktree the node made keeps the identity it was made with, and one it has no record of is refused.
    #[test]
    fn the_nodes_reads_refuse_a_worktree_whose_common_dir_link_moved() {
        let r = Repo::new(Some("https://example.com/links.git"));
        let checkout = discover_checkout(&r.dir).expect("checkout").project;
        let wt = r.wt("links-read");
        create_worktree(&r.dir, &wt, "links-read", None).expect("worktree");
        let (admin, common) = crate::sandbox::git_dirs(&wt).expect("verified");
        status(&wt).expect("status before");
        // A root commit of the agent's own, in the shared object store it may write, named by the other repository's branch.
        let tree = agent_git(&wt, &["rev-parse", "HEAD^{tree}"]);
        let other = agent_git(&wt, &["commit-tree", &tree, "-m", "another root"]);
        common_dir_elsewhere(&admin, &common, &[("links-read", &other)], None);
        // Control: plain git now reads the other branch.
        assert_eq!(agent_git(&wt, &["rev-parse", "HEAD"]), other);

        assert!(status(&wt).is_err(), "status answered: {:?}", status(&wt));
        assert!(log(&wt, 5).is_err(), "log answered");
        assert!(diff(&wt, None).is_err(), "diff answered");
        assert_eq!(
            discover_checkout(&wt).map(|c| c.project),
            Ok(checkout.clone())
        );
        // With no record of it (a restarted node), the identity is not read from the other repository either.
        let work = agora_platform::canonicalize_plain(&wt).expect("wt");
        made().retain(|m| m.work != work);
        assert!(
            discover_checkout(&wt).is_err(),
            "identity answered: {:?}",
            discover_checkout(&wt)
        );
        assert!(status(&wt).is_err(), "status answered with no record");
        let _ = remove_worktree(&r.dir, &wt, true);
        let _ = fs::remove_dir_all(&wt);
    }

    /// The project of a worktree the node made is the one of the checkout it was made from, not one its agent gives it by moving its
    /// branch to a root commit of its own or changing the shared remote.
    #[test]
    fn a_node_made_worktree_keeps_the_identity_of_its_checkout() {
        let r = Repo::new(Some("https://example.com/identity.git"));
        let checkout = discover_checkout(&r.dir).expect("checkout").project;
        let wt = r.wt("identity");
        create_worktree(&r.dir, &wt, "identity", None).expect("worktree");
        let tree = agent_git(&wt, &["rev-parse", "HEAD^{tree}"]);
        let other = agent_git(&wt, &["commit-tree", &tree, "-m", "another root"]);
        agent_git(&wt, &["update-ref", "refs/heads/identity", &other]);
        agent_git(
            &wt,
            &[
                "config",
                "remote.origin.url",
                "https://example.com/other.git",
            ],
        );
        // Control: read from the worktree's HEAD, the root is now the agent's.
        assert_eq!(
            agent_git(&wt, &["rev-list", "--max-parents=0", "HEAD"]),
            other
        );

        let found = discover_checkout(&wt).expect("worktree");
        assert_eq!(found.project, checkout);
        assert_eq!(
            found.root,
            agora_platform::canonicalize_plain(&wt).expect("wt")
        );
        fs::create_dir_all(wt.join("inner")).expect("inner");
        assert_eq!(
            discover_checkout(&wt.join("inner")).map(|c| c.project),
            Ok(checkout)
        );
        let _ = remove_worktree(&r.dir, &wt, true);
        let _ = fs::remove_dir_all(&wt);
    }

    /// The node removes a worktree with `--force` after its own checks, so it refuses one with a checked-out submodule itself, as git
    /// would without `--force`: edits inside the submodule are not seen by its `status` (`SUBMODULE_COMMITS_ONLY`).
    #[test]
    fn a_worktree_with_a_checked_out_submodule_is_removed_only_when_forced() {
        let sub = Repo::new(None);
        let r = Repo::new(None);
        agent_git(
            &r.dir,
            &[
                "submodule",
                "-q",
                "add",
                "--",
                &sub.dir.display().to_string(),
                "sub",
            ],
        );
        agent_git(&r.dir, &["commit", "-q", "-m", "submodule"]);
        let wt = r.wt("with-sub");
        create_worktree(&r.dir, &wt, "with-sub", None).expect("worktree");
        agent_git(&wt, &["submodule", "-q", "update", "--init"]);
        assert_eq!(
            remove_worktree(&r.dir, &wt, false),
            Err(GitError::Dirty(wt.clone()))
        );
        assert!(wt.join("sub").join("a.txt").exists(), "removed");
        remove_worktree(&r.dir, &wt, true).expect("forced");
        assert!(!wt.exists());
        let _ = fs::remove_dir_all(&wt);
    }

    /// A `commondir` longer than a git link file can be is refused unread, even when its first line names the real shared folder.
    #[test]
    fn a_commondir_longer_than_a_link_file_is_refused_unread() {
        let r = Repo::new(None);
        let wt = r.wt("links-long");
        create_worktree(&r.dir, &wt, "links-long", None).expect("worktree");
        let (admin, common) = crate::sandbox::git_dirs(&wt).expect("verified");
        let cd = admin.join("commondir");
        let orig = fs::read(&cd).expect("commondir");
        let mut padded = orig.clone();
        padded.extend(std::iter::repeat_n(b'\n', 8192));
        fs::write(&cd, &padded).expect("padded");
        assert!(!links_hold(&admin, &common), "a padded commondir held");
        assert!(matches!(status(&wt), Err(GitError::LinksMoved(_))));
        fs::write(&cd, &orig).expect("reset");
        status(&wt).expect("status once put back");
        let _ = remove_worktree(&r.dir, &wt, true);
        let _ = fs::remove_dir_all(&wt);
    }

    /// A huge sparse `commondir` (no disk used) is refused without being read. Unix only: NTFS does not make a file sparse unless
    /// asked, so the same length would be allocated there.
    #[cfg(unix)]
    #[test]
    fn a_huge_sparse_commondir_is_refused_unread() {
        let r = Repo::new(None);
        let wt = r.wt("links-sparse");
        create_worktree(&r.dir, &wt, "links-sparse", None).expect("worktree");
        let (admin, common) = crate::sandbox::git_dirs(&wt).expect("verified");
        let cd = admin.join("commondir");
        let orig = fs::read(&cd).expect("commondir");
        fs::OpenOptions::new()
            .write(true)
            .open(&cd)
            .expect("open")
            .set_len(1 << 36)
            .expect("sparse");
        let started = std::time::Instant::now();
        assert!(!links_hold(&admin, &common), "a sparse commondir held");
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        fs::remove_file(&cd).expect("unlink");
        fs::write(&cd, &orig).expect("reset");
        let _ = remove_worktree(&r.dir, &wt, true);
        let _ = fs::remove_dir_all(&wt);
    }

    /// The link check never waits on a FIFO put where `commondir` was, however the swap and the check interleave.
    #[test]
    fn the_link_check_never_waits_on_a_fifo_swapped_in() {
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;
        let r = Repo::new(None);
        let wt = r.wt("links-fifo");
        create_worktree(&r.dir, &wt, "links-fifo", None).expect("worktree");
        let (admin, common) = crate::sandbox::git_dirs(&wt).expect("verified");
        let cd = admin.join("commondir");
        let orig = fs::read(&cd).expect("commondir");
        let fifo = admin.join("fifo-probe");
        if !Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .is_ok_and(|s| s.success())
        {
            return; // no FIFOs on this system
        }
        let _ = fs::remove_file(&fifo);
        let stop = Arc::new(AtomicBool::new(false));
        let (a, o, s) = (admin.clone(), orig.clone(), stop.clone());
        // Swaps `commondir` between the plain file and a FIFO until told to stop.
        let swapper = std::thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                let f = a.join("plain-probe");
                fs::write(&f, &o).expect("plain");
                fs::rename(&f, a.join("commondir")).expect("rename");
                let p = a.join("fifo-probe");
                let _ = fs::remove_file(&p);
                if Command::new("mkfifo")
                    .arg(&p)
                    .status()
                    .is_ok_and(|s| s.success())
                {
                    fs::rename(&p, a.join("commondir")).expect("rename");
                }
            }
        });
        let done = Arc::new(AtomicU32::new(0));
        let (a, c, d, s) = (admin.clone(), common.clone(), done.clone(), stop.clone());
        let checker = std::thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                let _ = links_hold(&a, &c);
                d.fetch_add(1, Ordering::Relaxed);
            }
        });
        let mut last = 0;
        let mut stuck = false;
        for _ in 0..12 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let now = done.load(Ordering::Relaxed);
            if now == last {
                stuck = true;
                break;
            }
            last = now;
        }
        stop.store(true, Ordering::Relaxed);
        swapper.join().expect("swapper");
        // A check that waits on the FIFO is let go: opened for reading and writing, which never waits, it has a writer, which closes.
        let _ = fs::OpenOptions::new().read(true).write(true).open(&cd);
        // One stuck on a FIFO that was renamed away cannot be reached: left behind, the test fails below.
        if !stuck {
            checker.join().expect("checker");
        }
        let _ = fs::remove_file(&cd);
        fs::write(&cd, &orig).expect("reset");
        assert!(!stuck, "the link check waited on a FIFO");
        assert!(last > 0);
        let _ = remove_worktree(&r.dir, &wt, true);
        let _ = fs::remove_dir_all(&wt);
    }
}

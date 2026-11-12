//! Git operations on a node: checkout discovery, worktrees, status and diff (ADR-0003).
//!
//! Uses the `git` command line (stable, ubiquitous, no extra dependency). Every call runs with a clean locale,
//! no prompts, and paths/branches after `--` or validated first, so repository content cannot inject options.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, PoisonError};

use agora_core::{ProjectId, Worktree, WorktreeError};
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
            Self::Command { .. } | Self::Io(_) => {
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
        let out = plain_hardened(dir)
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
        Command::new("git")
            .args(["config", "--get", "probe.configenv"])
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "probe.configenv")
            .env("GIT_CONFIG_VALUE_0", "yes")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .is_ok_and(|o| o.stdout.trim_ascii() == b"yes")
    })
}

/// The Git folders of one worktree this node made, recorded by [`create_worktree`] as soon as `git worktree add` returns, before any
/// agent runs there. Plain canonical spellings.
struct Made {
    work: PathBuf,
    admin: PathBuf,
    common: PathBuf,
}

/// Every worktree this node made and has not removed. Its `.git` file and everything it names can be rewritten by the agent working
/// there, which can also build a whole repository inside the worktree whose `worktrees/<name>/gitdir` points back at it, so a check
/// of the file at the time of a command proves nothing; what git said when the node made the worktree does. A node that restarts
/// starts with none of these and falls back to [`crate::sandbox::git_dirs`].
static MADE: Mutex<Vec<Made>> = Mutex::new(Vec::new());

fn made() -> std::sync::MutexGuard<'static, Vec<Made>> {
    // A panic while holding the lock leaves a plain list behind, still valid.
    MADE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Records `work`'s Git folders (see [`MADE`]); nothing when they do not check out.
fn record_made(work: &Path) {
    let plain = agora_platform::canonicalize_plain;
    let Some((Ok(work), Ok(admin), Ok(common))) = crate::sandbox::git_dirs(work)
        .map(|(admin, common)| (plain(work), plain(&admin), plain(&common)))
    else {
        return;
    };
    let mut list = made();
    list.retain(|m| m.work != work);
    list.push(Made {
        work,
        admin,
        common,
    });
}

/// `(GIT_DIR, GIT_COMMON_DIR, GIT_WORK_TREE)` for `dir` when it is, or is inside, a worktree this node made.
fn made_dirs(dir: &Path) -> Option<(PathBuf, PathBuf, PathBuf)> {
    let dir = agora_platform::canonicalize_plain(dir).ok()?;
    made()
        .iter()
        .filter(|m| dir.starts_with(&m.work))
        // The innermost one, should one worktree have been made inside another.
        .max_by_key(|m| m.work.components().count())
        .map(|m| (m.admin.clone(), m.common.clone(), m.work.clone()))
}

/// `git -C dir` with [`HARDENED`] and the node's environment. In a worktree this node made (or a folder below it) the Git folders are
/// pinned (`GIT_DIR`, `GIT_COMMON_DIR`, `GIT_WORK_TREE`) to the ones recorded when it was made ([`MADE`]), so nothing the agent
/// writes into its `.git` file or admin folder's `commondir` points the node's git at a config the agent wrote. In any other linked
/// worktree they are pinned to the ones [`crate::sandbox::git_dirs`] verifies, and a `.git` that is a link is refused. Any other
/// `.git` file (a submodule's, a separate git dir) is followed as git does.
fn plain_hardened(dir: &Path) -> Command {
    let mut c = Command::new("git");
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
    if let Some((admin, common, work)) = made_dirs(dir) {
        c.env("GIT_DIR", admin)
            .env("GIT_COMMON_DIR", common)
            .env("GIT_WORK_TREE", work);
        return c;
    }
    let Ok(meta) = std::fs::symlink_metadata(dir.join(".git")) else {
        return c;
    };
    if meta.is_symlink() {
        c.args(REFUSED);
    } else if meta.is_file() {
        // Plain spellings, as for any path a process is given (no verbatim `\\?\` prefix on Windows).
        let plain = agora_platform::canonicalize_plain;
        if let Some((Ok(admin), Ok(common), Ok(work))) = crate::sandbox::git_dirs(dir)
            .map(|(admin, common)| (plain(&admin), plain(&common), plain(dir)))
        {
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

fn git(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    let out = hardened(dir)
        .args(args)
        .output()
        .map_err(|e| GitError::Io(e.to_string()))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(GitError::Command {
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

/// Find the checkout containing `path` and compute its machine-independent project identity.
pub fn discover_checkout(path: &Path) -> Result<CheckoutInfo, GitError> {
    let root = git(path, &["rev-parse", "--show-toplevel"])
        .map_err(|_| GitError::NotARepository(path.to_path_buf()))?;
    // Rebuilt from its parts so it uses this system's separator: git for Windows answers `C:/...`.
    let root: PathBuf = Path::new(root.trim()).components().collect();
    // A repository can have several root commits; the lexicographically smallest is a stable choice.
    let roots = git(&root, &["rev-list", "--max-parents=0", "HEAD"])?;
    let root_commit = roots
        .lines()
        .min()
        .ok_or_else(|| GitError::NotARepository(root.clone()))?
        .to_string();
    let remote = git(&root, &["remote", "get-url", "origin"]).ok();
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
    record_made(Path::new(path.as_ref()));
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
    let initial = git(
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
        if git(&self.checkout, &["worktree", "list", "--porcelain", "-z"])?
            .split('\0')
            .any(|field| field == checked_out)
        {
            return Ok(());
        }
        // Exit 1 means a direct ref. A symbolic ref or an inspection error must not authorize deletion.
        let symbolic = hardened(&self.checkout)
            .args(["symbolic-ref", "--quiet", &reference])
            .output()
            .map_err(|e| GitError::Io(e.to_string()))?;
        if symbolic.status.code() != Some(1) {
            return Ok(());
        }
        // Never follow a symbolic ref even if another process rewrites it after the check above.
        git(
            &self.checkout,
            &["update-ref", "--no-deref", "-d", &reference, &self.initial],
        )
        .map(|_| ())
    }
}

fn branch_exists(dir: &Path, branch: &str) -> Result<bool, GitError> {
    match git(
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
/// A submodule moved to another commit still shows; edits inside it do not. Removing such a worktree is refused by git itself
/// unless forced, so [`remove_worktree`] loses nothing.
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
    let b = git(worktree, &["symbolic-ref", "--quiet", "--short", "HEAD"]).ok();
    Ok(b.map(|s| s.trim().to_string()))
}

/// Commits on `branch` that no other branch or remote has.
fn unmerged_commits(dir: &Path, branch: &str) -> Result<u32, GitError> {
    let tip = format!("refs/heads/{branch}");
    // For `--branches` the pattern is relative to refs/heads/.
    let exclude = format!("--exclude={branch}");
    let n = git(
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

/// Remove a worktree. Refuses to discard uncommitted changes or unmerged commits unless `force` is set.
/// The branch itself is never deleted.
pub fn remove_worktree(checkout: &Path, worktree: &Path, force: bool) -> Result<(), GitError> {
    if !force {
        if is_dirty(worktree)? {
            return Err(GitError::Dirty(worktree.to_path_buf()));
        }
        if let Some(branch) = current_branch(worktree)? {
            if unmerged_commits(checkout, &branch)? > 0 {
                return Err(GitError::Unmerged(branch));
            }
        }
    }
    let path = worktree.to_string_lossy();
    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.extend(["--", path.as_ref()]);
    // Before git removes it: the folder must still exist to be spelled canonically.
    let removing = agora_platform::canonicalize_plain(worktree);
    if let Some((admin, _, work)) =
        made_dirs(worktree).filter(|(.., w)| removing.as_ref().is_ok_and(|r| r == w))
    {
        restore_git_file(&work, &admin);
    }
    git(checkout, &args)?;
    if let Ok(work) = removing {
        made().retain(|m| m.work != work);
    }
    Ok(())
}

/// Puts back the `.git` file of a worktree this node made, which its agent may have rewritten: git refuses to remove a worktree whose
/// `.git` does not point back at its admin folder. The old entry is unlinked, not written through (it may be a link to anywhere), and
/// the new file is created only if nothing took its place; a folder there is left for git to refuse.
fn restore_git_file(work: &Path, admin: &Path) {
    use std::io::Write;
    let dot_git = work.join(".git");
    if std::fs::symlink_metadata(&dot_git).is_ok_and(|m| !m.is_dir()) {
        let _ = std::fs::remove_file(&dot_git);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&dot_git)
    {
        let _ = writeln!(f, "gitdir: {}", admin.display());
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
    let git = |args: &[&str]| git(worktree, args);
    // Each node git is two processes (the filter question, then the command), which costs about a second a pair on Windows: the
    // commit comes from the same `status` as the changed paths (`# branch.oid`) instead of a `rev-parse` of its own.
    let (files_changed, head_commit) = parse_status_v2(&git(&[
        "status",
        "--porcelain=v2",
        "--branch",
        SUBMODULE_COMMITS_ONLY,
    ])?);
    let (insertions, deletions) = parse_shortstat(&git(&[
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        SUBMODULE_COMMITS_ONLY,
        "--shortstat",
        "HEAD",
    ])?);
    Ok(GitStatus {
        files_changed,
        insertions,
        deletions,
        head_commit,
    })
}

/// Changed paths (every entry line; renames are one line, paths are quoted) and the commit (`# branch.oid`, empty on a branch with no
/// commit yet, where the `diff HEAD` that follows fails anyway) from `git status --porcelain=v2 --branch`.
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

/// Most untracked files a diff shows, and the most bytes of one it reads.
const MAX_UNTRACKED: usize = 50;
const MAX_UNTRACKED_BYTES: u64 = 64 * 1024;

/// The most diff output ever held in memory; what is beyond it is dropped as it is read.
const MAX_DIFF_READ: u64 = 1024 * 1024;

/// Like [`git`], keeping at most `cap` bytes of stdout (the process is stopped when it has produced that much).
fn git_capped(dir: &Path, args: &[&str], cap: u64) -> Result<String, GitError> {
    use std::io::Read;
    let mut child = hardened(dir)
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
    if buf.len() as u64 >= cap || status.success() {
        Ok(String::from_utf8_lossy(&buf).into_owned())
    } else {
        Err(GitError::Command {
            args: args.join(" "),
            stderr: String::new(),
        })
    }
}

/// Unified diff of the changes against HEAD, optionally for one path. A new file that git does not track yet is shown as
/// all-added (what a reviewer would see), without touching the index.
pub fn diff(worktree: &Path, path: Option<&str>) -> Result<String, GitError> {
    let mut args = vec![
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        SUBMODULE_COMMITS_ONLY,
        "HEAD",
    ];
    if let Some(p) = path {
        args.extend(["--", p]);
    }
    let mut out = git_capped(worktree, &args, MAX_DIFF_READ)?;
    let mut ls = vec!["ls-files", "--others", "--exclude-standard", "-z"];
    if let Some(p) = path {
        ls.extend(["--", p]);
    }
    let listed = git(worktree, &ls)?;
    let names: Vec<&str> = listed
        .split('\0')
        .filter(|n| !n.is_empty())
        .take(MAX_UNTRACKED)
        .collect();
    if !names.is_empty() {
        out.push_str(&untracked_diff(worktree, &names)?);
    }
    Ok(out)
}

/// The diff of each untracked file against nothing, built here rather than by `git diff --no-index`. git given a path in the
/// worktree opens it itself, and the agent working there can swap a folder above it for a link between `ls-files` and that open
/// (measured: git follows a linked folder, though not a link in the file's own place), so the node would show a file from
/// anywhere; a copy on disk to diff instead is one more place to race (a shared temporary folder is the agent's to write on
/// Linux). Each file is read the way `files.read` reads it ([`crate::files::Files::untracked`]: checked on the open handle,
/// regular files only, at most [`MAX_UNTRACKED_BYTES`]; a link shows as a link), and the text is git's own for a new file
/// ([`new_file_diff`]). Files that are not plain, too large, or lead out of the worktree are left out, as before.
fn untracked_diff(worktree: &Path, names: &[&str]) -> Result<String, GitError> {
    let files = crate::files::Files::open(worktree, MAX_UNTRACKED_BYTES)
        .map_err(|e| GitError::Io(format!("{e:?}")))?;
    let mut out = Vec::new();
    for name in names {
        let (mode, data) = match files.untracked(name) {
            Ok(crate::files::Untracked::File { data, executable }) => {
                (if executable { "100755" } else { "100644" }, data)
            }
            Ok(crate::files::Untracked::Link(target)) => ("120000", target),
            Err(_) => continue,
        };
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
    if data[..data.len().min(8000)].contains(&0) {
        let _ = writeln!(out, "Binary files /dev/null and {b} differ");
        return out;
    }
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
    if git(worktree, &["rev-parse", "--verify", "-q", "HEAD"]).is_err() {
        return Ok(Vec::new());
    }
    let n = format!("-n{limit}");
    // Fields are split by the ASCII unit separator and records end with the record separator: neither can occur in a
    // subject or a name, so no quoting is needed.
    let out = git(
        worktree,
        &[
            "log",
            &n,
            "--no-color",
            "--format=%H%x1f%an%x1f%ct%x1f%s%x1e",
        ],
    )?;
    Ok(out
        .split('\u{1e}')
        .filter_map(|rec| {
            let mut f = rec.trim_start_matches('\n').splitn(4, '\u{1f}');
            let (id, author, ct, subject) = (f.next()?, f.next()?, f.next()?, f.next()?);
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
    let Ok(out) = hardened(worktree)
        .args(["rev-parse", "--git-path", "info/exclude"])
        .output()
    else {
        return;
    };
    if !out.status.success() {
        return;
    }
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
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
            if let Some(url) = remote {
                r.git(&["remote", "add", "origin", url]);
            }
            fs::write(r.dir.join("a.txt"), "one\n").expect("write");
            r.commit("first");
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
        assert_eq!(info.project.remote(), Some("github.com/Owner/Repo"));
        assert_eq!(info.project.root_commit().len(), 40);
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
        for b in ["", "-x", "a..b", "a b"] {
            assert!(
                matches!(
                    create_worktree(&r.dir, &r.wt("w1"), b, None),
                    Err(GitError::InvalidBranch(_))
                ),
                "{b}"
            );
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
        fs::write(admin.join("commondir"), format!("{}\n", fake.display())).expect("commondir");
        fs::write(wt.join(".gitattributes"), "*.txt filter=ev\n").expect("attributes");
        std::thread::sleep(std::time::Duration::from_millis(1100));
        // Keep the checkout's own line ending (`core.autocrlf` writes `one\r\n` on Windows): a size change lets git call the
        // file modified without hashing it, and the clean filter would never run.
        let checked_out = fs::read_to_string(wt.join("a.txt")).expect("read");
        fs::write(wt.join("a.txt"), checked_out.replace("one", "uno")).expect("edit");

        // Pinned: the node's git uses the real shared folder, not the one `commondir` names. The filter blanking alone would also
        // stop this plant, but not one added between its question and the command.
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
        status(&wt).expect("pinned status");
        diff(&wt, None).expect("pinned diff");
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
        };
        made().extend([entry(&outer, "outer"), entry(&inner, "inner")]);
        let found = made_dirs(&inner.join("deep")).map(|(admin, ..)| admin);
        let at_outer = made_dirs(&outer).map(|(admin, ..)| admin);
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
}

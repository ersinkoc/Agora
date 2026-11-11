//! Git operations on a node: checkout discovery, worktrees, status and diff (ADR-0003).
//!
//! Uses the `git` command line (stable, ubiquitous, no extra dependency). Every call runs with a clean locale,
//! no prompts, and paths/branches after `--` or validated first, so repository content cannot inject options.

use std::path::{Path, PathBuf};
use std::process::Command;

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
/// Still open: filter drivers (`.gitattributes` + `filter.<name>.*`), which cannot be switched off by name here.
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

/// `git -C dir` with [`HARDENED`] and the node's environment (no prompts, no optional locks, plain English output).
pub(crate) fn hardened(dir: &Path) -> Command {
    let mut c = Command::new("git");
    c.args(HARDENED)
        .arg("-C")
        .arg(dir)
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0");
    c
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
    let root = PathBuf::from(root.trim());
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
    git(checkout, &args).map(|_| ())
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

fn is_dirty(worktree: &Path) -> Result<bool, GitError> {
    Ok(!git(worktree, &["status", "--porcelain"])?.trim().is_empty())
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
    git(checkout, &args).map(|_| ())
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
    let files_changed = git(worktree, &["status", "--porcelain"])?.lines().count() as u32;
    let head_commit = git(worktree, &["rev-parse", "HEAD"])?.trim().to_string();
    let (insertions, deletions) = parse_shortstat(&git(
        worktree,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--shortstat",
            "HEAD",
        ],
    )?);
    Ok(GitStatus {
        files_changed,
        insertions,
        deletions,
        head_commit,
    })
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
    for name in git(worktree, &ls)?
        .split('\0')
        .filter(|n| !n.is_empty())
        .take(MAX_UNTRACKED)
    {
        let small = std::fs::metadata(worktree.join(name))
            .is_ok_and(|m| m.is_file() && m.len() <= MAX_UNTRACKED_BYTES);
        if !small {
            continue;
        }
        // `--no-index` exits 1 when the files differ, which is the normal case here.
        let shown = hardened(worktree)
            .args([
                "diff",
                "--no-index",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--",
                "/dev/null",
                name,
            ])
            .output()
            .map_err(|e| GitError::Io(e.to_string()))?;
        out.push_str(&String::from_utf8_lossy(&shown.stdout));
    }
    Ok(out)
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
        assert_eq!(clean.head_commit.len(), 40);
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
        fs::write(r.dir.join(".gitattributes"), "*.txt diff=conv\n").expect("attributes");
        let hook = r.dir.join(".git/hooks/post-checkout");
        fs::write(
            &hook,
            format!("#!/bin/sh\ntouch \"{}\"\n", mark("hook").display()),
        )
        .expect("hook");
        let _ = Command::new("chmod").arg("+x").arg(&hook).status();
        fs::write(r.dir.join("a.txt"), "two\n").expect("change");
        let planted = ["fsmonitor", "external", "textconv", "hook"];

        // Plain git, as an unhardened node would run it: which plants work on this system.
        let plain = |args: &[&str]| {
            let _ = Command::new("git")
                .arg("-C")
                .arg(&r.dir)
                .args(args)
                .output();
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
            effective.contains(&"fsmonitor"),
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

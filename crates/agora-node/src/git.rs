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

fn git(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
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
    let (insertions, deletions) =
        parse_shortstat(&git(worktree, &["diff", "--shortstat", "HEAD"])?);
    Ok(GitStatus {
        files_changed,
        insertions,
        deletions,
        head_commit,
    })
}

/// Unified diff of tracked changes against HEAD, optionally for one path.
pub fn diff(worktree: &Path, path: Option<&str>) -> Result<String, GitError> {
    let mut args = vec!["diff", "--no-color", "--no-ext-diff", "HEAD"];
    if let Some(p) = path {
        args.extend(["--", p]);
    }
    git(worktree, &args)
}

#[cfg(test)]
#[allow(clippy::expect_used)] // expect is allowed in tests
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

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
            // Canonicalize: macOS temp dirs are symlinks and git reports real paths.
            fs::create_dir_all(&dir).expect("mkdir");
            let dir = dir.canonicalize().expect("canonicalize");
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
}

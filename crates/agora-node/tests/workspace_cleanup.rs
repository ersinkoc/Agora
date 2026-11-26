//! ADR-0064: the node's workspace clean-up against real Git repositories in temporary folders. Work is never removed (uncommitted
//! changes, untracked files, commits on no other branch, a detached HEAD's commits), a folder that is not the recorded worktree (a link
//! swapped in, rewritten Git links) is never acted on, the grace period is honoured on an injected clock, and only build output git
//! ignores goes from a workspace left behind.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use agora_node::git;
use agora_node::workspaces::{Action, Clean, Reason, State, Users, Workspaces};

const H: u64 = 60 * 60 * 1000;
const T0: u64 = 1_000 * H;

fn run_git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "commit.gpgsign=false", "-c", "core.autocrlf=false"])
        .args(args)
        .env("GIT_AUTHOR_NAME", "fixture")
        .env("GIT_COMMITTER_NAME", "fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A checkout with one commit, a worktree folder and a node data folder, all in one temporary folder; and one worktree the node made
/// and recorded.
struct Rig {
    top: PathBuf,
    checkout: PathBuf,
    root: PathBuf,
    data: PathBuf,
    work: PathBuf,
    branch: String,
    ws: Workspaces,
}

impl Rig {
    fn new(name: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let top = std::env::temp_dir().join(format!(
            "agora-ws-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&top);
        let checkout = top.join("repo");
        fs::create_dir_all(&checkout).unwrap();
        run_git(&checkout, &["init", "-q", "-b", "main"]);
        fs::write(checkout.join("a.txt"), "a\n").unwrap();
        fs::write(checkout.join(".gitignore"), "/target/\n/node_modules/\n").unwrap();
        run_git(&checkout, &["add", "."]);
        run_git(&checkout, &["commit", "-q", "-m", "first"]);
        let root = top.join("worktrees");
        fs::create_dir_all(&root).unwrap();
        let data = top.join("data");
        fs::create_dir_all(&data).unwrap();
        let checkout = agora_platform::canonicalize_plain(&checkout).unwrap();
        let root = agora_platform::canonicalize_plain(&root).unwrap();
        let work = root.join("claude-1");
        let branch = "agent/claude-1".to_string();
        git::create_worktree(&checkout, &work, &branch, None).unwrap();
        let ws = Workspaces::open(&data, &root).unwrap();
        assert!(ws.record_made(&work, &checkout, &branch, T0));
        Self {
            top,
            checkout,
            root,
            data,
            work,
            branch,
            ws,
        }
    }

    /// The agent that worked there ended at `at`.
    fn ended(&self, at: u64) -> Users {
        let mut u = Users::default();
        u.ended(&self.work, at);
        u
    }

    fn clean(&self, users: &Users, now: u64, by_node: bool) -> Vec<Action> {
        let never = AtomicBool::new(false);
        self.ws
            .clean(
                &|| users.clone(),
                now,
                Clean {
                    by_node,
                    older_than_ms: None,
                    dry_run: false,
                },
                &never,
            )
            .into_iter()
            .map(|o| o.action)
            .collect()
    }

    fn branch_exists(&self) -> bool {
        Command::new("git")
            .arg("-C")
            .arg(&self.checkout)
            .args(["show-ref", "--verify", "--quiet"])
            .arg(format!("refs/heads/{}", self.branch))
            .status()
            .unwrap()
            .success()
    }

    fn log(&self) -> String {
        fs::read_to_string(self.data.join("workspaces").join("workspaces.log")).unwrap_or_default()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.top);
    }
}

fn left_behind(r: Reason) -> Action {
    Action::Kept(State::LeftBehind(r))
}

#[test]
fn a_clean_workspace_goes_only_after_the_grace_period_and_keeps_its_branch() {
    let rig = Rig::new("grace");
    let users = rig.ended(T0);
    // First seen free at T0: the 24 hours run from there.
    assert_eq!(
        rig.clean(&users, T0, true),
        vec![Action::Kept(State::Waiting {
            until_ms: T0 + 24 * H
        })]
    );
    assert_eq!(
        rig.clean(&users, T0 + 23 * H, true),
        vec![Action::Kept(State::Waiting {
            until_ms: T0 + 24 * H
        })]
    );
    assert!(rig.work.is_dir());
    assert_eq!(rig.clean(&users, T0 + 24 * H, true), vec![Action::Removed]);
    assert!(!rig.work.exists());
    assert!(rig.branch_exists(), "a removal never deletes the branch");
    assert!(rig.log().contains("\tremoved\tnode\t"), "{}", rig.log());
    assert!(rig.ws.records().is_empty());
}

#[test]
fn an_agent_that_ended_later_starts_the_grace_period_again() {
    let rig = Rig::new("later-end");
    rig.clean(&rig.ended(T0), T0, true);
    // It was resumed and ended again at T0 + 20 h.
    let users = rig.ended(T0 + 20 * H);
    assert_eq!(
        rig.clean(&users, T0 + 30 * H, true),
        vec![Action::Kept(State::Waiting {
            until_ms: T0 + 44 * H
        })]
    );
    assert_eq!(rig.clean(&users, T0 + 44 * H, true), vec![Action::Removed]);
}

#[test]
fn a_workspace_in_use_is_never_removed() {
    let rig = Rig::new("in-use");
    let mut users = Users::default();
    // A helper working in a folder below the worktree keeps it in use.
    fs::create_dir_all(rig.work.join("sub")).unwrap();
    users.busy(&rig.work.join("sub"));
    for by_node in [true, false] {
        assert_eq!(
            rig.clean(&users, T0 + 1000 * H, by_node),
            vec![Action::Kept(State::InUse)]
        );
    }
    assert!(rig.work.is_dir());
    assert_eq!(
        rig.clean(&Users::unknown(), T0 + 1000 * H, false),
        vec![Action::Kept(State::InUse)]
    );
}

#[test]
fn uncommitted_and_untracked_work_is_never_removed() {
    let rig = Rig::new("dirty");
    fs::write(rig.work.join("new.txt"), "untracked\n").unwrap();
    let users = rig.ended(T0);
    for by_node in [true, false] {
        assert_eq!(
            rig.clean(&users, T0 + 1000 * H, by_node),
            vec![left_behind(Reason::Changes)]
        );
    }
    fs::remove_file(rig.work.join("new.txt")).unwrap();
    fs::write(rig.work.join("a.txt"), "edited\n").unwrap();
    assert_eq!(
        rig.clean(&users, T0 + 1000 * H, false),
        vec![left_behind(Reason::Changes)]
    );
    assert_eq!(
        fs::read_to_string(rig.work.join("a.txt")).unwrap(),
        "edited\n"
    );
}

#[test]
fn commits_that_exist_nowhere_else_are_never_removed_until_pushed() {
    let rig = Rig::new("unpushed");
    fs::write(rig.work.join("b.txt"), "b\n").unwrap();
    run_git(&rig.work, &["add", "b.txt"]);
    run_git(&rig.work, &["commit", "-q", "-m", "work"]);
    let users = rig.ended(T0);
    assert_eq!(
        rig.clean(&users, T0 + 1000 * H, false),
        vec![left_behind(Reason::Unmerged)]
    );
    assert!(rig.work.join("b.txt").is_file());
    // Pushed to a remote: the commits exist elsewhere, and it may go.
    let remote = rig.top.join("remote.git");
    run_git(&rig.top, &["init", "-q", "--bare", "remote.git"]);
    run_git(
        &rig.checkout,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    run_git(&rig.checkout, &["push", "-q", "origin", &rig.branch]);
    assert_eq!(
        rig.clean(&users, T0 + 1000 * H, false),
        vec![Action::Removed]
    );
    assert!(rig.log().contains("\tremoved\tcli\t"), "{}", rig.log());
}

#[test]
fn a_detached_heads_own_commits_are_never_removed() {
    let rig = Rig::new("detached");
    run_git(&rig.work, &["checkout", "-q", "--detach"]);
    fs::write(rig.work.join("c.txt"), "c\n").unwrap();
    run_git(&rig.work, &["add", "c.txt"]);
    run_git(&rig.work, &["commit", "-q", "-m", "detached"]);
    assert_eq!(
        rig.clean(&rig.ended(T0), T0 + 1000 * H, false),
        vec![left_behind(Reason::Unmerged)]
    );
    assert!(rig.work.join("c.txt").is_file());
}

#[cfg(unix)]
#[test]
fn a_link_swapped_in_for_the_worktree_is_never_followed() {
    let rig = Rig::new("swap");
    // Someone's own folder, clean in git's eyes: a checkout of the same repository with nothing changed.
    let victim = rig.top.join("victim");
    fs::create_dir_all(&victim).unwrap();
    fs::write(victim.join("precious.txt"), "keep me\n").unwrap();
    let aside = rig.top.join("aside");
    fs::rename(&rig.work, &aside).unwrap();
    std::os::unix::fs::symlink(&victim, &rig.work).unwrap();
    for by_node in [true, false] {
        assert_eq!(
            rig.clean(&rig.ended(T0), T0 + 1000 * H, by_node),
            vec![left_behind(Reason::Moved)]
        );
    }
    assert_eq!(
        fs::read_to_string(victim.join("precious.txt")).unwrap(),
        "keep me\n"
    );
    // A real folder put in its place instead: at the recorded place, but without the links git wrote.
    fs::remove_file(&rig.work).unwrap();
    fs::create_dir(&rig.work).unwrap();
    fs::write(rig.work.join("precious.txt"), "keep me too\n").unwrap();
    assert_eq!(
        rig.clean(&rig.ended(T0), T0 + 1000 * H, false),
        vec![left_behind(Reason::LinksMoved)]
    );
    assert!(rig.work.join("precious.txt").is_file());
}

/// A second repository in which the worktree's branch would look merged and clean.
fn decoy(rig: &Rig) -> PathBuf {
    let decoy = rig.top.join("decoy");
    fs::create_dir_all(&decoy).unwrap();
    run_git(&decoy, &["init", "-q", "-b", "main"]);
    fs::write(decoy.join("x.txt"), "x\n").unwrap();
    run_git(&decoy, &["add", "x.txt"]);
    run_git(&decoy, &["commit", "-q", "-m", "decoy"]);
    agora_platform::canonicalize_plain(&decoy.join(".git")).unwrap()
}

#[test]
fn a_rewritten_commondir_cannot_redirect_the_checks() {
    let rig = Rig::new("commondir");
    // Work the agent wants gone with the worktree: a commit on its branch only.
    fs::write(rig.work.join("b.txt"), "b\n").unwrap();
    run_git(&rig.work, &["add", "b.txt"]);
    run_git(&rig.work, &["commit", "-q", "-m", "work"]);
    let admin = rig.checkout.join(".git/worktrees/claude-1");
    let decoy = decoy(&rig);
    fs::write(admin.join("commondir"), format!("{}\n", decoy.display())).unwrap();
    for by_node in [true, false] {
        assert_eq!(
            rig.clean(&rig.ended(T0), T0 + 1000 * H, by_node),
            vec![left_behind(Reason::LinksMoved)]
        );
    }
    assert!(rig.work.join("b.txt").is_file());
}

#[test]
fn a_rewritten_dot_git_file_cannot_redirect_the_checks() {
    let rig = Rig::new("dotgit");
    fs::write(rig.work.join("new.txt"), "work\n").unwrap();
    let decoy = decoy(&rig);
    fs::write(
        rig.work.join(".git"),
        format!("gitdir: {}\n", decoy.display()),
    )
    .unwrap();
    assert_eq!(
        rig.clean(&rig.ended(T0), T0 + 1000 * H, false),
        vec![left_behind(Reason::LinksMoved)]
    );
    assert!(rig.work.join("new.txt").is_file());
}

#[test]
fn after_a_restart_the_record_still_decides_and_a_tampered_one_still_refuses() {
    let rig = Rig::new("restart");
    // A node that restarts has no record in memory of the worktrees it made.
    git::forget_made_worktree(&rig.work);
    let reopened = Workspaces::open(&rig.data, &rig.root).unwrap();
    assert_eq!(reopened.records().len(), 1);
    let gitdir = rig.checkout.join(".git/worktrees/claude-1/gitdir");
    let original = fs::read(&gitdir).unwrap();
    fs::write(&gitdir, b"/elsewhere/.git\n").unwrap();
    assert_eq!(
        rig.clean(&rig.ended(T0), T0 + 1000 * H, false),
        vec![left_behind(Reason::LinksMoved)]
    );
    fs::write(&gitdir, &original).unwrap();
    assert_eq!(
        rig.clean(&rig.ended(T0), T0 + 1000 * H, false),
        vec![Action::Removed]
    );
    assert!(!rig.work.exists());
}

#[test]
fn only_worktrees_the_node_made_in_its_worktree_folder_are_recorded() {
    let rig = Rig::new("record");
    // A worktree made by hand (not by the node): nothing recorded, so never cleaned.
    let by_hand = rig.root.join("by-hand");
    run_git(
        &rig.checkout,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "mine",
            by_hand.to_str().unwrap(),
        ],
    );
    assert!(!rig.ws.record_made(&by_hand, &rig.checkout, "mine", T0));
    // One the node made outside its worktree folder: not recorded either.
    let outside = rig.top.join("outside");
    git::create_worktree(&rig.checkout, &outside, "agent/outside", None).unwrap();
    assert!(!rig
        .ws
        .record_made(&outside, &rig.checkout, "agent/outside", T0));
    assert_eq!(rig.ws.records().len(), 1);
    rig.clean(&Users::default(), T0 + 1000 * H, false);
    assert!(by_hand.is_dir() && outside.is_dir());
}

#[test]
fn a_claimed_workspace_refuses_starts_and_is_not_acted_on_twice() {
    let rig = Rig::new("claim");
    let dir = rig.data.join("workspaces");
    let record = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|x| x == "json"))
        .unwrap();
    let claim = record.with_extension("claim");
    assert!(!rig.ws.removing(&rig.work));
    // Another process claimed it.
    fs::rename(&record, &claim).unwrap();
    assert!(rig.ws.removing(&rig.work));
    assert!(rig.ws.removing(&rig.work.join("sub")));
    assert!(!rig.ws.removing(&rig.top.join("elsewhere")));
    assert_eq!(rig.clean(&rig.ended(T0), T0 + 1000 * H, false), vec![]);
    assert!(rig.work.is_dir());
    // A claim left by a process that ended mid-way is given back after an hour.
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 60 * 60);
    fs::File::options()
        .write(true)
        .open(&claim)
        .unwrap()
        .set_modified(old)
        .unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    assert_eq!(rig.clean(&rig.ended(T0), now, false), vec![Action::Removed]);
}

#[test]
fn a_dry_run_changes_nothing() {
    let rig = Rig::new("dry");
    let never = AtomicBool::new(false);
    let users = rig.ended(T0);
    let out = rig.ws.clean(
        &|| users.clone(),
        T0 + 1000 * H,
        Clean {
            by_node: false,
            older_than_ms: None,
            dry_run: true,
        },
        &never,
    );
    assert_eq!(out[0].action, Action::WouldRemove);
    assert!(rig.work.is_dir());
    assert!(rig.log().is_empty());
}

#[test]
fn older_than_keeps_workspaces_freed_more_recently() {
    let rig = Rig::new("older");
    let never = AtomicBool::new(false);
    let users = rig.ended(T0);
    let how = |days: u64| Clean {
        by_node: false,
        older_than_ms: Some(days * 24 * H),
        dry_run: false,
    };
    let out = rig
        .ws
        .clean(&|| users.clone(), T0 + 3 * 24 * H, how(7), &never);
    assert!(matches!(out[0].action, Action::Kept(State::Waiting { .. })));
    let out = rig
        .ws
        .clean(&|| users.clone(), T0 + 8 * 24 * H, how(7), &never);
    assert_eq!(out[0].action, Action::Removed);
}

#[test]
fn listing_tells_each_state_and_reason() {
    let rig = Rig::new("list");
    let users = rig.ended(T0);
    let listed = rig.ws.list(&users, T0 + H, true);
    assert_eq!(listed.len(), 1);
    assert!(matches!(listed[0].state, State::Waiting { .. }));
    assert_eq!(listed[0].branch, rig.branch);
    assert_eq!(
        rig.ws.list(&users, T0 + H, false)[0].state,
        State::Cleanable
    );
    fs::write(rig.work.join("new.txt"), "x").unwrap();
    assert_eq!(
        rig.ws.list(&users, T0 + H, false)[0].state,
        State::LeftBehind(Reason::Changes)
    );
    let mut busy = Users::default();
    busy.busy(&rig.work);
    assert_eq!(rig.ws.list(&busy, T0 + H, false)[0].state, State::InUse);
    // Listing changed nothing.
    assert!(rig.work.join("new.txt").is_file());
}

#[test]
fn a_workspace_already_gone_is_forgotten() {
    let rig = Rig::new("gone");
    run_git(
        &rig.checkout,
        &["worktree", "remove", "--force", rig.work.to_str().unwrap()],
    );
    assert_eq!(rig.clean(&rig.ended(T0), T0, true), vec![Action::Forgotten]);
    assert!(rig.ws.records().is_empty());
}

#[test]
fn build_output_git_ignores_goes_from_a_workspace_left_behind_after_the_grace_period() {
    let rig = Rig::new("build");
    fs::write(rig.work.join("Cargo.toml"), "[package]\n").unwrap();
    fs::create_dir_all(rig.work.join("target/debug")).unwrap();
    fs::write(rig.work.join("target/debug/big"), vec![0u8; 4096]).unwrap();
    // No package.json: node_modules is not build output here.
    fs::create_dir_all(rig.work.join("node_modules")).unwrap();
    fs::write(rig.work.join("node_modules/x"), "x").unwrap();
    // Work that keeps the workspace: an uncommitted file.
    fs::write(rig.work.join("notes.txt"), "mine\n").unwrap();
    let users = rig.ended(T0);
    // Within the grace period nothing goes.
    rig.clean(&users, T0, true);
    assert!(rig.work.join("target").is_dir());
    assert_eq!(
        rig.clean(&users, T0 + 24 * H, true),
        vec![Action::BuildOutputRemoved(vec!["target"])]
    );
    assert!(!rig.work.join("target").exists());
    assert!(rig.work.join("node_modules/x").is_file());
    assert!(rig.work.join("notes.txt").is_file());
    assert!(rig.work.join("Cargo.toml").is_file());
    assert!(rig.log().contains("build-output-removed:target"));
    // Once: the next pass leaves it be.
    fs::create_dir_all(rig.work.join("target")).unwrap();
    assert_eq!(
        rig.clean(&users, T0 + 48 * H, true),
        vec![left_behind(Reason::Changes)]
    );
    assert!(rig.work.join("target").is_dir());
}

#[test]
fn build_output_that_is_tracked_or_a_link_is_kept() {
    let rig = Rig::new("build-tracked");
    fs::write(rig.work.join("package.json"), "{}\n").unwrap();
    fs::create_dir_all(rig.work.join("node_modules")).unwrap();
    fs::write(rig.work.join("node_modules/vendored.js"), "x").unwrap();
    // Tracked despite the ignore rule: it is the project's, not build output.
    run_git(
        &rig.work,
        &["add", "-f", "package.json", "node_modules/vendored.js"],
    );
    run_git(&rig.work, &["commit", "-q", "-m", "vendored"]);
    fs::write(rig.work.join("Cargo.toml"), "[package]\n").unwrap();
    let elsewhere = rig.top.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::write(elsewhere.join("keep.txt"), "keep\n").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&elsewhere, rig.work.join("target")).unwrap();
    let users = rig.ended(T0);
    rig.clean(&users, T0, true);
    let out = rig.clean(&users, T0 + 24 * H, true);
    assert!(
        matches!(out[0], Action::Kept(State::LeftBehind(_))),
        "{out:?}"
    );
    assert!(rig.work.join("node_modules/vendored.js").is_file());
    assert!(elsewhere.join("keep.txt").is_file());
}

#[test]
fn the_cap_and_free_disk_refuse_a_new_worktree() {
    let rig = Rig::new("cap");
    fs::write(rig.work.join("big.bin"), vec![0u8; 8192]).unwrap();
    let mut busy = Users::default();
    busy.busy(&rig.work);
    // The node's pass measures the workspaces (also those in use).
    rig.clean(&busy, T0, true);
    let mut s = rig.ws.settings();
    assert_eq!(rig.ws.room(), Ok(()));
    s.max_bytes = 4096;
    rig.ws.set_settings(s);
    assert_eq!(rig.ws.room(), Err(("workspaces_gb", 0)));
    assert!(rig.ws.take_wake(), "a refusal wakes the clean-up");
    s.max_bytes = 0;
    s.min_free_bytes = u64::MAX;
    rig.ws.set_settings(s);
    assert!(matches!(rig.ws.room(), Err(("free_disk_gb", _))));
}

#[test]
fn a_stale_claim_beside_a_newer_record_goes_and_no_longer_refuses_starts() {
    let rig = Rig::new("claim-twice");
    let dir = rig.data.join("workspaces");
    let record = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|x| x == "json"))
        .unwrap();
    let claim = record.with_extension("claim");
    // A process claimed it and ended mid-way, and the other one's pass wrote the record again meanwhile.
    fs::copy(&record, &claim).unwrap();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 60 * 60);
    fs::File::options()
        .write(true)
        .open(&claim)
        .unwrap()
        .set_modified(old)
        .unwrap();
    assert!(rig.ws.removing(&rig.work));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let mut busy = Users::default();
    busy.busy(&rig.work);
    assert_eq!(
        rig.clean(&busy, now, false),
        vec![Action::Kept(State::InUse)]
    );
    assert!(!claim.exists());
    assert!(!rig.ws.removing(&rig.work));
}

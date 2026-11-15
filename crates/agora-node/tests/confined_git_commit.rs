//! An agent confined folder by folder (ADR-0050, the Windows profile) can commit in its own worktree: `git add` and `git commit` need
//! the worktree, its admin folder, `objects`, `refs` and the branch reflog under `logs/refs`, and nothing else of the main `.git`.
//! Only where the platform grants permissions folder by folder; elsewhere the test returns at once.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use agora_adapters::Manifest;
use agora_core::NodeId;
use agora_node::sandbox::{Level, SandboxPolicy};
use agora_node::supervisor::{
    NodeLimits, RestartPolicy, StartRequest, Supervisor, SupervisorConfig, SupervisorError,
};
use agora_node::terminal::TerminalOptions;

/// Every file and folder below `root` (links not followed).
fn walk(root: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(p) = stack.pop() {
        let Ok(meta) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        if meta.is_dir() && !meta.file_type().is_symlink() {
            if let Ok(entries) = std::fs::read_dir(&p) {
                stack.extend(entries.flatten().map(|e| e.path()));
            }
        }
        out.push(p);
    }
    out
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_confined_agent_commits_in_its_own_worktree() {
    if !agora_platform::NARROW_GIT_WRITES {
        return;
    }
    // The confinement is off by default until the maintainer switches it on; this test switches it on for its own process.
    std::env::set_var("AGORA_WINDOWS_CONFINEMENT", "1");
    if !agora_platform::sandbox_supported() {
        eprintln!("skipped: agents cannot be confined on this machine");
        return;
    }
    // Not under the user's temp folder: git `stat`s every folder above a ref it locks, and folders of the user profile refuse that to a
    // confined agent, so a checkout inside the profile cannot commit yet (reported separately; this test covers the reflog only).
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("agora-confined-commit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    // The target folder lies inside this repository's own checkout: git (the test's, the supervisor's and the agent's) must never
    // search up into it.
    std::env::set_var("GIT_CEILING_DIRECTORIES", env!("CARGO_TARGET_TMPDIR"));
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);

    // The identity is given on the command line: the user's global git config is outside what the agent may read.
    let script = "echo agent> b.txt && git add b.txt 2> add-err.txt && \
                  git -c user.name=agent -c user.email=agent@example.com commit -q -m from-agent 2> commit-err.txt && \
                  echo ok> committed.txt";
    let manifest = Manifest::parse(&format!(
        "kind = \"committer\"\ndisplay_name = \"Committer\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"cmd.exe\", \"/c\", {script:?}]\n"
    ))
    .unwrap();
    let mut sup = Supervisor::new(SupervisorConfig {
        node: NodeId::new("node-1"),
        restart: RestartPolicy::default(),
        terminal: TerminalOptions::default(),
        limits: NodeLimits::default(),
    });
    sup.register(manifest);
    sup.set_sandbox(SandboxPolicy {
        level: Level::Worktree,
        ..Default::default()
    });
    let req = StartRequest {
        kind: "committer".into(),
        checkout: repo.clone(),
        worktree_root: dir.join("worktrees"),
        name: "a".into(),
        base: None,
    };
    match sup.start(&req, Instant::now()) {
        Ok(_) => {}
        // A target folder where every user may create files (the inherited `Users` entries of a volume root, as on CI runners), or whose
        // label the user cannot set (a data volume's default ACL), is refused before start (ADR-0050 Amendment B, decisions 2 and 4), and
        // the refused start leaves no worktree. Do what the recipe says (keep only the user, with Full control, and SYSTEM) and start again.
        Err(SupervisorError::SandboxRefused(
            _,
            _,
            why @ ("broad_write_grant" | "label_failed"),
        )) => {
            eprintln!("refused before start ({why}); tightening the test folder");
            let who = Command::new("whoami").output().unwrap();
            let who = String::from_utf8_lossy(&who.stdout).trim().to_string();
            // On the folder only: Windows passes the new inherited entries down to everything below, which holds inherited ones only.
            let out = Command::new("icacls")
                .arg(&dir)
                .args(["/inheritance:r", "/grant:r"])
                .arg(format!("{who}:(OI)(CI)F"))
                .args(["/grant:r", "*S-1-5-18:(OI)(CI)F", "/Q"])
                .output()
                .unwrap();
            assert!(out.status.success(), "icacls: {out:?}");
            // The test repository must still be the one git finds (never a checkout above it).
            let top = git(&repo, &["rev-parse", "--show-toplevel"]);
            assert!(
                top.trim().ends_with(&format!(
                    "agora-confined-commit-{}/repo",
                    std::process::id()
                )),
                "git finds {top}"
            );
            // A refused start rolls back its unused branch, so tightening the folder permits a direct retry.
            assert!(git(&repo, &["branch", "--list", "agent/a"])
                .trim()
                .is_empty());
            sup.start(&req, Instant::now()).unwrap();
        }
        Err(e) => panic!("start: {e:?}"),
    }

    let worktree = dir.join("worktrees").join("a");
    let done = worktree.join("committed.txt");
    let end = Instant::now() + Duration::from_secs(30);
    while !done.exists() && Instant::now() < end {
        sup.tick(Instant::now());
        std::thread::sleep(Duration::from_millis(100));
    }
    let errors = ["add-err.txt", "commit-err.txt"]
        .iter()
        .map(|f| std::fs::read_to_string(worktree.join(f)).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        done.exists(),
        "the confined agent could not commit: {errors}"
    );
    assert_eq!(
        git(&worktree, &["log", "-1", "--format=%s"]).trim(),
        "from-agent"
    );
    // The main checkout's own state is untouched: its branch still points at the first commit.
    assert_eq!(
        git(&repo, &["log", "-1", "--format=%s", "main"]).trim(),
        "init"
    );
    drop(sup);
    // ADR-0050 Amendment B: once the agent has ended, no Low label is left anywhere below (labels taken back, and the explicit ones
    // Windows gives what a Low process creates cleared too).
    let labelled: Vec<String> = walk(&dir)
        .into_iter()
        .filter(|p| {
            let out = Command::new("icacls").arg(p).output().unwrap();
            String::from_utf8_lossy(&out.stdout).contains("Mandatory Label")
        })
        .map(|p| p.display().to_string())
        .collect();
    assert!(labelled.is_empty(), "left labelled: {labelled:?}");
    // git makes its objects read-only, and `remove_dir_all` cannot delete read-only files on Windows.
    for p in walk(&dir) {
        if let Ok(meta) = std::fs::metadata(&p) {
            let mut perm = meta.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            perm.set_readonly(false);
            let _ = std::fs::set_permissions(&p, perm);
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

//! Node-owned Git operations must not execute repository-configured inspection helpers or hooks.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use agora_node::git;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU32, Ordering},
};

struct Repo {
    root: PathBuf,
}
impl Repo {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "agora-git-helpers-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let repo = Self { root };
        repo.raw(&["init", "-q", "-b", "main"]);
        // Keep content assertions independent of the runner's global checkout conversion (Windows defaults to CRLF).
        repo.raw(&["config", "core.autocrlf", "false"]);
        fs::write(repo.root.join("a.txt"), "original\n").unwrap();
        repo.raw(&["add", "a.txt"]);
        repo.raw(&[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "fixture",
        ]);
        repo
    }
    fn raw(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(args)
            .env("GIT_AUTHOR_NAME", "fixture")
            .env("GIT_COMMITTER_NAME", "fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }
    fn helper(&self, name: &str) -> (String, PathBuf) {
        let dir = self.root.join(".git/helpers");
        fs::create_dir_all(&dir).unwrap();
        let script = dir.join(name);
        let marker = dir.join(format!("{name}.ran"));
        fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf executed > {}\nprintf 'token\\000'\n",
                quote(&marker)
            ),
        )
        .unwrap();
        agora_platform::set_executable(&script, true).unwrap();
        (quote(&script), marker)
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn quote(path: &Path) -> String {
    format!(
        "'{}'",
        path.to_string_lossy()
            .replace('\\', "/")
            .replace('\'', "'\\''")
    )
}
fn reset(marker: &Path) {
    assert!(
        marker.exists(),
        "unsafe control did not execute helper: {marker:?}"
    );
    fs::remove_file(marker).unwrap();
}

#[test]
fn status_does_not_execute_a_planted_fsmonitor() {
    let repo = Repo::new();
    fs::write(repo.root.join("a.txt"), "changed\n").unwrap();
    let (helper, marker) = repo.helper("fsmonitor");
    repo.raw(&["config", "core.fsmonitor", &helper]);
    repo.raw(&["config", "core.fsmonitorHookVersion", "2"]);
    repo.raw(&["status", "--porcelain"]);
    reset(&marker);
    let status = git::status(&repo.root).unwrap();
    assert!(
        !marker.exists(),
        "node status executed repository fsmonitor"
    );
    assert_eq!(
        (status.files_changed, status.insertions, status.deletions),
        (1, 1, 1)
    );
}

#[test]
fn diff_uses_raw_content_without_textconv_or_external_diff() {
    for textconv in [true, false] {
        let repo = Repo::new();
        let (helper, marker) = repo.helper("diff-helper");
        if textconv {
            fs::write(repo.root.join(".gitattributes"), "*.txt diff=fixture\n").unwrap();
            repo.raw(&["config", "diff.fixture.textconv", &helper]);
        } else {
            repo.raw(&["config", "diff.external", &helper]);
        }
        fs::write(repo.root.join("a.txt"), "changed\n").unwrap();
        repo.raw(&["diff", "HEAD", "--", "a.txt"]);
        reset(&marker);
        let shown = git::diff(&repo.root, Some("a.txt")).unwrap();
        assert!(
            !marker.exists(),
            "node diff executed configured helper (textconv={textconv})"
        );
        assert!(
            shown.contains("-original") && shown.contains("+changed"),
            "{shown}"
        );
        git::status(&repo.root).unwrap();
        assert!(
            !marker.exists(),
            "node status diff executed configured helper"
        );
        fs::write(repo.root.join("untracked.txt"), "new content\n").unwrap();
        let shown = git::diff(&repo.root, Some("untracked.txt")).unwrap();
        assert!(
            !marker.exists(),
            "untracked diff executed configured helper"
        );
        assert!(shown.contains("+new content"), "{shown}");
    }
}

#[test]
fn node_worktree_creation_does_not_execute_repository_post_checkout_hook() {
    let repo = Repo::new();
    let (_, marker) = repo.helper("post-checkout");
    let hooks = repo
        .root
        .join(".git/helpers")
        .to_string_lossy()
        .replace('\\', "/");
    repo.raw(&["config", "core.hooksPath", &hooks]);
    let unsafe_copy = repo.root.join("unsafe-copy");
    repo.raw(&[
        "worktree",
        "add",
        "-b",
        "control",
        unsafe_copy.to_str().unwrap(),
    ]);
    reset(&marker);
    repo.raw(&["worktree", "remove", unsafe_copy.to_str().unwrap()]);
    let safe_copy = repo.root.join("node-copy");
    git::create_worktree(&repo.root, &safe_copy, "node-copy", None).unwrap();
    assert!(
        !marker.exists(),
        "node worktree creation executed repository hook"
    );
    assert_eq!(
        fs::read_to_string(safe_copy.join("a.txt")).unwrap(),
        "original\n"
    );
    git::remove_worktree(&repo.root, &safe_copy, false).unwrap();
}

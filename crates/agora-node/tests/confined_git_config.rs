//! ADR-0078: a confined agent gets a git configuration the node wrote. The person's global configuration here holds an identity and
//! every kind of dangerous key; the agent's git sees the identity and none of the rest.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use agora_adapters::Manifest;
use agora_core::NodeId;
use agora_node::gitconfig;
use agora_node::sandbox::{Level, SandboxPolicy};
use agora_node::supervisor::{
    NodeLimits, RestartPolicy, StartRequest, Supervisor, SupervisorConfig,
};
use agora_node::terminal::TerminalOptions;

const PERSON: &str = "[user]\n\tname = Ada Example\n\temail = ada@example.com\n\
[init]\n\tdefaultBranch = trunk\n\
[credential]\n\thelper = store\n\
[url \"https://token@example.com/\"]\n\tinsteadOf = https://example.com/\n\
[alias]\n\tbang = !echo pwned\n\
[core]\n\tsshCommand = ssh -i /nowhere\n\thooksPath = /nowhere\n\tpager = cat\n\
[commit]\n\tgpgsign = true\n\
[include]\n\tpath = /nowhere/extra\n";

/// The tests share one `HOME` and one of them rewrites its file, so they run one at a time.
fn serial() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A home folder that holds the person's configuration, set as this process's `HOME` once (every test here shares it).
fn home() -> &'static PathBuf {
    static HOME: OnceLock<PathBuf> = OnceLock::new();
    HOME.get_or_init(|| {
        // Under the target folder, never under `/tmp`: the sandbox opens `/tmp`.
        let h = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("gitcfg-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&h);
        std::fs::create_dir_all(&h).unwrap();
        std::fs::write(h.join(".gitconfig"), PERSON).unwrap();
        std::env::set_var("HOME", &h);
        std::env::set_var("USERPROFILE", &h);
        for v in [
            "GIT_CONFIG_GLOBAL",
            "GIT_CONFIG_SYSTEM",
            "GIT_CONFIG_COUNT",
            "XDG_CONFIG_HOME",
        ] {
            std::env::remove_var(v);
        }
        std::env::set_var("GIT_CEILING_DIRECTORIES", env!("CARGO_TARGET_TMPDIR"));
        h
    })
}

fn git(dir: &Path, args: &[&str]) {
    assert!(Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .status()
        .unwrap()
        .success());
}

fn repo(name: &str) -> (PathBuf, PathBuf) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("gitcfg-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    (dir, repo)
}

#[test]
fn the_node_reads_only_the_listed_keys_from_the_persons_configuration() {
    let _serial = serial();
    home();
    let got = gitconfig::read_allowed();
    let keys: Vec<&str> = got.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, ["user.name", "user.email", "init.defaultBranch"]);
    // A dangerous key is not read even when asked for by name.
    assert_eq!(gitconfig::read_key("credential.helper"), None);
    assert_eq!(gitconfig::read_key("core.sshCommand"), None);
    assert!(!gitconfig::identity_missing());
}

#[test]
fn git_with_the_agents_environment_commits_as_the_person_and_sees_no_dangerous_key() {
    let _serial = serial();
    let h = home();
    let (dir, repo) = repo("env");
    let wt = dir.join("worktrees/a");
    std::fs::create_dir_all(&wt).unwrap();
    let file = gitconfig::write_for(&wt).unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    for bad in [
        "credential",
        "insteadOf",
        "alias",
        "sshCommand",
        "hooksPath",
        "gpgsign",
        "include",
        "pager",
    ] {
        assert!(!text.contains(bad), "{bad} in {text}");
    }

    // What a confined agent's process has: a home with the person's real file (here readable, to prove it is not used), the two
    // variables, and nothing else of the node's environment.
    let run = |args: &[&str]| {
        let mut c = Command::new("git");
        c.current_dir(&repo)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("HOME", h)
            .envs(gitconfig::env_for(&file))
            .args(args);
        c.output().unwrap()
    };
    std::fs::write(repo.join("b.txt"), "y").unwrap();
    assert!(run(&["add", "b.txt"]).status.success());
    // `commit.gpgsign = true` in the real file would make this fail (no key); the copy has no such key.
    let commit = run(&["commit", "-q", "-m", "from the agent"]);
    assert!(commit.status.success(), "{commit:?}");
    let who = run(&["log", "-1", "--format=%an <%ae>"]);
    assert_eq!(
        String::from_utf8_lossy(&who.stdout).trim(),
        "Ada Example <ada@example.com>"
    );
    for key in [
        "credential.helper",
        "core.sshCommand",
        "commit.gpgsign",
        "alias.bang",
        "core.hooksPath",
    ] {
        let out = run(&["config", "--get", key]);
        assert!(
            !out.status.success() && out.stdout.is_empty(),
            "{key} visible"
        );
    }
    // The configuration the agent's git has is the file and the repository's, nothing else.
    let list = run(&["config", "--list", "--show-origin"]);
    let list = String::from_utf8_lossy(&list.stdout);
    assert!(!list.contains(h.to_string_lossy().as_ref()), "{list}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_value_with_a_newline_in_the_real_configuration_adds_nothing_to_the_file() {
    // A real git reads a quoted value with `\n` as a newline; the node must neither write it raw nor let it open a section.
    let h = home();
    let evil = h.join(".gitconfig");
    let original = std::fs::read_to_string(&evil).unwrap();
    std::fs::write(
        &evil,
        "[user]\n\tname = \"Eve\\n[credential]\\n\\thelper = !x\"\n\temail = e@example.com\n",
    )
    .unwrap();
    let (dir, _) = repo("inject");
    let wt = dir.join("worktrees/a");
    std::fs::create_dir_all(&wt).unwrap();
    let file = gitconfig::write_for(&wt).unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&evil, original).unwrap();
    assert_eq!(text, "[user]\n\temail = \"e@example.com\"\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_owners_grant_of_the_global_configuration_is_left_as_it_is() {
    let _serial = serial();
    let h = home();
    let (dir, _) = repo("grant");
    let wt = dir.join("worktrees/a");
    std::fs::create_dir_all(&wt).unwrap();
    let by_default = SandboxPolicy {
        level: Level::Worktree,
        ..Default::default()
    };
    assert!(!by_default.grants_gitconfig());
    assert!(by_default.git_config(&wt).is_some());
    let granted = SandboxPolicy {
        level: Level::Worktree,
        read: vec![h.join(".gitconfig")],
        ..Default::default()
    };
    assert!(granted.grants_gitconfig());
    let before = gitconfig::path_for(&wt);
    let _ = std::fs::remove_file(&before);
    assert_eq!(granted.git_config(&wt), None);
    assert!(
        !before.exists(),
        "a file was written although the owner granted theirs"
    );
    // Not confined: nothing is written either.
    assert_eq!(SandboxPolicy::default().git_config(&wt), None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_really_confined_agent_commits_with_the_copied_identity_and_cannot_read_the_real_file() {
    let _serial = serial();
    if !agora_platform::sandbox_supported() {
        eprintln!("skipped: no Landlock on this machine");
        return;
    }
    let h = home();
    let script = format!(
        "{{ echo a > b.txt; git add b.txt; git commit -q -m x; git log -1 --format='%an <%ae>'; \
         git config --get commit.gpgsign; git config --get credential.helper; \
         cat {}/.gitconfig; echo \"GCG=$GIT_CONFIG_GLOBAL\"; echo done; }} > out 2>&1; sleep 30",
        h.display()
    );
    let (dir, repo_dir) = repo("landlock");
    let manifest = Manifest::parse(&format!(
        "kind = \"shy\"\ndisplay_name = \"Shy\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"/bin/sh\", \"-c\", {script:?}]\n"
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
        kind: "shy".into(),
        checkout: repo_dir,
        worktree_root: dir.join("worktrees"),
        name: "a".into(),
        base: None,
    };
    sup.start(&req, Instant::now()).unwrap();
    let out = dir.join("worktrees/a/out");
    let end = Instant::now() + Duration::from_secs(15);
    let read = || std::fs::read_to_string(&out).unwrap_or_default();
    while !read().contains("done") && Instant::now() < end {
        sup.tick(Instant::now());
        std::thread::sleep(Duration::from_millis(50));
    }
    let text = read();
    assert!(text.contains("Ada Example <ada@example.com>"), "{text}");
    assert!(text.contains("GCG="), "{text}");
    assert!(
        !text.contains("store") && !text.contains("token@") && !text.contains("pwned"),
        "{text}"
    );
    assert!(
        text.contains("Permission denied") || text.contains("No such file"),
        "{text}"
    );
    drop(sup);
    let _ = std::fs::remove_dir_all(dir);
}

//! ADR-0078: a confined agent gets a git configuration the node wrote. The person's global configuration here holds an identity and
//! every kind of dangerous key; the agent's git sees the identity and none of the rest.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "private_home/mod.rs"]
mod private_home;
#[path = "short_socket/mod.rs"]
mod short_socket;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

#[cfg(unix)]
use agora_adapters::Manifest;
#[cfg(unix)]
use agora_core::NodeId;
use agora_node::gitconfig;
use agora_node::sandbox::{Level, SandboxPolicy};
#[cfg(unix)]
use agora_node::supervisor::{
    NodeLimits, RestartPolicy, StartRequest, Supervisor, SupervisorConfig,
};
#[cfg(unix)]
use agora_node::terminal::TerminalOptions;
use agora_platform::{git_config_env, Sandbox};

const PERSON: &str = "[user]\n\tname = Ada Example\n\temail = ada@example.com\n\
[init]\n\tdefaultBranch = trunk\n\
[credential]\n\thelper = store\n\
[url \"https://token@example.com/\"]\n\tinsteadOf = https://example.com/\n\
[alias]\n\tbang = !echo pwned\n\
[core]\n\tsshCommand = ssh -i /nowhere\n\thooksPath = /nowhere\n\tpager = cat\n\
[commit]\n\tgpgsign = true\n\
[include]\n\tpath = /nowhere/extra\n";

#[cfg(unix)]
#[test]
fn two_confined_nodes_keep_generated_git_config_in_their_own_roots() {
    use agora_node::daemon::{Node, NodeConfig};

    let _serial = serial();
    if !agora_platform::sandbox_supported() {
        eprintln!("skipped: no confinement on this machine");
        return;
    }
    let fixture_home = home();
    let (base, checkout) = repo("n");
    let script = r#"
        {
            printf '%s\n' "$GIT_CONFIG_GLOBAL"
            git config --global --get user.name
            folder=$(dirname "$GIT_CONFIG_GLOBAL")
            if cat "$folder/neighbor" >/dev/null 2>&1; then echo READ_NEIGHBOR; fi
            if cat "$folder/../state" >/dev/null 2>&1; then echo READ_STATE; fi
            if ls "$folder" >/dev/null 2>&1; then echo LIST_FOLDER; fi
            if (echo changed >> "$GIT_CONFIG_GLOBAL") 2>/dev/null; then echo WRITE_FILE; fi
            if touch "$folder/another" 2>/dev/null; then echo WRITE_FOLDER; fi
            if rm -f "$GIT_CONFIG_GLOBAL" 2>/dev/null; then echo REMOVE_FILE; fi
            echo done
        } > observed
    "#;
    let terminal = format!("{script}\nsleep 30");
    let fake_adapter = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("agora-fake-agent");
    assert!(Command::new(env!("CARGO"))
        .args(["build", "--locked", "-q", "-p", "agora-fake-agent"])
        .status()
        .unwrap()
        .success());
    let adapter = format!("{script}\nexec \"$0\" --adapter");
    let terminal_manifest = format!(
        "kind = \"shy\"\ndisplay_name = \"Shy\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"/bin/sh\", \"-c\", {terminal:?}]\n"
    );
    let adapter_manifest = format!(
        "kind = \"shy\"\ndisplay_name = \"Shy\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = [\"/bin/sh\", \"-c\", {adapter:?}, {:?}]\n",
        fake_adapter.to_str().unwrap()
    );

    let observe = |node_root: &Path, node_id: &str, manifest: &str| {
        let data_dir = node_root.join("data");
        let manifests_dir = node_root.join("manifests");
        let worktree_root = node_root.join("worktrees");
        let config_dir = gitconfig::directory(&data_dir);
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(config_dir.join("neighbor"), "private").unwrap();
        std::fs::write(data_dir.join("state"), "private").unwrap();
        std::fs::create_dir_all(&manifests_dir).unwrap();
        let policy = format!(
            "sandbox = worktree\nsandbox.read = {}\n",
            fake_adapter.display()
        );
        std::fs::write(data_dir.join("policy"), policy).unwrap();
        std::fs::write(manifests_dir.join("shy.toml"), manifest).unwrap();
        // #926: the socket's folder fits whatever the checkout's depth, and its control token is out of the agent's reach.
        let sockets = short_socket::SocketDir::new(node_root, &format!("gitcfg-{node_id}"));
        let node = Node::start(&NodeConfig {
            node_id: node_id.into(),
            socket: sockets.socket("n.sock"),
            data_dir,
            worktree_root: worktree_root.clone(),
            manifests_dir,
            projects: vec![checkout.clone()],
            limits: NodeLimits::default(),
            resume_all: false,
        })
        .unwrap();
        node.supervisor
            .lock()
            .unwrap()
            .start(
                &StartRequest {
                    kind: "shy".into(),
                    checkout: checkout.clone(),
                    worktree_root: worktree_root.clone(),
                    name: format!("check-{node_id}"),
                    base: None,
                },
                Instant::now(),
            )
            .unwrap();
        let observed = worktree_root.join(format!("check-{node_id}/observed"));
        let deadline = Instant::now() + Duration::from_secs(10);
        let text = loop {
            let text = std::fs::read_to_string(&observed).unwrap_or_default();
            if text.contains("done") || Instant::now() >= deadline {
                break text;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let mut lines = text.lines();
        let config = PathBuf::from(lines.next().unwrap_or_default());
        assert_eq!(
            lines.collect::<Vec<_>>(),
            ["Ada Example", "done"],
            "{text:?}"
        );
        assert_eq!(
            config.parent(),
            Some(config_dir.canonicalize().unwrap().as_path())
        );
        // The socket's folder goes with the node: dropped after it.
        (node, config, sockets)
    };

    let one = base.join("one");
    let two = base.join("two");
    let (node_one, config_one, sockets_one) = observe(&one, "one", &terminal_manifest);
    let (node_two, config_two, sockets_two) = observe(&two, "two", &adapter_manifest);
    assert_ne!(config_one, config_two);
    assert!(!fixture_home.join(".agora/git-config").exists());
    drop(node_two);
    drop(node_one);
    drop((sockets_two, sockets_one));
    let _ = std::fs::remove_dir_all(base);
}

#[cfg(windows)]
#[test]
fn generated_git_config_grants_access_to_one_file() {
    let _serial = serial();
    home();
    std::env::set_var("AGORA_WINDOWS_CONFINEMENT", "1");
    assert!(agora_platform::sandbox_supported());

    let root = std::env::temp_dir().join(format!("gitcfg-{}-acl", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let work = root.join("work");
    let data = root.join("data");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    let mut profile = Sandbox::new(vec![], vec![work.clone()]);
    let policy = SandboxPolicy {
        level: Level::Worktree,
        data_dir: Some(data.clone()),
        ..Default::default()
    };
    let file = policy.git_config(&profile).unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    let config = agora_platform::canonicalize_plain(&gitconfig::directory(&data)).unwrap();
    std::fs::write(config.join("sibling"), "private").unwrap();
    let expected = config.join(file.file_name().unwrap());
    let acl = || {
        let result = Command::new("icacls").arg(&expected).output().unwrap();
        assert!(result.status.success(), "{result:?}");
        result.stdout
    };
    let before = acl();
    profile.git_config = Some(file.clone());
    let script = work.join("run.cmd");
    std::fs::write(
        &script,
        format!(
            "@echo off\r\n\
             echo GCG=\"%GIT_CONFIG_GLOBAL%\"\r\n\
             git config --global --get user.name && echo GIT-OK|| echo GIT-FAILED\r\n\
             more < \"%GIT_CONFIG_GLOBAL%\" && echo FILE-OK|| echo FILE-FAILED\r\n\
             more < \"{dir}\\sibling\" >nul 2>&1 && echo SIBLING-READ|| echo SIBLING-REFUSED\r\n\
             dir /b \"{dir}\\\" >nul 2>&1 && echo LISTED|| echo LIST-REFUSED\r\n\
             echo change>> \"%GIT_CONFIG_GLOBAL%\" && echo WRITTEN|| echo WRITE-REFUSED\r\n",
            dir = config.display(),
        ),
    )
    .unwrap();

    let mut cfg = agora_platform::PtyConfig::new(vec![
        "cmd.exe".into(),
        "/c".into(),
        script.to_str().unwrap().into(),
    ]);
    cfg.cwd = Some(work);
    cfg.sandbox = Some(profile);
    let pty = agora_platform::spawn(&cfg).unwrap();
    let mut bytes = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        match pty.output().recv_timeout(Duration::from_millis(100)) {
            Ok(chunk) => bytes.extend(chunk),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    drop(pty);

    let out = String::from_utf8_lossy(&bytes);
    assert!(
        out.contains(&format!("GCG=\"{}\"", expected.display())),
        "{out}"
    );
    for marker in [
        "Ada Example",
        "GIT-OK",
        "FILE-OK",
        "SIBLING-REFUSED",
        "LIST-REFUSED",
        "WRITE-REFUSED",
    ] {
        assert!(out.contains(marker), "{marker} missing: {out}");
    }
    println!("{out}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
    assert_eq!(acl(), before);
    let _ = std::fs::remove_dir_all(root);
}

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
        // First, so `short_socket` knows the real home folder before this one replaces it.
        private_home::init();
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
    // The reason is in the message: this is the one place a CI machine of another system shows why a write failed.
    let file = gitconfig::write_for_result(&dir)
        .unwrap_or_else(|e| panic!("writing {}: {e}", gitconfig::directory(&dir).display()));
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
            .envs(git_config_env(&file))
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
    assert!(
        !list.contains(h.join(".gitconfig").to_string_lossy().as_ref()),
        "{list}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_value_with_a_newline_in_the_real_configuration_adds_nothing_to_the_file() {
    let _serial = serial();
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
    let file = gitconfig::write_for(&dir).unwrap();
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
    let data_dir = dir.join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let folder = gitconfig::directory(&data_dir);
    let by_default = SandboxPolicy {
        level: Level::Worktree,
        data_dir: Some(data_dir.clone()),
        ..Default::default()
    };
    let granted = SandboxPolicy {
        read: vec![h.join(".gitconfig")],
        ..by_default.clone()
    };
    let profile = Sandbox::default();
    assert!(!by_default.grants_gitconfig());
    assert!(granted.grants_gitconfig());
    assert_eq!(granted.git_config(&profile), None);
    let off = SandboxPolicy {
        level: Level::Off,
        ..by_default.clone()
    };
    assert_eq!(off.git_config(&profile), None);
    for profile in [
        Sandbox::new(vec![data_dir.clone()], vec![]),
        Sandbox::new(vec![], vec![data_dir]),
    ] {
        assert_eq!(by_default.git_config(&profile), None);
    }
    assert!(!folder.exists(), "{folder:?}");
    assert!(by_default.git_config(&profile).is_some());
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn a_really_confined_agent_commits_with_the_copied_identity_and_cannot_read_the_real_file() {
    let _serial = serial();
    if !agora_platform::sandbox_supported() {
        eprintln!("skipped: no confinement on this machine");
        return;
    }
    let h = home();
    let script = format!(
        "{{ echo a > b.txt; git add b.txt; git commit -q -m x; git log -1 --format='%an <%ae>'; \
         git config --get commit.gpgsign; git config --get credential.helper; \
         cat {}/.gitconfig; echo \"GCG=$GIT_CONFIG_GLOBAL\"; \
         head -c 6 \"$GIT_CONFIG_GLOBAL\" | tr -d '\\n\\t' | sed 's/^/RD:/'; echo; \
         echo evil >> \"$GIT_CONFIG_GLOBAL\" || echo WRITE_REFUSED; \
         touch \"$(dirname \"$GIT_CONFIG_GLOBAL\")/planted\" || echo FOLDER_REFUSED; echo done; }} > out 2>&1; sleep 30",
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
    let data_dir = dir.join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    sup.set_sandbox(SandboxPolicy {
        level: Level::Worktree,
        protect: vec![data_dir.clone()],
        data_dir: Some(data_dir),
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
    // Review F6: the agent reads its file, and can write neither the file nor its folder.
    assert!(text.contains("RD:[user]"), "{text}");
    assert!(text.contains("WRITE_REFUSED"), "{text}");
    assert!(text.contains("FOLDER_REFUSED"), "{text}");
    assert!(!text.contains("evil"), "{text}");
    assert!(
        !text.contains("store") && !text.contains("token@") && !text.contains("pwned"),
        "{text}"
    );
    assert!(
        // The OS's own wording of a refusal: Landlock says "Permission denied", Seatbelt "Operation not permitted".
        text.contains("Permission denied")
            || text.contains("Operation not permitted")
            || text.contains("No such file"),
        "{text}"
    );
    drop(sup);
    let _ = std::fs::remove_dir_all(dir);
}

/// The node's own environment (`GIT_CONFIG_COUNT` and friends, `GIT_CONFIG_PARAMETERS`) applies on top of any global file unless the
/// agent's environment cancels it.
#[test]
fn settings_in_the_nodes_environment_do_not_reach_a_confined_agents_git() {
    let _serial = serial();
    let h = home();
    let (dir, repo) = repo("envleak");
    let file = gitconfig::write_for(&dir).unwrap();
    let run = |cancel: bool| {
        let mut c = Command::new("git");
        c.current_dir(&repo)
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("HOME", h)
            // What the node was started with.
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "core.sshCommand")
            .env("GIT_CONFIG_VALUE_0", "evil-count")
            .env("GIT_CONFIG_PARAMETERS", "'alias.zz'='!evil-params'")
            .args(["config", "--get-regexp", "^(core.sshcommand|alias.zz)$"]);
        if cancel {
            c.envs(git_config_env(&file));
        } else {
            // The same, with only the two variables of decisions 1 and 3.
            c.env("GIT_CONFIG_GLOBAL", &file)
                .env("GIT_CONFIG_NOSYSTEM", "1");
        }
        String::from_utf8_lossy(&c.output().unwrap().stdout).into_owned()
    };
    // Negative control: without the cancelling variables both settings apply.
    let leaked = run(false);
    assert!(
        leaked.contains("evil-count") && leaked.contains("evil-params"),
        "{leaked}"
    );
    assert_eq!(run(true), "");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_nodes_own_git_variables_do_not_change_what_is_read_as_the_persons_configuration() {
    let _serial = serial();
    let h = home();
    let other = h.join("elsewhere");
    std::fs::write(&other, "[user]\n\tname = Mallory\n").unwrap();
    std::env::set_var("GIT_CONFIG_GLOBAL", &other);
    std::env::set_var("GIT_CONFIG_COUNT", "1");
    std::env::set_var("GIT_CONFIG_KEY_0", "user.name");
    std::env::set_var("GIT_CONFIG_VALUE_0", "Mallory");
    std::env::set_var("GIT_CONFIG_PARAMETERS", "'user.name'='Mallory'");
    let name = gitconfig::read_key("user.name");
    for v in [
        "GIT_CONFIG_GLOBAL",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_KEY_0",
        "GIT_CONFIG_VALUE_0",
        "GIT_CONFIG_PARAMETERS",
    ] {
        std::env::remove_var(v);
    }
    assert_eq!(name.as_deref(), Some("Ada Example"));
}

#[cfg(unix)]
#[test]
fn a_file_whose_folder_is_reached_through_a_link_into_a_writable_folder_is_not_written() {
    let _serial = serial();
    home();
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("gitcfg-{}-link", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let data_dir = dir.join("node/data");
    let folder = gitconfig::directory(&data_dir);
    std::fs::create_dir_all(&folder).unwrap();
    let policy = SandboxPolicy {
        level: Level::Worktree,
        data_dir: Some(data_dir),
        ..Default::default()
    };
    let alias = dir.join("alias");
    std::os::unix::fs::symlink(&folder, &alias).unwrap();
    assert_eq!(policy.git_config(&Sandbox::new(vec![], vec![alias])), None);
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);

    let worktrees = dir.join("worktrees");
    std::fs::create_dir_all(&worktrees).unwrap();
    let made = policy
        .git_config(&Sandbox::new(vec![], vec![worktrees]))
        .unwrap();
    assert_eq!(
        made.parent(),
        Some(folder.canonicalize().unwrap().as_path())
    );
    assert!(made.exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_fresh_node_keeps_a_distinct_configuration_for_each_launch() {
    let _serial = serial();
    home();
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("gitcfg-{}-fresh", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let folder = gitconfig::directory(&root);
    assert!(!folder.exists());
    let policy = SandboxPolicy {
        level: Level::Worktree,
        data_dir: Some(root.clone()),
        ..Default::default()
    };
    let profile = Sandbox::default();
    let one = policy.git_config(&profile).unwrap();
    let two = policy.git_config(&profile).unwrap();
    let folder = folder.canonicalize().unwrap();
    for file in [&one, &two] {
        assert_eq!(file.parent(), Some(folder.as_path()));
        assert!(file.is_file());
    }
    assert_ne!(one, two);
    let _ = std::fs::remove_dir_all(root);
}

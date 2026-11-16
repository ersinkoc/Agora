// Appended to a temporary copy of the existing real-node control fixture.
#[test]
fn codex_review636_repository_prefixes_cannot_expose_secret_hunks() {
    assert!(agora_platform::LOCAL_SOCKETS_SUPPORTED && agora_platform::PTY_SUPPORTED);
    let e = env("codex636-prefix");
    // Use only the fake agent, rather than the original fixture's plain shell.
    let manifest = format!(
        "kind = \"shell\"\ndisplay_name = \"Synthetic\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = {}\n",
        serde_json::to_string(&vec![fake_agent_path()]).unwrap()
    );
    std::fs::write(e.dir.join("manifests/shell.toml"), manifest).unwrap();
    let n = node(&e, NodeLimits::default());
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let agent = started(ok(c.call(start_cmd("shell"))));
    let wt = PathBuf::from(&agent.worktree_id);
    let cases = [
        (".env", "SYNTHETIC_ENV_VALUE"),
        (".ssh/tokens", "SYNTHETIC_SSH_VALUE"),
        (".docker/config.json", "SYNTHETIC_DOCKER_VALUE"),
    ];
    for (path, _) in cases {
        std::fs::create_dir_all(wt.join(path).parent().unwrap()).unwrap();
        std::fs::write(wt.join(path), "synthetic old\n").unwrap();
    }
    git(
        &wt,
        &["add", "-f", ".env", ".ssh/tokens", ".docker/config.json"],
    );
    git(&wt, &["commit", "-q", "-m", "synthetic fixtures"]);
    for (path, marker) in cases {
        std::fs::write(wt.join(path), format!("{marker}\n")).unwrap();
        assert_eq!(
            err_code(c.call(read_cmd(&agent.worktree_id, path))),
            "files.secret"
        );
    }
    std::fs::write(wt.join("a.txt"), "PUBLIC_CONTROL\n").unwrap();
    let safe = diff_of(&mut c, &agent.worktree_id);
    assert!(safe.diff.contains("PUBLIC_CONTROL"));
    for (_, marker) in cases {
        assert!(!safe.diff.contains(marker));
    }
    assert_eq!(safe.secret_files.len(), 3);
    git(&wt, &["config", "diff.srcPrefix", "x"]);
    git(&wt, &["config", "diff.dstPrefix", "y"]);
    let changed = diff_of(&mut c, &agent.worktree_id);
    let leaked: Vec<_> = cases
        .iter()
        .filter_map(|(p, m)| changed.diff.contains(m).then_some(*p))
        .collect();
    if std::env::var_os("CODEX636_PINNED_CONTROL").is_some() {
        assert!(
            leaked.is_empty(),
            "explicit prefixes did not fix the demonstrated leak"
        );
        assert_eq!(changed.secret_files.len(), 3);
    } else {
        assert_eq!(
            leaked.len(),
            3,
            "the expected config-dependent disclosure was not reproduced"
        );
        assert!(changed.secret_files.is_empty());
    }
    // Restore only this scratch repository's settings; same process/policy.
    git(&wt, &["config", "--unset", "diff.srcPrefix"]);
    git(&wt, &["config", "--unset", "diff.dstPrefix"]);
    let restored = diff_of(&mut c, &agent.worktree_id);
    for (_, marker) in cases {
        assert!(!restored.diff.contains(marker));
    }
    assert_eq!(restored.secret_files.len(), 3);
    println!(
        "{}",
        serde_json::json!({"proof":"real node control protocol GitDiffResult", "head":"a55944bde36127b48cb0741b42a6b43a2c69516d", "files_read_refusals":3,"default_diff_secret_files":3,"custom_prefix_disclosed_paths":leaked,"custom_prefix_secret_files":changed.secret_files.len(),"restored_diff_secret_files":3,"pinned_control":std::env::var_os("CODEX636_PINNED_CONTROL").is_some()})
    );
    ok(c.call(Cmd::AgentStop(v1::AgentStop {
        agent_id: agent.agent_id,
        stop_helpers: false,
    })));
    drop(c);
    drop(n);
    let _ = std::fs::remove_dir_all(e.dir);
}

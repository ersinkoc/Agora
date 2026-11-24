//! #1150 (ADR-0092): an ACP registry kind that starts through a package runner (`npx`, `uvx`) starts the runner in an empty folder of
//! its own in the node's data folder, never in the project: a project's `.npmrc`, `package.json`, `node_modules`, `uv.toml`,
//! `pyproject.toml` or `.python-version` is not where the runner starts or above it, so it cannot change what the pinned package
//! resolves to. The owner's own configuration (`HOME`, `XDG_CONFIG_HOME`) still reaches the runner, and the agent is still told the
//! project's folder (ACP `session/new` `cwd`). The folder is emptied again at the next start. The runners here are local stubs that
//! record what they were given and run the fake ACP agent: no network, no real `npx` or `uvx`.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use agora_node::control::Client;
use agora_node::daemon::{Node, NodeConfig};
use agora_protocol::v1::{self, command_request::Command as Cmd};

const AGORA: &str = env!("CARGO_BIN_EXE_agora");

fn fake_agent() -> String {
    let mut p = PathBuf::from(AGORA);
    p.pop();
    p.push(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    assert!(
        Command::new(cargo)
            .args(["build", "-q", "-p", "agora-fake-agent"])
            .status()
            .expect("run cargo")
            .success(),
        "building agora-fake-agent failed"
    );
    p.to_string_lossy().into_owned()
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .status()
        .expect("git")
        .success();
    assert!(ok, "git {args:?}");
}

fn wait<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// What a stub runner recorded at one start: `key=value` lines.
fn record(log: &Path, start: usize) -> Vec<(String, String)> {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    text.split("---\n")
        .filter(|r| !r.trim().is_empty())
        .nth(start)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn get<'a>(rec: &'a [(String, String)], key: &str) -> &'a str {
    rec.iter()
        .find(|(k, _)| k == key)
        .map_or("", |(_, v)| v.as_str())
}

/// The files a package runner reads from the folder it starts in, or from one above it.
const PROJECT_CONFIG: &[&str] = &[
    ".npmrc",
    "package.json",
    "node_modules",
    "uv.toml",
    "pyproject.toml",
    ".python-version",
];

#[test]
fn a_registry_package_runner_starts_outside_the_project_and_keeps_the_owners_configuration() {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let fake = fake_agent();
    // The node runs `acp-bridge` from this build's binary, not from the test's own.
    std::env::set_var("AGORA_ACP_BRIDGE", AGORA);
    let dir = std::env::temp_dir().join(format!("agora-runner-folder-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let dir = std::fs::canonicalize(&dir).unwrap();

    // The owner's own configuration: a user `.npmrc` (a mirror) and a user uv configuration.
    let home = dir.join("home");
    let xdg = dir.join("xdg");
    std::fs::create_dir_all(xdg.join("uv")).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join(".npmrc"),
        "registry=https://owner-mirror.invalid/\n",
    )
    .unwrap();
    std::fs::write(
        xdg.join("uv/uv.toml"),
        "index-url = \"https://owner-mirror.invalid/simple\"\n",
    )
    .unwrap();
    std::env::set_var("HOME", &home);
    std::env::set_var("XDG_CONFIG_HOME", &xdg);

    // A project that carries package-manager configuration of its own.
    let repo = dir.join("demo");
    std::fs::create_dir_all(repo.join("node_modules/.bin")).unwrap();
    std::fs::write(repo.join(".npmrc"), "registry=https://project.invalid/\n").unwrap();
    std::fs::write(
        repo.join("package.json"),
        "{\"name\":\"demo\",\"bin\":{\"stub-agent@1.0.0\":\"x.js\"}}\n",
    )
    .unwrap();
    std::fs::write(repo.join("node_modules/.bin/node"), "#!/bin/sh\n").unwrap();
    std::fs::write(
        repo.join("uv.toml"),
        "index-url = \"https://project.invalid/simple\"\n",
    )
    .unwrap();
    std::fs::write(
        repo.join("pyproject.toml"),
        "[tool.uv]\nindex-url = \"https://project.invalid/simple\"\n",
    )
    .unwrap();
    std::fs::write(repo.join(".python-version"), "3.9\n").unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["add", "-f", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);

    // Stub runners on PATH, named as the converted manifests name them: each records where it started, what it was given and whether
    // the owner's configuration is there; leaves files behind in its folder, as an agent could; and runs the fake ACP agent with
    // everything the bridge sends it also written down.
    let bin = dir.join("bin");
    let rec = dir.join("rec");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&rec).unwrap();
    for runner in ["npx", "uvx"] {
        let log = rec.join(format!("{runner}.log"));
        let input = rec.join(format!("{runner}.in"));
        let script = format!(
            "#!/bin/sh\n{{\n  echo \"cwd=$(pwd -P)\"\n  echo \"args=$*\"\n  echo \"home=$HOME\"\n  echo \"xdg=$XDG_CONFIG_HOME\"\n  \
             echo \"entries=$(ls -A)\"\n  ls -A >/dev/null 2>&1 && echo listed=yes\n  [ -r \"$HOME/.npmrc\" ] && echo \"user_npmrc=$(cat \"$HOME/.npmrc\")\"\n  \
             [ -r \"$XDG_CONFIG_HOME/uv/uv.toml\" ] && echo \"user_uv=$(cat \"$XDG_CONFIG_HOME/uv/uv.toml\")\"\n  echo ---\n}} >> {log:?}\n\
             echo 'registry=https://left-behind.invalid/' > .npmrc\nmkdir -p node_modules/.bin\n\
             tee -a {input:?} | {fake:?} --acp\n",
        );
        let path = bin.join(runner);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![bin.clone()];
    paths.extend(std::env::split_paths(&path));
    std::env::set_var("PATH", std::env::join_paths(paths).unwrap());

    std::fs::create_dir_all(dir.join("manifests")).unwrap();
    std::fs::create_dir_all(dir.join("data")).unwrap();
    let node = Node::start(&NodeConfig {
        resume_all: false,
        node_id: "laptop".into(),
        socket: dir.join("c.sock"),
        data_dir: dir.join("data"),
        worktree_root: dir.join("wt"),
        manifests_dir: dir.join("manifests"),
        projects: vec![repo.clone()],
        limits: Default::default(),
    })
    .expect("start node");
    // Registered as the node registers the converted manifests (by origin), with the converter's launch lines.
    for (kind, argv) in [
        ("registry-stub-npx", r#""npx", "--yes", "stub-agent@1.0.0""#),
        ("registry-stub-uvx", r#""uvx", "stub-agent==1.0.0""#),
    ] {
        node.supervisor.lock().unwrap().register_from_registry(
            agora_adapters::Manifest::parse(&format!(
                "kind = \"{kind}\"\ndisplay_name = \"Stub\"\nversion = \"1.0.0\"\nmin_engine = 1\n[structured]\nacp = {{ argv = [{argv}] }}\n"
            ))
            .unwrap(),
        );
    }

    let idle = |id: &str| {
        let id = id.to_string();
        let node = &node;
        wait("the agent's session", move || {
            node.supervisor
                .lock()
                .unwrap()
                .list()
                .into_iter()
                .find(|v| v.id.as_str() == id && v.state == agora_core::AgentState::Idle)
                .map(|v| v.id)
        })
    };
    for (kind, runner, args) in [
        ("registry-stub-npx", "npx", "--yes stub-agent@1.0.0"),
        ("registry-stub-uvx", "uvx", "stub-agent==1.0.0"),
    ] {
        let log = rec.join(format!("{runner}.log"));
        // This machine's own control socket may start a registry kind without a device signature.
        let ok = Client::connect(node.socket().expect("control socket"))
            .unwrap()
            .call(Cmd::AgentStart(v1::AgentStart {
                kind: kind.into(),
                ..Default::default()
            }))
            .unwrap()
            .unwrap_or_else(|e| panic!("{kind}: {e:?}"));
        let Some(v1::command_ok::Result::AgentStart(s)) = ok.result else {
            panic!("{kind}: not started");
        };
        let id = idle(&s.agent_id);
        let worktree = node
            .supervisor
            .lock()
            .unwrap()
            .list()
            .into_iter()
            .find(|v| v.id == id)
            .map(|v| v.worktree)
            .expect("the agent's worktree");
        let worktree = std::fs::canonicalize(worktree).unwrap();
        let folder = dir.join("data/runner").join(id.as_str());

        let first = record(&log, 0);
        assert_eq!(get(&first, "cwd"), folder.to_str().unwrap(), "{first:?}");
        assert_eq!(get(&first, "args"), args, "{first:?}");
        // Nothing of the project where it starts (empty when it started) or above it (up to the test's own folder).
        assert_eq!(get(&first, "entries"), "", "{first:?}");
        assert_eq!(get(&first, "listed"), "yes", "{first:?}");
        for above in folder
            .ancestors()
            .skip(1)
            .take_while(|a| a.starts_with(&dir))
        {
            assert!(!above.starts_with(&repo) && !above.starts_with(&worktree));
            for name in PROJECT_CONFIG {
                assert!(!above.join(name).exists(), "{}", above.join(name).display());
            }
        }
        // The project still holds its files: the agent works there, and the runner did not see them.
        assert!(worktree.join(".npmrc").exists() && worktree.join("uv.toml").exists());
        // The owner's configuration is where the runner looks for it.
        assert_eq!(get(&first, "home"), home.to_str().unwrap());
        assert_eq!(get(&first, "xdg"), xdg.to_str().unwrap());
        assert_eq!(
            get(&first, "user_npmrc"),
            "registry=https://owner-mirror.invalid/"
        );
        assert_eq!(
            get(&first, "user_uv"),
            "index-url = \"https://owner-mirror.invalid/simple\""
        );
        // The agent is told the project's folder as its session's working folder.
        let sent = std::fs::read_to_string(rec.join(format!("{runner}.in"))).unwrap();
        let new_session = sent
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .find(|m| m["method"] == "session/new")
            .expect("session/new");
        assert_eq!(
            std::fs::canonicalize(new_session["params"]["cwd"].as_str().unwrap()).unwrap(),
            worktree
        );

        // What the runner left in its folder is gone at the next start of the same agent.
        assert!(folder.join(".npmrc").exists());
        node.supervisor.lock().unwrap().stop(&id).unwrap();
        node.supervisor
            .lock()
            .unwrap()
            .resume(&id, Instant::now())
            .unwrap();
        idle(id.as_str());
        let second = record(&log, 1);
        assert_eq!(get(&second, "cwd"), folder.to_str().unwrap(), "{second:?}");
        assert_eq!(get(&second, "entries"), "", "{second:?}");
        node.supervisor.lock().unwrap().stop(&id).unwrap();
    }
    drop(node);

    // Confined (where this machine confines agents): the runner's folder is opened to the agent as its worktree is, so the runner
    // starts there under the agent's confinement. The stub runner is granted by the owner's own `sandbox.read`, its record folder by
    // `sandbox.write`; the owner's home is not granted here, so only where it starts and that it starts are checked.
    if agora_platform::sandbox_supported() {
        let mut sup =
            agora_node::supervisor::Supervisor::new(agora_node::supervisor::SupervisorConfig {
                node: agora_core::NodeId::new("laptop"),
                restart: Default::default(),
                terminal: Default::default(),
                limits: Default::default(),
            });
        sup.set_sandbox(agora_node::sandbox::SandboxPolicy {
            level: agora_node::sandbox::Level::Worktree,
            read: vec![bin.clone()],
            write: vec![rec.clone()],
            ..Default::default()
        });
        let runners = dir.join("data-confined/runner");
        sup.set_runner_root(runners.clone());
        sup.register_from_registry(
            agora_adapters::Manifest::parse(&format!(
                "kind = \"registry-stub-npx\"\ndisplay_name = \"Stub\"\nversion = \"1.0.0\"\nmin_engine = 1\n[structured]\nacp = {{ argv = [{:?}, \"--yes\", \"stub-agent@1.0.0\"] }}\n",
                bin.join("npx").to_string_lossy()
            ))
            .unwrap(),
        );
        let id = sup
            .start(
                &agora_node::supervisor::StartRequest {
                    kind: "registry-stub-npx".into(),
                    checkout: repo.clone(),
                    worktree_root: dir.join("wt-confined"),
                    name: "confined".into(),
                    base: None,
                },
                Instant::now(),
            )
            .expect("a confined start");
        let confined = record(&rec.join("npx.log"), 2);
        assert_eq!(
            get(&confined, "cwd"),
            runners.join(id.as_str()).to_str().unwrap(),
            "{confined:?}"
        );
        assert_eq!(get(&confined, "entries"), "", "{confined:?}");
        assert_eq!(get(&confined, "listed"), "yes", "{confined:?}");
        sup.stop(&id).unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

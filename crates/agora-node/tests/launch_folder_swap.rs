//! F1012 (ADR-0052): a launch starts its agent in the folder it checked, or not at all. The folder is changed while the check waits
//! at its last git call: the node runs `git` by name, so a `git` of the test's own, first on PATH, forwards to the real one and, once
//! armed, stops at `symbolic-ref` until released. No sleeps; waits are bounded condition loops. In a test binary of its own, because it
//! changes PATH for the whole process.
#![allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "private_home/mod.rs"]
mod private_home;
#[path = "short_socket/mod.rs"]
mod short_socket;
use agora_node::{
    control::Client,
    daemon::{Node, NodeConfig},
    supervisor::NodeLimits,
};
use agora_protocol::v1::{self, command_ok::Result as Ok_, command_request::Command as Cmd};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const OWN: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const OTHER: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

fn fake() -> String {
    let p = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
    if !p.exists() {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        assert!(Command::new(cargo)
            .args(["build", "-q", "-p", "agora-fake-agent", "--target-dir"])
            .arg(p.parent().unwrap().parent().unwrap())
            .status()
            .unwrap()
            .success());
    }
    p.to_string_lossy().into_owned()
}

/// A repository with a portable identity of its own, so each folder is its own project.
fn repo(x: &Path, id: &str) {
    std::fs::create_dir_all(x).unwrap();
    std::fs::write(
        x.join(".agent-project.json"),
        format!(r#"{{"project_id":"{id}"}}"#),
    )
    .unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["add", "."],
        &["commit", "-qm", "init"],
    ] {
        assert!(Command::new("git")
            .arg("-C")
            .arg(x)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .status()
            .unwrap()
            .success());
    }
}

struct Env {
    base: PathBuf,
    gate: PathBuf,
    reports: PathBuf,
    socket: PathBuf,
    node: Option<Node>,
    seen: BTreeSet<String>,
    /// Dropped after the node (`drop` takes the node first).
    _sockets: short_socket::SocketDir,
}

impl Env {
    fn new() -> Self {
        private_home::init();
        let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("launch-folder-swap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let base = std::fs::canonicalize(&base).unwrap();
        let (gate, bin, reports) = (base.join("gate"), base.join("bin"), base.join("reports"));
        for d in [&gate, &bin, &reports, &base.join("manifests")] {
            std::fs::create_dir_all(d).unwrap();
        }
        let real_git = String::from_utf8(
            Command::new("sh")
                .args(["-c", "command -v git"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap();
        let g = gate.display();
        std::fs::write(
            bin.join("git"),
            format!(
                "#!/bin/sh\ncase \" $* \" in *\" symbolic-ref \"*) if [ -e '{g}/armed' ]; then rm -f '{g}/armed'; : > '{g}/reached'; \
                 while [ ! -e '{g}/release' ]; do :; done; fi;; esac\nexec '{}' \"$@\"\n",
                real_git.trim()
            ),
        )
        .unwrap();
        assert!(Command::new("chmod")
            .arg("+x")
            .arg(bin.join("git"))
            .status()
            .unwrap()
            .success());
        std::env::set_var(
            "PATH",
            format!(
                "{}:{}",
                bin.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        );
        let argv = serde_json::to_string(&vec![
            fake(),
            "--probe-directory".into(),
            reports
                .join("probe-{pid}.json")
                .to_string_lossy()
                .into_owned(),
        ])
        .unwrap();
        std::fs::write(
            base.join("manifests/probe.toml"),
            format!(
                "kind = \"probe\"\ndisplay_name = \"Probe\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = {argv}\n\
                 [mcp]\nmethod = \"cli_flag\"\npath = \".mcp.json\"\nflag = [\"--mcp-config\", \"{{config_file}}\"]\n"
            ),
        )
        .unwrap();
        // #1031: the socket's folder fits whatever the checkout's depth.
        let sockets = short_socket::SocketDir::new(&base, "lfs");
        let node = Node::start(&NodeConfig {
            resume_all: false,
            node_id: "node-launch-folder-swap".into(),
            socket: sockets.socket("n.sock"),
            data_dir: base.join("data"),
            worktree_root: base.join("worktrees"),
            manifests_dir: base.join("manifests"),
            projects: vec![],
            limits: NodeLimits::default(),
        })
        .unwrap();
        let socket = node.socket().unwrap().to_path_buf();
        Self {
            base,
            gate,
            reports,
            socket,
            node: Some(node),
            seen: BTreeSet::new(),
            _sockets: sockets,
        }
    }

    fn client(&self) -> Client {
        Client::connect(&self.socket).unwrap()
    }

    /// The working folders of the node's agents.
    fn agents(&self) -> Vec<String> {
        match self
            .client()
            .call(Cmd::AgentList(v1::AgentList::default()))
            .unwrap()
            .unwrap()
            .result
        {
            Some(Ok_::AgentList(l)) => l.agents.into_iter().map(|a| a.worktree_id).collect(),
            other => panic!("{other:?}"),
        }
    }

    /// The working folder in the first probe report not seen before (each start writes one of its own).
    fn next_report(&mut self) -> String {
        let end = Instant::now() + Duration::from_secs(20);
        loop {
            for e in std::fs::read_dir(&self.reports).unwrap().flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if !name.starts_with("probe-")
                    || !name.ends_with(".json")
                    || self.seen.contains(&name)
                {
                    continue;
                }
                let Some(v) = std::fs::read(e.path())
                    .ok()
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                else {
                    continue;
                };
                self.seen.insert(name);
                return v["cwd"].as_str().unwrap_or("").to_string();
            }
            assert!(Instant::now() < end, "no probe report");
            std::thread::yield_now();
        }
    }

    /// Two projects in `work`, `own/x` and `other/x`, and a launch of `own/x` whose check waits at the gate while `change` runs.
    /// Returns `own/x` and the answer.
    fn launch_while(
        &self,
        work: &str,
        change: impl FnOnce(&Path, &Path),
    ) -> (PathBuf, Result<v1::CommandOk, v1::ErrorInfo>) {
        let (own, other) = (
            self.base.join(work).join("own"),
            self.base.join(work).join("other"),
        );
        repo(&own.join("x"), OWN);
        repo(&other.join("x"), OTHER);
        let x = own.join("x");
        let _ = std::fs::remove_file(self.gate.join("release"));
        let _ = std::fs::remove_file(self.gate.join("reached"));
        std::fs::write(self.gate.join("armed"), "").unwrap();
        let cmd = Cmd::AgentLaunch(v1::AgentLaunch {
            path: x.to_string_lossy().into_owned(),
            project_id: format!("id:{OWN}"),
            kind: "probe".into(),
            ..Default::default()
        });
        let socket = self.socket.clone();
        let launcher =
            std::thread::spawn(move || Client::connect(&socket).unwrap().call(cmd).unwrap());
        let end = Instant::now() + Duration::from_secs(20);
        while !self.gate.join("reached").exists() {
            assert!(Instant::now() < end, "the launch never reached the gate");
            std::thread::yield_now();
        }
        change(&own, &other);
        std::fs::write(self.gate.join("release"), "").unwrap();
        (x, launcher.join().unwrap())
    }

    fn assert_refused(&self, answer: &Result<v1::CommandOk, v1::ErrorInfo>, before: usize) {
        let e = answer.as_ref().expect_err("the launch is refused");
        assert_eq!(e.code, "protocol.invalid_message", "{e:?}");
        assert!(
            matches!(
                e.params.get("field").and_then(|p| p.value.as_ref()),
                Some(v1::param_value::Value::Str(f)) if f == "directory.path"
            ),
            "{e:?}"
        );
        assert_eq!(self.agents().len(), before, "nothing was started");
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        self.node.take();
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

#[test]
fn a_launch_starts_in_the_folder_it_checked_or_not_at_all() {
    // The gate is a shell script named `git` and the swaps make links: where either is unavailable (Windows), nothing to run.
    let probe = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("launch-folder-swap-link-{}", std::process::id()));
    let _ = std::fs::remove_file(&probe);
    let links = agora_platform::symlink(Path::new("."), &probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    if !(links && agora_platform::PTY_SUPPORTED && agora_platform::LOCAL_SOCKETS_SUPPORTED) {
        return;
    }
    let mut env = Env::new();
    let link = agora_platform::symlink;

    // Swapped back before the start: the folder checked is there again, and the agent starts in it.
    let (x, answer) = env.launch_while("undone", |own, other| {
        std::fs::rename(own.join("x"), own.join("x.old")).unwrap();
        link(&other.join("x"), &own.join("x")).unwrap();
        std::fs::remove_file(own.join("x")).unwrap();
        std::fs::rename(own.join("x.old"), own.join("x")).unwrap();
    });
    assert!(answer.is_ok(), "{answer:?}");
    assert_eq!(PathBuf::from(env.next_report()), x);
    let before = env.agents().len();

    // The folder swapped for a link to another project's folder: refused, never followed.
    let (_, answer) = env.launch_while("folder", |own, other| {
        std::fs::rename(own.join("x"), own.join("x.old")).unwrap();
        link(&other.join("x"), &own.join("x")).unwrap();
    });
    env.assert_refused(&answer, before);

    // A folder above it swapped for a link: refused.
    let (_, answer) = env.launch_while("above", |own, other| {
        std::fs::rename(own, own.with_file_name("own.old")).unwrap();
        link(other, own).unwrap();
    });
    env.assert_refused(&answer, before);

    // No link at all (the same on every system): the folder moved away and a look-alike with the same identity put in its place.
    let (_, answer) = env.launch_while("look-alike", |own, _| {
        std::fs::rename(own.join("x"), own.join("x.old")).unwrap();
        std::fs::create_dir(own.join("x")).unwrap();
        std::fs::copy(
            own.join("x.old/.agent-project.json"),
            own.join("x/.agent-project.json"),
        )
        .unwrap();
    });
    env.assert_refused(&answer, before);
}

//! F1021: `agent.resume` (ADR-0060; the node's automatic resume uses the same `resume_agent`) checks the interrupted agent's
//! folder by its path (`directory::inspect`) and then starts it by that path. A folder swapped for a link to another project's folder
//! after the check (another agent of the project renamed it and put a link in its place) must not be followed: the folder is held
//! from before its check and the agent is resumed in the held folder, or not at all.
//! Gated as the F1012 test is: the node runs `git` by name, so a `git` of the test's own, first on PATH, forwards to the real one
//! and, once armed, stops at `symbolic-ref` (the check's last git call) until released. No sleeps; waits are bounded condition loops.
//! In a test binary of its own, because it changes PATH for the whole process.
#![allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "private_home/mod.rs"]
mod private_home;
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
    /// The node's socket, in the temporary folder as spelled (not under the long target path, and not resolved: a socket path
    /// has a short length limit).
    socket: PathBuf,
    gate: PathBuf,
    reports: PathBuf,
    node: Option<Node>,
    seen: BTreeSet<String>,
}

impl Env {
    fn new(name: &str) -> Self {
        private_home::init();
        let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("resume-folder-swap-{}-{name}", std::process::id()));
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
        let path = std::env::var("PATH").unwrap_or_default();
        if !path.starts_with(&bin.display().to_string()) {
            std::env::set_var("PATH", format!("{}:{path}", bin.display()));
        }
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
                "kind = \"probe\"\ndisplay_name = \"Probe\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = {argv}\n"
            ),
        )
        .unwrap();
        let socket = std::env::temp_dir().join(format!("rfs-{}-{name}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        let mut e = Self {
            base,
            socket,
            gate,
            reports,
            node: None,
            seen: BTreeSet::new(),
        };
        e.start();
        e
    }

    fn start(&mut self) {
        self.node = None;
        self.node = Some(
            Node::start(&NodeConfig {
                resume_all: false,
                node_id: "node-resume-folder-swap".into(),
                socket: self.socket.clone(),
                data_dir: self.base.join("data"),
                worktree_root: self.base.join("worktrees"),
                manifests_dir: self.base.join("manifests"),
                projects: vec![],
                limits: NodeLimits::default(),
            })
            .unwrap(),
        );
    }

    /// Agents running on the node now (every one is interrupted by the node restart before each resume).
    fn running(&self) -> usize {
        self.node
            .as_ref()
            .unwrap()
            .supervisor
            .lock()
            .unwrap()
            .list()
            .iter()
            .filter(|v| v.running)
            .count()
    }

    fn client(&self) -> Client {
        Client::connect(self.node.as_ref().unwrap().socket().unwrap()).unwrap()
    }

    fn next_report(&mut self) -> Option<String> {
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
                return Some(v["cwd"].as_str().unwrap_or("").to_string());
            }
            if Instant::now() > end {
                return None;
            }
            std::thread::yield_now();
        }
    }

    /// Launches the probe in `own/x`, restarts the node (the agent is interrupted), then resumes it while `change` runs at the
    /// check's last git call. Returns `own/x`, `other/x` and the resume's answer.
    fn resume_while(
        &mut self,
        work: &str,
        change: impl FnOnce(&Path, &Path),
    ) -> (PathBuf, PathBuf, Result<v1::CommandOk, v1::ErrorInfo>) {
        let (own, other) = (
            self.base.join(work).join("own"),
            self.base.join(work).join("other"),
        );
        repo(&own.join("x"), OWN);
        repo(&other.join("x"), OTHER);
        let x = own.join("x");
        let id = match self
            .client()
            .call(Cmd::AgentLaunch(v1::AgentLaunch {
                path: x.to_string_lossy().into_owned(),
                project_id: format!("id:{OWN}"),
                kind: "probe".into(),
                ..Default::default()
            }))
            .unwrap()
            .unwrap()
            .result
        {
            Some(Ok_::AgentLaunch(r)) => r.agent_id,
            other => panic!("{other:?}"),
        };
        assert_eq!(self.next_report().as_deref(), Some(x.to_str().unwrap()));
        // The node stops (its agents with it) and starts again on the same data folder: the agent is interrupted.
        self.start();
        let _ = std::fs::remove_file(self.gate.join("release"));
        let _ = std::fs::remove_file(self.gate.join("reached"));
        std::fs::write(self.gate.join("armed"), "").unwrap();
        let cmd = Cmd::AgentResume(v1::AgentResume {
            agent_id: id,
            new_session: true,
            authorization: None,
        });
        let socket = self.node.as_ref().unwrap().socket().unwrap().to_path_buf();
        let resumer =
            std::thread::spawn(move || Client::connect(&socket).unwrap().call(cmd).unwrap());
        let end = Instant::now() + Duration::from_secs(20);
        while !self.gate.join("reached").exists() {
            assert!(Instant::now() < end, "the resume never reached the gate");
            std::thread::yield_now();
        }
        change(&own, &other);
        std::fs::write(self.gate.join("release"), "").unwrap();
        (x, other.join("x"), resumer.join().unwrap())
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        self.node.take();
        let _ = std::fs::remove_file(&self.socket);
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn refused(answer: &Result<v1::CommandOk, v1::ErrorInfo>) -> bool {
    matches!(answer, Err(e) if e.code == "agent.folder_changed")
}

#[test]
fn a_resume_starts_in_the_folder_it_checked_or_not_at_all() {
    // The gate is a shell script named `git` and the swaps make links: where either is unavailable (Windows), nothing to run.
    let probe = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("resume-folder-swap-link-{}", std::process::id()));
    let _ = std::fs::remove_file(&probe);
    let links = agora_platform::symlink(Path::new("."), &probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    if !(links && agora_platform::PTY_SUPPORTED && agora_platform::LOCAL_SOCKETS_SUPPORTED) {
        return;
    }
    let link = |to: &Path, at: &Path| agora_platform::symlink(to, at).unwrap();
    let mut e = Env::new("cases");

    // Unchanged: resumed in its folder.
    let (x, _, answer) = e.resume_while("control", |_, _| {});
    assert!(answer.is_ok(), "{answer:?}");
    assert_eq!(e.next_report().as_deref(), Some(x.to_str().unwrap()));

    // Swapped and swapped back while the check waits: the folder checked is there again, resumed in it.
    let (x, _, answer) = e.resume_while("undone", |own, other| {
        std::fs::rename(own.join("x"), own.join("x.old")).unwrap();
        link(&other.join("x"), &own.join("x"));
        std::fs::remove_file(own.join("x")).unwrap();
        std::fs::rename(own.join("x.old"), own.join("x")).unwrap();
    });
    assert!(answer.is_ok(), "{answer:?}");
    assert_eq!(e.next_report().as_deref(), Some(x.to_str().unwrap()));

    // The folder swapped for a link to another project's folder: refused, nothing runs there.
    let (_, _, answer) = e.resume_while("folder", |own, other| {
        std::fs::rename(own.join("x"), own.join("x.old")).unwrap();
        link(&other.join("x"), &own.join("x"));
    });
    assert!(refused(&answer), "{answer:?}");
    assert_eq!(e.running(), 0, "nothing runs after a refused resume");

    // A folder above it swapped for a link: refused.
    let (_, _, answer) = e.resume_while("above", |own, other| {
        std::fs::rename(own, own.with_file_name("own.old")).unwrap();
        link(other, own);
    });
    assert!(refused(&answer), "{answer:?}");
    assert_eq!(e.running(), 0, "nothing runs after a refused resume");

    // No link at all: the folder moved away and a look-alike with the same identity put in its place after the check: refused.
    let (_, _, answer) = e.resume_while("look-alike", |own, _| {
        std::fs::rename(own.join("x"), own.join("x.old")).unwrap();
        std::fs::create_dir(own.join("x")).unwrap();
        std::fs::copy(
            own.join("x.old/.agent-project.json"),
            own.join("x/.agent-project.json"),
        )
        .unwrap();
    });
    assert!(refused(&answer), "{answer:?}");
    assert_eq!(e.running(), 0, "nothing runs after a refused resume");
    drop(e);
}

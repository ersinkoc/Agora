//! ADR-0084 (issue #546, second half): a confined agent gets its login from a secret the node holds. Real spawns of a real shell as
//! the "agent" (it dumps its own environment), a real store of owner-only files, the real policy parser. No accounts, tokens or
//! real coding agents: the "secrets" are made-up strings.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "private_home/mod.rs"]
mod private_home;
#[path = "short_socket/mod.rs"]
mod short_socket;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use agora_adapters::Manifest;
use agora_core::{AgentId, NodeId};
use agora_node::credentials::{Credentials, Secret, SecretStore};
use agora_node::daemon::read_policy;
use agora_node::supervisor::{
    NodeLimits, RestartPolicy, Restore, StartRequest, Supervisor, SupervisorConfig, SupervisorError,
};
use agora_node::terminal::TerminalOptions;

fn git(dir: &Path, args: &[&str]) {
    assert!(Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .status()
        .unwrap()
        .success());
}

/// A kind that dumps its environment and what it can read of the secrets folder into its working folder, then waits.
fn manifest(kind: &str, declares: &[&str], secrets: &Path) -> Manifest {
    let script = format!(
        "env > probe.tmp && mv probe.tmp probe.txt; \
         tr '\\0' ' ' < /proc/$$/cmdline > cmdline.txt 2>/dev/null; \
         cat {}/* > read.tmp 2> read.err; rc=$?; \
         echo $rc > read.rc.tmp; mv read.rc.tmp read.rc; mv read.tmp read.txt; sleep 60",
        secrets.display()
    );
    let launch = serde_json::to_string(&["/bin/sh", "-c", &script]).unwrap();
    let names = serde_json::to_string(declares).unwrap();
    Manifest::parse(&format!(
        "kind = \"{kind}\"\ndisplay_name = \"{kind}\"\nversion = \"1\"\nmin_engine = 4\n[process]\nlaunch = {launch}\n[accounts]\ncredential_env = {names}\n"
    ))
    .unwrap()
}

struct Setup {
    root: PathBuf,
    repo: PathBuf,
}

impl Setup {
    /// Under the target folder, not `/tmp`: a confined node refuses to start where agents may write (ADR-0082).
    fn new(name: &str) -> Self {
        private_home::init();
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("cred-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(root.join("data")).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "x").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "init"]);
        Self { root, repo }
    }

    fn data(&self) -> PathBuf {
        self.root.join("data")
    }

    fn store(&self) -> SecretStore {
        SecretStore::in_data(&self.data())
    }

    fn put(&self, name: &str, value: &str) {
        self.store()
            .set(name, &Secret::new(value).unwrap())
            .unwrap();
    }

    /// A supervisor as the node builds it: the policy file's sandbox and credentials lines, the kinds, the store.
    fn supervisor(&self, policy: &str, kinds: &[(&str, &[&str])]) -> Supervisor {
        std::fs::write(self.data().join("policy"), policy).unwrap();
        let p = read_policy(&self.data().join("policy"));
        let mut sup = Supervisor::new(SupervisorConfig {
            node: NodeId::new("node-cred"),
            restart: RestartPolicy::default(),
            terminal: TerminalOptions::default(),
            limits: NodeLimits::default(),
        });
        let mut sandbox = p.sandbox;
        sandbox.protect = vec![self.root.clone()];
        sup.set_sandbox(sandbox);
        sup.set_credentials(Credentials::new(self.store(), p.credentials));
        for (kind, names) in kinds {
            sup.register(manifest(kind, names, self.store().dir()));
        }
        sup
    }

    fn request(&self, kind: &str, name: &str) -> StartRequest {
        StartRequest {
            kind: kind.into(),
            checkout: self.repo.clone(),
            worktree_root: self.root.join("worktrees"),
            name: name.into(),
            base: None,
        }
    }

    fn worktree(&self, name: &str) -> PathBuf {
        self.root.join("worktrees").join(name)
    }

    /// The environment the agent in `name` dumped, as lines.
    fn probe(&self, name: &str) -> Vec<String> {
        let file = self.worktree(name).join("probe.txt");
        let end = Instant::now() + Duration::from_secs(15);
        while !file.exists() {
            assert!(Instant::now() < end, "the agent never reported");
            std::thread::sleep(Duration::from_millis(20));
        }
        std::fs::read_to_string(file)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn read(&self, name: &str, file: &str) -> String {
        let p = self.worktree(name).join(file);
        let end = Instant::now() + Duration::from_secs(15);
        while !p.exists() {
            assert!(Instant::now() < end, "no {file}");
            std::thread::sleep(Duration::from_millis(20));
        }
        std::fs::read_to_string(p).unwrap()
    }
}

impl Drop for Setup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn has(env: &[String], line: &str) -> bool {
    env.iter().any(|l| l == line)
}

fn names(env: &[String], name: &str) -> bool {
    env.iter().any(|l| l.starts_with(&format!("{name}=")))
}

#[test]
fn a_kind_gets_its_listed_secret_and_no_other_kind_does() {
    let s = Setup::new("kinds");
    s.put("CRA_TOKEN", "stored-value-a");
    s.put("CRB_TOKEN", "stored-value-b");
    // The node's own environment carries the OTHER kind's token and one that no manifest names.
    std::env::set_var("CRB_TOKEN", "node-env-b");
    std::env::set_var("CR_KEEP_ME", "kept");
    let mut sup = s.supervisor(
        "credentials.cra = CRA_TOKEN\n",
        &[("cra", &["CRA_TOKEN"]), ("crb", &["CRB_TOKEN"])],
    );
    let now = Instant::now();
    sup.start(&s.request("cra", "a1"), now).unwrap();
    // Kind b stands for a helper of another kind (ADR-0057): it starts through the same path as its own kind.
    sup.start(&s.request("crb", "b1"), now).unwrap();
    let a = s.probe("a1");
    let b = s.probe("b1");
    assert!(has(&a, "CRA_TOKEN=stored-value-a"));
    // Scrub: B's declared name is not inherited by A, though the node's environment has it.
    assert!(!names(&a, "CRB_TOKEN"), "{a:?}");
    // Not a credential name of any kind: untouched.
    assert!(has(&a, "CR_KEEP_ME=kept"));
    // Kind b has no policy line: neither the stored secret of A nor its own name (node environment) reaches it.
    assert!(!names(&b, "CRA_TOKEN"), "{b:?}");
    assert!(!names(&b, "CRB_TOKEN"), "{b:?}");
    assert!(has(&b, "CR_KEEP_ME=kept"));
    // The value is in the environment only: not in the command line.
    if let Ok(cmdline) = std::fs::read_to_string(s.worktree("a1").join("cmdline.txt")) {
        assert!(!cmdline.contains("stored-value-a"));
    }
    for id in sup
        .list()
        .iter()
        .map(|a| a.id.clone())
        .collect::<Vec<AgentId>>()
    {
        sup.stop(&id).unwrap();
    }
}

#[test]
fn the_nodes_environment_is_the_fallback_for_the_kinds_own_listed_name() {
    let s = Setup::new("fallback");
    std::env::set_var("CRF_TOKEN", "from-node-env");
    let mut sup = s.supervisor("credentials.crf = CRF_TOKEN\n", &[("crf", &["CRF_TOKEN"])]);
    sup.start(&s.request("crf", "f1"), Instant::now()).unwrap();
    assert!(has(&s.probe("f1"), "CRF_TOKEN=from-node-env"));
    // A stored secret wins over the environment.
    s.put("CRF_TOKEN", "stored-wins");
    sup.start(&s.request("crf", "f2"), Instant::now()).unwrap();
    assert!(has(&s.probe("f2"), "CRF_TOKEN=stored-wins"));
    // Negative control: without the policy line the kind gets nothing, from either source.
    let mut sup2 = s.supervisor("", &[("crf", &["CRF_TOKEN"])]);
    sup2.start(&s.request("crf", "f3"), Instant::now()).unwrap();
    assert!(!names(&s.probe("f3"), "CRF_TOKEN"));
}

#[test]
fn a_malformed_policy_line_passes_nothing() {
    let s = Setup::new("malformed");
    s.put("CRM_TOKEN", "stored-m");
    std::env::set_var("CRM_TOKEN", "env-m");
    for (i, line) in [
        "credentials.crm = CRM_TOKEN, X=1",
        "credentials.crm = CRM_TOKEN, *",
        "credentials.crm = 1BAD",
        "credentials.crm = ",
        "credentials.crm = CRM_TOKEN, crm_token",
    ]
    .iter()
    .enumerate()
    {
        let mut sup = s.supervisor(&format!("{line}\n"), &[("crm", &["CRM_TOKEN"])]);
        let name = format!("m{i}");
        sup.start(&s.request("crm", &name), Instant::now()).unwrap();
        let env = s.probe(&name);
        assert!(!names(&env, "CRM_TOKEN"), "{line}: {env:?}");
        assert!(!env
            .iter()
            .any(|l| l.contains("stored-m") || l.contains("env-m")));
    }
    // Positive control: the well-formed line passes it.
    let mut sup = s.supervisor("credentials.crm = CRM_TOKEN\n", &[("crm", &["CRM_TOKEN"])]);
    sup.start(&s.request("crm", "ok"), Instant::now()).unwrap();
    assert!(has(&s.probe("ok"), "CRM_TOKEN=stored-m"));
}

#[test]
fn a_listed_name_the_manifest_does_not_declare_is_not_passed() {
    let s = Setup::new("undeclared");
    s.put("CRU_OTHER", "undeclared-value");
    let mut sup = s.supervisor("credentials.cru = CRU_OTHER\n", &[("cru", &["CRU_TOKEN"])]);
    sup.start(&s.request("cru", "u1"), Instant::now()).unwrap();
    assert!(!names(&s.probe("u1"), "CRU_OTHER"));
}

#[test]
fn a_resumed_agent_gets_its_secret_again() {
    let s = Setup::new("resume");
    s.put("CRR_TOKEN", "resume-me");
    let policy = "credentials.crr = CRR_TOKEN\n";
    let kinds: &[(&str, &[&str])] = &[("crr", &["CRR_TOKEN"])];
    let mut sup = s.supervisor(policy, kinds);
    let id = sup.start(&s.request("crr", "r1"), Instant::now()).unwrap();
    assert!(has(&s.probe("r1"), "CRR_TOKEN=resume-me"));
    let project = agora_node::git::discover_checkout(&s.repo).unwrap().project;
    sup.stop(&id).unwrap();
    drop(sup);
    // The node restarted: a new supervisor, the agent restored from the ledger, then resumed (ADR-0060).
    std::fs::remove_file(s.worktree("r1").join("probe.txt")).unwrap();
    let mut sup = s.supervisor(policy, kinds);
    sup.restore(Restore {
        id: id.clone(),
        kind: "crr".into(),
        project,
        folder: s.worktree("r1"),
        directory: false,
        branch: "agent/r1".into(),
        session: None,
        carry_on: false,
        private_config: None,
    })
    .unwrap();
    sup.resume(&id, Instant::now()).unwrap();
    assert!(has(&s.probe("r1"), "CRR_TOKEN=resume-me"));
    // And a resume under a policy that no longer lists it passes nothing.
    sup.stop(&id).unwrap();
    std::fs::remove_file(s.worktree("r1").join("probe.txt")).unwrap();
    let mut sup = s.supervisor("", kinds);
    let project = agora_node::git::discover_checkout(&s.repo).unwrap().project;
    sup.restore(Restore {
        id: id.clone(),
        kind: "crr".into(),
        project,
        folder: s.worktree("r1"),
        directory: false,
        branch: "agent/r1".into(),
        session: None,
        carry_on: false,
        private_config: None,
    })
    .unwrap();
    sup.resume(&id, Instant::now()).unwrap();
    assert!(!names(&s.probe("r1"), "CRR_TOKEN"));
    sup.stop(&id).unwrap();
}

/// Landlock (Linux) or Seatbelt (macOS): a confined agent gets its own secret and cannot read the folder the secrets are in.
#[test]
fn a_confined_agent_gets_the_secret_and_cannot_read_the_store() {
    if !agora_platform::sandbox_supported() {
        eprintln!("skipped: no confinement here");
        return;
    }
    let s = Setup::new("confined");
    s.put("CRC_TOKEN", "confined-value");
    s.put("CRC_OTHER", "another-secret-the-agent-must-not-read");
    let mut sup = s.supervisor(
        "sandbox = worktree\ncredentials.crc = CRC_TOKEN\n",
        &[("crc", &["CRC_TOKEN"])],
    );
    sup.start(&s.request("crc", "c1"), Instant::now()).unwrap();
    assert!(has(&s.probe("c1"), "CRC_TOKEN=confined-value"));
    // The agent got the value it was given, but could not read the store: the read failed and nothing of it was kept.
    let rc = s.read("c1", "read.rc");
    assert_ne!(
        rc.trim(),
        "0",
        "the confined agent could read the secrets folder"
    );
    let read = s.read("c1", "read.txt");
    assert!(!read.contains("another-secret"), "{read}");
    assert!(!read.contains("confined-value"), "{read}");
    // Negative control: unconfined, the same probe can read it.
    let s2 = Setup::new("unconfined");
    s2.put("CRC_TOKEN", "confined-value");
    let mut sup2 = s2.supervisor("credentials.crc = CRC_TOKEN\n", &[("crc", &["CRC_TOKEN"])]);
    sup2.start(&s2.request("crc", "c2"), Instant::now())
        .unwrap();
    s2.probe("c2");
    assert!(s2.read("c2", "read.txt").contains("confined-value"));
}

#[test]
fn a_grant_that_reaches_the_secrets_folder_refuses_the_start() {
    if !agora_platform::sandbox_supported() {
        return;
    }
    let s = Setup::new("reach");
    s.put("CRG_TOKEN", "x");
    let policy = format!(
        "sandbox = worktree\nsandbox.read = {}\ncredentials.crg = CRG_TOKEN\n",
        s.data().display()
    );
    let mut sup = s.supervisor(&policy, &[("crg", &["CRG_TOKEN"])]);
    let e = sup
        .start(&s.request("crg", "g1"), Instant::now())
        .unwrap_err();
    assert!(
        matches!(
            e,
            SupervisorError::SandboxRefused(_, _, "secrets_reachable")
        ),
        "{e:?}"
    );
}

/// The value is nowhere a node writes: not in the event store, the ledger, or any file of the node's data folder but the store itself.
#[test]
fn the_value_is_in_no_file_of_the_node() {
    use agora_node::control::Client;
    use agora_node::daemon::{Node, NodeConfig};
    use agora_protocol::v1::{self, command_ok::Result as Ok_, command_request::Command as Cmd};

    let s = Setup::new("nofile");
    let value = "never-written-anywhere-9f3a1c";
    s.put("CRN_TOKEN", value);
    std::fs::write(s.data().join("policy"), "credentials.crn = CRN_TOKEN\n").unwrap();
    std::fs::create_dir_all(s.root.join("manifests")).unwrap();
    let m = manifest("crn", &["CRN_TOKEN"], s.store().dir());
    let launch = serde_json::to_string(&m.process.launch).unwrap();
    std::fs::write(
        s.root.join("manifests/crn.toml"),
        format!(
            "kind = \"crn\"\ndisplay_name = \"crn\"\nversion = \"1\"\nmin_engine = 4\n[process]\nlaunch = {launch}\n[accounts]\ncredential_env = [\"CRN_TOKEN\"]\n"
        ),
    )
    .unwrap();
    // #926: the socket's folder fits whatever the checkout's depth, and its control token is out of the agent's reach.
    let sockets = short_socket::SocketDir::new(&s.root, "cred-nofile");
    let node = Node::start(&NodeConfig {
        resume_all: false,
        node_id: "node-nofile".into(),
        socket: sockets.socket("node.sock"),
        data_dir: s.data(),
        worktree_root: s.root.join("worktrees"),
        manifests_dir: s.root.join("manifests"),
        projects: vec![s.repo.clone()],
        limits: NodeLimits::default(),
    })
    .unwrap();
    let mut c = Client::connect(node.socket().unwrap()).unwrap();
    let started = c
        .call(Cmd::AgentStart(v1::AgentStart {
            kind: "crn".into(),
            ..Default::default()
        }))
        .unwrap()
        .unwrap();
    let Some(Ok_::AgentStart(started)) = started.result else {
        panic!("not started");
    };
    // The agent really has it ...
    let dir = std::fs::read_dir(s.root.join("worktrees"))
        .unwrap()
        .flatten()
        .next()
        .unwrap()
        .path();
    let end = Instant::now() + Duration::from_secs(15);
    while !dir.join("probe.txt").exists() {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(20));
    }
    let probe = std::fs::read_to_string(dir.join("probe.txt")).unwrap();
    assert!(probe.contains(&format!("CRN_TOKEN={value}")));
    // ... and nothing the node keeps does. Give the event sink a moment to write.
    std::thread::sleep(Duration::from_millis(500));
    drop(node);
    let mut looked = 0;
    let mut stack = vec![s.data()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p == s.store().dir() {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(bytes) = std::fs::read(&p) {
                looked += 1;
                assert!(
                    !bytes.windows(value.len()).any(|w| w == value.as_bytes()),
                    "the value is in {}",
                    p.display()
                );
            }
        }
    }
    assert!(looked > 1, "the data folder was not searched");
    let _ = started;
}

#[test]
fn the_policy_file_lists_names_per_kind_and_a_bad_line_lists_none() {
    let s = Setup::new("policy");
    let read = |text: &str| {
        std::fs::write(s.data().join("policy"), text).unwrap();
        read_policy(&s.data().join("policy"))
    };
    let p = read(
        "credentials.claude-code = A_TOKEN, B_KEY\ncredentials.codex=OPENAI_API_KEY\n# comment\n",
    );
    assert_eq!(p.credentials["claude-code"], ["A_TOKEN", "B_KEY"]);
    assert_eq!(p.credentials["codex"], ["OPENAI_API_KEY"]);
    assert!(p.problems.is_empty() && p.credentials_malformed.is_empty());
    // A bad line empties that kind only and is remembered for `agora doctor`; the later good line for the same kind wins.
    let p = read("credentials.codex = A=1\ncredentials.claude-code = A_TOKEN\n");
    assert!(!p.credentials.contains_key("codex"));
    assert_eq!(p.credentials_malformed, ["codex"]);
    assert_eq!(p.credentials["claude-code"], ["A_TOKEN"]);
    let p = read("credentials.codex = A=1\ncredentials.codex = OK_NAME\n");
    assert_eq!(p.credentials["codex"], ["OK_NAME"]);
    // A key that names no kind is a problem line (and forbids skipping permissions, as any line the node does not understand).
    let p = read("credentials. = A_TOKEN\ncredentials.Bad_Kind = A_TOKEN\n");
    assert!(p.credentials.is_empty());
    assert_eq!(p.problems.len(), 2);
}

/// `agora-fake-agent --adapter`, built once per test binary.
fn fake_agent() -> String {
    let mut p = std::env::current_exe().expect("exe");
    p.pop();
    p.pop();
    p.push(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
    static FRESH: std::sync::Once = std::sync::Once::new();
    FRESH.call_once(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        assert!(Command::new(cargo)
            .args(["build", "-q", "-p", "agora-fake-agent"])
            .status()
            .expect("run cargo")
            .success());
    });
    p.to_string_lossy().into_owned()
}

/// Audit F830: the refusal holds for an adapter (L2/L3) kind too, at its first start and when it is resumed after a secret was stored.
/// The adapter copies what it can read of the store into its folder before it becomes the fake adapter.
#[test]
fn a_grant_that_reaches_the_secrets_folder_refuses_an_adapter_start_and_resume() {
    if !agora_platform::sandbox_supported() {
        return;
    }
    let s = Setup::new("reach-adapter");
    let fake = fake_agent();
    let policy = format!(
        "sandbox = worktree\nsandbox.read = {}, {}\ncredentials.cra = CRA_TOKEN\n",
        s.data().display(),
        Path::new(&fake).parent().unwrap().display()
    );
    std::fs::write(s.data().join("policy"), &policy).unwrap();
    let mut sup = s.supervisor(&policy, &[]);
    let command = serde_json::to_string(&[
        "/bin/sh",
        "-c",
        &format!(
            "cat {}/* > read.txt 2> /dev/null; exec {fake} --adapter",
            s.store().dir().display()
        ),
    ])
    .unwrap();
    sup.register(
        Manifest::parse(&format!(
            "kind = \"cra\"\ndisplay_name = \"cra\"\nversion = \"1\"\nmin_engine = 4\n[adapter]\ncommand = {command}\n[accounts]\ncredential_env = [\"CRA_TOKEN\"]\n"
        ))
        .unwrap(),
    );
    // An empty store does not refuse: the adapter starts, then is stopped.
    let id = sup.start(&s.request("cra", "a1"), Instant::now()).unwrap();
    sup.stop(&id).unwrap();
    let _ = std::fs::remove_file(s.worktree("a1").join("read.txt"));
    s.put("CRA_OTHER", "a-secret-the-adapter-must-not-read");
    let refused = |e: SupervisorError| {
        matches!(
            e,
            SupervisorError::SandboxRefused(_, _, "secrets_reachable")
        )
    };
    // Resumed once the store holds a secret: refused, nothing read.
    assert!(refused(sup.resume(&id, Instant::now()).unwrap_err()));
    assert!(!s.worktree("a1").join("read.txt").exists());
    // A first start: refused, nothing read.
    assert!(refused(
        sup.start(&s.request("cra", "a2"), Instant::now())
            .unwrap_err()
    ));
    assert!(!s.worktree("a2").join("read.txt").exists());
}

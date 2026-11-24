//! ADR-0101 end to end: a node runs the fake ACP agent behind the real bridge (`agora acp-bridge`), the agent offers each combination
//! of ACP's option kinds, and a trusted device (a real P-256 key) answers. A single Allow never reaches the agent as `allow_always`,
//! "always" selects only the one `allow_always`, the policy and the offer are checked on the node, and an always answer the agent
//! took is recorded. The agent is the fake ACP agent, never a real one.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use agora_node::approvals::{always_signed_message, signed_message, Pending};
use agora_node::control::Client;
use agora_node::daemon::{Node, NodeConfig};
use agora_protocol::v1::{self, command_request::Command as Cmd, event::Body, ApprovalScope};
use ring::rand::SystemRandom;
use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};

const AGORA: &str = env!("CARGO_BIN_EXE_agora");

fn fake_agent() -> String {
    let mut p = PathBuf::from(AGORA);
    p.pop();
    p.push(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
    static FRESH: std::sync::Once = std::sync::Once::new();
    FRESH.call_once(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        assert!(
            Command::new(cargo)
                .args(["build", "-q", "-p", "agora-fake-agent"])
                .status()
                .expect("run cargo")
                .success(),
            "building agora-fake-agent failed"
        );
    });
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

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

struct Env {
    dir: PathBuf,
    node: Node,
    key: EcdsaKeyPair,
    rng: SystemRandom,
}

impl Drop for Env {
    fn drop(&mut self) {
        let ids: Vec<_> = self
            .node
            .supervisor
            .lock()
            .unwrap()
            .list()
            .into_iter()
            .filter(|v| v.running)
            .map(|v| v.id)
            .collect();
        for id in ids {
            let _ = self.node.supervisor.lock().unwrap().stop(&id);
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A node with the fake ACP kind, `policy` in its policy file, and the device `phone` trusted.
fn env(tag: &str, policy: Option<&str>) -> Option<Env> {
    env_with(tag, policy, None)
}

/// [`env`] on a node that holds `secret` in its store (#1147).
fn env_with(tag: &str, policy: Option<&str>, secret: Option<&str>) -> Option<Env> {
    if !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return None;
    }
    // The node runs `acp-bridge` from this binary, not from the test's own.
    std::env::set_var("AGORA_ACP_BRIDGE", AGORA);
    let dir = std::env::temp_dir().join(format!("agora-always-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("demo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    std::fs::create_dir_all(dir.join("manifests")).unwrap();
    std::fs::write(
        dir.join("manifests/acp-fake.toml"),
        format!("kind = \"acp-fake\"\ndisplay_name = \"Fake ACP\"\nversion = \"1\"\nmin_engine = 1\n[structured]\nacp = {{ argv = [{:?}, \"--acp\"] }}\n", fake_agent()),
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("data")).unwrap();
    if let Some(v) = secret {
        agora_node::credentials::SecretStore::in_data(&dir.join("data"))
            .set("TOKEN", &agora_node::credentials::Secret::new(v).unwrap())
            .unwrap();
    }
    if let Some(p) = policy {
        std::fs::write(dir.join("data/policy"), p).unwrap();
    }
    let rng = SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
    let key =
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng).unwrap();
    agora_node::approvals::trust_device(
        &dir.join("data/trusted-devices"),
        "phone",
        &hex(key.public_key().as_ref()),
    )
    .unwrap();
    let node = Node::start(&NodeConfig {
        resume_all: false,
        node_id: "laptop".into(),
        socket: dir.join("c.sock"),
        data_dir: dir.join("data"),
        worktree_root: dir.join("wt"),
        manifests_dir: dir.join("manifests"),
        projects: vec![repo],
        limits: Default::default(),
    })
    .expect("start node");
    Some(Env {
        dir,
        node,
        key,
        rng,
    })
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

impl Env {
    fn client(&self) -> Client {
        Client::connect(self.node.socket().expect("control socket")).unwrap()
    }

    fn call(&self, cmd: Cmd) -> Result<v1::CommandOk, v1::ErrorInfo> {
        self.client().call(cmd).unwrap()
    }

    fn start(&self) -> String {
        let ok = self
            .call(Cmd::AgentStart(v1::AgentStart {
                kind: "acp-fake".into(),
                ..Default::default()
            }))
            .unwrap();
        let Some(v1::command_ok::Result::AgentStart(s)) = ok.result else {
            panic!("not started");
        };
        let id = s.agent_id;
        wait("the agent's session", || {
            let views = self.node.supervisor.lock().unwrap().list();
            views
                .iter()
                .any(|v| v.id.as_str() == id && v.state == agora_core::AgentState::Idle)
                .then_some(())
        });
        id
    }

    fn ask(&self, agent: &str, kinds: &str) {
        self.call(Cmd::AgentPrompt(v1::AgentPrompt {
            agent_id: agent.into(),
            text: format!("ask-kinds {kinds}"),
        }))
        .unwrap();
    }

    fn events(&self) -> Vec<Body> {
        self.node
            .events
            .events_after(0)
            .into_iter()
            .filter_map(|e| e.body)
            .collect()
    }

    /// The latest request announced for `agent` after `seen` requests.
    fn request(&self, agent: &str, seen: usize) -> v1::ApprovalRequested {
        wait("an approval request", || {
            self.events()
                .into_iter()
                .filter_map(|b| match b {
                    Body::ApprovalRequested(r) if r.agent_id == agent => Some(r),
                    _ => None,
                })
                .nth(seen)
        })
    }

    fn said(&self, agent: &str, text: &str) -> bool {
        self.node
            .chat
            .snapshot(&agora_core::AgentId::new(agent))
            .iter()
            .any(|e| e.kind == v1::ChatKind::Text as i32 && e.text == text)
    }

    fn wait_said(&self, agent: &str, text: &str) {
        wait(text, || self.said(agent, text).then_some(()));
    }

    fn sign(&self, msg: &[u8]) -> Vec<u8> {
        self.key.sign(&self.rng, msg).unwrap().as_ref().to_vec()
    }

    /// A decision signed over the bytes the scope `signed_always` names, sent with `scope`.
    fn decide(
        &self,
        r: &v1::ApprovalRequested,
        approved: bool,
        signed_always: bool,
        scope: ApprovalScope,
    ) -> Result<v1::CommandOk, v1::ErrorInfo> {
        let msg = if signed_always {
            always_signed_message("laptop", &r.approval_id, &pending(r))
        } else {
            signed_message("laptop", &r.approval_id, approved)
        };
        self.call(Cmd::ApprovalResolve(v1::ApprovalResolve {
            approval_id: r.approval_id.clone(),
            approved,
            signer_device_id: "phone".into(),
            signature: self.sign(&msg),
            scope: scope as i32,
            ..Default::default()
        }))
    }
}

/// The node's request as a window rebuilds it from what was announced (what an always decision is signed over).
fn pending(r: &v1::ApprovalRequested) -> Pending {
    Pending {
        id: r.approval_id.clone(),
        agent_id: r.agent_id.clone(),
        action: r.action.clone(),
        detail: r.detail.clone(),
        requested_ms: 0,
        expires_ms: r.expires_at_ms,
        binding: None,
        offer: agora_node::approvals::Offer {
            once: r.scopes.contains(&(ApprovalScope::Once as i32)),
            always: (!r.always_option_id.is_empty()).then(|| agora_node::approvals::AlwaysOption {
                label: r.always_label.clone(),
                option_id: r.always_option_id.clone(),
            }),
        },
    }
}

fn resolved(e: &Env, id: &str) -> Option<v1::ApprovalResolved> {
    e.events().into_iter().find_map(|b| match b {
        Body::ApprovalResolved(r) if r.approval_id == id => Some(r),
        _ => None,
    })
}

fn always_records(e: &Env, agent: &str) -> Vec<v1::AgentAlwaysAllowed> {
    e.events()
        .into_iter()
        .filter_map(|b| match b {
            Body::AgentAlwaysAllowed(a) if a.agent_id == agent => Some(a),
            _ => None,
        })
        .collect()
}

#[test]
fn the_person_answers_once_or_always_and_the_agent_gets_exactly_that() {
    let Some(e) = env("answers", None) else {
        return;
    };
    let once = ApprovalScope::Once as i32;
    let always = ApprovalScope::Always as i32;
    let agent = e.start();

    // All four kinds: the card offers both answers; a single Allow selects `allow_once`, never `allow_always`.
    e.ask(&agent, "reject_always,allow_always,reject_once,allow_once");
    let r = e.request(&agent, 0);
    assert_eq!(r.scopes, vec![once, always]);
    assert_eq!(
        (r.always_label.as_str(), r.always_option_id.as_str()),
        ("Name of allow_always", "allow_always")
    );
    e.decide(&r, true, false, ApprovalScope::Once).unwrap();
    e.wait_said(&agent, "selected:allow_once");
    assert_eq!(resolved(&e, &r.approval_id).unwrap().scope, once);
    assert!(always_records(&e, &agent).is_empty());

    // A hub that turns that single answer into "always" (or the reverse) fails the signature; the card stays, nothing is sent.
    e.ask(&agent, "reject_always,allow_always,reject_once,allow_once");
    let r = e.request(&agent, 1);
    assert_eq!(
        e.decide(&r, true, false, ApprovalScope::Always)
            .unwrap_err()
            .code,
        "approval.signature_invalid"
    );
    assert_eq!(
        e.decide(&r, true, true, ApprovalScope::Once)
            .unwrap_err()
            .code,
        "approval.signature_invalid"
    );
    assert!(
        resolved(&e, &r.approval_id).is_none(),
        "the card still waits"
    );
    // The person's "always": only `allow_always` is selected, the audit record says ALWAYS, and the agent holds it.
    e.decide(&r, true, true, ApprovalScope::Always).unwrap();
    e.wait_said(&agent, "selected:allow_always");
    let done = resolved(&e, &r.approval_id).unwrap();
    assert_eq!(
        (done.approved, done.scope, done.signer_device_id.as_str()),
        (true, always, "phone")
    );
    let held = always_records(&e, &agent);
    assert_eq!(held.len(), 1);
    assert_eq!(
        (held[0].approval_id.as_str(), held[0].action.as_str()),
        (r.approval_id.as_str(), "run_command")
    );
    let entry = e
        .node
        .chat
        .snapshot(&agora_core::AgentId::new(&agent))
        .into_iter()
        .find(|c| c.approval_id == r.approval_id)
        .unwrap();
    assert!(entry.permission_always);
    assert_eq!(
        entry.permission_outcome,
        v1::PermissionOutcome::Approved as i32
    );

    // An agent that offers only "always": a single Allow is refused before anything is sent (the card stays); "always" works.
    e.ask(&agent, "allow_always,reject_once");
    let r = e.request(&agent, 2);
    assert_eq!(r.scopes, vec![always]);
    assert_eq!(
        e.decide(&r, true, false, ApprovalScope::Once)
            .unwrap_err()
            .code,
        "approval.scope_unavailable"
    );
    assert_eq!(
        e.decide(&r, true, false, ApprovalScope::Unspecified)
            .unwrap_err()
            .code,
        "approval.scope_unavailable"
    );
    assert!(resolved(&e, &r.approval_id).is_none());
    e.decide(&r, true, true, ApprovalScope::Always).unwrap();
    e.wait_said(&agent, "selected:allow_always");

    // A single answer only: "always" is not offered and is refused.
    e.ask(&agent, "allow_once,reject_once,something_new");
    let r = e.request(&agent, 3);
    assert!(r.scopes.is_empty() && r.always_option_id.is_empty());
    assert_eq!(
        e.decide(&r, true, true, ApprovalScope::Always)
            .unwrap_err()
            .code,
        "approval.scope_unavailable"
    );
    // A denial picks `reject_once`.
    e.decide(&r, false, false, ApprovalScope::Once).unwrap();
    e.wait_said(&agent, "selected:reject_once");

    // Two always options are ambiguous: only the single answer is offered.
    e.ask(&agent, "allow_always,allow_always2,allow_once");
    let r = e.request(&agent, 4);
    assert!(r.scopes.is_empty());
    e.decide(&r, true, false, ApprovalScope::Once).unwrap();
    e.wait_said(&agent, "selected:allow_once");

    // Nothing to allow (rejections and an unknown kind): no card, the agent is refused (`reject_once`, else cancelled), the person
    // is told.
    let requests = e
        .events()
        .iter()
        .filter(|b| matches!(b, Body::ApprovalRequested(_)))
        .count();
    e.ask(&agent, "reject_always,reject_once");
    e.wait_said(&agent, "selected:reject_once");
    e.ask(&agent, "something_new");
    e.wait_said(&agent, "cancelled");
    assert_eq!(
        e.events()
            .iter()
            .filter(|b| matches!(b, Body::ApprovalRequested(_)))
            .count(),
        requests,
        "no card"
    );
    let notices = e
        .node
        .chat
        .snapshot(&agora_core::AgentId::new(&agent))
        .into_iter()
        .filter(|c| c.code == "adapter.permission_unanswerable")
        .count();
    assert_eq!(notices, 2);
    // The session ledger keeps the agent's always answers for a node that takes it up (ADR-0093); when the agent ends, they end with
    // it.
    assert_eq!(e.node.ledger.kept_session(&agent).always.len(), 2);
    e.call(Cmd::AgentStop(v1::AgentStop {
        agent_id: agent.clone(),
        ..Default::default()
    }))
    .unwrap();
    wait("the kept always answers to go", || {
        e.node
            .ledger
            .kept_session(&agent)
            .always
            .is_empty()
            .then_some(())
    });
}

/// `skip_permissions = forbid`, and `confined_only` on a node that does not confine its agents (this test's, `sandbox` off): "always" is
/// never offered, never accepted, and an agent that offers only "always" gets no card: it is refused at once and the person is told why.
#[test]
fn a_policy_that_forbids_acting_unasked_forbids_always() {
    for policy in ["forbid", "confined_only"] {
        refused_under(policy);
    }
}

fn refused_under(policy: &str) {
    let Some(e) = env(policy, Some(&format!("skip_permissions = {policy}\n"))) else {
        return;
    };
    assert_eq!(
        e.node.policy_problems,
        Vec::<String>::new(),
        "{policy}: the policy file is read as written"
    );
    let agent = e.start();
    e.ask(&agent, "allow_once,allow_always,reject_once");
    let r = e.request(&agent, 0);
    assert!(r.scopes.is_empty(), "the always answer is not offered");
    // Even if a window signed one over the words the agent offered, the node refuses it.
    let mut forged = r.clone();
    forged.scopes = vec![ApprovalScope::Once as i32, ApprovalScope::Always as i32];
    forged.always_label = "Name of allow_always".into();
    forged.always_option_id = "allow_always".into();
    assert_eq!(
        e.decide(&forged, true, true, ApprovalScope::Always)
            .unwrap_err()
            .code,
        "approval.signature_invalid",
        "the node signs over what it announced"
    );
    e.decide(&r, true, false, ApprovalScope::Once).unwrap();
    e.wait_said(&agent, "selected:allow_once");

    // Always-only under `forbid`: the request is said and withdrawn, the agent refused, the notice given.
    e.ask(&agent, "allow_always,reject_once");
    e.wait_said(&agent, "selected:reject_once");
    let r = e.request(&agent, 1);
    let done = resolved(&e, &r.approval_id).unwrap();
    assert!(!done.approved && done.signer_device_id.is_empty());
    assert!(e
        .node
        .chat
        .snapshot(&agora_core::AgentId::new(&agent))
        .iter()
        .any(|c| c.code == "approval.always_only_forbidden"
            && c.params.get("action").map(String::as_str) == Some("run_command")));
    assert!(always_records(&e, &agent).is_empty());
}

/// #1147 (audit of #1157): a value the node holds that is the agent's name for its "always" option never reaches the card (the hub's
/// event); the always signature binds what the card showed, so the person's "always" still reaches the agent as `allow_always`.
#[test]
fn a_secret_in_the_always_options_name_never_reaches_the_card() {
    let Some(e) = env_with("secret-label", None, Some("Name of allow_always")) else {
        return;
    };
    let agent = e.start();
    e.ask(&agent, "reject_always,allow_always,reject_once,allow_once");
    let r = e.request(&agent, 0);
    assert_eq!(
        (r.always_label.as_str(), r.always_option_id.as_str()),
        ("<redacted>", "allow_always")
    );
    e.decide(&r, true, true, ApprovalScope::Always).unwrap();
    e.wait_said(&agent, "selected:allow_always");
}

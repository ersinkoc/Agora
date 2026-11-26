//! ADR-0090 Amendment A through a real node: switching a running agent to a mode or option that skips permission questions is an
//! approval the owner signs, and the node applies exactly what it rendered. The agent is `agora-fake-agent --adapter --modes`, never a
//! real one; a device "signs" by sending the bytes themselves (`EchoVerifier`), so the node's whole approval path runs.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use agora_node::approvals::{skip_signed_message, SkipBinding};
use agora_node::control::Client;
use agora_node::daemon::{Node, NodeConfig};
use agora_node::supervisor::NodeLimits;
use agora_protocol::v1::{self, command_ok::Result as Ok_, command_request::Command as Cmd};

type Reply = std::io::Result<Result<v1::CommandOk, v1::ErrorInfo>>;

struct EchoVerifier;

impl agora_node::approvals::Verifier for EchoVerifier {
    fn verify(&self, device: &str, message: &[u8], signature: &[u8]) -> bool {
        device == "phone" && message == signature
    }
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

fn fake_adapter_path() -> String {
    let mut p = std::env::current_exe().unwrap();
    p.pop();
    p.pop();
    p.push(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
    // Built once per test binary, even when present: a binary left from an older checkout may lack newer flags.
    static FRESH: std::sync::Once = std::sync::Once::new();
    FRESH.call_once(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        assert!(Command::new(cargo)
            .args(["build", "-q", "-p", "agora-fake-agent"])
            .status()
            .unwrap()
            .success());
    });
    p.to_string_lossy().into_owned()
}

struct Env {
    dir: PathBuf,
    repo: PathBuf,
}

/// A repository, manifests for the adapter kinds `modal` and `cfgmode` (whose mode is a config option of category `mode`) and, when
/// given, the node's policy file.
fn env(name: &str, policy: Option<&str>) -> Env {
    let dir = std::env::temp_dir().join(format!("agora-skip-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("myproject");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    let manifests = dir.join("manifests");
    std::fs::create_dir_all(&manifests).unwrap();
    std::fs::write(
        manifests.join("modal.toml"),
        format!("kind = \"modal\"\ndisplay_name = \"M\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = [{:?}, \"--adapter\", \"--modes\"]\n", fake_adapter_path()),
    )
    .unwrap();
    std::fs::write(
        manifests.join("cfgmode.toml"),
        format!("kind = \"cfgmode\"\ndisplay_name = \"C\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = [{:?}, \"--adapter\", \"--modes\", \"--mode-config\"]\n", fake_adapter_path()),
    )
    .unwrap();
    // P2-16: kinds whose agent starts in a mode, or with an option on, that skips permissions (its own default or a saved setting),
    // one whose mode is not in its own list, one that reports its turns, and ones whose reports come after the node's wait for them.
    for (kind, flags) in [
        ("startskip", &["--start-mode=bypassPermissions"][..]),
        ("startauto", &["--start-auto-approve"][..]),
        ("startunlisted", &["--start-mode=yolo"][..]),
        ("turnsskip", &["--turns"][..]),
        (
            "slowskip",
            &["--report-delay-ms=3000", "--start-mode=bypassPermissions"][..],
        ),
    ] {
        let mut command = vec![fake_adapter_path(), "--adapter".into(), "--modes".into()];
        command.extend(flags.iter().map(|f| (*f).to_string()));
        std::fs::write(
            manifests.join(format!("{kind}.toml")),
            format!("kind = \"{kind}\"\ndisplay_name = \"S\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = {command:?}\n"),
        )
        .unwrap();
    }
    if let Some(policy) = policy {
        std::fs::create_dir_all(dir.join("data")).unwrap();
        std::fs::write(dir.join("data/policy"), policy).unwrap();
    }
    Env { dir, repo }
}

fn node(e: &Env, resume_all: bool) -> Node {
    let n = Node::start(&NodeConfig {
        resume_all,
        node_id: "node-test".into(),
        socket: e.dir.join("c.sock"),
        data_dir: e.dir.join("data"),
        worktree_root: e.dir.join("worktrees"),
        manifests_dir: e.dir.join("manifests"),
        projects: vec![e.repo.clone()],
        limits: NodeLimits::default(),
    })
    .expect("start node");
    n.approvals.set_verifier(Box::new(EchoVerifier));
    n
}

fn ok(r: Reply) -> v1::CommandOk {
    r.expect("io").expect("command failed")
}

fn error(r: Reply) -> v1::ErrorInfo {
    r.expect("io").expect_err("expected an error")
}

fn param(e: &v1::ErrorInfo, key: &str) -> String {
    match e.params.get(key).and_then(|p| p.value.clone()) {
        Some(v1::param_value::Value::Str(s)) => s,
        other => panic!("no string param {key}: {other:?}"),
    }
}

/// A started agent whose adapter has reported its modes and options.
fn agent(n: &Node, c: &mut Client) -> (String, PathBuf) {
    agent_of(n, c, "modal")
}

/// A started agent of `kind` (`modal` or `cfgmode`) whose adapter has reported its modes and options, with a clean record.
fn agent_of(n: &Node, c: &mut Client, kind: &str) -> (String, PathBuf) {
    let r = ok(c.call(Cmd::AgentStart(v1::AgentStart {
        kind: kind.into(),
        ..Default::default()
    })));
    let Some(Ok_::AgentStart(s)) = r.result else {
        panic!("not a start")
    };
    let view = n
        .supervisor
        .lock()
        .unwrap()
        .list()
        .into_iter()
        .find(|v| v.id.as_str() == s.agent_id)
        .unwrap();
    // The reports arrive just after the start: a mode that asks is accepted once they are in.
    let end = Instant::now() + Duration::from_secs(15);
    while c
        .call(Cmd::AgentSetMode(v1::AgentSetMode {
            agent_id: s.agent_id.clone(),
            mode: "default".into(),
        }))
        .unwrap()
        .is_err()
    {
        assert!(Instant::now() < end, "the modes were never reported");
        std::thread::sleep(Duration::from_millis(50));
    }
    // The config report comes with it; wait until a safe change is accepted.
    while c
        .call(Cmd::AgentSetConfig(v1::AgentSetConfig {
            agent_id: s.agent_id.clone(),
            config_id: "auto_approve".into(),
            value: "false".into(),
        }))
        .unwrap()
        .is_err()
    {
        assert!(Instant::now() < end, "the config was never reported");
        std::thread::sleep(Duration::from_millis(50));
    }
    // The adapter writes its line AFTER it answers the node, so the two setup switches above are in the record only a moment later (on
    // a slow machine, later than any clean-up made right here). Wait for exactly those two lines, then start from a clean record of
    // what the agent is told from now on.
    let record = view.worktree.join("applied.txt");
    let first = if kind == "cfgmode" {
        "applied config:approvals=default"
    } else {
        "applied mode:default"
    };
    let setup = [first, "applied config:auto_approve=false"];
    let read = || -> Vec<String> {
        std::fs::read_to_string(&record)
            .map(|t| t.lines().map(str::to_string).collect())
            .unwrap_or_default()
    };
    while read() != setup {
        assert!(
            Instant::now() < end,
            "the setup switches were never recorded"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    while record.exists() {
        let _ = std::fs::remove_file(&record);
        assert!(Instant::now() < end, "the record could not be cleared");
        std::thread::sleep(Duration::from_millis(20));
    }
    (s.agent_id, view.worktree)
}

fn set_mode(c: &mut Client, agent: &str, mode: &str) -> Reply {
    c.call(Cmd::AgentSetMode(v1::AgentSetMode {
        agent_id: agent.into(),
        mode: mode.into(),
    }))
}

fn set_config(c: &mut Client, agent: &str, id: &str, value: &str) -> Reply {
    c.call(Cmd::AgentSetConfig(v1::AgentSetConfig {
        agent_id: agent.into(),
        config_id: id.into(),
        value: value.into(),
    }))
}

/// The line a harmless switch leaves, used to know the adapter has handled everything sent before it (it handles them in order).
const MARK: &str = "applied config:auto_approve=false";

/// Sends the harmless switch: whatever the adapter was told before it is in `applied.txt` once this line is.
fn marker(c: &mut Client, agent: &str) {
    ok(set_config(c, agent, "auto_approve", "false"));
}

/// Waits (up to 20 s) until what the agent was told, line by line, is exactly `want`, then says so; a mismatch shows what it was.
fn applied(worktree: &Path, want: &[&str]) {
    let read = || -> Vec<String> {
        std::fs::read_to_string(worktree.join("applied.txt"))
            .map(|t| t.lines().map(str::to_string).collect())
            .unwrap_or_default()
    };
    let end = Instant::now() + Duration::from_secs(20);
    while read() != want && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(read(), want);
}

fn skip_requests(n: &Node) -> Vec<v1::ApprovalRequested> {
    n.events
        .events_after(0)
        .into_iter()
        .filter_map(|e| match e.body {
            Some(v1::event::Body::ApprovalRequested(a)) if a.action == "agent.skip_permissions" => {
                Some(a)
            }
            _ => None,
        })
        .collect()
}

fn resolved(n: &Node, id: &str) -> Vec<v1::ApprovalResolved> {
    n.events
        .events_after(0)
        .into_iter()
        .filter_map(|e| match e.body {
            Some(v1::event::Body::ApprovalResolved(r)) if r.approval_id == id => Some(r),
            _ => None,
        })
        .collect()
}

/// The decision a trusted device sends for `request`: signed over the skip domain, naming `agent`, `option` and `value`.
fn decision(
    request: &v1::ApprovalRequested,
    approved: bool,
    agent: &str,
    option: &str,
    value: &str,
) -> v1::ApprovalResolve {
    let binding = SkipBinding {
        agent: agent.into(),
        option: option.into(),
        value: value.into(),
    };
    v1::ApprovalResolve {
        approval_id: request.approval_id.clone(),
        approved,
        signer_device_id: "phone".into(),
        signature: skip_signed_message(
            "node-test",
            &request.approval_id,
            approved,
            &binding,
            &request.detail,
        ),
        helper_grant: 0,
        helpers_may_start: false,
        ..Default::default()
    }
}

/// The honest decision for what the card shows.
fn honest(request: &v1::ApprovalRequested, approved: bool) -> v1::ApprovalResolve {
    let d: serde_json::Value = serde_json::from_str(&request.detail).unwrap();
    decision(
        request,
        approved,
        d["agent"].as_str().unwrap(),
        d["option"].as_str().unwrap(),
        d["value"].as_str().unwrap(),
    )
}

fn local() -> bool {
    agora_platform::LOCAL_SOCKETS_SUPPORTED
}

/// The whole path: the switch is not applied and not refused, the card is the node's own text, the owner's signed decision applies exactly
/// the mode the node rendered, and a second use of the decision is refused.
#[test]
fn an_approved_switch_applies_exactly_the_rendered_mode_once() {
    if !local() {
        return;
    }
    let e = env("approve", None);
    let n = node(&e, false);
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let (id, worktree) = agent(&n, &mut c);

    let err = error(set_mode(&mut c, &id, "bypassPermissions"));
    assert_eq!(err.code, "agent.approval_needed");
    let approval = param(&err, "approval_id");
    // Nothing is applied before the owner decides: the adapter has handled the marker and holds no other line.
    marker(&mut c, &id);
    applied(&worktree, &[MARK]);
    // The card: the node's own rendering of agent, folder, machine, option and value.
    let requests = skip_requests(&n);
    assert_eq!(requests.len(), 1);
    let card = &requests[0];
    assert_eq!(card.approval_id, approval);
    assert_eq!(card.agent_id, id);
    let d: serde_json::Value = serde_json::from_str(&card.detail).unwrap();
    assert_eq!(d["node"], "node-test");
    assert_eq!(d["agent"], id.as_str());
    assert_eq!(d["agent_kind"], "modal");
    assert_eq!(d["option_type"], "mode");
    assert_eq!(d["option"], "mode");
    assert_eq!(d["value"], "bypassPermissions");
    assert_eq!(d["value_name"], "Bypass Permissions");
    assert_eq!(d["scope"], "session");
    let folder = d["folder"].as_str().unwrap();
    assert!(
        Path::new(folder).ends_with(worktree.file_name().unwrap()),
        "{folder}"
    );

    let r = ok(c.call(Cmd::ApprovalResolve(honest(card, true))));
    assert!(matches!(r.result, Some(Ok_::Empty(_))));
    applied(&worktree, &[MARK, "applied mode:bypassPermissions"]);
    let done = resolved(&n, &approval);
    assert_eq!(done.len(), 1);
    assert!(done[0].approved && done[0].signer_device_id == "phone");
    // Single use: the same decision again is not an approval of anything.
    let again = error(c.call(Cmd::ApprovalResolve(honest(card, true))));
    assert_eq!(again.code, "protocol.invalid_message");
    marker(&mut c, &id);
    applied(&worktree, &[MARK, "applied mode:bypassPermissions", MARK]);
}

/// A decision that names another option or value than the card, another agent, the wrong domain, a device the node does not trust, or a
/// grant is refused, changes nothing, and leaves the card waiting for the right one.
#[test]
fn a_decision_that_does_not_match_the_card_is_refused() {
    if !local() {
        return;
    }
    let e = env("mismatch", None);
    let n = node(&e, false);
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let (id, worktree) = agent(&n, &mut c);
    let err = error(set_mode(&mut c, &id, "bypassPermissions"));
    assert_eq!(err.code, "agent.approval_needed");
    let card = skip_requests(&n).remove(0);

    let mut refused = |what: &str, d: v1::ApprovalResolve| {
        assert_eq!(
            error(c.call(Cmd::ApprovalResolve(d))).code,
            "approval.signature_invalid",
            "{what}"
        );
    };
    // Signed for another value, another option, another agent.
    refused("value", decision(&card, true, &id, "mode", "default"));
    refused(
        "option",
        decision(&card, true, &id, "auto_approve", "bypassPermissions"),
    );
    refused(
        "agent",
        decision(&card, true, "agent-99", "mode", "bypassPermissions"),
    );
    // The bytes of the right decision with the verdict flipped.
    let mut flipped = honest(&card, false);
    flipped.approved = true;
    refused("verdict", flipped);
    // A device the node does not trust, even over the right bytes.
    let mut stranger = honest(&card, true);
    stranger.signer_device_id = "stranger".into();
    refused("device", stranger);
    // The plain approval message of every other request: not this card's domain.
    let mut plain = honest(&card, true);
    plain.signature = agora_node::approvals::signed_message("node-test", &card.approval_id, true);
    refused("domain", plain);
    // A signature over another card's detail.
    let mut other_detail = card.clone();
    other_detail.detail = card.detail.replace("Bypass Permissions", "Harmless");
    refused(
        "detail",
        decision(&other_detail, true, &id, "mode", "bypassPermissions"),
    );
    // A grant means nothing here.
    let mut grant = honest(&card, true);
    grant.helper_grant = 3;
    refused("grant", grant);
    marker(&mut c, &id);
    applied(&worktree, &[MARK]);
    assert_eq!(n.approvals.pending_count(), 1, "the card still waits");

    // The right decision still works.
    ok(c.call(Cmd::ApprovalResolve(honest(&card, true))));
    applied(&worktree, &[MARK, "applied mode:bypassPermissions"]);
}

/// A denial applies nothing and ends the card.
#[test]
fn a_denied_switch_applies_nothing() {
    if !local() {
        return;
    }
    let e = env("deny", None);
    let n = node(&e, false);
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let (id, worktree) = agent(&n, &mut c);
    assert_eq!(
        error(set_mode(&mut c, &id, "bypassPermissions")).code,
        "agent.approval_needed"
    );
    let card = skip_requests(&n).remove(0);
    ok(c.call(Cmd::ApprovalResolve(honest(&card, false))));
    marker(&mut c, &id);
    applied(&worktree, &[MARK]);
    let done = resolved(&n, &card.approval_id);
    assert!(!done[0].approved && done[0].signer_device_id == "phone");
    assert_eq!(n.approvals.pending_count(), 0);
}

/// `forbid` refuses before any card exists, as it refused before this amendment; a mode that asks stays possible.
#[test]
fn a_forbidding_policy_refuses_before_any_card() {
    if !local() {
        return;
    }
    let e = env("forbid", Some("skip_permissions = forbid\n"));
    let n = node(&e, false);
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let (id, worktree) = agent(&n, &mut c);
    let err = error(set_mode(&mut c, &id, "bypassPermissions"));
    assert_eq!(err.code, "agent.option_forbidden");
    assert_eq!(param(&err, "policy"), "forbid");
    let err = error(set_config(&mut c, &id, "auto_approve", "true"));
    assert_eq!(err.code, "agent.option_forbidden");
    assert!(skip_requests(&n).is_empty());
    assert_eq!(n.approvals.pending_count(), 0);
    ok(set_mode(&mut c, &id, "plan"));
    applied(&worktree, &["applied mode:plan"]);
}

/// Values that keep protection are never asked about, and switching back to one needs no approval, with a card waiting or not.
#[test]
fn safe_values_and_switching_back_never_need_approval() {
    if !local() {
        return;
    }
    let e = env("safe", None);
    let n = node(&e, false);
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let (id, worktree) = agent(&n, &mut c);
    ok(set_mode(&mut c, &id, "plan"));
    ok(set_config(&mut c, &id, "auto_approve", "false"));
    // An approved bypass, then back to a mode that asks.
    assert_eq!(
        error(set_mode(&mut c, &id, "bypassPermissions")).code,
        "agent.approval_needed"
    );
    let card = skip_requests(&n).remove(0);
    ok(c.call(Cmd::ApprovalResolve(honest(&card, true))));
    ok(set_mode(&mut c, &id, "default"));
    applied(
        &worktree,
        &[
            "applied mode:plan",
            "applied config:auto_approve=false",
            "applied mode:bypassPermissions",
            "applied mode:default",
        ],
    );
    assert_eq!(
        skip_requests(&n).len(),
        1,
        "only the bypass was asked about"
    );
}

/// A boolean option that makes the agent act without asking is asked about like a mode, and its card names the option and the value.
#[test]
fn a_config_option_that_skips_is_asked_about_and_applied_as_rendered() {
    if !local() {
        return;
    }
    let e = env("config", None);
    let n = node(&e, false);
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let (id, worktree) = agent(&n, &mut c);
    let err = error(set_config(&mut c, &id, "auto_approve", "true"));
    assert_eq!(err.code, "agent.approval_needed");
    let card = skip_requests(&n).remove(0);
    let d: serde_json::Value = serde_json::from_str(&card.detail).unwrap();
    assert_eq!(d["option_type"], "config");
    assert_eq!(d["option"], "auto_approve");
    assert_eq!(d["option_name"], "Auto approve");
    assert_eq!(d["value"], "true");
    // A decision signed as a mode switch to the same value is another message.
    assert_eq!(
        error(c.call(Cmd::ApprovalResolve(decision(
            &card, true, &id, "mode", "true"
        ))))
        .code,
        "approval.signature_invalid"
    );
    ok(c.call(Cmd::ApprovalResolve(honest(&card, true))));
    applied(&worktree, &["applied config:auto_approve=true"]);
}

/// A mode the node cannot place (nothing in its words says it asks, `turbo`) counts as skipping, so it is asked about too, whatever the
/// adapter's own flag said.
#[test]
fn a_mode_the_classifier_cannot_place_is_asked_about() {
    if !local() {
        return;
    }
    let e = env("unknown", None);
    let n = node(&e, false);
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let (id, worktree) = agent(&n, &mut c);
    assert_eq!(
        error(set_mode(&mut c, &id, "turbo")).code,
        "agent.approval_needed"
    );
    let card = skip_requests(&n).remove(0);
    assert!(
        card.detail.contains("\"value\":\"turbo\""),
        "{}",
        card.detail
    );
    marker(&mut c, &id);
    applied(&worktree, &[MARK]);
}

/// One card per agent, option and value: asking again gets the same card; another value of the same option replaces it, and the replaced
/// card can no longer be decided.
#[test]
fn asking_again_gets_the_same_card_and_another_value_replaces_it() {
    if !local() {
        return;
    }
    let e = env("dedupe", None);
    let n = node(&e, false);
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let (id, worktree) = agent(&n, &mut c);
    let first = param(
        &error(set_mode(&mut c, &id, "bypassPermissions")),
        "approval_id",
    );
    let again = param(
        &error(set_mode(&mut c, &id, "bypassPermissions")),
        "approval_id",
    );
    assert_eq!(first, again);
    assert_eq!(skip_requests(&n).len(), 1, "announced once");
    assert_eq!(n.approvals.pending_count(), 1);
    // Another value of the same option.
    let second = param(&error(set_mode(&mut c, &id, "turbo")), "approval_id");
    assert_ne!(first, second);
    assert_eq!(n.approvals.pending_count(), 1);
    let replaced = resolved(&n, &first);
    assert_eq!(replaced.len(), 1);
    assert!(!replaced[0].approved && replaced[0].signer_device_id.is_empty());
    let old = skip_requests(&n)
        .into_iter()
        .find(|r| r.approval_id == first)
        .unwrap();
    assert_eq!(
        error(c.call(Cmd::ApprovalResolve(honest(&old, true)))).code,
        "protocol.invalid_message"
    );
    marker(&mut c, &id);
    applied(&worktree, &[MARK]);
    // A different option is a different card.
    let third = param(
        &error(set_config(&mut c, &id, "auto_approve", "true")),
        "approval_id",
    );
    assert_ne!(second, third);
    assert_eq!(n.approvals.pending_count(), 2);
}

/// Setting the option by another command settles it: the waiting card for it is withdrawn and can no longer be decided.
#[test]
fn another_switch_of_the_option_withdraws_its_card() {
    if !local() {
        return;
    }
    let e = env("supersede", None);
    let n = node(&e, false);
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let (id, worktree) = agent(&n, &mut c);
    let approval = param(
        &error(set_mode(&mut c, &id, "bypassPermissions")),
        "approval_id",
    );
    let card = skip_requests(&n).remove(0);
    ok(set_mode(&mut c, &id, "plan"));
    assert_eq!(n.approvals.pending_count(), 0);
    let done = resolved(&n, &approval);
    assert!(!done[0].approved && done[0].signer_device_id.is_empty());
    assert_eq!(
        error(c.call(Cmd::ApprovalResolve(honest(&card, true)))).code,
        "protocol.invalid_message"
    );
    applied(&worktree, &["applied mode:plan"]);
}

/// The permission is the agent's session: when the agent ends, its waiting card is withdrawn and a late decision applies nothing.
#[test]
fn the_end_of_the_agent_withdraws_its_card() {
    if !local() {
        return;
    }
    let e = env("ends", None);
    let n = node(&e, false);
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let (id, worktree) = agent(&n, &mut c);
    let approval = param(
        &error(set_mode(&mut c, &id, "bypassPermissions")),
        "approval_id",
    );
    let card = skip_requests(&n).remove(0);
    // The adapter has handled everything sent so far (it handles them in order): it holds the marker and nothing else.
    marker(&mut c, &id);
    applied(&worktree, &[MARK]);
    ok(c.call(Cmd::AgentStop(v1::AgentStop {
        agent_id: id.clone(),
        ..Default::default()
    })));
    let end = Instant::now() + Duration::from_secs(15);
    while n.approvals.pending_count() > 0 {
        assert!(Instant::now() < end, "the card was never withdrawn");
        std::thread::sleep(Duration::from_millis(50));
    }
    let done = resolved(&n, &approval);
    assert_eq!(done.len(), 1);
    assert!(!done[0].approved && done[0].signer_device_id.is_empty());
    let late = error(c.call(Cmd::ApprovalResolve(honest(&card, true))));
    assert_eq!(late.code, "protocol.invalid_message");
    // The agent is gone: the decision found no card to apply, so the adapter was told nothing more than the marker.
    applied(&worktree, &[MARK]);
}

/// A stopped agent has no session to switch (audit F1320): a mode that skips is not asked about for it (it keeps the modes it last
/// reported, which must not raise a card), and once it is resumed the new run asks again and the approval applies there.
#[test]
fn a_stopped_agent_gets_no_card_and_its_resumed_run_asks_again() {
    if !local() {
        return;
    }
    let e = env("stopped", None);
    let n = node(&e, false);
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let (id, worktree) = agent(&n, &mut c);
    ok(c.call(Cmd::AgentStop(v1::AgentStop {
        agent_id: id.clone(),
        ..Default::default()
    })));
    for mode in ["bypassPermissions", "turbo", "plan"] {
        assert_eq!(
            error(set_mode(&mut c, &id, mode)).code,
            "agent.not_found",
            "{mode}"
        );
    }
    assert!(skip_requests(&n).is_empty(), "no card for a stopped agent");
    assert_eq!(n.approvals.pending_count(), 0);
    n.supervisor
        .lock()
        .unwrap()
        .resume(&agora_core::AgentId::new(id.clone()), Instant::now())
        .expect("resume");
    let end = Instant::now() + Duration::from_secs(15);
    let approval = loop {
        match set_mode(&mut c, &id, "bypassPermissions").unwrap() {
            Err(e) if e.code == "agent.approval_needed" => break param(&e, "approval_id"),
            other => {
                assert!(
                    Instant::now() < end,
                    "the resumed agent never asked: {other:?}"
                );
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    };
    while set_config(&mut c, &id, "auto_approve", "false")
        .unwrap()
        .is_err()
    {
        assert!(Instant::now() < end, "the config was never reported again");
        std::thread::sleep(Duration::from_millis(50));
    }
    applied(&worktree, &[MARK]);
    let card = skip_requests(&n)
        .into_iter()
        .find(|r| r.approval_id == approval)
        .unwrap();
    ok(c.call(Cmd::ApprovalResolve(honest(&card, true))));
    marker(&mut c, &id);
    applied(&worktree, &[MARK, "applied mode:bypassPermissions", MARK]);
}

/// A crash of the adapter ends the session too.
#[test]
fn a_crash_of_the_adapter_withdraws_its_card() {
    if !local() {
        return;
    }
    let e = env("crash", None);
    let n = node(&e, false);
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let (id, _worktree) = agent(&n, &mut c);
    assert_eq!(
        error(set_mode(&mut c, &id, "bypassPermissions")).code,
        "agent.approval_needed"
    );
    ok(c.call(Cmd::AgentPrompt(v1::AgentPrompt {
        agent_id: id.clone(),
        text: "crash".into(),
    })));
    let end = Instant::now() + Duration::from_secs(20);
    while n.approvals.pending_count() > 0 {
        assert!(Instant::now() < end, "the card was never withdrawn");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A node restart forgets every card and every approval: an old card's decision is not an approval of anything, and the agent that
/// comes back asks again.
#[test]
fn a_restart_does_not_restore_a_card_or_an_approval() {
    if !local() {
        return;
    }
    let e = env("restart", None);
    let (id, old_card) = {
        let n = node(&e, false);
        let mut c = Client::connect(n.socket().unwrap()).unwrap();
        let (id, _) = agent(&n, &mut c);
        assert_eq!(
            error(set_mode(&mut c, &id, "bypassPermissions")).code,
            "agent.approval_needed"
        );
        let card = skip_requests(&n).remove(0);
        // An approved switch, too: the session of this node run.
        ok(c.call(Cmd::ApprovalResolve(honest(&card, true))));
        assert_eq!(
            error(set_mode(&mut c, &id, "turbo")).code,
            "agent.approval_needed"
        );
        (id, skip_requests(&n).remove(1))
    };
    let n = node(&e, true);
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    assert_eq!(n.approvals.pending_count(), 0, "no card comes back");
    assert_eq!(
        error(c.call(Cmd::ApprovalResolve(honest(&old_card, true)))).code,
        "protocol.invalid_message"
    );
    // The agent comes back (resumed) and asks again; an agent that is not back yet is started afresh and asks too.
    let end = Instant::now() + Duration::from_secs(30);
    let back = loop {
        let list = n.supervisor.lock().unwrap().list();
        if list.iter().any(|v| v.id.as_str() == id && v.running) {
            break id.clone();
        }
        if Instant::now() > end {
            break agent(&n, &mut c).0;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        let r = set_mode(&mut c, &back, "bypassPermissions").unwrap();
        match r {
            Err(e) if e.code == "agent.approval_needed" => break,
            Err(_) => {}
            Ok(_) => panic!("a restarted agent took a mode that skips without asking"),
        }
        assert!(Instant::now() < end, "never asked again");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// An adapter that raises a request under a name only the node uses (`agent.start`) gets no card and is answered "denied" at once;
/// nothing is announced and nothing waits.
#[test]
fn an_adapter_that_names_a_node_action_is_denied_without_a_card() {
    if !local() {
        return;
    }
    let e = env("reserved", None);
    let n = node(&e, false);
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let (id, worktree) = agent(&n, &mut c);
    ok(c.call(Cmd::AgentPrompt(v1::AgentPrompt {
        agent_id: id.clone(),
        text: "ask-reserved".into(),
    })));
    applied(&worktree, &["answered:denied"]);
    assert_eq!(n.approvals.pending_count(), 0);
    let announced = n
        .events
        .events_after(0)
        .into_iter()
        .any(|e| matches!(e.body, Some(v1::event::Body::ApprovalRequested(_))));
    assert!(!announced, "no card was announced");
}

/// When the agent's mode is a config option of category `mode` (an ACP agent through the bridge), the mode command and the config
/// command switch ONE setting: switching it back through either withdraws a card raised through the other, so a late approval of the
/// old card can no longer turn the agent to the mode that skips (audit F1322). An unrelated option keeps the card.
#[test]
fn switching_one_setting_back_through_the_other_command_withdraws_its_card() {
    if !local() {
        return;
    }
    let e = env("twin", None);
    let n = node(&e, false);
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let pending = |n: &Node, id: &str| n.approvals.waiting().iter().any(|p| p.id == id);

    // A mode card; an unrelated option keeps it; the same setting switched back through config withdraws it, and a late decision
    // is refused and applies nothing.
    let (id, worktree) = agent_of(&n, &mut c, "cfgmode");
    let mode_card = param(
        &error(set_mode(&mut c, &id, "bypassPermissions")),
        "approval_id",
    );
    ok(set_config(&mut c, &id, "auto_approve", "false"));
    assert!(
        pending(&n, &mode_card),
        "an unrelated option keeps the card"
    );
    ok(set_config(&mut c, &id, "approvals", "default"));
    assert!(
        !pending(&n, &mode_card),
        "the config switch withdraws the mode card"
    );
    let card = skip_requests(&n)
        .into_iter()
        .find(|r| r.approval_id == mode_card)
        .unwrap();
    assert_eq!(
        error(c.call(Cmd::ApprovalResolve(honest(&card, true)))).code,
        "protocol.invalid_message"
    );
    marker(&mut c, &id);
    applied(
        &worktree,
        &[
            "applied config:auto_approve=false",
            "applied config:approvals=default",
            MARK,
        ],
    );

    // A config card; the same setting switched back through the mode command withdraws it.
    let (id2, _) = agent_of(&n, &mut c, "cfgmode");
    let cfg_card = param(
        &error(set_config(&mut c, &id2, "approvals", "bypassPermissions")),
        "approval_id",
    );
    ok(set_mode(&mut c, &id2, "default"));
    assert!(
        !pending(&n, &cfg_card),
        "the mode switch withdraws the config card"
    );

    // An agent whose mode is not a config option: a config switch leaves its mode card alone.
    let (m, _) = agent(&n, &mut c);
    let m_card = param(
        &error(set_mode(&mut c, &m, "bypassPermissions")),
        "approval_id",
    );
    ok(set_config(&mut c, &m, "auto_approve", "false"));
    assert!(
        pending(&n, &m_card),
        "a non-twin option keeps the mode card"
    );
}

/// #1147 (audit of #1157): a value the node holds that is the agent's own name for a mode never reaches the card (the queue, the hub's
/// event, the window); the decision signs what the card showed and still applies the mode the agent named by its id.
#[test]
fn a_secret_in_the_agents_name_for_a_mode_never_reaches_the_card() {
    if !local() {
        return;
    }
    let e = env("secret-name", None);
    // The fake agent calls its mode `bypassPermissions` "Bypass Permissions": a value this node holds.
    agora_node::credentials::SecretStore::in_data(&e.dir.join("data"))
        .set(
            "TOKEN",
            &agora_node::credentials::Secret::new("Bypass Permissions").unwrap(),
        )
        .unwrap();
    let n = node(&e, false);
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let (id, worktree) = agent(&n, &mut c);
    let err = error(set_mode(&mut c, &id, "bypassPermissions"));
    assert_eq!(err.code, "agent.approval_needed");
    let card = skip_requests(&n).remove(0);
    let d: serde_json::Value = serde_json::from_str(&card.detail).unwrap();
    assert_eq!(d["value_name"], "<redacted>", "{}", card.detail);
    assert!(!card.detail.contains("Bypass"), "{}", card.detail);
    assert!(n
        .approvals
        .waiting()
        .iter()
        .all(|p| !p.detail.contains("Bypass")));
    let r = ok(c.call(Cmd::ApprovalResolve(honest(&card, true))));
    assert!(matches!(r.result, Some(Ok_::Empty(_))));
    applied(&worktree, &["applied mode:bypassPermissions"]);
}

/// P2-16: starts an agent of `kind`, without waiting for anything after the start.
fn start_of(c: &mut Client, kind: &str) -> String {
    let r = ok(c.call(Cmd::AgentStart(v1::AgentStart {
        kind: kind.into(),
        ..Default::default()
    })));
    let Some(Ok_::AgentStart(s)) = r.result else {
        panic!("not a start")
    };
    s.agent_id
}

/// P2-16: the agent's view, if the node still has it.
fn view(n: &Node, id: &str) -> Option<agora_node::supervisor::AgentView> {
    n.supervisor
        .lock()
        .unwrap()
        .list()
        .into_iter()
        .find(|v| v.id.as_str() == id)
}

/// P2-16: the node's notices in the agent's conversation that it stopped it for what it had in use, as `(policy, setting)`.
fn skip_refusals(n: &Node, id: &str) -> Vec<(String, String)> {
    n.chat
        .snapshot(&agora_core::AgentId::new(id))
        .into_iter()
        .filter(|e| e.code == agora_node::chat::SKIP_REFUSED)
        .map(|e| {
            (
                e.params.get("policy").cloned().unwrap_or_default(),
                e.params.get("setting").cloned().unwrap_or_default(),
            )
        })
        .collect()
}

/// P2-16: waits (up to 20 s) until the node stopped the agent for what it had in use, and returns its notice's `(policy, setting)`.
fn refused_by_node(n: &Node, id: &str) -> (String, String) {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        let stopped =
            view(n, id).is_some_and(|v| !v.running && v.state == agora_core::AgentState::Done);
        let said = skip_refusals(n, id);
        if stopped && !said.is_empty() {
            assert_eq!(said.len(), 1, "said once");
            return said[0].clone();
        }
        assert!(
            Instant::now() < end,
            "the agent was never stopped: {:?}",
            view(n, id)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// P2-16: a start of `kind` that the node refuses for what the agent reports it uses: `agent.option_forbidden` with the id in use and
/// the policy, and nothing of it left running.
fn start_refused(n: &Node, c: &mut Client, kind: &str) -> (String, String) {
    let err = error(c.call(Cmd::AgentStart(v1::AgentStart {
        kind: kind.into(),
        ..Default::default()
    })));
    assert_eq!(err.code, "agent.option_forbidden", "{err:?}");
    assert_eq!(param(&err, "kind"), kind);
    assert!(
        n.supervisor
            .lock()
            .unwrap()
            .list()
            .iter()
            .all(|v| v.kind != kind || !v.running),
        "an agent of {kind} runs"
    );
    (param(&err, "option"), param(&err, "policy"))
}

/// The worktrees agents of this node were started in (each a folder under the node's worktree root).
fn worktrees(e: &Env) -> Vec<PathBuf> {
    std::fs::read_dir(e.dir.join("worktrees"))
        .map(|d| d.filter_map(|x| x.ok()).map(|x| x.path()).collect())
        .unwrap_or_default()
}

/// P2-16 (security review of #1244, V1): under `forbid`, a start of an agent whose own session starts in a mode that skips is
/// refused before it can take any prompt: the start is answered `agent.option_forbidden` (the mode, the policy), no agent of it runs,
/// and a prompt sent at once to the id it would have had reaches nothing.
#[test]
fn a_forbidding_policy_refuses_the_start_of_an_agent_that_starts_in_a_mode_that_skips() {
    if !local() {
        return;
    }
    let e = env("start-forbid", Some("skip_permissions = forbid\n"));
    let n = node(&e, false);
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let sock = n.socket().unwrap().to_path_buf();
    // A prompt sent over and over, from before the start, to the id the agent would get.
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = std::sync::Arc::clone(&stop);
    let sender = std::thread::spawn(move || {
        let mut c2 = Client::connect(&sock).unwrap();
        while !flag.load(std::sync::atomic::Ordering::Acquire) {
            let r = c2.call(Cmd::AgentPrompt(v1::AgentPrompt {
                agent_id: "agent-1".into(),
                text: "hang".into(),
            }));
            assert!(
                matches!(&r, Ok(Err(e)) if e.code == "agent.not_found"),
                "a prompt was taken: {r:?}"
            );
        }
    });
    assert_eq!(
        start_refused(&n, &mut c, "startskip"),
        ("bypassPermissions".into(), "forbid".into())
    );
    stop.store(true, std::sync::atomic::Ordering::Release);
    sender.join().unwrap();
    assert!(
        worktrees(&e).iter().all(|w| !w.join("hang.txt").exists()),
        "the agent read a prompt"
    );
    // Nothing was reported to the hub as the agent's options.
    assert!(!n.events.events_after(0).into_iter().any(|e| matches!(
        e.body,
        Some(v1::event::Body::AgentSessionOptions(o)) if o.current_mode == "bypassPermissions"
    )));
}

/// P2-16: the same for a config option that skips and is on from the start.
#[test]
fn a_forbidding_policy_refuses_the_start_of_an_agent_with_an_option_that_skips() {
    if !local() {
        return;
    }
    let e = env("start-auto", Some("skip_permissions = forbid\n"));
    let n = node(&e, false);
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    assert_eq!(
        start_refused(&n, &mut c, "startauto"),
        ("auto_approve".into(), "forbid".into())
    );
}

/// P2-16 (security review of #1244, V3): a current mode its own list does not have is judged on its id (`yolo` skips), never dropped
/// as unknown.
#[test]
fn a_forbidding_policy_refuses_an_agent_whose_mode_in_use_is_not_in_its_list() {
    if !local() {
        return;
    }
    let e = env("start-unlisted", Some("skip_permissions = forbid\n"));
    let n = node(&e, false);
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    assert_eq!(
        start_refused(&n, &mut c, "startunlisted"),
        ("yolo".into(), "forbid".into())
    );
}

/// P2-16: `confined_only` refuses it for an agent whose process would not be confined (this node does not confine).
#[test]
fn a_confined_only_policy_refuses_an_unconfined_agent_that_starts_in_a_mode_that_skips() {
    if !local() {
        return;
    }
    let e = env("start-confined", Some("skip_permissions = confined_only\n"));
    let n = node(&e, false);
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    assert_eq!(
        start_refused(&n, &mut c, "startskip"),
        ("bypassPermissions".into(), "confined_only".into())
    );
}

/// P2-16: an agent whose first reports come after the node's wait for them starts, but takes no prompt until they came and were
/// judged (`adapter.no_answer`, call `session`); when they say it skips, it is stopped and its conversation says why.
#[test]
fn a_forbidding_policy_takes_no_prompt_before_a_late_first_report_and_then_stops_the_agent() {
    if !local() {
        return;
    }
    let e = env("slow-report", Some("skip_permissions = forbid\n"));
    let n = node(&e, false);
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let id = start_of(&mut c, "slowskip");
    let worktree = view(&n, &id).unwrap().worktree;
    let err = error(c.call(Cmd::AgentPrompt(v1::AgentPrompt {
        agent_id: id.clone(),
        text: "hang".into(),
    })));
    assert_eq!(err.code, "adapter.no_answer", "{err:?}");
    assert_eq!(param(&err, "call"), "session");
    assert_eq!(refused_by_node(&n, &id), ("forbid".into(), "mode".into()));
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        !worktree.join("hang.txt").exists(),
        "the agent read a prompt"
    );
}

/// P2-16: under `forbid` an agent that starts in a mode that asks runs, and one that switches itself to a mode that skips later is
/// stopped, with a notice; a prompt to it is then refused.
#[test]
fn a_forbidding_policy_keeps_a_normal_agent_and_stops_one_that_switches_itself() {
    if !local() {
        return;
    }
    let e = env("self-switch", Some("skip_permissions = forbid\n"));
    let n = node(&e, false);
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let (id, _) = agent(&n, &mut c);
    std::thread::sleep(Duration::from_millis(600));
    assert!(view(&n, &id).is_some_and(|v| v.running));
    assert!(skip_refusals(&n, &id).is_empty());
    ok(c.call(Cmd::AgentPrompt(v1::AgentPrompt {
        agent_id: id.clone(),
        text: "mode:bypassPermissions".into(),
    })));
    assert_eq!(refused_by_node(&n, &id), ("forbid".into(), "mode".into()));
    let agent = agora_core::AgentId::new(&id);
    assert!(!n.supervisor.lock().unwrap().skips_permissions_now(&agent));
    std::thread::sleep(Duration::from_millis(600));
    assert!(
        view(&n, &id).is_some_and(|v| !v.running),
        "it was started again"
    );
    let refused = error(c.call(Cmd::AgentPrompt(v1::AgentPrompt {
        agent_id: id.clone(),
        text: "hello".into(),
    })));
    assert_eq!(refused.code, "agent.not_found");
}

/// P2-16 (security review of #1244, V2): a prompt queued behind a turn does not go out in the tick that reads the turn's end together
/// with a report that makes the node stop the agent: it is dropped with the queue.
#[test]
fn a_queued_prompt_never_goes_out_in_the_tick_that_stops_the_agent() {
    if !local() {
        return;
    }
    for round in 0..3 {
        let e = env(
            &format!("queued-{round}"),
            Some("skip_permissions = forbid\n"),
        );
        let n = node(&e, false);
        let mut c = Client::connect(n.socket().unwrap()).unwrap();
        let id = start_of(&mut c, "turnsskip");
        let worktree = view(&n, &id).unwrap().worktree;
        let end = Instant::now() + Duration::from_secs(15);
        while c
            .call(Cmd::AgentSetMode(v1::AgentSetMode {
                agent_id: id.clone(),
                mode: "default".into(),
            }))
            .unwrap()
            .is_err()
        {
            assert!(Instant::now() < end, "the modes were never reported");
            std::thread::sleep(Duration::from_millis(50));
        }
        ok(c.call(Cmd::AgentPrompt(v1::AgentPrompt {
            agent_id: id.clone(),
            text: "slow-bypass".into(),
        })));
        let queued = ok(c.call(Cmd::AgentPrompt(v1::AgentPrompt {
            agent_id: id.clone(),
            text: "hang".into(),
        })));
        assert!(
            matches!(queued.result, Some(Ok_::AgentPrompt(ref p)) if p.queued),
            "{queued:?}"
        );
        assert_eq!(refused_by_node(&n, &id), ("forbid".into(), "mode".into()));
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            !worktree.join("hang.txt").exists(),
            "round {round}: the queued prompt reached the agent"
        );
    }
}

/// P2-16: a resume is judged as a start is: an agent whose session comes back in a mode the policy now refuses is not resumed
/// (`agent.option_forbidden`) and stays stopped.
#[test]
fn a_resume_into_a_mode_that_skips_is_refused() {
    if !local() {
        return;
    }
    let e = env("resume", None);
    let n = node(&e, false);
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let id = start_of(&mut c, "startskip");
    let agent = agora_core::AgentId::new(&id);
    ok(c.call(Cmd::AgentStop(v1::AgentStop {
        agent_id: id.clone(),
        ..Default::default()
    })));
    n.supervisor
        .lock()
        .unwrap()
        .set_skip_permissions(agora_node::launch_options::SkipPermissions::Forbid);
    let r = n.supervisor.lock().unwrap().resume(&agent, Instant::now());
    match r {
        Err(agora_node::supervisor::SupervisorError::OptionForbidden(kind, option, policy)) => {
            assert_eq!(
                (kind.as_str(), option.as_str(), policy),
                ("startskip", "bypassPermissions", "forbid")
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(view(&n, &id).is_some_and(|v| !v.running));
    assert!(!n.supervisor.lock().unwrap().skips_permissions_now(&agent));
}

/// P2-16: a crashed adapter started again is judged as a start is: back in a mode the policy now refuses, it is stopped in the
/// same tick, with a notice, and not started again.
#[test]
fn a_restart_after_a_crash_into_a_mode_that_skips_stops_the_agent() {
    if !local() {
        return;
    }
    let e = env("crash", None);
    let n = node(&e, false);
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let id = start_of(&mut c, "startskip");
    let agent = agora_core::AgentId::new(&id);
    let end = Instant::now() + Duration::from_secs(20);
    while !n.supervisor.lock().unwrap().skips_permissions_now(&agent) {
        assert!(Instant::now() < end, "the mode in use was never reported");
        std::thread::sleep(Duration::from_millis(50));
    }
    // It crashes under `allow`; the policy is `forbid` by the time it is started again (a restart waits a second).
    let _ = c.call(Cmd::AgentPrompt(v1::AgentPrompt {
        agent_id: id.clone(),
        text: "crash".into(),
    }));
    n.supervisor
        .lock()
        .unwrap()
        .set_skip_permissions(agora_node::launch_options::SkipPermissions::Forbid);
    assert_eq!(refused_by_node(&n, &id), ("forbid".into(), "mode".into()));
    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        view(&n, &id).is_some_and(|v| !v.running),
        "it was started again"
    );
}

/// P2-16: under `allow` (the default) the node does not stop such an agent: it runs, shown and checked as acting freely.
#[test]
fn an_allowing_policy_keeps_an_agent_that_starts_in_a_mode_that_skips() {
    if !local() {
        return;
    }
    let e = env("start-allow", None);
    let n = node(&e, false);
    let mut c = Client::connect(n.socket().unwrap()).unwrap();
    let id = start_of(&mut c, "startskip");
    let agent = agora_core::AgentId::new(&id);
    let end = Instant::now() + Duration::from_secs(20);
    while !n.supervisor.lock().unwrap().skips_permissions_now(&agent) {
        assert!(Instant::now() < end, "the mode in use was never reported");
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(600));
    assert!(view(&n, &id).is_some_and(|v| v.running));
    assert!(skip_refusals(&n, &id).is_empty());
}

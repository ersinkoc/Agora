//! ADR-0059: launch options chosen at start, against a real node and the fake agent (no accounts, tokens or real coding agents).
//! The node's `skip_permissions` policy (forbid / allow / confined_only), the arguments really on the command line, the kinds the
//! window lists, and the events that carry the badge.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use agora_node::{
    control::Client,
    daemon::{Node, NodeConfig},
    supervisor::NodeLimits,
};
use agora_protocol::v1::{
    self, command_ok::Result as Ok_, command_request::Command as Cmd, event::Body,
};
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

const FLAG: &str = "--fake-skip-permissions";

fn fake() -> String {
    let p = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!("agora-fake-agent{}", std::env::consts::EXE_SUFFIX));
    static BUILD: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    BUILD.get_or_init(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let mut command = Command::new(cargo);
        command
            .args(["build", "-q", "-p", "agora-fake-agent", "--target-dir"])
            .arg(p.parent().unwrap().parent().unwrap());
        if p.parent().unwrap().ends_with("release") {
            command.arg("--release");
        }
        assert!(
            command.status().unwrap().success(),
            "building the fake failed"
        );
    });
    assert!(p.exists());
    p.to_string_lossy().into_owned()
}

fn supported() -> bool {
    agora_platform::PTY_SUPPORTED && agora_platform::LOCAL_SOCKETS_SUPPORTED
}

struct Env {
    root: PathBuf,
    folder: PathBuf,
    node: Option<Node>,
}

impl Env {
    /// A node whose policy file says `policy` (none: no policy file, the default).
    fn new(name: &str, policy: Option<&str>) -> Self {
        let root = std::env::temp_dir().join(format!("launch-opt-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("manifests")).unwrap();
        std::fs::create_dir_all(root.join("data")).unwrap();
        // Outside the node's own folder: a confined node refuses to launch in that one.
        let folder =
            std::env::temp_dir().join(format!("launch-opt-work-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        if let Some(policy) = policy {
            std::fs::write(root.join("data/policy"), policy).unwrap();
        }
        let launch = serde_json::to_string(&vec![
            fake(),
            "--probe-directory".into(),
            root.join("probe-{pid}.json").to_string_lossy().into_owned(),
        ])
        .unwrap();
        let options = format!(
            "[[launch.option]]\nid = \"skip-permissions\"\nlabel = \"option-skip-permissions\"\nargs = [\"{FLAG}\"]\nskips_permissions = true\n\
             [[launch.option]]\nid = \"resume-last\"\nlabel = \"option-resume-last\"\nargs = [\"--fake-continue\"]\n"
        );
        std::fs::write(
            root.join("manifests/probe.toml"),
            format!("kind = \"probe\"\ndisplay_name = \"Probe\"\nversion = \"1\"\nmin_engine = 2\n[process]\nlaunch = {launch}\n{options}"),
        )
        .unwrap();
        std::fs::write(
            root.join("manifests/adapter.toml"),
            format!(
                "kind = \"adapter\"\ndisplay_name = \"Adapter\"\nversion = \"1\"\nmin_engine = 2\n[adapter]\ncommand = [{:?}, \"--adapter\"]\n{options}",
                fake()
            ),
        )
        .unwrap();
        let node = Node::start(&NodeConfig {
            node_id: "node-options".into(),
            socket: root.join("n.sock"),
            data_dir: root.join("data"),
            worktree_root: root.join("worktrees"),
            manifests_dir: root.join("manifests"),
            projects: vec![],
            limits: NodeLimits::default(),
        })
        .unwrap();
        Self {
            root,
            folder,
            node: Some(node),
        }
    }

    fn node(&self) -> &Node {
        self.node.as_ref().unwrap()
    }

    /// A start as the window sends it: `id!` stands for "the user confirmed id" (sent in the separate confirmed list, never in the id).
    fn launch(&self, kind: &str, options: &[&str]) -> Result<String, v1::ErrorInfo> {
        let ids = options
            .iter()
            .map(|o| o.trim_end_matches('!').to_string())
            .collect();
        let confirmed = options
            .iter()
            .filter_map(|o| o.strip_suffix('!').map(str::to_string))
            .collect();
        self.launch_raw(kind, ids, confirmed)
    }

    /// The ids and confirmations exactly as given.
    fn launch_raw(
        &self,
        kind: &str,
        ids: Vec<String>,
        confirmed: Vec<String>,
    ) -> Result<String, v1::ErrorInfo> {
        let mut c = Client::connect(self.node().socket().unwrap()).unwrap();
        let info = match c
            .call(Cmd::DirectoryInspect(v1::DirectoryInspect {
                path: self.folder.to_string_lossy().into_owned(),
                ..Default::default()
            }))
            .unwrap()
            .unwrap()
            .result
        {
            Some(Ok_::DirectoryInfo(i)) => i,
            other => panic!("{other:?}"),
        };
        let ok = c
            .call(Cmd::AgentLaunch(v1::AgentLaunch {
                path: info.path,
                project_id: info.project_id,
                kind: kind.into(),
                launch_options: ids,
                confirmed_launch_options: confirmed,
                ..Default::default()
            }))
            .unwrap()?;
        match ok.result {
            Some(Ok_::AgentLaunch(r)) => Ok(r.agent_id),
            other => panic!("{other:?}"),
        }
    }

    fn reports(&self) -> Vec<Vec<String>> {
        std::fs::read_dir(&self.root)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("probe-"))
            .filter_map(|e| {
                serde_json::from_slice::<serde_json::Value>(&std::fs::read(e.path()).ok()?).ok()
            })
            .map(|v| {
                v["args"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|a| a.as_str().unwrap().to_string())
                    .collect()
            })
            .collect()
    }

    fn wait_reports(&self, n: usize) -> Vec<Vec<String>> {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            let r = self.reports();
            if r.len() >= n {
                return r;
            }
            assert!(Instant::now() < end, "no probe report");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn agents(&self) -> usize {
        self.node().supervisor.lock().unwrap().list().len()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        self.node.take();
        let _ = std::fs::remove_dir_all(&self.root);
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

fn forbidden(e: &v1::ErrorInfo) -> String {
    assert_eq!(e.code, "agent.option_forbidden", "{e:?}");
    match e.params.get("policy").and_then(|p| p.value.as_ref()) {
        Some(v1::param_value::Value::Str(s)) => s.clone(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn by_default_the_option_is_allowed_and_its_flag_is_on_the_command_line() {
    if !supported() {
        return;
    }
    let e = Env::new("allow", None);
    // The window learns the options (never the arguments) from `agent.kinds`.
    let mut c = Client::connect(e.node().socket().unwrap()).unwrap();
    let kinds = match c
        .call(Cmd::AgentKinds(v1::AgentKinds {}))
        .unwrap()
        .unwrap()
        .result
    {
        Some(Ok_::AgentKinds(k)) => k.kinds,
        other => panic!("{other:?}"),
    };
    let probe = kinds.iter().find(|k| k.kind == "probe").unwrap();
    assert_eq!(
        probe.launch_options,
        [
            v1::LaunchOption {
                id: "skip-permissions".into(),
                label: "option-skip-permissions".into(),
                skips_permissions: true,
            },
            v1::LaunchOption {
                id: "resume-last".into(),
                label: "option-resume-last".into(),
                skips_permissions: false,
            },
        ]
    );

    let plain = e.launch("probe", &[]).unwrap();
    let skip = e
        .launch("probe", &["resume-last", "skip-permissions!"])
        .unwrap();
    let reports = e.wait_reports(2);
    let with: Vec<_> = reports
        .iter()
        .filter(|a| a.contains(&FLAG.to_string()))
        .collect();
    assert_eq!(with.len(), 1, "{reports:?}");
    // The manifest's order: the launch command, then the options' arguments.
    let args = with[0];
    let at = |s: &str| args.iter().position(|a| a == s).unwrap();
    assert!(at("--probe-directory") < at(FLAG) && at(FLAG) < at("--fake-continue"));
    let views = e.node().supervisor.lock().unwrap().list();
    let view = |id: &str| views.iter().find(|v| v.id.as_str() == id).unwrap();
    assert!(!view(&plain).skips_permissions && view(&plain).launch_options.is_empty());
    assert!(view(&skip).skips_permissions);
    assert_eq!(
        view(&skip).launch_options,
        ["skip-permissions", "resume-last"]
    );
    let events = e.node().events.events_after(0);
    assert!(events.iter().any(|ev| matches!(&ev.body, Some(Body::AgentStarted(s))
        if s.agent_id == skip && s.skips_permissions && s.launch_options == ["skip-permissions", "resume-last"])));
}

#[test]
fn forbid_refuses_before_anything_starts_and_a_typo_forbids_too() {
    if !supported() {
        return;
    }
    for (name, policy) in [
        ("forbid", "skip_permissions = forbid\n"),
        ("typo", "skip_permissions = alow\n"),
    ] {
        let e = Env::new(name, Some(policy));
        for kind in ["probe", "adapter"] {
            let err = e.launch(kind, &["skip-permissions!"]).unwrap_err();
            assert_eq!(forbidden(&err), "forbid");
        }
        assert_eq!(e.agents(), 0, "nothing started");
        // A harmless option is not the policy's business; an unknown one is a bad request.
        e.launch("probe", &["resume-last"]).unwrap();
        let err = e.launch("probe", &["no-such-option"]).unwrap_err();
        assert_eq!(err.code, "agent.option_invalid");
        let err = e
            .launch("probe", &["resume-last", "resume-last"])
            .unwrap_err();
        assert_eq!(err.code, "agent.option_invalid");
        let reports = e.wait_reports(1);
        assert!(reports.iter().all(|a| !a.contains(&FLAG.to_string())));
    }
}

#[test]
fn confined_only_needs_the_agents_confinement() {
    if !supported() {
        return;
    }
    let e = Env::new("confined-off", Some("skip_permissions = confined_only\n"));
    let err = e.launch("probe", &["skip-permissions!"]).unwrap_err();
    assert_eq!(forbidden(&err), "confined_only");
    assert_eq!(e.agents(), 0);
    drop(e);
    if !agora_platform::sandbox_supported() {
        return;
    }
    let e = Env::new(
        "confined-on",
        Some("skip_permissions = confined_only\nsandbox = worktree\n"),
    );
    // Confined, the same start goes through (the confined probe cannot write its report outside its folder, so the view says it).
    let id = e.launch("probe", &["skip-permissions!"]).unwrap();
    let views = e.node().supervisor.lock().unwrap().list();
    assert!(views
        .iter()
        .any(|v| v.id.as_str() == id && v.skips_permissions));
}

/// A structured kind gets the option's arguments on its driver's command (the ACP server command for `[structured] acp`; here the
/// fake L3 adapter, the same path). Read from the running process's command line where the system shows it.
#[test]
fn a_structured_agent_gets_the_arguments_on_its_command() {
    if !supported() || !Path::new("/proc/self/cmdline").exists() {
        return;
    }
    let e = Env::new("structured", None);
    let id = e.launch("adapter", &["skip-permissions!"]).unwrap();
    let pids = e.node().supervisor.lock().unwrap().running_pids();
    let lines: Vec<String> = pids
        .iter()
        .filter_map(|p| std::fs::read(format!("/proc/{p}/cmdline")).ok())
        .map(|b| String::from_utf8_lossy(&b).replace('\0', " "))
        .collect();
    assert!(
        lines
            .iter()
            .any(|l| l.contains("--adapter") && l.contains(FLAG)),
        "{lines:?}"
    );
    let views = e.node().supervisor.lock().unwrap().list();
    assert!(views
        .iter()
        .any(|v| v.id.as_str() == id && v.skips_permissions));
}

/// The review's case: a hub relays `agent.kinds` with `skips_permissions` turned off and a harmless label, so the window shows the
/// option as a plain checkbox, asks nothing, and signs the bare id. The node knows from its own manifest that the option skips
/// permissions and refuses the start without the signed confirmation marker; with it, the start goes through.
#[test]
fn a_hub_that_hides_a_skip_option_as_harmless_gets_the_start_refused() {
    if !supported() {
        return;
    }
    let e = Env::new("unconfirmed", None);
    for kind in ["probe", "adapter"] {
        let err = e.launch(kind, &["skip-permissions"]).unwrap_err();
        assert_eq!(err.code, "agent.option_unconfirmed", "{err:?}");
    }
    assert_eq!(e.agents(), 0, "nothing started");
    let id = e.launch("probe", &["skip-permissions!"]).unwrap();
    let views = e.node().supervisor.lock().unwrap().list();
    let v = views.iter().find(|v| v.id.as_str() == id).unwrap();
    // The marker is the user's confirmation, not part of the id the agent carries.
    assert!(v.skips_permissions);
    assert_eq!(v.launch_options, ["skip-permissions"]);
}

/// The second review's case: a hub advertises `skip-permissions!` as a harmless option, the window's checkbox would sign exactly that
/// id. A `!` is never a confirmation and never part of an id: the node refuses it (`agent.option_invalid`) and nothing starts; an id
/// with a comma is refused the same way.
#[test]
fn an_advertised_id_with_a_bang_or_a_comma_is_refused() {
    if !supported() {
        return;
    }
    let e = Env::new("bang", None);
    for kind in ["probe", "adapter"] {
        for id in ["skip-permissions!", "a,b", "Skip-Permissions"] {
            let err = e.launch_raw(kind, vec![id.into()], Vec::new()).unwrap_err();
            assert_eq!(err.code, "agent.option_invalid", "{id}: {err:?}");
        }
    }
    // A confirmation for an id that was not chosen is not a way in either.
    let err = e
        .launch_raw(
            "probe",
            vec!["resume-last".into()],
            vec!["skip-permissions".into()],
        )
        .unwrap_err();
    assert_eq!(err.code, "agent.option_invalid");
    assert_eq!(e.agents(), 0, "nothing started");
    // The legitimate confirmed start still works.
    e.launch_raw(
        "probe",
        vec!["skip-permissions".into()],
        vec!["skip-permissions".into()],
    )
    .unwrap();
}

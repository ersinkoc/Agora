//! F1020: a crashed folder agent's restart holds its folder again by the path its start held it at, and starts in it or not
//! at all. Gated: the agent waits for a gate file before it exits non-zero; each change is made before the gate opens, and restarts
//! are driven by `tick` with a time past the back-off. Cases: unchanged (control), the folder swapped for a link, a folder above it
//! swapped, the swap undone before a later restart (it recovers), and a start whose path was spelled through a link (as a temporary
//! folder is on macOS) still restarts.
#![allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "private_home/mod.rs"]
mod private_home;
use agora_adapters::Manifest;
use agora_core::{NodeId, ProjectId};
use agora_node::supervisor::{
    NodeLimits, RestartPolicy, StartRequest, Supervisor, SupervisorConfig, SupervisorEvent,
};
use agora_node::terminal::TerminalOptions;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

struct Case {
    base: PathBuf,
    own: PathBuf,
    other: PathBuf,
    reports: PathBuf,
    gate: PathBuf,
}

fn case(name: &str) -> Case {
    private_home::init();
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("restart-folder-swap-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let base = std::fs::canonicalize(&base).unwrap();
    let c = Case {
        own: base.join("own/x"),
        other: base.join("other/x"),
        reports: base.join("reports"),
        gate: base.join("gate"),
        base,
    };
    for d in [&c.own, &c.other, &c.reports] {
        std::fs::create_dir_all(d).unwrap();
    }
    c
}

fn supervisor(c: &Case) -> Supervisor {
    let script = format!(
        "pwd -P > {r}/run-$$.tmp && mv {r}/run-$$.tmp {r}/run-$$; while [ ! -e {g} ]; do sleep 0.05; done; exit 3",
        r = c.reports.display(),
        g = c.gate.display()
    );
    let manifest = Manifest::parse(&format!(
        "kind = \"crash\"\ndisplay_name = \"Crash\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"/bin/sh\", \"-c\", {script:?}]\n"
    ))
    .unwrap();
    let mut sup = Supervisor::new(SupervisorConfig {
        node: NodeId::new("node-1"),
        restart: RestartPolicy::default(),
        terminal: TerminalOptions::default(),
        limits: NodeLimits::default(),
    });
    sup.register(manifest);
    sup
}

fn next_report(c: &Case, seen: &mut Vec<String>) -> Option<String> {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        for e in std::fs::read_dir(&c.reports).unwrap().flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.ends_with(".tmp") || seen.contains(&n) {
                continue;
            }
            seen.push(n);
            return Some(
                std::fs::read_to_string(e.path())
                    .unwrap()
                    .trim()
                    .to_string(),
            );
        }
        if Instant::now() > end {
            return None;
        }
        std::thread::yield_now();
    }
}

struct Run {
    sup: Supervisor,
    seen: Vec<String>,
    later: Instant,
}

impl Run {
    /// Started in `checkout` (the path as the caller spells it); its first report must be `own/x`.
    fn start(c: &Case, checkout: &Path) -> Run {
        let mut sup = supervisor(c);
        let req = StartRequest {
            kind: "crash".into(),
            checkout: checkout.to_path_buf(),
            worktree_root: c.base.join("node").join("worktrees"),
            name: "a".into(),
            base: None,
        };
        sup.start_selected(
            &req,
            ProjectId::directory("node-1", &c.own.to_string_lossy()),
            Path::new(""),
            false,
            Instant::now(),
        )
        .expect("first start");
        let mut seen = Vec::new();
        assert_eq!(
            next_report(c, &mut seen).as_deref(),
            Some(c.own.display().to_string().as_str()),
            "the first start"
        );
        Run {
            sup,
            seen,
            later: Instant::now(),
        }
    }

    /// Opens the gate (the running process exits non-zero), waits for the exit, then drives one restart. Returns where the restarted
    /// process ran (None: not started) and the events of the restart attempt.
    fn crash_and_restart(&mut self, c: &Case) -> (Option<String>, Vec<SupervisorEvent>) {
        std::fs::write(&c.gate, "").unwrap();
        let end = Instant::now() + Duration::from_secs(20);
        let mut events = Vec::new();
        while !events.iter().any(|e| {
            matches!(
                e,
                SupervisorEvent::Restarting { .. } | SupervisorEvent::GaveUp { .. }
            )
        }) {
            assert!(Instant::now() < end, "no exit seen: {events:?}");
            events.extend(self.sup.tick(Instant::now()));
            std::thread::yield_now();
        }
        let _ = std::fs::remove_file(&c.gate);
        self.later += Duration::from_secs(3600);
        let restarted = self.sup.tick(self.later);
        let started = restarted
            .iter()
            .any(|e| matches!(e, SupervisorEvent::Started { .. }));
        let ran = if started {
            next_report(c, &mut self.seen)
        } else {
            None
        };
        (ran, restarted)
    }

    /// A restart that was refused is retried after its back-off: drives it.
    fn retry(&mut self, c: &Case) -> (Option<String>, Vec<SupervisorEvent>) {
        self.later += Duration::from_secs(3600);
        let restarted = self.sup.tick(self.later);
        let started = restarted
            .iter()
            .any(|e| matches!(e, SupervisorEvent::Started { .. }));
        let ran = if started {
            next_report(c, &mut self.seen)
        } else {
            None
        };
        (ran, restarted)
    }
}

fn swap(from: &Path, to: &Path) {
    std::fs::rename(from, from.with_file_name("swapped.old")).unwrap();
    agora_platform::symlink(to, from).unwrap();
}

fn unswap(at: &Path) {
    std::fs::remove_file(at).unwrap();
    std::fs::rename(at.with_file_name("swapped.old"), at).unwrap();
}

#[test]
fn a_restart_starts_in_the_folder_its_start_held_or_not_at_all() {
    // The agent is a shell script and the swaps make links: where either is unavailable (Windows), nothing to run.
    let probe = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("restart-folder-swap-link-{}", std::process::id()));
    let _ = std::fs::remove_file(&probe);
    let links = agora_platform::symlink(Path::new("."), &probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    if !(links && agora_platform::PTY_SUPPORTED && Path::new("/bin/sh").exists()) {
        return;
    }
    let refused = |ev: &[SupervisorEvent]| {
        ev.iter()
            .any(|e| matches!(e, SupervisorEvent::Restarting { .. }))
            && !ev
                .iter()
                .any(|e| matches!(e, SupervisorEvent::Started { .. }))
    };

    // Control: unchanged, the restart runs in the same folder.
    let c = case("control");
    let mut r = Run::start(&c, &c.own);
    let (ran, _) = r.crash_and_restart(&c);
    assert_eq!(ran.as_deref(), Some(c.own.to_str().unwrap()));
    drop(r);
    let _ = std::fs::remove_dir_all(&c.base);

    // The folder swapped for a link: refused (and retried after its back-off), never run in the other folder.
    let c = case("folder");
    let mut r = Run::start(&c, &c.own);
    swap(&c.own, &c.other);
    let (ran, ev) = r.crash_and_restart(&c);
    assert_eq!(ran, None);
    assert!(refused(&ev), "{ev:?}");
    // The swap undone: the next attempt starts in the folder again.
    unswap(&c.own);
    let (ran, _) = r.retry(&c);
    assert_eq!(ran.as_deref(), Some(c.own.to_str().unwrap()));
    drop(r);
    let _ = std::fs::remove_dir_all(&c.base);

    // A folder above it swapped for a link: refused.
    let c = case("above");
    let mut r = Run::start(&c, &c.own);
    swap(c.own.parent().unwrap(), c.other.parent().unwrap());
    let (ran, ev) = r.crash_and_restart(&c);
    assert_eq!(ran, None);
    assert!(refused(&ev), "{ev:?}");
    drop(r);
    let _ = std::fs::remove_dir_all(&c.base);

    // Started by a path spelled through a link the owner made (a temporary folder through `/var` on macOS): restarts as before.
    let c = case("spelled");
    let alias = c.base.join("alias");
    agora_platform::symlink(&c.base, &alias).unwrap();
    let mut r = Run::start(&c, &alias.join("own/x"));
    let (ran, _) = r.crash_and_restart(&c);
    assert_eq!(ran.as_deref(), Some(c.own.to_str().unwrap()));
    drop(r);
    let _ = std::fs::remove_dir_all(&c.base);
}

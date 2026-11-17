//! F1023: cases: unchanged (control), the folder swapped for a link (refused), the swap undone (the next
//! attempt starts), a folder above swapped (refused), and the folder swapped while the restarted adapter starts (the adapter's own
//! shell swaps it after it began, before it answers: refused after the start, the adapter stopped). A crashed folder adapter (L3)
//! agent is started again (`restart_adapter`) by its path; its folder is held again and must be in place before and after. Gated: the adapter is a shell that
//! reports where it runs and becomes the fake adapter; the test swaps the folder, then ends the adapter process it started (its pid is
//! in its report), waits for the crash to be seen, and drives the restart with a time past its back-off.
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
use std::process::Command;
use std::time::{Duration, Instant};

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

struct Case {
    pub base: PathBuf,
    pub own: PathBuf,
    pub other: PathBuf,
    pub reports: PathBuf,
}

fn case(tag: &str, name: &str) -> Case {
    private_home::init();
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("{tag}-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let base = std::fs::canonicalize(&base).unwrap();
    let c = Case {
        own: base.join("own/x"),
        other: base.join("other/x"),
        reports: base.join("reports"),
        base,
    };
    for d in [&c.own, &c.other, &c.reports] {
        std::fs::create_dir_all(d).unwrap();
    }
    c
}

/// A report: (pid, where it ran).
fn next_report(c: &Case, seen: &mut Vec<String>) -> Option<(String, String)> {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        for e in std::fs::read_dir(&c.reports).unwrap().flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.ends_with(".tmp") || seen.contains(&n) {
                continue;
            }
            seen.push(n.clone());
            let pid = n.trim_start_matches("run-").to_string();
            return Some((
                pid,
                std::fs::read_to_string(e.path())
                    .unwrap()
                    .trim()
                    .to_string(),
            ));
        }
        if Instant::now() > end {
            return None;
        }
        std::thread::yield_now();
    }
}

struct Run {
    sup: Supervisor,
    id: agora_core::AgentId,
    seen: Vec<String>,
    pid: String,
    later: Instant,
}

impl Run {
    fn start(c: &Case) -> Run {
        let script = format!(
            "pwd -P > {r}/run-$$.tmp && mv {r}/run-$$.tmp {r}/run-$$; \
             if [ -e {b}/swap-now ]; then rm {b}/swap-now; mv {o} {o}.old && ln -s {t} {o}; fi; \
             if [ -e {b}/linger ]; then rm {b}/linger; sleep 120 & echo $! > {b}/lingering; fi; exec {f} --adapter",
            r = c.reports.display(),
            b = c.base.display(),
            o = c.own.display(),
            t = c.other.display(),
            f = fake()
        );
        let manifest = Manifest::parse(&format!(
            "kind = \"adapt\"\ndisplay_name = \"Adapt\"\nversion = \"1\"\nmin_engine = 1\n[adapter]\ncommand = [\"/bin/sh\", \"-c\", {script:?}]\n"
        ))
        .unwrap();
        let mut sup = Supervisor::new(SupervisorConfig {
            node: NodeId::new("node-1"),
            restart: RestartPolicy::default(),
            terminal: TerminalOptions::default(),
            limits: NodeLimits::default(),
        });
        sup.register(manifest);
        let req = StartRequest {
            kind: "adapt".into(),
            checkout: c.own.clone(),
            worktree_root: c.base.join("node").join("worktrees"),
            name: "a".into(),
            base: None,
        };
        let id = sup
            .start_selected(
                &req,
                ProjectId::directory("node-1", &c.own.to_string_lossy()),
                Path::new(""),
                false,
                Instant::now(),
            )
            .expect("first start");
        let mut seen = Vec::new();
        let (pid, at) = next_report(c, &mut seen).expect("first report");
        assert_eq!(at, c.own.display().to_string(), "the first start");
        Run {
            sup,
            id,
            seen,
            pid,
            later: Instant::now(),
        }
    }

    /// Ends the adapter process this test started, waits until the crash is seen, and drives one restart. Returns where the
    /// restarted adapter ran (None: not started) and the restart's events.
    fn crash_and_restart(&mut self, c: &Case) -> (Option<String>, Vec<SupervisorEvent>) {
        assert!(Command::new("kill")
            .args(["-9", &self.pid])
            .status()
            .unwrap()
            .success());
        let end = Instant::now() + Duration::from_secs(20);
        let mut events = Vec::new();
        while !events.iter().any(|e| {
            matches!(
                e,
                SupervisorEvent::Restarting { .. } | SupervisorEvent::GaveUp { .. }
            )
        }) {
            assert!(Instant::now() < end, "no crash seen: {events:?}");
            events.extend(self.sup.tick(Instant::now()));
            std::thread::yield_now();
        }
        self.retry(c)
    }

    fn retry(&mut self, c: &Case) -> (Option<String>, Vec<SupervisorEvent>) {
        self.later += Duration::from_secs(3600);
        let restarted = self.sup.tick(self.later);
        let started = restarted
            .iter()
            .any(|e| matches!(e, SupervisorEvent::Started { .. }));
        let ran = if started {
            next_report(c, &mut self.seen).map(|(pid, at)| {
                self.pid = pid;
                at
            })
        } else {
            None
        };
        (ran, restarted)
    }
}

#[test]
fn an_adapter_restart_starts_in_the_folder_its_start_held_or_not_at_all() {
    // The adapter is a shell script and the swaps make links: where either is unavailable (Windows), nothing to run.
    let probe = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "adapter-restart-folder-swap-link-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&probe);
    let links = agora_platform::symlink(Path::new("."), &probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    if !(links && Path::new("/bin/sh").exists()) {
        return;
    }
    let started = |ev: &[SupervisorEvent]| {
        ev.iter()
            .any(|e| matches!(e, SupervisorEvent::Started { .. }))
    };
    let own_s = |c: &Case| c.own.display().to_string();

    let c = case("adapter-restart-folder-swap", "control");
    let mut r = Run::start(&c);
    let (ran, _) = r.crash_and_restart(&c);
    assert_eq!(ran, Some(own_s(&c)));
    drop(r);
    let _ = std::fs::remove_dir_all(&c.base);

    let c = case("adapter-restart-folder-swap", "folder");
    let mut r = Run::start(&c);
    std::fs::rename(&c.own, c.own.with_file_name("x.old")).unwrap();
    agora_platform::symlink(&c.other, &c.own).unwrap();
    let (ran, ev) = r.crash_and_restart(&c);
    assert!(ran.is_none() && !started(&ev), "{ev:?}");
    std::fs::remove_file(&c.own).unwrap();
    std::fs::rename(c.own.with_file_name("x.old"), &c.own).unwrap();
    let (ran, _) = r.retry(&c);
    assert_eq!(ran, Some(own_s(&c)));
    drop(r);
    let _ = std::fs::remove_dir_all(&c.base);

    let c = case("adapter-restart-folder-swap", "above");
    let mut r = Run::start(&c);
    let (up, to) = (c.own.parent().unwrap(), c.other.parent().unwrap());
    std::fs::rename(up, up.with_file_name("own.old")).unwrap();
    agora_platform::symlink(to, up).unwrap();
    let (ran, ev) = r.crash_and_restart(&c);
    assert!(ran.is_none() && !started(&ev), "{ev:?}");
    drop(r);
    let _ = std::fs::remove_dir_all(&c.base);

    let c = case("adapter-restart-folder-swap", "during");
    let mut r = Run::start(&c);
    std::fs::write(c.base.join("swap-now"), "").unwrap();
    let (ran, ev) = r.crash_and_restart(&c);
    assert!(ran.is_none() && !started(&ev), "{ev:?}");
    assert!(
        !c.base.join("swap-now").exists(),
        "the restarted adapter did run (and swapped)"
    );
    drop(r);
    let _ = std::fs::remove_dir_all(&c.base);

    // Codex review: the same, with a process below the adapter that keeps its output open. The refused adapter is ended with
    // everything below it, and the supervisor does not wait for that process (the restart returns at once).
    let c = case("adapter-restart-folder-swap", "lingering");
    let mut r = Run::start(&c);
    std::fs::write(c.base.join("swap-now"), "").unwrap();
    std::fs::write(c.base.join("linger"), "").unwrap();
    let begun = Instant::now();
    let (ran, ev) = r.crash_and_restart(&c);
    let took = begun.elapsed();
    assert!(ran.is_none() && !started(&ev), "{ev:?}");
    assert!(
        took < Duration::from_secs(60),
        "the refused restart waited for the process below the adapter: {took:?}"
    );
    let lingering = std::fs::read_to_string(c.base.join("lingering")).unwrap();
    let end = Instant::now() + Duration::from_secs(20);
    while Command::new("kill")
        .args(["-0", lingering.trim()])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success()
    {
        assert!(
            Instant::now() < end,
            "the process below the refused adapter still runs"
        );
        std::thread::yield_now();
    }
    drop(r);
    let _ = std::fs::remove_dir_all(&c.base);
}

/// Codex review: an adapter agent's resume holds its selected folder and checks it right before the adapter is started, so a folder
/// swapped for a link after the hold (here by the mesh hook, which runs between the hold and the start) is never started in. The
/// control: the same resume with no swap runs in the folder.
#[test]
fn an_adapter_resume_starts_in_the_folder_its_start_held_or_not_at_all() {
    let probe = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "adapter-resume-folder-swap-link-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&probe);
    let links = agora_platform::symlink(Path::new("."), &probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    if !(links && Path::new("/bin/sh").exists()) {
        return;
    }
    for swap in [false, true] {
        let c = case(
            "adapter-resume-folder-swap",
            if swap { "swap" } else { "control" },
        );
        let mut r = Run::start(&c);
        r.sup.stop(&r.id).expect("stop");
        if swap {
            let (own, other) = (c.own.clone(), c.other.clone());
            r.sup.set_launch_hook(Box::new(move |_| {
                std::fs::rename(&own, own.with_file_name("x.old")).unwrap();
                agora_platform::symlink(&other, &own).unwrap();
                None
            }));
        }
        let resumed = r.sup.resume(&r.id, Instant::now());
        if swap {
            assert!(resumed.is_err(), "{resumed:?}");
            // Nothing ran: no report at all (the adapter reports where it runs as its first act).
            let any = std::fs::read_dir(&c.reports)
                .unwrap()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .find(|n| !r.seen.contains(n));
            assert!(any.is_none(), "an adapter ran: {any:?}");
        } else {
            resumed.expect("resume");
            let (_, at) = next_report(&c, &mut r.seen).expect("resumed report");
            assert_eq!(at, c.own.display().to_string());
        }
        drop(r);
        let _ = std::fs::remove_dir_all(&c.base);
    }
}

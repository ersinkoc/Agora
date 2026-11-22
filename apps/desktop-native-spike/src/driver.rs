//! `agora-native-spike run`: the measurement harness. Starts a disposable hub and node (its own `AGORA_HOME`, a free port), starts
//! the fixture agents, launches the app once per fixture, samples what only the outside can see, and writes one JSON file per run
//! plus `summary.json` with the ADR-0099 go/no-go numbers. The same command on Linux, macOS and Windows.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::metrics::epoch_ms;
use crate::os;

struct Opts {
    agora: Option<PathBuf>,
    out: PathBuf,
    only: Option<Vec<String>>,
    warm_runs: usize,
    label: String,
    not_representative: Option<String>,
    keep: bool,
}

fn parse(args: &[String]) -> Opts {
    let mut o = Opts {
        agora: None,
        out: PathBuf::from("spike-results"),
        only: None,
        warm_runs: 5,
        label: String::new(),
        not_representative: None,
        keep: false,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--agora" => o.agora = it.next().map(PathBuf::from),
            "--out" => o.out = it.next().map(PathBuf::from).unwrap_or(o.out),
            "--only" => {
                o.only = it
                    .next()
                    .map(|s| s.split(',').map(str::to_string).collect())
            }
            "--warm-runs" => o.warm_runs = it.next().and_then(|s| s.parse().ok()).unwrap_or(5),
            "--label" => o.label = it.next().cloned().unwrap_or_default(),
            "--not-representative" => o.not_representative = it.next().cloned(),
            "--keep" => o.keep = true,
            _ => {}
        }
    }
    o
}

struct Stack {
    children: Vec<Child>,
    work: PathBuf,
    hub_dir: PathBuf,
    addr: String,
    env: Vec<(String, String)>,
    agora: PathBuf,
}

impl Stack {
    fn run(&self, args: &[&str]) -> String {
        let mut c = Command::new(&self.agora);
        c.args(args);
        for (k, v) in &self.env {
            c.env(k, v);
        }
        c.output()
            .map(|o| {
                format!(
                    "{}{}",
                    String::from_utf8_lossy(&o.stdout),
                    String::from_utf8_lossy(&o.stderr)
                )
            })
            .unwrap_or_default()
    }

    fn login_code(&self) -> Option<String> {
        let out = self.run(&["hub", "login", "--hub-dir", &self.hub_dir.to_string_lossy()]);
        // "... app: <code> (..." as the browser harness reads it.
        out.split("app: ")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .map(str::to_string)
    }
}

impl Drop for Stack {
    fn drop(&mut self) {
        for c in self.children.iter_mut().rev() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn code_in(s: &str) -> Option<String> {
    // XXXX-XXXX, letters and digits.
    s.split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
        .find(|w| {
            w.len() == 9
                && w.as_bytes()[4] == b'-'
                && w.chars()
                    .all(|c| c == '-' || c.is_ascii_uppercase() || c.is_ascii_digit())
        })
        .map(str::to_string)
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map_or(17543, |a| a.port())
}

fn repo_root(exe: &Path) -> Option<PathBuf> {
    let mut p = exe.parent()?.to_path_buf();
    loop {
        if p.join("crates").join("agora-vt").is_dir() {
            return Some(p);
        }
        if !p.pop() {
            return None;
        }
    }
}

fn toml_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

fn start_stack(agora: &Path, exe: &Path) -> Result<Stack, String> {
    let work = std::env::temp_dir().join(format!("agsp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    let home = work.join("h");
    let hub_dir = work.join("hub");
    let proj = work.join("proj");
    std::fs::create_dir_all(home.join("manifests")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&proj).map_err(|e| e.to_string())?;
    let exe_s = exe.to_string_lossy().to_string();
    let manifest = |kind: &str, launch: &[&str]| {
        let l: Vec<String> = launch.iter().map(|s| toml_str(s)).collect();
        format!("kind = \"{kind}\"\ndisplay_name = \"{kind}\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [{}]\n", l.join(", "))
    };
    let files = [
        (
            "spike-echo",
            manifest("spike-echo", &[&exe_s, "helper", "echo"]),
        ),
        (
            "spike-flood",
            manifest(
                "spike-flood",
                &[&exe_s, "helper", "flood", "10000000", "14"],
            ),
        ),
        (
            "spike-trickle",
            manifest("spike-trickle", &[&exe_s, "helper", "flood", "2000", "14"]),
        ),
    ];
    for (k, body) in files {
        std::fs::write(home.join("manifests").join(format!("{k}.toml")), body)
            .map_err(|e| e.to_string())?;
    }
    if std::env::consts::OS != "windows" {
        // End-to-end probe: a real program turns SGR mouse on and prints each byte it receives in hex.
        let probe = manifest("spike-probe", &[&exe_s, "helper", "probe"]);
        std::fs::write(home.join("manifests").join("spike-probe.toml"), probe)
            .map_err(|e| e.to_string())?;
    }
    let env = vec![
        ("AGORA_HOME".to_string(), home.to_string_lossy().to_string()),
        ("AGORA_LANG".to_string(), "en".to_string()),
        ("GIT_AUTHOR_NAME".to_string(), "t".to_string()),
        ("GIT_AUTHOR_EMAIL".to_string(), "t@x".to_string()),
        ("GIT_COMMITTER_NAME".to_string(), "t".to_string()),
        ("GIT_COMMITTER_EMAIL".to_string(), "t@x".to_string()),
    ];
    for a in [
        &["init", "-q", "-b", "main"][..],
        &["commit", "-q", "--allow-empty", "-m", "init"],
    ] {
        let mut c = Command::new("git");
        c.arg("-C").arg(&proj).args(a);
        for (k, v) in &env {
            c.env(k, v);
        }
        c.status().map_err(|e| format!("git: {e}"))?;
    }
    let port = free_port();
    let addr = format!("127.0.0.1:{port}");
    let mut stack = Stack {
        children: Vec::new(),
        work: work.clone(),
        hub_dir: hub_dir.clone(),
        addr: addr.clone(),
        env,
        agora: agora.to_path_buf(),
    };
    let spawn = |stack: &Stack, args: &[&str]| -> Result<Child, String> {
        let mut c = Command::new(&stack.agora);
        c.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for (k, v) in &stack.env {
            c.env(k, v);
        }
        c.spawn().map_err(|e| format!("spawn {args:?}: {e}"))
    };
    let hub = spawn(
        &stack,
        &[
            "hub",
            "run",
            "--listen",
            &addr,
            "--hub-dir",
            &hub_dir.to_string_lossy(),
        ],
    )?;
    stack.children.push(hub);
    let end = Instant::now() + Duration::from_secs(60);
    while std::net::TcpStream::connect(&addr).is_err() {
        if Instant::now() > end {
            return Err("the hub did not start listening".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    std::thread::sleep(Duration::from_secs(1));
    let code = code_in(&stack.run(&["hub", "code", "--hub-dir", &hub_dir.to_string_lossy()]))
        .ok_or("no enrollment code")?;
    let enrolled = stack.run(&[
        "node",
        "enroll",
        "--hub",
        &format!("wss://localhost:{port}"),
        "--ca",
        &hub_dir.join("ca.pem").to_string_lossy(),
        "--code",
        &code,
        "--id",
        "laptop",
    ]);
    let node = spawn(
        &stack,
        &["node", "run", "--project", &proj.to_string_lossy()],
    )
    .map_err(|e| format!("{e} ({enrolled})"))?;
    stack.children.push(node);
    std::thread::sleep(Duration::from_secs(3));
    Ok(stack)
}

/// Starts the fixture agents through the hub (`agent.start`, as any client) and returns their hub-wide ids by role.
fn start_agents(stack: &Stack) -> Result<Value, String> {
    use agora_tui::client::{sign_in, Hub, Session, Update};
    let ca_pem =
        std::fs::read_to_string(stack.hub_dir.join("ca.pem")).map_err(|e| e.to_string())?;
    let hub = Hub {
        addr: stack.addr.clone(),
        ca_pem,
    };
    let code = stack.login_code().ok_or("no login code")?;
    let token = sign_in(&hub, &code).map_err(|e| format!("sign in: {e:?}"))?;
    let mut s = Session::connect(&hub, &token).map_err(|e| format!("connect: {e:?}"))?;
    let end = Instant::now() + Duration::from_secs(30);
    let mut project = None;
    while project.is_none() && Instant::now() < end {
        if let Update::Snapshot(snap) = s.next(Duration::from_millis(250)) {
            let online = snap.nodes.iter().any(|n| n.node_id == "laptop" && n.online);
            project = online
                .then(|| {
                    snap.checkouts
                        .iter()
                        .find(|c| c.node_id == "laptop")
                        .map(|c| c.project_id.clone())
                })
                .flatten();
            if project.is_none() {
                // Ask again: the hub sends a fresh snapshot to a new connection.
                std::thread::sleep(Duration::from_millis(500));
                s = Session::connect(&hub, &token).map_err(|e| format!("connect: {e:?}"))?;
            }
        }
    }
    let project = project.ok_or("the node or its project did not appear")?;
    let mut roles = serde_json::Map::new();
    let mut plan = vec![
        ("echo", "spike-echo", 1),
        ("flood", "spike-flood", 4),
        ("trickle", "spike-trickle", 20),
    ];
    if std::env::consts::OS != "windows" {
        plan.push(("probe", "spike-probe", 1));
    }
    for (role, kind, n) in plan {
        let mut ids = Vec::new();
        for _ in 0..n {
            let id = s
                .start_agent("laptop", &project, kind)
                .map_err(|e| format!("start {kind}: {e}"))?;
            ids.push(json!(if id.contains('/') {
                id
            } else {
                format!("laptop/{id}")
            }));
        }
        roles.insert(role.to_string(), Value::Array(ids));
    }
    roles.insert("flood_secs".into(), json!(14));
    // Let the agents' terminals come up.
    std::thread::sleep(Duration::from_secs(3));
    Ok(Value::Object(roles))
}

struct AppRun {
    result: Value,
    driver: serde_json::Map<String, Value>,
}

/// A process's CPU time and when it was read: the middle of the call, since a sample can take half a second (PowerShell on
/// Windows; on Linux a `/proc` read of a few milliseconds, so the numbers stay comparable with earlier Linux runs).
fn cpu_sample(pid: u32) -> Option<(Instant, f64)> {
    let a = Instant::now();
    let c = os::cpu_ms(pid)?;
    Some((a + a.elapsed() / 2, c))
}

fn run_app(
    exe: &Path,
    stack: &Stack,
    stack_file: &Path,
    fixture: &str,
    out: &Path,
    before_exit: Option<&mut dyn FnMut(u32)>,
) -> Result<AppRun, String> {
    let code = stack.login_code().ok_or("no login code")?;
    let result = out.join(format!("{fixture}.app.json"));
    let _ = std::fs::remove_file(&result);
    let spawn_ms = epoch_ms();
    let mut child = Command::new(exe)
        .args([
            "app",
            "--hub",
            &stack.addr,
            "--ca",
            &stack.hub_dir.join("ca.pem").to_string_lossy(),
            "--code",
            &code,
            "--fixture",
            fixture,
            "--result",
            &result.to_string_lossy(),
            "--stack",
            &stack_file.to_string_lossy(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("spawn app: {e}"))?;
    let pid = child.id();
    // F4 waits for a "sampled" line after each phase the driver samples, so the process is alive for the sample on every system.
    let mut stdin = child.stdin.take();
    let mut sampled = || {
        if let Some(s) = stdin.as_mut() {
            let _ = writeln!(s, "sampled").and_then(|()| s.flush());
        }
    };
    let (tx, rx) = mpsc::channel::<String>();
    if let Some(stdout) = child.stdout.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let _ = tx.send(line);
            }
        });
    }
    let mut d = serde_json::Map::new();
    d.insert("spawn_epoch_ms".into(), json!(spawn_ms as u64));
    let mut cpu_begin: Option<(Instant, f64)> = None;
    let end = Instant::now() + Duration::from_secs(180);
    let mut hook = before_exit;
    loop {
        if Instant::now() > end {
            let _ = child.kill();
            d.insert("timed_out".into(), json!(true));
            break;
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(line) => {
                let phase = line
                    .strip_prefix("SPIKE-PHASE ")
                    .and_then(|s| s.split_whitespace().next())
                    .unwrap_or("");
                match phase {
                    "f4-begin" => {
                        std::thread::sleep(Duration::from_millis(500));
                        d.insert("rss_bytes".into(), json!(os::rss(pid)));
                        d.insert("tree_rss_bytes".into(), json!(os::tree_rss(pid)));
                        cpu_begin = cpu_sample(pid);
                        sampled();
                    }
                    "f4-end" => {
                        if let (Some((t0, c0)), Some((t1, c1))) = (cpu_begin, cpu_sample(pid)) {
                            let wall = (t1 - t0).as_secs_f64() * 1000.0;
                            d.insert(
                                "idle_cpu_percent_of_one_core".into(),
                                json!((c1 - c0) / wall * 100.0),
                            );
                            d.insert("idle_cpu_window_ms".into(), json!(wall));
                        }
                        d.insert("rss_bytes_end".into(), json!(os::rss(pid)));
                        sampled();
                    }
                    "hold" => {
                        if let Some(h) = hook.as_mut() {
                            h(pid);
                        }
                    }
                    "done" => {
                        if let Some(h) = hook.as_mut() {
                            h(pid);
                        }
                        d.insert("rss_bytes_at_done".into(), json!(os::rss(pid)));
                    }
                    _ => {}
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if let Ok(Some(_)) = child.try_wait() {
            break;
        }
    }
    let status = child.wait().ok().and_then(|s| s.code());
    d.insert("exit_code".into(), json!(status));
    let result: Value = std::fs::read(&result)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(json!({"error": "no result file"}));
    let spawn = spawn_ms as f64;
    if let Some(f) = result["first_frame_epoch_ms"].as_f64() {
        d.insert("spawn_to_first_frame_ms".into(), json!(f - spawn));
    }
    if let Some(f) = result["first_useful_paint_epoch_ms"].as_f64() {
        d.insert("spawn_to_first_useful_paint_ms".into(), json!(f - spawn));
    }
    if let Some(f) = result["main_entered_epoch_ms"].as_f64() {
        d.insert("spawn_to_main_ms".into(), json!(f - spawn));
    }
    Ok(AppRun { result, driver: d })
}

fn run_app_hold(
    exe: &Path,
    stack: &Stack,
    stack_file: &Path,
    out: &Path,
    hook: &mut dyn FnMut(u32),
) -> Result<AppRun, String> {
    run_app(exe, stack, stack_file, "a11y", out, Some(hook))
}

fn save(out: &Path, name: &str, run: &AppRun) -> Value {
    let mut v = run.result.clone();
    if let Value::Object(o) = &mut v {
        o.insert("driver".into(), Value::Object(run.driver.clone()));
    }
    let _ = std::fs::write(
        out.join(format!("{name}.json")),
        serde_json::to_vec_pretty(&v).unwrap_or_default(),
    );
    v
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    if v.is_empty() {
        f64::NAN
    } else {
        v[v.len() / 2]
    }
}

pub fn main(args: &[String]) -> i32 {
    let o = parse(args);
    let Ok(exe) = std::env::current_exe() else {
        return 1;
    };
    let root = repo_root(&exe);
    let bin = if std::env::consts::OS == "windows" {
        "agora.exe"
    } else {
        "agora"
    };
    let agora = o.agora.clone().or_else(|| {
        let r = root.as_ref()?;
        [
            r.join("target/release").join(bin),
            r.join("target/debug").join(bin),
        ]
        .into_iter()
        .find(|p| p.is_file())
    });
    let Some(agora) = agora else {
        eprintln!(
            "no agora binary: build it (`cargo build --release -p agora`) or pass --agora PATH"
        );
        return 2;
    };
    let _ = std::fs::create_dir_all(&o.out);
    let out = o.out.canonicalize().unwrap_or(o.out.clone());
    eprintln!("hub and node from {}", agora.display());
    let stack = match start_stack(&agora, &exe) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("stack: {e}");
            return 1;
        }
    };
    let roles = match start_agents(&stack) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("agents: {e}");
            return 1;
        }
    };
    let stack_file = stack.work.join("stack.json");
    let _ = std::fs::write(
        &stack_file,
        serde_json::to_vec_pretty(&roles).unwrap_or_default(),
    );
    let want = |f: &str| o.only.as_ref().is_none_or(|l| l.iter().any(|x| x == f));
    let mut summary = serde_json::Map::new();
    let mut gates = serde_json::Map::new();

    if want("start") {
        // Cold: the app's binary and the libraries it maps evicted from the page cache first (Linux; elsewhere the first run after
        // the build is reported as "first run"). Warm: the median of the following runs.
        let mut mapped = Vec::new();
        let mut grab = |pid: u32| {
            if mapped.is_empty() {
                mapped = os::mapped_files(pid);
            }
        };
        let probe = run_app(&exe, &stack, &stack_file, "start", &out, Some(&mut grab));
        let mut files = mapped.clone();
        files.push(exe.to_string_lossy().to_string());
        let evicted = os::evict(&files);
        let cold = if evicted {
            run_app(&exe, &stack, &stack_file, "start", &out, None)
        } else {
            probe
        };
        let mut warm = Vec::new();
        let mut warm_first_frame = Vec::new();
        for i in 0..o.warm_runs {
            if let Ok(r) = run_app(&exe, &stack, &stack_file, "start", &out, None) {
                save(&out, &format!("start-warm-{i}"), &r);
                warm.extend(
                    r.driver
                        .get("spawn_to_first_useful_paint_ms")
                        .and_then(Value::as_f64),
                );
                warm_first_frame.extend(
                    r.driver
                        .get("spawn_to_first_frame_ms")
                        .and_then(Value::as_f64),
                );
            }
        }
        if let Ok(c) = &cold {
            save(&out, "start-cold", c);
            let cold_ms = c
                .driver
                .get("spawn_to_first_useful_paint_ms")
                .and_then(Value::as_f64)
                .unwrap_or(f64::NAN);
            gates.insert("cold_start_first_useful_paint_ms".into(), json!({"value": cold_ms, "limit": 500, "pass": cold_ms <= 500.0, "page_cache_evicted": evicted, "files_evicted": files.len()}));
            summary.insert(
                "cold_spawn_to_first_frame_ms".into(),
                json!(c.driver.get("spawn_to_first_frame_ms")),
            );
        }
        summary.insert(
            "warm_start_first_useful_paint_ms_median".into(),
            json!(median(&mut warm)),
        );
        summary.insert(
            "warm_start_first_frame_ms_median".into(),
            json!(median(&mut warm_first_frame)),
        );
    }
    if want("f1") {
        if let Ok(r) = run_app(&exe, &stack, &stack_file, "f1", &out, None) {
            let v = save(&out, "f1", &r);
            let p95 = v["key_to_frame_presented_ms"]["p95"]
                .as_f64()
                .unwrap_or(f64::NAN);
            gates.insert("f1_echo_presented_p95_ms".into(), json!({"value": p95, "limit": 16, "pass": p95 <= 16.0, "p50": v["key_to_frame_presented_ms"]["p50"], "painted_p95": v["key_to_frame_painted_ms"]["p95"], "samples": v["samples"]}));
        }
    }
    for f in ["f2", "f3"] {
        if want(f) {
            if let Ok(r) = run_app(&exe, &stack, &stack_file, f, &out, None) {
                let v = save(&out, f, &r);
                let rec = &v["record"];
                let fps = if f == "f2" {
                    rec["per_pane"][0]["frames"]["fps"].as_f64()
                } else {
                    rec["window_frames"]["fps"].as_f64()
                }
                .unwrap_or(f64::NAN);
                let stalls = rec["main_thread_heartbeat"]["stalls_over_50ms"]
                    .as_u64()
                    .unwrap_or(u64::MAX);
                let gaps = rec["window_frames"]["gaps_over_50ms"]
                    .as_u64()
                    .unwrap_or(u64::MAX);
                gates.insert(
                    format!("{f}_fps_and_stalls"),
                    json!({"fps": fps, "fps_limit": 60, "main_thread_stalls_over_50ms": stalls, "frame_gaps_over_50ms": gaps,
                           "pass": fps >= 58.0 && stalls == 0 && gaps == 0,
                           "received_mb_per_s": rec["per_pane"].as_array().map(|a| a.iter().map(|p| p["received_mb_per_s"].clone()).collect::<Vec<_>>()),
                           "window_active_fraction": rec["window_active_fraction"]}),
                );
            }
        }
    }
    if want("switch") {
        if let Ok(r) = run_app(&exe, &stack, &stack_file, "switch", &out, None) {
            let v = save(&out, "switch", &r);
            let open = v["open_terminal_to_snapshot_painted_ms"]["p95"]
                .as_f64()
                .unwrap_or(f64::NAN);
            let front = v["switch_open_terminal_to_front_presented_ms"]["p95"]
                .as_f64()
                .unwrap_or(f64::NAN);
            let agent = v["switch_selected_agent_presented_ms"]["p95"]
                .as_f64()
                .unwrap_or(f64::NAN);
            gates.insert(
                "switch_to_terminal_snapshot_p95_ms".into(),
                json!({"value": open, "limit": 150, "pass": open <= 150.0}),
            );
            gates.insert(
                "switch_tab_p95_ms".into(),
                json!({"value": front, "limit": 50, "pass": front <= 50.0}),
            );
            gates.insert(
                "switch_agent_p95_ms".into(),
                json!({"value": agent, "limit": 50, "pass": agent <= 50.0}),
            );
        }
    }
    if want("f4") {
        if let Ok(r) = run_app(&exe, &stack, &stack_file, "f4", &out, None) {
            save(&out, "f4", &r);
            let rss = r
                .driver
                .get("rss_bytes")
                .and_then(Value::as_f64)
                .unwrap_or(f64::NAN)
                / 1e6;
            let cpu = r
                .driver
                .get("idle_cpu_percent_of_one_core")
                .and_then(Value::as_f64)
                .unwrap_or(f64::NAN);
            gates.insert("f4_resident_mb".into(), json!({"value": rss, "limit": 200, "pass": rss <= 200.0, "tauri_same_machine": "measure with `agora-native-spike measure --pid <tauri pid> --tree` (see howto)"}));
            gates.insert(
                "idle_cpu_percent".into(),
                json!({"value": cpu, "limit": 1, "pass": cpu <= 1.0}),
            );
        }
    }
    if want("a11y") {
        // F5: on Linux the AT-SPI tree is read (and the button pressed) by scripts/atspi_check.py while the window holds. Run the
        // harness inside `dbus-run-session` with AGORA_SPIKE_PRIVATE_BUS=1 so the accessibility bus is switched on only there.
        let mut atspi: Option<Value> = None;
        let script = root
            .as_ref()
            .map(|r| r.join("apps/desktop-native-spike/scripts/atspi_check.py"));
        let private = std::env::var("AGORA_SPIKE_PRIVATE_BUS").is_ok_and(|v| v == "1");
        if std::env::consts::OS == "linux" && private {
            let _ = Command::new("gdbus")
                .args([
                    "call",
                    "--session",
                    "--dest",
                    "org.a11y.Bus",
                    "--object-path",
                    "/org/a11y/bus",
                    "--method",
                    "org.freedesktop.DBus.Properties.Set",
                    "org.a11y.Status",
                    "IsEnabled",
                    "<true>",
                ])
                .output();
        }
        let mut hook = |pid: u32| {
            if atspi.is_some() || std::env::consts::OS != "linux" {
                return;
            }
            std::thread::sleep(Duration::from_secs(2));
            let out = script.as_ref().and_then(|s| {
                Command::new("python3")
                    .arg("-I")
                    .arg(s)
                    .arg(pid.to_string())
                    .output()
                    .ok()
            });
            atspi = Some(out.and_then(|o| serde_json::from_slice(&o.stdout).ok()).unwrap_or(json!({"error": "atspi_check.py gave no result (python3-gi with Atspi, a running AT-SPI bus)"})));
        };
        if let Ok(mut r) = run_app_hold(&exe, &stack, &stack_file, &out, &mut hook) {
            r.driver
                .insert("atspi".into(), atspi.clone().unwrap_or(json!(null)));
            let v = save(&out, "a11y", &r);
            let a = &v["driver"]["atspi"];
            let items = a["list_items"].as_array().map_or(0, Vec::len);
            let button = a["buttons"]
                .as_array()
                .is_some_and(|b| b.iter().any(|n| n == "Open terminal"));
            let pressed = v["terminals_open_after"].as_u64() > v["terminals_open_before"].as_u64();
            gates.insert(
                "f5_accessibility_tree".into(),
                json!({"list_items_exposed": items, "button_exposed_with_name": button, "button_pressed_through_atspi_opened_terminal": pressed,
                       "pass_tree": items > 0 && button, "screen_reader_announcement": "not verified by the harness: listen with Orca / VoiceOver / Narrator (howto)"}),
            );
        }
    }
    if want("selftest") {
        if let Ok(r) = run_app(&exe, &stack, &stack_file, "selftest", &out, None) {
            let v = save(&out, "selftest", &r);
            summary.insert(
                "selftest".into(),
                json!({"checks": v["checks"], "passed": v["passed"]}),
            );
        }
    }
    let size = std::fs::metadata(&exe).map(|m| m.len()).unwrap_or(0);
    gates.insert("binary_mb".into(), json!({"value": size as f64 / 1e6, "limit": 40, "pass": size as f64 / 1e6 <= 40.0, "note": "this file, as built (release profile strips symbols)"}));
    summary.insert("gates".into(), Value::Object(gates));
    summary.insert("machine".into(), os::machine());
    summary.insert("label".into(), json!(o.label));
    summary.insert(
        "representative".into(),
        json!(o.not_representative.is_none()),
    );
    summary.insert(
        "not_representative_because".into(),
        json!(o.not_representative),
    );
    summary.insert("gpui".into(), json!(crate::GPUI_SOURCE));
    summary.insert("finished_epoch_ms".into(), json!(epoch_ms() as u64));
    let _ = std::fs::write(
        out.join("summary.json"),
        serde_json::to_vec_pretty(&Value::Object(summary.clone())).unwrap_or_default(),
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&Value::Object(summary)).unwrap_or_default()
    );
    if o.keep {
        eprintln!("kept {}", stack.work.display());
        std::mem::forget(stack);
    } else {
        let work = stack.work.clone();
        drop(stack);
        std::thread::sleep(Duration::from_millis(500));
        let _ = std::fs::remove_dir_all(work);
    }
    0
}

/// `agora-native-spike measure --pid N [--tree] [--secs S]`: resident memory and CPU of any process (the Tauri app for F4's comparison).
pub fn measure(args: &[String]) -> i32 {
    let mut pid = None;
    let mut tree = false;
    let mut secs = 10u64;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--pid" => pid = it.next().and_then(|s| s.parse::<u32>().ok()),
            "--tree" => tree = true,
            "--secs" => secs = it.next().and_then(|s| s.parse().ok()).unwrap_or(10),
            _ => {}
        }
    }
    let Some(pid) = pid else {
        eprintln!("--pid N");
        return 2;
    };
    let c0 = os::cpu_ms(pid);
    std::thread::sleep(Duration::from_secs(secs));
    let c1 = os::cpu_ms(pid);
    let v = json!({
        "pid": pid,
        "rss_bytes": os::rss(pid),
        "tree_rss_bytes": tree.then(|| os::tree_rss(pid)),
        "cpu_percent_of_one_core_main_process": c0.zip(c1).map(|(a, b)| (b - a) / (secs as f64 * 1000.0) * 100.0),
        "machine": os::machine(),
    });
    println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
    0
}

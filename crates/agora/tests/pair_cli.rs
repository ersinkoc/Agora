//! Pairing from the command line (ADR-0065 step 3), with real `agora` processes: a hub, the owner's `agora pair open` screen, and a new
//! machine's `agora join`. The number on the machine is compared with the number on the owner's screen, the owner answers, and the
//! machine ends up enrolled and connected. The refusal paths store nothing, and the number is found in exactly one place.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use agora_hub::projection::View;
use agora_hub::store::SqliteStore;

const AGORA: &str = env!("CARGO_BIN_EXE_agora");

struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn agora(home: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(AGORA);
    c.args(args).env("AGORA_HOME", home).env("AGORA_LANG", "en");
    c
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("agora-paircli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A process whose output lines are read as they come.
struct Live {
    child: Proc,
    lines: mpsc::Receiver<String>,
    seen: Vec<String>,
    stdin: Option<ChildStdin>,
}

impl Live {
    fn start(mut c: Command) -> Self {
        let mut child = c
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let (tx, lines) = mpsc::channel();
        for pipe in [
            Box::new(child.stdout.take().unwrap()) as Box<dyn std::io::Read + Send>,
            Box::new(child.stderr.take().unwrap()),
        ] {
            let tx = tx.clone();
            std::thread::spawn(move || {
                for l in BufReader::new(pipe).lines().map_while(Result::ok) {
                    if tx.send(l).is_err() {
                        break;
                    }
                }
            });
        }
        let stdin = child.stdin.take();
        Self {
            child: Proc(child),
            lines,
            seen: Vec::new(),
            stdin,
        }
    }

    /// The first line (from now on) that contains `needle`.
    fn wait_for(&mut self, needle: &str) -> String {
        let end = Instant::now() + Duration::from_secs(30);
        loop {
            let left = end.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(l) => {
                    self.seen.push(l.clone());
                    if l.contains(needle) {
                        return l;
                    }
                }
                Err(_) => panic!("never saw {needle:?} in {:#?}", self.seen),
            }
        }
    }

    /// Waits for the process to end and returns its status and everything it printed.
    fn finish(mut self) -> (bool, String) {
        self.stdin = None;
        let end = Instant::now() + Duration::from_secs(30);
        let ok = loop {
            if let Some(s) = self.child.0.try_wait().unwrap() {
                break s.success();
            }
            assert!(Instant::now() < end, "did not end: {:#?}", self.seen);
            std::thread::sleep(Duration::from_millis(50));
        };
        while let Ok(l) = self.lines.recv_timeout(Duration::from_millis(300)) {
            self.seen.push(l);
        }
        (ok, self.seen.join("\n"))
    }
}

/// The owner's screen, on a pseudo-terminal: `agora pair open` reads its answer from a terminal only, so this is the one honest way to
/// answer it. Output is searched as text (a Windows console adds control sequences around it, never inside a word).
struct Screen {
    pty: Box<dyn agora_platform::Pty>,
    seen: String,
    from: usize,
}

impl Screen {
    fn start(home: &Path, args: &[String]) -> Self {
        let mut argv = vec![AGORA.to_string(), "pair".to_string()];
        argv.extend(args.iter().cloned());
        let pty = agora_platform::spawn(&agora_platform::PtyConfig {
            argv,
            cwd: None,
            env: vec![
                ("AGORA_HOME".into(), home.display().to_string()),
                ("AGORA_LANG".into(), "en".into()),
            ],
            cols: 200,
            rows: 40,
            limits: agora_platform::ProcessLimits::default(),
            sandbox: None,
            identity: None,
            env_remove: Vec::new(),
        })
        .unwrap();
        Self {
            pty,
            seen: String::new(),
            from: 0,
        }
    }

    fn pump(&mut self) {
        if let Ok(b) = self.pty.output().recv_timeout(Duration::from_millis(100)) {
            self.seen.push_str(&String::from_utf8_lossy(&b));
        }
    }

    /// Waits for `needle` in what was printed since the last wait, and moves past it. Returns what was printed up to and including it.
    fn wait_for(&mut self, needle: &str) -> String {
        let end = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(i) = self.seen[self.from..].find(needle) {
                let upto = self.from + i + needle.len();
                let got = self.seen[self.from..upto].to_string();
                self.from = upto;
                return got;
            }
            assert!(
                Instant::now() < end,
                "never saw {needle:?} in {:?}",
                &self.seen[self.from..]
            );
            self.pump();
        }
    }

    /// The number on the card, `482-193`.
    fn card_number(&mut self) -> String {
        self.wait_for(". Number ");
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            let rest = &self.seen[self.from..];
            let n: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '-')
                .collect();
            if n.len() == 7 {
                return n;
            }
            assert!(Instant::now() < end, "no number on the card: {rest:?}");
            self.pump();
        }
    }

    /// Types `keys` after the question was shown and the owner had time to read it (longer than the screen's minimum wait).
    fn answer(&mut self, keys: &str) {
        self.wait_for("[y/N]");
        std::thread::sleep(Duration::from_millis(1500));
        self.pty.write_input(keys.as_bytes()).unwrap();
    }

    fn finish(mut self) -> (bool, String) {
        let end = Instant::now() + Duration::from_secs(30);
        let ok = loop {
            if let Some(s) = self.pty.try_wait().unwrap() {
                break s.success();
            }
            assert!(Instant::now() < end, "did not end: {}", self.seen);
            self.pump();
        };
        while let Ok(b) = self.pty.output().recv_timeout(Duration::from_millis(300)) {
            self.seen.push_str(&String::from_utf8_lossy(&b));
        }
        (ok, std::mem::take(&mut self.seen))
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = self.pty.kill();
    }
}

struct Hub {
    _proc: Proc,
    _out: BufReader<std::process::ChildStdout>,
    dir: PathBuf,
    port: String,
}

fn hub(root: &Path) -> Hub {
    let (dir, home) = (root.join("hub"), root.join("hub-home"));
    std::fs::create_dir_all(&home).unwrap();
    let mut child = agora(
        &home,
        &[
            "hub",
            "run",
            "--listen",
            "127.0.0.1:0",
            "--name",
            "localhost",
            "--hub-dir",
        ],
    )
    .arg(&dir)
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .spawn()
    .unwrap();
    let mut line = String::new();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    out.read_line(&mut line).unwrap();
    let proc = Proc(child);
    let addr = line
        .split("running on ")
        .nth(1)
        .and_then(|r| r.split(". Its").next())
        .expect(&line)
        .trim()
        .to_string();
    Hub {
        _proc: proc,
        _out: out,
        dir,
        port: addr.rsplit(':').next().unwrap().to_string(),
    }
}

impl Hub {
    fn pair(&self, home: &Path, args: &[&str]) -> Command {
        let mut c = agora(home, &["pair"]);
        c.args(args).arg("--hub-dir").arg(&self.dir);
        c
    }

    /// The owner's screen, open and waiting for a machine.
    fn open(&self, home: &Path) -> Screen {
        let mut o = Screen::start(
            home,
            &[
                "open".to_string(),
                "--hub-dir".to_string(),
                self.dir.display().to_string(),
            ],
        );
        o.wait_for("Waiting for a machine");
        o
    }
}

fn join(home: &Path, address: &str, extra: &[&str]) -> Live {
    let mut c = agora(home, &["join", address]);
    c.args(extra);
    Live::start(c)
}

/// `482-193` out of a line that says `Number 482-193.` or `Number 482-193. Approve...`.
fn number_in(line: &str) -> String {
    let after = line.split("Number ").nth(1).expect(line);
    after
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '-')
        .collect()
}

fn nothing_stored(home: &Path) {
    let link = home.join("link");
    for f in ["node-id", "key.pem", "cert.pem", "ca.pem", "url"] {
        assert!(!link.join(f).exists(), "{f} was stored in {link:?}");
    }
}

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                files_under(&p, out);
            } else {
                out.push(p);
            }
        }
    }
}

#[test]
fn the_owner_opens_a_machine_joins_the_numbers_match_and_the_node_connects() {
    let root = tmp("flow");
    let hub = hub(&root);
    let owner_home = root.join("owner");
    std::fs::create_dir_all(&owner_home).unwrap();

    // The fingerprint of the hub's CA, short form: what `agora pair open` and `agora pair status` print on the hub's screen, and what a
    // machine that finds the hub by itself shows its person to compare.
    let fingerprint = agora_hub::discovery::short_fingerprint(
        &agora_hub::tls::leaf_hash(&std::fs::read_to_string(hub.dir.join("ca.pem")).unwrap())
            .unwrap(),
    );
    assert_eq!(fingerprint.split(' ').count(), 6);

    // Closed by default, and the status says how to open it.
    let out = hub.pair(&owner_home, &["status"]).output().unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains(&fingerprint),
        "status does not print the fingerprint {fingerprint}: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("Closed to new machines"),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );

    // A closed hub refuses the machine, with the hub's own sentence, and nothing is stored.
    let machine = root.join("machine");
    let (ok, text) = join(&machine, &format!("localhost:{}", hub.port), &[]).finish();
    assert!(!ok && text.contains("not open for new machines"), "{text}");
    nothing_stored(&machine);

    // No address: the machine looks for a hub that is open and, finding none (this hub is closed, and listens on loopback only), says
    // how to get one. Nothing is stored. (A found hub is tested in `pair_node.rs` and `discovery.rs`; the fixed discovery port keeps it out of real-process tests.)
    let out = agora(&machine, &["join"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let text =
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert!(
        text.contains("agora pair open") && text.contains("agora join 192.168"),
        "{text}"
    );
    nothing_stored(&machine);

    // The owner opens it; the screen says what to do next.
    let mut owner = hub.open(&owner_home);
    assert!(owner.seen.contains("agora join "), "{}", owner.seen);
    assert!(
        owner.seen.contains(&fingerprint),
        "open does not print the fingerprint {fingerprint}: {}",
        owner.seen
    );
    let status = hub.pair(&owner_home, &["status"]).output().unwrap();
    assert!(
        String::from_utf8_lossy(&status.stdout).contains("Open for new machines"),
        "{}",
        String::from_utf8_lossy(&status.stdout)
    );

    // The machine asks; both screens show the same number.
    let mut m = join(
        &machine,
        &format!("localhost:{}", hub.port),
        &["--id", "laptop"],
    );
    let on_machine = number_in(&m.wait_for("Number "));
    let card = owner.wait_for("wants to join");
    assert!(card.contains("\"laptop\""), "{card}");
    assert_eq!(owner.card_number(), on_machine);
    assert_eq!(on_machine.len(), 7, "{on_machine}");

    // The owner compares and approves, at the keyboard of a terminal, after reading the question.
    owner.answer("y\r");
    let (ok, joined) = m.finish();
    assert!(ok, "{joined}");
    assert!(
        joined.contains("Joined. This machine is \"laptop\""),
        "{joined}"
    );
    assert!(joined.contains("fingerprint"), "{joined}");
    let (ok, screen) = owner.finish();
    assert!(ok && screen.contains("Approved"), "{screen}");
    // One machine per opening: it closed itself.
    assert!(screen.contains("closed to new machines again"), "{screen}");
    let status = hub.pair(&owner_home, &["status"]).output().unwrap();
    assert!(String::from_utf8_lossy(&status.stdout).contains("Closed"));

    // Stored like `agora node enroll` stores it.
    let link = machine.join("link");
    assert_eq!(
        std::fs::read_to_string(link.join("node-id")).unwrap(),
        "laptop"
    );
    assert_eq!(
        std::fs::read_to_string(link.join("url")).unwrap(),
        format!("wss://localhost:{}", hub.port)
    );
    assert_eq!(
        std::fs::read_to_string(link.join("ca.pem")).unwrap(),
        std::fs::read_to_string(hub.dir.join("ca.pem")).unwrap()
    );
    let key = std::fs::read_to_string(link.join("key.pem")).unwrap();
    assert!(key.contains("PRIVATE KEY"));
    assert!(std::fs::read_to_string(link.join("cert.pem"))
        .unwrap()
        .contains("BEGIN CERTIFICATE"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(link.join("key.pem"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "the key is readable by others: {mode:o}");
    }
    // `agora` will start the node here: the settings of a joined machine are in place.
    let settings = std::fs::read_to_string(machine.join("machine.toml")).unwrap();
    assert!(settings.contains("roles = [\"node\"]"), "{settings}");

    // The private key never reached the hub: the hub's files do not hold a piece of it.
    let body: String = key.lines().filter(|l| !l.starts_with("-----")).collect();
    // Not the start: every P-256 key begins with the same fixed bytes. This is the part that is the key.
    let piece = &body[60..100];
    let mut hub_files = Vec::new();
    files_under(&hub.dir, &mut hub_files);
    for f in hub_files {
        let bytes = std::fs::read(&f).unwrap_or_default();
        assert!(
            !bytes.windows(piece.len()).any(|w| w == piece.as_bytes()),
            "the machine's private key is in {f:?}"
        );
    }

    // Joining again is refused unless asked for.
    let out = agora(&machine, &["join", &format!("localhost:{}", hub.port)])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("already joined"));

    // The node starts and connects with what it was given.
    if agora_platform::LOCAL_SOCKETS_SUPPORTED && agora_platform::PTY_SUPPORTED {
        let logs = root.join("node-logs");
        std::fs::create_dir_all(&logs).unwrap();
        let node = agora(&machine, &["node", "run"])
            .stdout(std::fs::File::create(logs.join("out.log")).unwrap())
            .stderr(std::fs::File::create(logs.join("err.log")).unwrap())
            .spawn()
            .unwrap();
        let _node = Proc(node);
        let end = Instant::now() + Duration::from_secs(30);
        loop {
            let store = SqliteStore::open(&hub.dir.join("hub.db")).unwrap();
            let (view, _) = View::rebuild(&store).unwrap();
            if view.nodes.get("laptop").is_some_and(|n| n.online) {
                break;
            }
            assert!(
                Instant::now() < end,
                "the joined node never connected: {view:?}"
            );
            std::thread::sleep(Duration::from_millis(200));
        }
        // The number was shown once, to the person, and is in no log and no file of the machine.
        let digits = on_machine.replace('-', "");
        let mut files = Vec::new();
        files_under(&machine, &mut files);
        files_under(&logs, &mut files);
        for f in files {
            let bytes = std::fs::read(&f).unwrap_or_default();
            let has = |needle: &str| bytes.windows(needle.len()).any(|w| w == needle.as_bytes());
            assert!(!has(&on_machine), "the number is in {f:?}");
            // The digits alone are checked where the files are text (a database may hold any six digits by chance).
            if !f.to_string_lossy().ends_with(".db") && f.extension().is_none_or(|e| e != "wal") {
                let in_data = f.starts_with(machine.join("data"));
                assert!(in_data || !has(&digits), "the digits are in {f:?}");
            }
        }
    }
}

#[test]
fn the_machine_prints_the_number_once_and_only_on_standard_output() {
    let root = tmp("once");
    let hub = hub(&root);
    let (owner_home, machine) = (root.join("owner"), root.join("machine"));
    std::fs::create_dir_all(&owner_home).unwrap();
    let mut owner = hub.open(&owner_home);
    let mut m = join(&machine, &format!("localhost:{}", hub.port), &[]);
    let on_machine = number_in(&m.wait_for("Number "));
    owner.wait_for("wants to join");
    owner.answer("y\r");
    let (ok, all) = m.finish();
    assert!(ok, "{all}");
    assert_eq!(all.matches(&on_machine).count(), 1, "{all}");
    assert_eq!(
        all.matches(&on_machine.replace('-', "")).count(),
        0,
        "{all}"
    );
    let _ = owner.finish();
}

#[test]
fn a_denied_machine_stores_nothing_and_the_hubs_own_address_is_named_in_its_certificate() {
    let root = tmp("refuse");
    let hub = hub(&root);
    let owner_home = root.join("owner");
    std::fs::create_dir_all(&owner_home).unwrap();

    // The owner compares and says no.
    let denied = root.join("denied");
    let mut owner = hub.open(&owner_home);
    let mut m = join(&denied, &format!("localhost:{}", hub.port), &[]);
    m.wait_for("Number ");
    owner.wait_for("wants to join");
    owner.answer("n\r");
    let (ok, text) = m.finish();
    assert!(
        !ok && text.contains("The owner said no. Nothing was saved."),
        "{text}"
    );
    nothing_stored(&denied);
    drop(owner);

    // ADR-0065 section 5b: the hub listens on 127.0.0.1, so its certificate names that address, and a machine that dials the IP number
    // (not `localhost`) now completes (it used to fail closed: the leaf named `localhost` only). The address that is in no name fails
    // closed in `pair_node.rs`.
    let byip = root.join("byip");
    let mut owner = hub.open(&owner_home);
    let mut m = join(&byip, &format!("127.0.0.1:{}", hub.port), &[]);
    m.wait_for("Number ");
    owner.wait_for("wants to join");
    owner.answer("y\r");
    let (ok, text) = m.finish();
    assert!(ok, "{text}");
    assert!(byip.join("link").join("node-id").exists(), "{text}");
}

/// #943 (RM-07): a hub that listens on loopback only cannot be reached from another machine, so `agora pair open` says so and how to
/// make it listen on the network, and offers only an address the hub listens on and its certificate names (a second Agora on this
/// same computer). It used to say `agora join <this computer's name>:PORT`, which another machine cannot reach. The offered address
/// is then dialled and the join completes.
#[test]
fn a_loopback_only_hub_says_other_machines_cannot_reach_it_and_offers_only_this_computer() {
    let root = tmp("loopback");
    let hub = hub(&root);
    let owner_home = root.join("owner");
    std::fs::create_dir_all(&owner_home).unwrap();
    let mut owner = hub.open(&owner_home);
    let screen = owner.seen.clone();
    assert!(screen.contains("listens on this computer only"), "{screen}");
    // How to listen on the network: this computer's network address first, every IPv4 interface only as the fallback.
    assert!(screen.contains("`agora --listen "), "{screen}");
    assert!(
        screen.contains(&format!(
            "`0.0.0.0:{}` instead listens on every IPv4 interface",
            hub.port
        )),
        "{screen}"
    );
    let local = format!("agora join 127.0.0.1:{}", hub.port);
    assert!(screen.contains(&local), "{screen}");
    // Every `agora join` it suggests is that one: no host name another machine would be told to dial.
    assert_eq!(
        screen.matches("agora join ").count(),
        screen.matches(&local).count(),
        "{screen}"
    );
    if let Some(host) = agora_platform::hostname().filter(|h| !h.trim().is_empty()) {
        assert!(
            !screen.contains(&format!("agora join {}:", host.trim())),
            "{screen}"
        );
    }

    // The address it offered works on this computer.
    let second = root.join("second");
    let mut m = join(&second, &format!("127.0.0.1:{}", hub.port), &[]);
    m.wait_for("Number ");
    owner.wait_for("wants to join");
    owner.answer("y\r");
    let (ok, text) = m.finish();
    assert!(ok, "{text}");
}

/// F821: the owner's screen ends, with a failure and a sentence, when the hub it answers for goes away; it used to wait for ever
/// and spin on the closed connection.
#[test]
fn the_owners_screen_ends_when_the_hub_goes_away() {
    let root = tmp("hubgone");
    let hub = hub(&root);
    let owner = root.join("owner");
    std::fs::create_dir_all(&owner).unwrap();
    let screen = hub.open(&owner);
    drop(hub);
    let (ok, text) = screen.finish();
    assert!(!ok, "{text}");
    assert!(text.contains("The hub did not answer"), "{text}");
}

#[test]
fn wrong_arguments_are_refused_before_anything_happens() {
    let root = tmp("args");
    let hub = hub(&root);
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    for minutes in ["0", "11", "x"] {
        let out = hub
            .pair(&home, &["open", "--minutes", minutes])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{minutes}");
    }
    let out = hub
        .pair(&home, &["close", "--minutes", "3"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    // An address that is not one.
    let out = agora(&home, &["join", "not an address"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    // Nobody listens.
    let free = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let out = agora(&home, &["join", &format!("127.0.0.1:{free}")])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Could not reach the hub"));
    nothing_stored(&home);
    // A hub folder with no running hub says so.
    let out = agora(&home, &["pair", "status", "--hub-dir"])
        .arg(root.join("nothing-here"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("No hub is running"));
}

/// Feeds `y` lines into `c`'s standard input for as long as it listens: what a script (`yes | agora pair open`) does.
fn feed_yes(c: &mut Child) -> std::thread::JoinHandle<()> {
    let mut stdin = c.stdin.take().unwrap();
    std::thread::spawn(move || {
        while stdin.write_all(b"y\n").is_ok() && stdin.flush().is_ok() {
            std::thread::sleep(Duration::from_millis(20));
        }
    })
}

#[test]
fn a_script_feeding_yes_through_a_pipe_never_approves_a_machine() {
    let root = tmp("pipe");
    let hub = hub(&root);
    let (owner_home, machine) = (root.join("owner"), root.join("machine"));
    std::fs::create_dir_all(&owner_home).unwrap();

    // The feed against a closed hub: the command refuses to start, so nothing is opened and nothing can be approved.
    let mut feed = hub
        .pair(&owner_home, &["open"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let feeder = feed_yes(&mut feed);
    let out = feed.wait_with_output().unwrap();
    let _ = feeder.join();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("at a terminal"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let status = hub.pair(&owner_home, &["status"]).output().unwrap();
    assert!(String::from_utf8_lossy(&status.stdout).contains("Closed"));

    // The feed against a hub that is open (a person opened it at a terminal): the same refusal, and the machine stays pending until the
    // person answers.
    let mut screen = hub.open(&owner_home);
    let mut m = join(&machine, &format!("localhost:{}", hub.port), &[]);
    m.wait_for("Number ");
    screen.wait_for("wants to join");
    let mut feed = hub
        .pair(&owner_home, &["open"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let feeder = feed_yes(&mut feed);
    let out = feed.wait_with_output().unwrap();
    let _ = feeder.join();
    assert_eq!(out.status.code(), Some(2));
    // Several seconds with the feed gone: the machine has not been approved and nothing is stored.
    std::thread::sleep(Duration::from_secs(3));
    assert!(
        m.child.0.try_wait().unwrap().is_none(),
        "the machine was answered without a person"
    );
    nothing_stored(&machine);
    // The person at the terminal says no.
    screen.answer("n\r");
    let (ok, text) = m.finish();
    assert!(!ok && text.contains("The owner said no"), "{text}");
    nothing_stored(&machine);
}

#[test]
fn a_key_that_comes_too_soon_after_the_question_is_not_an_answer() {
    let root = tmp("soon");
    let hub = hub(&root);
    let (owner_home, machine) = (root.join("owner"), root.join("machine"));
    std::fs::create_dir_all(&owner_home).unwrap();
    let mut screen = hub.open(&owner_home);
    let mut m = join(&machine, &format!("localhost:{}", hub.port), &[]);
    m.wait_for("Number ");
    screen.wait_for("wants to join");
    screen.wait_for("[y/N]");
    // Past the point where what was typed before the question is thrown away, but well within the time a person needs to read it.
    std::thread::sleep(Duration::from_millis(150));
    screen.pty.write_input(b"y\r").unwrap();
    screen.wait_for("came too soon");
    // It was not taken as a yes: the person then answers no, and that is what the machine gets.
    screen.answer("n\r");
    let (ok, text) = m.finish();
    assert!(!ok && text.contains("The owner said no"), "{text}");
    nothing_stored(&machine);
}

/// An owner's window on the hub (the web window and `agora tui` use the same owner commands and cards).
fn window(hub: &Hub) -> agora_tui::client::Session {
    let token = std::fs::read_to_string(hub.dir.join("token")).unwrap();
    let addr = agora_tui::client::Hub {
        addr: format!("localhost:{}", hub.port),
        ca_pem: std::fs::read_to_string(hub.dir.join("ca.pem")).unwrap(),
    };
    agora_tui::client::Session::connect(&addr, token.trim()).unwrap()
}

/// The window's card for `name`.
fn window_card(
    w: &mut agora_tui::client::Session,
    name: &str,
) -> agora_protocol::v1::PairRequested {
    let end = Instant::now() + Duration::from_secs(30);
    while Instant::now() < end {
        let _ = w.next(Duration::from_millis(250));
        for n in w.take_pair_notes() {
            if let agora_tui::client::PairNote::Requested(c) = n {
                if c.name == name {
                    return c;
                }
            }
        }
    }
    panic!("no card for {name:?} in the window");
}

/// #942 (RM-07): the owner answers in a window while `agora pair open` is asking about the same machine. The terminal's question
/// ends and says it was answered in a window: a deny leaves the screen waiting for the next machine with the hub still open; an
/// approval closes the hub to new machines (one machine per opening, whoever approves) and the screen ends.
#[test]
fn an_answer_from_a_window_ends_the_question_at_the_terminal() {
    use agora_protocol::v1::{self, command_request::Command as Cmd};
    let root = tmp("window");
    let hub = hub(&root);
    let owner_home = root.join("owner");
    std::fs::create_dir_all(&owner_home).unwrap();
    let mut screen = hub.open(&owner_home);
    let mut w = window(&hub);
    let wait = Duration::from_secs(5);
    let open = |w: &mut agora_tui::client::Session| match w
        .command_info("", Cmd::PairStatus(v1::PairStatus {}), wait)
        .unwrap()
        .result
    {
        Some(v1::command_ok::Result::PairStatus(s)) => s.open,
        other => panic!("{other:?}"),
    };

    // A deny from the window: the terminal stops asking about that machine and waits for the next; the hub stays open.
    let first = root.join("first");
    let mut m = join(
        &first,
        &format!("localhost:{}", hub.port),
        &["--id", "rm-join2"],
    );
    m.wait_for("Number ");
    screen.wait_for("wants to join");
    screen.wait_for("[y/N]");
    let card = window_card(&mut w, "rm-join2");
    w.command_info(
        "",
        Cmd::PairDeny(v1::PairDeny {
            pair_id: card.pair_id,
        }),
        wait,
    )
    .unwrap();
    let (ok, text) = m.finish();
    assert!(!ok && text.contains("The owner said no"), "{text}");
    screen.wait_for("Answered in a window: \"rm-join2\" was denied");
    assert!(open(&mut w), "a deny closed the hub");

    // An approval from the window: the machine joins, the terminal's question ends, and the hub is closed to new machines again.
    let second = root.join("second");
    let mut m = join(
        &second,
        &format!("localhost:{}", hub.port),
        &["--id", "rm-join3"],
    );
    m.wait_for("Number ");
    screen.wait_for("wants to join");
    screen.wait_for("[y/N]");
    let card = window_card(&mut w, "rm-join3");
    w.command_info(
        "",
        Cmd::PairApprove(v1::PairApprove {
            pair_id: card.pair_id,
            replace: false,
        }),
        wait,
    )
    .unwrap();
    let (ok, joined) = m.finish();
    assert!(
        ok && joined.contains("Joined. This machine is \"rm-join3\""),
        "{joined}"
    );
    screen.wait_for("Answered in a window: \"rm-join3\" was approved");
    let (ok, text) = screen.finish();
    assert!(ok, "{text}");
    assert!(text.contains("closed to new machines again"), "{text}");
    assert!(
        !open(&mut w),
        "the hub stayed open after the window approved"
    );
    let status = hub.pair(&owner_home, &["status"]).output().unwrap();
    assert!(
        String::from_utf8_lossy(&status.stdout).contains("Closed"),
        "{}",
        String::from_utf8_lossy(&status.stdout)
    );
}

/// The owner's screen asking about machine "rm-a" while machine "rm-b" waits its turn; the window holds both cards.
struct TwoWaiting {
    _hub: Hub,
    screen: Screen,
    _a: Live,
    b: Live,
    b_home: PathBuf,
    window: agora_tui::client::Session,
    card_a: agora_protocol::v1::PairRequested,
}

fn two_waiting(root: &Path) -> TwoWaiting {
    let hub = hub(root);
    let owner_home = root.join("owner");
    std::fs::create_dir_all(&owner_home).unwrap();
    let mut screen = hub.open(&owner_home);
    let mut w = window(&hub);
    let addr = format!("localhost:{}", hub.port);
    let mut a = join(&root.join("a"), &addr, &["--id", "rm-a"]);
    a.wait_for("Number ");
    screen.wait_for("\"rm-a\"");
    screen.wait_for("[y/N]");
    let b_home = root.join("b");
    let mut b = join(&b_home, &addr, &["--id", "rm-b"]);
    b.wait_for("Number ");
    let card_a = window_card(&mut w, "rm-a");
    window_card(&mut w, "rm-b");
    TwoWaiting {
        _hub: hub,
        screen,
        _a: a,
        b,
        b_home,
        window: w,
        card_a,
    }
}

/// #942 Codex review: an `agora pair open` that falls behind (paused here) while the owner approves its machine in a window and then
/// opens a new round from that window must not close the new round when it catches up: it closes only the opening it made itself, and
/// on a hub that closed itself at the approval it closes nothing. It says the hub was opened again elsewhere and leaves it so.
#[cfg(unix)]
#[test]
fn a_terminal_that_falls_behind_never_closes_a_round_opened_again_in_a_window() {
    use agora_protocol::v1::{self, command_request::Command as Cmd};
    let root = tmp("behind");
    let TwoWaiting {
        _hub,
        screen,
        _a,
        b: _b,
        window: mut w,
        card_a,
        ..
    } = two_waiting(&root);
    let wait = Duration::from_secs(5);
    let pid = screen.pty.pid().to_string();
    let signal = |sig: &str| {
        assert!(Command::new("kill")
            .args([sig, pid.as_str()])
            .status()
            .unwrap()
            .success());
    };
    let status = |w: &mut agora_tui::client::Session| match w
        .command_info("", Cmd::PairStatus(v1::PairStatus {}), wait)
        .unwrap()
        .result
    {
        Some(v1::command_ok::Result::PairStatus(s)) => s,
        other => panic!("{other:?}"),
    };
    let first = status(&mut w);
    assert!(first.open);
    // The terminal stops while it asks about "rm-a"; meanwhile the window approves "rm-a" and opens a new round.
    signal("-STOP");
    w.command_info(
        "",
        Cmd::PairApprove(v1::PairApprove {
            pair_id: card_a.pair_id.clone(),
            replace: false,
        }),
        wait,
    )
    .unwrap();
    assert!(!status(&mut w).open, "the approval did not close the hub");
    std::thread::sleep(Duration::from_millis(20));
    w.command_info("", Cmd::PairOpen(v1::PairOpen {}), wait)
        .unwrap();
    let round = status(&mut w);
    assert!(round.open && round.closes_at_ms != first.closes_at_ms);
    signal("-CONT");
    let (ok, text) = screen.finish();
    assert!(ok, "{text}");
    assert!(
        text.contains("Answered in a window: \"rm-a\" was approved"),
        "{text}"
    );
    let after = status(&mut w);
    assert!(
        after.open && after.closes_at_ms == round.closes_at_ms,
        "the terminal closed the round the window opened: {after:?}"
    );
    assert!(
        text.contains("opened for new machines again somewhere else"),
        "{text}"
    );
    assert!(!text.contains("closed to new machines again"), "{text}");
}

fn deny_in_window(w: &mut agora_tui::client::Session, card: &agora_protocol::v1::PairRequested) {
    use agora_protocol::v1::{self, command_request::Command as Cmd};
    w.command_info(
        "",
        Cmd::PairDeny(v1::PairDeny {
            pair_id: card.pair_id.clone(),
        }),
        Duration::from_secs(5),
    )
    .unwrap();
}

/// #942 review: when the question moves to another machine without a keystroke (the one asked about was answered in a window), a
/// `y` meant for the first one never approves the next. The next card is set apart by a line and shows its own name and number; keys
/// typed just before the move and within two seconds after it are thrown away (the wait starts again, and nothing is denied).
#[test]
fn a_key_meant_for_a_request_answered_in_a_window_never_approves_the_next_machine() {
    let root = tmp("moved");
    let TwoWaiting {
        _hub,
        mut screen,
        _a,
        mut b,
        b_home,
        window: mut w,
        card_a,
    } = two_waiting(&root);
    // The owner had read "rm-a" and is typing yes when the window denies it: the key lands on "rm-a" or on the next question.
    std::thread::sleep(Duration::from_millis(1500));
    deny_in_window(&mut w, &card_a);
    screen.pty.write_input(b"y\r").unwrap();
    // The next question is set apart and names the next machine with its number.
    screen.wait_for("A different machine is asking now");
    let card = screen.wait_for("[y/N]");
    assert!(
        card.contains("\"rm-b\"") && card.contains(". Number "),
        "{card}"
    );
    // A yes typed soon after it appeared (later than the 700 ms an ordinary question asks for, sooner than two seconds) is thrown
    // away, again and again.
    for _ in 0..2 {
        std::thread::sleep(Duration::from_millis(1200));
        screen.pty.write_input(b"y\r").unwrap();
        screen.wait_for("[y/N]");
    }
    assert!(
        screen
            .seen
            .contains("That key came before you could read the new request"),
        "{}",
        screen.seen
    );
    std::thread::sleep(Duration::from_secs(3));
    assert!(
        b.child.0.try_wait().unwrap().is_none(),
        "the next machine was answered by a key meant for the one before: {:#?}",
        b.seen
    );
    assert!(!screen.seen.contains("Approved."), "{}", screen.seen);
    nothing_stored(&b_home);
}

/// #942 review: after the longer wait, a yes typed for the next machine approves it, and the hub closes as after any approval.
#[test]
fn after_the_wait_a_yes_approves_the_machine_the_question_moved_to() {
    let root = tmp("movedyes");
    let TwoWaiting {
        _hub,
        mut screen,
        _a,
        b,
        window: mut w,
        card_a,
        ..
    } = two_waiting(&root);
    deny_in_window(&mut w, &card_a);
    screen.wait_for("Answered in a window: \"rm-a\" was denied");
    screen.wait_for("A different machine is asking now");
    let card = screen.wait_for("[y/N]");
    assert!(card.contains("\"rm-b\""), "{card}");
    std::thread::sleep(Duration::from_millis(2500));
    screen.pty.write_input(b"y\r").unwrap();
    let (ok, joined) = b.finish();
    assert!(
        ok && joined.contains("Joined. This machine is \"rm-b\""),
        "{joined}"
    );
    let (ok, text) = screen.finish();
    assert!(
        ok && text.contains("Approved. \"rm-b\" is joining"),
        "{text}"
    );
    assert!(text.contains("closed to new machines again"), "{text}");
}

/// Audit F890: `agora join` looks at "already joined" before it waits for the owner, and again before it stores: a link this machine
/// got meanwhile (`agora` enrolling with its own hub, `agora node enroll`, another join) is kept unless `--replace` was given. Each
/// join has a hub of its own (a hub limits pairing attempts per address per minute); the link is planted only after the number is
/// shown, and the owner approves after that.
#[test]
fn a_link_made_while_the_join_waited_is_kept_unless_the_person_asked_to_replace_it() {
    let root = tmp("waited");
    let joined_meanwhile = |name: &str, extra: &[&str], same_hub: bool| {
        let hub = hub(&root.join(format!("hub-{name}")));
        let owner_home = root.join(format!("owner-{name}"));
        std::fs::create_dir_all(&owner_home).unwrap();
        let machine = root.join(name);
        let mut owner = hub.open(&owner_home);
        let mut args = vec!["--id", name];
        args.extend_from_slice(extra);
        let mut m = join(&machine, &format!("localhost:{}", hub.port), &args);
        m.wait_for("Number ");
        let link = machine.join("link");
        std::fs::create_dir_all(&link).unwrap();
        let hub_ca = std::fs::read_to_string(hub.dir.join("ca.pem")).unwrap();
        let (ca, id) = if same_hub {
            (hub_ca.as_str(), name)
        } else {
            ("FIRSTCA", "first")
        };
        for (f, c) in [
            ("key.pem", "FIRSTKEY"),
            ("ca.pem", ca),
            ("cert.pem", "FIRSTCERT"),
            ("url", "wss://first:1"),
            ("node-id", id),
        ] {
            std::fs::write(link.join(f), c).unwrap();
        }
        owner.wait_for("wants to join");
        owner.answer("y\r");
        let (ok, text) = m.finish();
        let read = |f: &str| std::fs::read_to_string(link.join(f)).unwrap_or_default();
        (ok, read("node-id"), read("key.pem"), text)
    };
    let (ok, id, key, text) = joined_meanwhile("kept", &[], false);
    assert!(!ok && text.contains("already joined a hub"), "{text}");
    assert_eq!((id.as_str(), key.as_str()), ("first", "FIRSTKEY"), "{text}");
    let (ok, id, key, text) = joined_meanwhile("replaced", &["--replace"], false);
    assert!(ok, "{text}");
    assert_eq!(id, "replaced", "{text}");
    assert!(key.contains("PRIVATE KEY"), "{text}");
    // The same hub under the same name (Codex review): the new certificate supersedes that link, so it is stored.
    let (ok, id, key, text) = joined_meanwhile("same", &[], true);
    assert!(ok, "{text}");
    assert_eq!(id, "same", "{text}");
    assert!(key.contains("PRIVATE KEY"), "{text}");
}

/// Audit F1032: the join's second look at "already joined" used to come before it took the link folder's guard, so a save that was
/// under way then (another `agora` enrolling with its own hub, `agora node enroll`) passed it with `node-id` removed; the join waited for
/// the guard and then overwrote the link that save had just completed, without `--replace`. The order is forced: the test is that
/// other save. It holds `link/lock` before the owner approves, waits until the join process has the lock file open (it reached the
/// guard), then completes its link and lets go: the join must see it and keep it. Where the system shows no open files of a process
/// (no `/proc`), the order cannot be forced and the test has nothing to check.
#[test]
fn a_link_saved_while_the_join_waited_for_the_folder_is_kept() {
    let open_files = |pid: u32| PathBuf::from(format!("/proc/{pid}/fd"));
    if !open_files(std::process::id()).is_dir() {
        return;
    }
    let root = tmp("guarded");
    let hub = hub(&root.join("hub"));
    let owner_home = root.join("owner");
    std::fs::create_dir_all(&owner_home).unwrap();
    let machine = root.join("racer");
    let mut owner = hub.open(&owner_home);
    let mut m = join(
        &machine,
        &format!("localhost:{}", hub.port),
        &["--id", "racer"],
    );
    m.wait_for("Number ");
    let link = machine.join("link");
    std::fs::create_dir_all(&link).unwrap();
    let lock = link.join("lock");
    let guard = agora_platform::open_lock_private(&lock).unwrap();
    guard.lock().unwrap();
    let lock = std::fs::canonicalize(&lock).unwrap();
    owner.wait_for("wants to join");
    owner.answer("y\r");
    let pid = m.child.0.id();
    let reached_guard = || {
        std::fs::read_dir(open_files(pid)).is_ok_and(|d| {
            d.filter_map(Result::ok)
                .any(|e| std::fs::read_link(e.path()).is_ok_and(|p| p == lock))
        })
    };
    while !reached_guard() {
        assert!(
            m.child.0.try_wait().unwrap().is_none(),
            "the join ended before it reached the folder's guard"
        );
        std::thread::yield_now();
    }
    for (f, c) in [
        ("key.pem", "FIRSTKEY"),
        ("ca.pem", "FIRSTCA"),
        ("cert.pem", "FIRSTCERT"),
        ("url", "wss://first:1"),
        ("node-id", "first"),
    ] {
        std::fs::write(link.join(f), c).unwrap();
    }
    drop(guard);
    let (ok, text) = m.finish();
    let read = |f: &str| std::fs::read_to_string(link.join(f)).unwrap_or_default();
    assert!(!ok && text.contains("already joined a hub"), "{text}");
    assert_eq!(
        (read("node-id").as_str(), read("key.pem").as_str()),
        ("first", "FIRSTKEY"),
        "{text}"
    );
}

/// Codex review of F1030: an approved join gets its certificate when the owner approves, then waits for the link folder. If this
/// machine was enrolled with the same hub under the same name in that time (`agora node enroll`, or `agora` enrolling with its own hub),
/// the hub revoked the join's certificate when it issued the newer one, and the join, finding the same hub and name, saved its revoked
/// certificate over the newer one. The order is forced as for F1032: the test holds `link/lock` before the owner approves and waits
/// until the join reached the guard; then this machine is enrolled by name with the same hub (into a folder of its own, copied in as
/// that other save) and the guard is let go. The join must keep the newer certificate. Where the system shows no open files of a
/// process (no `/proc`), the order cannot be forced and the test has nothing to check.
#[test]
fn a_newer_certificate_of_the_same_hub_saved_while_the_join_waited_is_kept() {
    let open_files = |pid: u32| PathBuf::from(format!("/proc/{pid}/fd"));
    if !open_files(std::process::id()).is_dir() {
        return;
    }
    let root = tmp("newer");
    let hub = hub(&root.join("hub"));
    let owner_home = root.join("owner");
    std::fs::create_dir_all(&owner_home).unwrap();
    let machine = root.join("racer");
    let mut owner = hub.open(&owner_home);
    let mut m = join(
        &machine,
        &format!("localhost:{}", hub.port),
        &["--id", "racer"],
    );
    m.wait_for("Number ");
    let link = machine.join("link");
    std::fs::create_dir_all(&link).unwrap();
    let lock = link.join("lock");
    let guard = agora_platform::open_lock_private(&lock).unwrap();
    guard.lock().unwrap();
    let lock = std::fs::canonicalize(&lock).unwrap();
    owner.wait_for("wants to join");
    owner.answer("y\r");
    let pid = m.child.0.id();
    let reached_guard = || {
        std::fs::read_dir(open_files(pid)).is_ok_and(|d| {
            d.filter_map(Result::ok)
                .any(|e| std::fs::read_link(e.path()).is_ok_and(|p| p == lock))
        })
    };
    while !reached_guard() {
        assert!(
            m.child.0.try_wait().unwrap().is_none(),
            "the join ended before it reached the folder's guard"
        );
        std::thread::yield_now();
    }
    // The join holds its certificate now. The same machine is enrolled again with the same hub under the same name.
    let other = root.join("enrolled");
    std::fs::create_dir_all(&other).unwrap();
    let out = agora(&other, &["hub", "code", "--hub-dir"])
        .arg(&hub.dir)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let code = text
        .split("Enrollment code: ")
        .nth(1)
        .and_then(|r| r.split(' ').next())
        .expect(&text)
        .to_string();
    let out = agora(
        &other,
        &[
            "node",
            "enroll",
            "--hub",
            &format!("wss://localhost:{}", hub.port),
            "--ca",
        ],
    )
    .arg(hub.dir.join("ca.pem"))
    .args(["--code", &code, "--id", "racer"])
    .stdin(Stdio::null())
    .output()
    .unwrap();
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let newer = other.join("link");
    for f in ["key.pem", "ca.pem", "cert.pem", "url", "node-id"] {
        std::fs::copy(newer.join(f), link.join(f)).unwrap();
    }
    drop(guard);
    let (ok, text) = m.finish();
    let read = |d: &Path, f: &str| std::fs::read_to_string(d.join(f)).unwrap_or_default();
    // Issued a moment apart, the two often end in the same second: the join then says it cannot tell which one the hub kept. Either
    // way the newer link stays.
    assert!(
        !ok && (text.contains("newer certificate is kept") || text.contains("cannot be told")),
        "{text}"
    );
    assert_eq!(read(&link, "cert.pem"), read(&newer, "cert.pem"), "{text}");
    assert_eq!(read(&link, "key.pem"), read(&newer, "key.pem"), "{text}");
}

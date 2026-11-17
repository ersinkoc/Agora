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

    // Closed by default, and the status says how to open it.
    let out = hub.pair(&owner_home, &["status"]).output().unwrap();
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
    // how to get one. Nothing is stored. (A found hub is tested in `pair_names.rs`.)
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
    // closed in `pair_names.rs`.
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

//! The Phase 2 exit criterion as a script: a node runs agents in worktrees and reports state, controlled
//! locally through the CLI. Real `agora` processes, real sockets, a real PTY for `terminal attach`; the
//! "agent" is a plain `sh` (never a real coding agent).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use agora_platform::{spawn, PtyConfig};

mod short_socket;

const AGORA: &str = env!("CARGO_BIN_EXE_agora");

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

struct Setup {
    home: PathBuf,
    repo: PathBuf,
}

fn setup(name: &str) -> Setup {
    setup_in(&std::env::temp_dir(), name)
}

fn setup_in(base: &Path, name: &str) -> Setup {
    setup_at(base.join(format!("agora-cli-{}-{name}", std::process::id())))
}

/// A fresh node home at `home` (emptied first).
fn setup_at(home: PathBuf) -> Setup {
    let _ = std::fs::remove_dir_all(&home);
    let repo = home.join("demo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "hi").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    std::fs::create_dir_all(home.join("manifests")).unwrap();
    std::fs::write(
        home.join("manifests/shell.toml"),
        "kind = \"shell\"\ndisplay_name = \"Shell\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\"]\n",
    )
    .unwrap();
    Setup { home, repo }
}

/// The home is removed with the setup: before, every run left its homes in the temp folder. Everything a test starts is declared
/// after its setup, so it ends first.
impl Drop for Setup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

struct NodeProc(Child);

impl Drop for NodeProc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_node(s: &Setup) -> NodeProc {
    start_node_at(s, &s.home)
}
fn start_node_at(s: &Setup, cwd: &Path) -> NodeProc {
    // Owned before the wait: a node that is slow to start is still killed when the test gives up on it (left running, it held the
    // binary open and broke the next build).
    let node = NodeProc(
        Command::new(AGORA)
            .current_dir(cwd)
            .args(["node", "run", "--project"])
            .arg(&s.repo)
            .env("AGORA_HOME", &s.home)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start node"),
    );
    let sock = s.home.join("node.sock");
    // A node on a busy Windows machine took up to 24 s to open its socket (measured), mostly in git.
    let end = Instant::now() + Duration::from_secs(60);
    while !sock.exists() {
        assert!(Instant::now() < end, "node did not create its socket");
        std::thread::sleep(Duration::from_millis(50));
    }
    node
}

/// Run `agora <args>` and return (exit code, stdout, stderr).
fn cli(s: &Setup, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(AGORA)
        .args(args)
        .env("AGORA_HOME", &s.home)
        // Text assertions below are English whatever the machine running the tests is set to.
        .env("AGORA_LANG", "en")
        .output()
        .expect("run agora");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn supported() -> bool {
    agora_platform::LOCAL_SOCKETS_SUPPORTED && agora_platform::PTY_SUPPORTED
}

/// Whether `sh` (the test agent) can be found on `PATH`. Every Unix has it; on Windows it comes with Git's `usr\bin`, which CI runners
/// put on `PATH` and a default Git for Windows install does not.
fn sh_on_path() -> bool {
    std::env::var_os("PATH").is_some_and(|p| {
        std::env::split_paths(&p).any(|d| d.join("sh").is_file() || d.join("sh.exe").is_file())
    })
}

/// The tests that start the `sh` agent run only where it can start; elsewhere they say why they did nothing.
fn shell_agents_supported() -> bool {
    if !supported() {
        return false;
    }
    if !sh_on_path() {
        eprintln!("skipped: no `sh` on PATH (on Windows, add Git's usr\\bin to PATH)");
        return false;
    }
    true
}

#[test]
fn start_attach_stop_from_the_command_line() {
    if !shell_agents_supported() {
        return;
    }
    let s = setup("e2e");
    let _node = start_node(&s);

    // Start an agent in its own worktree.
    let (code, out, err) = cli(
        &s,
        &["agent", "start", "shell", "--project", "demo", "--worktree"],
    );
    assert_eq!(code, 0, "stderr: {err}");
    assert!(out.starts_with("Started agent agent-1 in "), "{out}");
    let worktree = out.trim().rsplit(" in ").next().unwrap().to_string();
    assert!(
        Path::new(&worktree).join("README.md").exists(),
        "own worktree: {worktree}"
    );
    assert!(std::fs::canonicalize(&worktree)
        .unwrap()
        .starts_with(std::fs::canonicalize(s.home.join("worktrees")).unwrap()));

    // Attach in a real terminal: type a command, see its output, detach with Ctrl-].
    let mut cfg = PtyConfig::new(vec![
        AGORA.into(),
        "terminal".into(),
        "attach".into(),
        "agent-1".into(),
    ]);
    cfg.env
        .push(("AGORA_HOME".into(), s.home.display().to_string()));
    cfg.env.push(("AGORA_LANG".into(), "en".into()));
    let mut attach = spawn(&cfg).expect("spawn attach");
    attach.write_input(b"echo attached-$((6*7))\r").unwrap();
    let mut seen = String::new();
    let end = Instant::now() + Duration::from_secs(15);
    while !seen.contains("attached-42") {
        assert!(
            Instant::now() < end,
            "never saw the command output; got: {seen:?}"
        );
        if let Ok(chunk) = attach.output().recv_timeout(Duration::from_millis(100)) {
            seen.push_str(&String::from_utf8_lossy(&chunk));
        }
    }
    // Resizing the window resizes the agent's terminal: the shell inside reports the new size.
    attach.resize(101, 33).unwrap();
    attach.write_input(b"stty size\r").unwrap();
    let end = Instant::now() + Duration::from_secs(15);
    while !seen.contains("33 101") {
        assert!(
            Instant::now() < end,
            "the agent never saw the new size; got: {seen:?}"
        );
        if let Ok(chunk) = attach.output().recv_timeout(Duration::from_millis(100)) {
            seen.push_str(&String::from_utf8_lossy(&chunk));
        }
    }
    attach.write_input(&[0x1d]).unwrap();
    let status = attach.wait().unwrap();
    assert!(status.success(), "attach exited with {status:?}");
    while let Ok(chunk) = attach.output().recv_timeout(Duration::from_millis(500)) {
        seen.push_str(&String::from_utf8_lossy(&chunk));
    }
    assert!(seen.contains("Detached from agent-1."), "{seen:?}");

    // The agent is still running after detaching; stop it.
    let (code, out, err) = cli(&s, &["agent", "stop", "agent-1"]);
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out.trim(), "Stopped agent agent-1");
}

/// Runs `agora terminal attach agent-1` in a real terminal, types `typed` once `ready` shows (if any), waits for `until` in its output,
/// detaches with Ctrl-], and returns everything it wrote to the terminal.
fn attach_session(s: &Setup, typed: &[u8], until: &str) -> Vec<u8> {
    let mut cfg = PtyConfig::new(vec![
        AGORA.into(),
        "terminal".into(),
        "attach".into(),
        "agent-1".into(),
    ]);
    cfg.env
        .push(("AGORA_HOME".into(), s.home.display().to_string()));
    cfg.env.push(("AGORA_LANG".into(), "en".into()));
    let mut attach = spawn(&cfg).expect("spawn attach");
    if !typed.is_empty() {
        attach.write_input(typed).unwrap();
    }
    let mut seen = Vec::new();
    let end = Instant::now() + Duration::from_secs(15);
    while !String::from_utf8_lossy(&seen).contains(until) {
        assert!(
            Instant::now() < end,
            "never saw {until:?}; got: {:?}",
            String::from_utf8_lossy(&seen)
        );
        if let Ok(chunk) = attach.output().recv_timeout(Duration::from_millis(100)) {
            seen.extend_from_slice(&chunk);
        }
    }
    attach.write_input(&[0x1d]).unwrap();
    let status = attach.wait().unwrap();
    assert!(status.success(), "attach exited with {status:?}");
    while let Ok(chunk) = attach.output().recv_timeout(Duration::from_millis(500)) {
        seen.extend_from_slice(&chunk);
    }
    seen
}

/// The first `n` bytes of `b` with control bytes escaped, for assertion messages.
fn shown(b: &[u8], n: usize) -> String {
    b.iter()
        .take(n)
        .map(|&c| match c {
            0x1b => "\\e".to_string(),
            0x20..=0x7e => (c as char).to_string(),
            _ => format!("\\x{c:02x}"),
        })
        .collect()
}

/// ADR-0100 and the review of #953: the node's repaint switches the program's modes on in the user's own terminal when attaching
/// late, and detaching switches them off again and gives the terminal back its own keyboard flags (the repaint's kitty pop is not
/// passed on, and only the entry attach pushed is popped).
#[test]
fn detaching_gives_the_users_terminal_back_its_own_modes() {
    use agora_vt::{AlacrittyTerminal, CursorShape, MouseEncoding, MouseTracking, Size, Terminal};
    if !shell_agents_supported() {
        return;
    }
    let s = setup("attach-modes");
    let _node = start_node(&s);
    let (code, _, err) = cli(&s, &["agent", "start", "shell", "--project", "demo"]);
    assert_eq!(code, 0, "stderr: {err}");
    // The program switches the alternate screen, mouse, focus, keypad, paste, keyboard flags and a cursor shape on.
    attach_session(
        &s,
        b"printf '\\033[?1049h\\033[?1003h\\033[?1006h\\033[?1004h\\033=\\033[?2004h\\033[>5u\\033[5 q'; echo MODES-$((1+1))\r",
        "MODES-2",
    );
    // A late attach: the repaint carries those modes.
    let late = attach_session(&s, b"", "\x1b[?1003h");
    // Printed (shown only when the test fails): what the user's terminal got, start and end.
    println!("start: {}", shown(&late, 300));
    println!(
        "end: {}",
        shown(&late[late.len().saturating_sub(300)..], 300)
    );
    // On Windows the test's terminal is ConPTY, which is a terminal itself: it writes its own start (win32-input-mode and focus
    // reports, `CSI ? 9001 h CSI ? 1004 h`) before anything of the program's, then passes through the modes it does not keep and
    // keeps the ones it does (pen, cursor visibility, autowrap, focus), writing its own at the end. Only that start is set aside here;
    // everything after it is checked as on the other systems.
    let body = late
        .strip_prefix(b"\x1b[?9001h\x1b[?1004h")
        .unwrap_or(&late);
    assert!(
        body.starts_with(b"\x1b[>0u"),
        "attach pushes its own keyboard entry first: {}",
        shown(&late, 300)
    );
    let text = String::from_utf8_lossy(&late);
    assert!(
        !text.contains("\x1b[<32u"),
        "the repaint's pop is not passed on"
    );
    let tail = &text[text.rfind("\x1b[?1003h").unwrap_or(0)..];
    assert!(
        tail.contains("\x1b[?1049l"),
        "detach leaves the alternate screen: {tail:?}"
    );
    // The user's terminal, with keyboard flags of its own (a shell or tmux pushed 1), after the whole session.
    let mut host = AlacrittyTerminal::new(Size { cols: 80, rows: 24 });
    host.feed(b"\x1b[>1u");
    let before = host.modes();
    host.feed(&late);
    let m = host.modes();
    assert!(!m.alt_screen, "back on the main screen");
    assert_eq!(m.mouse_tracking, MouseTracking::Off);
    assert_eq!(m.mouse_encoding, MouseEncoding::Default);
    assert!(!m.focus_reporting && !m.application_keypad && !m.bracketed_paste);
    assert_eq!(m.keyboard_flags, 1, "the terminal's own keyboard flags");
    // Alternate scroll is restored by XTRESTORE where the terminal has it; this emulator has not, so it ends off (xterm's default).
    assert!(!m.alternate_scroll);
    assert_eq!(
        m,
        agora_vt::Modes {
            alternate_scroll: false,
            ..before
        }
    );
    assert!(host.cursor().visible);
    assert_eq!(host.cursor().shape, CursorShape::Default);
}

/// The agent pushes keyboard entries on the main screen while attached and is still running at detach: detaching pops them and its
/// own entry, so the user's terminal is back on its own flags (review of #953).
#[test]
fn detaching_pops_what_the_agent_pushed_on_the_main_screen() {
    use agora_vt::{AlacrittyTerminal, Size, Terminal};
    if !shell_agents_supported() {
        return;
    }
    let s = setup("attach-pushes");
    let _node = start_node(&s);
    let (code, _, err) = cli(&s, &["agent", "start", "shell", "--project", "demo"]);
    assert_eq!(code, 0, "stderr: {err}");
    let session = attach_session(
        &s,
        b"printf '\\033[>5u\\033[>7u'; echo PUSHED-$((1+1))\r",
        "PUSHED-2",
    );
    let mut host = AlacrittyTerminal::new(Size { cols: 80, rows: 24 });
    host.feed(b"\x1b[>1u");
    host.feed(&session);
    assert_eq!(host.modes().keyboard_flags, 1, "the terminal's own flags");
    // Detaching from the main screen does not send `CSI ? 1049 l`: xterm and xterm.js would restore an old saved cursor on it and
    // write the next lines over the top of the screen.
    let text = String::from_utf8_lossy(&session);
    let tail = &text[text.rfind("PUSHED-2").unwrap_or(0)..];
    assert!(!tail.contains("\x1b[?1049l"), "{tail:?}");
}

#[test]
fn errors_are_rendered_from_codes_and_exit_nonzero() {
    if !shell_agents_supported() {
        return;
    }
    let s = setup("errors");
    let _node = start_node(&s);

    let (code, _, err) = cli(&s, &["agent", "stop", "agent-9"]);
    assert_eq!(code, 1);
    assert!(
        err.contains("agent.not_found") && err.contains("agent-9 was not found"),
        "{err}"
    );

    let (code, _, err) = cli(&s, &["agent", "prompt", "agent-9", "hello", "there"]);
    assert_eq!(code, 1);
    assert!(err.contains("agent.not_found"), "{err}");
    let (code, _, err) = cli(&s, &["agent", "prompt", "agent-9"]);
    assert_eq!(code, 2, "a prompt needs text: {err}");

    let (code, _, err) = cli(&s, &["agent", "start", "nope"]);
    assert_eq!(code, 1);
    assert!(err.contains("agent.kind_unknown"), "{err}");

    let (code, _, err) = cli(&s, &["agent", "start", "shell", "--project", "ghost"]);
    assert_eq!(code, 1);
    assert!(err.contains("project.not_found"), "{err}");

    // Listing (ADR-0025): empty first, then one tab-separated line per agent.
    let (code, out, _) = cli(&s, &["agent", "list"]);
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "This node has no agents.");
    let (code, _, err) = cli(
        &s,
        &["agent", "start", "shell", "--project", "demo", "--worktree"],
    );
    assert_eq!(code, 0, "{err}");
    let (code, out, _) = cli(&s, &["agent", "list"]);
    assert_eq!(code, 0);
    let cols: Vec<&str> = out.trim().split('\t').collect();
    assert_eq!(
        (cols[0], cols[1], cols[3]),
        ("agent-1", "shell", "running"),
        "{out}"
    );
    cli(&s, &["agent", "stop", "agent-1"]);
    let (_, out, _) = cli(&s, &["agent", "list"]);
    assert!(out.trim().ends_with("stopped"), "{out}");
}

/// The ledger entry of `id` as the node wrote it (`<home>/data/sessions/ledger.json`), `None` until it is there.
fn ledger_entry(s: &Setup, id: &str) -> Option<serde_json::Value> {
    let v: serde_json::Value =
        serde_json::from_slice(&std::fs::read(s.home.join("data/sessions/ledger.json")).ok()?)
            .ok()?;
    v["entries"]
        .as_array()?
        .iter()
        .find(|e| e["agent_id"] == id)
        .cloned()
}

/// The lines of `agora agent list`, once the node answers (a restarted node's socket appears before it serves).
fn agent_list_lines(s: &Setup) -> Vec<String> {
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        let (code, out, err) = cli(s, &["agent", "list"]);
        if code == 0 {
            return out.lines().map(str::to_string).collect();
        }
        assert!(Instant::now() < end, "agent list failed: {err}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// ADR-0107 (#1121): after the node was killed, `agora agent list` lists the agent it found interrupted with `interrupted` in the
/// fourth column and what can be done in a fifth, in id order with the agent that ended, whose line is the four columns it always was.
#[test]
fn agent_list_shows_the_agents_a_restarted_node_found_interrupted() {
    if !shell_agents_supported() {
        return;
    }
    let s = setup("interrupted");
    let node = start_node(&s);
    for _ in 0..2 {
        let (code, _, err) = cli(
            &s,
            &["agent", "start", "shell", "--project", "demo", "--worktree"],
        );
        assert_eq!(code, 0, "{err}");
    }
    let (code, _, err) = cli(&s, &["agent", "stop", "agent-1"]);
    assert_eq!(code, 0, "{err}");
    // The node records the end before it is killed, so agent-1 is an ended agent and only agent-2 was running.
    let end = Instant::now() + Duration::from_secs(30);
    while !ledger_entry(&s, "agent-1").is_some_and(|e| e["ended"] == true)
        || ledger_entry(&s, "agent-2").is_none()
    {
        assert!(
            Instant::now() < end,
            "the ledger never recorded both agents"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    // Killed without stopping its agents; a new node starts on the same home.
    drop(node);
    let _ = std::fs::remove_file(s.home.join("node.sock"));
    let _node = start_node(&s);
    let lines = agent_list_lines(&s);
    assert_eq!(lines.len(), 2, "{lines:?}");
    let ended: Vec<&str> = lines[0].split('\t').collect();
    assert_eq!(ended.len(), 4, "{lines:?}");
    assert_eq!(
        (ended[0], ended[1], ended[3]),
        ("agent-1", "shell", "stopped"),
        "{lines:?}"
    );
    let cols: Vec<&str> = lines[1].split('\t').collect();
    assert_eq!(cols.len(), 5, "{lines:?}");
    assert_eq!(
        (cols[0], cols[1], cols[3]),
        ("agent-2", "shell", "interrupted"),
        "{lines:?}"
    );
    // A shell has no session to continue: Start again, once its process is gone. Where it outlived the node, stop ends it.
    if cols[4] == "still-running" {
        let (code, _, err) = cli(&s, &["agent", "stop", "agent-2"]);
        assert_eq!(code, 0, "{err}");
        let lines = agent_list_lines(&s);
        assert!(
            lines[1].ends_with("\tinterrupted\tstart-again"),
            "{lines:?}"
        );
    } else {
        assert_eq!(cols[4], "start-again", "{lines:?}");
    }
}

/// #1138 review F2: a node whose only agent is interrupted lists it, and does not also say it has no agents (that sentence is for no
/// line at all, not for no `agents` row).
#[test]
fn a_node_with_only_an_interrupted_agent_does_not_say_it_has_none() {
    if !shell_agents_supported() {
        return;
    }
    let s = setup("only-interrupted");
    let node = start_node(&s);
    let (code, _, err) = cli(
        &s,
        &["agent", "start", "shell", "--project", "demo", "--worktree"],
    );
    assert_eq!(code, 0, "{err}");
    let end = Instant::now() + Duration::from_secs(30);
    while ledger_entry(&s, "agent-1").is_none() {
        assert!(Instant::now() < end, "the ledger never recorded the agent");
        std::thread::sleep(Duration::from_millis(50));
    }
    drop(node);
    let _ = std::fs::remove_file(s.home.join("node.sock"));
    let _node = start_node(&s);
    let lines = agent_list_lines(&s);
    assert!(
        !lines.iter().any(|l| l.contains("This node has no agents.")),
        "{lines:?}"
    );
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].starts_with("agent-1\tshell\t"), "{lines:?}");
    assert!(lines[0].contains("\tinterrupted\t"), "{lines:?}");
    // A survivor is ended here, so the test leaves no `sh` behind.
    if lines[0].ends_with("\tstill-running") {
        let (code, _, err) = cli(&s, &["agent", "stop", "agent-1"]);
        assert_eq!(code, 0, "{err}");
    }
}

/// RM-19 (#903): `agora agent start` for a kind whose program this machine cannot find says so in words, with the program, for a
/// terminal kind and for an ACP kind (started through the bridge, which ends at once), as `agora agent kinds` already marks it missing.
#[test]
fn a_missing_agent_program_is_said_in_words() {
    if !supported() {
        return;
    }
    let s = setup("missing-program");
    std::fs::write(
        s.home.join("manifests/rm-missing.toml"),
        "kind = \"rm-missing\"\ndisplay_name = \"Missing\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"rm-no-such-program\"]\n",
    )
    .unwrap();
    std::fs::write(
        s.home.join("manifests/rm-missing-acp.toml"),
        "kind = \"rm-missing-acp\"\ndisplay_name = \"Missing ACP\"\nversion = \"1\"\nmin_engine = 1\n[structured]\nacp = { argv = [\"rm-no-such-acp-program\", \"acp\"] }\n",
    )
    .unwrap();
    let _node = start_node(&s);
    let (code, out, _) = cli(&s, &["agent", "kinds"]);
    assert_eq!(code, 0);
    assert!(out.contains("rm-missing\tMissing\tmissing"), "{out}");
    for (kind, program) in [
        ("rm-missing", "rm-no-such-program"),
        ("rm-missing-acp", "rm-no-such-acp-program"),
    ] {
        let (code, _, err) = cli(
            &s,
            &["agent", "start", kind, "--project", "demo", "--worktree"],
        );
        assert_eq!(code, 1, "{err}");
        assert!(err.contains("agent.program_missing"), "{err}");
        assert!(
            err.contains(&format!("cannot find {program} in its PATH")),
            "{err}"
        );
        assert!(!err.contains("Something went wrong"), "{err}");
    }
    let (_, out, _) = cli(&s, &["agent", "list"]);
    assert_eq!(out.trim(), "This node has no agents.");
}

#[test]
fn without_a_node_the_cli_says_so_and_bad_arguments_print_usage() {
    if !supported() {
        return;
    }
    let s = setup("nonode");
    let (code, _, err) = cli(&s, &["agent", "stop", "agent-1"]);
    assert_eq!(code, 1);
    assert!(err.contains("No node is listening"), "{err}");

    let (code, _, err) = cli(&s, &["agent", "start"]);
    assert_eq!(code, 2);
    assert!(err.contains("Usage:"), "{err}");
    let (code, _, err) = cli(&s, &["frobnicate"]);
    assert_eq!(code, 2);
    assert!(err.contains("Usage:"), "{err}");
    let (code, _, err) = cli(&s, &["agent", "stop", "--bogus"]);
    assert_eq!(code, 2);
    assert!(err.contains("not understood"), "{err}");

    // Attaching needs a terminal; with piped stdio it refuses instead of corrupting anything.
    let _node = start_node(&s);
    let (code, _, err) = cli(&s, &["terminal", "attach", "agent-1"]);
    assert_eq!(code, 1);
    assert!(err.contains("interactive terminal"), "{err}");
}

/// ADR-0093: a holder that speaks only link version 1 cannot keep structured agents across a node restart; doctor says so from the note
/// the node leaves, as a warning (the exit status is the same), and never attaches to the holder or ends anything.
#[test]
fn doctor_notes_a_holder_that_speaks_only_link_version_1() {
    if !agora_platform::HOLDER_BUILT {
        return;
    }
    let s = setup("doctor-old-holder");
    let (base_code, base_out, _) = cli(&s, &["doctor"]);
    assert!(!base_out.contains("agent keeper"), "{base_out}");

    // A stand-in for the old holder: a process of ours that only sleeps. Nothing listens on a socket for it.
    let mut fake = Command::new("sleep").arg("600").spawn().unwrap();
    let start = agora_platform::process_start_time(fake.id()).unwrap();
    std::fs::create_dir_all(s.home.join("run")).unwrap();
    let note = s.home.join("run/hold.link");
    std::fs::write(&note, format!("1 {} {start}\n", fake.id())).unwrap();
    std::fs::write(
        s.home.join("run/hold.pid"),
        format!("{} {start} 2\n", fake.id()),
    )
    .unwrap();

    let (code, out, err) = cli(&s, &["doctor"]);
    assert_eq!(code, base_code, "{out}{err}");
    assert!(
        out.contains(
            "Chat agents on this machine are resumed after a node restart, not kept running"
        ),
        "{out}"
    );
    assert!(out.contains("2 agents now"), "{out}");
    // Read only: no socket was made or used, and the holder still runs.
    assert!(!s.home.join("run/hold.sock").exists());
    assert!(agora_platform::process_start_time(fake.id()).is_some());

    // A version 2 holder: nothing.
    std::fs::write(&note, format!("2 {} {start}\n", fake.id())).unwrap();
    let (_, out, _) = cli(&s, &["doctor"]);
    assert!(!out.contains("agent keeper"), "{out}");

    // Version 1 but the holder has ended: nothing.
    std::fs::write(&note, format!("1 {} {start}\n", fake.id())).unwrap();
    let _ = fake.kill();
    let _ = fake.wait();
    let (_, out, _) = cli(&s, &["doctor"]);
    assert!(!out.contains("agent keeper"), "{out}");
    let _ = std::fs::remove_dir_all(&s.home);
}

#[test]
fn doctor_reports_agents_and_exits_zero_on_a_healthy_machine() {
    let s = setup("doctor");
    let (code, out, err) = cli(&s, &["doctor"]);
    // Without terminal support (Windows for now) doctor correctly reports a problem and exits 1.
    let expected = if agora_platform::PTY_SUPPORTED { 0 } else { 1 };
    assert_eq!(code, expected, "{out}{err}");
    assert!(out.contains("Git is available"), "{out}");
    // The built-in kinds and the folder's `shell`.
    let loaded = agora_node::manifests::load(std::path::Path::new(""))
        .0
        .len();
    assert!(
        out.contains(&format!("{loaded} agent descriptions loaded.")),
        "{out}"
    );
    // The manifest launches `sh`: doctor says where the kind came from and whether its program is found; a missing one is a note,
    // not a failure.
    if agora_platform::PTY_SUPPORTED {
        if sh_on_path() {
            assert!(
                out.contains("shell (from the manifests folder): `sh` is found"),
                "{out}"
            );
        } else {
            assert!(
                out.contains("[note] shell (from the manifests folder): this machine cannot find `sh` in its PATH"),
                "{out}"
            );
        }
    }
    assert!(out.contains("codex (built in): "), "{out}");
    // No node is running in this setup: a note, not a failure.
    assert!(out.contains("[note] No node is running"), "{out}");
}

#[test]
fn node_stdio_carries_the_framed_protocol_over_pipes() {
    use agora_protocol::frame::DEFAULT_MAX_PAYLOAD;
    use agora_protocol::v1::{command_request::Command as Cmd, command_response::Result as Res};
    use agora_protocol::{Decoder, Frame};
    use prost::Message;

    if !supported() {
        return;
    }
    let s = setup("stdio");
    let _node = start_node(&s);
    let mut child = Command::new(AGORA)
        .args(["node", "stdio"])
        .env("AGORA_HOME", &s.home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let req = agora_protocol::v1::CommandRequest {
        request_id: "r1".into(),
        idempotency_key: String::new(),
        target_node: String::new(),
        command: Some(Cmd::AgentStop(agora_protocol::v1::AgentStop {
            agent_id: "agent-404".into(),
            stop_helpers: false,
        })),
    };
    let bytes = Frame {
        stream_id: 1,
        kind: 0,
        seq: None,
        payload: req.encode_to_vec(),
    }
    .encode(DEFAULT_MAX_PAYLOAD)
    .unwrap();
    child.stdin.as_mut().unwrap().write_all(&bytes).unwrap();

    let mut out = child.stdout.take().unwrap();
    let mut dec = Decoder::new(DEFAULT_MAX_PAYLOAD);
    let mut buf = [0u8; 4096];
    let frame = loop {
        let n = out.read(&mut buf).unwrap();
        assert!(n > 0, "bridge closed before answering");
        dec.push(&buf[..n]);
        if let Some(f) = dec.next_frame().unwrap() {
            break f;
        }
    };
    assert_eq!((frame.stream_id, frame.kind), (1, 1));
    let resp = agora_protocol::v1::CommandResponse::decode(frame.payload.as_slice()).unwrap();
    assert_eq!(resp.request_id, "r1");
    match resp.result {
        Some(Res::Err(e)) => assert_eq!(e.code, "agent.not_found"),
        other => panic!("expected agent.not_found, got {other:?}"),
    }
    // The remote side hanging up ends the bridge.
    drop(child.stdin.take());
    let status = child.wait().unwrap();
    assert!(status.success());
}

#[test]
fn node_stdio_without_a_node_says_so() {
    if !supported() {
        return;
    }
    let s = setup("stdio-none");
    let (code, _, err) = cli(&s, &["node", "stdio"]);
    assert_eq!(code, 1);
    assert!(err.contains("No node is listening"), "{err}");
}

#[test]
fn mesh_stdio_is_an_mcp_server_for_one_agent() {
    if !shell_agents_supported() {
        return;
    }
    let s = setup("mesh-stdio");
    let _node = start_node(&s);
    let (code, out, err) = cli(
        &s,
        &["agent", "start", "shell", "--project", "demo", "--worktree"],
    );
    assert_eq!(code, 0, "{out}{err}");
    // This is the command an agent's MCP configuration would run.
    let socket = s.home.join("m").join("agent-1.sock");
    let mut child = Command::new(AGORA)
        .args(["mesh", "stdio", "--socket"])
        .arg(&socket)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut out = std::io::BufReader::new(child.stdout.take().unwrap());
    let mut ask = |req: &str| -> String {
        use std::io::BufRead;
        writeln!(stdin, "{req}").unwrap();
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        line
    };
    let init = ask(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
    );
    assert!(init.contains("agora-mesh"), "{init}");
    let tools = ask(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
    assert!(
        tools.contains("send_message") && tools.contains("handoff"),
        "{tools}"
    );
    let me = ask(
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"agents_list","arguments":{}}}"#,
    );
    assert!(me.contains("agent-1"), "{me}");
    drop(stdin);
    assert!(child.wait().unwrap().success());
}

/// What an agent's MCP client does: read the config the node wrote, run the command in it, speak MCP to it.
#[test]
fn the_injected_mcp_config_launches_a_working_mesh_server() {
    if !shell_agents_supported() {
        return;
    }
    let s = setup("inject-e2e");
    std::fs::write(
        s.home.join("manifests/mcp-shell.toml"),
        "kind = \"mcp-shell\"\ndisplay_name = \"S\"\nversion = \"1\"\nmin_engine = 1\n[process]\nlaunch = [\"sh\"]\n[mcp]\nmethod = \"config_file\"\npath = \".mcp.json\"\n",
    )
    .unwrap();
    let _node = start_node(&s);
    let (code, out, err) = cli(
        &s,
        &[
            "agent",
            "start",
            "mcp-shell",
            "--project",
            "demo",
            "--worktree",
        ],
    );
    assert_eq!(code, 0, "{out}{err}");
    let worktree = out.trim().rsplit(" in ").next().unwrap().to_string();
    let cfg: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(Path::new(&worktree).join(".mcp.json")).unwrap(),
    )
    .unwrap();
    let server = &cfg["mcpServers"]["agora-mesh"];
    let args: Vec<String> = server["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_string())
        .collect();
    let mut child = Command::new(server["command"].as_str().unwrap())
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut out = std::io::BufReader::new(child.stdout.take().unwrap());
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{}}}}"#
    )
    .unwrap();
    let mut line = String::new();
    std::io::BufRead::read_line(&mut out, &mut line).unwrap();
    assert!(line.contains("agora-mesh"), "{line}");
    drop(stdin);
    assert!(child.wait().unwrap().success());
}

#[test]
fn a_node_owner_can_trust_a_device_and_bad_input_trusts_nothing() {
    let home = std::env::temp_dir().join(format!("agora-trustcli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let key = format!("04{}", "ab".repeat(64));
    let run = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_agora"))
            .args(args)
            .env("AGORA_HOME", &home)
            .env("AGORA_LANG", "en")
            .output()
            .unwrap()
    };
    assert!(run(&["node", "trust", "phone", &key]).status.success());
    let file = home.join("data/trusted-devices");
    // One line, the id and the key as before; ADR-0061 adds when its trust began (`since=`).
    let written = std::fs::read_to_string(&file).unwrap();
    assert!(
        written.starts_with(&format!("phone {key} since=")),
        "{written}"
    );
    assert_eq!(written.lines().count(), 1);
    assert!(!run(&["node", "trust", "phone", "zz"]).status.success());
    assert!(!run(&["node", "trust", "phone"]).status.success());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), written);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_registered_project_name_wins_over_same_named_folders_in_both_processes() {
    if !agora_platform::PTY_SUPPORTED || !agora_platform::LOCAL_SOCKETS_SUPPORTED {
        return;
    }
    let s = setup("project-name");
    let caller = s.home.join("caller");
    let caller_demo = caller.join("demo");
    std::fs::create_dir_all(&caller_demo).unwrap();
    let _node = start_node_at(&s, &caller);
    for (option, value, expected) in [
        ("--project", "demo", s.repo.as_path()),
        ("--cwd", "demo", caller_demo.as_path()),
    ] {
        let out = Command::new(AGORA)
            .args(["agent", "start", "shell", option, value])
            .current_dir(&caller)
            .env("AGORA_HOME", &s.home)
            .env("AGORA_LANG", "en")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8(out.stdout).unwrap();
        let cwd = stdout.trim().rsplit(" in ").next().unwrap();
        assert_eq!(
            std::fs::canonicalize(cwd).unwrap(),
            std::fs::canonicalize(expected).unwrap()
        );
    }
}

/// ADR-0082: where the node confines agents on Linux, the control socket needs its token; `agora` reads it itself, so the commands and
/// their output are the same, and `agora node stdio` (the bridge a remote hub uses over ssh) carries a connection that is already
/// proved.
#[test]
fn with_a_confining_node_the_cli_reads_the_control_token_itself() {
    if !shell_agents_supported()
        || !agora_platform::CONTROL_TOKEN_REQUIRED
        || !agora_platform::sandbox_supported()
    {
        return;
    }
    // A folder agents are not granted (the system temp folder is writable to them, and the node refuses to start one then), short
    // enough for the node's sockets in a deep checkout (#1040). Declared before the node, so it is removed after the node ends.
    let dir = short_socket::SocketDir::new(
        &Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("agora-cli-{}-token", std::process::id())),
        "cli-token",
    );
    let s = setup_at(dir.path().to_path_buf());
    std::fs::create_dir_all(s.home.join("data")).unwrap();
    std::fs::write(s.home.join("data/policy"), "sandbox = worktree\n").unwrap();
    let _node = start_node(&s);
    let token = s.home.join("control.token");
    let end = Instant::now() + Duration::from_secs(20);
    while !token.exists() {
        assert!(Instant::now() < end, "no control token");
        std::thread::sleep(Duration::from_millis(50));
    }
    let (code, out, err) = cli(&s, &["agent", "list"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("no agents"), "{out}");

    // Without the file the client has nothing to present: refused, said in words.
    let hidden = s.home.join("hidden.token");
    std::fs::rename(&token, &hidden).unwrap();
    let (code, _, err) = cli(&s, &["agent", "list"]);
    assert_eq!(code, 1);
    assert!(err.contains("control token"), "{err}");
    std::fs::rename(&hidden, &token).unwrap();

    // The stdio bridge: one request frame in, its answer out.
    let mut bridge = Command::new(AGORA)
        .args(["node", "stdio"])
        .env("AGORA_HOME", &s.home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let req = agora_protocol::v1::CommandRequest {
        request_id: "1".into(),
        command: Some(agora_protocol::v1::command_request::Command::AgentList(
            agora_protocol::v1::AgentList::default(),
        )),
        ..Default::default()
    };
    let bytes = agora_protocol::Frame {
        stream_id: agora_protocol::frame::STREAM_COMMANDS,
        kind: 0,
        seq: None,
        payload: prost::Message::encode_to_vec(&req),
    }
    .encode(agora_protocol::frame::DEFAULT_MAX_PAYLOAD)
    .unwrap();
    bridge.stdin.as_mut().unwrap().write_all(&bytes).unwrap();
    let mut dec = agora_protocol::Decoder::new(agora_protocol::frame::DEFAULT_MAX_PAYLOAD);
    let mut out = bridge.stdout.take().unwrap();
    let mut buf = [0u8; 4096];
    let frame = loop {
        let n = out.read(&mut buf).unwrap();
        assert!(n > 0, "the bridge closed");
        dec.push(&buf[..n]);
        if let Some(f) = dec.next_frame().unwrap() {
            break f;
        }
    };
    let resp =
        <agora_protocol::v1::CommandResponse as prost::Message>::decode(frame.payload.as_slice())
            .unwrap();
    assert!(
        matches!(
            resp.result,
            Some(agora_protocol::v1::command_response::Result::Ok(_))
        ),
        "{resp:?}"
    );
    let _ = bridge.kill();
    let _ = bridge.wait();
}

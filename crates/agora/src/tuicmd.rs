//! `agora tui`: the terminal console (ADR-0043). Slice 1 shows machines, agents and what waits for the user; slice 2 answers the
//! first waiting approval from the keyboard (`y` allow, `n` deny) with this terminal's own key (ADR-0028); slice 3 opens an agent's
//! terminal through the hub (`Tab` picks the agent, `Enter` opens it, Ctrl-] comes back).

use std::io::{IsTerminal, Read, Write};
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use agora_i18n::{key, Arg};
use agora_protocol::v1::Snapshot;
use agora_tui::client::{sign_in, ClientError, Hub, Session, TermEvent, Update};
use agora_tui::device::Device;
use agora_tui::render::render;

use crate::hubcli::{hub_dir, new_login_code, DEFAULT_LISTEN};
use crate::{home, Opts, Say};

/// Fewest milliseconds between two refreshes (each one is a fresh snapshot).
const REFRESH_MS: u64 = 500;

/// This terminal's key, made the first time it is needed and kept in a file only its owner can read.
fn device() -> Option<Device> {
    let path = home().join("tui-device.p8");
    if let Some(d) = std::fs::read(&path)
        .ok()
        .and_then(|b| Device::from_pkcs8(&b))
    {
        return Some(d);
    }
    let (d, bytes) = Device::generate()?;
    std::fs::create_dir_all(home()).ok()?;
    agora_platform::write_private(&path, &bytes).ok()?;
    Some(d)
}

/// Answers the first waiting approval. Returns the line to show.
fn answer(say: &Say, session: &mut Session, snapshot: &Snapshot, approved: bool) -> String {
    let Some(device) = device() else {
        return say.text(
            key!("tui-answer-failed"),
            &[("code", Arg::Str("device".into()))],
        );
    };
    match session.resolve_first_approval(snapshot, &device, approved) {
        Ok(false) => String::new(),
        Ok(true) => say.text(
            if approved {
                key!("tui-allowed")
            } else {
                key!("tui-denied")
            },
            &[],
        ),
        Err(code) if code == "approval.signature_invalid" => say.text(
            key!("tui-not-trusted"),
            &[
                ("id", Arg::Str(device.id.clone())),
                ("key", Arg::Str(device.public_key_hex())),
            ],
        ),
        Err(code) => say.text(key!("tui-answer-failed"), &[("code", Arg::Str(code))]),
    }
}

/// Ctrl-] : the key that leaves an opened terminal (the same one `agora terminal attach` uses).
const DETACH: u8 = 0x1d;

/// Shows an agent's terminal until the user detaches or the program ends. `Err` is the error code when it could not be opened.
/// `Ok(true)` means the link to the hub was lost while watching.
fn attach(
    session: &mut Session,
    snapshot: &Snapshot,
    agent: &str,
    rx: &mpsc::Receiver<u8>,
) -> Result<bool, String> {
    let (node, local) = agent.split_once('/').unwrap_or(("", agent));
    // The machine's own terminal id; the snapshot's table says which agent has one.
    if !snapshot.terminals.iter().any(|t| t.agent_id == agent) {
        return Err("terminal.not_found".into());
    }
    let terminal = format!("t-{local}");
    let stream = session.terminal_open(node, &terminal)?;
    if let Some((cols, rows)) = agora_platform::terminal_size() {
        session.terminal_resize(node, &terminal, cols, rows);
    }
    let mut out = std::io::stdout();
    let _ = write!(out, "\x1b[2J\x1b[H");
    let _ = out.flush();
    let mut lost = false;
    'watch: loop {
        match session.terminal_next(stream, Duration::from_millis(50)) {
            TermEvent::Data(b) | TermEvent::Screen(b) => {
                let _ = out.write_all(&b);
                let _ = out.flush();
            }
            TermEvent::Exit(_) | TermEvent::Ended => break,
            TermEvent::Lost => {
                lost = true;
                break;
            }
            TermEvent::Resized(..) | TermEvent::Quiet => {}
        }
        let mut typed = Vec::new();
        while let Ok(k) = rx.try_recv() {
            if k == DETACH {
                break 'watch;
            }
            typed.push(k);
        }
        if !typed.is_empty() {
            session.terminal_input(node, &terminal, &typed);
        }
    }
    session.terminal_close(stream);
    Ok(lost)
}

pub fn tui(say: &Say, o: &Opts) -> ExitCode {
    let dir = hub_dir(o);
    // On the hub's own machine the directory has everything: its CA and a way to make a code. Elsewhere the user passes them.
    let ca_pem = match &o.ca {
        Some(path) => std::fs::read_to_string(path).ok(),
        None => std::fs::read_to_string(dir.join("ca.pem")).ok(),
    };
    let Some(ca_pem) = ca_pem else {
        say.err(key!("tui-need-hub"), &[]);
        return ExitCode::from(2);
    };
    let addr = o.hub.clone().unwrap_or_else(|| DEFAULT_LISTEN.to_string());
    let hub = Hub {
        addr: addr.clone(),
        ca_pem,
    };
    let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let text = |id: &str, args: &[(&str, Arg)]| say.text(id, args);
    let hub_arg = [("hub", Arg::Str(addr))];

    // Keys come from the terminal in raw mode (only when there is one); a thread hands them over so the loop never blocks on them.
    let raw = std::io::stdin()
        .is_terminal()
        .then(agora_platform::RawMode::enable)
        .and_then(Result::ok);
    let keys = raw.is_some();
    let (tx, rx) = mpsc::channel::<u8>();
    if keys {
        std::thread::spawn(move || {
            let mut b = [0u8; 1];
            while std::io::stdin().read(&mut b).is_ok_and(|n| n == 1) {
                if tx.send(b[0]).is_err() {
                    break;
                }
            }
        });
    }
    let eol = if keys { "\r\n" } else { "\n" };

    let mut token: Option<String> = None;
    let mut first = true;
    let mut snapshot: Option<Snapshot> = None;
    let mut notice = String::new();
    let mut selected = String::new();
    loop {
        if first {
            say.err(key!("tui-connecting"), &hub_arg);
        }
        let session = (|| {
            let t = match &token {
                Some(t) => t.clone(),
                None => {
                    // A code from the flag the first time; on the hub's machine a new one whenever it is needed.
                    let code = match (&o.code, o.hub.is_none()) {
                        (Some(c), _) if first => c.clone(),
                        (_, true) => new_login_code(&dir).map_err(|_| ClientError::Refused)?,
                        _ => return Err(ClientError::Refused),
                    };
                    sign_in(&hub, &code)?
                }
            };
            let s = Session::connect(&hub, &t)?;
            token = Some(t);
            Ok(s)
        })();
        let mut session = match session {
            Ok(s) => s,
            Err(ClientError::Refused | ClientError::TooMany) => {
                token = None;
                if first || o.hub.is_some() {
                    say.err(key!("tui-signin-failed"), &[]);
                    return ExitCode::from(1);
                }
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
            Err(ClientError::Unreachable) => {
                say.err(key!("tui-lost"), &hub_arg);
                first = false;
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
        };
        first = false;
        let mut dirty_since: Option<Instant> = None;
        let mut last_refresh = Instant::now();
        let draw = |snapshot: &Option<Snapshot>, selected: &str, notice: &str| {
            if let Some(s) = snapshot {
                let lines = render(s, color, keys, selected, notice, &text);
                let mut out = std::io::stdout().lock();
                let _ = write!(out, "\x1b[H\x1b[2J{}{eol}", lines.join(eol));
                let _ = out.flush();
            }
        };
        loop {
            match session.next(Duration::from_millis(250)) {
                Update::Snapshot(s) => {
                    snapshot = Some(*s);
                    draw(&snapshot, &selected, &notice);
                    dirty_since = None;
                }
                Update::Changed => {
                    dirty_since.get_or_insert_with(Instant::now);
                }
                Update::Quiet => {}
                Update::Closed => {
                    say.err(key!("tui-lost"), &hub_arg);
                    std::thread::sleep(Duration::from_secs(2));
                    break;
                }
            }
            let mut link_lost = false;
            while let Ok(k) = rx.try_recv() {
                match k {
                    3 | b'q' => return ExitCode::SUCCESS, // Ctrl-C is a plain byte in raw mode
                    b'y' | b'n' => {
                        if let Some(s) = &snapshot {
                            notice = answer(say, &mut session, s, k == b'y');
                            dirty_since.get_or_insert_with(Instant::now);
                            draw(&snapshot, &selected, &notice);
                        }
                    }
                    b'\t' => {
                        if let Some(s) = &snapshot {
                            let with_terminal: Vec<&str> =
                                s.terminals.iter().map(|t| t.agent_id.as_str()).collect();
                            let at = with_terminal.iter().position(|a| *a == selected);
                            let next = at.map_or(0, |i| (i + 1) % with_terminal.len().max(1));
                            selected = with_terminal
                                .get(next)
                                .map(|a| (*a).to_string())
                                .unwrap_or_default();
                            draw(&snapshot, &selected, &notice);
                        }
                    }
                    b'\r' | b'\n' => {
                        if let (Some(s), false) = (&snapshot, selected.is_empty()) {
                            match attach(&mut session, s, &selected, &rx) {
                                Ok(true) => {
                                    say.err(key!("tui-lost"), &hub_arg);
                                    link_lost = true;
                                }
                                Ok(false) => notice = String::new(),
                                Err(code) => {
                                    notice = say.text(
                                        key!("tui-opening-failed"),
                                        &[("code", Arg::Str(code))],
                                    );
                                }
                            }
                            dirty_since.get_or_insert_with(Instant::now);
                            draw(&snapshot, &selected, &notice);
                        }
                    }
                    _ => {}
                }
            }
            if link_lost {
                std::thread::sleep(Duration::from_secs(2));
                break;
            }
            if dirty_since.is_some() && last_refresh.elapsed() >= Duration::from_millis(REFRESH_MS)
            {
                // The hub answers a new subscription with a fresh snapshot; a live feed applying single events is later work.
                break;
            }
            if dirty_since.is_none() {
                last_refresh = Instant::now();
            }
        }
    }
}

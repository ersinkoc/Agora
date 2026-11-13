//! `agora tui`: the terminal console (ADR-0043). Slice 1 shows machines, agents and what waits for the user; slice 2 answers the
//! first waiting approval from the keyboard (`y` allow, `n` deny) with this terminal's own key (ADR-0028); slice 3 opens an agent's
//! terminal through the hub (`Tab` picks the agent, `Enter` opens it, Ctrl-] comes back).

use std::io::{IsTerminal, Read, Write};
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use agora_i18n::{key, Arg};
use agora_protocol::v1::{param_value::Value, ChatEntry, ErrorInfo, Snapshot};
use agora_tui::client::{sign_in, ChatEvent, ClientError, Hub, Session, TermEvent, Update};
use agora_tui::device::Device;
use agora_tui::preview::{Preview, Pumped};
use agora_tui::render::{chat_command, plain, render, render_chat, session_line, ChatCommand};
use agora_tui::split::{frame, tree_order, View};

use crate::hubcli::{hub_dir, new_login_code, DEFAULT_LISTEN};
use crate::{home, Opts, Say};

/// Fewest milliseconds between two refreshes (each one is a fresh snapshot).
const REFRESH_MS: u64 = 500;

/// The hub decides whether an agent can be forgotten; keep its localized reason even when the snapshot was stale.
fn forget_notice(say: &Say, result: Result<(), ErrorInfo>) -> String {
    let Err(error) = result else {
        return say.text(key!("tui-forgot"), &[]);
    };
    let reason = match error.code.as_str() {
        "client.unreachable" => say.text(key!("tui-forget-disconnected"), &[]),
        "node.timeout" if error.params.is_empty() => say.text(key!("tui-forget-timeout"), &[]),
        _ => {
            let args: Vec<_> = error
                .params
                .iter()
                .filter_map(|(k, v)| match &v.value {
                    Some(_) if error.code == "auth.forbidden" && k == "action" => Some((
                        k.as_str(),
                        Arg::Str(say.text(key!("tui-forget-action"), &[])),
                    )),
                    Some(Value::Str(s)) => Some((k.as_str(), Arg::Str(s.clone()))),
                    Some(Value::Int(i)) => Some((k.as_str(), Arg::Num(*i))),
                    None => None,
                })
                .collect();
            say.0
                .render(&error.code.replace('.', "-"), &args)
                .unwrap_or(error.code)
        }
    };
    // Error parameters come from the hub; they must not become terminal control sequences.
    plain(&say.text(key!("tui-forget-failed"), &[("reason", Arg::Str(reason))])).replace('\n', " ")
}

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
fn answer(
    say: &Say,
    session: &mut Session,
    snapshot: &Snapshot,
    selected: &str,
    approved: bool,
) -> String {
    let Some(device) = device() else {
        return say.text(
            key!("tui-answer-failed"),
            &[("code", Arg::Str("device".into()))],
        );
    };
    match session.resolve_approval(snapshot, &device, approved, Some(selected)) {
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

const GRANT_TTL: Duration = Duration::from_secs(60 * 60);

/// A machine refused until its owner allows it (ADR-0041). Asks, and on `y` signs a grant with this device's key. `Ok(true)` means it
/// was given and the action can be repeated; `Ok(false)` that the user said no; `Err` is the line to show.
fn ask_grant(
    say: &Say,
    session: &mut Session,
    rx: &mpsc::Receiver<u8>,
    node: &str,
    scope: &str,
    commands: &[&str],
) -> Result<bool, String> {
    let mut out = std::io::stdout();
    let _ = write!(out, "\x1b[H\x1b[2J{}", say.text(key!("tui-grant-ask"), &[]));
    let _ = out.flush();
    let end = Instant::now() + Duration::from_secs(30);
    let yes = loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(k) => break k == b'y',
            Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() < end => {}
            Err(_) => break false,
        }
    };
    if !yes {
        return Ok(false);
    }
    let fail = |code: String| say.text(key!("tui-answer-failed"), &[("code", Arg::Str(code))]);
    let device = device().ok_or_else(|| fail("device".into()))?;
    match session.grant(node, &device, scope, commands, GRANT_TTL) {
        Ok(()) => Ok(true),
        Err(code) if code == "approval.signature_invalid" => Err(say.text(
            key!("tui-not-trusted"),
            &[
                ("id", Arg::Str(device.id.clone())),
                ("key", Arg::Str(device.public_key_hex())),
            ],
        )),
        Err(code) => Err(fail(code)),
    }
}

/// What `action` came to once a refusal for a missing grant was handled.
enum Tried<T> {
    Done(Result<T, String>),
    /// The user did not allow it.
    Declined,
    /// Asking failed; the line to show.
    Line(String),
}

/// Runs `action`; when the machine wants a grant first, asks the user and, if given, runs it once more.
fn with_grant<T>(
    say: &Say,
    session: &mut Session,
    rx: &mpsc::Receiver<u8>,
    (node, scope, commands): (&str, &str, &[&str]),
    mut action: impl FnMut(&mut Session) -> Result<T, String>,
) -> Tried<T> {
    let first = action(session);
    if !first
        .as_ref()
        .is_err_and(|c| c == "approval.grant_required")
    {
        return Tried::Done(first);
    }
    match ask_grant(say, session, rx, node, scope, commands) {
        Ok(true) => Tried::Done(action(session)),
        Ok(false) => Tried::Declined,
        Err(line) => Tried::Line(line),
    }
}

/// What typing to one agent needs a grant for (what the web client asks for too).
const TYPING: [&str; 3] = ["agent.stop", "agent.prompt", "terminal.input"];

/// Ctrl-] : the key that leaves an opened terminal (the same one `agora terminal attach` uses).
const DETACH: u8 = 0x1d;

/// Shows an agent's terminal until the user detaches or the program ends. `Err` is the error code when it could not be opened.
/// `Ok(true)` means the link to the hub was lost while watching.
fn attach(
    say: &Say,
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
    let mut stream = session.terminal_open(node, &terminal)?;
    let mut size = agora_platform::terminal_size();
    if let Some((cols, rows)) = size {
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
        // What was typed was refused until the owner allows it: ask, and show the screen again either way (the typed keys are lost).
        if session.take_grant_wanted() {
            let scope = format!("agent:{local}");
            let _ = ask_grant(say, session, rx, node, &scope, &TYPING);
            session.terminal_close(stream);
            stream = session.terminal_open(node, &terminal)?;
            if let Some((cols, rows)) = size {
                session.terminal_resize(node, &terminal, cols, rows);
            }
            let _ = write!(out, "\x1b[2J\x1b[H");
            let _ = out.flush();
        }
        // A window that was resized is told to the machine, so the agent redraws for the new size.
        let now = agora_platform::terminal_size();
        if now != size {
            size = now;
            if let Some((cols, rows)) = now {
                session.terminal_resize(node, &terminal, cols, rows);
            }
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

/// The conversation of an agent that has no terminal, with a line to type the next request on. Ctrl-] leaves it. Same results as
/// [`attach`].
fn chat(
    say: &Say,
    session: &mut Session,
    agent: &str,
    rx: &mpsc::Receiver<u8>,
    color: bool,
    snapshot: Option<agora_protocol::v1::AgentSnapshot>,
) -> Result<bool, String> {
    let (node, local) = agent.split_once('/').unwrap_or(("", agent));
    let stream = session.chat_open(node, local)?;
    let text = |id: &str, args: &[(&str, Arg)]| say.text(id, args);
    let mut entries: Vec<ChatEntry> = Vec::new();
    let mut typed: Vec<u8> = Vec::new();
    let mut note = String::new();
    let mut lost = false;
    let mut dirty = true;
    // ADR-0058: the line that asked to confirm a mode that does not ask; typing it again switches.
    let mut confirming: Option<String> = None;
    'watch: loop {
        match session.chat_next(stream, Duration::from_millis(100)) {
            ChatEvent::Lines(lines, replace) => {
                if replace {
                    entries = lines;
                } else {
                    let last = entries.last().map_or(0, |e| e.entry_id);
                    entries.extend(lines.into_iter().filter(|e| e.entry_id > last));
                }
                dirty = true;
            }
            ChatEvent::Ended => break,
            ChatEvent::Lost => {
                lost = true;
                break;
            }
            ChatEvent::Quiet => {}
        }
        while let Ok(k) = rx.try_recv() {
            match k {
                DETACH => break 'watch,
                b'\r' | b'\n' => {
                    let line = String::from_utf8_lossy(&typed).trim().to_string();
                    typed.clear();
                    let asked = chat_command(&line, snapshot.as_ref(), confirming.as_deref());
                    confirming = None;
                    match asked {
                        ChatCommand::Prompt => {}
                        ChatCommand::Unknown { id, offered } => {
                            note = say.text(
                                key!("tui-option-unknown"),
                                &[("id", Arg::Str(id)), ("ids", Arg::Str(offered.join(", ")))],
                            );
                            dirty = true;
                            continue;
                        }
                        ChatCommand::Confirm { id } => {
                            note = say.text(key!("tui-option-confirm"), &[("id", Arg::Str(id))]);
                            confirming = Some(line);
                            dirty = true;
                            continue;
                        }
                        ChatCommand::Switch { model, id } => {
                            note = match session.set_option(node, local, model, &id) {
                                Ok(()) => say.text(key!("tui-option-switched"), &[]),
                                Err(code) => {
                                    say.text(key!("tui-send-failed"), &[("code", Arg::Str(code))])
                                }
                            };
                            dirty = true;
                            continue;
                        }
                    }
                    if !line.is_empty() {
                        let scope = format!("agent:{local}");
                        note = match with_grant(say, session, rx, (node, &scope, &TYPING), |s| {
                            s.prompt(node, local, &line)
                        }) {
                            Tried::Done(Ok(())) | Tried::Declined => String::new(),
                            Tried::Done(Err(code)) => {
                                say.text(key!("tui-send-failed"), &[("code", Arg::Str(code))])
                            }
                            Tried::Line(l) => l,
                        };
                    }
                    dirty = true;
                }
                0x7f | 0x08 => {
                    // Backspace removes one character, not one byte.
                    while typed.pop().is_some_and(|b| b & 0xC0 == 0x80) {}
                    dirty = true;
                }
                b if b >= 0x20 => {
                    typed.push(b);
                    dirty = true;
                }
                _ => {}
            }
        }
        if dirty {
            dirty = false;
            let size = agora_platform::terminal_size()
                .map_or((80, 24), |(c, r)| (usize::from(c), usize::from(r)));
            let mut lines = render_chat(
                &entries,
                local,
                &String::from_utf8_lossy(&typed),
                size,
                color,
                &text,
            );
            if !note.is_empty() {
                lines.insert(lines.len().saturating_sub(1), plain(&note));
            }
            // What the agent runs with, as it was when the conversation was opened (ADR-0058).
            if let Some(s) = snapshot.as_ref().and_then(|a| session_line(a, &text)) {
                lines.insert(0, plain(&s));
            }
            let mut out = std::io::stdout();
            let _ = write!(out, "\x1b[H\x1b[2J{}", lines.join("\r\n"));
            let _ = out.flush();
        }
    }
    session.terminal_close(stream);
    Ok(lost)
}

/// `s`: asks which agent to start on the machine the pointer is on (or the first online one with a project), then starts it in that
/// project. Returns the line to show.
fn start(
    say: &Say,
    session: &mut Session,
    snapshot: &Snapshot,
    selected: &str,
    rx: &mpsc::Receiver<u8>,
) -> String {
    let from_selected = snapshot.agents.iter().find(|a| a.agent_id == selected);
    let target = from_selected
        .map(|a| (a.node_id.clone(), a.project_id.clone()))
        .or_else(|| {
            snapshot
                .checkouts
                .iter()
                .find(|c| {
                    snapshot
                        .nodes
                        .iter()
                        .any(|n| n.node_id == c.node_id && n.online)
                })
                .map(|c| (c.node_id.clone(), c.project_id.clone()))
        });
    let Some((node, project)) = target else {
        return say.text(key!("tui-start-nowhere"), &[]);
    };
    let machine = [("machine", Arg::Str(node.clone()))];
    let kinds = match session.agent_kinds(&node) {
        Ok(k) if !k.is_empty() => k,
        Ok(_) => return say.text(key!("tui-start-none"), &machine),
        Err(code) => return say.text(key!("tui-start-failed"), &[("code", Arg::Str(code))]),
    };
    let kinds: Vec<_> = kinds.into_iter().take(9).collect();
    let mut menu = vec![say.text(key!("tui-start-pick"), &machine), String::new()];
    for (i, (_, name)) in kinds.iter().enumerate() {
        menu.push(say.text(
            key!("tui-start-item"),
            &[
                ("n", Arg::Num(i as i64 + 1)),
                ("name", Arg::Str(plain(name))),
            ],
        ));
    }
    let mut out = std::io::stdout();
    let _ = write!(out, "\x1b[H\x1b[2J{}", menu.join("\r\n"));
    let _ = out.flush();
    // Wait for one key; anything but a listed number cancels.
    let end = Instant::now() + Duration::from_secs(30);
    let pick = loop {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(k) => break Some(k),
            Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() < end => {}
            Err(_) => break None,
        }
    };
    let Some(index) = pick.and_then(|k| (b'1'..=b'9').contains(&k).then(|| usize::from(k - b'1')))
    else {
        return String::new();
    };
    let Some((kind, _)) = kinds.get(index) else {
        return String::new();
    };
    let scope = format!("project:{project}");
    match with_grant(say, session, rx, (&node, &scope, &["agent.start"]), |s| {
        s.start_agent(&node, &project, kind)
    }) {
        Tried::Done(Ok(agent)) => say.text(key!("tui-started"), &[("agent", Arg::Str(agent))]),
        Tried::Done(Err(code)) => say.text(key!("tui-start-failed"), &[("code", Arg::Str(code))]),
        Tried::Declined => String::new(),
        Tried::Line(l) => l,
    }
}

/// One key of the folder picker, `ESC [ x` sequences read as the arrows they are; `None` when nothing came in `wait`.
fn read_key(rx: &mpsc::Receiver<u8>, wait: Duration) -> Option<u8> {
    let k = rx.recv_timeout(wait).ok()?;
    if k != 0x1b {
        return Some(k);
    }
    match (
        rx.recv_timeout(Duration::from_millis(15)),
        rx.recv_timeout(Duration::from_millis(15)),
    ) {
        (Ok(b'['), Ok(b'A')) => Some(b'k'),
        (Ok(b'['), Ok(b'B')) => Some(b'j'),
        (Ok(b'['), Ok(b'D')) => Some(0x7f),
        (Ok(b'['), Ok(b'C')) => Some(b'\r'),
        _ => Some(0x1b),
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Start an agent in a folder the user picks on a machine (ADR-0053): the machine's folders through `directory.list`, then the chosen
/// one through `directory.inspect` and `agent.launch`, each signed by this terminal's key for exactly that folder (ADR-0052). Answers
/// the line to show; empty when the user cancelled.
#[allow(clippy::too_many_lines)] // one dialogue: browse, choose, confirm, launch
fn start_in_folder(
    say: &Say,
    session: &mut Session,
    snapshot: &Snapshot,
    selected: &str,
    rx: &mpsc::Receiver<u8>,
    color: bool,
) -> String {
    let Some(device) = device() else {
        return say.text(
            key!("tui-answer-failed"),
            &[("code", Arg::Str("device".into()))],
        );
    };
    let not_trusted = |device: &Device| {
        say.text(
            key!("tui-not-trusted"),
            &[
                ("id", Arg::Str(device.id.clone())),
                ("key", Arg::Str(device.public_key_hex())),
            ],
        )
    };
    let online: Vec<&agora_protocol::v1::NodeSnapshot> =
        snapshot.nodes.iter().filter(|n| n.online).collect();
    if online.is_empty() {
        return say.text(key!("tui-start-nowhere"), &[]);
    }
    // The selected agent's machine when it is online, else the first online one; `m` goes to the next.
    let mut at = snapshot
        .agents
        .iter()
        .find(|a| a.agent_id == selected)
        .and_then(|a| online.iter().position(|n| n.node_id == a.node_id))
        .unwrap_or(0);
    let mut path = String::new();
    let mut notice = String::new();
    let text = |id: &str, args: &[(&str, Arg)]| say.text(id, args);
    let chosen = 'browse: loop {
        let node = online[at];
        let name = if node.hostname.is_empty() {
            node.node_id.clone()
        } else {
            node.hostname.clone()
        };
        let Some(auth) =
            device.authorize(&node.node_id, "list", &path, "", "", false, false, now_ms())
        else {
            return say.text(
                key!("tui-answer-failed"),
                &[("code", Arg::Str("client.sign".into()))],
            );
        };
        let folder = match session.list_folder(&node.node_id, &path, auth) {
            Ok(f) => f,
            Err(code) if code == "approval.signature_invalid" => return not_trusted(&device),
            Err(code) if code == "protocol.capability_missing" => {
                return say.text(key!("tui-folder-unsupported"), &[])
            }
            Err(code) if !path.is_empty() => {
                // A folder that cannot be listed (gone, a secret one): back to the home folder, saying why.
                notice = say.text(key!("tui-folder-failed"), &[("code", Arg::Str(code))]);
                path.clear();
                continue;
            }
            Err(code) => return say.text(key!("tui-folder-failed"), &[("code", Arg::Str(code))]),
        };
        let mut pick = 0usize;
        loop {
            let (cols, rows) = agora_platform::terminal_size().unwrap_or((100, 30));
            let view = agora_tui::picker::Picker {
                machine: &name,
                folder: &folder,
                selected: pick,
                notice: &notice,
                color,
                cols,
                rows,
            };
            let mut out = std::io::stdout();
            let _ = out.write_all(agora_tui::picker::frame(&view, &text).as_bytes());
            let _ = out.flush();
            let Some(k) = read_key(rx, Duration::from_secs(120)) else {
                return String::new();
            };
            notice.clear();
            let count = folder.entries.len();
            match k {
                b'j' if count > 0 => pick = (pick + 1) % count,
                b'k' if count > 0 => pick = (pick + count - 1) % count,
                b'\r' | b'\n' => {
                    if let Some(e) = folder.entries.get(pick) {
                        path = agora_tui::picker::child(&folder.path, &e.name);
                        continue 'browse;
                    }
                }
                0x7f | 0x08 | b'h' => {
                    if !folder.parent.is_empty() {
                        path = folder.parent.clone();
                        continue 'browse;
                    }
                }
                b'm' if online.len() > 1 => {
                    at = (at + 1) % online.len();
                    path.clear();
                    continue 'browse;
                }
                b' ' | b'.' => break 'browse (node.node_id.clone(), folder.path.clone()),
                0x1b | b'q' | 3 => return String::new(),
                _ => {}
            }
        }
    };
    let (node, folder) = chosen;
    let Some(auth) = device.authorize(&node, "inspect", &folder, "", "", false, false, now_ms())
    else {
        return say.text(
            key!("tui-answer-failed"),
            &[("code", Arg::Str("client.sign".into()))],
        );
    };
    let info = match session.inspect_folder(&node, &folder, auth) {
        Ok(i) => i,
        Err(code) if code == "approval.signature_invalid" => return not_trusted(&device),
        Err(code) => return say.text(key!("tui-start-failed"), &[("code", Arg::Str(code))]),
    };
    // Which agent: the machine's installed kinds by number, as `s` asks.
    let machine = [("machine", Arg::Str(node.clone()))];
    let kinds = match session.agent_kinds(&node) {
        Ok(k) if !k.is_empty() => k,
        Ok(_) => return say.text(key!("tui-start-none"), &machine),
        Err(code) => return say.text(key!("tui-start-failed"), &[("code", Arg::Str(code))]),
    };
    let kinds: Vec<_> = kinds.into_iter().take(9).collect();
    let mut menu = vec![
        say.text(key!("tui-start-pick"), &machine),
        plain(&info.path),
        String::new(),
    ];
    for (i, (_, name)) in kinds.iter().enumerate() {
        menu.push(say.text(
            key!("tui-start-item"),
            &[
                ("n", Arg::Num(i as i64 + 1)),
                ("name", Arg::Str(plain(name))),
            ],
        ));
    }
    let mut out = std::io::stdout();
    let _ = write!(out, "\x1b[H\x1b[2J{}", menu.join("\r\n"));
    let _ = out.flush();
    let Some(index) = read_key(rx, Duration::from_secs(60))
        .and_then(|k| (b'1'..=b'9').contains(&k).then(|| usize::from(k - b'1')))
    else {
        return String::new();
    };
    let Some((kind, _)) = kinds.get(index) else {
        return String::new();
    };
    // A folder whose project changed since an agent last ran there needs the user's yes (ADR-0052).
    let accept = !info.previous_project_id.is_empty();
    if accept {
        let _ = write!(
            out,
            "\x1b[H\x1b[2J{}",
            say.text(key!("tui-folder-identity"), &[])
        );
        let _ = out.flush();
        if read_key(rx, Duration::from_secs(60)) != Some(b'y') {
            return String::new();
        }
    }
    let Some(auth) = device.authorize(
        &node,
        "launch",
        &info.path,
        kind,
        &info.project_id,
        false,
        accept,
        now_ms(),
    ) else {
        return say.text(
            key!("tui-answer-failed"),
            &[("code", Arg::Str("client.sign".into()))],
        );
    };
    match session.launch_in(
        &node,
        agora_protocol::v1::AgentLaunch {
            path: info.path.clone(),
            project_id: info.project_id.clone(),
            kind: kind.clone(),
            separate_worktree: false,
            authorization: Some(auth),
            accept_identity_change: accept,
        },
    ) {
        Ok(agent) => say.text(key!("tui-started"), &[("agent", Arg::Str(agent))]),
        Err(code) if code == "approval.signature_invalid" => not_trusted(&device),
        Err(code) => say.text(key!("tui-start-failed"), &[("code", Arg::Str(code))]),
    }
}

/// Keeps the panel's terminal on the selected agent: stops watching the old one and opens the new one when it has a terminal.
fn follow(
    session: &mut Session,
    snapshot: &Snapshot,
    selected: &str,
    preview: &mut Option<Preview>,
) {
    let wanted = snapshot.terminals.iter().any(|t| t.agent_id == selected);
    if wanted && preview.as_ref().is_some_and(|p| p.agent == selected) {
        return;
    }
    if let Some(p) = preview.take() {
        p.close(session);
    }
    if wanted {
        *preview = Preview::open(session, selected).ok();
    }
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
    // The console takes the whole window while it runs (the alternate screen) and gives the old one back when it ends.
    struct FullScreen(bool);
    impl Drop for FullScreen {
        fn drop(&mut self) {
            if self.0 {
                let mut out = std::io::stdout();
                let _ = write!(out, "\x1b[?25h\x1b[?1049l");
                let _ = out.flush();
            }
        }
    }
    let _full = FullScreen(keys);
    if keys {
        let _ = write!(std::io::stdout(), "\x1b[?1049h");
    }
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
    let mut cost = 0u64;
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
        // With a keyboard: machines and agents on the left, the selected agent's terminal on the right, a status line. Without one
        // (output piped): the plain list, as before.
        let draw = |snapshot: &Option<Snapshot>,
                    selected: &str,
                    notice: &str,
                    cost: u64,
                    preview: Option<&Preview>| {
            if let Some(s) = snapshot {
                let mut out = std::io::stdout().lock();
                if keys {
                    let (cols, rows) = agora_platform::terminal_size().unwrap_or((100, 30));
                    let view = View {
                        snapshot: s,
                        selected,
                        screen: preview.filter(|p| p.agent == selected).map(Preview::screen),
                        notice,
                        color,
                        cols,
                        rows,
                    };
                    let _ = out.write_all(frame(&view, &text).as_bytes());
                } else {
                    let lines = render(s, color, keys, selected, notice, cost, &text);
                    let _ = write!(out, "\x1b[H\x1b[2J{}{eol}", lines.join(eol));
                }
                let _ = out.flush();
            }
        };
        let mut preview: Option<Preview> = None;
        let mut last_draw = Instant::now();
        // A change that came too soon after the last frame is drawn on a later turn, never dropped.
        let mut pending = false;
        let mut window = agora_platform::terminal_size();
        loop {
            // Watching a terminal: wake often enough that its output looks live.
            let wait = if preview.is_some() { 30 } else { 250 };
            match session.next(Duration::from_millis(wait)) {
                Update::Snapshot(s) => {
                    snapshot = Some(*s);
                    cost = session.usage_total().unwrap_or(cost);
                    draw(&snapshot, &selected, &notice, cost, preview.as_ref());
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
            if keys {
                let mut redraw = pending || window != agora_platform::terminal_size();
                if let Some(s) = &snapshot {
                    // Something is always selected while there are agents, and the panel shows the selected one.
                    let order = tree_order(s);
                    if !order.contains(&selected) {
                        // First choice: an agent whose terminal can be shown.
                        selected = order
                            .iter()
                            .find(|a| s.terminals.iter().any(|t| &t.agent_id == *a))
                            .or(order.first())
                            .cloned()
                            .unwrap_or_default();
                        redraw = true;
                    }
                    follow(&mut session, s, &selected, &mut preview);
                }
                window = agora_platform::terminal_size();
                if let Some(p) = preview.as_mut() {
                    match p.pump(&mut session) {
                        Pumped::Nothing => {}
                        Pumped::Changed => redraw = true,
                        Pumped::Ended => {
                            preview = None;
                            redraw = true;
                        }
                        Pumped::Lost => link_lost = true,
                    }
                }
                // At most 25 frames a second, however fast the agent writes.
                pending = redraw && last_draw.elapsed() < Duration::from_millis(40);
                if redraw && !pending {
                    draw(&snapshot, &selected, &notice, cost, preview.as_ref());
                    last_draw = Instant::now();
                }
            }
            while let Ok(k) = rx.try_recv() {
                // Arrow keys and Shift-Tab arrive as `ESC [ x`.
                let k = if k == 0x1b {
                    match (
                        rx.recv_timeout(Duration::from_millis(15)),
                        rx.recv_timeout(Duration::from_millis(15)),
                    ) {
                        (Ok(b'['), Ok(b'A')) => b'k',
                        (Ok(b'['), Ok(b'B')) => b'j',
                        (Ok(b'['), Ok(b'Z')) => b'K',
                        _ => continue,
                    }
                } else {
                    k
                };
                match k {
                    b'\t' | b'j' | b'k' | b'K' if keys => {
                        if let Some(s) = &snapshot {
                            let order = tree_order(s);
                            let at = order.iter().position(|a| *a == selected);
                            let len = order.len().max(1);
                            let next = match (k, at) {
                                (b'\t' | b'j', Some(i)) => (i + 1) % len,
                                (_, Some(i)) => (i + len - 1) % len,
                                (_, None) => 0,
                            };
                            selected = order.get(next).cloned().unwrap_or_default();
                            follow(&mut session, s, &selected, &mut preview);
                            draw(&snapshot, &selected, &notice, cost, preview.as_ref());
                            last_draw = Instant::now();
                        }
                    }
                    3 | b'q' => return ExitCode::SUCCESS, // Ctrl-C is a plain byte in raw mode
                    b'y' | b'n' => {
                        if let Some(s) = &snapshot {
                            notice = answer(say, &mut session, s, &selected, k == b'y');
                            dirty_since.get_or_insert_with(Instant::now);
                            draw(&snapshot, &selected, &notice, cost, preview.as_ref());
                        }
                    }
                    b'f' => {
                        if let Some(sn) = &snapshot {
                            // The picker reads keys itself; the panel's terminal is let go meanwhile.
                            if let Some(p) = preview.take() {
                                p.close(&mut session);
                            }
                            notice = start_in_folder(say, &mut session, sn, &selected, &rx, color);
                            dirty_since.get_or_insert_with(Instant::now);
                            draw(&snapshot, &selected, &notice, cost, preview.as_ref());
                        }
                    }
                    b's' => {
                        if let Some(sn) = &snapshot {
                            notice = start(say, &mut session, sn, &selected, &rx);
                            dirty_since.get_or_insert_with(Instant::now);
                            draw(&snapshot, &selected, &notice, cost, preview.as_ref());
                        }
                    }
                    b'd' => {
                        // A closed agent leaves every client's list; a running one is stopped first (x).
                        let running = snapshot.as_ref().is_some_and(|s| {
                            s.agents.iter().any(|a| a.agent_id == selected && a.running)
                        });
                        if !selected.is_empty() {
                            notice = if running {
                                say.text(key!("tui-forget-running"), &[])
                            } else {
                                forget_notice(say, session.forget_agent(&selected))
                            };
                            dirty_since.get_or_insert_with(Instant::now);
                            draw(&snapshot, &selected, &notice, cost, preview.as_ref());
                        }
                    }
                    b'x' => {
                        if !selected.is_empty() {
                            let (node, local) = selected.split_once('/').unwrap_or(("", &selected));
                            let scope = format!("agent:{local}");
                            notice = match with_grant(
                                say,
                                &mut session,
                                &rx,
                                (node, &scope, &TYPING),
                                |s| s.stop_agent(node, local),
                            ) {
                                Tried::Done(Ok(())) => say.text(key!("tui-stopped"), &[]),
                                Tried::Done(Err(code)) => {
                                    say.text(key!("tui-stop-failed"), &[("code", Arg::Str(code))])
                                }
                                Tried::Declined => String::new(),
                                Tried::Line(l) => l,
                            };
                            dirty_since.get_or_insert_with(Instant::now);
                            draw(&snapshot, &selected, &notice, cost, preview.as_ref());
                        }
                    }
                    b'\r' | b'\n' => {
                        if let (Some(s), false) = (&snapshot, selected.is_empty()) {
                            // The full-screen view reads the same terminal: stop watching it here first.
                            if let Some(p) = preview.take() {
                                p.close(&mut session);
                            }
                            let has_terminal = s.terminals.iter().any(|t| t.agent_id == selected);
                            let opened = if has_terminal {
                                attach(say, &mut session, s, &selected, &rx)
                            } else {
                                let agent =
                                    s.agents.iter().find(|a| a.agent_id == selected).cloned();
                                chat(say, &mut session, &selected, &rx, color, agent)
                            };
                            match opened {
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
                            draw(&snapshot, &selected, &notice, cost, preview.as_ref());
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

#[cfg(test)]
mod forget_tests {
    use super::*;
    use agora_i18n::Catalog;
    use agora_protocol::error::{Error, ErrorCode};

    #[test]
    fn a_remote_refusal_explains_the_reason_in_the_selected_language() {
        let running = Error::new(ErrorCode::AgentRunning)
            .with("agent_id", "laptop/agent-1")
            .to_info();
        let tr = Say(Catalog::turkish());
        assert_eq!(
            forget_notice(&tr, Err(running.clone())),
            "Kaldırılamadı: laptop/agent-1 hâlâ çalışıyor. Önce durdurun."
        );
        let en = Say(Catalog::english());
        let english = forget_notice(&en, Err(running));
        assert!(english.contains("laptop/agent-1"));
        assert!(english.contains("Stop it first"));
        let denied = Error::new(ErrorCode::AuthForbidden)
            .with("action", "AgentForget")
            .to_info();
        assert_eq!(
            forget_notice(&tr, Err(denied)),
            "Kaldırılamadı: Bunu yapmaya izniniz yok (ajanı kaldırma)."
        );
    }

    #[test]
    fn unknown_codes_and_transport_failures_remain_useful_without_control_sequences() {
        let tr = Say(Catalog::turkish());
        let error = |code: &str| ErrorInfo {
            code: code.into(),
            ..Default::default()
        };
        assert_eq!(forget_notice(&tr, Ok(())), "Listeden kaldırıldı.");
        assert_eq!(
            forget_notice(&tr, Err(error("agent.new_refusal"))),
            "Kaldırılamadı: agent.new_refusal"
        );
        assert_eq!(
            forget_notice(&tr, Err(error("client.unreachable"))),
            "Kaldırılamadı: Bağlantı kesildi. Yeniden bağlanmasını bekleyin."
        );
        assert_eq!(
            forget_notice(&tr, Err(error("node.timeout"))),
            "Kaldırılamadı: Zamanında yanıt alınamadı. Tekrar deneyin."
        );
        let untrusted = Error::new(ErrorCode::AgentRunning)
            .with("agent_id", "laptop/\u{1b}[2Jagent\n\u{202e}-1")
            .to_info();
        let line = forget_notice(&tr, Err(untrusted));
        assert!(!line.contains(['\u{1b}', '\n', '\u{202e}']));
    }
}

//! `agora tui`: the terminal console (ADR-0043). Slice 1 shows machines, agents and what waits for the user; slice 2 answers the
//! first waiting approval from the keyboard (`y` allow, `n` deny) with this terminal's own key (ADR-0028); slice 3 opens an agent's
//! terminal through the hub (`Tab` picks the agent, `Enter` opens it, Ctrl-] comes back).

use std::io::{IsTerminal, Read, Write};
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use agora_i18n::{key, Arg};
use agora_protocol::v1::{param_value::Value, ChatEntry, ErrorInfo, Snapshot};
use agora_tui::app::{Action, App, Area, Ctx, Event as AppEvent, Row, Screen as AppScreen};
use agora_tui::client::{sign_in, ChatEvent, ClientError, Hub, Session, TermEvent, Update};
use agora_tui::device::Device;
use agora_tui::input::Parser;
use agora_tui::pane::ColorMode;
use agora_tui::preview::{Preview, Pumped};
use agora_tui::render::{chat_command, plain, render, render_chat, session_line, ChatCommand};
use agora_tui::split::{app_details, app_rows};
use agora_tui::split::{frame, reader_frame, start_card_fit, start_card_lines, tree_order, View};

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

/// The first press of `u` or `U`: which key, for which agent, when.
type Armed = (u8, String, Instant);

/// How long the second press of `u` / `U` may come after the first.
const CONFIRM_WITHIN: Duration = Duration::from_secs(5);

#[derive(Debug, PartialEq, Eq)]
enum ResumeKey {
    NotInterrupted,
    /// `u` on an agent that cannot continue its own conversation: it never quietly starts a new one.
    CannotResume,
    /// Its process from before the restart still runs: `x` stops it first.
    StillRunning,
    /// First press: the same key again within [`CONFIRM_WITHIN`] does it.
    Confirm {
        fresh: bool,
    },
    Go {
        fresh: bool,
    },
}

/// ADR-0060: what `u` (Resume) or `U` (Start again) does now for the selected agent. Each needs the same key twice within five seconds;
/// another key, another agent or a later press starts over.
fn resume_key(
    snapshot: &Snapshot,
    selected: &str,
    key: u8,
    armed: &mut Option<Armed>,
    now: Instant,
) -> ResumeKey {
    let fresh = key == b'U';
    let Some(a) = snapshot
        .agents
        .iter()
        .find(|a| a.agent_id == selected && a.interrupted && !a.running)
    else {
        *armed = None;
        return ResumeKey::NotInterrupted;
    };
    if a.still_running {
        *armed = None;
        return ResumeKey::StillRunning;
    }
    if !fresh && !a.resumable {
        *armed = None;
        return ResumeKey::CannotResume;
    }
    let again = armed.take().is_some_and(|(k, id, at)| {
        k == key && id == selected && now.duration_since(at) <= CONFIRM_WITHIN
    });
    if again {
        ResumeKey::Go { fresh }
    } else {
        *armed = Some((key, selected.to_string(), now));
        ResumeKey::Confirm { fresh }
    }
}

/// ADR-0060: Resume (or, with `fresh`, Start again) the selected agent, which its machine reported interrupted. Signed by this
/// terminal's key for exactly that agent and choice.
fn resume(
    say: &Say,
    session: &mut Session,
    snapshot: &Snapshot,
    selected: &str,
    fresh: bool,
) -> String {
    let Some(a) = snapshot
        .agents
        .iter()
        .find(|a| a.agent_id == selected && a.interrupted && !a.running)
    else {
        return say.text(key!("tui-resume-not-interrupted"), &[]);
    };
    if !fresh && !a.resumable {
        return say.text(key!("tui-resume-cannot"), &[]);
    }
    let (node, local) = selected.split_once('/').unwrap_or(("", selected));
    let fail = |code: &str| {
        say.text(
            key!("tui-answer-failed"),
            &[("code", Arg::Str(code.into()))],
        )
    };
    let Some(device) = device() else {
        return fail("device");
    };
    let Some(auth) = device.authorize(node, "resume", local, "", "", fresh, false, now_ms()) else {
        return fail("client.sign");
    };
    let asked = agora_protocol::v1::AgentResume {
        agent_id: local.to_string(),
        new_session: fresh,
        authorization: Some(auth),
    };
    match session.resume_in(node, asked) {
        Ok(agent) => say.text(key!("tui-resumed"), &[("agent", Arg::Str(agent))]),
        Err(e) if e.code == "approval.signature_invalid" => say.text(
            key!("tui-not-trusted"),
            &[
                ("id", Arg::Str(device.id.clone())),
                ("key", Arg::Str(device.public_key_hex())),
            ],
        ),
        Err(e) => resume_notice(say, &e),
    }
}

/// Why a resume did not happen, in the selected language (the node's code and parameters, rendered from `errors.ftl`).
fn resume_notice(say: &Say, error: &ErrorInfo) -> String {
    let args: Vec<_> = error
        .params
        .iter()
        .filter_map(|(k, v)| match &v.value {
            Some(Value::Str(s)) => Some((k.as_str(), Arg::Str(s.clone()))),
            Some(Value::Int(i)) => Some((k.as_str(), Arg::Num(*i))),
            None => None,
        })
        .collect();
    let reason = say
        .0
        .render(&error.code.replace('.', "-"), &args)
        .unwrap_or_else(|| error.code.clone());
    // Parameters come from the node; they must not become terminal control sequences.
    plain(&say.text(key!("tui-resume-failed"), &[("reason", Arg::Str(reason))])).replace('\n', " ")
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

/// The name this terminal registers under (ADR-0061): what the person sees beside its fingerprint on their other devices.
fn device_name(say: &Say) -> String {
    let host = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_default();
    let name = say.text(key!("tui-device-name"), &[("host", Arg::Str(host))]);
    name.chars().filter(|c| !c.is_control()).take(64).collect()
}

/// `a`: asks the person's other devices to approve this terminal (ADR-0061). Returns the line to show.
fn ask_approval(say: &Say, session: &mut Session) -> String {
    let Some(device) = device() else {
        return say.text(
            key!("tui-answer-failed"),
            &[("code", Arg::Str("device".into()))],
        );
    };
    match session.ask_approval(&device) {
        Ok(()) => say.text(
            key!("tui-device-asked"),
            &[("fingerprint", Arg::Str(device.fingerprint()))],
        ),
        Err(code) => say.text(key!("tui-answer-failed"), &[("code", Arg::Str(code))]),
    }
}

/// `v`: the first device of the person that waits for approval, shown with its fingerprint and the machines that trust this terminal
/// (where the approval counts); `y` approves it with this terminal's key (ADR-0061). Returns the line to show.
fn approve_device(
    say: &Say,
    session: &mut Session,
    snapshot: Option<&Snapshot>,
    rx: &mpsc::Receiver<u8>,
) -> String {
    let fail = |code: String| say.text(key!("tui-answer-failed"), &[("code", Arg::Str(code))]);
    let Some(device) = device() else {
        return fail("device".into());
    };
    let list = match session.list_devices() {
        Ok(l) => l,
        Err(code) => return fail(code),
    };
    let Some(waiting) = list.into_iter().find(|d| {
        d.approval_asked_ms > 0
                && d.revoked_ms == 0
                && d.device_id != device.id
                // Only a device that registered itself with its own key: the hub cannot make up a request.
                && agora_node::device_trust::registration_verifies(d)
    }) else {
        return say.text(
            key!("tui-devices-none-waiting"),
            &[("fingerprint", Arg::Str(device.fingerprint()))],
        );
    };
    let machines: Vec<String> = snapshot
        .map(|s| {
            s.nodes
                .iter()
                .filter(|n| n.trusted_device_ids.contains(&device.id))
                .map(|n| {
                    plain(if n.hostname.is_empty() {
                        &n.node_id
                    } else {
                        &n.hostname
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    if machines.is_empty() {
        return say.text(key!("tui-device-cannot-vouch"), &[]);
    }
    // The name and the fingerprint come from the hub: shown as plain text, never as terminal control sequences.
    let name = agora_node::device_trust::shown(&plain(&waiting.name), 40);
    let fingerprint = agora_tui::device::fingerprint(&waiting.public_key);
    let mut out = std::io::stdout();
    let _ = write!(
        out,
        "\x1b[H\x1b[2J{}",
        say.text(
            key!("tui-device-card"),
            &[
                ("name", Arg::Str(name.clone())),
                ("fingerprint", Arg::Str(fingerprint)),
                ("machines", Arg::Str(machines.join(", "))),
            ],
        )
        .replace('\n', "\r\n")
    );
    let _ = out.flush();
    if read_key(rx, Duration::from_secs(120)) != Some(b'y') {
        return String::new();
    }
    match session.endorse_device(&device, &waiting) {
        Ok(()) => say.text(key!("tui-device-approved"), &[("name", Arg::Str(name))]),
        Err(code) => fail(code),
    }
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
                ("fingerprint", Arg::Str(device.fingerprint())),
            ],
        ),
        Err(code) => say.text(key!("tui-answer-failed"), &[("code", Arg::Str(code))]),
    }
}

/// The `agent.start` request y would answer when it does not fit the split view at the terminal's size now and was not read whole in
/// the reader (ADR-0057): y must not allow it yet.
fn start_unread(
    s: &Snapshot,
    selected: &str,
    color: bool,
    read_whole: &std::collections::HashSet<String>,
    text: agora_tui::render::Text<'_>,
    (cols, rows): (u16, u16),
) -> Option<agora_protocol::v1::ApprovalSnapshot> {
    let view = View {
        snapshot: s,
        selected,
        screen: None,
        notice: "",
        color,
        cols,
        rows,
        now_ms: now_ms(),
    };
    match start_card_fit(&view, text) {
        Some((a, false)) if !read_whole.contains(&a.approval_id) => Some(a.clone()),
        _ => None,
    }
}

/// The full-screen reader of a start request (ADR-0057). Returns whether its last line was in view before the reader was left.
fn read_start(
    a: &agora_protocol::v1::ApprovalSnapshot,
    rx: &mpsc::Receiver<u8>,
    text: agora_tui::render::Text<'_>,
) -> bool {
    let mut top = 0usize;
    let mut seen_end = false;
    loop {
        let (cols, rows) = agora_platform::terminal_size().unwrap_or((100, 30));
        let Some(lines) = start_card_lines(a, usize::from(cols).saturating_sub(3), text) else {
            return false;
        };
        let room = usize::from(rows.max(3)) - 1;
        top = top.min(lines.len().saturating_sub(room));
        let (out, end) = reader_frame(&lines, top, cols, rows, text);
        seen_end |= end;
        let mut o = std::io::stdout();
        let _ = o.write_all(out.as_bytes());
        let _ = o.flush();
        // Arrow keys come as j / k (`read_key`); Esc as itself.
        let Some(k) = read_key(rx, Duration::from_secs(300)) else {
            return seen_end;
        };
        match k {
            b'j' => top += 1,
            b'k' => top = top.saturating_sub(1),
            b' ' => top += room,
            b'q' | 3 | 0x1b => return seen_end,
            _ => {}
        }
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
                ("fingerprint", Arg::Str(device.fingerprint())),
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
    mut snapshot: Option<agora_protocol::v1::AgentSnapshot>,
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
                            // The same per-agent grant the web client asks for (its `grantFor`): this one switch for this agent.
                            let scope = format!("agent:{local}");
                            let command = if model {
                                "agent.set_model"
                            } else {
                                "agent.set_mode"
                            };
                            note = match with_grant(
                                say,
                                session,
                                rx,
                                (node, &scope, &[command]),
                                |s| s.set_option(node, local, model, &id),
                            ) {
                                Tried::Done(Ok(())) => {
                                    if let Some(a) = snapshot.as_mut() {
                                        agora_tui::render::switched(a, model, &id);
                                    }
                                    say.text(key!("tui-option-switched"), &[])
                                }
                                Tried::Done(Err(code)) => {
                                    say.text(key!("tui-send-failed"), &[("code", Arg::Str(code))])
                                }
                                Tried::Declined => String::new(),
                                Tried::Line(l) => l,
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
            // What the agent runs with: as it was when the conversation was opened, and every switch made here since (ADR-0058).
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
    let (kinds, missing) = match session.agent_kinds_all(&node) {
        Ok(all) => split_kinds(say, all),
        Err(code) => return say.text(key!("tui-start-failed"), &[("code", Arg::Str(code))]),
    };
    if kinds.is_empty() {
        let none = say.text(key!("tui-start-none"), &machine);
        return missing.map_or(none.clone(), |m| format!("{none} {m}"));
    }
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
    if let Some(m) = missing {
        menu.push(String::new());
        menu.push(m);
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
                ("fingerprint", Arg::Str(device.fingerprint())),
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
    let (kinds, missing) = match session.agent_kinds_all(&node) {
        Ok(all) => split_kinds(say, all),
        Err(code) => return say.text(key!("tui-start-failed"), &[("code", Arg::Str(code))]),
    };
    if kinds.is_empty() {
        let none = say.text(key!("tui-start-none"), &machine);
        return missing.map_or(none.clone(), |m| format!("{none} {m}"));
    }
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
    if let Some(m) = missing {
        menu.push(String::new());
        menu.push(m);
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
    // ADR-0059: how it acts. "Ask before acting" is the default (Enter, 1, or any key that is not an option); an option that lets the
    // agent act without asking is used only after a second question whose default keeps asking.
    let offered = session.kind_options(&node, kind).unwrap_or_default();
    let skips: Vec<_> = offered
        .iter()
        .filter(|o| o.skips_permissions)
        .take(8)
        .collect();
    let mut launch_options = Vec::new();
    // Only what this window itself asked about and the user accepted (a separate signed list, never part of an id).
    let mut confirmed_launch_options = Vec::new();
    if !skips.is_empty() {
        let label = |o: &agora_protocol::v1::LaunchOption| {
            let said = say.text(&o.label, &[]);
            plain(if said == o.label { &o.id } else { &said })
        };
        let mut menu = vec![say.text(key!("tui-start-acts"), &[]), String::new()];
        menu.push(say.text(
            key!("tui-start-item"),
            &[
                ("n", Arg::Num(1)),
                ("name", Arg::Str(say.text(key!("option-ask"), &[]))),
            ],
        ));
        for (i, o) in skips.iter().enumerate() {
            menu.push(say.text(
                key!("tui-start-item"),
                &[("n", Arg::Num(i as i64 + 2)), ("name", Arg::Str(label(o)))],
            ));
        }
        let _ = write!(out, "\x1b[H\x1b[2J{}", menu.join("\r\n"));
        let _ = out.flush();
        let Some(k) = read_key(rx, Duration::from_secs(60)) else {
            return String::new();
        };
        if matches!(k, 0x1b | b'q' | 3) {
            return String::new();
        }
        let picked = (b'2'..=b'9')
            .contains(&k)
            .then(|| skips.get(usize::from(k - b'2')))
            .flatten();
        if let Some(o) = picked {
            let _ = write!(
                out,
                "\x1b[H\x1b[2J{}",
                say.text(
                    key!("tui-start-confirm-skip"),
                    &[("name", Arg::Str(label(o)))]
                )
            );
            let _ = out.flush();
            // Confirmed here: signed with the start; the node refuses an option that skips permissions without it.
            if read_key(rx, Duration::from_secs(60)) == Some(b'y') {
                launch_options.push(o.id.clone());
                confirmed_launch_options.push(o.id.clone());
            }
        }
    }
    let Some(auth) = device.authorize_launch(
        &node,
        &info.path,
        kind,
        &info.project_id,
        false,
        accept,
        &launch_options,
        &confirmed_launch_options,
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
            launch_options,
            confirmed_launch_options,
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
    // On the hub's machine (no `--hub`, or `--hub-dir` given with it: the front door's hub on a port of its own, ADR-0062) a sign-in
    // code is made from the hub's folder whenever one is needed; nobody types one.
    let local = o.hub.is_none() || o.hub_dir.is_some();
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
    // ADR-0063: with a keyboard the console is the app shell, drawn with ratatui on the alternate screen (left again when it is
    // dropped); without one (output piped) the plain list, as before.
    let mut console_state = if keys {
        match agora_tui::backend::Screen::enter() {
            Ok(window) => Some(Console {
                window,
                app: App::default(),
                parser: Parser::default(),
                snapshot: None,
                selected: String::new(),
                notice: String::new(),
                armed: None,
                read_whole: std::collections::HashSet::new(),
                colors: if color {
                    ColorMode::from_env(None, std::env::var("COLORTERM").ok().as_deref())
                } else {
                    ColorMode::Mono
                },
            }),
            Err(_) => None,
        }
    } else {
        None
    };
    let _full = FullScreen(keys && console_state.is_none());
    if keys && console_state.is_none() {
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
    // ADR-0060: the first press of u / U, waiting for the same key again (see `resume_key`).
    let mut armed: Option<Armed> = None;
    loop {
        if first {
            say.err(key!("tui-connecting"), &hub_arg);
        }
        let session = (|| {
            let t = match &token {
                Some(t) => t.clone(),
                None => {
                    // A code from the flag the first time; on the hub's machine a new one whenever it is needed.
                    let code = match (&o.code, local) {
                        (Some(c), _) if first => c.clone(),
                        (_, true) => new_login_code(&dir).map_err(|_| ClientError::Refused)?,
                        _ => return Err(ClientError::Refused),
                    };
                    sign_in(&hub, &code)?
                }
            };
            let mut s = Session::connect(&hub, &t)?;
            token = Some(t);
            // ADR-0061: this terminal's key is registered with the hub at every sign-in (a candidate, never trust by itself).
            if let Some(d) = device() {
                let _ = s.register_device(&d, &device_name(say));
            }
            Ok(s)
        })();
        let mut session = match session {
            Ok(s) => s,
            Err(ClientError::Refused | ClientError::TooMany) => {
                token = None;
                if first || !local {
                    // On the hub's machine nobody is asked for a code: the client makes its own.
                    if local {
                        say.err(key!("tui-signin-failed-local"), &[]);
                    } else {
                        say.err(key!("tui-signin-failed"), &[]);
                    }
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
        if let Some(c) = console_state.as_mut() {
            match console(say, &mut session, &rx, &hub_arg, color, c) {
                Next::Quit => return ExitCode::SUCCESS,
                Next::Reconnect => continue,
            }
        }
        let mut dirty_since: Option<Instant> = None;
        // ADR-0057: the `agent.start` requests read whole in the reader, which y may then allow even where the panel cannot show them.
        let mut read_whole: std::collections::HashSet<String> = std::collections::HashSet::new();
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
                        now_ms: now_ms(),
                    };
                    let _ = out.write_all(frame(&view, &text).as_bytes());
                } else {
                    let lines = render(s, color, keys, selected, notice, cost, now_ms(), &text);
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
                            // A start request is allowed only once all of it was in view: in the panel, or in the reader.
                            if k == b'y'
                                && start_unread(
                                    s,
                                    &selected,
                                    color,
                                    &read_whole,
                                    &text,
                                    agora_platform::terminal_size().unwrap_or((100, 30)),
                                )
                                .is_some()
                            {
                                notice = say.text(key!("tui-split-read-first"), &[]);
                                draw(&snapshot, &selected, &notice, cost, preview.as_ref());
                                continue;
                            }
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
                    b'a' => {
                        notice = ask_approval(say, &mut session);
                        draw(&snapshot, &selected, &notice, cost, preview.as_ref());
                    }
                    b'v' => {
                        if let Some(p) = preview.take() {
                            p.close(&mut session);
                        }
                        notice = approve_device(say, &mut session, snapshot.as_ref(), &rx);
                        dirty_since.get_or_insert_with(Instant::now);
                        draw(&snapshot, &selected, &notice, cost, preview.as_ref());
                    }
                    b's' => {
                        if let Some(sn) = &snapshot {
                            notice = start(say, &mut session, sn, &selected, &rx);
                            dirty_since.get_or_insert_with(Instant::now);
                            draw(&snapshot, &selected, &notice, cost, preview.as_ref());
                        }
                    }
                    // ADR-0060: an agent that stopped with its machine comes back (u) or starts again in its folder (U).
                    b'u' | b'U' => {
                        if let Some(sn) = &snapshot {
                            let local = selected
                                .split_once('/')
                                .map_or(selected.as_str(), |(_, l)| l);
                            let agent = [("agent", Arg::Str(local.to_string()))];
                            notice = match resume_key(sn, &selected, k, &mut armed, Instant::now())
                            {
                                ResumeKey::NotInterrupted => {
                                    say.text(key!("tui-resume-not-interrupted"), &[])
                                }
                                ResumeKey::CannotResume => say.text(key!("tui-resume-cannot"), &[]),
                                ResumeKey::StillRunning => {
                                    say.text(key!("tui-resume-still-running"), &[])
                                }
                                ResumeKey::Confirm { fresh: false } => {
                                    say.text(key!("tui-resume-confirm"), &agent)
                                }
                                ResumeKey::Confirm { fresh: true } => {
                                    say.text(key!("tui-again-confirm"), &agent)
                                }
                                ResumeKey::Go { fresh } => {
                                    resume(say, &mut session, sn, &selected, fresh)
                                }
                            };
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
                            // ADR-0057: an agent that started helpers asks whether they stop too; yes unless the user says no.
                            let helpers = snapshot.as_ref().map_or(0, |s| {
                                s.agents
                                    .iter()
                                    .filter(|a| a.running && a.parent_agent_id == selected)
                                    .count()
                            });
                            let with_helpers = helpers == 0 || {
                                let mut out = std::io::stdout();
                                let _ = write!(
                                    out,
                                    "\x1b[H\x1b[2J{}",
                                    say.text(
                                        key!("tui-stop-helpers"),
                                        &[("count", Arg::Num(helpers as i64))]
                                    )
                                );
                                let _ = out.flush();
                                !matches!(rx.recv_timeout(Duration::from_secs(30)), Ok(b'n'))
                            };
                            notice = match with_grant(
                                say,
                                &mut session,
                                &rx,
                                (node, &scope, &TYPING),
                                |s| s.stop_agent_with(node, local, helpers > 0 && with_helpers),
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
                    b'\r' | b'\n'
                        if snapshot
                            .as_ref()
                            .and_then(|s| {
                                start_unread(
                                    s,
                                    &selected,
                                    color,
                                    &read_whole,
                                    &text,
                                    agora_platform::terminal_size().unwrap_or((100, 30)),
                                )
                            })
                            .is_some() =>
                    {
                        // ADR-0057: Enter reads a start request that does not fit the panel, before anything else.
                        if let Some(a) = snapshot.as_ref().and_then(|s| {
                            start_unread(
                                s,
                                &selected,
                                color,
                                &read_whole,
                                &text,
                                agora_platform::terminal_size().unwrap_or((100, 30)),
                            )
                        }) {
                            if read_start(&a, &rx, &text) {
                                read_whole.insert(a.approval_id.clone());
                            }
                        }
                        notice = String::new();
                        draw(&snapshot, &selected, &notice, cost, preview.as_ref());
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

/// What the console does after its loop ends.
enum Next {
    Quit,
    /// The link was lost or a fresh snapshot is due: connect again and come back.
    Reconnect,
}

/// The console's state that outlives one hub session.
struct Console {
    window: agora_tui::backend::Window,
    app: App,
    parser: Parser,
    snapshot: Option<Snapshot>,
    /// The selected agent, by hub-wide id (the tree's rows move as agents come and go).
    selected: String,
    notice: String,
    armed: Option<Armed>,
    /// ADR-0057: start requests read whole (in the inspector or the reader), which y may then allow.
    read_whole: std::collections::HashSet<String>,
    colors: ColorMode,
}

/// The terminal reports mouse presses, drags and the wheel (SGR encoding) while the console holds the mouse, and pastes come
/// bracketed; both are switched off for the full-screen questions, which read plain bytes.
fn mouse_mode(on: bool) {
    let mut out = std::io::stdout();
    let _ = out.write_all(if on {
        b"\x1b[?1000h\x1b[?1002h\x1b[?1006h\x1b[?2004h"
    } else {
        b"\x1b[?1000l\x1b[?1002l\x1b[?1006l\x1b[?2004l"
    });
    let _ = out.flush();
}

fn window_area() -> Area {
    let (cols, rows) = agora_platform::terminal_size().unwrap_or((100, 30));
    Area::new(0, 0, cols, rows)
}

/// ADR-0063: the console on the app shell. The tree (machines, agents), the open agents as tabs with their live terminals (typing goes
/// to the agent; Ctrl-] or F12 back), the inspector, and the letters of the earlier console for the selected agent (y / n answer, s
/// start, f folder, x stop, u / U resume, d remove), whose questions still take the whole screen for a moment.
fn console(
    say: &Say,
    session: &mut Session,
    rx: &mpsc::Receiver<u8>,
    hub_arg: &[(&str, Arg)],
    color: bool,
    c: &mut Console,
) -> Next {
    let text = |id: &str, args: &[(&str, Arg)]| say.text(id, args);
    let mut dirty_since: Option<Instant> = None;
    let mut last_refresh = Instant::now();
    let mut preview: Option<Preview> = None;
    let mut redraw = true;
    let mut last_draw = Instant::now() - Duration::from_secs(1);
    let mut area = window_area();
    // The agent whose terminal was last sized to the pane, so a tab brought to the front is sized once.
    let mut sized: Option<(String, u16, u16)> = None;
    mouse_mode(c.app.mouse);
    let mut fresh = true;
    loop {
        // Watching a terminal: wake often, so typing echoes and output looks live.
        let wait = if preview.is_some() { 15 } else { 50 };
        match session.next(Duration::from_millis(wait)) {
            Update::Snapshot(s) => {
                c.snapshot = Some(*s);
                dirty_since = None;
                redraw = true;
            }
            Update::Changed => {
                dirty_since.get_or_insert_with(Instant::now);
            }
            Update::Quiet => {}
            Update::Closed => {
                say.err(key!("tui-lost"), hub_arg);
                std::thread::sleep(Duration::from_secs(2));
                return Next::Reconnect;
            }
        }
        let rows = c
            .snapshot
            .as_ref()
            .map(|s| app_rows(s, &text))
            .unwrap_or_default();
        if let Some(i) = rows.iter().position(|r| !r.header && r.id == c.selected) {
            c.app.selected = i;
        }
        let mut events = Vec::new();
        let now = window_area();
        if now != area {
            area = now;
            let _ = c.window.resized();
            events.push(AppEvent::Resize {
                cols: area.width,
                rows: area.height,
            });
        }
        let mut bytes = Vec::new();
        while let Ok(b) = rx.try_recv() {
            bytes.push(b);
        }
        events.extend(c.parser.push(&bytes));
        if bytes.is_empty() {
            events.extend(c.parser.flush());
        }
        for ev in events {
            redraw = true;
            let modes = preview
                .as_ref()
                .map(|p| p.screen().modes())
                .unwrap_or_default();
            let acts = c.app.handle(
                ev,
                &Ctx {
                    rows: &rows,
                    area,
                    modes,
                    t: &text,
                },
            );
            c.selected = rows
                .get(c.app.selected)
                .filter(|r| !r.header)
                .map(|r| r.id.clone())
                .unwrap_or_default();
            for act in acts {
                match act {
                    Action::Input(b) => {
                        if let Some((node, local)) =
                            c.app.active_agent().and_then(|a| a.split_once('/'))
                        {
                            session.terminal_input(node, &format!("t-{local}"), &b);
                        }
                    }
                    Action::PaneSize { .. } => sized = None,
                    Action::MouseCapture(on) => mouse_mode(on),
                    Action::Open(_) | Action::Close(_) | Action::Scroll(_) => {}
                    Action::Quit => {
                        if let Some(p) = preview.take() {
                            p.close(session);
                        }
                        mouse_mode(false);
                        return Next::Quit;
                    }
                    Action::Chat(id) => {
                        let agent = c
                            .snapshot
                            .as_ref()
                            .and_then(|s| s.agents.iter().find(|a| a.agent_id == id).cloned());
                        match full_screen(c, session, &mut preview, |session| {
                            chat(say, session, &id, rx, color, agent)
                        }) {
                            Ok(true) => {
                                say.err(key!("tui-lost"), hub_arg);
                                std::thread::sleep(Duration::from_secs(2));
                                return Next::Reconnect;
                            }
                            Ok(false) => c.notice.clear(),
                            Err(code) => {
                                c.notice = say
                                    .text(key!("tui-opening-failed"), &[("code", Arg::Str(code))])
                            }
                        }
                        dirty_since.get_or_insert_with(Instant::now);
                    }
                    Action::Command(k) => {
                        if command(say, session, rx, color, c, &mut preview, area, k) {
                            dirty_since.get_or_insert_with(Instant::now);
                        }
                    }
                }
            }
        }
        if let Some(s) = &c.snapshot {
            follow(session, s, c.app.active_agent().unwrap_or(""), &mut preview);
        }
        // The open agent's terminal takes the pane's size (as `agora terminal attach` takes the window's), once per agent and size.
        let pane = c.app.areas(area).terminal;
        if let Some(active) = c.app.active_agent() {
            let want = (active.to_string(), pane.width, pane.height);
            if sized.as_ref() != Some(&want) && pane.width > 0 && pane.height > 0 {
                if let Some((node, local)) = active.split_once('/') {
                    session.terminal_resize(node, &format!("t-{local}"), pane.width, pane.height);
                }
                sized = Some(want);
            }
        }
        if let Some(p) = preview.as_mut() {
            match p.pump(session) {
                Pumped::Nothing => {}
                Pumped::Changed => redraw = true,
                Pumped::Ended => {
                    preview = None;
                    redraw = true;
                }
                Pumped::Lost => {
                    say.err(key!("tui-lost"), hub_arg);
                    std::thread::sleep(Duration::from_secs(2));
                    return Next::Reconnect;
                }
            }
        }
        // At most 25 frames a second, however fast the agent writes; input was handled before this. The first frame of a run is drawn
        // over a cleared screen: lines written meanwhile ("connecting…", a lost link) must not stay under it.
        if redraw && last_draw.elapsed() >= Duration::from_millis(40) {
            if std::mem::take(&mut fresh) {
                let _ = c.window.invalidate();
            }
            draw_console(say, c, &rows, preview.as_ref(), area);
            last_draw = Instant::now();
            redraw = false;
        }
        if dirty_since.is_some() && last_refresh.elapsed() >= Duration::from_millis(REFRESH_MS) {
            // The hub answers a new subscription with a fresh snapshot; a live feed applying single events is later work.
            if let Some(p) = preview.take() {
                p.close(session);
            }
            return Next::Reconnect;
        }
        if dirty_since.is_none() {
            last_refresh = Instant::now();
        }
    }
}

fn draw_console(say: &Say, c: &mut Console, rows: &[Row], preview: Option<&Preview>, area: Area) {
    let text = |id: &str, args: &[(&str, Arg)]| say.text(id, args);
    let inspector = c.app.areas(area).inspector;
    let details = c
        .snapshot
        .as_ref()
        .map(|s| app_details(s, &c.selected, inspector.width, inspector.height, &text).lines)
        .unwrap_or_default();
    // The footer's notice: the last thing done, or the hub's counts (machines, agents, what needs the user).
    let notice = if c.notice.is_empty() {
        c.snapshot.as_ref().map_or_else(String::new, |s| {
            let waiting = s.approvals.len()
                + s.agents
                    .iter()
                    .filter(|a| {
                        a.running && a.state == agora_protocol::v1::AgentState::Blocked as i32
                    })
                    .count();
            say.text(
                key!("tui-split-status"),
                &[
                    (
                        "online",
                        Arg::Num(s.nodes.iter().filter(|n| n.online).count() as i64),
                    ),
                    ("total", Arg::Num(s.nodes.len() as i64)),
                    (
                        "agents",
                        Arg::Num(s.agents.iter().filter(|a| a.running).count() as i64),
                    ),
                    ("waiting", Arg::Num(waiting as i64)),
                ],
            )
        })
    } else {
        c.notice.clone()
    };
    let term = preview
        .filter(|p| Some(p.agent.as_str()) == c.app.active_agent())
        .map(Preview::screen);
    // An open tab whose agent has finished says so, instead of "opening".
    let ended = c.app.active_agent().is_some_and(|id| {
        c.snapshot
            .as_ref()
            .is_some_and(|s| s.agents.iter().any(|a| a.agent_id == id && !a.running))
    });
    let ended_text = say.text(key!("tui-state-finished"), &[]);
    let no_screen = ended.then_some(ended_text.as_str());
    let (app, colors) = (&c.app, c.colors);
    let _ = c.window.draw(|f| {
        app.draw(
            f,
            &AppScreen {
                rows,
                terminal: term,
                details: &details,
                notice: &notice,
                no_screen,
                colors,
                t: &text,
            },
        );
    });
}

/// Runs a full-screen question or view (it reads plain bytes and writes its own frames): the panel's terminal is let go and the
/// mouse released meanwhile, and the console is drawn whole again afterwards. Returns what `run` said.
fn full_screen<T>(
    c: &mut Console,
    session: &mut Session,
    preview: &mut Option<Preview>,
    run: impl FnOnce(&mut Session) -> T,
) -> T {
    if let Some(p) = preview.take() {
        p.close(session);
    }
    mouse_mode(false);
    let _ = std::io::stdout().write_all(b"\x1b[?25h");
    let out = run(session);
    mouse_mode(c.app.mouse);
    let _ = c.window.invalidate();
    out
}

/// A letter for the selected agent. Returns whether the hub's state will change (a fresh snapshot is wanted).
#[allow(clippy::too_many_arguments)]
fn command(
    say: &Say,
    session: &mut Session,
    rx: &mpsc::Receiver<u8>,
    color: bool,
    c: &mut Console,
    preview: &mut Option<Preview>,
    area: Area,
    k: char,
) -> bool {
    let text = |id: &str, args: &[(&str, Arg)]| say.text(id, args);
    let Some(s) = c.snapshot.clone() else {
        return false;
    };
    let selected = c.selected.clone();
    match k {
        'y' | 'n' => {
            // ADR-0057: a start request is allowed only once all of it was in view: in the inspector, or in the reader, which y opens.
            let inspector = c.app.areas(area).inspector;
            let details = app_details(&s, &selected, inspector.width, inspector.height, &text);
            if let Some((id, false)) = details.start.clone() {
                if k == 'y' && !c.read_whole.contains(&id) {
                    if let Some(a) = s.approvals.iter().find(|a| a.approval_id == id).cloned() {
                        let read = full_screen(c, session, preview, |_| read_start(&a, rx, &text));
                        if read {
                            c.read_whole.insert(id);
                            c.notice = say.text(key!("tui-app-start-read"), &[]);
                        } else {
                            c.notice = say.text(key!("tui-split-read-first"), &[]);
                        }
                    }
                    return false;
                }
            }
            if let Some((id, true)) = details.start {
                c.read_whole.insert(id);
            }
            c.notice = answer(say, session, &s, &selected, k == 'y');
            true
        }
        'f' => {
            c.notice = full_screen(c, session, preview, |session| {
                start_in_folder(say, session, &s, &selected, rx, color)
            });
            true
        }
        's' => {
            c.notice = full_screen(c, session, preview, |session| {
                start(say, session, &s, &selected, rx)
            });
            true
        }
        // ADR-0060: an agent that stopped with its machine comes back (u) or starts again in its folder (U).
        'u' | 'U' => {
            let local = selected
                .split_once('/')
                .map_or(selected.as_str(), |(_, l)| l);
            let agent = [("agent", Arg::Str(local.to_string()))];
            let byte = if k == 'u' { b'u' } else { b'U' };
            c.notice = match resume_key(&s, &selected, byte, &mut c.armed, Instant::now()) {
                ResumeKey::NotInterrupted => say.text(key!("tui-resume-not-interrupted"), &[]),
                ResumeKey::CannotResume => say.text(key!("tui-resume-cannot"), &[]),
                ResumeKey::StillRunning => say.text(key!("tui-resume-still-running"), &[]),
                ResumeKey::Confirm { fresh: false } => say.text(key!("tui-resume-confirm"), &agent),
                ResumeKey::Confirm { fresh: true } => say.text(key!("tui-again-confirm"), &agent),
                ResumeKey::Go { fresh } => resume(say, session, &s, &selected, fresh),
            };
            true
        }
        'd' if !selected.is_empty() => {
            // A closed agent leaves every client's list; a running one is stopped first (x).
            let running = s.agents.iter().any(|a| a.agent_id == selected && a.running);
            c.notice = if running {
                say.text(key!("tui-forget-running"), &[])
            } else {
                forget_notice(say, session.forget_agent(&selected))
            };
            true
        }
        'x' if !selected.is_empty() => {
            let (node, local) = selected.split_once('/').unwrap_or(("", &selected));
            let (node, local) = (node.to_string(), local.to_string());
            let scope = format!("agent:{local}");
            // ADR-0057: an agent that started helpers asks whether they stop too; yes unless the user says no.
            let helpers = s
                .agents
                .iter()
                .filter(|a| a.running && a.parent_agent_id == selected)
                .count();
            c.notice = full_screen(c, session, preview, |session| {
                let with_helpers = helpers == 0 || {
                    let mut out = std::io::stdout();
                    let _ = write!(
                        out,
                        "\x1b[H\x1b[2J{}",
                        say.text(
                            key!("tui-stop-helpers"),
                            &[("count", Arg::Num(helpers as i64))]
                        )
                    );
                    let _ = out.flush();
                    !matches!(rx.recv_timeout(Duration::from_secs(30)), Ok(b'n'))
                };
                match with_grant(say, session, rx, (&node, &scope, &TYPING), |s| {
                    s.stop_agent_with(&node, &local, helpers > 0 && with_helpers)
                }) {
                    Tried::Done(Ok(())) => say.text(key!("tui-stopped"), &[]),
                    Tried::Done(Err(code)) => {
                        say.text(key!("tui-stop-failed"), &[("code", Arg::Str(code))])
                    }
                    Tried::Declined => String::new(),
                    Tried::Line(l) => l,
                }
            });
            true
        }
        _ => false,
    }
}

/// A machine's agents split into the ones it can start (numbered in the menu) and one line naming the others: the machine cannot
/// find their program in its `PATH`. The program names are not in the kinds answer yet, so the line names the agents.
fn split_kinds(
    say: &Say,
    all: Vec<(String, String, bool)>,
) -> (Vec<(String, String)>, Option<String>) {
    let mut installed = Vec::new();
    let mut missing = Vec::new();
    for (kind, name, ok) in all {
        if ok {
            installed.push((kind, name));
        } else {
            missing.push(plain(&name));
        }
    }
    let line = (!missing.is_empty()).then(|| {
        say.text(
            key!("tui-start-missing"),
            &[("names", Arg::Str(missing.join(", ")))],
        )
    });
    (installed, line)
}

#[cfg(test)]
mod forget_tests {
    use super::*;
    use agora_i18n::Catalog;
    use agora_protocol::error::{Error, ErrorCode};

    #[test]
    #[allow(clippy::unwrap_used)]
    fn y_does_not_allow_a_start_request_that_was_never_wholly_in_view() {
        // ADR-0057: an asking agent chooses the prompt's length, so what is below the panel's edge must be read before y counts.
        let detail = serde_json::json!({
            "node": "mac2", "agent": "agent-1", "agent_kind": "claude-code", "kind": "codex", "folder": "/w",
            "inside": true, "worktree": false, "depth": 1,
            "prompt": (1..=40).map(|i| format!("step {i}")).collect::<Vec<_>>().join("\n"),
        })
        .to_string();
        let s = Snapshot {
            approvals: vec![agora_protocol::v1::ApprovalSnapshot {
                approval_id: "mac2/a1".into(),
                agent_id: "mac2/agent-1".into(),
                action: "agent.start".into(),
                detail,
                ..Default::default()
            }],
            ..Default::default()
        };
        let say = Say(Catalog::english());
        let text = |id: &str, args: &[(&str, Arg)]| say.text(id, args);
        let mut read = std::collections::HashSet::new();
        // 40 prompt lines on a 10-row terminal: y is refused.
        assert!(start_unread(&s, "mac2/agent-1", false, &read, &text, (100, 10)).is_some());
        // The panel says how much is hidden.
        let view = View {
            snapshot: &s,
            selected: "mac2/agent-1",
            screen: None,
            notice: "",
            color: false,
            cols: 100,
            rows: 10,
            now_ms: 0,
        };
        let frame = frame(&view, &text);
        assert!(
            frame.contains("more lines: press Enter to read all"),
            "{frame}"
        );
        // Read whole in the reader: allowed.
        read.insert("mac2/a1".to_string());
        assert!(start_unread(&s, "mac2/agent-1", false, &read, &text, (100, 10)).is_none());
        // On a terminal tall enough to show all of it: allowed without the reader.
        assert!(start_unread(
            &s,
            "mac2/agent-1",
            false,
            &Default::default(),
            &text,
            (100, 80)
        )
        .is_none());
        // The reader counts as read only once its last line was on screen.
        let lines = start_card_lines(&s.approvals[0], 97, &text).unwrap();
        assert!(!reader_frame(&lines, 0, 100, 10, &text).1);
        assert!(reader_frame(&lines, lines.len(), 100, 10, &text).1);
    }

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
    fn u_and_u_shifted_each_need_the_same_key_twice_and_u_never_starts_a_new_session() {
        let agent = |id: &str, resumable: bool| agora_protocol::v1::AgentSnapshot {
            agent_id: id.into(),
            interrupted: true,
            resumable,
            ..Default::default()
        };
        let snap = Snapshot {
            agents: vec![agent("n/agent-1", true), agent("n/agent-2", false)],
            ..Default::default()
        };
        let t0 = Instant::now();
        let mut armed = None;
        assert_eq!(
            resume_key(&snap, "n/agent-1", b'u', &mut armed, t0),
            ResumeKey::Confirm { fresh: false }
        );
        assert_eq!(
            resume_key(
                &snap,
                "n/agent-1",
                b'u',
                &mut armed,
                t0 + Duration::from_secs(4)
            ),
            ResumeKey::Go { fresh: false }
        );
        // Too late, another key, or another agent: asks again.
        assert_eq!(
            resume_key(&snap, "n/agent-1", b'U', &mut armed, t0),
            ResumeKey::Confirm { fresh: true }
        );
        assert_eq!(
            resume_key(
                &snap,
                "n/agent-1",
                b'U',
                &mut armed,
                t0 + Duration::from_secs(6)
            ),
            ResumeKey::Confirm { fresh: true }
        );
        assert_eq!(
            resume_key(
                &snap,
                "n/agent-1",
                b'u',
                &mut armed,
                t0 + Duration::from_secs(7)
            ),
            ResumeKey::Confirm { fresh: false }
        );
        // u on one that cannot resume says so, whatever was pressed before; U still starts it again after a second press.
        assert_eq!(
            resume_key(&snap, "n/agent-2", b'u', &mut armed, t0),
            ResumeKey::CannotResume
        );
        assert_eq!(
            resume_key(&snap, "n/agent-2", b'u', &mut armed, t0),
            ResumeKey::CannotResume
        );
        assert_eq!(
            resume_key(&snap, "n/agent-2", b'U', &mut armed, t0),
            ResumeKey::Confirm { fresh: true }
        );
        assert_eq!(
            resume_key(&snap, "n/agent-2", b'U', &mut armed, t0),
            ResumeKey::Go { fresh: true }
        );
        assert_eq!(
            resume_key(&snap, "n/agent-9", b'u', &mut armed, t0),
            ResumeKey::NotInterrupted
        );
        // Its process from before the restart still runs: neither key starts anything.
        let mut running = agent("n/agent-3", true);
        running.still_running = true;
        let snap = Snapshot {
            agents: vec![running],
            ..Default::default()
        };
        for k in *b"uUuU" {
            assert_eq!(
                resume_key(&snap, "n/agent-3", k, &mut armed, t0),
                ResumeKey::StillRunning
            );
        }
        let tr = Say(Catalog::turkish());
        for k in [
            "tui-resume-cannot",
            "tui-resume-confirm",
            "tui-again-confirm",
            "tui-resume-still-running",
            "tui-interrupted-running",
        ] {
            assert!(
                tr.0.render(k, &[("agent", Arg::Str("agent-1".into()))])
                    .is_some(),
                "{k}"
            );
        }
    }

    #[test]
    fn a_resume_refusal_is_said_in_the_selected_language() {
        let tr = Say(Catalog::turkish());
        let changed = Error::new(ErrorCode::AgentFolderChanged)
            .with("agent_id", "agent-3")
            .with("path", "/work/\x1b[2Ja")
            .to_info();
        let line = resume_notice(&tr, &changed);
        assert!(line.starts_with("Başlamadı: "), "{line}");
        assert!(line.contains("agent-3") && !line.contains('\x1b'), "{line}");
        let en = Say(Catalog::english());
        let parent = Error::new(ErrorCode::AgentParentNotRunning)
            .with("agent_id", "agent-4")
            .with("parent_agent_id", "agent-3")
            .to_info();
        assert!(resume_notice(&en, &parent).contains("Resume that agent first"));
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

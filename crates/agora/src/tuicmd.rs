//! `agora tui`: the terminal console (ADR-0043). Slice 1 shows machines, agents and what waits for the user; slice 2 answers the
//! first waiting approval from the keyboard (`y` allow, `n` deny) with this terminal's own key (ADR-0028); slice 3 opens an agent's
//! terminal through the hub (`Tab` picks the agent, `Enter` opens it, Ctrl-] comes back).

use std::io::{IsTerminal, Read, Write};
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use agora_i18n::{key, Arg};
use agora_protocol::v1::{
    command_request::Command as WireCommand, param_value::Value, ChatEntry, ErrorInfo, Snapshot,
};
use agora_tui::app::{
    Action, App, Area, Ctx, Event as AppEvent, Row, Screen as AppScreen, ALLOW_ALWAYS,
};
use agora_tui::client::{sign_in, Answer, ChatEvent, ClientError, Hub, Session, TermEvent, Update};
use agora_tui::device::Device;
use agora_tui::input::Parser;
use agora_tui::pane::ColorMode;
use agora_tui::preview::{Preview, Pumped};
use agora_tui::render::{chat_command, plain, render, render_chat, session_line, ChatCommand};
use agora_tui::split::{app_details, app_rows};
use agora_tui::split::{frame, reader_frame, start_card_fit, start_card_lines, tree_order, View};

use crate::hubcli::{hub_dir, new_login_code, own_hub_addr, DEFAULT_LISTEN};
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
        // Named as the tree names it (the agent keeps its id when it comes back).
        Ok(agent) => {
            let name = snapshot
                .agents
                .iter()
                .find(|a| a.agent_id == agent || a.agent_id == selected)
                .map_or(agent.clone(), |a| {
                    agora_tui::split::agent_name(&a.agent_id, &a.kind)
                });
            say.text(key!("tui-resumed"), &[("agent", Arg::Str(name))])
        }
        Err(e) if e.code == "approval.signature_invalid" => {
            not_trusted(say, &device, node_binary(snapshot, node).as_deref())
        }
        Err(e) => resume_notice(say, &e),
    }
}

/// ADR-0049: the hub refused `command` because of the person's role or projects (a viewer, or a member outside their projects), naming
/// the refused command's schema name (`AgentStart`) in `action`. A machine that limits this device to other projects names its own
/// action (`directory`, `grant.create`), so that refusal is not the person's role (#1044).
fn refused_for_role(error: &ErrorInfo, command: &str) -> bool {
    error.code == "auth.forbidden"
        && matches!(
            error.params.get("action").and_then(|v| v.value.as_ref()),
            Some(Value::Str(s)) if s == command
        )
}

/// Why a start (`command`: `AgentStart`, or `DirectoryInspect` for a folder) did not happen: the person's role in words when the hub
/// refused it for that, else the code.
fn start_refusal(say: &Say, error: &ErrorInfo, command: &str) -> String {
    if refused_for_role(error, command) {
        return say.text(key!("tui-start-role"), &[]);
    }
    say.text(
        key!("tui-start-failed"),
        &[("code", Arg::Str(error.code.clone()))],
    )
}

/// Why a resume did not happen, in the selected language (the node's code and parameters, rendered from `errors.ftl`; the person's role
/// in words when the hub refused it for that, #1044).
fn resume_notice(say: &Say, error: &ErrorInfo) -> String {
    if refused_for_role(error, "AgentResume") {
        return say.text(key!("tui-resume-role"), &[]);
    }
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
    ask: &mut dyn Asker,
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
    let card = say.text(
        key!("tui-device-card"),
        &[
            ("name", Arg::Str(name.clone())),
            ("fingerprint", Arg::Str(fingerprint)),
            ("machines", Arg::Str(machines.join(", "))),
        ],
    );
    if ask.key(&lines_of(&card), Duration::from_secs(120)) != Some(b'y') {
        return String::new();
    }
    match session.endorse_device(&device, &waiting) {
        Ok(()) => say.text(key!("tui-device-approved"), &[("name", Arg::Str(name))]),
        Err(code) => fail(code),
    }
}

/// How long the owner has to read a card and answer it before the box closes by itself (the request stays pending on the hub).
const PAIR_ASK_FOR: Duration = Duration::from_secs(120);

/// Asks the hub whether this session is the owner's: `PairStatus` is answered to the owner only, and says the role for the folder
/// picker too (#1086, [`agora_tui::pair::status_role`]). `None` when it could not tell (the link failed): it is asked again at the next
/// connection.
fn pair_owner(session: &mut Session) -> Option<(agora_tui::pair::Owner, agora_tui::picker::Role)> {
    let answer = session.command_info(
        "",
        WireCommand::PairStatus(agora_protocol::v1::PairStatus {}),
        Duration::from_secs(3),
    );
    agora_tui::pair::status_role(answer.as_ref().err().map(|e| e.code.as_str()))
}

fn pair_asked_line(say: &Say, count: usize) -> String {
    if count > 1 {
        say.text(
            key!("tui-pair-asked-more"),
            &[("count", Arg::Num(count as i64))],
        )
    } else {
        say.text(key!("tui-pair-asked"), &[])
    }
}

fn pair_result_line(say: &Say, name: &str, ended: agora_tui::pair::Ended) -> String {
    use agora_tui::pair::Ended;
    let id = match ended {
        Ended::Approved => key!("tui-pair-result-approved"),
        Ended::Denied => key!("tui-pair-result-denied"),
        Ended::Expired => key!("tui-pair-result-expired"),
        Ended::Closed => key!("tui-pair-result-closed"),
    };
    say.text(id, &[("name", Arg::Str(agora_tui::pair::field(name)))])
}

/// `p`: the first machine that asks to join, as a card. `a` asks "do the two numbers match?" and only `y` after that approves; `d` denies;
/// `r` (when the name already belongs to a machine) chooses Replace; Esc or a pause leaves it pending. Returns the line to show.
fn answer_pair(say: &Say, session: &mut Session, ask: &mut Over<'_>) -> String {
    use agora_tui::pair::{can_answer, card_lines, too_soon, Dialog, Outcome};
    let fail = |code: String| say.text(key!("tui-pair-failed"), &[("code", Arg::Str(code))]);
    let Some(mut card) = ask.c.pair.first().cloned() else {
        return say.text(key!("tui-pair-none"), &[]);
    };
    let mut dialog = Dialog::default();
    // False once a command failed because the link is gone: then no answer is offered.
    let mut link_up = true;
    // Keys typed before the card was on screen answer nothing.
    while ask.rx().try_recv().is_ok() {}
    loop {
        let width = usize::from(window_area().width).saturating_sub(6).max(20);
        let lines = card_lines(
            &card,
            dialog.step,
            dialog.replace,
            now_ms(),
            link_up,
            width,
            &|id, args| say.text(id, args),
        );
        let shown = Instant::now();
        let key = match ask.key(&lines, PAIR_ASK_FOR) {
            // A key that was already on its way when the screen came up is not an answer to it: the card is shown again.
            Some(k) if too_soon(k, shown.elapsed()) => {
                while ask.rx().try_recv().is_ok() {}
                continue;
            }
            // A pause leaves the request pending on the hub.
            None => return String::new(),
            Some(k) => k,
        };
        match dialog.key(key, card.taken, can_answer(&card, now_ms(), link_up)) {
            Outcome::Stay => {}
            Outcome::Leave => return String::new(),
            Outcome::Deny => {
                match session.command_info(
                    "",
                    WireCommand::PairDeny(agora_protocol::v1::PairDeny {
                        pair_id: card.pair_id.clone(),
                    }),
                    Duration::from_secs(5),
                ) {
                    Ok(_) => return say.text(key!("tui-pair-denied"), &[]),
                    Err(e) if e.code == "pair.unknown" => {
                        return say.text(key!("tui-pair-gone"), &[])
                    }
                    // The link is gone: the card says so and offers no answer; leaving still works.
                    Err(e) if e.code == "client.unreachable" => link_up = false,
                    Err(e) => return fail(e.code),
                }
            }
            Outcome::Approve { replace } => {
                match session.command_info(
                    "",
                    WireCommand::PairApprove(agora_protocol::v1::PairApprove {
                        pair_id: card.pair_id.clone(),
                        replace,
                    }),
                    Duration::from_secs(5),
                ) {
                    Ok(_) => return say.text(key!("tui-pair-answer-sent"), &[]),
                    // The hub kept the request open and said the name belongs to a machine: the card now offers Replace, and
                    // approving takes both keys again.
                    Err(e)
                        if e.code == "pair.state"
                            && matches!(
                                e.params.get("state").and_then(|v| v.value.as_ref()),
                                Some(Value::Str(s)) if s == "name_in_use"
                            ) =>
                    {
                        card.taken = true;
                        ask.c.pair.mark_taken(&card.pair_id);
                        dialog.name_taken();
                    }
                    Err(e) if e.code == "pair.unknown" => {
                        return say.text(key!("tui-pair-gone"), &[])
                    }
                    Err(e) if e.code == "client.unreachable" => link_up = false,
                    Err(e) => return fail(e.code),
                }
            }
        }
    }
}

/// Answers the first waiting approval. Returns the line to show.
fn answer(
    say: &Say,
    session: &mut Session,
    snapshot: &Snapshot,
    selected: &str,
    answer: Answer,
) -> String {
    match agora_tui::client::answered_approval(snapshot, Some(selected)).cloned() {
        Some(a) => answer_request(say, session, snapshot, &a, answer),
        None => String::new(),
    }
}

/// Answers exactly the request `a`: refused first when it does not offer that answer (ADR-0101), otherwise signed and sent. Returns
/// the line to show.
fn answer_request(
    say: &Say,
    session: &mut Session,
    snapshot: &Snapshot,
    a: &agora_protocol::v1::ApprovalSnapshot,
    answer: Answer,
) -> String {
    // ADR-0101: an answer the request does not offer is never sent; the line says which one it offers.
    if let Some(id) = agora_tui::client::not_offered(a, answer) {
        return say.text(id, &[]);
    }
    let Some(device) = device() else {
        return say.text(
            key!("tui-answer-failed"),
            &[("code", Arg::Str("device".into()))],
        );
    };
    match session.resolve_request(a, &device, answer) {
        Ok(()) => say.text(
            match answer {
                Answer::Allow => key!("tui-allowed"),
                Answer::AllowAlways => {
                    agora_tui::client::always_text(key!("tui-allowed-always"), false)
                }
                Answer::Deny => key!("tui-denied"),
            },
            &[],
        ),
        Err(code) if code == "approval.signature_invalid" => {
            // The machine of the agent that asked.
            let node = a.agent_id.split_once('/').map_or("", |(n, _)| n);
            not_trusted(say, &device, node_binary(snapshot, node).as_deref())
        }
        Err(code) => say.text(key!("tui-answer-failed"), &[("code", Arg::Str(code))]),
    }
}

/// ADR-0059 decision 5: before y allows the start request `selected` points at whose helper would act without asking, a confirmation
/// answered with its own key, `s`: a held or repeated y (the key that opened it) never confirms but keeps asking, as does any other key
/// and no key at all; nothing is answered then. `true` when there is nothing to confirm or the user pressed s.
fn confirm_skipping_start(
    say: &Say,
    s: &Snapshot,
    selected: &str,
    text: agora_tui::render::Text<'_>,
    ask: &mut dyn Asker,
) -> bool {
    let Some(a) = agora_tui::client::answered_approval(s, Some(selected)) else {
        return true;
    };
    confirm_skipping_request(say, a, text, ask)
}

/// [`confirm_skipping_start`] for exactly the request `a`.
fn confirm_skipping_request(
    say: &Say,
    a: &agora_protocol::v1::ApprovalSnapshot,
    text: agora_tui::render::Text<'_>,
    ask: &mut dyn Asker,
) -> bool {
    let Some(names) = agora_tui::render::start_skipping_options(&a.action, &a.detail, text) else {
        return true;
    };
    let question = lines_of(&say.text(
        key!("tui-start-approval-confirm-skip"),
        &[("name", Arg::Str(names))],
    ));
    ask.key(&question, Duration::from_secs(60)) == Some(b's')
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
    ask: &mut dyn Asker,
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
        ask.frame(out.as_bytes());
        // Arrow keys come as j / k (`read_key`); Esc as itself. A resized window draws the reader again for its size.
        let k = match key_or_resize(ask.rx(), Duration::from_secs(300)) {
            Some(Some(k)) => k,
            Some(None) => continue,
            None => return seen_end,
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

/// Where the `agora` program of `node` is when it is not in that machine's `PATH`, as its folder and file (ADR-0069
/// `NodeSnapshot.binary`): the hub's owner receives the full path and is shown its last two parts; everyone else receives the file
/// name alone, which says nothing about where it is, so `None` then, as when it is in the `PATH` or the machine is older.
fn node_binary(snapshot: &Snapshot, node: &str) -> Option<String> {
    let full = snapshot
        .nodes
        .iter()
        .find(|n| n.node_id == node)
        .map(|n| n.binary.as_str())
        .filter(|b| !b.is_empty())?;
    let sep = if full.contains('\\') { '\\' } else { '/' };
    let mut parts = full.rsplit(sep).filter(|p| !p.is_empty());
    let file = parts.next()?;
    let folder = parts.next()?;
    Some(format!("{folder}{sep}{file}"))
}

/// This device is not trusted by `node`: how to trust it there, naming where its `agora` program is when the machine said so.
fn not_trusted(say: &Say, device: &Device, binary: Option<&str>) -> String {
    let mut args = vec![
        ("id", Arg::Str(device.id.clone())),
        ("key", Arg::Str(device.public_key_hex())),
        ("fingerprint", Arg::Str(device.fingerprint())),
    ];
    match binary {
        Some(b) => {
            args.push(("binary", Arg::Str(plain(b))));
            say.text(key!("tui-not-trusted-at"), &args)
        }
        None => say.text(key!("tui-not-trusted"), &args),
    }
}

/// A machine refused until its owner allows it (ADR-0041). Asks, and on `y` signs a grant with this device's key. `Ok(true)` means it
/// was given and the action can be repeated; `Ok(false)` that the user said no; `Err` is the line to show.
fn ask_grant(
    say: &Say,
    session: &mut Session,
    ask: &mut dyn Asker,
    node: &str,
    scope: &str,
    commands: &[&str],
) -> Result<bool, String> {
    let question = lines_of(&grant_question(say, scope, commands));
    if fresh_key(ask, &question, Duration::from_secs(30)) != Some(b'y') {
        return Ok(false);
    }
    let fail = |code: String| say.text(key!("tui-answer-failed"), &[("code", Arg::Str(code))]);
    let device = device().ok_or_else(|| fail("device".into()))?;
    match session.grant(node, &device, scope, commands, GRANT_TTL) {
        Ok(()) => Ok(true),
        // No snapshot here: the general sentence.
        Err(code) if code == "approval.signature_invalid" => Err(not_trusted(say, &device, None)),
        Err(code) => Err(fail(code)),
    }
}

/// The question for a grant of `commands` on `scope` (#810): it names the agent and what is allowed, and is the same set that gets
/// signed. A set this table does not know is asked in the general words, so nothing is ever signed that no sentence names.
fn grant_question(say: &Say, scope: &str, commands: &[&str]) -> String {
    let agent = [(
        "agent",
        // Written to the screen as it is: no control character of an id reaches the terminal.
        Arg::Str(plain(scope.strip_prefix("agent:").unwrap_or(scope)).replace('\n', " ")),
    )];
    match commands {
        ["terminal.input"] => say.text(key!("tui-grant-ask-typing"), &agent),
        ["agent.prompt"] => say.text(key!("tui-grant-ask-prompt"), &agent),
        ["agent.stop"] => say.text(key!("tui-grant-ask-stop"), &agent),
        ["agent.set_mode"] => say.text(key!("tui-grant-ask-mode"), &agent),
        ["agent.set_model"] => say.text(key!("tui-grant-ask-model"), &agent),
        ["task.dispatch"] => say.text(key!("tui-grant-ask-give"), &agent),
        _ => say.text(key!("tui-grant-ask"), &[]),
    }
}

/// Asks `lines` once and waits up to `wait` for one key after it (ADR-0063): the question is drawn once, then every key typed until
/// it was on the screen (meant for an agent, a `y` among them, also one that came while it was drawn) is set aside, so only an answer
/// after it counts; the terminal frames pastes for as long as it is asked (whatever the console's mouse setting), and a paste never
/// answers.
fn fresh_key(ask: &mut dyn Asker, lines: &[String], wait: Duration) -> Option<u8> {
    let framed = bracketed_paste(true);
    let answer = ask.answer(lines, wait);
    bracketed_paste(framed);
    answer
}

/// Lets `tracker` see keys that were forwarded to an agent, so it knows whether a bracketed paste is open at the end of them.
fn track_forwarded(tracker: &mut Parser, bytes: &[u8]) {
    let _ = tracker.push(bytes);
}

/// Sets aside every key typed so far by reading it through `parser` and dropping what it makes: a bracketed paste that began
/// before, or begins among them, keeps its framing (its text stays a paste, and the parser leaves paste mode at its end marker).
fn set_aside_keys(parser: &mut Parser, rx: &mpsc::Receiver<u8>) {
    let mut bytes = Vec::new();
    while let Ok(b) = rx.try_recv() {
        bytes.push(b);
    }
    let _ = parser.push(&bytes);
}

/// The answer in one read of the keyboard: only when it is exactly one key or click, never a paste nor several keys at once.
fn one_answer(events: &[AppEvent], area: Area, lines: &[String]) -> Option<u8> {
    // Pointer motion with no button is the terminal's report, not the user's answer: it neither answers nor spoils one that came
    // in the same read.
    let deliberate: Vec<&AppEvent> = events
        .iter()
        .filter(|e| {
            !matches!(
                e,
                AppEvent::Mouse(agora_tui::app::Mouse {
                    kind: agora_tui::app::MouseKind::Move,
                    ..
                })
            )
        })
        .collect();
    match deliberate.as_slice() {
        [e] if !matches!(e, AppEvent::Paste(_)) => agora_tui::app::dialog_key(e, area, lines),
        _ => None,
    }
}

/// Reads the keyboard through `parser` (a paste keeps its framing) until [`one_answer`] gives an answer or `wait` is over.
fn read_answer(
    parser: &mut Parser,
    rx: &mpsc::Receiver<u8>,
    lines: &[String],
    wait: Duration,
) -> Option<u8> {
    let end = Instant::now() + wait;
    loop {
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        let events = match rx.recv_timeout(left.min(Duration::from_millis(50))) {
            Ok(b) => {
                let mut bytes = vec![b];
                while let Ok(b) = rx.try_recv() {
                    bytes.push(b);
                }
                parser.push(&bytes)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => parser.flush(),
            Err(mpsc::RecvTimeoutError::Disconnected) => return None,
        };
        if let Some(k) = one_answer(&events, window_area(), lines) {
            return Some(k);
        }
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
    ask: &mut dyn Asker,
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
    match ask_grant(say, session, ask, node, scope, commands) {
        Ok(true) => Tried::Done(action(session)),
        Ok(false) => Tried::Declined,
        Err(line) => Tried::Line(line),
    }
}

// What a grant asks for is the least the refused action needs, never more (#810, what the web client asks for too): one command each.
/// Keys typed into an agent's terminal (`attach`, and the grant key Ctrl-] then g, whose notice names typing).
const TYPING_INPUT: [&str; 1] = ["terminal.input"];
/// A message sent to an agent in a chat pane.
const PROMPT: [&str; 1] = ["agent.prompt"];
/// Stopping an agent.
const STOP: [&str; 1] = ["agent.stop"];
/// ADR-0095: giving a board task to an agent (a grant for that agent, as the web window asks).
const GIVE: [&str; 1] = ["task.dispatch"];

/// The places that ask for a grant (#810): each one's command set comes from here, so a test can say what every place signs.
#[derive(Clone, Copy, Debug)]
enum Site {
    /// The split view's attached terminal.
    AttachTyping,
    /// The grant key (Ctrl-] then g) in the console.
    GrantKey,
    /// A message from the split view's chat.
    SplitPrompt,
    /// A message from the console's chat tab.
    ChatPrompt,
    /// Stopping from the split view.
    SplitStop,
    /// Stopping from the console.
    ConsoleStop,
    /// "Give to an agent" from the board.
    BoardGive,
}

impl Site {
    fn commands(self) -> &'static [&'static str] {
        match self {
            Site::AttachTyping | Site::GrantKey => &TYPING_INPUT,
            Site::SplitPrompt | Site::ChatPrompt => &PROMPT,
            Site::SplitStop | Site::ConsoleStop => &STOP,
            Site::BoardGive => &GIVE,
        }
    }
}

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
    // Follows the paste framing of what was forwarded, so a question asked after a paste's opening marker went out reads the rest
    // of that paste as a paste (its `y` answers nothing).
    let mut forwarded = Parser::default();
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
            // Output was dropped while a question waited: the screen is asked for again.
            TermEvent::Behind => {
                session.terminal_close(stream);
                stream = session.terminal_open(node, &terminal)?;
                let _ = write!(out, "\x1b[2J\x1b[H");
                let _ = out.flush();
            }
            TermEvent::Resized(..) | TermEvent::Quiet => {}
        }
        // What was typed was refused until the owner allows it: ask, and show the screen again either way (the typed keys are lost).
        if session
            .take_grant_wanted()
            .iter()
            .any(|r| r.is_for(node, &terminal))
        {
            let scope = format!("agent:{local}");
            let mut plain = Plain::with(rx, std::mem::take(&mut forwarded));
            let _ = ask_grant(
                say,
                session,
                &mut plain,
                node,
                &scope,
                Site::AttachTyping.commands(),
            );
            forwarded = plain.1;
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
            track_forwarded(&mut forwarded, &typed);
            session.terminal_input(node, &terminal, &typed);
        }
        // The terminal closed (window gone, SSH lost): stop watching instead of running on unseen.
        if terminal_gone() {
            break;
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
        if terminal_gone() {
            break;
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
                                &mut Plain::new(rx),
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
                        note = match with_grant(
                            say,
                            session,
                            &mut Plain::new(rx),
                            (node, &scope, Site::SplitPrompt.commands()),
                            |s| s.prompt(node, local, &line),
                        ) {
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
    ask: &mut dyn Asker,
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
    let pick = pick_kind(
        say,
        ask,
        &node,
        &[],
        &kinds,
        missing.as_deref(),
        Duration::from_secs(30),
    );
    let Some((kind, _)) = pick.and_then(|i| kinds.get(i)) else {
        return String::new();
    };
    let scope = format!("project:{project}");
    // The last refusal with its parameters, so the line can tell the person's role from anything else (#1044).
    let mut refused = ErrorInfo::default();
    match with_grant(say, session, ask, (&node, &scope, &["agent.start"]), |s| {
        s.start_agent(&node, &project, kind).map_err(|e| {
            let code = e.code.clone();
            refused = e;
            code
        })
    }) {
        Tried::Done(Ok(agent)) => say.text(key!("tui-started"), &[("agent", Arg::Str(agent))]),
        Tried::Done(Err(_)) => start_refusal(say, &refused, "AgentStart"),
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
    escape_key(rx)
}

/// Like [`read_key`], but says when the keys are gone for good (the terminal closed): `Err`.
fn read_key_or_gone(rx: &mpsc::Receiver<u8>, wait: Duration) -> Result<Option<u8>, ()> {
    match rx.recv_timeout(wait) {
        Ok(k) if k != 0x1b => Ok(Some(k)),
        Ok(_) => Ok(escape_key(rx)),
        Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(()),
    }
}

/// The rest of a key that began with ESC, read whole: `ESC [ … A/B/D/C` (with xterm modifiers too) as k / j / ⌫ / Enter, any other
/// `ESC [` sequence (Page Down, a click or the wheel, which the review and the files still receive) as no key, so it never reads as
/// Esc nor leaves its tail to be read as typed keys; ESC with anything else, or alone for 15 ms, as Esc.
fn escape_key(rx: &mpsc::Receiver<u8>) -> Option<u8> {
    let next = || rx.recv_timeout(Duration::from_millis(15)).ok();
    if next() != Some(b'[') {
        // ESC and one more byte (Alt and a key) are Esc, as before.
        let _ = next();
        return Some(0x1b);
    }
    // Parameters up to the final byte; one longer than `input`'s longest is not a key and is dropped.
    for _ in 0..32 {
        match next() {
            Some(b'A') => return Some(b'k'),
            Some(b'B') => return Some(b'j'),
            Some(b'D') => return Some(0x7f),
            Some(b'C') => return Some(b'\r'),
            Some(0x40..=0x7e) => return None,
            Some(_) => {}
            None => return Some(0x1b),
        }
    }
    None
}

/// A key within `wait` (`Some(Some(k))`), or the window resized meanwhile (`Some(None)`: draw again for the new size), or nothing:
/// no key in time, or the terminal is gone (`None`). For the screens a question draws itself (the folder picker, the reader).
fn key_or_resize(rx: &mpsc::Receiver<u8>, wait: Duration) -> Option<Option<u8>> {
    let end = Instant::now() + wait;
    let area = window_area();
    while Instant::now() < end {
        match read_key_or_gone(rx, Duration::from_millis(50)) {
            Ok(Some(k)) => return Some(Some(k)),
            Err(()) => return None,
            Ok(None) if window_area() != area => return Some(None),
            Ok(None) => {}
        }
    }
    None
}

/// Why a full-screen view (the diff, the files) wakes: a key, or a reason to draw again.
enum Wake {
    Key(u8),
    /// The window was resized, or something the view shows changed (a request arrived: see [`ViewWait::banner`]).
    Redraw,
    /// The terminal closed, the link to the hub went, or nothing happened for ten minutes: the view ends.
    Gone,
}

/// A full-screen view's wait, as event-driven as the console's (the TUI review): keys, a resized window, and the hub's events,
/// which keep being read meanwhile so the snapshot stays current and a new request is said in the view's footer.
struct ViewWait {
    area: Area,
    idle_until: Instant,
    /// What the view says in its footer for something that wants the user meanwhile; empty when nothing does.
    banner: String,
}

impl ViewWait {
    /// A request that already waits when the view opens is said from the start, not only one that arrives meanwhile.
    fn new(say: &Say, ask: &Over<'_>) -> Self {
        let waiting = ask
            .c
            .snapshot
            .as_ref()
            .is_some_and(|s| !s.approvals.is_empty());
        Self {
            area: window_area(),
            idle_until: Instant::now() + Duration::from_secs(600),
            banner: if waiting {
                say.text(key!("tui-view-asked"), &[])
            } else {
                String::new()
            },
        }
    }

    fn wait(&mut self, say: &Say, ask: &mut Over<'_>, session: &mut Session) -> Wake {
        loop {
            match read_key_or_gone(ask.rx, Duration::from_millis(50)) {
                Ok(Some(k)) => {
                    self.idle_until = Instant::now() + Duration::from_secs(600);
                    return Wake::Key(k);
                }
                Err(()) => return Wake::Gone,
                Ok(None) => {}
            }
            let now = window_area();
            if now != self.area {
                self.area = now;
                let _ = ask.c.window.resized();
                return Wake::Redraw;
            }
            match session.next(Duration::ZERO) {
                Update::Snapshot(s) => {
                    ask.c.give_lacking.settle(&s);
                    ask.c.snapshot = Some(*s);
                }
                Update::Changed => ask.c.stale = true,
                Update::Closed => return Wake::Gone,
                Update::Quiet => {}
            }
            if session.take_asked() {
                self.banner = say.text(key!("tui-view-asked"), &[]);
                return Wake::Redraw;
            }
            // ADR-0090: an agent's config options changed (the picker's "does not ask" mark may have).
            if session.take_configs_changed() {
                return Wake::Redraw;
            }
            if Instant::now() >= self.idle_until {
                return Wake::Gone;
            }
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// What the folder picker asks a machine (#1099: apart from the session, so the dialogue is tested without a hub).
trait Folders {
    /// The folder `path` on `node` (`""`: its home folder), signed for exactly that folder; `client.sign` when this terminal could not
    /// sign.
    fn list(
        &mut self,
        node: &str,
        path: &str,
    ) -> Result<agora_protocol::v1::DirectoryEntries, ErrorInfo>;
}

/// The machines' folders through the hub, each `directory.list` signed by this terminal's key (ADR-0052).
struct SignedFolders<'a> {
    session: &'a mut Session,
    device: &'a Device,
}

impl Folders for SignedFolders<'_> {
    fn list(
        &mut self,
        node: &str,
        path: &str,
    ) -> Result<agora_protocol::v1::DirectoryEntries, ErrorInfo> {
        let Some(auth) = self
            .device
            .authorize(node, "list", path, "", "", false, false, now_ms())
        else {
            return Err(ErrorInfo {
                code: "client.sign".into(),
                ..ErrorInfo::default()
            });
        };
        self.session.list_folder(node, path, auth)
    }
}

/// How the folder picker ended.
#[derive(Debug, PartialEq, Eq)]
enum Browsed {
    /// The folder chosen on the machine.
    Chosen { node: String, path: String },
    /// The line to show instead (empty: the user cancelled).
    Said(String),
    /// The machine does not trust this terminal's key.
    NotTrusted(String),
}

/// The folder picker of [`start_in_folder`] (ADR-0053): one machine's folders at a time, from where the person's role lets them look
/// (#1086, #1099). `role` is what this connection knows of it; set here when a machine's answer tells more.
#[allow(clippy::too_many_lines)] // one dialogue: where to open, what each answer means, the keys
fn browse_folders(
    say: &Say,
    folders: &mut dyn Folders,
    snapshot: &Snapshot,
    selected: &str,
    ask: &mut dyn Asker,
    color: bool,
    role: &mut agora_tui::picker::Role,
) -> Browsed {
    let signing_failed = || {
        Browsed::Said(say.text(
            key!("tui-answer-failed"),
            &[("code", Arg::Str("client.sign".into()))],
        ))
    };
    let online: Vec<&agora_protocol::v1::NodeSnapshot> =
        snapshot.nodes.iter().filter(|n| n.online).collect();
    if online.is_empty() {
        return Browsed::Said(say.text(key!("tui-start-nowhere"), &[]));
    }
    // The selected agent's machine when it is online, else the first online one; `m` goes to the next.
    let mut at = snapshot
        .agents
        .iter()
        .find(|a| a.agent_id == selected)
        .and_then(|a| online.iter().position(|n| n.node_id == a.node_id))
        .unwrap_or(0);
    // #1086: a person the hub said is not the owner may look only inside their projects (ADR-0049 Amendment A), so their picker opens
    // at their project's folder on the machine (or offers their projects there) and never goes above one; the owner's opens at the
    // home folder, as before. #1099: with the role not known (the plain list), it opens at the project folder too, as the window does,
    // and goes up as the machine says, so a member is never refused a first folder.
    use agora_tui::picker::{project_roots, At, Nav, Role};
    let nav_for = |node: &str, role: Role| Nav::for_role(role, project_roots(snapshot, node));
    let mut nav = nav_for(&online[at].node_id, *role);
    let mut target = nav.start();
    // #1099: the choice of projects is offered once the machine let this person look into one of them (a viewer is told at once);
    // asked once per machine in one dialogue, so going round the machines with `m` does not ask again.
    let mut probed: std::collections::HashSet<String> = std::collections::HashSet::new();
    // What was shown before the last step: a step into a folder the hub refuses for the person's role goes back to it.
    let mut before: Option<(Nav, At, agora_protocol::v1::DirectoryEntries)> = None;
    let mut notice = String::new();
    let text = |id: &str, args: &[(&str, Arg)]| say.text(id, args);
    'browse: loop {
        let node = online[at];
        let name = if node.hostname.is_empty() {
            node.node_id.clone()
        } else {
            node.hostname.clone()
        };
        let folder = match &target {
            At::Projects => {
                let first = nav.probe(snapshot, &node.node_id);
                if let Some(first) = first.filter(|_| probed.insert(node.node_id.clone())) {
                    match folders.list(&node.node_id, &first) {
                        Err(e) if e.code == "client.sign" => return signing_failed(),
                        Err(e) if e.code == "approval.signature_invalid" => {
                            return Browsed::NotTrusted(node.node_id.clone())
                        }
                        Err(e) if e.code == "protocol.capability_missing" => {
                            return Browsed::Said(say.text(key!("tui-folder-unsupported"), &[]))
                        }
                        // Not to be taken for a viewer's: a folder a project they were not granted was also reported in is refused
                        // to a member too, so the choice stays, with the reason.
                        Err(e) if refused_for_role(&e, "DirectoryList") => {
                            *role = Role::Limited;
                            nav = nav_for(&node.node_id, Role::Limited);
                            notice = say.text(key!("tui-folder-role"), &[]);
                        }
                        Ok(_) | Err(_) => {}
                    }
                }
                nav.projects_listing()
            }
            At::Folder(path) => {
                match folders.list(&node.node_id, path) {
                    Ok(f) => {
                        // #1099: a way up from a member's project folder: the hub no longer keeps them inside their projects.
                        if nav.answered(&f) {
                            *role = nav.role();
                        }
                        f
                    }
                    Err(e) if e.code == "client.sign" => return signing_failed(),
                    Err(e) if e.code == "approval.signature_invalid" => {
                        return Browsed::NotTrusted(node.node_id.clone())
                    }
                    Err(e) if e.code == "protocol.capability_missing" => {
                        return Browsed::Said(say.text(key!("tui-folder-unsupported"), &[]))
                    }
                    // #1044: the hub does not let this person's role look into this folder.
                    Err(e) if refused_for_role(&e, "DirectoryList") => {
                        let roots = project_roots(snapshot, &node.node_id);
                        if !nav.confined() && path.is_empty() && !roots.is_empty() {
                            // Refused the home folder: a member, who may look in their projects.
                            *role = Role::Limited;
                            nav = Nav::projects(roots);
                            target = nav.start();
                            continue;
                        }
                        let Some((back, at_before, shown)) = before.take() else {
                            return Browsed::Said(say.text(key!("tui-folder-role"), &[]));
                        };
                        // A folder of another project inside theirs, or a link out of it: they stay where they were, told why.
                        nav = back;
                        target = at_before;
                        notice = say.text(key!("tui-folder-role"), &[]);
                        shown
                    }
                    Err(e) => {
                        // A folder that cannot be listed (gone, a secret one): back to where the picker opens, saying why.
                        let mut fresh = nav_for(&node.node_id, *role);
                        let mut start = fresh.start();
                        if start == target && *role == Role::Unknown && !path.is_empty() {
                            // #1099: with the role not known, the project folder itself: the home folder, where the owner's picker
                            // always opened (a member is refused it once and goes on in their projects).
                            fresh = Nav::machine();
                            start = fresh.start();
                        }
                        if start == target {
                            return Browsed::Said(
                                say.text(key!("tui-folder-failed"), &[("code", Arg::Str(e.code))]),
                            );
                        }
                        notice = say.text(key!("tui-folder-failed"), &[("code", Arg::Str(e.code))]);
                        nav = fresh;
                        target = start;
                        before = None;
                        continue;
                    }
                }
            }
        };
        let projects = target == At::Projects;
        let mut pick = 0usize;
        loop {
            let (cols, rows) = agora_platform::terminal_size().unwrap_or((100, 30));
            let view = agora_tui::picker::Picker {
                machine: &name,
                folder: &folder,
                selected: pick,
                notice: &notice,
                projects,
                color,
                cols,
                rows,
            };
            ask.frame(agora_tui::picker::frame(&view, &text).as_bytes());
            // A resized window draws the picker again for its size.
            let k = match key_or_resize(ask.rx(), Duration::from_secs(120)) {
                Some(Some(k)) => k,
                Some(None) => continue,
                None => return Browsed::Said(String::new()),
            };
            notice.clear();
            let count = folder.entries.len();
            match k {
                b'j' if count > 0 => pick = (pick + 1) % count,
                b'k' if count > 0 => pick = (pick + count - 1) % count,
                b'\r' | b'\n' => {
                    let back = nav.clone();
                    let next = if projects {
                        nav.open_project(pick)
                    } else {
                        folder.entries.get(pick).map(|e| nav.open(&folder, &e.name))
                    };
                    if let Some(next) = next {
                        before = Some((back, std::mem::replace(&mut target, next), folder));
                        continue 'browse;
                    }
                }
                0x7f | 0x08 | b'h' if !projects => {
                    let back = nav.clone();
                    if let Some(next) = nav.up(&folder) {
                        before = Some((back, std::mem::replace(&mut target, next), folder));
                        continue 'browse;
                    }
                }
                b'm' if online.len() > 1 => {
                    at = (at + 1) % online.len();
                    nav = nav_for(&online[at].node_id, *role);
                    target = nav.start();
                    before = None;
                    continue 'browse;
                }
                b' ' | b'.' if !projects => {
                    return Browsed::Chosen {
                        node: node.node_id.clone(),
                        path: folder.path.clone(),
                    }
                }
                0x1b | b'q' | 3 => return Browsed::Said(String::new()),
                _ => {}
            }
        }
    }
}

/// Start an agent in a folder the user picks on a machine (ADR-0053): the machine's folders through `directory.list`, then the chosen
/// one through `directory.inspect` and `agent.launch`, each signed by this terminal's key for exactly that folder (ADR-0052). Answers
/// the line to show; empty when the user cancelled. `role`: what this connection knows of the person's role (#1086, #1099); set here
/// when the hub refuses a folder for the role.
#[allow(clippy::too_many_lines)] // one dialogue: browse, choose, confirm, launch
fn start_in_folder(
    say: &Say,
    session: &mut Session,
    snapshot: &Snapshot,
    selected: &str,
    ask: &mut dyn Asker,
    color: bool,
    role: &mut agora_tui::picker::Role,
) -> String {
    let Some(device) = device() else {
        return say.text(
            key!("tui-answer-failed"),
            &[("code", Arg::Str("device".into()))],
        );
    };
    let not_trusted = |device: &Device, node: &str| {
        not_trusted(say, device, node_binary(snapshot, node).as_deref())
    };
    let (node, folder) = match browse_folders(
        say,
        &mut SignedFolders {
            session,
            device: &device,
        },
        snapshot,
        selected,
        ask,
        color,
        role,
    ) {
        Browsed::Chosen { node, path } => (node, path),
        Browsed::Said(line) => return line,
        Browsed::NotTrusted(node) => return not_trusted(&device, &node),
    };
    let Some(auth) = device.authorize(&node, "inspect", &folder, "", "", false, false, now_ms())
    else {
        return say.text(
            key!("tui-answer-failed"),
            &[("code", Arg::Str("client.sign".into()))],
        );
    };
    let info = match session.inspect_folder(&node, &folder, auth) {
        Ok(i) => i,
        Err(e) if e.code == "approval.signature_invalid" => return not_trusted(&device, &node),
        Err(e) => return start_refusal(say, &e, "DirectoryInspect"),
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
    let pick = pick_kind(
        say,
        ask,
        &node,
        &[plain(&info.path)],
        &kinds,
        missing.as_deref(),
        Duration::from_secs(60),
    );
    let Some((kind, _)) = pick.and_then(|i| kinds.get(i)) else {
        return String::new();
    };
    // A folder whose project changed since an agent last ran there needs the user's yes (ADR-0052).
    let accept = !info.previous_project_id.is_empty();
    if accept {
        let question = lines_of(&say.text(key!("tui-folder-identity"), &[]));
        if ask.key(&question, Duration::from_secs(60)) != Some(b'y') {
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
        let Some(k) = ask.key(&menu, Duration::from_secs(60)) else {
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
            let question = lines_of(&say.text(
                key!("tui-start-confirm-skip"),
                &[("name", Arg::Str(label(o)))],
            ));
            // Confirmed here: signed with the start; the node refuses an option that skips permissions without it.
            if ask.key(&question, Duration::from_secs(60)) == Some(b'y') {
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
            // ADR-0066: a signed launch carries no settings until its signature covers them (PR 2).
            settings: Vec::new(),
        },
    ) {
        Ok(agent) => say.text(key!("tui-started"), &[("agent", Arg::Str(agent))]),
        Err(code) if code == "approval.signature_invalid" => not_trusted(&device, &node),
        Err(code) => say.text(key!("tui-start-failed"), &[("code", Arg::Str(code))]),
    }
}

/// Keeps the panel's terminal on the selected agent: stops watching the old one and opens the new one when it has a terminal.
fn follow(
    session: &mut Session,
    snapshot: &Snapshot,
    selected: &str,
    preview: &mut Option<Preview>,
    tried: &mut Option<(String, Instant)>,
) {
    let wanted = snapshot.terminals.iter().any(|t| t.agent_id == selected);
    if wanted && preview.as_ref().is_some_and(|p| p.agent == selected) {
        return;
    }
    if let Some(p) = preview.take() {
        p.close(session);
    }
    // A terminal that could not be opened (its machine not back yet after a reconnect) is tried again every 2 s, not on every
    // turn of the loop.
    if wanted
        && !tried
            .as_ref()
            .is_some_and(|(a, at)| a == selected && at.elapsed() < RETRY_OPEN)
    {
        *preview = Preview::open(session, selected).ok();
        *tried = preview
            .is_none()
            .then(|| (selected.to_string(), Instant::now()));
    }
}

/// Says `text` in the console's footer (a box when it is long) and draws the console at once: the link to the hub is lost or
/// being made, and the console is not running its loop to draw it. Before, these lines went to stderr, over the screen.
fn show_notice(say: &Say, c: &mut Console, text: String) {
    c.notice = text;
    let t = |id: &str, args: &[(&str, Arg)]| say.text(id, args);
    let rows = c
        .snapshot
        .as_ref()
        .map(|s| app_rows(s, now_ms(), &t))
        .unwrap_or_default();
    draw_console(say, c, &rows, None, None, window_area(), None);
}

/// How long a notice stays in the footer before the summary comes back.
const NOTICE_FOR: Duration = Duration::from_secs(10);

/// How long a terminal or a conversation that could not be opened waits before it is tried again.
const RETRY_OPEN: Duration = Duration::from_secs(2);

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
    // Without `--hub`: where the hub in that folder says it listens (it may run on a port of its own), else the default.
    let addr = o
        .hub
        .clone()
        .or_else(|| own_hub_addr(&dir))
        .unwrap_or_else(|| DEFAULT_LISTEN.to_string());
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
    if let Some(raw) = raw.as_ref() {
        // A stop signal (SIGTERM, SIGHUP, SIGINT) puts the terminal back and then ends the process as it always did, wherever the
        // program is waiting: no loop of this console has to look for it.
        let _ = agora_platform::restore_terminal_on_stop(raw, agora_tui::backend::RESET);
    }
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
                shown_request: None,
                stale: false,
                not_sized: std::collections::HashSet::new(),
                drafts: std::collections::HashMap::new(),
                open_failed_notice: None,
                notice_at: Instant::now(),
                links: agora_tui::termio::LinkConfirm::default(),
                note: Note::default(),
                pair: agora_tui::pair::Book::default(),
                role: agora_tui::picker::Role::Unknown,
                colors: if color {
                    ColorMode::from_env(
                        None,
                        std::env::var("COLORTERM").ok().as_deref(),
                        std::env::var("TERM").ok().as_deref(),
                        std::env::var("WT_SESSION")
                            .or_else(|_| std::env::var("TERM_PROGRAM"))
                            .ok()
                            .as_deref(),
                    )
                } else {
                    ColorMode::Mono
                },
                typing_refused: None,
                typing_notice: String::new(),
                typing_allowed: None,
                asked: false,
                typing_taken: std::collections::HashMap::new(),
                give_lacking: agora_tui::board::Lacking::default(),
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
            // The terminal is gone (closed window, lost SSH session): the client ends instead of running on unseen.
            STDIN_GONE.store(true, std::sync::atomic::Ordering::Release);
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
        // No terminal left to show anything in: stop trying to connect.
        if terminal_gone() {
            return ExitCode::SUCCESS;
        }
        if first {
            match console_state.as_mut() {
                // The console's own screen says it (stderr would write over it).
                Some(c) => show_notice(say, c, say.text(key!("tui-connecting"), &hub_arg)),
                None => say.err(key!("tui-connecting"), &hub_arg),
            }
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
                    let message = if local {
                        say.text(key!("tui-signin-failed-local"), &[])
                    } else {
                        say.text(key!("tui-signin-failed"), &[])
                    };
                    // The message is for the screen the person came from: the console's screen is left first (it would not be seen
                    // on the alternate screen), and a line ends with CR LF while the keyboard is still in raw mode.
                    drop(console_state.take());
                    if keys {
                        let mut out = std::io::stdout();
                        let _ = out.write_all(agora_tui::backend::RESET);
                        let _ = out.flush();
                    }
                    eprint!("{message}{eol}");
                    return ExitCode::from(1);
                }
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
            Err(ClientError::Unreachable) => {
                match console_state.as_mut() {
                    Some(c) => show_notice(say, c, say.text(key!("tui-lost"), &hub_arg)),
                    None => say.err(key!("tui-lost"), &hub_arg),
                }
                first = false;
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
        };
        first = false;
        if let Some(c) = console_state.as_mut() {
            connected_again(
                say,
                &hub_arg,
                &mut c.notice,
                &mut c.give_lacking,
                &mut c.pair,
                &mut c.role,
            );
            match console(say, &mut session, &rx, &hub_arg, color, c) {
                Next::Quit => return ExitCode::SUCCESS,
                Next::Reconnect => continue,
            }
        }
        let mut dirty_since: Option<Instant> = None;
        // #1086, #1099: the plain list does not ask the hub for the role; it is learnt from a folder the hub refuses for it.
        let mut role = agora_tui::picker::Role::Unknown;
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
                    follow(&mut session, s, &selected, &mut preview, &mut None);
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
            // The terminal is gone: end, as the console does.
            if terminal_gone() {
                return ExitCode::SUCCESS;
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
                            follow(&mut session, s, &selected, &mut preview, &mut None);
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
                            if k == b'y'
                                && !confirm_skipping_start(
                                    say,
                                    s,
                                    &selected,
                                    &text,
                                    &mut Plain::new(&rx),
                                )
                            {
                                notice = String::new();
                                draw(&snapshot, &selected, &notice, cost, preview.as_ref());
                                continue;
                            }
                            let reply = if k == b'y' {
                                Answer::Allow
                            } else {
                                Answer::Deny
                            };
                            notice = answer(say, &mut session, s, &selected, reply);
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
                            notice = start_in_folder(
                                say,
                                &mut session,
                                sn,
                                &selected,
                                &mut Plain::new(&rx),
                                color,
                                &mut role,
                            );
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
                        notice = approve_device(
                            say,
                            &mut session,
                            snapshot.as_ref(),
                            &mut Plain::new(&rx),
                        );
                        dirty_since.get_or_insert_with(Instant::now);
                        draw(&snapshot, &selected, &notice, cost, preview.as_ref());
                    }
                    b's' => {
                        if let Some(sn) = &snapshot {
                            notice = start(say, &mut session, sn, &selected, &mut Plain::new(&rx));
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
                                let question = say.text(
                                    key!("tui-stop-helpers"),
                                    &[("count", Arg::Num(helpers as i64))],
                                );
                                Plain::new(&rx).key(&lines_of(&question), Duration::from_secs(30))
                                    != Some(b'n')
                            };
                            notice = match with_grant(
                                say,
                                &mut session,
                                &mut Plain::new(&rx),
                                (node, &scope, Site::SplitStop.commands()),
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
                            if read_start(&a, &mut Plain::new(&rx), &text) {
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
            if dirty_since.is_some()
                && refresh_due(last_refresh.elapsed(), session.typing_unanswered())
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

/// The console's shortest time between two frames: 60 a second, the ADR-0022 budget for a keystroke's echo.
const FRAME: Duration = Duration::from_millis(16);

/// A routine refresh replaces the session, and with it the numbers that match a keystroke to its reply: a refusal (and the hint to
/// allow typing) of a keystroke still in flight would be lost. So it waits until every typing reply has arrived, at most
/// [`REFRESH_PATIENCE`] longer than usual (a reply that never comes must not stop refreshing for good).
fn refresh_due(since_dirty: Duration, typing_unanswered: usize) -> bool {
    since_dirty >= Duration::from_millis(REFRESH_MS)
        && (typing_unanswered == 0
            || since_dirty >= Duration::from_millis(REFRESH_MS) + REFRESH_PATIENCE)
}

/// How much longer a refresh waits for typing replies.
const REFRESH_PATIENCE: Duration = Duration::from_secs(3);

/// What a new connection to the hub changes in the console's memory. Every one drops the pairing cards held from the last, which are
/// stale until the hub sends the ones waiting now. After a lost link ("Connecting…" or "lost, trying again" still shown; a routine
/// refresh keeps its own notice) that notice ends, what machines said about taking tasks may be old, and the role is asked again:
/// the hub ends a connection when the person's role or grants change (#1099).
fn connected_again(
    say: &Say,
    hub_arg: &[(&str, Arg)],
    notice: &mut String,
    give_lacking: &mut agora_tui::board::Lacking,
    pair: &mut agora_tui::pair::Book,
    role: &mut agora_tui::picker::Role,
) {
    if *notice == say.text(key!("tui-connecting"), hub_arg)
        || *notice == say.text(key!("tui-lost"), hub_arg)
    {
        notice.clear();
        give_lacking.clear();
        pair.lost();
        *role = agora_tui::picker::Role::Unknown;
    }
    pair.reset();
}

/// What the console does after its loop ends.
enum Next {
    Quit,
    /// The link was lost or a fresh snapshot is due: connect again and come back.
    Reconnect,
}

/// Set when the keyboard's terminal is gone for good (its stdin ended or failed): every loop ends then. A client whose terminal
/// closed kept running, signed in and polling (the TUI review found six), since nothing read the end of its input.
static STDIN_GONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn terminal_gone() -> bool {
    STDIN_GONE.load(std::sync::atomic::Ordering::Acquire)
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
    /// The hub said something changed while a full-screen view held the screen: the console asks for a fresh snapshot after it.
    stale: bool,
    /// Agents whose terminal is never sized to the pane: the one this client itself runs in (see the console's size loop).
    not_sized: std::collections::HashSet<String>,
    /// What was typed into each chat agent's line and not sent yet: kept when the conversation is opened again (a fresh snapshot
    /// reconnects the console, another tab comes to the front), so a draft is never lost.
    drafts: std::collections::HashMap<String, Vec<u8>>,
    /// The "could not open" notice put up for a terminal or conversation, cleared when the open later works (also after a
    /// reconnect, when the machine is back).
    open_failed_notice: Option<String>,
    /// When the footer's notice was put up: it gives way to the summary after NOTICE_FOR.
    notice_at: Instant,
    /// The two-click rule for opening an address (see `LinkConfirm`); told when the footer was drawn.
    links: agora_tui::termio::LinkConfirm,
    /// A request shown whole outside the inspector (the chat tab's pane shows its agent's): y / n may answer it without asking again.
    shown_request: Option<String>,
    note: Note,
    /// ADR-0065: the machines that ask to join, for an owner's session. Memory only.
    pair: agora_tui::pair::Book,
    /// #1086, #1099: what the hub said of this person's role on this connection (`PairStatus`, or a folder refused for the role), for
    /// the folder picker. Asked again after a lost link, since the hub ends a connection when the role changes.
    role: agora_tui::picker::Role,
    colors: ColorMode,
    /// ADR-0041: typing into the agent in front was refused until its owner allows it; `g` (from the list: Ctrl-] then g) asks.
    typing_refused: Option<TypingRefused>,
    /// The footer notice that said so, taken away with it.
    typing_notice: String,
    /// The agent (`node/local`) typing was allowed for here, and the last request sent before: a refusal of that typing, read
    /// later, is old news. Per session (request numbers start over on a reconnect).
    typing_allowed: Option<(String, u64)>,
    /// A question was asked over the console ([`over_console`]) while a batch of input was handled: the rest of that batch was
    /// typed before the question was seen, so none of it may answer anything (ADR-0063).
    asked: bool,
    /// The newest typing each terminal's machine took in this session, (node, terminal) to request: a refusal read in a later turn
    /// for typing sent before it is old news (answers can come out of order).
    typing_taken: Taken,
    /// ADR-0095: machines that answered a "Give to an agent" in a way that says they do not take tasks (`board::lacks_give`); their
    /// agents are not offered again until the machine goes offline or comes back with another build, or the link to the hub is lost
    /// (as the web window does, #885).
    give_lacking: agora_tui::board::Lacking,
}

/// A notice too long for the footer is shown whole in a box above it until the next key (the trust command must be readable and
/// copyable); `c` copies the command in it, when it has one.
#[derive(Default)]
struct Note {
    /// The notice the box was opened for (a new notice opens it again).
    seen: String,
    open: bool,
    /// The box has been on screen: only a key after that closes it (a key in the same burst as the command must not).
    drawn: bool,
    /// What `c` puts on the clipboard: the `agora node trust` command the notice asks the user to run.
    copy: Option<String>,
}

/// The `agora node trust` command this terminal's notices ask the user to run on a machine.
fn trust_command() -> Option<String> {
    device().map(|d| format!("agora node trust {} {}", d.id, d.public_key_hex()))
}

/// The terminal reports mouse presses, drags and the wheel (SGR encoding) while the console holds the mouse, and not while a screen a
/// question draws itself reads plain bytes. Pastes are framed independently of this ([`bracketed_paste`]), so a paste that began
/// before a question stays one while the mouse is off.
fn mouse_mode(on: bool) {
    let mut out = std::io::stdout();
    // Motion with no button (1003) is only switched on while the open program asked for it (`mouse_motion`): never by this.
    HOST_MOTION.store(false, std::sync::atomic::Ordering::Relaxed);
    write_mouse(&mut out, on);
}

/// Writes [`mouse_sequence`] to `out`.
fn write_mouse(out: &mut impl Write, on: bool) {
    let _ = out.write_all(mouse_sequence(on));
    let _ = out.flush();
}

/// What [`mouse_mode`] writes: the mouse modes only, never the paste framing (DECSET 2004).
fn mouse_sequence(on: bool) -> &'static [u8] {
    if on {
        b"\x1b[?1003l\x1b[?1000h\x1b[?1002h\x1b[?1006h"
    } else {
        b"\x1b[?1003l\x1b[?1000l\x1b[?1002l\x1b[?1006l"
    }
}

/// Whether this terminal frames pastes (DECSET 2004) right now.
static HOST_PASTE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Asks this terminal to frame pastes (2004) or not; returns whether it did before, to put back.
fn bracketed_paste(on: bool) -> bool {
    set_paste(&mut std::io::stdout(), &HOST_PASTE, on)
}

/// [`bracketed_paste`] on any writer and state: writes DECSET 2004 only when `on` differs from what `state` says it is.
fn set_paste(out: &mut impl Write, state: &std::sync::atomic::AtomicBool, on: bool) -> bool {
    let was = state.swap(on, std::sync::atomic::Ordering::Relaxed);
    if was != on {
        let _ = out.write_all(if on { b"\x1b[?2004h" } else { b"\x1b[?2004l" });
        let _ = out.flush();
    }
    was
}

/// Whether this terminal is reporting motion with no button (DECSET 1003) right now.
static HOST_MOTION: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Asks this terminal for motion with no button while `on` (the open program switched on "any motion" mouse reporting and the
/// mouse is held), and stops asking when it is not: a terminal that reports every movement is a load nobody wants for nothing.
fn mouse_motion(on: bool) {
    if HOST_MOTION.swap(on, std::sync::atomic::Ordering::Relaxed) != on {
        let mut out = std::io::stdout();
        let _ = out.write_all(if on { b"\x1b[?1003h" } else { b"\x1b[?1003l" });
        let _ = out.flush();
    }
}

fn window_area() -> Area {
    let (cols, rows) = agora_platform::terminal_size().unwrap_or((100, 30));
    Area::new(0, 0, cols, rows)
}

/// ADR-0063: the console on the app shell. The tree (machines, agents), the open agents as tabs with their live terminals (typing goes
/// to the agent; Ctrl-] or F12 back), the inspector, and the letters of the earlier console for the selected agent (y / n answer, s
/// start, f folder, x stop, u / U resume, d remove), whose questions are asked in a box over the console; the folder picker and the
/// reader of a long start request still take the whole screen.
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
    // The open chat agent's conversation (one at a time, the active tab's), and an agent whose conversation could not be opened (not
    // tried again until another tab comes to the front).
    let mut chat: Option<ChatPane> = None;
    // A conversation that could not be opened, and when: tried again after RETRY_OPEN (its machine may be back after a reconnect).
    let mut chat_failed: Option<(String, Instant)> = None;
    // The same for a terminal.
    let mut open_tried: Option<(String, Instant)> = None;
    // How many things needed the user at the last turn: a new one clears an old notice so the footer's summary shows it.
    let mut last_waiting = 0usize;

    let mut redraw = true;
    let mut last_draw = Instant::now() - Duration::from_secs(1);
    let mut area = window_area();
    // The agent whose terminal was last sized to the pane, so a tab brought to the front is sized once.
    let mut sized: Option<(String, u16, u16)> = None;
    mouse_mode(c.app.mouse);
    // The console can ask at any time, so pastes are framed for as long as it runs (never tied to the mouse).
    bracketed_paste(true);
    let mut fresh = true;
    let mut typed_at: Option<Instant> = None;
    // A refusal is matched by its session's request numbers, which a new session (a reconnect, also a routine refresh) starts over:
    // a pending refusal stays (typing is still refused there), and any typing the new session gets taken ends it.
    carry_refusal(
        &mut c.typing_refused,
        &mut c.typing_allowed,
        &mut c.typing_taken,
    );
    // ADR-0065: only the hub knows whether this session is the owner's (it answers `PairStatus` to the owner only).
    if c.pair.owner == agora_tui::pair::Owner::Unknown {
        if let Some((owner, role)) = pair_owner(session) {
            c.pair.set_owner(owner);
            c.role = role;
        }
    }
    let mut pair_seen = c.pair.shown().len();
    loop {
        // Watching a terminal: wake often, so typing echoes and output looks live; with a frame waiting to be drawn, wake when it may
        // be (any frame from the hub wakes this at once anyway).
        let mut wait = Duration::from_millis(if preview.is_some() || chat.is_some() {
            15
        } else {
            50
        });
        if redraw {
            wait = wait.min(FRAME.saturating_sub(last_draw.elapsed()));
        }
        // Input outranks output (ADR-0022): keys arrive on another channel than the hub's frames, so while the user types (a key in the
        // last second) the wait is short enough that a key is read within a couple of milliseconds; idle, the console sleeps longer.
        if typed_at.is_some_and(|t| t.elapsed() < Duration::from_secs(1)) {
            wait = wait.min(Duration::from_millis(2));
        }
        match session.next(wait) {
            Update::Snapshot(s) => {
                // A machine that is not connected now, or came back with another build, is offered tasks again.
                c.give_lacking.settle(&s);
                c.snapshot = Some(*s);
                dirty_since = None;
                redraw = true;
            }
            Update::Changed => {
                dirty_since.get_or_insert_with(Instant::now);
            }
            Update::Quiet => {}
            Update::Closed => {
                show_notice(say, c, say.text(key!("tui-lost"), hub_arg));
                std::thread::sleep(Duration::from_secs(2));
                return Next::Reconnect;
            }
        }
        // ADR-0065: the cards the hub sent an owner's window, and what became of the ones it ended.
        for note in session.take_pair_notes() {
            c.pair.apply(note);
        }
        let waiting_cards = c.pair.shown().len();
        if waiting_cards > pair_seen && !c.note.open {
            c.notice = pair_asked_line(say, waiting_cards);
            redraw = true;
        }
        pair_seen = waiting_cards;
        if let Some((name, ended)) = c.pair.take_end() {
            c.notice = pair_result_line(say, &name, ended);
            redraw = true;
        }
        let rows = c
            .snapshot
            .as_ref()
            .map(|s| app_rows(s, now_ms(), &text))
            .unwrap_or_default();
        if let Some(i) = rows.iter().position(|r| !r.header && r.id == c.selected) {
            c.app.selected = i;
        } else if let Some(id) = c.app.settle_selection(&rows) {
            // Nothing selected yet (the start) or the selected agent is gone: the row the tree shows chosen is the one the inspector
            // and the letters act on, never a heading or nothing.
            c.selected = id.to_string();
            redraw = true;
        }
        // Motion with no button is asked of this terminal only while the program in front asked for it.
        mouse_motion(
            c.app.mouse
                && preview
                    .as_ref()
                    .filter(|p| Some(p.agent.as_str()) == c.app.active_agent())
                    .is_some_and(|p| p.program_modes().track == agora_tui::termio::Track::Any),
        );
        let mut events = Vec::new();
        let now = window_area();
        if now != area {
            // The window took exactly the size last given to the open tab's terminal: this client runs in that terminal
            // (`agora tui` started in an agent's shell, then that agent opened). Sizing it again would shrink this window again,
            // down to nothing (seen on Windows: 1x17 for good), so that agent's terminal is left at its size from now on. A pane is
            // always smaller than the window, so this never matches an ordinary resize.
            if let Some((agent, w, h)) = sized.as_ref() {
                if (*w, *h) == (now.width, now.height) && !c.not_sized.contains(agent) {
                    c.not_sized.insert(agent.clone());
                    c.notice = say.text(key!("tui-app-self-terminal"), &[]);
                }
            }
            area = now;
            let _ = c.window.resized();
            // `resized` clears the screen: draw it again now, not at the next key (seen on Windows: blank after a resize).
            redraw = true;
            events.push(AppEvent::Resize {
                cols: area.width,
                rows: area.height,
            });
        }
        let typed = read_keys(
            rx,
            &mut c.parser,
            &mut typed_at,
            Instant::now(),
            &mut events,
        );
        if !typed && terminal_gone() {
            if let Some(p) = preview.take() {
                p.close(session);
            }
            return Next::Quit;
        }
        // The footer's question about an address goes when the footer says something else or runs out.
        c.links.check(Instant::now(), &c.notice, NOTICE_FOR);
        // Any key or click closes the box of a long notice (and still does what it does); nothing reaches the program under it.
        let overlay = c.note.open && c.note.drawn && !events.is_empty();
        if overlay {
            c.note.open = false;
            redraw = true;
        }
        c.asked = false;
        'batch: for (index, ev) in events.into_iter().enumerate() {
            // Keys typed with the one that opened a question (a command, a chat send) came before it was seen: they are dropped,
            // not handled after it closes (a `y` among them could otherwise answer another request in view).
            if std::mem::take(&mut c.asked) {
                redraw = true;
                break 'batch;
            }
            // A key that only goes to the agent changes nothing on screen until the agent answers: drawing for it would hold the
            // echo back by up to a frame (ADR-0022). Anything else is drawn.
            let before = c.app.clone();
            let modes = preview
                .as_ref()
                .map(|p| p.screen().modes())
                .unwrap_or_default();
            // The program's mouse and focus modes, only of the agent in front (never the one a tab switch just left). The event that
            // closes a box over the pane is not the program's (`covered`: the app also ends a press the program has, with a release).
            let prog = preview
                .as_ref()
                .filter(|p| Some(p.agent.as_str()) == c.app.active_agent())
                .map(Preview::program_modes)
                .unwrap_or_default();
            let covered = overlay && index == 0;
            // The release of the Ctrl+click that showed an address is that click's own; anything else but a Ctrl+click forgets it.
            let ctrl_click = matches!(&ev, AppEvent::Mouse(m)
                if m.kind == agora_tui::app::MouseKind::Down
                    && m.button == 0
                    && m.mods & agora_tui::termio::CTRL != 0);
            match &ev {
                AppEvent::Mouse(m) if m.kind == agora_tui::app::MouseKind::Up => c.links.release(),
                _ if ctrl_click => {}
                _ => c.links.other(),
            }
            // Pointer motion the program asked for is passed on by the console, not typed: a refusal of it asks nothing.
            let passive = passive_input(&ev);
            let acts = c.app.handle(
                ev,
                &Ctx {
                    rows: &rows,
                    area,
                    modes,
                    prog,
                    covered,
                    t: &text,
                },
            );
            if ctrl_click && !acts.iter().any(|a| matches!(a, Action::OpenLink { .. })) {
                c.links.other();
            }
            // The panes that gained or lost the keyboard: a program that asked for focus events (1004) is told.
            for (agent, gained) in c.app.take_pane_focus() {
                let asked = preview
                    .as_ref()
                    .filter(|p| p.agent == agent)
                    .and_then(|p| agora_tui::termio::focus_bytes(&p.program_modes(), gained));
                if let (Some(bytes), Some((node, local))) = (asked, agent.split_once('/')) {
                    session.terminal_report(node, &format!("t-{local}"), bytes);
                }
            }
            if c.app != before || acts.iter().any(|a| !matches!(a, Action::Input(_))) {
                redraw = true;
            }
            c.selected = rows
                .get(c.app.selected)
                .filter(|r| !r.header)
                .map(|r| r.id.clone())
                .unwrap_or_default();
            for act in acts {
                match act {
                    Action::Input(b) => {
                        let active = c.app.active_agent().map(str::to_string);
                        match chat.as_mut().filter(|p| Some(&p.agent) == active.as_ref()) {
                            // Typing in a chat never answers a request, whatever letters it holds (the TUI review: `maybe`
                            // allowed one): a waiting request is answered with Alt+Y / Alt+N only, and the draft is kept.
                            Some(pane) => {
                                chat_input(say, session, rx, c, pane, &b);
                                if pane.typed.is_empty() {
                                    c.drafts.remove(&pane.agent);
                                } else {
                                    c.drafts.insert(pane.agent.clone(), pane.typed.clone());
                                }
                                // What was typed is drawn: the shell's own state did not change, the pane's did.
                                redraw = true;
                            }
                            None => {
                                if let Some((node, local)) =
                                    active.as_deref().and_then(|a| a.split_once('/'))
                                {
                                    let t = format!("t-{local}");
                                    if passive {
                                        session.terminal_report(node, &t, &b);
                                    } else {
                                        session.terminal_input(node, &t, &b);
                                    }
                                }
                            }
                        }
                    }
                    // The release of a press that ended with its pane: it goes to that agent's terminal, in front or not.
                    Action::InputFor { agent, bytes } => {
                        if let Some((node, local)) = agent.split_once('/') {
                            session.terminal_input(node, &format!("t-{local}"), &bytes);
                        }
                    }
                    Action::PaneSize { .. } => sized = None,
                    Action::MouseCapture(on) => mouse_mode(on),
                    // Ctrl+click on a web address: the first click shows exactly what would open, the second one on the same
                    // address opens it. Never a plain click, never without showing it.
                    Action::OpenLink { row, col } => {
                        let height = c.app.areas(area).terminal.height;
                        let link = preview
                            .as_ref()
                            .filter(|p| Some(p.agent.as_str()) == c.app.active_agent())
                            .and_then(|p| {
                                agora_tui::pane::link_at(p.screen(), height, c.app.scroll, row, col)
                            });
                        match link {
                            Some(url) => {
                                let asked = say.text(
                                    key!("tui-link-confirm"),
                                    &[("url", Arg::Str(url.clone()))],
                                );
                                match c.links.click(&url, &asked, Instant::now(), NOTICE_FOR) {
                                    agora_tui::termio::LinkStep::Confirm => {
                                        c.notice = asked;
                                        // Shown again: the footer's clock starts over.
                                        c.notice_at = Instant::now();
                                    }
                                    agora_tui::termio::LinkStep::Open => {
                                        c.notice = if agora_platform::open_url(&url).is_ok() {
                                            say.text(
                                                key!("tui-link-opened"),
                                                &[("url", Arg::Str(url))],
                                            )
                                        } else {
                                            say.text(
                                                key!("tui-link-failed"),
                                                &[("url", Arg::Str(url))],
                                            )
                                        };
                                    }
                                }
                                redraw = true;
                            }
                            None => c.links.other(),
                        }
                    }
                    // Closing a tab only closes the view: the agent keeps running, and the footer says so.
                    Action::Close(id) => {
                        let name = c
                            .snapshot
                            .as_ref()
                            .map_or_else(|| id.clone(), |s| agora_tui::split::agent_label(s, &id));
                        c.notice = say.text(key!("tui-tab-closed"), &[("agent", Arg::Str(name))]);
                    }
                    // Text selected in the agent's pane goes to the clipboard through the terminal (OSC 52: over SSH too).
                    Action::Copy(sel) => {
                        let height = c.app.areas(area).terminal.height;
                        let text = preview
                            .as_ref()
                            .filter(|p| Some(p.agent.as_str()) == c.app.active_agent())
                            .map(|p| {
                                agora_tui::pane::selected_text(
                                    p.screen(),
                                    height,
                                    c.app.scroll,
                                    sel,
                                )
                            })
                            .unwrap_or_default();
                        if !text.is_empty() {
                            let mut out = std::io::stdout();
                            let _ = out.write_all(&agora_tui::clip::osc52(&text));
                            let _ = out.flush();
                            c.notice = say.text(
                                key!("tui-copied-text"),
                                &[("count", Arg::Num(text.chars().count() as i64))],
                            );
                        }
                    }
                    // The wheel moved the tab's view (App.scroll): never further back than its screen and history hold.
                    Action::Scroll(_) => {
                        let height = c.app.areas(area).terminal.height;
                        if let Some(p) = preview
                            .as_ref()
                            .filter(|p| Some(p.agent.as_str()) == c.app.active_agent())
                        {
                            c.app.scroll = c.app.scroll.min(
                                agora_tui::pane::TerminalPane::max_scroll(p.screen(), height),
                            );
                        }
                    }
                    Action::Open(_) => {}
                    Action::Quit => {
                        if let Some(p) = preview.take() {
                            p.close(session);
                        }
                        mouse_mode(false);
                        bracketed_paste(false);
                        return Next::Quit;
                    }
                    Action::CleanUpOld => {
                        c.notice = clean_up_old(say, session, c, rx);
                        dirty_since.get_or_insert_with(Instant::now);
                    }
                    Action::Command(k) => {
                        // A command's result replaces the last notice: the same long result (the trust command again) opens its
                        // box again instead of staying cut in the footer.
                        c.notice.clear();
                        c.note.seen.clear();
                        if command(say, session, rx, color, c, area, k)
                            | std::mem::take(&mut c.stale)
                        {
                            dirty_since.get_or_insert_with(Instant::now);
                        }
                    }
                    // Alt+Y, Alt+N, Alt+A in an agent's tab: exactly that tab's request, never the tree's selection or another one.
                    Action::Answer {
                        agent,
                        approval,
                        answer,
                    } => {
                        c.notice.clear();
                        c.note.seen.clear();
                        // A chat tab shows its agent's request in place of the input line: it is in view there only when all of it,
                        // the always lines included, fits the pane as drawn (otherwise the box shows it and takes the answer).
                        if let (Some(p), Some(a)) = (
                            chat.as_ref().filter(|p| p.agent == agent),
                            c.snapshot
                                .as_ref()
                                .and_then(|s| {
                                    agora_tui::client::exact_approval(s, &agent, &approval)
                                })
                                .cloned(),
                        ) {
                            let pane = c.app.areas(area).terminal;
                            let head = p
                                .snapshot
                                .as_ref()
                                .and_then(|x| session_line(x, &|id, args| say.text(id, args)))
                                .is_some();
                            let width = usize::from(pane.width).max(12);
                            let note_rows = if p.note.is_empty() {
                                0
                            } else {
                                agora_tui::render::wrap_row(&plain(&p.note), width, width).len()
                            };
                            c.shown_request = chat_shows_request(
                                say,
                                &a,
                                (pane.width, pane.height),
                                head,
                                note_rows,
                            )
                            .then(|| approval.clone());
                        }
                        let target = Target::Tab { agent, approval };
                        if answer_command(say, session, rx, c, area, &target, answer)
                            | std::mem::take(&mut c.stale)
                        {
                            dirty_since.get_or_insert_with(Instant::now);
                        }
                    }
                }
            }
        }
        // While typing stays refused, its notice stays (or comes back after another one) so the key that asks is never hidden.
        if c.notice.is_empty() && c.typing_refused.is_some() {
            c.notice = c.typing_notice.clone();
        }
        if !c.notice.is_empty()
            && c.notice == c.note.seen
            // A short notice never gets a box to close (`drawn` stays false): it ends with its time too.
            && (!c.note.open || !c.note.drawn)
            && c.notice_at.elapsed() >= NOTICE_FOR
            && !(c.typing_refused.is_some() && c.notice == c.typing_notice)
        {
            // An old notice ("Allowed.") gives the footer back to the summary of machines, agents and what needs the user.
            c.notice.clear();
        }
        if c.notice != c.note.seen {
            c.notice_at = Instant::now();
            c.note.seen = c.notice.clone();
            c.note.open = !c.notice.is_empty();
            c.note.drawn = false;
            c.note.copy = trust_command().filter(|cmd| c.notice.contains(cmd.as_str()));
            redraw = true;
        }
        if let Some(s) = &c.snapshot {
            follow(
                session,
                s,
                c.app.active_agent().unwrap_or(""),
                &mut preview,
                &mut open_tried,
            );
            match follow_chat(
                session,
                s,
                c.app.active_agent(),
                &mut chat,
                &mut chat_failed,
                &c.drafts,
            ) {
                Err(code) => {
                    c.notice = say.text(key!("tui-opening-failed"), &[("code", Arg::Str(code))]);
                    c.open_failed_notice = Some(c.notice.clone());
                    redraw = true;
                }
                // Opened after all (its machine is back): the old "could not open" no longer holds.
                Ok(true) if c.open_failed_notice.take().is_some_and(|n| n == c.notice) => {
                    c.notice.clear();
                    redraw = true;
                }
                Ok(_) => {}
            }
            // A new request clears an old notice ("Allowed.") so the footer's summary says it.
            let waiting = needs_you_count(s);
            if waiting > last_waiting && !c.note.open {
                c.notice.clear();
                redraw = true;
            }
            last_waiting = waiting;
        }
        if let Some(pane) = chat.as_mut() {
            match session.chat_next(pane.stream, Duration::from_millis(1)) {
                ChatEvent::Lines(lines, replace) => {
                    if replace {
                        pane.entries = lines;
                    } else {
                        let last = pane.entries.last().map_or(0, |e| e.entry_id);
                        pane.entries
                            .extend(lines.into_iter().filter(|e| e.entry_id > last));
                    }
                    redraw = true;
                }
                ChatEvent::Ended => {
                    if let Some(p) = chat.take() {
                        session.terminal_close(p.stream);
                    }
                    redraw = true;
                }
                ChatEvent::Lost => {
                    show_notice(say, c, say.text(key!("tui-lost"), hub_arg));
                    std::thread::sleep(Duration::from_secs(2));
                    return Next::Reconnect;
                }
                ChatEvent::Quiet => {}
            }
        }
        // The open agent's terminal takes the pane's size (as `agora terminal attach` takes the window's), once per agent and size.
        let pane = c.app.areas(area).terminal;
        if let Some(active) = c.app.active_agent() {
            let want = (active.to_string(), pane.width, pane.height);
            if sized.as_ref() != Some(&want)
                && pane.width > 0
                && pane.height > 0
                && !c.not_sized.contains(active)
            {
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
                    show_notice(say, c, say.text(key!("tui-lost"), hub_arg));
                    std::thread::sleep(Duration::from_secs(2));
                    return Next::Reconnect;
                }
            }
        }
        // ADR-0041: typing into the agent in front was refused until its owner allows it. The footer says so and names the key
        // that asks (Ctrl-] then g); nothing opens by itself, so no key typed to the agent can answer a question. The typed keys are
        // lost. Another tab's refusal is dropped (typing into it again says it there).
        let refused = session.take_grant_wanted();
        for (node, terminal, request) in session.take_typing_taken() {
            let newest = c.typing_taken.entry((node, terminal)).or_insert(request);
            *newest = (*newest).max(request);
        }
        match typing_refusal(
            &mut c.typing_refused,
            &refused,
            &c.typing_taken,
            c.app.active_agent(),
            c.typing_allowed.as_ref(),
        ) {
            Notice::Show => {
                let agent = c
                    .typing_refused
                    .as_ref()
                    .map(|p| p.agent.clone())
                    .unwrap_or_default();
                let name = c.snapshot.as_ref().map_or_else(
                    || agent.clone(),
                    |s| agora_tui::split::agent_label(s, &agent),
                );
                c.typing_notice =
                    say.text(key!("tui-typing-needs-ok"), &[("agent", Arg::Str(name))]);
                c.notice = c.typing_notice.clone();
                // Said again even when the same words were said before (a new refusal after the last one ended).
                c.note.seen.clear();
                redraw = true;
            }
            Notice::End => {
                if c.notice == c.typing_notice {
                    c.notice.clear();
                    redraw = true;
                }
            }
            Notice::Same => {}
        }
        // At most one frame per FRAME (ADR-0022: an echo within 16 ms), however fast the agent writes; input was handled before this.
        // The first frame of a run is drawn over a cleared screen: lines written meanwhile ("connecting…", a lost link) must not stay
        // under it.
        if redraw && last_draw.elapsed() >= FRAME {
            if std::mem::take(&mut fresh) {
                let _ = c.window.invalidate();
            }
            draw_console(say, c, &rows, preview.as_ref(), chat.as_ref(), area, None);
            last_draw = Instant::now();
            redraw = false;
        }
        if dirty_since.is_some() && refresh_due(last_refresh.elapsed(), session.typing_unanswered())
        {
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

/// What a new session (a reconnect, also a routine refresh) keeps of the typing refusals: the pending one stays, renumbered, since
/// typing is still refused there; what was allowed or taken was counted in the old session's numbers and ends.
fn carry_refusal(
    refused: &mut Option<TypingRefused>,
    allowed: &mut Option<(String, u64)>,
    taken: &mut Taken,
) {
    if let Some(p) = refused.as_mut() {
        p.request = 0;
    }
    *allowed = None;
    taken.clear();
}

/// Typing into `agent` (`node/local`), the agent in front, was refused for want of a grant (its newest refused `request`).
#[derive(Clone, Debug, PartialEq, Eq)]
struct TypingRefused {
    agent: String,
    request: u64,
}

impl TypingRefused {
    fn is_for(&self, node: &str, terminal_id: &str) -> bool {
        terminal_id
            .strip_prefix("t-")
            .is_some_and(|local| self.agent.split_once('/') == Some((node, local)))
    }
}

/// The newest typing each terminal's machine took: (node, terminal) to request.
type Taken = std::collections::HashMap<(String, String), u64>;

/// What the footer does about a typing refusal this turn.
#[derive(Debug, PartialEq, Eq)]
enum Notice {
    /// Say that typing into the agent in front needs the user's OK, and which key asks.
    Show,
    /// The refusal no longer holds: take the notice away.
    End,
    Same,
}

/// ADR-0041: what this turn's answers to typing change. A refusal for the agent in front starts the notice; it ends when another tab
/// comes to the front or that agent's machine takes typing sent after the refusal (the grant was given, here or elsewhere). Nothing
/// is ever asked from here: the question opens only on its own key ([`grant_key_target`]).
fn typing_refusal(
    pending: &mut Option<TypingRefused>,
    refused: &[agora_tui::client::Refused],
    taken: &Taken,
    active: Option<&str>,
    allowed: Option<&(String, u64)>,
) -> Notice {
    let taken_after = |node: &str, terminal: &str, request: u64| {
        taken
            .iter()
            .any(|((n, t), r)| n == node && t == terminal && *r > request)
    };
    let mut out = Notice::Same;
    if pending.as_ref().is_some_and(|p| {
        Some(p.agent.as_str()) != active
            || taken
                .iter()
                .any(|((n, t), r)| p.is_for(n, t) && *r > p.request)
    }) {
        *pending = None;
        out = Notice::End;
    }
    let in_front = refused.iter().rev().find(|r| {
        let agent = r
            .terminal_id
            .strip_prefix("t-")
            .map(|local| format!("{}/{local}", r.node));
        agent.as_deref().is_some_and(|a| Some(a) == active)
            && !taken_after(&r.node, &r.terminal_id, r.request)
            // Typing sent before it was allowed here, refused late: not a new refusal.
            && !allowed.is_some_and(|(a, last)| agent.as_deref() == Some(a.as_str()) && r.request <= *last)
    });
    if let (Some(r), Some(agent)) = (in_front, active) {
        match pending {
            Some(p) => p.request = p.request.max(r.request),
            None => {
                *pending = Some(TypingRefused {
                    agent: agent.to_string(),
                    request: r.request,
                });
                out = Notice::Show;
            }
        }
    }
    out
}

/// The agent (node, local id) the grant key asks for: the one in front, and only while its typing is refused. `None`: the key does
/// nothing (no grant is wanted there).
fn grant_key_target(
    pending: Option<&TypingRefused>,
    active: Option<&str>,
) -> Option<(String, String)> {
    let p = pending.filter(|p| Some(p.agent.as_str()) == active)?;
    let (node, local) = p.agent.split_once('/')?;
    Some((node.to_string(), local.to_string()))
}

/// Whether what `ev` sends to the program is passed on by the console rather than typed: pointer motion with no button (a press, a
/// drag, the wheel, keys and pastes are the user's).
fn passive_input(ev: &AppEvent) -> bool {
    matches!(ev, AppEvent::Mouse(m) if m.kind == agora_tui::app::MouseKind::Move)
}

/// How long after its last byte an unfinished escape sequence is read as the keys typed (a lone ESC as Esc). The reader thread hands
/// bytes over one at a time, so a turn of the loop can come between two bytes of one key; only a pause this long ends a sequence.
const KEY_GAP: Duration = Duration::from_millis(10);

/// The bytes the keyboard's reader thread handed over since the last turn, read as the console's events (added to `events`), with
/// `typed_at` moved to `now` when any came. Returns whether any came.
fn read_keys(
    rx: &mpsc::Receiver<u8>,
    parser: &mut Parser,
    typed_at: &mut Option<Instant>,
    now: Instant,
    events: &mut Vec<AppEvent>,
) -> bool {
    let mut bytes = Vec::new();
    while let Ok(b) = rx.try_recv() {
        bytes.push(b);
    }
    if bytes.is_empty() {
        // A turn that found nothing is not yet a pause (an arrow was read as Esc and `[A`): flush once KEY_GAP passed.
        if typed_at.is_none_or(|t| now.saturating_duration_since(t) >= KEY_GAP) {
            events.extend(parser.flush());
        }
        return false;
    }
    *typed_at = Some(now);
    events.extend(parser.push(&bytes));
    true
}

fn draw_console(
    say: &Say,
    c: &mut Console,
    rows: &[Row],
    preview: Option<&Preview>,
    chat: Option<&ChatPane>,
    area: Area,
    dialog: Option<&[String]>,
) {
    let text = |id: &str, args: &[(&str, Arg)]| say.text(id, args);
    let details = c
        .snapshot
        .as_ref()
        .map(|s| inspector_details(c, s, area, &text).lines)
        .unwrap_or_default();
    // The footer's notice: the last thing done, or the hub's counts (machines, agents, what needs the user).
    let notice = if c.notice.is_empty() {
        c.snapshot.as_ref().map_or_else(String::new, |s| {
            let waiting = needs_you_count(s) + c.pair.shown().len();
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
    // A chat agent's conversation as it fits the pane: what it runs with first, then its lines, a note, and the line being typed.
    let pane = c.app.areas(area).terminal;
    let conversation = chat
        .filter(|p| Some(p.agent.as_str()) == c.app.active_agent())
        .map(|p| {
            let local = p.agent.split_once('/').map_or(p.agent.as_str(), |(_, l)| l);
            let head = p
                .snapshot
                .as_ref()
                .and_then(|a| session_line(a, &text))
                .map(|l| plain(&l));
            let room = usize::from(pane.height)
                .saturating_sub(usize::from(head.is_some()) + usize::from(!p.note.is_empty()));
            // A request of this agent that waits takes the input line's place: what it asks, and the keys that answer it.
            let waiting: Vec<String> = c
                .snapshot
                .as_ref()
                .and_then(|s| waiting_for(s, &p.agent))
                .map(|a| chat_request_block(say, a, usize::from(pane.width)).0)
                .unwrap_or_default();
            let room = room.saturating_sub(waiting.len());
            // The agent speaks under its name (as in the tree), not its id.
            let speaker = p.snapshot.as_ref().map_or_else(
                || local.to_string(),
                |a| agora_tui::split::agent_name(&a.agent_id, &a.kind),
            );
            let mut lines = render_chat(
                &p.entries,
                &speaker,
                &String::from_utf8_lossy(&p.typed),
                (usize::from(pane.width), room.max(2)),
                false,
                &text,
            );
            if !p.note.is_empty() {
                let at = lines.len().saturating_sub(1);
                let width = usize::from(pane.width).max(12);
                lines.splice(
                    at..at,
                    agora_tui::render::wrap_row(&plain(&p.note), width, width),
                );
            }
            // The request goes above the line being typed, which stays in view (the draft is kept while it waits).
            if !waiting.is_empty() {
                let at = lines.len().saturating_sub(1);
                lines.splice(at..at, waiting);
            }
            if let Some(h) = head {
                lines.insert(0, h);
            }
            lines
        });
    // A notice too long for the footer, whole in a box while it is new.
    let note: Option<Vec<String>> =
        (c.note.open && notice.chars().count() > usize::from(area.width) * 2 / 3).then(|| {
            let mut lines = lines_of(&notice);
            lines.push(say.text(
                if c.note.copy.is_some() {
                    key!("tui-note-copy")
                } else {
                    key!("tui-note-close")
                },
                &[],
            ));
            lines
        });
    if note.is_some() {
        c.note.drawn = true;
    }
    let (app, colors) = (&c.app, c.colors);
    let drawn = c.window.draw(|f| {
        app.draw(
            f,
            &AppScreen {
                rows,
                terminal: term,
                details: &details,
                notice: &notice,
                no_screen,
                conversation: conversation.as_deref(),
                dialog,
                note: note.as_deref(),
                colors,
                t: &text,
            },
        );
    });
    // The footer with the question about an address is on screen now: a second Ctrl+click may open it.
    if drawn.is_ok() && !c.notice.is_empty() {
        c.links.drawn();
    }
}

/// How many things need the user: each agent that waits (blocked, or with a request) once, however many ways it waits, and each
/// request that belongs to no agent.
fn needs_you_count(s: &Snapshot) -> usize {
    let mut agents: std::collections::HashSet<&str> = s
        .agents
        .iter()
        .filter(|a| {
            a.running
                && a.state == agora_protocol::v1::AgentState::Blocked as i32
                // An agent of an offline machine is not known to wait now (its row says the machine is offline).
                && !s.nodes.iter().any(|n| n.node_id == a.node_id && !n.online)
        })
        .map(|a| a.agent_id.as_str())
        .collect();
    let mut other = 0;
    for a in &s.approvals {
        if a.agent_id.is_empty() {
            other += 1;
        } else {
            agents.insert(a.agent_id.as_str());
        }
    }
    agents.len() + other
}

/// The request of `agent` that waits for an answer, if one does (only its own, never another agent's).
fn waiting_for<'a>(
    s: &'a Snapshot,
    agent: &str,
) -> Option<&'a agora_protocol::v1::ApprovalSnapshot> {
    s.approvals.iter().find(|a| a.agent_id == agent)
}

/// The block a chat agent's request takes in place of its input line: what it asks (at most four rows), ADR-0101's always lines (the
/// agent's own words and what "always" means) and the keys that answer. The flag says whether the request's own text is there whole.
fn chat_request_block(
    say: &Say,
    a: &agora_protocol::v1::ApprovalSnapshot,
    width: usize,
) -> (Vec<String>, bool) {
    let text = |id: &str, args: &[(&str, Arg)]| say.text(id, args);
    let width = width.max(12);
    let what = plain(format!("{} {}", a.action, a.detail).trim());
    let rows = agora_tui::render::wrap_row(&what, width, width);
    let whole = rows.len() <= CHAT_REQUEST_ROWS;
    let mut block = vec![say.text(key!("tui-attention"), &[])];
    block.extend(rows.into_iter().take(CHAT_REQUEST_ROWS));
    for l in agora_tui::render::always_lines(a, &text) {
        block.extend(agora_tui::render::wrap_row(&l, width, width));
    }
    block.extend(agora_tui::render::wrap_row(
        &say.text(
            agora_tui::client::answer_hint(
                a,
                key!("tui-chat-answer"),
                key!("tui-chat-answer-always"),
                key!("tui-chat-answer-always-only"),
            ),
            &[],
        ),
        width,
        width,
    ));
    (block, whole)
}

/// The most rows a chat agent's request takes of its pane; a longer one is read in the box.
const CHAT_REQUEST_ROWS: usize = 4;

/// Whether a chat pane of `width` x `height` shows `a`'s block whole (Codex's re-review of ADR-0101 part B): the request's text uncut
/// and every line of the block on screen, under the pane's head line (`head`) and note (`note`) and above at least the input line and
/// one conversation row (the conversation keeps two rows; more lines than the pane are cut at its bottom). Only then is the request in
/// view there; otherwise its answer is taken in the box, which shows it all.
fn chat_shows_request(
    say: &Say,
    a: &agora_protocol::v1::ApprovalSnapshot,
    (width, height): (u16, u16),
    head: bool,
    note_rows: usize,
) -> bool {
    let (block, whole) = chat_request_block(say, a, usize::from(width));
    whole && usize::from(head) + note_rows + block.len() + 2 <= usize::from(height)
}

/// A chat agent's conversation shown in the console's pane (ADR-0063): its stream, what came, and the line being typed.
struct ChatPane {
    agent: String,
    stream: u32,
    entries: Vec<ChatEntry>,
    typed: Vec<u8>,
    note: String,
    /// ADR-0058: the line that asked to confirm a mode that does not ask; typing it again switches.
    confirming: Option<String>,
    /// What it runs with, as opened and as switched here.
    snapshot: Option<agora_protocol::v1::AgentSnapshot>,
}

/// The conversation follows the active tab: opened when a chat agent's tab comes to the front, closed when another does. An agent
/// whose conversation could not be opened is not tried again until another tab has been in front.
fn follow_chat(
    session: &mut Session,
    s: &Snapshot,
    active: Option<&str>,
    chat: &mut Option<ChatPane>,
    failed: &mut Option<(String, Instant)>,
    drafts: &std::collections::HashMap<String, Vec<u8>>,
) -> Result<bool, String> {
    let wanted = active.filter(|id| {
        s.agents.iter().any(|a| a.agent_id == *id && a.running)
            && !s.terminals.iter().any(|t| t.agent_id == *id)
    });
    if chat.as_ref().map(|p| p.agent.as_str()) == wanted {
        return Ok(false);
    }
    if let Some(p) = chat.take() {
        session.terminal_close(p.stream);
    }
    let Some(id) = wanted else {
        *failed = None;
        return Ok(false);
    };
    if failed
        .as_ref()
        .is_some_and(|(f, at)| f == id && at.elapsed() < RETRY_OPEN)
    {
        return Ok(false);
    }
    let (node, local) = id.split_once('/').unwrap_or(("", id));
    match session.chat_open(node, local) {
        Ok(stream) => {
            *chat = Some(ChatPane {
                agent: id.to_string(),
                stream,
                entries: Vec::new(),
                typed: drafts.get(id).cloned().unwrap_or_default(),
                note: String::new(),
                confirming: None,
                snapshot: s.agents.iter().find(|a| a.agent_id == id).cloned(),
            });
            *failed = None;
            Ok(true)
        }
        Err(code) => {
            *failed = Some((id.to_string(), Instant::now()));
            Err(code)
        }
    }
}

/// One key (or a paste) typed into a chat agent's tab: text goes to the line, Backspace takes a character back, Enter sends it (or
/// switches the mode or model it names, as the full-screen chat did). Keys that are escape sequences (arrows) are ignored.
fn chat_input(
    say: &Say,
    session: &mut Session,
    rx: &mpsc::Receiver<u8>,
    c: &mut Console,
    pane: &mut ChatPane,
    bytes: &[u8],
) {
    if bytes.first() == Some(&0x1b) {
        return;
    }
    // A paste is one chunk of several bytes: its line breaks are spaces, it is sent with Enter like typed text.
    let paste = bytes.len() > 1 && std::str::from_utf8(bytes).is_ok_and(|t| t.chars().count() > 1);
    for &b in bytes {
        match b {
            b'\r' | b'\n' if paste => pane.typed.push(b' '),
            // A request of this agent waits: the draft stays until it is answered (Alt+Y / Alt+N), so no message goes out with a
            // permission pending.
            b'\r' | b'\n'
                if c.snapshot
                    .as_ref()
                    .and_then(|s| waiting_for(s, &pane.agent))
                    .is_some() =>
            {
                pane.note = say.text(key!("tui-chat-answer-first"), &[]);
            }
            b'\r' | b'\n' => chat_send(say, session, rx, c, pane),
            0x7f | 0x08 => {
                // Backspace removes one character, not one byte.
                while pane.typed.pop().is_some_and(|b| b & 0xC0 == 0x80) {}
            }
            b if b >= 0x20 => pane.typed.push(b),
            _ => {}
        }
    }
}

fn chat_send(
    say: &Say,
    session: &mut Session,
    rx: &mpsc::Receiver<u8>,
    c: &mut Console,
    pane: &mut ChatPane,
) {
    let line = String::from_utf8_lossy(&pane.typed).trim().to_string();
    pane.typed.clear();
    let (node, local) = pane.agent.split_once('/').unwrap_or(("", &pane.agent));
    let (node, local) = (node.to_string(), local.to_string());
    let scope = format!("agent:{local}");
    let asked = chat_command(&line, pane.snapshot.as_ref(), pane.confirming.as_deref());
    pane.confirming = None;
    pane.note = match asked {
        ChatCommand::Unknown { id, offered } => say.text(
            key!("tui-option-unknown"),
            &[("id", Arg::Str(id)), ("ids", Arg::Str(offered.join(", ")))],
        ),
        ChatCommand::Confirm { id } => {
            pane.confirming = Some(line);
            say.text(key!("tui-option-confirm"), &[("id", Arg::Str(id))])
        }
        ChatCommand::Switch { model, id } => {
            // The same per-agent grant the web client asks for: this one switch for this agent.
            let command = if model {
                "agent.set_model"
            } else {
                "agent.set_mode"
            };
            let tried = over_console(say, c, session, Some(pane), rx, |session, ask| {
                with_grant(say, session, ask, (&node, &scope, &[command]), |s| {
                    s.set_option(&node, &local, model, &id)
                })
            });
            match tried {
                Tried::Done(Ok(())) => {
                    if let Some(a) = pane.snapshot.as_mut() {
                        agora_tui::render::switched(a, model, &id);
                    }
                    say.text(key!("tui-option-switched"), &[])
                }
                Tried::Done(Err(code)) => {
                    say.text(key!("tui-send-failed"), &[("code", Arg::Str(code))])
                }
                Tried::Declined => String::new(),
                Tried::Line(l) => l,
            }
        }
        ChatCommand::Prompt if line.is_empty() => String::new(),
        ChatCommand::Prompt => {
            let tried = over_console(say, c, session, Some(pane), rx, |session, ask| {
                with_grant(
                    say,
                    session,
                    ask,
                    (&node, &scope, Site::ChatPrompt.commands()),
                    |s| s.prompt(&node, &local, &line),
                )
            });
            match tried {
                Tried::Done(Ok(())) | Tried::Declined => String::new(),
                Tried::Done(Err(code)) => {
                    say.text(key!("tui-send-failed"), &[("code", Arg::Str(code))])
                }
                Tried::Line(l) => l,
            }
        }
    };
}

/// Where a question is asked and its answer read: over the console as a dialog ([`Over`]), or over the whole screen in the earlier
/// split view ([`Plain`]).
trait Asker {
    /// The keys the user types.
    fn rx(&self) -> &mpsc::Receiver<u8>;
    /// Shows a question, one string per line.
    fn show(&mut self, lines: &[String]);
    /// Writes a screen the question drew itself (the folder picker, the reader of a start request).
    fn frame(&mut self, bytes: &[u8]);
    /// Shows `lines` and waits up to `wait` for one key (arrows as `read_key` reads them).
    fn key(&mut self, lines: &[String], wait: Duration) -> Option<u8> {
        self.show(lines);
        read_key(self.rx(), wait)
    }
    /// Sets aside every key typed so far.
    fn discard(&mut self) {
        while self.rx().try_recv().is_ok() {}
    }
    /// Shows `lines` once, sets aside what was typed until then, and waits up to `wait` for the answer to a question whose `y` signs
    /// something: one key or click, never a paste ([`one_answer`]).
    fn answer(&mut self, lines: &[String], wait: Duration) -> Option<u8> {
        self.show(lines);
        self.discard();
        read_answer(&mut Parser::default(), self.rx(), lines, wait)
    }
    /// Goes on waiting up to `wait` for the answer to the question [`Asker::answer`] put on the screen, which is still there as it
    /// was: nothing is drawn or set aside.
    fn answer_more(&mut self, lines: &[String], wait: Duration) -> Option<u8> {
        read_answer(&mut Parser::default(), self.rx(), lines, wait)
    }
}

/// The text of a question as lines.
fn lines_of(text: &str) -> Vec<String> {
    text.split('\n').map(str::to_string).collect()
}

/// The earlier split view's questions: each takes the whole screen. Its parser reads both the keys set aside and the answer, so a
/// paste around the question keeps its framing there too.
struct Plain<'a>(&'a mpsc::Receiver<u8>, Parser);

impl<'a> Plain<'a> {
    fn new(rx: &'a mpsc::Receiver<u8>) -> Self {
        Self(rx, Parser::default())
    }

    /// With a parser that already saw the keys forwarded before (a paste that began there goes on as a paste).
    fn with(rx: &'a mpsc::Receiver<u8>, parser: Parser) -> Self {
        Self(rx, parser)
    }
}

impl Asker for Plain<'_> {
    fn rx(&self) -> &mpsc::Receiver<u8> {
        self.0
    }

    fn discard(&mut self) {
        set_aside_keys(&mut self.1, self.0);
    }

    fn answer(&mut self, lines: &[String], wait: Duration) -> Option<u8> {
        self.show(lines);
        self.discard();
        read_answer(&mut self.1, self.0, lines, wait)
    }

    fn answer_more(&mut self, lines: &[String], wait: Duration) -> Option<u8> {
        read_answer(&mut self.1, self.0, lines, wait)
    }

    fn show(&mut self, lines: &[String]) {
        self.frame(format!("\x1b[H\x1b[2J{}", lines.join("\r\n")).as_bytes());
    }

    fn frame(&mut self, bytes: &[u8]) {
        let mut out = std::io::stdout();
        let _ = out.write_all(bytes);
        let _ = out.flush();
    }
}

/// ADR-0063: a question asked in a box over the console, which stays in view around it (its tree, tabs and inspector as they were).
struct Over<'a> {
    say: &'a Say,
    c: &'a mut Console,
    chat: Option<&'a ChatPane>,
    rx: &'a mpsc::Receiver<u8>,
    /// A screen the question drew itself is on the terminal: the next box is drawn over a cleared one.
    stale: bool,
    /// The terminal reports the mouse: while a box is open (when the console holds the mouse), never while a screen the question
    /// drew itself reads plain bytes.
    mouse: bool,
    /// The window's area when the box was last drawn: a key is read against it, and a size that changed since drops the keys that
    /// came with the change until the box is drawn again.
    drawn: Option<Area>,
}

impl Asker for Over<'_> {
    fn rx(&self) -> &mpsc::Receiver<u8> {
        self.rx
    }

    fn show(&mut self, lines: &[String]) {
        // A question is on the screen: the rest of the console's batch was typed before it (`Console::asked`).
        self.c.asked = true;
        if std::mem::take(&mut self.stale) {
            let _ = self.c.window.invalidate();
        }
        if self.c.app.mouse && !self.mouse {
            mouse_mode(true);
            self.mouse = true;
        }
        bracketed_paste(true);
        let text = |id: &str, args: &[(&str, Arg)]| self.say.text(id, args);
        let rows = self
            .c
            .snapshot
            .as_ref()
            .map(|s| app_rows(s, now_ms(), &text))
            .unwrap_or_default();
        let area = window_area();
        draw_console(self.say, self.c, &rows, None, self.chat, area, Some(lines));
        self.drawn = Some(area);
    }

    fn frame(&mut self, bytes: &[u8]) {
        self.c.asked = true;
        if std::mem::take(&mut self.mouse) {
            mouse_mode(false);
        }
        // A screen a question draws itself reads plain bytes: no framing there.
        bracketed_paste(false);
        self.stale = true;
        Plain::new(self.rx).frame(bytes);
    }

    /// Sets aside every key typed so far through the console's parser, so a paste around the question keeps its framing.
    fn discard(&mut self) {
        set_aside_keys(&mut self.c.parser, self.rx);
    }

    /// Keys and clicks through the console's own reader: a click on a numbered line is its number, a click outside the box cancels
    /// (`dialog_key`). A window resized meanwhile gets the box drawn again for its size.
    fn key(&mut self, lines: &[String], wait: Duration) -> Option<u8> {
        self.show(lines);
        self.read(lines, wait, |events, area, lines| {
            events
                .iter()
                .find_map(|e| agora_tui::app::dialog_key(e, area, lines))
        })
    }

    /// One key or click ([`one_answer`]) through the console's parser: a paste never answers.
    fn answer(&mut self, lines: &[String], wait: Duration) -> Option<u8> {
        self.show(lines);
        self.discard();
        self.read(lines, wait, one_answer)
    }

    fn answer_more(&mut self, lines: &[String], wait: Duration) -> Option<u8> {
        self.read(lines, wait, one_answer)
    }
}

impl Over<'_> {
    fn read(
        &mut self,
        lines: &[String],
        wait: Duration,
        pick: fn(&[AppEvent], Area, &[String]) -> Option<u8>,
    ) -> Option<u8> {
        // `lines` is on the screen already, drawn at `self.drawn`.
        let end = Instant::now() + wait;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            let events = match self.rx.recv_timeout(left.min(Duration::from_millis(50))) {
                Ok(b) => {
                    let mut bytes = vec![b];
                    while let Ok(b) = self.rx.try_recv() {
                        bytes.push(b);
                    }
                    self.c.parser.push(&bytes)
                }
                Err(mpsc::RecvTimeoutError::Timeout) => self.c.parser.flush(),
                Err(mpsc::RecvTimeoutError::Disconnected) => return None,
            };
            let drawn = self.drawn.unwrap_or_else(window_area);
            match box_step(drawn, window_area(), &events, lines, pick) {
                BoxStep::Key(k) => return Some(k),
                BoxStep::Wait => {}
                // The window changed size: what came with the change is dropped (it was aimed at the box as it was), and the box is
                // drawn again for the new size before any key counts.
                BoxStep::Redraw => {
                    let _ = self.c.window.resized();
                    self.show(lines);
                }
            }
        }
    }
}

/// One read of a box's keys: the window's size now against the size the box was drawn at.
#[derive(Debug, PartialEq, Eq)]
enum BoxStep {
    Key(u8),
    Wait,
    Redraw,
}

fn box_step(
    drawn: Area,
    now: Area,
    events: &[AppEvent],
    lines: &[String],
    pick: fn(&[AppEvent], Area, &[String]) -> Option<u8>,
) -> BoxStep {
    if now != drawn {
        return BoxStep::Redraw;
    }
    pick(events, drawn, lines).map_or(BoxStep::Wait, BoxStep::Key)
}

/// Asks over the console (`run` shows its questions with the [`Over`] it is given): the panel's terminal stays open meanwhile (its
/// frames wait in the session and are applied afterwards, so its scrollback and the modes it learned are kept, ADR-0063), the
/// mouse is reported only while a box is open, and the console is drawn again afterwards. Returns what `run` said.
#[allow(clippy::too_many_arguments)]
fn over_console<T>(
    say: &Say,
    c: &mut Console,
    session: &mut Session,
    chat: Option<&ChatPane>,
    rx: &mpsc::Receiver<u8>,
    run: impl FnOnce(&mut Session, &mut Over<'_>) -> T,
) -> T {
    // The mouse stays as the console had it while a box is open; a screen the question draws itself switches it off.
    let mouse = c.app.mouse;
    let mut ask = Over {
        say,
        c,
        chat,
        rx,
        stale: false,
        mouse,
        drawn: None,
    };
    let out = run(session, &mut ask);
    let stale = ask.stale;
    mouse_mode(c.app.mouse);
    bracketed_paste(true);
    if stale {
        let _ = c.window.invalidate();
    }
    out
}

/// Which request a key answers: the tree's (`y`, `n`, the commands: the selected agent's request, else the first waiting, as the
/// inspector shows it), or exactly one agent tab's request (Alt+Y, Alt+N, Alt+A: never another one).
#[derive(Clone, Debug, PartialEq, Eq)]
enum Target {
    Selected,
    Tab { agent: String, approval: String },
}

/// The request `target` names in `s`, and the agent whose details show it.
fn target_request(
    s: &Snapshot,
    selected: &str,
    target: &Target,
) -> Option<(agora_protocol::v1::ApprovalSnapshot, String)> {
    match target {
        Target::Selected => agora_tui::client::answered_approval(s, Some(selected))
            .map(|a| (a.clone(), selected.to_string())),
        Target::Tab { agent, approval } => agora_tui::client::exact_approval(s, agent, approval)
            .map(|a| (a.clone(), agent.clone())),
    }
}

/// The lines of the box that shows a request before it is answered: who asks and what, ADR-0101's always lines (the agent's own words
/// and what "always" means), and the keys that answer.
fn request_box_lines(
    say: &Say,
    s: &Snapshot,
    a: &agora_protocol::v1::ApprovalSnapshot,
) -> Vec<String> {
    let text = |id: &str, args: &[(&str, Arg)]| say.text(id, args);
    let mut lines = vec![say.text(key!("tui-attention"), &[])];
    lines.push(plain(
        format!(
            "{}: {} {}",
            agora_tui::split::agent_label(s, &a.agent_id),
            a.action,
            a.detail
        )
        .trim(),
    ));
    lines.extend(agora_tui::render::always_lines(a, &text));
    lines.push(String::new());
    lines.push(say.text(
        agora_tui::client::answer_hint(
            a,
            key!("tui-request-answer"),
            key!("tui-request-answer-always"),
            key!("tui-request-answer-always-only"),
        ),
        &[],
    ));
    lines
}

/// The inspector's details as the console draws them: the selected agent's; beside the agent's own tab with the keyboard in it, only its
/// request, answered with Alt+Y / Alt+N.
fn inspector_details(
    c: &Console,
    s: &Snapshot,
    area: Area,
    text: agora_tui::render::Text<'_>,
) -> agora_tui::split::Details {
    let inspector = c.app.areas(area).inspector;
    if c.app.focus == agora_tui::app::Focus::Terminal {
        agora_tui::split::app_details_in_tab(
            s,
            &c.selected,
            inspector.width,
            inspector.height,
            text,
        )
    } else {
        app_details(s, &c.selected, inspector.width, inspector.height, text)
    }
}

/// ADR-0057 with Codex's re-review of ADR-0101 part B: a start request counts as read from the inspector only when the inspector shows
/// that request's card (`displayed`) and showed it whole.
fn start_read_in_inspector(displayed: bool, shown: Option<&(String, bool)>, id: &str) -> bool {
    displayed && matches!(shown, Some((d, true)) if d == id)
}

/// Whether the request box shows all of its lines in a window of `area` (ADR-0101: an "always" answer is taken only then).
fn always_box_shown(lines: &[String], area: Area) -> bool {
    agora_tui::app::dialog_fits(area, lines)
}

/// The request the inspector shows, as it is drawn: beside a tab with the keyboard, only the selected agent's own request; in the tree,
/// the selected agent's request, else the first waiting (`app_details`).
fn displayed_request<'a>(
    s: &'a Snapshot,
    selected: &str,
    terminal_focus: bool,
) -> Option<&'a agora_protocol::v1::ApprovalSnapshot> {
    if terminal_focus {
        waiting_for(s, selected)
    } else {
        agora_tui::client::answered_approval(s, Some(selected))
    }
}

/// How an answer is taken (ADR-0101 and Codex's reviews of #1012): "always" only ever from the request box, which shows the whole request,
/// the agent's own words and the duration warning, after one deliberate key there; a single Allow or a Deny at once when the request is
/// in view (its own card whole in the inspector, or whole in its chat tab), else in the box too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Take {
    Box,
    Send(Answer),
}

fn take_answer(reply: Answer, in_view: bool) -> Take {
    if reply == Answer::AllowAlways || !in_view {
        Take::Box
    } else {
        Take::Send(reply)
    }
}

/// Whether the request `a` is in view where the person looks: the inspector shows exactly that request (`displayed`) and all of its rows
/// (`fits`), or its chat tab showed it whole (`shown`, the id set when all of its block fitted the pane).
fn request_in_view(
    a: &agora_protocol::v1::ApprovalSnapshot,
    displayed: Option<&agora_protocol::v1::ApprovalSnapshot>,
    fits: bool,
    shown: Option<&str>,
) -> bool {
    (fits && displayed.is_some_and(|d| d.approval_id == a.approval_id))
        || shown == Some(a.approval_id.as_str())
}

/// Answers the request `target` names with `reply`. Returns whether the hub's state will change.
fn answer_command(
    say: &Say,
    session: &mut Session,
    rx: &mpsc::Receiver<u8>,
    c: &mut Console,
    area: Area,
    target: &Target,
    reply: Answer,
) -> bool {
    let text = |id: &str, args: &[(&str, Arg)]| say.text(id, args);
    let Some(s) = c.snapshot.clone() else {
        return false;
    };
    let selected = c.selected.clone();
    let Some((a, _owner)) = target_request(&s, &selected, target) else {
        // A tab's request that is no longer waiting: nothing is answered, and never another request instead.
        c.shown_request = None;
        return false;
    };
    let shown = c.shown_request.take();
    // ADR-0101: "always" on a request that does not offer it is refused at once; nothing is shown or sent.
    if let Some(id) = agora_tui::client::not_offered(&a, reply) {
        c.notice = say.text(id, &[]);
        return false;
    }
    let inspector = c.app.areas(area).inspector;
    // What the inspector shows, exactly as it is drawn (the tree's selected agent; beside a tab with the keyboard, only its own request).
    let details = inspector_details(c, &s, area, &text);
    let displayed = displayed_request(
        &s,
        &selected,
        c.app.focus == agora_tui::app::Focus::Terminal,
    );
    let fits = inspector.width > 0 && details.lines.len() <= usize::from(inspector.height);
    // ADR-0057: a start request is allowed only once all of it was in view: its own card whole in the inspector, or the reader, which
    // y opens.
    if a.action == "agent.start" {
        let id = a.approval_id.clone();
        let own_card = displayed.is_some_and(|d| d.approval_id == id);
        if start_read_in_inspector(own_card, details.start.as_ref(), &id) {
            c.read_whole.insert(id.clone());
        }
        if reply == Answer::Allow && !c.read_whole.contains(&id) {
            let read = over_console(say, c, session, None, rx, |_, ask| {
                read_start(&a, ask, &text)
            });
            if read {
                c.read_whole.insert(id);
                c.notice = say.text(key!("tui-app-start-read"), &[]);
            } else {
                c.notice = say.text(key!("tui-split-read-first"), &[]);
            }
            return false;
        }
        if reply == Answer::Allow
            && !over_console(say, c, session, None, rx, |_, ask| {
                confirm_skipping_request(say, &a, &text, ask)
            })
        {
            c.notice = String::new();
            return false;
        }
        c.notice = answer_request(say, session, &s, &a, reply);
        return true;
    }
    let in_view = request_in_view(&a, displayed, fits, shown.as_deref());
    let reply = match take_answer(reply, in_view) {
        Take::Send(r) => r,
        Take::Box => {
            let lines = request_box_lines(say, &s, &a);
            // One deliberate key or click, never a paste, a key typed before the box was shown, or one that came with a resize
            // before the box was drawn again (`Over::read`); the box's size is the one it was last drawn at.
            let (key, drawn) = over_console(say, c, session, None, rx, |_, ask| {
                let key = ask.answer(&lines, Duration::from_secs(120));
                (key, ask.drawn)
            });
            match box_reply(agora_tui::client::box_answer(key), &lines, drawn) {
                Ok(r) => r,
                Err(Some(id)) => {
                    c.notice = say.text(id, &[]);
                    return false;
                }
                Err(None) => {
                    c.notice = String::new();
                    return false;
                }
            }
        }
    };
    c.notice = answer_request(say, session, &s, &a, reply);
    true
}

/// The answer a key in the request box gives, checked against the box as it was last drawn (`drawn`): "always" only from a box that
/// showed all of its lines (Codex's re-review: the duration warning may be below a cut box's last row), else the line that says why
/// nothing was sent. `Err(None)`: no answer (another key, or none).
fn box_reply(
    key: Option<Answer>,
    lines: &[String],
    drawn: Option<Area>,
) -> Result<Answer, Option<&'static str>> {
    match key {
        Some(Answer::AllowAlways) if !drawn.is_some_and(|area| always_box_shown(lines, area)) => {
            Err(Some(key!("tui-always-too-small")))
        }
        Some(r) => Ok(r),
        None => Err(None),
    }
}

/// A letter for the selected agent. Returns whether the hub's state will change (a fresh snapshot is wanted).
#[allow(clippy::too_many_arguments)]
fn command(
    say: &Say,
    session: &mut Session,
    rx: &mpsc::Receiver<u8>,
    color: bool,
    c: &mut Console,
    area: Area,
    k: char,
) -> bool {
    let text = |id: &str, args: &[(&str, Arg)]| say.text(id, args);
    let Some(s) = c.snapshot.clone() else {
        return false;
    };
    let selected = c.selected.clone();
    match k {
        'y' | 'n' | ALLOW_ALWAYS => {
            let reply = match k {
                'y' => Answer::Allow,
                ALLOW_ALWAYS => Answer::AllowAlways,
                _ => Answer::Deny,
            };
            answer_command(say, session, rx, c, area, &Target::Selected, reply)
        }
        // ADR-0061: ask the person's other devices to approve this terminal (a), or approve one that waits (v).
        'a' => {
            c.notice = ask_approval(say, session);
            false
        }
        // ADR-0065: the first machine that asks to join, read whole and answered with two deliberate keys.
        'p' => {
            c.notice = over_console(say, c, session, None, rx, |session, ask| {
                answer_pair(say, session, ask)
            });
            true
        }
        'v' => {
            let snapshot = c.snapshot.clone();
            c.notice = over_console(say, c, session, None, rx, |session, ask| {
                approve_device(say, session, snapshot.as_ref(), ask)
            });
            true
        }
        'f' => {
            let mut role = c.role;
            c.notice = over_console(say, c, session, None, rx, |session, ask| {
                start_in_folder(say, session, &s, &selected, ask, color, &mut role)
            });
            c.role = role;
            true
        }
        's' => {
            c.notice = over_console(say, c, session, None, rx, |session, ask| {
                start(say, session, &s, &selected, ask)
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
            c.notice = over_console(say, c, session, None, rx, |session, ask| {
                let with_helpers = helpers == 0 || {
                    let question = say.text(
                        key!("tui-stop-helpers"),
                        &[("count", Arg::Num(helpers as i64))],
                    );
                    ask.key(&lines_of(&question), Duration::from_secs(30)) != Some(b'n')
                };
                match with_grant(
                    say,
                    session,
                    ask,
                    (&node, &scope, Site::ConsoleStop.commands()),
                    |s| s.stop_agent_with(&node, &local, helpers > 0 && with_helpers),
                ) {
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
        // What the selected agent changed (the web's review screen): its diff, whole-screen, until q or Esc. Like the console it
        // redraws when the window is resized and says when a new request arrives meanwhile.
        'r' if !selected.is_empty() => {
            let Some((node, target)) = agora_tui::client::place_of(&s, &selected) else {
                c.notice = say.text(key!("tui-review-nowhere"), &[]);
                return false;
            };
            let title = agora_tui::split::agent_label(&s, &selected);
            c.notice = over_console(say, c, session, None, rx, |session, ask| {
                let diff = match session.diff(&node, &target) {
                    Ok(d) => d,
                    Err(code) => {
                        return say.text(key!("tui-review-failed"), &[("code", Arg::Str(code))])
                    }
                };
                let lines =
                    agora_tui::review::lines_with_secret(&diff.diff, &diff.secret_files, &text);
                let colors = ask.c.colors;
                let mut top = 0usize;
                let mut view = ViewWait::new(say, ask);
                loop {
                    let room = usize::from(window_area().height.saturating_sub(2)).max(1);
                    top = agora_tui::review::clamp_top(top, lines.len(), room);
                    let banner = view.banner.clone();
                    let _ = ask.c.window.draw(|f| {
                        agora_tui::review::draw(f, &title, &lines, top, &banner, colors, &text);
                    });
                    match view.wait(say, ask, session) {
                        Wake::Key(b'j') => top += 1,
                        Wake::Key(b'k') => top = top.saturating_sub(1),
                        Wake::Key(b' ' | b'f') => top += room,
                        Wake::Key(b'b') => top = top.saturating_sub(room),
                        Wake::Key(b'g') => top = 0,
                        Wake::Key(b'G') => top = lines.len(),
                        Wake::Gone | Wake::Key(b'q' | 0x1b | 3 | 0x7f) => return String::new(),
                        Wake::Key(_) | Wake::Redraw => {}
                    }
                }
            });
            // The console is drawn whole again over the review.
            let _ = c.window.invalidate();
            false
        }
        // The selected agent's files (the web's file browser): its folder's entries, Enter into a folder or a file, ⌫ up, q back.
        'o' if !selected.is_empty() => {
            let Some((node, place)) = agora_tui::client::place_of(&s, &selected) else {
                c.notice = say.text(key!("tui-review-nowhere"), &[]);
                return false;
            };
            let title = agora_tui::split::agent_label(&s, &selected);
            c.notice = over_console(say, c, session, None, rx, |session, ask| {
                use agora_tui::files;
                let mut path = String::new();
                let mut notice = String::new();
                let mut view = ViewWait::new(say, ask);
                'folder: loop {
                    let entries = match session.files_list(&node, &place, &path) {
                        Ok(e) => files::sorted(e),
                        Err(code) if !path.is_empty() => {
                            notice =
                                say.text(key!("tui-files-failed"), &[("code", Arg::Str(code))]);
                            path = files::parent(&path);
                            continue;
                        }
                        Err(code) => {
                            return say.text(key!("tui-files-failed"), &[("code", Arg::Str(code))])
                        }
                    };
                    let mut at = 0usize;
                    loop {
                        let banner = view.banner.clone();
                        let _ = ask.c.window.draw(|f| {
                            files::draw_listing(
                                f, &title, &path, &entries, at, &notice, &banner, &text,
                            );
                        });
                        let wake = view.wait(say, ask, session);
                        if matches!(wake, Wake::Key(_)) {
                            notice.clear();
                        }
                        let count = entries.len();
                        match wake {
                            Wake::Key(b'j') if count > 0 => at = (at + 1).min(count - 1),
                            Wake::Key(b'k') => at = at.saturating_sub(1),
                            Wake::Key(b'\r' | b'\n') => {
                                let Some(e) = entries.get(at) else { continue };
                                // A secret file is not asked for (ADR-0080): the reason is said instead.
                                if !files::openable(e) {
                                    notice = say.text(key!("tui-files-secret"), &[]);
                                    continue;
                                }
                                let target = files::child(&path, &e.name);
                                if e.is_dir {
                                    path = target;
                                    continue 'folder;
                                }
                                let content = match session.files_read(&node, &place, &target) {
                                    Ok(data) => files::content(&data),
                                    Err(code) => {
                                        notice = files::read_failure(&code, &text);
                                        continue;
                                    }
                                };
                                let count = match &content {
                                    files::Content::Text(l) => l.len(),
                                    files::Content::Binary(_) => 0,
                                };
                                let mut top = 0usize;
                                loop {
                                    let room =
                                        usize::from(window_area().height.saturating_sub(2)).max(1);
                                    top = top.min(count.saturating_sub(room));
                                    let banner = view.banner.clone();
                                    let _ = ask.c.window.draw(|f| {
                                        files::draw_file(
                                            f, &title, &target, &content, top, &banner, &text,
                                        );
                                    });
                                    match view.wait(say, ask, session) {
                                        Wake::Key(b'j') => top += 1,
                                        Wake::Key(b'k') => top = top.saturating_sub(1),
                                        Wake::Key(b' ' | b'f') => top += room,
                                        Wake::Key(b'b') => top = top.saturating_sub(room),
                                        Wake::Key(b'g') => top = 0,
                                        Wake::Key(b'G') => top = count,
                                        Wake::Key(0x7f | 0x08 | b'h' | b'q' | 0x1b) => break,
                                        Wake::Gone | Wake::Key(3) => return String::new(),
                                        Wake::Key(_) | Wake::Redraw => {}
                                    }
                                }
                            }
                            Wake::Key(0x7f | 0x08 | b'h') if !path.is_empty() => {
                                path = files::parent(&path);
                                continue 'folder;
                            }
                            Wake::Gone | Wake::Key(b'q' | 0x1b | 3) => return String::new(),
                            Wake::Key(_) | Wake::Redraw => {}
                        }
                    }
                }
            });
            let _ = c.window.invalidate();
            false
        }
        // ADR-0095: the board of the selected agent's project, whole-screen, and "Give to an agent" on a Backlog task. Like the review
        // it redraws when the window is resized and says when a new request arrives meanwhile.
        'b' if !selected.is_empty() => {
            let Some(project) = agora_tui::board::project_of(&s, &selected) else {
                c.notice = say.text(key!("tui-board-no-project"), &[]);
                return false;
            };
            c.notice = over_console(say, c, session, None, rx, |session, ask| {
                board_view(say, session, ask, &project)
            });
            let _ = c.window.invalidate();
            false
        }
        // ADR-0041: the grant the footer asked for when typing into the agent in front was refused. Only this key opens the question
        // (from the list, Ctrl-] then g): what follows is meant for it. Without such a refusal it does nothing.
        'g' => {
            let Some((node, local)) =
                grant_key_target(c.typing_refused.as_ref(), c.app.active_agent())
            else {
                return false;
            };
            let scope = format!("agent:{local}");
            let asked = over_console(say, c, session, None, rx, |s, ask| {
                ask_grant(say, s, ask, &node, &scope, Site::GrantKey.commands())
            });
            match asked {
                Ok(true) => {
                    c.typing_allowed = Some((format!("{node}/{local}"), session.last_request()));
                    c.typing_refused = None;
                    if c.notice == c.typing_notice {
                        c.notice.clear();
                    }
                    c.notice = say.text(key!("tui-granted"), &[]);
                }
                Ok(false) => {}
                Err(line) => c.notice = line,
            }
            false
        }
        'c' => {
            if let Some(cmd) = c.note.copy.clone() {
                let mut out = std::io::stdout();
                let _ = out.write_all(&agora_tui::clip::osc52(&cmd));
                let _ = out.flush();
                c.notice = say.text(key!("tui-copied"), &[]);
            }
            false
        }
        _ => false,
    }
}

/// ADR-0095: a project's board, whole-screen (Backlog, Doing, Done; the keyboard on a card), until q or Esc. Enter or `g` on a Backlog
/// task gives it to an agent ([`give_task`]); what that came to stays in the footer. The snapshot is the console's: a task given here
/// is shown under Doing at once, other changes when the board is opened again. Answers the line the console shows after it (none).
fn board_view(say: &Say, session: &mut Session, ask: &mut Over<'_>, project: &str) -> String {
    let text = |id: &str, args: &[(&str, Arg)]| say.text(id, args);
    // Tasks given from here: shown under Doing until the hub's next snapshot says where they are.
    let mut given: Vec<(String, String)> = Vec::new();
    let mut at = 0usize;
    // The task the keyboard is on: it stays on it when the board changes (a task just given moves to Doing).
    let mut on: Option<String> = None;
    let mut notice = String::new();
    let mut view = ViewWait::new(say, ask);
    loop {
        let Some(b) = ask
            .c
            .snapshot
            .as_ref()
            .map(|s| agora_tui::board::board(s, project, &given, &text))
        else {
            return String::new();
        };
        let cards: Vec<(agora_tui::board::Column, agora_tui::board::Card)> = b
            .cards()
            .into_iter()
            .map(|(col, card)| (col, card.clone()))
            .collect();
        at = b.select(on.as_deref(), at);
        on = cards.get(at).map(|(_, card)| card.task_id.clone());
        let banner = view.banner.clone();
        let _ = ask.c.window.draw(|f| {
            agora_tui::board::draw_board(f, &b, at, &notice, &banner, &text);
        });
        let wake = view.wait(say, ask, session);
        if matches!(wake, Wake::Key(_)) {
            notice.clear();
        }
        match wake {
            Wake::Key(b'j') if !cards.is_empty() => {
                at = (at + 1).min(cards.len() - 1);
                on = None;
            }
            Wake::Key(b'k') => {
                at = at.saturating_sub(1);
                on = None;
            }
            Wake::Key(b'\r' | b'\n' | b'g') => match cards.get(at) {
                Some((agora_tui::board::Column::Backlog, card)) => {
                    notice = give_task(say, session, ask, &mut view, &b, card, &mut given);
                    let _ = ask.c.window.invalidate();
                }
                Some(_) => notice = say.text(key!("tui-board-backlog-only"), &[]),
                None => {}
            },
            Wake::Gone | Wake::Key(b'q' | 0x1b | 3) => return String::new(),
            Wake::Key(_) | Wake::Redraw => {}
        }
    }
}

/// "Give to an agent" for one Backlog task (ADR-0095 decisions 3, 4, 8): a picker of the project's agents that can take it, then a
/// confirmation that only a fresh `g` answers (not Enter, the key that chose the agent, so a held key never gives it; keys typed before
/// it was on the screen, a paste, or several keys at once never answer it), then the command to the agent's machine. A machine that
/// wants a grant first gets the console's one-hour question for that agent. Answers what happened, or nothing when the user went back.
fn give_task(
    say: &Say,
    session: &mut Session,
    ask: &mut Over<'_>,
    view: &mut ViewWait,
    b: &agora_tui::board::Board,
    task: &agora_tui::board::Card,
    given: &mut Vec<(String, String)>,
) -> String {
    use agora_tui::board;
    let text = |id: &str, args: &[(&str, Arg)]| say.text(id, args);
    let mut at = 0usize;
    'pick: loop {
        let Some(s) = ask.c.snapshot.clone() else {
            return String::new();
        };
        let configs = session.configs().clone();
        let choices: Vec<(String, String)> =
            board::candidates(&s, &b.project_id, &ask.c.give_lacking)
                .into_iter()
                .map(|(a, depth)| {
                    (
                        a.agent_id.clone(),
                        board::choice_line(&s, a, depth, &configs, &text),
                    )
                })
                .collect();
        at = at.min(choices.len().saturating_sub(1));
        let banner = view.banner.clone();
        let _ = ask.c.window.draw(|f| {
            board::draw_picker(f, &task.title, &b.project, &choices, at, "", &banner, &text);
        });
        let agent_id = match view.wait(say, ask, session) {
            Wake::Key(b'j') if !choices.is_empty() => {
                at = (at + 1).min(choices.len() - 1);
                continue;
            }
            Wake::Key(b'k') => {
                at = at.saturating_sub(1);
                continue;
            }
            Wake::Key(b'\r' | b'\n') => match choices.get(at) {
                Some((id, _)) => id.clone(),
                None => continue,
            },
            Wake::Gone | Wake::Key(b'q' | 0x1b | 3 | 0x7f | 0x08) => return String::new(),
            Wake::Key(_) | Wake::Redraw => continue,
        };
        // The confirmation, from the agent as it is now.
        {
            let Some(s) = ask.c.snapshot.clone() else {
                return String::new();
            };
            let Some(a) = s
                .agents
                .iter()
                .find(|a| a.agent_id == agent_id)
                .filter(|a| board::can_take(&s, a, &b.project_id, &ask.c.give_lacking))
            else {
                return board::cannot_take_text(&s, &agent_id, &ask.c.give_lacking, &text);
            };
            // The hub keeps being read while it waits: a report that changes the agent's options or mode redraws it.
            let mut source = GiveConfirm {
                say,
                session: &mut *session,
                s: &s,
                agent: a,
                task: &task.title,
                project: &b.project,
                kept: None,
                stale: false,
            };
            let given = confirm_give(ask, Duration::from_secs(120), &window_area, &mut source);
            let (kept, stale) = (source.kept, source.stale);
            if let Some(new) = kept {
                ask.c.give_lacking.settle(&new);
                ask.c.snapshot = Some(new);
            }
            ask.c.stale |= stale;
            if !given {
                continue 'pick;
            }
        }
        // Checked again at the moment of sending: it may have stopped, been closed or interrupted, or left the project meanwhile.
        let Some(s) = ask.c.snapshot.clone() else {
            return String::new();
        };
        let Some(a) = s
            .agents
            .iter()
            .find(|a| a.agent_id == agent_id)
            .filter(|a| board::can_take(&s, a, &b.project_id, &ask.c.give_lacking))
        else {
            return board::cannot_take_text(&s, &agent_id, &ask.c.give_lacking, &text);
        };
        let name = board::agent_name(&s, a);
        let (node, request) = board::give_request(a, &b.project_id, &task.task_id);
        let scope = format!(
            "agent:{}",
            agent_id
                .split_once('/')
                .map_or(agent_id.as_str(), |(_, l)| l)
        );
        let mut refusal: Option<ErrorInfo> = None;
        let tried = with_grant(
            say,
            session,
            ask,
            (&node, &scope, Site::BoardGive.commands()),
            |session| {
                session
                    .command_info(&node, request.clone(), Duration::from_secs(15))
                    .map_err(|e| {
                        let code = e.code.clone();
                        refusal = Some(e);
                        code
                    })
            },
        );
        return match tried {
            Tried::Done(Ok(ok)) => {
                given.push((task.task_id.clone(), agent_id.clone()));
                // The board changed: the console asks for a fresh snapshot once the board is closed.
                ask.c.stale = true;
                let result = match &ok.result {
                    Some(agora_protocol::v1::command_ok::Result::TaskDispatch(r)) => Some(r),
                    _ => None,
                };
                board::result_text(result, &name, &text)
            }
            Tried::Done(Err(code)) => {
                let e = refusal.unwrap_or(ErrorInfo {
                    code,
                    ..Default::default()
                });
                if board::lacks_give(&e) {
                    ask.c.give_lacking.note(&s, &node);
                }
                board::refusal_text(&e, &text)
            }
            Tried::Declined => say.text(key!("tui-give-grant-declined"), &[]),
            Tried::Line(l) => l,
        };
    }
}

/// What "Give to an agent"'s confirmation reads while it waits ([`confirm_give`]).
trait ConfirmSource {
    /// Reads what the hub said meanwhile: whether the confirmation may read differently now (an agent's config options or mode
    /// changed), or the link is gone.
    fn poll(&mut self) -> Refresh;
    /// The confirmation as it reads now in a window of `area`; `Err` when the window cannot show what must be read before `g`.
    fn lines(&self, area: Area) -> Result<Vec<String>, Vec<String>>;
}

/// What the hub said while the confirmation waited.
#[derive(Debug, PartialEq, Eq)]
enum Refresh {
    Same,
    /// Something the confirmation shows may have changed.
    Changed,
    /// The link to the hub is gone.
    Gone,
}

/// The confirmation of a give from the board: the agent as the snapshot shows it and its options and mode as its machine reported
/// them since. A snapshot or a change read while it waits is kept for the console.
struct GiveConfirm<'a> {
    say: &'a Say,
    session: &'a mut Session,
    s: &'a Snapshot,
    agent: &'a agora_protocol::v1::AgentSnapshot,
    task: &'a str,
    project: &'a str,
    kept: Option<Snapshot>,
    stale: bool,
}

impl ConfirmSource for GiveConfirm<'_> {
    fn poll(&mut self) -> Refresh {
        match self.session.next(Duration::ZERO) {
            Update::Snapshot(new) => self.kept = Some(*new),
            Update::Changed => self.stale = true,
            Update::Closed => return Refresh::Gone,
            Update::Quiet => {}
        }
        if self.session.take_configs_changed() {
            Refresh::Changed
        } else {
            Refresh::Same
        }
    }

    fn lines(&self, area: Area) -> Result<Vec<String>, Vec<String>> {
        let text = |id: &str, args: &[(&str, Arg)]| self.say.text(id, args);
        agora_tui::board::confirm_lines(
            self.s,
            self.agent,
            self.session.configs(),
            self.task,
            self.project,
            area,
            &text,
        )
    }
}

/// The answer to "Give to an agent"'s confirmation: true only for a `g` pressed after it is on the screen ([`Asker::answer`]: keys typed
/// before it are set aside, a paste or several keys at once never answer). `n`, Esc, q, ⌫ or no answer in `wait` go back; any other
/// key (Enter repeated by a held key, the key that chose the agent) asks again.
///
/// While it waits it reads the hub ([`ConfirmSource::poll`]) and the window's size (`area`). When the confirmation reads differently
/// (the agent now acts without asking, or no longer does; the window was resized) it is drawn again and only a `g` pressed after that
/// gives the task: a `g` pressed while the old one was shown, even in the same moment the change was read, does not. A window too small
/// for the lines that must be read says so, and `g` gives nothing there.
fn confirm_give(
    ask: &mut dyn Asker,
    wait: Duration,
    area: &dyn Fn() -> Area,
    source: &mut dyn ConfirmSource,
) -> bool {
    let end = Instant::now() + wait;
    let mut at = area();
    let mut fit = source.lines(at);
    let mut fresh = true;
    loop {
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return false;
        }
        let slice = left.min(CONFIRM_SLICE);
        let shown = match &fit {
            Ok(l) | Err(l) => l.clone(),
        };
        let got = if std::mem::take(&mut fresh) {
            ask.answer(&shown, slice)
        } else {
            ask.answer_more(&shown, slice)
        };
        let changed = match source.poll() {
            Refresh::Gone => return false,
            Refresh::Changed => true,
            Refresh::Same => false,
        };
        let now = area();
        if changed || now != at {
            at = now;
            let new = source.lines(at);
            if new != fit {
                fit = new;
                fresh = true;
                continue;
            }
        }
        match got {
            Some(b'g') if fit.is_ok() => return true,
            Some(b'n' | b'q' | 0x1b | 0x7f) => return false,
            // Any other key (Enter repeated by a held key, the key that chose the agent, g where it cannot be given) asks again.
            Some(_) => fresh = true,
            None => {}
        }
    }
}

/// How often the confirmation looks for a change while it waits.
const CONFIRM_SLICE: Duration = Duration::from_millis(200);

/// ADR-0073 "Clean up old machines": the machines not seen for a day, each with when it was last seen, in a box that asks once:
/// y forgets them, r forgets them and refuses them if they connect again, any other key keeps them. Returns the line to show.
fn clean_up_old(
    say: &Say,
    session: &mut Session,
    c: &mut Console,
    rx: &mpsc::Receiver<u8>,
) -> String {
    let now = now_ms();
    let text = |id: &str, args: &[(&str, Arg)]| say.text(id, args);
    let old: Vec<(String, String)> = c
        .snapshot
        .as_ref()
        .map(|s| {
            s.nodes
                .iter()
                .filter(|n| agora_tui::split::is_old(n, now))
                .map(|n| {
                    let name = if n.hostname.is_empty() {
                        &n.node_id
                    } else {
                        &n.hostname
                    };
                    (
                        n.node_id.clone(),
                        format!(
                            "  {}  {}",
                            plain(name),
                            agora_tui::render::presence_text(n, now, &text)
                        ),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    if old.is_empty() {
        return say.text(key!("tui-cleanup-none"), &[]);
    }
    let count = [(
        "count",
        Arg::Num(i64::try_from(old.len()).unwrap_or(i64::MAX)),
    )];
    let mut lines = vec![say.text(key!("tui-cleanup-title"), &count), String::new()];
    lines.extend(old.iter().map(|(_, l)| l.clone()));
    lines.push(String::new());
    lines.push(say.text(key!("tui-cleanup-explain"), &[]));
    lines.push(say.text(key!("tui-cleanup-keys"), &[]));
    let answer = over_console(say, c, session, None, rx, |_, ask| {
        ask.key(&lines, Duration::from_secs(120))
    });
    let refuse = match answer {
        Some(b'y') => false,
        Some(b'r') => true,
        _ => return String::new(),
    };
    let mut forgotten = 0i64;
    for (id, _) in &old {
        match session.forget_node(id, refuse) {
            Ok(()) => forgotten += 1,
            Err(e) => {
                return say.text(
                    key!("tui-cleanup-failed"),
                    &[("code", Arg::Str(e.code)), ("count", Arg::Num(forgotten))],
                )
            }
        }
    }
    say.text(key!("tui-cleanup-done"), &[("count", Arg::Num(forgotten))])
}

/// Asks which of a machine's installed `kinds` to start (`s`, `f`), nine to a page numbered 1 to 9: one key picks, `n` turns to the
/// next page (after the last, the first again) when there is more than one, and the question says so; any other key, or none in
/// `wait`, cancels. `under_title` goes below the question (the folder `f` starts in). The index into `kinds`, or `None`.
fn pick_kind(
    say: &Say,
    ask: &mut dyn Asker,
    machine: &str,
    under_title: &[String],
    kinds: &[(String, String)],
    missing: Option<&str>,
    wait: Duration,
) -> Option<usize> {
    const PAGE: usize = 9;
    let pages = kinds.len().div_ceil(PAGE);
    let mut page = 0;
    loop {
        let machine = ("machine", Arg::Str(machine.to_string()));
        let title = if pages > 1 {
            say.text(
                key!("tui-start-pick-page"),
                &[
                    machine,
                    ("page", Arg::Num(page as i64 + 1)),
                    ("pages", Arg::Num(pages as i64)),
                ],
            )
        } else {
            say.text(key!("tui-start-pick"), &[machine])
        };
        let shown = &kinds[(page * PAGE).min(kinds.len())..((page + 1) * PAGE).min(kinds.len())];
        let mut menu = vec![title];
        menu.extend_from_slice(under_title);
        menu.push(String::new());
        for (i, (_, name)) in shown.iter().enumerate() {
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
            menu.push(m.to_string());
        }
        match ask.key(&menu, wait)? {
            b'n' if pages > 1 => page = (page + 1) % pages,
            k @ b'1'..=b'9' => {
                let i = usize::from(k - b'1');
                return (i < shown.len()).then_some(page * PAGE + i);
            }
            _ => return None,
        }
    }
}

/// A machine's agents split into the ones it can start (numbered in the menu) and one line naming the others: the machine cannot
/// find their program in its `PATH`. Each is named with the program the machine looked for (ADR-0069), when the machine says it.
fn split_kinds(
    say: &Say,
    all: Vec<agora_tui::client::Kind>,
) -> (Vec<(String, String)>, Option<String>) {
    let mut installed = Vec::new();
    let mut missing = Vec::new();
    for k in all {
        if k.installed {
            installed.push((k.kind, k.name));
        } else if k.program.is_empty() {
            missing.push(plain(&k.name));
        } else {
            missing.push(say.text(
                key!("tui-start-missing-item"),
                &[
                    ("name", Arg::Str(plain(&k.name))),
                    ("program", Arg::Str(plain(&k.program))),
                ],
            ));
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
mod typing_grant_tests {
    use super::*;
    use agora_tui::app::Key;
    use agora_tui::client::Refused;

    fn typing_refusal_now(
        pending: &mut Option<TypingRefused>,
        refused: &[Refused],
        taken: &[(String, String, u64)],
        active: Option<&str>,
    ) -> Notice {
        typing_refusal(pending, refused, &taken_map(taken), active, None)
    }

    fn taken_map(taken: &[(String, String, u64)]) -> Taken {
        let mut map = Taken::new();
        for (n, t, r) in taken {
            let newest = map.entry((n.clone(), t.clone())).or_insert(*r);
            *newest = (*newest).max(*r);
        }
        map
    }

    #[test]
    fn a_refusal_of_typing_sent_before_it_was_allowed_is_old_news() {
        // Codex review of #796: typing refused late, after the grant key allowed it, must not bring the notice back.
        let front = Some("laptop/a1");
        let allowed = ("laptop/a1".to_string(), 7);
        let mut pending = None;
        assert_eq!(
            typing_refusal(
                &mut pending,
                &[refused("laptop", "a1", 7)],
                &Taken::new(),
                front,
                Some(&allowed)
            ),
            Notice::Same
        );
        assert_eq!(pending, None);
        // Control: typing sent after it was allowed and refused again (the grant ended) is a new refusal.
        assert_eq!(
            typing_refusal(
                &mut pending,
                &[refused("laptop", "a1", 8)],
                &Taken::new(),
                front,
                Some(&allowed)
            ),
            Notice::Show
        );
        // Another agent's watermark changes nothing here.
        let other = ("laptop/a2".to_string(), 99);
        let mut pending = None;
        assert_eq!(
            typing_refusal(
                &mut pending,
                &[refused("laptop", "a1", 3)],
                &Taken::new(),
                front,
                Some(&other)
            ),
            Notice::Show
        );
    }

    #[test]
    fn the_grant_key_signs_typing_only() {
        assert_eq!(TYPING_INPUT, ["terminal.input"]);
    }

    /// #810: each place that asks signs the least its action needs, and the question names exactly that and the agent.
    /// A stop grant does not authorize typing (and the other way round).
    #[test]
    #[allow(clippy::unwrap_used)]
    fn every_place_that_asks_signs_the_command_it_names() {
        for (site, want) in [
            (Site::AttachTyping, "terminal.input"),
            (Site::GrantKey, "terminal.input"),
            (Site::SplitPrompt, "agent.prompt"),
            (Site::ChatPrompt, "agent.prompt"),
            (Site::SplitStop, "agent.stop"),
            (Site::ConsoleStop, "agent.stop"),
            (Site::BoardGive, "task.dispatch"),
        ] {
            assert_eq!(site.commands(), [want], "{site:?}");
        }
        // Every call goes through the table: no place names a set of its own.
        // A Windows checkout may turn line ends into CRLF; the searches below use LF.
        let src = include_str!("tuicmd.rs").replace("\r\n", "\n");
        let code = &src[..src.find("#[cfg(test)]").unwrap()];
        assert_eq!(code.matches(".commands()").count(), 7);
    }

    /// An agent's id is written to the terminal: no control character of it gets there.
    #[test]
    fn the_question_shows_no_control_character_of_the_id() {
        let say = Say(agora_i18n::Catalog::english());
        for c in [&TYPING_INPUT, &PROMPT, &STOP, &GIVE] {
            let q = grant_question(&say, "agent:a\x1b[2Jb\nc", c);
            assert!(!q.contains('\x1b') && !q.contains('\n'), "{q:?}");
            assert!(q.contains("a[2Jb c"), "{q:?}");
        }
    }

    #[test]
    fn each_grant_covers_only_what_its_question_says() {
        let say = Say(agora_i18n::Catalog::english());
        // attach() and the grant key; both chat prompts; both stops; the option switches; a start (the general words).
        let sites: [(&[&str], &str); 7] = [
            (&TYPING_INPUT, "type into"),
            (&GIVE, "give board tasks to"),
            (&PROMPT, "send messages to"),
            (&STOP, "stop"),
            (&["agent.set_mode"], "mode"),
            (&["agent.set_model"], "model"),
            (&["agent.start"], "wants your OK"),
        ];
        for (commands, words) in sites {
            let q = grant_question(&say, "agent:a1", commands);
            assert!(q.contains(words), "{commands:?}: {q}");
            if commands != ["agent.start"] {
                assert!(q.contains("a1"), "{q}");
            }
        }
        for (a, b) in [
            (&STOP, &TYPING_INPUT),
            (&PROMPT, &STOP),
            (&TYPING_INPUT, &PROMPT),
        ] {
            assert!(a.iter().all(|c| !b.contains(c)));
            assert_ne!(
                grant_question(&say, "agent:a1", a),
                grant_question(&say, "agent:a1", b)
            );
        }
        // The Turkish questions name the agent too.
        let tr = Say(agora_i18n::Catalog::turkish());
        for c in [&TYPING_INPUT, &PROMPT, &STOP, &GIVE] {
            assert!(grant_question(&tr, "agent:a1", c).contains("a1"));
        }
    }

    fn refused(node: &str, local: &str, request: u64) -> Refused {
        Refused {
            node: node.into(),
            terminal_id: format!("t-{local}"),
            request,
        }
    }

    fn taken(node: &str, local: &str, request: u64) -> (String, String, u64) {
        (node.into(), format!("t-{local}"), request)
    }

    /// Codex review of #796 (fourth): a refused keystroke into the agent in front shows the notice and opens nothing; another tab's
    /// refusal shows nothing; more refusals while it shows say nothing new.
    #[test]
    #[allow(clippy::unwrap_used)]
    fn a_refused_keystroke_shows_the_notice_and_opens_nothing() {
        let mut pending = None;
        let front = Some("laptop/a1");
        assert_eq!(
            typing_refusal_now(&mut pending, &[refused("laptop", "a2", 3)], &[], front),
            Notice::Same
        );
        assert_eq!(pending, None);
        assert_eq!(
            typing_refusal_now(&mut pending, &[refused("mac", "a1", 3)], &[], front),
            Notice::Same
        );
        assert_eq!(
            typing_refusal_now(&mut pending, &[refused("laptop", "a1", 4)], &[], front),
            Notice::Show
        );
        assert_eq!(
            pending,
            Some(TypingRefused {
                agent: "laptop/a1".into(),
                request: 4
            })
        );
        // More of the same typing refused: the notice stands, nothing new is said.
        assert_eq!(
            typing_refusal_now(&mut pending, &[refused("laptop", "a1", 6)], &[], front),
            Notice::Same
        );
        assert_eq!(pending.as_ref().map(|p| p.request), Some(6));
        // Nothing in this path asks: the question opens only through `command`'s `g`.
        // A Windows checkout may turn line ends into CRLF; the searches below use LF.
        let src = include_str!("tuicmd.rs").replace("\r\n", "\n");
        let c0 = src.find("\nfn console(").unwrap();
        let console = &src[c0..c0 + src[c0..].find("\nfn read_keys(").unwrap()];
        let t0 = console.find("session.take_grant_wanted()").unwrap();
        let path = &console[t0..t0
            + console[t0..]
                .find("// At most one frame per FRAME")
                .unwrap()];
        assert!(
            !path.contains("ask_grant") && !path.contains("over_console"),
            "{path}"
        );
    }

    /// The notice ends when another tab comes to the front, or when the agent's machine takes typing sent after the refusal; a
    /// refusal older than typing already taken never shows it.
    #[test]
    fn the_notice_ends_with_a_new_tab_or_the_next_accepted_keystroke() {
        let mut pending = None;
        typing_refusal_now(
            &mut pending,
            &[refused("laptop", "a1", 4)],
            &[],
            Some("laptop/a1"),
        );
        assert_eq!(
            typing_refusal_now(&mut pending, &[], &[], Some("laptop/a2")),
            Notice::End
        );
        assert_eq!(pending, None);
        typing_refusal_now(
            &mut pending,
            &[refused("laptop", "a1", 5)],
            &[],
            Some("laptop/a1"),
        );
        // Typing taken before the refusal does not end it; typing taken after does.
        assert_eq!(
            typing_refusal_now(
                &mut pending,
                &[],
                &[taken("laptop", "a1", 4)],
                Some("laptop/a1")
            ),
            Notice::Same
        );
        assert_eq!(
            typing_refusal_now(
                &mut pending,
                &[],
                &[taken("laptop", "a1", 7)],
                Some("laptop/a1")
            ),
            Notice::End
        );
        assert_eq!(pending, None);
        assert_eq!(
            typing_refusal_now(
                &mut pending,
                &[refused("laptop", "a1", 8)],
                &[taken("laptop", "a1", 9)],
                Some("laptop/a1")
            ),
            Notice::Same
        );
        assert_eq!(pending, None);
        // Ended and refused again in one turn: said again.
        typing_refusal_now(
            &mut pending,
            &[refused("laptop", "a1", 10)],
            &[],
            Some("laptop/a1"),
        );
        assert_eq!(
            typing_refusal_now(
                &mut pending,
                &[refused("laptop", "a1", 12)],
                &[taken("laptop", "a1", 11)],
                Some("laptop/a1")
            ),
            Notice::Show
        );
    }

    /// The key asks for the agent in front, only while its typing is refused; without such a refusal it does nothing.
    #[test]
    fn the_grant_key_asks_for_the_agent_in_front_only() {
        let p = TypingRefused {
            agent: "laptop/a1".into(),
            request: 4,
        };
        assert_eq!(
            grant_key_target(Some(&p), Some("laptop/a1")),
            Some(("laptop".into(), "a1".into()))
        );
        assert_eq!(grant_key_target(Some(&p), Some("laptop/a2")), None);
        assert_eq!(grant_key_target(Some(&p), None), None);
        assert_eq!(grant_key_target(None, Some("laptop/a1")), None);
    }

    /// The key cannot come from typing to an agent: in the pane every key, `g` and a paste included, goes to the agent; the
    /// console's own `g` is reached from the list (Ctrl-] then g). A paste starting with `y` while the notice shows is typing too.
    #[test]
    fn typing_in_the_pane_never_opens_the_question_and_ctrl_bracket_g_does() {
        let rows = vec![Row {
            id: "laptop/a1".into(),
            label: "a1".into(),
            state: String::new(),
            needs_you: false,
            terminal: true,
            header: false,
            group: false,
            request: false,
            request_always: false,
            approval: String::new(),
        }];
        let text = |id: &str, _: &[(&str, Arg)]| id.to_string();
        let cx = Ctx {
            rows: &rows,
            area: Area::new(0, 0, 120, 40),
            modes: Default::default(),
            prog: Default::default(),
            covered: false,
            t: &text,
        };
        let mut app = App::default();
        let _ = app.handle(AppEvent::Key(Key::Enter), &cx);
        assert_eq!(app.active_agent(), Some("laptop/a1"));
        let typed = [
            app.handle(AppEvent::Key(Key::Char('g')), &cx),
            app.handle(AppEvent::Paste("yes".into()), &cx),
            app.handle(AppEvent::Key(Key::Char('y')), &cx),
        ];
        for acts in &typed {
            assert!(
                acts.iter().all(|a| matches!(a, Action::Input(_))),
                "{acts:?}"
            );
            assert!(!acts.is_empty());
        }
        let _ = app.handle(AppEvent::Key(Key::Ctrl(']')), &cx);
        assert_eq!(
            app.handle(AppEvent::Key(Key::Char('g')), &cx),
            vec![Action::Command('g')]
        );
    }

    /// Inside the question (opened on its key), a bracketed paste keeps its framing around the keys set aside and never answers.
    #[test]
    #[allow(clippy::unwrap_used)]
    fn a_paste_around_the_grant_question_never_answers_it_and_keys_after_it_still_do() {
        let area = Area::new(0, 0, 80, 24);
        let q = ["q".to_string()];
        let (tx, rx) = mpsc::channel();
        let send = |bytes: &[u8]| bytes.iter().for_each(|b| tx.send(*b).unwrap());
        let mut p = Parser::default();
        send(b"\x1b[200~ab");
        set_aside_keys(&mut p, &rx);
        assert_eq!(one_answer(&p.push(b"y\x1b[201~"), area, &q), None);
        assert_eq!(one_answer(&p.push(b"y"), area, &q), Some(b'y'));
        let mut p = Parser::default();
        assert!(p.push(b"\x1b[200~ab").is_empty());
        send(b"cd\x1b[201~");
        set_aside_keys(&mut p, &rx);
        assert_eq!(one_answer(&p.push(b"n"), area, &q), Some(b'n'));
        let mut p = Parser::default();
        send(b"\x1b[20");
        set_aside_keys(&mut p, &rx);
        assert_eq!(one_answer(&p.push(b"0~y\x1b[201~"), area, &q), None);
        assert_eq!(one_answer(&p.push(b"yes"), area, &q), None);
    }

    /// An asker over a key queue: `while_drawn` arrives while the question is drawn (typed before it could be seen), `on_show` once
    /// it is on the screen (after the keys typed until then were set aside).
    struct Keys<'a> {
        rx: &'a mpsc::Receiver<u8>,
        tx: &'a mpsc::Sender<u8>,
        while_drawn: &'a [u8],
        on_show: &'a [u8],
        drawn: usize,
    }
    impl Asker for Keys<'_> {
        fn rx(&self) -> &mpsc::Receiver<u8> {
            self.rx
        }
        fn show(&mut self, _: &[String]) {
            if self.drawn == 0 {
                self.while_drawn.iter().for_each(|k| {
                    let _ = self.tx.send(*k);
                });
            }
            self.drawn += 1;
        }
        fn discard(&mut self) {
            while self.rx.try_recv().is_ok() {}
            self.on_show.iter().for_each(|k| {
                let _ = self.tx.send(*k);
            });
        }
        fn frame(&mut self, _: &[u8]) {}
    }

    /// A key queued before the question never answers it; the one answer after its one draw does; a framed paste never does.
    #[test]
    #[allow(clippy::unwrap_used)]
    fn only_a_key_pressed_after_the_grant_question_answers_it() {
        let (tx, rx) = mpsc::channel();
        let wait = Duration::from_millis(5);
        for k in *b"yyy" {
            tx.send(k).unwrap();
        }
        let mut ask = Keys {
            rx: &rx,
            tx: &tx,
            while_drawn: b"",
            on_show: b"",
            drawn: 0,
        };
        assert_eq!(fresh_key(&mut ask, &["q".into()], wait), None);
        let mut ask = Keys {
            rx: &rx,
            tx: &tx,
            while_drawn: b"",
            on_show: b"y",
            drawn: 0,
        };
        assert_eq!(fresh_key(&mut ask, &["q".into()], wait), Some(b'y'));
        assert_eq!(ask.drawn, 1, "the question is drawn once");
        let mut ask = Keys {
            rx: &rx,
            tx: &tx,
            while_drawn: b"",
            on_show: b"\x1b[200~y\x1b[201~",
            drawn: 0,
        };
        assert_eq!(
            fresh_key(&mut ask, &["q".into()], Duration::from_millis(20)),
            None
        );
        // Codex review of #796: a `y` that came while the question was being drawn was typed before it could be seen.
        let mut ask = Keys {
            rx: &rx,
            tx: &tx,
            while_drawn: b"y",
            on_show: b"",
            drawn: 0,
        };
        assert_eq!(fresh_key(&mut ask, &["q".into()], wait), None);
    }

    /// Codex review of #796: in the split view's questions too, a paste that began before the question and ends after it was drawn
    /// keeps its framing, so its text never answers.
    #[test]
    #[allow(clippy::unwrap_used)]
    fn a_paste_across_a_split_view_question_never_answers_it() {
        let q = ["q".to_string()];
        let wait = Duration::from_millis(20);
        let (tx, rx) = mpsc::channel();
        b"\x1b[200~ab".iter().for_each(|b| tx.send(*b).unwrap());
        let mut plain = Plain::new(&rx);
        plain.discard();
        // The paste's next byte, on its own after the question: still the paste.
        tx.send(b'y').unwrap();
        assert_eq!(read_answer(&mut plain.1, &rx, &q, wait), None);
        // What the old fallback did (raw bytes dropped, a fresh parser for the answer): that `y` signed.
        tx.send(b'y').unwrap();
        assert_eq!(
            read_answer(&mut Parser::default(), &rx, &q, wait),
            Some(b'y')
        );
        // Once the paste ended, a key answers.
        b"\x1b[201~".iter().for_each(|b| tx.send(*b).unwrap());
        assert_eq!(read_answer(&mut plain.1, &rx, &q, wait), None);
        tx.send(b'y').unwrap();
        assert_eq!(read_answer(&mut plain.1, &rx, &q, wait), Some(b'y'));
    }

    /// Pointer motion the console passes on is not typing (its refusal asks nothing); presses, drags, the wheel, keys and pastes are.
    #[test]
    fn only_motion_with_no_button_is_passed_on_rather_than_typed() {
        use agora_tui::app::{Mouse, MouseKind};
        let mouse = |kind| {
            AppEvent::Mouse(Mouse {
                kind,
                ..Default::default()
            })
        };
        assert!(passive_input(&mouse(MouseKind::Move)));
        for kind in [
            MouseKind::Down,
            MouseKind::Up,
            MouseKind::Drag,
            MouseKind::ScrollUp,
            MouseKind::ScrollDown,
        ] {
            assert!(!passive_input(&mouse(kind)), "{kind:?}");
        }
        assert!(!passive_input(&AppEvent::Key(Key::Char('y'))));
        assert!(!passive_input(&AppEvent::Paste("y".into())));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod confirm_give_tests {
    use super::*;

    /// An asker that, for each time the confirmation is drawn, first gets the keys typed before it could be seen (held keys,
    /// repeats), then the keys pressed once it is on the screen.
    struct Draws {
        rx: mpsc::Receiver<u8>,
        tx: mpsc::Sender<u8>,
        draws: Vec<(&'static [u8], &'static [u8])>,
        drawn: usize,
        shown: Vec<Vec<String>>,
    }

    impl Asker for Draws {
        fn rx(&self) -> &mpsc::Receiver<u8> {
            &self.rx
        }
        fn show(&mut self, lines: &[String]) {
            self.shown.push(lines.to_vec());
            if let Some((before, _)) = self.draws.get(self.drawn) {
                before.iter().for_each(|k| self.tx.send(*k).unwrap());
            }
        }
        fn discard(&mut self) {
            while self.rx.try_recv().is_ok() {}
            if let Some((_, after)) = self.draws.get(self.drawn) {
                after.iter().for_each(|k| self.tx.send(*k).unwrap());
            }
            self.drawn += 1;
        }
        fn frame(&mut self, _: &[u8]) {}
    }

    fn asker(draws: Vec<(&'static [u8], &'static [u8])>) -> Draws {
        let (tx, rx) = mpsc::channel();
        Draws {
            rx,
            tx,
            draws,
            drawn: 0,
            shown: Vec::new(),
        }
    }

    /// A confirmation whose agent starts acting without asking at the `change`-th look at the hub (0: never); with the warning it
    /// needs 10 rows.
    struct Source {
        polls: usize,
        change: usize,
        gone: bool,
    }

    impl ConfirmSource for Source {
        fn poll(&mut self) -> Refresh {
            self.polls += 1;
            if self.gone {
                Refresh::Gone
            } else if self.polls == self.change {
                Refresh::Changed
            } else {
                Refresh::Same
            }
        }
        fn lines(&self, area: Area) -> Result<Vec<String>, Vec<String>> {
            let warned = self.change > 0 && self.polls >= self.change;
            if warned && area.height < 10 {
                return Err(vec!["too small".into()]);
            }
            let mut l = vec!["q".to_string()];
            if warned {
                l.push("warning".into());
            }
            Ok(l)
        }
    }

    fn source(change: usize) -> Source {
        Source {
            polls: 0,
            change,
            gone: false,
        }
    }

    fn big() -> Area {
        Area::new(0, 0, 80, 24)
    }

    fn confirm(draws: Vec<(&'static [u8], &'static [u8])>) -> (bool, usize) {
        let mut ask = asker(draws);
        let given = confirm_give(&mut ask, Duration::from_millis(20), &big, &mut source(0));
        (given, ask.drawn)
    }

    /// ADR-0095 (the web needed this too): Enter, which chose the agent, held or repeated, never gives the task; nor does a `g` typed
    /// before the confirmation was on the screen, a held `g` arriving as several keys at once, or a pasted `g`. Only a fresh `g` does.
    #[test]
    fn only_a_fresh_g_after_the_confirmation_is_shown_gives_the_task() {
        // A held Enter: typed before each draw and repeated after it. It keeps asking and, when the keys stop, goes back.
        let (given, drawn) = confirm(vec![(b"\r\r", b"\r"); 5]);
        assert!(!given);
        assert_eq!(drawn, 6, "each Enter asked again");
        // A g typed before the confirmation could be seen is set aside.
        assert_eq!(confirm(vec![(b"g", b"")]), (false, 1));
        // Several keys at once (a held key's burst) and a paste never answer.
        assert!(!confirm(vec![(b"", b"gg")]).0);
        assert!(!confirm(vec![(b"", b"\x1b[200~g\x1b[201~")]).0);
        // n goes back at once.
        assert_eq!(confirm(vec![(b"", b"n")]), (false, 1));
        // A fresh g gives it, also after Enter asked again.
        assert_eq!(confirm(vec![(b"", b"g")]), (true, 1));
        assert_eq!(confirm(vec![(b"\r", b"\r"), (b"\r", b"g")]), (true, 2));
    }

    /// A report that makes the agent act without asking while the confirmation is open (a config option, or a live mode change,
    /// Codex review of #893): it is drawn again with the warning, and a `g` pressed before that, even in the same moment the report
    /// was read, does not give the task; a fresh `g` after it does. A lost link ends it.
    #[test]
    fn a_change_while_the_confirmation_is_open_is_shown_before_g_can_give() {
        let warned = vec!["q".to_string(), "warning".to_string()];
        let mut ask = asker(vec![(b"", b"g"), (b"", b"")]);
        assert!(!confirm_give(
            &mut ask,
            Duration::from_millis(500),
            &big,
            &mut source(1)
        ));
        assert_eq!(ask.shown[1], warned, "{:?}", ask.shown);
        let mut ask = asker(vec![(b"", b""), (b"", b"g")]);
        assert!(confirm_give(
            &mut ask,
            Duration::from_secs(5),
            &big,
            &mut source(1)
        ));
        assert_eq!(ask.shown.last(), Some(&warned));
        // A look that changes nothing draws nothing new.
        let mut ask = asker(vec![(b"", b"g")]);
        let mut same = source(0);
        assert!(confirm_give(
            &mut ask,
            Duration::from_secs(5),
            &big,
            &mut same
        ));
        assert_eq!(ask.shown.len(), 1);
        let mut ask = asker(vec![(b"", b"g")]);
        let mut gone = source(0);
        gone.gone = true;
        assert!(!confirm_give(
            &mut ask,
            Duration::from_secs(5),
            &big,
            &mut gone
        ));
    }

    /// Codex review of #893: where the window cannot show the lines that must be read (the warning, the keys), the confirmation says
    /// the terminal is too small and `g` gives nothing; once the window is large enough, a fresh `g` does.
    #[test]
    fn a_window_too_small_for_the_warning_never_gives_the_task() {
        let small = || Area::new(0, 0, 80, 8);
        // The agent acts without asking from the start (the change is read at once) in a small window: g only asks again.
        let mut ask = asker(vec![(b"", b""), (b"", b"g"), (b"", b"g"), (b"", b"")]);
        let mut s = source(1);
        assert!(!confirm_give(
            &mut ask,
            Duration::from_millis(700),
            &small,
            &mut s
        ));
        assert!(
            ask.shown
                .iter()
                .skip(1)
                .all(|l| l == &["too small".to_string()]),
            "{:?}",
            ask.shown
        );
        // The window grows: the whole confirmation is drawn and a fresh g gives the task.
        let size = std::cell::Cell::new(Area::new(0, 0, 80, 8));
        let area = || size.get();
        let mut ask = asker(vec![(b"", b""), (b"", b"g"), (b"", b"g")]);
        let mut s = source(1);
        // Grown before the third draw.
        struct Growing<'a> {
            inner: &'a mut Source,
            size: &'a std::cell::Cell<Area>,
        }
        impl ConfirmSource for Growing<'_> {
            fn poll(&mut self) -> Refresh {
                let r = self.inner.poll();
                if self.inner.polls == 2 {
                    self.size.set(Area::new(0, 0, 80, 24));
                }
                r
            }
            fn lines(&self, area: Area) -> Result<Vec<String>, Vec<String>> {
                self.inner.lines(area)
            }
        }
        let mut g = Growing {
            inner: &mut s,
            size: &size,
        };
        assert!(confirm_give(
            &mut ask,
            Duration::from_secs(5),
            &area,
            &mut g
        ));
        assert_eq!(
            ask.shown.last(),
            Some(&vec!["q".to_string(), "warning".to_string()])
        );
    }
}

#[cfg(test)]
mod adr0069_tests {
    use super::*;
    use agora_i18n::Catalog;

    fn kind(name: &str, installed: bool, program: &str) -> agora_tui::client::Kind {
        agora_tui::client::Kind {
            kind: name.to_lowercase(),
            name: name.into(),
            installed,
            program: program.into(),
        }
    }

    #[test]
    fn a_missing_agent_names_the_program_its_machine_looked_for() {
        let say = Say(Catalog::english());
        let (ok, line) = split_kinds(
            &say,
            vec![
                kind("Claude Code", true, "claude"),
                kind("WrongStack", false, "wstack"),
                // An older machine says no program: the agent's name alone.
                kind("Codex", false, ""),
            ],
        );
        assert_eq!(
            ok,
            vec![("claude code".to_string(), "Claude Code".to_string())]
        );
        let line = line.unwrap_or_default();
        assert!(line.contains("WrongStack (`wstack`), Codex."), "{line}");
        assert!(
            line.contains("cannot find their programs in its PATH"),
            "{line}"
        );
        assert_eq!(
            split_kinds(&say, vec![kind("Claude Code", true, "claude")]).1,
            None
        );
    }

    #[test]
    fn the_trust_hint_names_the_folder_of_agora_when_the_machine_said_it() {
        let say = Say(Catalog::english());
        let s = Snapshot {
            nodes: vec![
                // The hub's owner receives the full path.
                agora_protocol::v1::NodeSnapshot {
                    node_id: "win".into(),
                    binary: r"C:\Users\alice\Downloads\agora.exe".into(),
                    ..Default::default()
                },
                agora_protocol::v1::NodeSnapshot {
                    node_id: "lin".into(),
                    binary: "/home/alice/agora".into(),
                    ..Default::default()
                },
                // Everyone else receives the file name alone.
                agora_protocol::v1::NodeSnapshot {
                    node_id: "other".into(),
                    binary: "agora".into(),
                    ..Default::default()
                },
                agora_protocol::v1::NodeSnapshot {
                    node_id: "mac".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        assert_eq!(
            node_binary(&s, "win").as_deref(),
            Some(r"Downloads\agora.exe")
        );
        assert_eq!(node_binary(&s, "lin").as_deref(), Some("alice/agora"));
        assert_eq!(node_binary(&s, "other"), None);
        assert_eq!(node_binary(&s, "mac"), None);
        assert_eq!(node_binary(&s, "gone"), None);
        let (device, _) = Device::generate().unwrap_or_else(|| panic!("a device key"));
        let at = not_trusted(&say, &device, node_binary(&s, "win").as_deref());
        assert!(
            at.contains(r"in the folder of Downloads\agora.exe, run: ./agora node trust"),
            "{at}"
        );
        assert!(at.contains(&device.fingerprint()), "{at}");
        let general = not_trusted(&say, &device, None);
        assert!(
            general.contains("run it in the folder of the agora program"),
            "{general}"
        );
        assert!(general.contains(&device.public_key_hex()), "{general}");
    }
}

#[cfg(test)]
mod skipping_start_tests {
    use super::*;
    use agora_i18n::Catalog;

    /// Answers each question with the next key, and keeps what it was shown.
    struct Script {
        rx: mpsc::Receiver<u8>,
        shown: Vec<String>,
    }

    impl Asker for Script {
        fn rx(&self) -> &mpsc::Receiver<u8> {
            &self.rx
        }
        fn show(&mut self, lines: &[String]) {
            self.shown.extend_from_slice(lines);
        }
        fn frame(&mut self, _: &[u8]) {}
    }

    fn script(keys: &[u8]) -> Script {
        let (tx, rx) = mpsc::channel();
        for k in keys {
            tx.send(*k).unwrap_or_else(|_| panic!("send"));
        }
        Script {
            rx,
            shown: Vec::new(),
        }
    }

    /// ADR-0059 decision 5: y allows a start whose helper would act without asking only after the confirmation, naming the option in
    /// this terminal's words; any other key keeps asking. A start without such an option is not asked about.
    #[test]
    fn y_on_a_start_that_acts_without_asking_asks_for_the_confirmation_first() {
        let snapshot = |detail: serde_json::Value| Snapshot {
            approvals: vec![agora_protocol::v1::ApprovalSnapshot {
                approval_id: "mac2/a1".into(),
                agent_id: "mac2/agent-1".into(),
                action: "agent.start".into(),
                detail: detail.to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let skipping = snapshot(serde_json::json!({"kind": "codex", "prompt": "x",
            "launch_options": [{"id": "skip-permissions", "label": "option-skip-permissions", "skips_permissions": true}]}));
        let say = Say(Catalog::english());
        let text = |id: &str, args: &[(&str, Arg)]| say.text(id, args);
        let mut yes = script(b"s");
        assert!(confirm_skipping_start(
            &say,
            &skipping,
            "mac2/agent-1",
            &text,
            &mut yes
        ));
        let asked = yes.shown.join(" ");
        assert!(
            asked.contains("Let the helper act without asking? With \"Don't ask\""),
            "{asked}"
        );
        assert!(asked.contains("Press s"), "{asked}");
        // The key that opened it, held or repeated, keeps asking: a repeated y never confirms.
        let mut repeated = script(b"yyyy");
        assert!(!confirm_skipping_start(
            &say,
            &skipping,
            "mac2/agent-1",
            &text,
            &mut repeated
        ));
        for key in [b'y', b'Y', b'S', b'n', b'\r', 0x1b] {
            let mut other = script(&[key]);
            assert!(!confirm_skipping_start(
                &say,
                &skipping,
                "mac2/agent-1",
                &text,
                &mut other
            ));
        }
        // The card says how it acts and what that means, before the prompt, in the lines y answers; an agent's own label is not used.
        let lines = start_card_lines(&skipping.approvals[0], 80, &text).unwrap_or_default();
        let at = |needle: &str| lines.iter().position(|l| l.contains(needle));
        assert!(at("How it acts: Don't ask").is_some(), "{lines:?}");
        assert!(
            at("If you allow it, this helper runs commands").is_some(),
            "{lines:?}"
        );
        let prompt = lines.iter().position(|l| l == "  x");
        assert!(prompt.is_some() && at("How it acts") < prompt, "{lines:?}");
        let odd = snapshot(serde_json::json!({"kind": "codex", "prompt": "x",
            "launch_options": [{"id": "skip-permissions", "label": "Totally safe", "skips_permissions": true}]}));
        let odd_lines = start_card_lines(&odd.approvals[0], 80, &text).unwrap_or_default();
        assert!(
            odd_lines
                .iter()
                .any(|l| l.contains("How it acts: skip-permissions")),
            "{odd_lines:?}"
        );
        assert!(
            !odd_lines.iter().any(|l| l.contains("Totally safe")),
            "{odd_lines:?}"
        );
        // No key at all (the wait ran out) keeps asking too.
        assert!(!confirm_skipping_start(
            &say,
            &skipping,
            "mac2/agent-1",
            &text,
            &mut script(&[])
        ));
        // A start without options, and one whose options do not skip permissions, are answered as before.
        for detail in [
            serde_json::json!({"kind": "codex", "prompt": "x"}),
            serde_json::json!({"kind": "codex", "launch_options": [{"id": "fast", "label": "option-fast", "skips_permissions": false}]}),
        ] {
            let mut none = script(&[]);
            assert!(confirm_skipping_start(
                &say,
                &snapshot(detail),
                "mac2/agent-1",
                &text,
                &mut none
            ));
            assert!(none.shown.is_empty());
        }
    }
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

    /// ADR-0066: the card lists the settings in this terminal's words, before the prompt, and they count toward what must be read: a
    /// request that fits the panel without them but not with them is `unread` until the reader showed all of it.
    #[test]
    #[allow(clippy::unwrap_used)]
    fn a_start_card_shows_the_settings_in_words_and_they_count_toward_what_must_be_read() {
        let card = |settings: serde_json::Value, prompt_lines: usize| {
            let detail = serde_json::json!({
                "node": "mac2", "agent": "agent-1", "agent_kind": "claude-code", "kind": "codex", "folder": "/w",
                "inside": true, "worktree": false, "depth": 1, "grant_allowed": false, "settings": settings,
                "prompt": (1..=prompt_lines).map(|i| format!("step {i}")).collect::<Vec<_>>().join("\n"),
            })
            .to_string();
            Snapshot {
                approvals: vec![agora_protocol::v1::ApprovalSnapshot {
                    approval_id: "mac2/a1".into(),
                    agent_id: "mac2/agent-1".into(),
                    action: "agent.start".into(),
                    detail,
                    ..Default::default()
                }],
                ..Default::default()
            }
        };
        let say = Say(Catalog::english());
        let text = |id: &str, args: &[(&str, Arg)]| say.text(id, args);
        let settings = serde_json::json!([
            {"id": "model", "label": "setting-model", "value": "gpt-6.1-sol"},
            // A label that is not one of this terminal's words, and a value with a hidden character: shown as the id and made visible.
            {"id": "effort", "label": "Pay no attention", "value": "high\u{1b}[2J"},
        ]);
        let with = card(settings, 3);
        let lines = start_card_lines(&with.approvals[0], 80, &text).unwrap();
        let at = |needle: &str| lines.iter().position(|l| l.contains(needle)).unwrap();
        assert!(
            lines[at("Model: gpt-6.1-sol")].starts_with("  "),
            "{lines:?}"
        );
        assert!(
            at("Model: gpt-6.1-sol") < at("effort: high\\u{1b}[2J"),
            "{lines:?}"
        );
        assert!(
            at("effort: high") < at("step 1"),
            "the settings come before the prompt: {lines:?}"
        );
        assert!(
            !lines
                .iter()
                .any(|l| l.contains('\u{1b}') || l.contains("Pay no attention")),
            "{lines:?}"
        );
        // Without settings there is nothing new on the card.
        let plain = card(serde_json::json!([]), 3);
        let plain_lines = start_card_lines(&plain.approvals[0], 80, &text).unwrap();
        assert_eq!(plain_lines.len() + 2, lines.len());
        // Room for exactly the plain card: with its two settings it no longer fits, so y waits for the reader.
        let rows = u16::try_from(plain_lines.len() + 2).unwrap();
        let none = std::collections::HashSet::new();
        assert!(start_unread(&plain, "mac2/agent-1", false, &none, &text, (100, rows)).is_none());
        assert!(start_unread(&with, "mac2/agent-1", false, &none, &text, (100, rows)).is_some());
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
    fn a_start_or_resume_the_hub_refused_for_the_persons_role_says_the_role_not_a_device() {
        // #1044: the hub refuses a viewer's (or a member's outside their projects) command naming the command's schema name; a machine
        // that limits this device to other projects names its own action.
        let refused = |action: &str| {
            Error::new(ErrorCode::AuthForbidden)
                .with("action", action)
                .to_info()
        };
        for say in [Say(Catalog::english()), Say(Catalog::turkish())] {
            let role = say.text(key!("tui-start-role"), &[]);
            assert_eq!(
                start_refusal(&say, &refused("AgentStart"), "AgentStart"),
                role
            );
            assert_eq!(
                start_refusal(&say, &refused("DirectoryInspect"), "DirectoryInspect"),
                role
            );
            let code = say.text(
                key!("tui-start-failed"),
                &[("code", Arg::Str("auth.forbidden".into()))],
            );
            assert_eq!(
                start_refusal(&say, &refused("directory"), "DirectoryInspect"),
                code
            );
            assert_eq!(
                start_refusal(&say, &refused("AgentStop"), "AgentStart"),
                code
            );
            assert_eq!(
                resume_notice(&say, &refused("AgentResume")),
                say.text(key!("tui-resume-role"), &[])
            );
            assert_ne!(
                resume_notice(&say, &refused("directory")),
                say.text(key!("tui-resume-role"), &[])
            );
            // The folder picker's `directory.list` is the first command a folder start sends.
            assert!(say.0.render("tui-folder-role", &[]).is_some());
        }
        assert!(refused_for_role(&refused("DirectoryList"), "DirectoryList"));
        assert!(!refused_for_role(&refused("directory"), "DirectoryList"));
        assert!(!refused_for_role(
            &Error::new(ErrorCode::NodeOffline)
                .with("action", "DirectoryList")
                .to_info(),
            "DirectoryList"
        ));
        let en = Say(Catalog::english());
        for line in [
            start_refusal(&en, &refused("AgentStart"), "AgentStart"),
            resume_notice(&en, &refused("AgentResume")),
        ] {
            assert!(
                line.starts_with("Your role") && !line.contains("device"),
                "{line}"
            );
        }
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

    #[test]
    fn an_agent_that_waits_counts_once_in_the_footer() {
        let blocked = |id: &str| agora_protocol::v1::AgentSnapshot {
            agent_id: id.into(),
            running: true,
            state: agora_protocol::v1::AgentState::Blocked as i32,
            ..Default::default()
        };
        let request = |agent: &str| agora_protocol::v1::ApprovalSnapshot {
            agent_id: agent.into(),
            ..Default::default()
        };
        // One agent, blocked and with its request: one thing needs the user (the review saw "2 need you").
        let snap = Snapshot {
            agents: vec![blocked("n/agent-1")],
            approvals: vec![request("n/agent-1")],
            ..Default::default()
        };
        assert_eq!(needs_you_count(&snap), 1);
        // Two agents (one with two requests), and a request of no agent: three.
        let snap = Snapshot {
            agents: vec![blocked("n/agent-1")],
            approvals: vec![request("n/agent-2"), request("n/agent-2"), request("")],
            ..Default::default()
        };
        assert_eq!(needs_you_count(&snap), 3);
        // Only its own request is the chat tab's to answer.
        assert!(waiting_for(&snap, "n/agent-1").is_none());
        assert!(waiting_for(&snap, "n/agent-2").is_some());
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod read_keys_tests {
    use super::*;
    use agora_tui::app::{Key, Mouse, MouseKind};

    /// The console loop's keyboard side, driven turn by turn: what the reader thread handed over before a turn, and when it ran.
    struct Keys {
        tx: mpsc::Sender<u8>,
        rx: mpsc::Receiver<u8>,
        parser: Parser,
        typed_at: Option<Instant>,
        t0: Instant,
    }

    impl Keys {
        fn new() -> Self {
            let (tx, rx) = mpsc::channel();
            Keys {
                tx,
                rx,
                parser: Parser::default(),
                typed_at: None,
                t0: Instant::now(),
            }
        }

        /// One turn of the loop `ms` after the start, with `bytes` sent by the reader thread since the turn before.
        fn turn(&mut self, ms: u64, bytes: &[u8]) -> Vec<AppEvent> {
            for &b in bytes {
                self.tx.send(b).unwrap();
            }
            let mut events = Vec::new();
            read_keys(
                &self.rx,
                &mut self.parser,
                &mut self.typed_at,
                self.t0 + Duration::from_millis(ms),
                &mut events,
            );
            events
        }

        fn turns(&mut self, turns: &[(u64, &[u8])]) -> Vec<AppEvent> {
            turns.iter().flat_map(|&(ms, b)| self.turn(ms, b)).collect()
        }
    }

    fn k(k: Key) -> AppEvent {
        AppEvent::Key(k)
    }

    /// Checks every case, printing each, and fails at the end if any differed.
    fn check(cases: Vec<(&str, Vec<AppEvent>, Vec<AppEvent>)>) {
        let mut failed = Vec::new();
        for (name, expected, actual) in cases {
            println!("{name}\n  EXPECTED: {expected:?}\n  ACTUAL:   {actual:?}");
            if expected != actual {
                failed.push(name);
            }
        }
        assert!(failed.is_empty(), "differed: {failed:?}");
    }

    #[test]
    fn an_escape_sequence_whose_bytes_reach_separate_turns_is_still_one_key() {
        // The reader thread hands bytes over one at a time, so a turn of the loop (2 ms while typing, at once while the hub's frames
        // stream in) can find the channel empty between the bytes of one key. A turn that found nothing flushed the parser then:
        // an arrow came out as Esc and the text `[A` typed into the agent, a click as Alt-[ and `<0;10;5M`.
        let down = AppEvent::Mouse(Mouse {
            kind: MouseKind::Down,
            col: 9,
            row: 4,
            ..Default::default()
        });
        check(vec![
            (
                "arrow split after ESC",
                vec![k(Key::Up)],
                Keys::new().turns(&[(0, b"\x1b"), (1, b""), (2, b"[A")]),
            ),
            (
                "mouse report split after ESC [",
                vec![down.clone()],
                Keys::new().turns(&[(0, b"\x1b["), (1, b""), (2, b"<0;10;5M")]),
            ),
            (
                "paste start split after ESC [ 2",
                vec![AppEvent::Paste("ls\r".into())],
                Keys::new().turns(&[(0, b"\x1b[2"), (1, b""), (2, b"00~ls\r\x1b[201~")]),
            ),
            // Controls: unaffected before the fix as after it.
            (
                "control: arrow in one turn",
                vec![k(Key::Up)],
                Keys::new().turns(&[(0, b"\x1b[A")]),
            ),
            (
                "control: mouse report in one turn",
                vec![down],
                Keys::new().turns(&[(0, b"\x1b[<0;10;5M")]),
            ),
            (
                "control: lone Esc, nothing more for 100 ms",
                vec![k(Key::Esc)],
                Keys::new().turns(&[(0, b"\x1b"), (100, b"")]),
            ),
            (
                "control: UTF-8 character split by an empty turn",
                vec![k(Key::Char('ş'))],
                Keys::new().turns(&[(0, b"\xc5"), (1, b""), (2, b"\x9f")]),
            ),
        ]);
    }

    #[test]
    fn a_pause_of_key_gap_after_the_last_byte_still_ends_a_sequence() {
        let gap = u64::try_from(KEY_GAP.as_millis()).unwrap();
        let mut lone = Keys::new();
        let before = lone.turns(&[(0, b"\x1b"), (2, b""), (gap - 1, b"")]);
        check(vec![
            // A lone Esc keypress: nothing until KEY_GAP after the byte, Esc at exactly KEY_GAP.
            ("lone Esc before KEY_GAP", vec![], before),
            (
                "lone Esc at KEY_GAP",
                vec![k(Key::Esc)],
                lone.turn(gap, b""),
            ),
            // The pause counts from the last byte, not the last turn: empty turns in between change nothing.
            (
                "arrow after many empty turns within KEY_GAP",
                vec![k(Key::Down)],
                Keys::new().turns(&[
                    (0, b"\x1b"),
                    (2, b""),
                    (4, b""),
                    (6, b"["),
                    (8, b""),
                    (gap + 5, b"B"),
                ]),
            ),
            // Alt+x as a terminal sends it, split by an empty turn, stays Alt+x.
            (
                "Alt+x split by an empty turn",
                vec![k(Key::Alt('x'))],
                Keys::new().turns(&[(0, b"\x1b"), (1, b""), (2, b"x")]),
            ),
            // An Esc that really was alone, then text: Esc, then the text, nothing lost or reordered.
            (
                "Esc after a pause, then text",
                vec![
                    k(Key::Char('a')),
                    k(Key::Esc),
                    k(Key::Char('[')),
                    k(Key::Char('A')),
                ],
                Keys::new().turns(&[(0, b"a\x1b"), (gap, b""), (gap + 1, b"[A")]),
            ),
            // A mouse release split twice (the `m` must not reach the shell as a key).
            (
                "mouse release split twice",
                vec![AppEvent::Mouse(Mouse {
                    kind: MouseKind::Up,
                    col: 9,
                    row: 4,
                    ..Default::default()
                })],
                Keys::new().turns(&[
                    (0, b"\x1b"),
                    (1, b""),
                    (2, b"[<0;1"),
                    (3, b""),
                    (4, b"0;5m"),
                ]),
            ),
            // Nothing typed yet: an empty turn is nothing.
            (
                "no keys at all",
                vec![],
                Keys::new().turns(&[(0, b""), (100, b"")]),
            ),
        ]);
    }
}

#[cfg(test)]
mod escape_key_tests {
    use super::*;

    /// What `read_key_or_gone` returns.
    type Got = Result<Option<u8>, ()>;

    /// What `read_key_or_gone` makes of `bytes` (all sent before it reads; the keyboard then closes, so nothing waits on the clock),
    /// and the bytes it left for whoever reads next (the console's parser once a view or picker returns).
    fn read(bytes: &[u8]) -> (Got, Vec<u8>) {
        let (tx, rx) = mpsc::channel();
        for &b in bytes {
            let _ = tx.send(b);
        }
        drop(tx);
        let got = read_key_or_gone(&rx, Duration::from_millis(50));
        (got, rx.try_iter().collect())
    }

    #[test]
    fn a_longer_escape_sequence_is_read_whole_and_never_closes_a_view_as_esc() {
        // The review, the files and the reader close on Esc (0x1b). They keep the console's mouse reporting, so the wheel sends
        // `ESC [ < 65 ; x ; y M`; Page Down sends `ESC [ 6 ~`. Only two bytes after ESC were read: the view closed as if Esc was
        // pressed and the rest of the sequence was left for the console, which read it as typed keys.
        let cases: [(&str, &[u8], Got); 7] = [
            ("wheel down in a view", b"\x1b[<65;10;3M", Ok(None)),
            ("click in a view", b"\x1b[<0;10;3M", Ok(None)),
            ("Page Down", b"\x1b[6~", Ok(None)),
            ("Ctrl+Down (xterm modifier)", b"\x1b[1;5B", Ok(Some(b'j'))),
            // Controls: unaffected before the fix as after it.
            ("control: Down", b"\x1b[B", Ok(Some(b'j'))),
            ("control: lone Esc", b"\x1b", Ok(Some(0x1b))),
            ("control: q", b"q", Ok(Some(b'q'))),
        ];
        let mut failed = Vec::new();
        for (name, bytes, key) in cases {
            let (got, left) = read(bytes);
            println!(
                "{name}\n  EXPECTED: key {key:?}, left for the console {:?}\n  ACTUAL:   key {got:?}, left for the console {:?}",
                "",
                String::from_utf8_lossy(&left)
            );
            if got != key || !left.is_empty() {
                failed.push(name);
            }
        }
        assert!(failed.is_empty(), "differed: {failed:?}");
    }

    #[test]
    fn what_follows_an_escape_sequence_is_the_next_key_and_unfinished_ones_are_esc() {
        let (tx, rx) = mpsc::channel();
        for &b in b"\x1b[6~q\x1b[<64;1;1Mk" {
            let _ = tx.send(b);
        }
        drop(tx);
        let wait = Duration::from_millis(50);
        let mut got = Vec::new();
        for _ in 0..5 {
            got.push(read_key_or_gone(&rx, wait));
        }
        let cases = [
            // Page Down, q, the wheel, k, then the keyboard is gone: in order, nothing lost or eaten.
            (
                "keys after sequences",
                format!(
                    "{:?}",
                    [Ok(None), Ok(Some(b'q')), Ok(None), Ok(Some(b'k')), Err(())]
                ),
                format!("{got:?}"),
            ),
            // `ESC [` with nothing after it is Esc, as before.
            (
                "unfinished ESC [",
                format!("{:?}", (Ok::<_, ()>(Some(0x1bu8)), "")),
                {
                    let (k, left) = read(b"\x1b[");
                    format!("{:?}", (k, String::from_utf8_lossy(&left)))
                },
            ),
            // Alt and a key: Esc, both bytes taken, as before.
            ("Alt+x", format!("{:?}", (Ok::<_, ()>(Some(0x1bu8)), "")), {
                let (k, left) = read(b"\x1bx");
                format!("{:?}", (k, String::from_utf8_lossy(&left)))
            }),
            // The earlier split view's questions read the same way.
            (
                "read_key: Page Down then y",
                format!("{:?}", (None::<u8>, Some(b'y'))),
                {
                    let (tx, rx) = mpsc::channel();
                    for &b in b"\x1b[5~y" {
                        let _ = tx.send(b);
                    }
                    drop(tx);
                    format!("{:?}", (read_key(&rx, wait), read_key(&rx, wait)))
                },
            ),
        ];
        let mut failed = Vec::new();
        for (name, expected, actual) in cases {
            println!("{name}\n  EXPECTED: {expected}\n  ACTUAL:   {actual}");
            if expected != actual {
                failed.push(name);
            }
        }
        assert!(failed.is_empty(), "differed: {failed:?}");
    }
}

/// Follow-up #829: the console's question path (paste framing, refresh, answers).
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod question_path_tests {
    use super::*;
    use agora_tui::app::{Key, Mouse, MouseKind};

    const PASTE_ON: &[u8] = b"\x1b[200~";

    fn area() -> Area {
        Area::new(0, 0, 100, 30)
    }

    fn key(c: char) -> AppEvent {
        AppEvent::Key(Key::Char(c))
    }

    fn moved() -> AppEvent {
        AppEvent::Mouse(Mouse {
            kind: MouseKind::Move,
            col: 3,
            row: 4,
            ..Mouse::default()
        })
    }

    /// P1: turning the mouse off or on writes no paste framing (DECSET 2004), so a paste that began before a question stays framed
    /// while the mouse is released; the framing is written by its own function, once per change.
    #[test]
    fn the_mouse_never_writes_paste_framing() {
        for on in [true, false] {
            let mut out = Vec::new();
            write_mouse(&mut out, on);
            assert!(!String::from_utf8_lossy(&out).contains("2004"), "{on}");
        }
        let state = std::sync::atomic::AtomicBool::new(false);
        let mut out = Vec::new();
        assert!(!set_paste(&mut out, &state, true));
        write_mouse(&mut out, false);
        // Still on after the mouse went off: nothing switched it off, and asking again writes nothing.
        assert!(set_paste(&mut out, &state, true));
        let text = String::from_utf8_lossy(&out).to_string();
        assert_eq!(text.matches("\x1b[?2004h").count(), 1, "{text:?}");
        assert!(!text.contains("2004l"));
        assert!(set_paste(&mut out, &state, false));
        assert!(String::from_utf8_lossy(&out).ends_with("\x1b[?2004l"));
    }

    /// A refusal still pending when a routine refresh replaces the session stays (renumbered); what was counted in the old session's
    /// numbers ends.
    #[test]
    fn a_refusal_is_kept_across_a_refresh() {
        let mut refused = Some(TypingRefused {
            agent: "laptop/a1".into(),
            request: 7,
        });
        let mut allowed = Some(("laptop/a1".to_string(), 5));
        let mut taken = Taken::new();
        taken.insert(("laptop".into(), "t-a1".into()), 6);
        carry_refusal(&mut refused, &mut allowed, &mut taken);
        assert_eq!(
            refused,
            Some(TypingRefused {
                agent: "laptop/a1".into(),
                request: 0
            })
        );
        assert!(allowed.is_none() && taken.is_empty());
    }

    /// P2: a routine refresh waits for typing replies still in flight, but not for ever.
    #[test]
    fn a_refresh_waits_for_typing_replies() {
        let due = Duration::from_millis(REFRESH_MS);
        assert!(!refresh_due(due - Duration::from_millis(1), 0));
        assert!(refresh_due(due, 0));
        assert!(!refresh_due(due, 2), "a keystroke is still in flight");
        assert!(!refresh_due(
            due + REFRESH_PATIENCE - Duration::from_millis(1),
            1
        ));
        assert!(refresh_due(due + REFRESH_PATIENCE, 1));
    }

    /// P2: the question after a refused keystroke reads the rest of a paste, whose opening marker was forwarded, as a paste.
    #[test]
    fn a_paste_forwarded_before_the_question_goes_on_as_a_paste() {
        let lines = vec!["Allow?".to_string()];
        // Control: a fresh parser takes the pasted `y` for an answer.
        let (tx, rx) = mpsc::channel();
        tx.send(b'y').unwrap();
        assert_eq!(
            read_answer(
                &mut Parser::default(),
                &rx,
                &lines,
                Duration::from_millis(200)
            ),
            Some(b'y')
        );
        // The forwarded batch ended inside a paste: its `y` is text, and the end marker closes it.
        let mut tracker = Parser::default();
        track_forwarded(&mut tracker, [PASTE_ON, b"abc"].concat().as_slice());
        let (tx, rx) = mpsc::channel();
        let mut plain = Plain::with(&rx, tracker);
        for b in b"y\x1b[201~" {
            tx.send(*b).unwrap();
        }
        assert_eq!(
            read_answer(&mut plain.1, plain.0, &lines, Duration::from_millis(200)),
            None
        );
        // The paste is over: now a deliberate `y` answers.
        tx.send(b'y').unwrap();
        assert_eq!(
            read_answer(&mut plain.1, plain.0, &lines, Duration::from_millis(200)),
            Some(b'y')
        );
    }

    /// P2: a question over the console keeps the open terminal (and with it its scrollback and learned modes). A source check, since
    /// `over_console` needs a live `Session` (a TLS connection to a hub), which a unit test cannot make.
    #[test]
    fn a_question_does_not_close_the_preview() {
        // A Windows checkout may turn line ends into CRLF; the searches below use LF.
        let src = include_str!("tuicmd.rs").replace("\r\n", "\n");
        let a = src.find("\nfn over_console<").unwrap();
        let body = &src[a..a + src[a..].find("\n}\n").unwrap()];
        assert!(
            !body.contains("preview") && !body.contains(".close("),
            "{body}"
        );
    }

    /// P3: pointer motion with no button in the same read neither answers nor spoils a deliberate key.
    #[test]
    fn passive_motion_does_not_spoil_an_answer() {
        let lines = vec!["Allow?".to_string()];
        assert_eq!(one_answer(&[key('y')], area(), &lines), Some(b'y'));
        assert_eq!(
            one_answer(&[moved(), key('y'), moved()], area(), &lines),
            Some(b'y')
        );
        assert_eq!(one_answer(&[moved(), moved()], area(), &lines), None);
        // Still never several keys, nor a paste.
        assert_eq!(
            one_answer(&[moved(), key('y'), key('y')], area(), &lines),
            None
        );
        assert_eq!(
            one_answer(&[moved(), AppEvent::Paste("y".into())], area(), &lines),
            None
        );
    }
}

#[cfg(test)]
mod start_pick_tests {
    use super::*;
    use agora_i18n::Catalog;

    /// Answers each question with the next key, and keeps each question it was shown.
    struct Script {
        rx: mpsc::Receiver<u8>,
        shown: Vec<Vec<String>>,
    }

    impl Asker for Script {
        fn rx(&self) -> &mpsc::Receiver<u8> {
            &self.rx
        }
        fn show(&mut self, lines: &[String]) {
            self.shown.push(lines.to_vec());
        }
        fn frame(&mut self, _: &[u8]) {}
    }

    fn script(keys: &[u8]) -> Script {
        let (tx, rx) = mpsc::channel();
        for k in keys {
            tx.send(*k).unwrap_or_else(|_| panic!("send"));
        }
        Script {
            rx,
            shown: Vec::new(),
        }
    }

    /// Sixteen installed kinds, as on the machine of RM-59: the last ones are Shell and WrongStack.
    fn sixteen() -> Vec<(String, String)> {
        let mut kinds: Vec<_> = (1..=14)
            .map(|i| (format!("kind{i:02}"), format!("Agent {i:02}")))
            .collect();
        kinds.push(("shell".into(), "Shell".into()));
        kinds.push(("wrongstack".into(), "WrongStack".into()));
        kinds
    }

    fn pick(keys: &[u8], kinds: &[(String, String)]) -> (Option<usize>, Vec<Vec<String>>) {
        let say = Say(Catalog::english());
        let mut ask = script(keys);
        let got = pick_kind(
            &say,
            &mut ask,
            "blackview",
            &[],
            kinds,
            None,
            Duration::from_millis(50),
        );
        (got, ask.shown)
    }

    /// #910 (RM-59): with sixteen kinds every one can be chosen: n turns to the next nine, whose numbers start again at 1, and the
    /// question says so in this terminal's words (from the locale, never the message id).
    #[test]
    fn every_kind_of_sixteen_can_be_chosen_and_the_question_says_how_to_reach_the_rest() {
        let kinds = sixteen();
        let (got, shown) = pick(b"n7", &kinds);
        assert_eq!(got, Some(15), "{shown:#?}");
        assert_eq!(kinds[15].1, "WrongStack");
        assert_eq!(shown.len(), 2, "{shown:#?}");
        assert_eq!(
            shown[0][0],
            "Start which agent on blackview? Press its number, n for the next page (1 of 2), any other key cancels."
        );
        assert!(shown[0].iter().any(|l| l == "9  Agent 09"), "{shown:#?}");
        assert!(!shown[0].iter().any(|l| l.contains("Shell")), "{shown:#?}");
        assert_eq!(
            shown[1][0],
            "Start which agent on blackview? Press its number, n for the next page (2 of 2), any other key cancels."
        );
        assert!(shown[1].iter().any(|l| l == "1  Agent 10"), "{shown:#?}");
        assert!(shown[1].iter().any(|l| l == "6  Shell"), "{shown:#?}");
        assert!(shown[1].iter().any(|l| l == "7  WrongStack"), "{shown:#?}");
        // The 10th kind is 1 on the second page; n on the last page comes back to the first.
        assert_eq!(pick(b"n1", &kinds).0, Some(9));
        assert_eq!(pick(b"nn3", &kinds).0, Some(2));
        // A number past the last page's end, and any other key, cancel.
        assert_eq!(pick(b"n8", &kinds).0, None);
        assert_eq!(pick(b"x", &kinds).0, None);
        assert_eq!(pick(b"", &kinds).0, None);
    }

    /// Nine kinds or fewer fit on one page: the question is the one it always was, and n cancels like any other key.
    #[test]
    fn nine_kinds_or_fewer_keep_the_one_page_question() {
        let kinds: Vec<_> = sixteen().into_iter().take(9).collect();
        let (got, shown) = pick(b"9", &kinds);
        assert_eq!(got, Some(8));
        assert_eq!(
            shown[0][0],
            "Start which agent on blackview? Press its number, any other key cancels."
        );
        assert_eq!(pick(b"n", &kinds).0, None);
    }
}

/// Codex's review of ADR-0101 part B: a tab's Alt key answers exactly that tab's request, signed for it, whatever the tree selects.
#[cfg(test)]
mod tab_answer_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use agora_i18n::Catalog;
    use agora_protocol::v1::{ApprovalScope, ApprovalSnapshot};

    fn ask(n: u32) -> ApprovalSnapshot {
        ApprovalSnapshot {
            approval_id: format!("mac/ap-{n}"),
            agent_id: format!("mac/agent-{n}"),
            action: "run_command".into(),
            detail: format!("git status {n}"),
            scopes: vec![ApprovalScope::Once as i32, ApprovalScope::Always as i32],
            always_label: format!("Always allow git status {n}"),
            always_option_id: format!("opt-{n}"),
            ..Default::default()
        }
    }

    fn two() -> Snapshot {
        Snapshot {
            approvals: vec![ask(1), ask(2)],
            ..Default::default()
        }
    }

    #[test]
    fn the_tabs_request_is_signed_not_the_trees_selection() {
        let s = two();
        // The tree is on agent-1; the keyboard is in agent-2's tab.
        let tab = Target::Tab {
            agent: "mac/agent-2".into(),
            approval: "mac/ap-2".into(),
        };
        let (a, owner) = target_request(&s, "mac/agent-1", &tab).unwrap();
        assert_eq!(
            (a.approval_id.as_str(), owner.as_str()),
            ("mac/ap-2", "mac/agent-2")
        );
        let (bytes, scope) = agora_tui::client::answer_message(&a, Answer::AllowAlways).unwrap();
        assert_eq!(scope, ApprovalScope::Always);
        assert_eq!(
            bytes,
            agora_tui::device::always_message(
                "mac",
                "ap-2",
                "agent-2",
                "run_command",
                "git status 2",
                "Always allow git status 2",
                "opt-2"
            )
        );
        // Alt+Y signs the single answer of ap-2 too.
        assert_eq!(
            agora_tui::client::answer_message(&a, Answer::Allow)
                .unwrap()
                .0,
            agora_tui::device::approval_message("mac", "ap-2", true)
        );
        // The tree's own keys keep answering the selected agent's request.
        let (a, _) = target_request(&s, "mac/agent-1", &Target::Selected).unwrap();
        assert_eq!(a.approval_id, "mac/ap-1");
    }

    #[test]
    fn a_tabs_request_that_is_gone_answers_nothing_never_another() {
        let s = Snapshot {
            approvals: vec![ask(1)],
            ..Default::default()
        };
        let gone = Target::Tab {
            agent: "mac/agent-2".into(),
            approval: "mac/ap-2".into(),
        };
        assert_eq!(target_request(&s, "mac/agent-2", &gone), None);
        // An id that now belongs to another agent is not this tab's either.
        let moved = Target::Tab {
            agent: "mac/agent-2".into(),
            approval: "mac/ap-1".into(),
        };
        assert_eq!(target_request(&s, "mac/agent-1", &moved), None);
    }

    /// Codex's re-review: with the tree on agent-1 and the keyboard in agent-2's tab, agent-2's start request is never marked read from
    /// the inspector, which shows agent-1's card.
    #[test]
    fn a_tabs_start_request_is_not_read_from_another_agents_card() {
        let start = |n: u32| ApprovalSnapshot {
            approval_id: format!("mac/ap-{n}"),
            agent_id: format!("mac/agent-{n}"),
            action: "agent.start".into(),
            detail: r#"{"kind":"codex","prompt":"fix it"}"#.into(),
            ..Default::default()
        };
        let s = Snapshot {
            approvals: vec![start(1), start(2)],
            ..Default::default()
        };
        let text = |id: &str, _: &[(&str, Arg)]| id.to_string();
        // What the inspector draws beside the tab: the tree's selection (agent-1), whole at this size.
        let shown = agora_tui::split::app_details_in_tab(&s, "mac/agent-1", 80, 40, &text).start;
        assert_eq!(shown, Some(("mac/ap-1".to_string(), true)));
        // The inspector shows agent-1's own request beside the tab, not the tab's (agent-2's).
        let displayed = displayed_request(&s, "mac/agent-1", true).map(|d| d.approval_id.clone());
        assert_eq!(displayed.as_deref(), Some("mac/ap-1"));
        let own_card = displayed.as_deref() == Some("mac/ap-2");
        assert!(!start_read_in_inspector(
            own_card,
            shown.as_ref(),
            "mac/ap-2"
        ));
        // Even a card of the same id is not read when the inspector shows another agent.
        assert!(!start_read_in_inspector(
            false,
            Some(&("mac/ap-2".to_string(), true)),
            "mac/ap-2"
        ));
        // The tab's own agent selected: its own card counts.
        let own = agora_tui::split::app_details_in_tab(&s, "mac/agent-2", 80, 40, &text).start;
        let own_card =
            displayed_request(&s, "mac/agent-2", true).is_some_and(|d| d.approval_id == "mac/ap-2");
        assert!(start_read_in_inspector(own_card, own.as_ref(), "mac/ap-2"));
        // A card shown cut is never read.
        assert!(!start_read_in_inspector(
            true,
            Some(&("mac/ap-2".to_string(), false)),
            "mac/ap-2"
        ));
    }

    /// The conservative path (Codex's third review of #1012): "always" is taken ONLY in the request box. Every entry point (the inspector
    /// showing it whole, the commands with the keyboard in a tab, Alt+A in a tab, Alt+A in a chat that shows it whole) opens the box and
    /// signs nothing itself.
    #[test]
    fn every_always_entry_point_opens_the_box_and_signs_nothing() {
        let a = ask(2);
        let s = two();
        // The inspector shows exactly this request, whole (the tree on agent-2): in view, yet "always" goes to the box.
        let shown = displayed_request(&s, "mac/agent-2", false);
        assert!(request_in_view(&a, shown, true, None));
        assert_eq!(take_answer(Answer::AllowAlways, true), Take::Box);
        // A chat tab that showed all of it: still the box.
        assert!(request_in_view(&a, None, false, Some("mac/ap-2")));
        assert_eq!(take_answer(Answer::AllowAlways, true), Take::Box);
        // Alt+A in a terminal tab whose request is not in the inspector: the box.
        assert!(!request_in_view(
            &a,
            displayed_request(&s, "mac/agent-1", true),
            true,
            None
        ));
        assert_eq!(take_answer(Answer::AllowAlways, false), Take::Box);
        // A single Allow or a Deny of a request in view goes at once; out of view, the box too.
        assert_eq!(take_answer(Answer::Allow, true), Take::Send(Answer::Allow));
        assert_eq!(take_answer(Answer::Deny, true), Take::Send(Answer::Deny));
        assert_eq!(take_answer(Answer::Allow, false), Take::Box);
        assert_eq!(take_answer(Answer::Deny, false), Take::Box);
    }

    /// The commands (Alt+K) with the keyboard in a tab whose agent has no request: the fleet's first request is answered only in the box,
    /// never as if the inspector (which beside a tab shows no other agent's request) had shown it.
    #[test]
    fn a_command_from_a_tab_never_answers_a_request_the_inspector_does_not_show() {
        let s = Snapshot {
            approvals: vec![ask(2)],
            ..Default::default()
        };
        let (a, _) = target_request(&s, "mac/agent-1", &Target::Selected).unwrap();
        assert_eq!(
            a.approval_id, "mac/ap-2",
            "the fleet's first request, for the box"
        );
        assert_eq!(displayed_request(&s, "mac/agent-1", true), None);
        let in_view = request_in_view(&a, displayed_request(&s, "mac/agent-1", true), true, None);
        assert!(!in_view);
        for reply in [Answer::Allow, Answer::Deny, Answer::AllowAlways] {
            assert_eq!(take_answer(reply, in_view), Take::Box, "{reply:?}");
        }
        // In the tree the inspector does show the first request (app_details): a single answer may go from there.
        assert_eq!(
            displayed_request(&s, "mac/agent-1", false).map(|d| d.approval_id.as_str()),
            Some("mac/ap-2")
        );
    }

    /// Only a key in the box signs, and "always" only from a box last drawn whole.
    #[test]
    fn only_a_key_in_a_box_drawn_whole_gives_always() {
        let say = Say(Catalog::english());
        let lines = request_box_lines(&say, &two(), &ask(2));
        let big = Area {
            x: 0,
            y: 0,
            width: 120,
            height: 40,
        };
        let small = Area {
            x: 0,
            y: 0,
            width: 120,
            height: u16::try_from(lines.len()).unwrap(),
        };
        assert_eq!(
            box_reply(Some(Answer::AllowAlways), &lines, Some(big)),
            Ok(Answer::AllowAlways)
        );
        assert_eq!(
            box_reply(Some(Answer::AllowAlways), &lines, Some(small)),
            Err(Some("tui-always-too-small"))
        );
        assert_eq!(
            box_reply(Some(Answer::AllowAlways), &lines, None),
            Err(Some("tui-always-too-small"))
        );
        assert_eq!(
            box_reply(Some(Answer::Allow), &lines, Some(small)),
            Ok(Answer::Allow)
        );
        assert_eq!(box_reply(None, &lines, Some(big)), Err(None));
    }

    /// A key that comes with a change of the window's size is dropped: the box is drawn again first, and only a key after that counts.
    #[test]
    fn a_key_that_comes_with_a_resize_is_dropped_until_the_box_is_drawn_again() {
        use agora_tui::app::Key;
        let lines = vec!["question".to_string(), "A allows it always".to_string()];
        let drawn = Area {
            x: 0,
            y: 0,
            width: 40,
            height: 10,
        };
        let bigger = Area {
            x: 0,
            y: 0,
            width: 120,
            height: 40,
        };
        let key = [AppEvent::Key(Key::Char('A'))];
        assert_eq!(
            box_step(drawn, bigger, &key, &lines, one_answer),
            BoxStep::Redraw
        );
        assert_eq!(
            box_step(bigger, bigger, &key, &lines, one_answer),
            BoxStep::Key(b'A')
        );
        assert_eq!(
            box_step(drawn, drawn, &[], &lines, one_answer),
            BoxStep::Wait
        );
    }

    /// Codex's re-review: Alt+A in a chat tab counts the request as read only when all of its block, the duration warning included,
    /// fits the pane as drawn; otherwise the box opens (and nothing is signed without a key there).
    #[test]
    fn a_chat_shows_an_always_request_only_when_all_of_it_fits() {
        let say = Say(Catalog::english());
        let a = ask(2);
        let (block, whole) = chat_request_block(&say, &a, 40);
        assert!(whole);
        assert!(block.iter().any(|l| l.contains("possibly")), "{block:#?}");
        let need = block.len() + 2;
        // A compact pane that would cut the warning: not in view (the box takes the answer).
        assert!(!chat_shows_request(
            &say,
            &a,
            (40, u16::try_from(need - 1).unwrap()),
            false,
            0
        ));
        // The head line or a note take rows too.
        assert!(!chat_shows_request(
            &say,
            &a,
            (40, u16::try_from(need).unwrap()),
            true,
            0
        ));
        assert!(!chat_shows_request(
            &say,
            &a,
            (40, u16::try_from(need).unwrap()),
            false,
            1
        ));
        // Room for all of it: in view.
        assert!(chat_shows_request(
            &say,
            &a,
            (40, u16::try_from(need).unwrap()),
            false,
            0
        ));
        assert!(chat_shows_request(&say, &a, (40, 60), true, 2));
        // A request longer than its four rows is cut in the pane: never in view there, whatever the height.
        let mut long = ask(2);
        long.detail = "x".repeat(400);
        assert!(!chat_request_block(&say, &long, 40).1);
        assert!(!chat_shows_request(&say, &long, (40, 200), false, 0));
    }

    /// Codex's re-review: in a window too small for the whole request box, its `A` signs nothing.
    #[test]
    fn an_always_answer_is_never_taken_from_a_cut_box() {
        let say = Say(Catalog::english());
        let lines = request_box_lines(&say, &two(), &ask(2));
        let area = |width, height| Area {
            x: 0,
            y: 0,
            width,
            height,
        };
        assert!(always_box_shown(&lines, area(120, 40)));
        assert!(!always_box_shown(
            &lines,
            area(120, u16::try_from(lines.len()).unwrap())
        ));
        assert!(!always_box_shown(&lines, area(30, 12)));
    }

    #[test]
    fn the_request_box_warns_before_an_always_answer_is_given() {
        let say = Say(Catalog::english());
        let s = two();
        let lines = request_box_lines(&say, &s, &ask(2));
        let all = lines.join("\n");
        assert!(
            all.contains("In its own words: “Always allow git status 2”"),
            "{all}"
        );
        assert!(
            all.contains("the agent decides what that covers and how long it keeps the answer, possibly longer than this session"),
            "{all}"
        );
        assert!(all.contains("A allows it always"), "{all}");
        // A request without "always" has neither.
        let mut once = ask(1);
        once.scopes = vec![ApprovalScope::Once as i32];
        let all = request_box_lines(&say, &s, &once).join("\n");
        assert!(
            !all.contains("In its own words") && !all.contains("possibly longer"),
            "{all}"
        );
    }
}

/// #1099: the folder picker's dialogue against a machine that answers as the hub would for a member, a viewer or the owner, and what a
/// new connection does to the role the console holds.
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod folder_role_tests {
    use super::*;
    use agora_i18n::Catalog;
    use agora_protocol::v1::{CheckoutSnapshot, DirectoryEntries, NodeSnapshot, ParamValue};
    use agora_tui::picker::Role;
    use std::collections::HashMap;

    /// Answers each folder as written here (anything else: gone), and keeps what it was asked.
    #[derive(Default)]
    struct Machine {
        answers: HashMap<(String, String), Result<DirectoryEntries, ErrorInfo>>,
        asked: Vec<(String, String)>,
    }

    impl Machine {
        fn folder(mut self, node: &str, path: &str, parent: &str) -> Self {
            let listing = DirectoryEntries {
                path: if path.is_empty() { "/home/me" } else { path }.into(),
                parent: parent.into(),
                ..DirectoryEntries::default()
            };
            self.answers.insert((node.into(), path.into()), Ok(listing));
            self
        }
        fn refused(mut self, node: &str, path: &str) -> Self {
            let mut e = ErrorInfo {
                code: "auth.forbidden".into(),
                ..ErrorInfo::default()
            };
            e.params.insert(
                "action".into(),
                ParamValue {
                    value: Some(Value::Str("DirectoryList".into())),
                },
            );
            self.answers.insert((node.into(), path.into()), Err(e));
            self
        }
        fn paths(&self) -> Vec<&str> {
            self.asked.iter().map(|(_, p)| p.as_str()).collect()
        }
    }

    impl Folders for Machine {
        fn list(&mut self, node: &str, path: &str) -> Result<DirectoryEntries, ErrorInfo> {
            self.asked.push((node.into(), path.into()));
            self.answers
                .get(&(node.into(), path.into()))
                .cloned()
                .unwrap_or_else(|| {
                    Err(ErrorInfo {
                        code: "directory.not_found".into(),
                        ..ErrorInfo::default()
                    })
                })
        }
    }

    /// Types the keys given, then the keyboard is gone (the picker ends as cancelled); keeps every frame drawn.
    struct Keys {
        rx: mpsc::Receiver<u8>,
        frames: Vec<String>,
    }

    impl Asker for Keys {
        fn rx(&self) -> &mpsc::Receiver<u8> {
            &self.rx
        }
        fn show(&mut self, _: &[String]) {}
        fn frame(&mut self, bytes: &[u8]) {
            self.frames
                .push(String::from_utf8_lossy(bytes).into_owned());
        }
    }

    fn keys(typed: &[u8]) -> Keys {
        let (tx, rx) = mpsc::channel();
        for k in typed {
            tx.send(*k).unwrap();
        }
        Keys {
            rx,
            frames: Vec::new(),
        }
    }

    /// Machines `n1` and `n2`, online, with project folders at the paths given.
    fn snapshot(n1: &[&str], n2: &[&str]) -> Snapshot {
        let node = |id: &str| NodeSnapshot {
            node_id: id.into(),
            hostname: id.into(),
            online: true,
            ..NodeSnapshot::default()
        };
        let checkouts = n1
            .iter()
            .map(|p| ("n1", p))
            .chain(n2.iter().map(|p| ("n2", p)))
            .enumerate()
            .map(|(i, (n, p))| CheckoutSnapshot {
                checkout_id: format!("c{i}"),
                node_id: n.into(),
                project_id: format!("p{i}"),
                path: (*p).into(),
            })
            .collect();
        Snapshot {
            nodes: vec![node("n1"), node("n2")],
            checkouts,
            ..Snapshot::default()
        }
    }

    fn browse(
        machine: &mut Machine,
        snapshot: &Snapshot,
        typed: &[u8],
        role: &mut Role,
    ) -> (Browsed, Vec<String>) {
        let say = Say(Catalog::english());
        let mut k = keys(typed);
        let out = browse_folders(&say, machine, snapshot, "", &mut k, false, role);
        (out, k.frames)
    }

    #[test]
    fn with_the_role_not_known_a_project_folder_that_cannot_be_listed_opens_the_home_folder() {
        // The owner's plain list opened at the home folder before #1099; when the project folder is gone it still does, instead of
        // ending the picker with "could not list" before anything was drawn.
        let mut m = Machine::default().folder("n1", "", "/home");
        let mut role = Role::Unknown;
        let (out, frames) = browse(&mut m, &snapshot(&["/srv/gone"], &[]), b"", &mut role);
        assert_eq!(m.paths(), ["/srv/gone", ""]);
        assert_eq!(out, Browsed::Said(String::new()), "drawn, then cancelled");
        assert!(frames.last().unwrap().contains("/home/me"), "{frames:?}");
    }

    #[test]
    fn a_person_refused_every_folder_is_told_before_choosing_among_their_projects() {
        // A viewer with two projects on n1: the first is asked for before the choice is offered, and the reason shows with it. Going
        // round the machines with `m` does not ask again.
        let s = snapshot(&["/srv/a", "/srv/b"], &["/srv/c"]);
        let mut m = Machine::default()
            .refused("n1", "/srv/a")
            .refused("n1", "/srv/b")
            .folder("n2", "/srv/c", "");
        let mut role = Role::Unknown;
        let (out, frames) = browse(&mut m, &s, b"mm", &mut role);
        assert_eq!(out, Browsed::Said(String::new()));
        assert_eq!(m.paths(), ["/srv/a", "/srv/c"]);
        assert_eq!(role, Role::Limited);
        let said = Say(Catalog::english()).text(key!("tui-folder-role"), &[]);
        assert!(frames[0].contains(&said), "{frames:?}");
    }

    #[test]
    fn a_member_made_the_owner_goes_up_from_the_project_folder_when_the_hub_names_a_way_up() {
        // The console still holds "not the owner" (the role changed over a routine refresh); the hub's answer names a way up.
        let s = snapshot(&["/srv/mine"], &[]);
        let mut m = Machine::default()
            .folder("n1", "/srv/mine", "/srv")
            .folder("n1", "/srv", "/");
        let mut role = Role::Limited;
        let (out, _) = browse(&mut m, &s, &[0x7f], &mut role);
        assert_eq!(out, Browsed::Said(String::new()));
        assert_eq!(m.paths(), ["/srv/mine", "/srv"]);
        assert_eq!(role, Role::Unknown);
        // A member's answer names none: ⌫ goes nowhere and the role stands.
        let mut m = Machine::default().folder("n1", "/srv/mine", "");
        let mut role = Role::Limited;
        browse(&mut m, &s, &[0x7f], &mut role);
        assert_eq!(m.paths(), ["/srv/mine"]);
        assert_eq!(role, Role::Limited);
    }

    #[test]
    fn a_lost_link_asks_for_the_role_again_and_a_routine_refresh_does_not() {
        let say = Say(Catalog::english());
        let hub = [("hub", Arg::Str("hub:7443".into()))];
        let mut give = agora_tui::board::Lacking::default();
        let mut pair = agora_tui::pair::Book::default();
        pair.set_owner(agora_tui::pair::Owner::No);
        let mut role = Role::Limited;
        // A refresh: the notice is the console's own.
        let mut notice = "something else".to_string();
        connected_again(&say, &hub, &mut notice, &mut give, &mut pair, &mut role);
        assert_eq!(
            (pair.owner, role, notice.as_str()),
            (agora_tui::pair::Owner::No, Role::Limited, "something else")
        );
        // The link was lost (the hub ends it when the role changes): asked again.
        let mut notice = say.text(key!("tui-lost"), &hub);
        connected_again(&say, &hub, &mut notice, &mut give, &mut pair, &mut role);
        assert_eq!(
            (pair.owner, role, notice.as_str()),
            (agora_tui::pair::Owner::Unknown, Role::Unknown, "")
        );
    }
}

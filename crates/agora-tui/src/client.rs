//! The hub client of the terminal console (ADR-0043): sign in with a one-time code, then follow the hub's snapshot. It speaks the
//! same protocol as the browser (ADR-0027) and trusts only the hub's own CA.
//!
//! Every call blocks the calling thread (a small runtime is built per call), so the terminal loop stays plain code.

use std::sync::Arc;

use agora_i18n::key;
use std::time::Duration;

use crate::device::{approval_message, grant_message, new_grant_id, Device};
use agora_protocol::frame::{kinds, STREAM_CLOSE, STREAM_COMMANDS, STREAM_CONTROL, STREAM_EVENTS};
use agora_protocol::v1::{
    command_ok, command_request, command_response, CommandOk, CommandRequest, CommandResponse,
    ErrorInfo, PersonRole, Snapshot,
};
use agora_protocol::ws::{connect_with_token, Conn};
use agora_protocol::Frame;
use prost::Message;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;

/// Where the hub is and which CA signed its certificate.
#[derive(Clone)]
pub struct Hub {
    /// `host:port`.
    pub addr: String,
    pub ca_pem: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ClientError {
    /// The hub could not be reached or spoke something else (connection, TLS, framing).
    Unreachable,
    /// The code or the token was not accepted.
    Refused,
    /// Too many wrong codes: the hub asks to wait.
    TooMany,
}

fn runtime() -> Result<tokio::runtime::Runtime, ClientError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| ClientError::Unreachable)
}

/// The names the hub's certificate may carry for this address: the host itself, and for an IP number also `localhost` (a hub
/// started with no `--name` names only that). Each is tried in turn against the pinned CA.
fn server_names(addr: &str) -> Vec<ServerName<'static>> {
    let host = addr
        .rsplit_once(':')
        .map_or(addr, |(h, _)| h)
        .trim_matches(['[', ']']);
    let mut names = Vec::new();
    if let Ok(name) = ServerName::try_from(host.to_string()) {
        names.push(name);
    }
    if host.parse::<std::net::IpAddr>().is_ok() {
        names.extend(ServerName::try_from("localhost".to_string()).ok());
    }
    names
}

async fn tls(hub: &Hub) -> Result<TlsStream<TcpStream>, ClientError> {
    let mut roots = rustls::RootCertStore::empty();
    for c in CertificateDer::pem_slice_iter(hub.ca_pem.as_bytes()) {
        roots
            .add(c.map_err(|_| ClientError::Unreachable)?)
            .map_err(|_| ClientError::Unreachable)?;
    }
    let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|_| ClientError::Unreachable)?
    .with_root_certificates(roots)
    .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(cfg));
    for name in server_names(&hub.addr) {
        let Ok(Ok(tcp)) = tokio::time::timeout(
            Duration::from_secs(10),
            agora_protocol::dial::connect(&hub.addr),
        )
        .await
        else {
            return Err(ClientError::Unreachable);
        };
        if let Ok(Ok(stream)) =
            tokio::time::timeout(Duration::from_secs(10), connector.connect(name, tcp)).await
        {
            return Ok(stream);
        }
    }
    Err(ClientError::Unreachable)
}

/// Trades a one-time sign-in code (`agora hub login`) for a session token.
pub fn sign_in(hub: &Hub, code: &str) -> Result<String, ClientError> {
    runtime()?.block_on(async {
        let mut s = tls(hub).await?;
        let body = serde_json::json!({ "code": code }).to_string();
        let host = hub.addr.clone();
        let req = format!(
            "POST /api/session HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        s.write_all(req.as_bytes()).await.map_err(|_| ClientError::Unreachable)?;
        let mut raw = Vec::new();
        // The hub closes after answering; a missing close_notify after the answer is not an error here.
        let _ = tokio::time::timeout(Duration::from_secs(10), s.read_to_end(&mut raw)).await;
        let text = String::from_utf8_lossy(&raw);
        let status = text.split_whitespace().nth(1).unwrap_or("");
        match status {
            "200" => {
                let json = text.split_once("\r\n\r\n").map_or("", |(_, b)| b);
                serde_json::from_str::<serde_json::Value>(json)
                    .ok()
                    .and_then(|v| v.get("token").and_then(|t| t.as_str()).map(str::to_string))
                    .ok_or(ClientError::Unreachable)
            }
            "401" => Err(ClientError::Refused),
            "429" => Err(ClientError::TooMany),
            _ => Err(ClientError::Unreachable),
        }
    })
}

/// A live connection: one snapshot, then the positions of the events that follow.
pub struct Session {
    rt: tokio::runtime::Runtime,
    conn: Conn<TlsStream<TcpStream>>,
    requests: u64,
    /// Frames of terminal streams that arrived while something else was waiting.
    term_frames: std::collections::VecDeque<Frame>,
    next_stream: u32,
    /// An event arrived while a command was waiting for its answer.
    changed: bool,
    /// A request for the user's answer arrived since [`Session::take_asked`] was last called (a full-screen view says so).
    asked: bool,
    /// A snapshot that arrived while something else was waiting (the hub's first one comes right after subscribing, often while
    /// the device registration waits for its answer): `next` hands it out first, so it is never lost.
    snapshot: Option<Box<Snapshot>>,
    /// Typing sent without waiting for the answer, and the newest of it a machine refused (per terminal) because it wants a grant first.
    typing: Typing,
    /// What the events after the snapshot said that the snapshot has no field for (ADR-0090 config options).
    seen: Seen,
    /// History (kind 4, ADR-0100) per terminal stream, held for the screen it comes before: [`Session::terminal_history`].
    history: HeldHistory,
    /// ADR-0106: the role of the person this connection signed in as, from the hub's first frame; unspecified from an older hub.
    person_role: PersonRole,
}

/// How long [`Session::connect`] waits for the hub's first frame: the `Welcome` naming the person's role (ADR-0106), or an older hub's
/// snapshot. Both come at once; a hub that sends neither in time leaves the role unspecified, and its frames are read as they come.
const FIRST_FRAME_WITHIN: Duration = Duration::from_secs(5);

/// ADR-0106: the role a `Welcome` names, when `f` is one.
fn welcome_role(f: &Frame) -> Option<PersonRole> {
    if (f.stream_id, f.kind) != (STREAM_CONTROL, kinds::WELCOME) {
        return None;
    }
    agora_protocol::v1::Welcome::decode(f.payload.as_slice())
        .ok()
        .map(|w| w.person_role())
}

/// The agents' config options (ADR-0090: the snapshot has no field for them; the hub replays each running agent's last
/// `AgentSessionConfig` right after the snapshot, at its position, and live changes follow the same way), by hub-wide agent id, and the
/// snapshot's position: an event at or before it says nothing the snapshot did not.
#[derive(Debug, Default)]
pub struct Seen {
    pub configs: Configs,
    snapshot_pos: Option<u64>,
    /// A report changed an agent's options since [`Session::take_configs_changed`] (a view that shows them draws again).
    pub configs_changed: bool,
}

/// What the events after the snapshot said about the agents' sessions, by hub-wide agent id: their config options (ADR-0090) and, from
/// a live `AgentSessionOptions` newer than the snapshot, their modes and current mode (ADR-0058), which then stand in for the
/// snapshot's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Configs {
    pub options: std::collections::HashMap<String, Vec<agora_protocol::v1::SessionConfigOption>>,
    pub modes: std::collections::HashMap<String, (Vec<agora_protocol::v1::SessionOption>, String)>,
}

/// At most this many agents' options are kept, each at most [`MAX_OPTIONS`] (a hub cannot make the client's memory grow).
const MAX_CONFIG_AGENTS: usize = 4096;
const MAX_OPTIONS: usize = 64;

impl Seen {
    /// Reads one positioned event: keeps an agent's config options (each report replaces the last), and says whether the event is
    /// newer than the snapshot (a fresh snapshot is wanted for it).
    fn note(&mut self, f: &Frame) -> bool {
        let Ok(p) = agora_protocol::v1::PositionedEvent::decode(f.payload.as_slice()) else {
            return true;
        };
        let newer = self.snapshot_pos.is_none_or(|at| p.pos > at);
        match p.event.as_ref().map(|e| (&e.producer_id, &e.body)) {
            Some((producer, Some(agora_protocol::v1::event::Body::AgentSessionConfig(c)))) => {
                let id = format!("{producer}/{}", c.agent_id);
                let options = &mut self.configs.options;
                let before = options.get(&id).cloned();
                if c.options.is_empty() {
                    options.remove(&id);
                } else if options.len() < MAX_CONFIG_AGENTS || options.contains_key(&id) {
                    options.insert(
                        id.clone(),
                        c.options.iter().take(MAX_OPTIONS).cloned().collect(),
                    );
                }
                if options.get(&id) != before.as_ref() {
                    self.configs_changed = true;
                }
            }
            // A mode change after the snapshot (an older one is in the snapshot already).
            Some((producer, Some(agora_protocol::v1::event::Body::AgentSessionOptions(o))))
                if newer =>
            {
                let id = format!("{producer}/{}", o.agent_id);
                let modes = &mut self.configs.modes;
                if modes.len() < MAX_CONFIG_AGENTS || modes.contains_key(&id) {
                    let now = (
                        o.modes.iter().take(MAX_OPTIONS).cloned().collect(),
                        o.current_mode.clone(),
                    );
                    if modes.get(&id) != Some(&now) {
                        modes.insert(id, now);
                        self.configs_changed = true;
                    }
                }
            }
            _ => {}
        }
        newer
    }
}

/// Typing a machine refused because it wants a grant first (ADR-0041): the terminal it was for, and its request's number (typing sent
/// later has a larger one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    pub node: String,
    pub terminal_id: String,
    pub request: u64,
}

impl Refused {
    /// Whether this is typing into `terminal_id` on `node`.
    pub fn is_for(&self, node: &str, terminal_id: &str) -> bool {
        self.node == node && self.terminal_id == terminal_id
    }
}

/// Typing waiting for its answer, by request (the hub answers under the client's request id), so a refusal names its terminal.
#[derive(Default)]
struct Typing {
    /// By request: the terminal, and whether the user typed it (a focus report or pointer motion the console sends by itself is not
    /// typing: its refusal asks nothing).
    sent: std::collections::VecDeque<(u64, String, String, bool)>,
    /// The newest refusal of each terminal not read yet (a console picks the one in front; two may come in one turn).
    refused: Vec<Refused>,
    /// The newest typing of each terminal a machine took, not read yet: (node, terminal, request).
    taken: Vec<(String, String, u64)>,
}

impl Typing {
    /// Typing waiting for its answer is kept until the answer is read: every key of a long paste is one request, and the hub answers
    /// past its own limit of 256 waiting (docs/specs/protocol.md) at once, so this many are only reached when answers stop coming.
    /// Beyond it the oldest is forgotten, and its refusal, should it still come, asks nothing (fails closed).
    const KEPT: usize = 4096;
    /// Terminals with a refusal not read yet; more is not a console a user types into.
    const REFUSED: usize = 64;

    fn sent(&mut self, request: u64, node: &str, terminal_id: &str) {
        self.track(request, node, terminal_id, true);
    }

    fn track(&mut self, request: u64, node: &str, terminal_id: &str, typed: bool) {
        if self.sent.len() >= Self::KEPT {
            self.sent.pop_front();
        }
        self.sent
            .push_back((request, node.to_string(), terminal_id.to_string(), typed));
    }

    /// A frame nobody waits for: when it is a command answer, the typing it answers is forgotten and a refusal for a missing grant
    /// kept. Every reader of the link calls this, so no refusal is lost to whichever reader got it. Returns whether it was one.
    fn answer(&mut self, f: &Frame) -> bool {
        if (f.stream_id, f.kind) != (STREAM_COMMANDS, 1) {
            return false;
        }
        if let Ok(r) = CommandResponse::decode(f.payload.as_slice()) {
            self.reply(&r);
        }
        true
    }

    fn reply(&mut self, r: &CommandResponse) {
        let Some(n) = r
            .request_id
            .strip_prefix('t')
            .and_then(|n| n.parse::<u64>().ok())
        else {
            return;
        };
        let Some((request, node, terminal_id, typed)) = self
            .sent
            .iter()
            .position(|(id, ..)| *id == n)
            .and_then(|i| self.sent.remove(i))
        else {
            return;
        };
        if typed && matches!(r.result, Some(command_response::Result::Ok(_)) | None) {
            let newer = self
                .taken
                .iter()
                .any(|(n, t, req)| *n == node && *t == terminal_id && *req > request);
            if !newer {
                self.taken
                    .retain(|(n, t, _)| !(*n == node && *t == terminal_id));
                if self.taken.len() >= Self::REFUSED {
                    self.taken.remove(0);
                }
                self.taken.push((node, terminal_id, request));
            }
            return;
        }
        if typed && refused_for_grant(r) {
            // Answers may come out of order: a terminal keeps its newest typing's refusal, by request number.
            if self
                .refused
                .iter()
                .any(|old| old.is_for(&node, &terminal_id) && old.request > request)
            {
                return;
            }
            self.refused.retain(|old| !old.is_for(&node, &terminal_id));
            if self.refused.len() >= Self::REFUSED {
                self.refused.remove(0);
            }
            self.refused.push(Refused {
                node,
                terminal_id,
                request,
            });
        }
    }
}

/// An agent a machine offers (`agent.kinds`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kind {
    pub kind: String,
    /// The name to show (the kind itself when the manifest gives none).
    pub name: String,
    /// Whether the machine finds its program.
    pub installed: bool,
    /// ADR-0069: the program the machine looks for in its `PATH` (a bare name such as `wstack`); empty from an older machine.
    pub program: String,
}

/// ADR-0092: the prefix of every kind a machine converted from the ACP registry (`agora_adapters::registry_convert`); such a kind is
/// untested in Agora. A hand-made kind of the same name or id has already replaced it on the machine.
pub const REGISTRY_PREFIX: &str = "registry-";

/// Whether `kind` is one of the ACP registry's (ADR-0092), by its name as the window tells it (`registry-<id>`).
pub fn is_registry_kind(kind: &str) -> bool {
    kind.starts_with(REGISTRY_PREFIX)
}

/// A machine's kinds as the start pickers list them (#1134), each group in the machine's own order, as the window's Start does.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StartKinds {
    /// The installed hand-made kinds: listed first.
    pub installed: Vec<Kind>,
    /// The installed ACP registry kinds: listed after them, under "Untested".
    pub untested: Vec<Kind>,
    /// The hand-made kinds whose program the machine cannot find. A registry kind that is not installed is left out: there are
    /// dozens, and the window's Settings → Agents lists them under "Add an agent".
    pub missing: Vec<Kind>,
}

/// Sorts `all` into [`StartKinds`]; nothing is chosen for the person.
pub fn start_kinds(all: Vec<Kind>) -> StartKinds {
    let mut out = StartKinds::default();
    for k in all {
        match (k.installed, is_registry_kind(&k.kind)) {
            (true, false) => out.installed.push(k),
            (true, true) => out.untested.push(k),
            (false, false) => out.missing.push(k),
            (false, true) => {}
        }
    }
    out
}

/// ADR-0065: what the hub tells an owner's window about pairing.
#[derive(Debug, Clone, PartialEq)]
pub enum PairNote {
    /// A machine (or browser) is waiting for the owner's answer.
    Requested(agora_protocol::v1::PairRequested),
    /// A request was decided, expired or ended.
    Decided(agora_protocol::v1::PairDecided),
}

/// What the hub sent next.
pub enum Update {
    Snapshot(Box<Snapshot>),
    /// An event happened somewhere; the console asks for a fresh snapshot.
    Changed,
    /// Nothing within the wait.
    Quiet,
    Closed,
}

impl Session {
    /// Connects and subscribes; the first update is the snapshot.
    pub fn connect(hub: &Hub, token: &str) -> Result<Session, ClientError> {
        let rt = runtime()?;
        let conn = rt.block_on(async {
            let stream = tls(hub).await?;
            let host = hub
                .addr
                .rsplit_once(':')
                .map_or(hub.addr.as_str(), |(h, _)| h);
            let mut conn = connect_with_token(stream, &format!("wss://{host}/"), token)
                .await
                .map_err(|_| ClientError::Refused)?;
            let sub = agora_protocol::v1::Subscribe { after_pos: None }.encode_to_vec();
            conn.send(&Frame {
                stream_id: STREAM_EVENTS,
                kind: kinds::SUBSCRIBE,
                seq: None,
                payload: sub,
            })
            .await
            .map_err(|_| ClientError::Unreachable)?;
            // ADR-0106: a hub that tells the role sends its `Welcome` before anything else; an older hub sends the snapshot first
            // (this subscription asks for one), which is kept for `next`. A closed connection is found by `next`, as before.
            let first = match tokio::time::timeout(FIRST_FRAME_WITHIN, conn.recv()).await {
                Ok(Ok(Some(f))) => Some(f),
                _ => None,
            };
            Ok::<_, ClientError>((conn, first))
        })?;
        let (conn, first) = conn;
        let mut session = Session {
            rt,
            conn,
            requests: 0,
            term_frames: std::collections::VecDeque::new(),
            next_stream: agora_protocol::frame::FIRST_DYNAMIC_STREAM,
            changed: false,
            asked: false,
            snapshot: None,
            typing: Typing::default(),
            seen: Seen::default(),
            history: HeldHistory::default(),
            person_role: PersonRole::Unspecified,
        };
        if let Some(f) = first {
            match welcome_role(&f) {
                Some(role) => session.person_role = role,
                None => set_aside(
                    f,
                    &mut session.changed,
                    &mut session.asked,
                    &mut session.snapshot,
                    &mut session.term_frames,
                    &mut session.seen,
                ),
            }
        }
        Ok(session)
    }

    /// ADR-0106: the role of the person this connection signed in as, as the hub said when it opened (the hub ends a connection whose
    /// role or grants changed). `Unspecified` from a hub that predates it: the caller learns the role as it did before.
    pub fn person_role(&self) -> PersonRole {
        self.person_role
    }

    /// Waits up to `wait` for the hub to say something.
    pub fn next(&mut self, wait: Duration) -> Update {
        // A kept snapshot first. `changed` is left as it is: `set_aside` cleared it when the snapshot came, so if it is set now an
        // event arrived after the snapshot, and the next call reports it.
        if let Some(s) = self.snapshot.take() {
            return Update::Snapshot(s);
        }
        if std::mem::take(&mut self.changed) {
            return Update::Changed;
        }
        let conn = &mut self.conn;
        let queue = &mut self.term_frames;
        let asked = &mut self.asked;
        let typing = &mut self.typing;
        let seen = &mut self.seen;
        let person_role = &mut self.person_role;
        self.rt.block_on(async {
            match tokio::time::timeout(wait, conn.recv()).await {
                Err(_) => Update::Quiet,
                // ADR-0106: a `Welcome` that came after `connect` stopped waiting for it.
                Ok(Ok(Some(f))) if (f.stream_id, f.kind) == (STREAM_CONTROL, kinds::WELCOME) => {
                    if let Some(role) = welcome_role(&f) {
                        *person_role = role;
                    }
                    Update::Quiet
                }
                // The answer to something typed without waiting: only a missing grant matters.
                Ok(Ok(Some(f))) if typing.answer(&f) => Update::Quiet,
                // A terminal being watched alongside: kept for `terminal_next`, never dropped here.
                Ok(Ok(Some(f))) if f.stream_id >= agora_protocol::frame::FIRST_DYNAMIC_STREAM => {
                    queue.push_back(f);
                    Update::Quiet
                }
                Ok(Ok(Some(f))) if f.stream_id == STREAM_EVENTS && f.kind == kinds::SNAPSHOT => {
                    match Snapshot::decode(f.payload.as_slice()) {
                        Ok(s) => {
                            seen.snapshot_pos = Some(s.pos);
                            Update::Snapshot(Box::new(s))
                        }
                        Err(_) => Update::Closed,
                    }
                }
                Ok(Ok(Some(f)))
                    if f.stream_id == STREAM_EVENTS && f.kind == kinds::POSITIONED_EVENT =>
                {
                    if asks_the_user(&f) {
                        *asked = true;
                    }
                    // The config options the hub replays right after the snapshot are kept, and want no fresh snapshot.
                    if seen.note(&f) {
                        Update::Changed
                    } else {
                        Update::Quiet
                    }
                }
                Ok(Ok(Some(f))) if f.stream_id == STREAM_EVENTS && is_pair_frame(&f) => {
                    keep_pair_frame(f, queue);
                    Update::Quiet
                }
                Ok(Ok(Some(_))) => Update::Quiet,
                Ok(_) => Update::Closed,
            }
        })
    }

    /// ADR-0065: the pairing notices an owner's window got since the last call, in order (cards waiting and decisions). Empty for
    /// anyone but an owner. The hub sends a card once, so a caller that wants cards reads this after every [`Session::next`] and
    /// [`Session::command_ok`].
    pub fn take_pair_notes(&mut self) -> Vec<PairNote> {
        let mut notes = Vec::new();
        let mut rest = std::collections::VecDeque::new();
        for f in self.term_frames.drain(..) {
            if f.stream_id == STREAM_EVENTS && f.kind == kinds::PAIR_REQUESTED {
                if let Ok(c) = agora_protocol::v1::PairRequested::decode(f.payload.as_slice()) {
                    notes.push(PairNote::Requested(c));
                }
            } else if f.stream_id == STREAM_EVENTS && f.kind == kinds::PAIR_DECIDED {
                if let Ok(d) = agora_protocol::v1::PairDecided::decode(f.payload.as_slice()) {
                    notes.push(PairNote::Decided(d));
                }
            } else {
                rest.push_back(f);
            }
        }
        self.term_frames = rest;
        notes
    }

    /// ADR-0090: each agent's config options as its machine last reported them in this session, by hub-wide agent id.
    pub fn configs(&self) -> &Configs {
        &self.seen.configs
    }

    /// Whether a report changed an agent's config options since the last call (then cleared).
    pub fn take_configs_changed(&mut self) -> bool {
        std::mem::take(&mut self.seen.configs_changed)
    }

    /// Whether a request for the user's answer arrived since the last call (then cleared).
    pub fn take_asked(&mut self) -> bool {
        std::mem::take(&mut self.asked)
    }

    /// Runs a command on a machine through the hub (ADR-0029) and waits for its answer. `Err` carries the error code the machine or
    /// the hub answered with (`approval.signature_invalid`, `node.offline`, ...), or `client.unreachable` when the link failed.
    pub fn command(
        &mut self,
        node: &str,
        command: command_request::Command,
        wait: Duration,
    ) -> Result<(), String> {
        self.command_ok(node, command, wait).map(|_| ())
    }

    /// Like [`Session::command`], returning what the machine answered with.
    pub fn command_ok(
        &mut self,
        node: &str,
        command: command_request::Command,
        wait: Duration,
    ) -> Result<CommandOk, String> {
        self.command_info(node, command, wait).map_err(|e| e.code)
    }

    /// Keeps the peer's error parameters for actions that render the localized reason.
    pub fn command_info(
        &mut self,
        node: &str,
        command: command_request::Command,
        wait: Duration,
    ) -> Result<CommandOk, ErrorInfo> {
        self.requests += 1;
        let id = format!("t{}", self.requests);
        let req = CommandRequest {
            request_id: id.clone(),
            idempotency_key: String::new(),
            target_node: node.to_string(),
            command: Some(command),
        };
        let conn = &mut self.conn;
        let changed = &mut self.changed;
        let kept = &mut self.snapshot;
        let term_frames = &mut self.term_frames;
        let typing = &mut self.typing;
        let asked = &mut self.asked;
        let seen = &mut self.seen;
        self.rt.block_on(async {
            conn.send(&Frame {
                stream_id: STREAM_COMMANDS,
                kind: 0,
                seq: None,
                payload: req.encode_to_vec(),
            })
            .await
            .map_err(|_| command_error("client.unreachable"))?;
            let end = tokio::time::Instant::now() + wait;
            loop {
                let left = end.saturating_duration_since(tokio::time::Instant::now());
                let f = match tokio::time::timeout(left, conn.recv()).await {
                    Ok(Ok(Some(f))) => f,
                    Err(_) => return Err(command_error("node.timeout")),
                    Ok(_) => return Err(command_error("client.unreachable")),
                };
                if (f.stream_id, f.kind) == (STREAM_COMMANDS, 1) {
                    let Ok(r) = CommandResponse::decode(f.payload.as_slice()) else {
                        continue;
                    };
                    if r.request_id != id {
                        typing.reply(&r);
                        continue;
                    }
                    return match r.result {
                        Some(command_response::Result::Err(e)) => Err(e),
                        Some(command_response::Result::Ok(ok)) => Ok(ok),
                        None => Ok(CommandOk::default()),
                    };
                }
                set_aside(f, changed, asked, kept, term_frames, seen);
            }
        })
    }

    /// Answers the first approval waiting in `snapshot` with this device's signature (ADR-0028). `Ok(true)` when there was one and the
    /// machine took the answer, `Ok(false)` when nothing was waiting, `Err` with the error code otherwise
    /// (`approval.signature_invalid` means the machine does not trust this device yet).
    pub fn resolve_first_approval(
        &mut self,
        snapshot: &Snapshot,
        device: &Device,
        approved: bool,
    ) -> Result<bool, String> {
        self.resolve_approval(snapshot, device, approved, None)
    }

    /// Like [`Session::resolve_first_approval`], answering `agent`'s request when it has one (the agent the console points at), else
    /// the first.
    pub fn resolve_approval(
        &mut self,
        snapshot: &Snapshot,
        device: &Device,
        approved: bool,
        agent: Option<&str>,
    ) -> Result<bool, String> {
        let answer = if approved {
            Answer::Allow
        } else {
            Answer::Deny
        };
        self.resolve_answer(snapshot, device, answer, agent)
    }

    /// [`Session::resolve_approval`] with the answer the key named; ADR-0101: [`Answer::AllowAlways`] only for a request that offers it,
    /// and a plain Allow never for one that offers only that ([`answer_message`]). Refused here, nothing sent, with
    /// `approval.scope_unavailable` otherwise.
    pub fn resolve_answer(
        &mut self,
        snapshot: &Snapshot,
        device: &Device,
        answer: Answer,
        agent: Option<&str>,
    ) -> Result<bool, String> {
        let Some(a) = answered_approval(snapshot, agent) else {
            return Ok(false);
        };
        self.resolve_request(a, device, answer).map(|()| true)
    }

    /// Answers exactly the request `a` (an agent's tab answers its own request, never another one): signed and sent as
    /// [`answer_message`] says, or refused before anything is sent.
    pub fn resolve_request(
        &mut self,
        a: &agora_protocol::v1::ApprovalSnapshot,
        device: &Device,
        answer: Answer,
    ) -> Result<(), String> {
        // Hub-wide ids are `<node>/<local id>`; the machine knows the local one.
        let (node, local) = a
            .approval_id
            .split_once('/')
            .unwrap_or(("", a.approval_id.as_str()));
        let (message, scope) = answer_message(a, answer).map_err(str::to_string)?;
        let signature = device
            .sign(&message)
            .ok_or_else(|| "client.sign".to_string())?;
        let cmd = command_request::Command::ApprovalResolve(agora_protocol::v1::ApprovalResolve {
            approval_id: local.to_string(),
            approved: answer != Answer::Deny,
            signer_device_id: device.id.clone(),
            signature,
            helper_grant: 0,
            helpers_may_start: false,
            // ADR-0101: UNSPECIFIED for a single answer (signed over the v1 bytes, as before), ALWAYS over its own bytes.
            scope: scope as i32,
        });
        self.command(node, cmd, Duration::from_secs(15))
    }

    /// Opens a terminal of a machine (ADR-0029): the hub relays its screen and output on a stream of ours. `terminal_id` is the
    /// machine's own id. Returns the stream; read it with [`Session::terminal_next`].
    pub fn terminal_open(&mut self, node: &str, terminal_id: &str) -> Result<u32, String> {
        self.terminal_open_with_history(node, terminal_id, 0)
    }

    /// [`Session::terminal_open`] for a view that wants the newest `lines` lines of history before the first screen (ADR-0100
    /// decision 2; the machine sends at most what it keeps, an older one none). The history is held for that screen: take it with
    /// [`Session::terminal_history`] when [`TermEvent::Screen`] arrives.
    pub fn terminal_open_with_history(
        &mut self,
        node: &str,
        terminal_id: &str,
        lines: u32,
    ) -> Result<u32, String> {
        self.open_stream(
            node,
            command_request::Command::TerminalSubscribe(agora_protocol::v1::TerminalSubscribe {
                terminal_id: terminal_id.to_string(),
                format: String::new(),
                history_lines: lines,
            }),
        )
    }

    /// The history (kind 4) that came on `stream` before the [`TermEvent::Screen`] just returned, taken: empty when none came (none
    /// was asked for, the machine keeps none, or it is older).
    pub fn terminal_history(&mut self, stream: u32) -> Vec<u8> {
        self.history.take(stream)
    }

    /// Opens the conversation of an agent that has no terminal (ADR-0038). `agent_id` is the machine's own id. Read it with
    /// [`Session::chat_next`].
    pub fn chat_open(&mut self, node: &str, agent_id: &str) -> Result<u32, String> {
        self.open_stream(
            node,
            command_request::Command::ChatSubscribe(agora_protocol::v1::ChatSubscribe {
                agent_id: agent_id.to_string(),
            }),
        )
    }

    fn open_stream(
        &mut self,
        node: &str,
        command: command_request::Command,
    ) -> Result<u32, String> {
        let stream = self.next_stream;
        self.next_stream += 1;
        self.requests += 1;
        let id = format!("t{}", self.requests);
        let req = CommandRequest {
            request_id: id.clone(),
            idempotency_key: String::new(),
            target_node: node.to_string(),
            command: Some(command),
        };
        let conn = &mut self.conn;
        let changed = &mut self.changed;
        let kept = &mut self.snapshot;
        let term_frames = &mut self.term_frames;
        let asked = &mut self.asked;
        let seen = &mut self.seen;
        let typing = &mut self.typing;
        self.rt.block_on(async {
            conn.send(&Frame {
                stream_id: stream,
                kind: 0,
                seq: None,
                payload: req.encode_to_vec(),
            })
            .await
            .map_err(|_| "client.unreachable".to_string())?;
            let end = tokio::time::Instant::now() + Duration::from_secs(15);
            loop {
                let left = end.saturating_duration_since(tokio::time::Instant::now());
                let f = match tokio::time::timeout(left, conn.recv()).await {
                    Ok(Ok(Some(f))) => f,
                    Err(_) => return Err("node.timeout".to_string()),
                    Ok(_) => return Err("client.unreachable".to_string()),
                };
                if (f.stream_id, f.kind) == (STREAM_COMMANDS, 1) {
                    let Ok(r) = CommandResponse::decode(f.payload.as_slice()) else {
                        continue;
                    };
                    if r.request_id == id {
                        return match r.result {
                            Some(command_response::Result::Err(e)) => Err(e.code),
                            _ => Ok(stream),
                        };
                    }
                    typing.reply(&r);
                } else {
                    set_aside(f, changed, asked, kept, term_frames, seen);
                }
            }
        })
    }

    /// The next thing on a terminal stream, waiting up to `wait`. History frames (kind 4) are held for the screen they come before
    /// ([`Session::terminal_history`]) and never returned; the frame after them is.
    pub fn terminal_next(&mut self, stream: u32, wait: Duration) -> TermEvent {
        let mut wait = wait;
        loop {
            let f = match self.terminal_frame(stream, wait) {
                Ok(f) => f,
                Err(e) => {
                    if e == TermEvent::Lost {
                        // Every subscription on the connection is over: history whose screen never came is dropped.
                        self.history.clear_all();
                    }
                    return e;
                }
            };
            if let Some(e) = self.history.route(&f) {
                return e;
            }
            // The screen follows its history directly: take what is already here without waiting again.
            wait = Duration::ZERO;
        }
    }

    /// The next frame of `stream`, or [`TermEvent::Quiet`] (nothing in time) or [`TermEvent::Lost`].
    fn terminal_frame(&mut self, stream: u32, wait: Duration) -> Result<Frame, TermEvent> {
        if let Some(pos) = self.term_frames.iter().position(|f| f.stream_id == stream) {
            if let Some(f) = self.term_frames.remove(pos) {
                return Ok(f);
            }
        }
        let conn = &mut self.conn;
        let changed = &mut self.changed;
        let kept = &mut self.snapshot;
        let queue = &mut self.term_frames;
        let typing = &mut self.typing;
        let asked = &mut self.asked;
        let seen = &mut self.seen;
        self.rt.block_on(async {
            match tokio::time::timeout(wait, conn.recv()).await {
                Err(_) => Err(TermEvent::Quiet),
                Ok(Ok(Some(f))) if f.stream_id == stream => Ok(f),
                Ok(Ok(Some(f))) => {
                    // The answer to something typed without waiting: only a missing grant matters.
                    if !typing.answer(&f) {
                        set_aside(f, changed, asked, kept, queue, seen);
                    }
                    Err(TermEvent::Quiet)
                }
                Ok(_) => Err(TermEvent::Lost),
            }
        })
    }

    /// The next thing on a chat stream, waiting up to `wait`.
    pub fn chat_next(&mut self, stream: u32, wait: Duration) -> ChatEvent {
        // Chat frames share the machinery of terminal streams: kind 10 is a snapshot, kind 11 one entry.
        let take = |f: &Frame| -> Option<ChatEvent> {
            if f.kind == STREAM_CLOSE {
                return Some(ChatEvent::Ended);
            }
            match f.kind {
                10 => agora_protocol::v1::ChatSnapshot::decode(f.payload.as_slice())
                    .ok()
                    .map(|c| ChatEvent::Lines(c.entries, true)),
                11 => agora_protocol::v1::ChatEntry::decode(f.payload.as_slice())
                    .ok()
                    .map(|e| ChatEvent::Lines(vec![e], false)),
                _ => None,
            }
        };
        if let Some(pos) = self.term_frames.iter().position(|f| f.stream_id == stream) {
            if let Some(f) = self.term_frames.remove(pos) {
                return take(&f).unwrap_or(ChatEvent::Quiet);
            }
        }
        let conn = &mut self.conn;
        let changed = &mut self.changed;
        let kept = &mut self.snapshot;
        let queue = &mut self.term_frames;
        let asked = &mut self.asked;
        let seen = &mut self.seen;
        let typing = &mut self.typing;
        self.rt.block_on(async {
            match tokio::time::timeout(wait, conn.recv()).await {
                Err(_) => ChatEvent::Quiet,
                Ok(Ok(Some(f))) if f.stream_id == stream => take(&f).unwrap_or(ChatEvent::Quiet),
                Ok(Ok(Some(f))) => {
                    if !typing.answer(&f) {
                        set_aside(f, changed, asked, kept, queue, seen);
                    }
                    ChatEvent::Quiet
                }
                Ok(_) => ChatEvent::Lost,
            }
        })
    }

    /// Sends a request to an agent (a routed `agent.prompt`) and waits for the machine to take it.
    pub fn prompt(&mut self, node: &str, agent_id: &str, text: &str) -> Result<(), String> {
        self.command(
            node,
            command_request::Command::AgentPrompt(agora_protocol::v1::AgentPrompt {
                agent_id: agent_id.to_string(),
                text: text.to_string(),
            }),
            Duration::from_secs(15),
        )
    }

    /// ADR-0058: switches a chat agent's mode (`model == false`) or model to one of the ids it offered.
    pub fn set_option(
        &mut self,
        node: &str,
        agent_id: &str,
        model: bool,
        id: &str,
    ) -> Result<(), String> {
        let command = if model {
            command_request::Command::AgentSetModel(agora_protocol::v1::AgentSetModel {
                agent_id: agent_id.to_string(),
                model: id.to_string(),
            })
        } else {
            command_request::Command::AgentSetMode(agora_protocol::v1::AgentSetMode {
                agent_id: agent_id.to_string(),
                mode: id.to_string(),
            })
        };
        self.command(node, command, Duration::from_secs(15))
    }

    /// What the agents used so far, as the hub estimates it: total in micro-USD (the hub answers this itself, ADR-0033). The hub
    /// answers it to the owner only, so a member's or viewer's session (ADR-0106) does not ask: `auth.forbidden` without a request,
    /// where the hub would refuse it and write the refusal down. An older hub's session asks as before.
    pub fn usage_total(&mut self) -> Result<u64, String> {
        if matches!(self.person_role, PersonRole::Member | PersonRole::Viewer) {
            return Err(agora_protocol::ErrorCode::AuthForbidden
                .as_str()
                .to_string());
        }
        let ok = self.command_ok(
            "",
            command_request::Command::UsageReport(agora_protocol::v1::UsageReport {}),
            Duration::from_secs(10),
        )?;
        match ok.result {
            Some(command_ok::Result::UsageReport(r)) => Ok(r.total_micro_usd),
            _ => Ok(0),
        }
    }

    /// The agents a machine can start: `(kind, display name)` of the installed ones.
    /// The folders inside `path` on `node` (ADR-0053 `directory.list`; empty `path` is the home folder there), with this device's
    /// signature for exactly that folder. Errors keep their parameters (#1044).
    pub fn list_folder(
        &mut self,
        node: &str,
        path: &str,
        authorization: agora_protocol::v1::DirectoryAuthorization,
    ) -> Result<agora_protocol::v1::DirectoryEntries, ErrorInfo> {
        let ok = self.command_info(
            node,
            command_request::Command::DirectoryList(agora_protocol::v1::DirectoryList {
                path: path.to_string(),
                authorization: Some(authorization),
            }),
            Duration::from_secs(30),
        )?;
        match ok.result {
            Some(command_ok::Result::DirectoryEntries(e)) => Ok(e),
            _ => Err(ErrorInfo {
                code: "protocol.invalid_message".into(),
                ..Default::default()
            }),
        }
    }

    /// What an agent changed, as a unified diff (what the web's review screen reads): its folder's (`workspace.query` "diff",
    /// ADR-0053) or its worktree's (`git.diff`). Ids are the node's own (without the `node/` prefix).
    pub fn diff(
        &mut self,
        node: &str,
        target: &Place,
    ) -> Result<agora_protocol::v1::GitDiffResult, String> {
        let command = match target {
            Place::Folder(id) => {
                command_request::Command::WorkspaceQuery(agora_protocol::v1::WorkspaceQuery {
                    workspace_id: id.clone(),
                    operation: "diff".into(),
                    ..Default::default()
                })
            }
            Place::Worktree(id) => command_request::Command::GitDiff(agora_protocol::v1::GitDiff {
                worktree_id: id.clone(),
                path: String::new(),
            }),
        };
        let ok = self.command_ok(node, command, Duration::from_secs(30))?;
        match ok.result {
            Some(command_ok::Result::GitDiff(d)) => Ok(d),
            _ => Err("protocol.invalid_message".into()),
        }
    }

    /// The entries of `path` (relative, empty for the top) in an agent's folder or worktree, as the web's file browser lists them.
    pub fn files_list(
        &mut self,
        node: &str,
        place: &Place,
        path: &str,
    ) -> Result<Vec<agora_protocol::v1::FileEntry>, String> {
        let command = match place {
            Place::Folder(id) => {
                command_request::Command::WorkspaceQuery(agora_protocol::v1::WorkspaceQuery {
                    workspace_id: id.clone(),
                    operation: "list".into(),
                    path: path.to_string(),
                    ..Default::default()
                })
            }
            Place::Worktree(id) => {
                command_request::Command::FilesList(agora_protocol::v1::FilesList {
                    worktree_id: id.clone(),
                    path: path.to_string(),
                })
            }
        };
        let ok = self.command_ok(node, command, Duration::from_secs(30))?;
        match ok.result {
            Some(command_ok::Result::FilesList(l)) => Ok(l.entries),
            _ => Err("protocol.invalid_message".into()),
        }
    }

    /// The bytes of the file at `path` in an agent's folder or worktree.
    pub fn files_read(&mut self, node: &str, place: &Place, path: &str) -> Result<Vec<u8>, String> {
        let command = match place {
            Place::Folder(id) => {
                command_request::Command::WorkspaceQuery(agora_protocol::v1::WorkspaceQuery {
                    workspace_id: id.clone(),
                    operation: "read".into(),
                    path: path.to_string(),
                    ..Default::default()
                })
            }
            Place::Worktree(id) => {
                command_request::Command::FilesRead(agora_protocol::v1::FilesRead {
                    worktree_id: id.clone(),
                    path: path.to_string(),
                })
            }
        };
        let ok = self.command_ok(node, command, Duration::from_secs(30))?;
        match ok.result {
            Some(command_ok::Result::FilesRead(r)) => Ok(r.data),
            _ => Err("protocol.invalid_message".into()),
        }
    }

    /// What `node` makes of a folder before an agent starts there (ADR-0052 `directory.inspect`): its canonical path and project. Errors
    /// keep their parameters (#1044).
    pub fn inspect_folder(
        &mut self,
        node: &str,
        path: &str,
        authorization: agora_protocol::v1::DirectoryAuthorization,
    ) -> Result<agora_protocol::v1::DirectoryInfo, ErrorInfo> {
        let ok = self.command_info(
            node,
            command_request::Command::DirectoryInspect(agora_protocol::v1::DirectoryInspect {
                path: path.to_string(),
                create_project_id: false,
                authorization: Some(authorization),
            }),
            Duration::from_secs(30),
        )?;
        match ok.result {
            Some(command_ok::Result::DirectoryInfo(i)) => Ok(i),
            _ => Err(ErrorInfo {
                code: "protocol.invalid_message".into(),
                ..Default::default()
            }),
        }
    }

    /// Starts an agent of `launch.kind` in an inspected folder (ADR-0052 `agent.launch`); answers its hub-wide id.
    pub fn launch_in(
        &mut self,
        node: &str,
        launch: agora_protocol::v1::AgentLaunch,
    ) -> Result<String, String> {
        let ok = self.command_ok(
            node,
            command_request::Command::AgentLaunch(launch),
            Duration::from_secs(45),
        )?;
        match ok.result {
            Some(command_ok::Result::AgentLaunch(r)) => Ok(format!("{node}/{}", r.agent_id)),
            _ => Err("protocol.invalid_message".into()),
        }
    }

    /// ADR-0060: brings back an interrupted agent (`agent.resume`, signed by this terminal's key for that agent and choice); answers
    /// its hub-wide id. Errors keep the node's code and parameters for a localized reason.
    pub fn resume_in(
        &mut self,
        node: &str,
        resume: agora_protocol::v1::AgentResume,
    ) -> Result<String, ErrorInfo> {
        let ok = self.command_info(
            node,
            command_request::Command::AgentResume(resume),
            Duration::from_secs(45),
        )?;
        match ok.result {
            Some(command_ok::Result::AgentLaunch(r)) => Ok(format!("{node}/{}", r.agent_id)),
            _ => Err(ErrorInfo {
                code: "protocol.invalid_message".into(),
                ..Default::default()
            }),
        }
    }

    pub fn agent_kinds(&mut self, node: &str) -> Result<Vec<(String, String)>, String> {
        Ok(self
            .agent_kinds_all(node)?
            .into_iter()
            .filter(|k| k.installed)
            .map(|k| (k.kind, k.name))
            .collect())
    }

    /// Every agent a machine offers, the ones whose program the machine cannot find included.
    pub fn agent_kinds_all(&mut self, node: &str) -> Result<Vec<Kind>, String> {
        let ok = self.command_ok(
            node,
            command_request::Command::AgentKinds(agora_protocol::v1::AgentKinds {}),
            Duration::from_secs(15),
        )?;
        let Some(command_ok::Result::AgentKinds(k)) = ok.result else {
            return Ok(Vec::new());
        };
        Ok(k.kinds
            .into_iter()
            .map(|k| {
                let name = if k.display_name.is_empty() {
                    k.kind.clone()
                } else {
                    k.display_name
                };
                Kind {
                    kind: k.kind,
                    name,
                    installed: k.installed,
                    program: k.program,
                }
            })
            .collect())
    }

    /// ADR-0059: the launch options `kind` offers on `node` (none from an older machine or an unknown kind).
    pub fn kind_options(
        &mut self,
        node: &str,
        kind: &str,
    ) -> Result<Vec<agora_protocol::v1::LaunchOption>, String> {
        let ok = self.command_ok(
            node,
            command_request::Command::AgentKinds(agora_protocol::v1::AgentKinds {}),
            Duration::from_secs(15),
        )?;
        let Some(command_ok::Result::AgentKinds(k)) = ok.result else {
            return Ok(Vec::new());
        };
        Ok(usable_options(
            k.kinds
                .into_iter()
                .find(|k| k.kind == kind)
                .map(|k| k.launch_options)
                .unwrap_or_default(),
        ))
    }

    /// Starts an agent of `kind` in a project (its hub-wide id) on a machine; returns the agent's own id. Errors keep their parameters,
    /// so a refusal for the person's role can be told from one for this device (#1044).
    pub fn start_agent(
        &mut self,
        node: &str,
        project_id: &str,
        kind: &str,
    ) -> Result<String, ErrorInfo> {
        let ok = self.command_info(
            node,
            command_request::Command::AgentStart(agora_protocol::v1::AgentStart {
                project_id: project_id.to_string(),
                kind: kind.to_string(),
                ..Default::default()
            }),
            Duration::from_secs(30),
        )?;
        match ok.result {
            Some(command_ok::Result::AgentStart(s)) => Ok(s.agent_id),
            _ => Ok(String::new()),
        }
    }

    /// Gives a machine the owner's permission (ADR-0041) for `commands` within `scope` (`node`, `agent:<id>` or `project:<id>`) for
    /// `ttl`, signed with this device's key. `Err` carries the error code (`approval.signature_invalid`: the machine does not trust this
    /// device yet).
    pub fn grant(
        &mut self,
        node: &str,
        device: &Device,
        scope: &str,
        commands: &[&str],
        ttl: Duration,
    ) -> Result<(), String> {
        let grant_id = new_grant_id().ok_or_else(|| "client.sign".to_string())?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(0));
        let expires_at_ms = now + u64::try_from(ttl.as_millis()).unwrap_or(0);
        let commands: Vec<String> = commands.iter().map(|c| (*c).to_string()).collect();
        let signature = device
            .sign(&grant_message(
                node,
                &grant_id,
                scope,
                &commands,
                expires_at_ms,
            ))
            .ok_or_else(|| "client.sign".to_string())?;
        self.command(
            node,
            command_request::Command::GrantCreate(agora_protocol::v1::GrantCreate {
                grant_id,
                scope: scope.to_string(),
                commands,
                expires_at_ms,
                signer_device_id: device.id.clone(),
                signature,
            }),
            Duration::from_secs(15),
        )
    }

    /// ADR-0061: registers this terminal's key and name with the hub under the signed-in person (a candidate for their machines, never
    /// trust by itself). Sent after every sign-in; the hub keeps when it was last used.
    pub fn register_device(&mut self, device: &Device, name: &str) -> Result<(), String> {
        let registration = device
            .registration(name, now_ms())
            .ok_or_else(|| "client.sign".to_string())?;
        self.command(
            "",
            command_request::Command::DeviceRegister(registration),
            Duration::from_secs(15),
        )
    }

    /// ADR-0061: the signed-in person's registered devices, as the hub lists them.
    pub fn list_devices(&mut self) -> Result<Vec<agora_protocol::v1::DeviceInfo>, String> {
        let ok = self.command_ok(
            "",
            command_request::Command::DeviceList(agora_protocol::v1::DeviceList {}),
            Duration::from_secs(15),
        )?;
        match ok.result {
            Some(command_ok::Result::DeviceList(l)) => Ok(l.devices),
            _ => Ok(Vec::new()),
        }
    }

    /// ADR-0061: asks the person's other devices to approve this one.
    pub fn ask_approval(&mut self, device: &Device) -> Result<(), String> {
        let ask = device
            .ask(now_ms())
            .ok_or_else(|| "client.sign".to_string())?;
        self.command(
            "",
            command_request::Command::DeviceAskApproval(ask),
            Duration::from_secs(15),
        )
    }

    /// ADR-0061: this terminal vouches for `target` on every machine of the person; the hub carries it and each machine verifies it.
    pub fn endorse_device(
        &mut self,
        device: &Device,
        target: &agora_protocol::v1::DeviceInfo,
    ) -> Result<(), String> {
        let now = now_ms();
        let endorsement = device
            .endorse(target, &[], now)
            .ok_or_else(|| "client.sign".to_string())?;
        self.command(
            "",
            command_request::Command::DeviceEndorse(agora_protocol::v1::DeviceEndorse {
                endorsement: Some(endorsement),
            }),
            Duration::from_secs(15),
        )
    }

    /// Stops an agent on a machine.
    pub fn stop_agent(&mut self, node: &str, agent_id: &str) -> Result<(), String> {
        self.stop_agent_with(node, agent_id, false)
    }

    /// Stops an agent and, with `helpers`, the helpers it started and theirs (ADR-0057).
    pub fn stop_agent_with(
        &mut self,
        node: &str,
        agent_id: &str,
        helpers: bool,
    ) -> Result<(), String> {
        self.command(
            node,
            command_request::Command::AgentStop(agora_protocol::v1::AgentStop {
                agent_id: agent_id.to_string(),
                stop_helpers: helpers,
            }),
            Duration::from_secs(15),
        )
    }

    /// Removes a closed agent from every client's list (ADR-0053 `agent.forget`). The hub answers it itself, so it names no machine;
    /// `agent_id` is the hub-wide `<node>/<id>`. `agent.running` while it runs. Errors keep the hub's code and parameters for
    /// localized messages; other command helpers retain their code-only result.
    pub fn forget_agent(&mut self, agent_id: &str) -> Result<(), ErrorInfo> {
        self.command_info(
            "",
            command_request::Command::AgentForget(agora_protocol::v1::AgentForget {
                agent_id: agent_id.to_string(),
            }),
            Duration::from_secs(15),
        )
        .map(|_| ())
    }

    /// ADR-0073: the hub stops listing an offline machine (owner only); `refuse_return` also refuses it if it connects again.
    pub fn forget_node(&mut self, node_id: &str, refuse_return: bool) -> Result<(), ErrorInfo> {
        self.command_info(
            "",
            command_request::Command::NodeForget(agora_protocol::v1::NodeForget {
                node_id: node_id.to_string(),
                refuse_return,
            }),
            Duration::from_secs(15),
        )
        .map(|_| ())
    }

    /// Keys typed into the terminal (an ordinary routed command; the machine runs them in the order sent). Fire and forget. Never sent
    /// for a viewer (ADR-0106): their window watches, and the hub would refuse and write down every key.
    pub fn terminal_input(&mut self, node: &str, terminal_id: &str, data: &[u8]) {
        if self.person_role == PersonRole::Viewer {
            return;
        }
        let request = self.send_nowait(
            node,
            command_request::Command::TerminalInput(agora_protocol::v1::TerminalInput {
                terminal_id: terminal_id.to_string(),
                data: data.to_vec(),
            }),
        );
        self.typing.sent(request, node, terminal_id);
    }

    /// What the console sends to a terminal by itself, not typed by the user (a focus report, pointer motion): sent like typing, but
    /// a machine's refusal of it asks for no grant. Never sent for a viewer, as typing is not.
    pub fn terminal_report(&mut self, node: &str, terminal_id: &str, data: &[u8]) {
        if self.person_role == PersonRole::Viewer {
            return;
        }
        let request = self.send_nowait(
            node,
            command_request::Command::TerminalInput(agora_protocol::v1::TerminalInput {
                terminal_id: terminal_id.to_string(),
                data: data.to_vec(),
            }),
        );
        self.typing.track(request, node, terminal_id, false);
    }

    /// The typing machines refused because they want a grant first (ADR-0041) since the last call: the newest per terminal, oldest
    /// first. Reading it clears it.
    pub fn take_grant_wanted(&mut self) -> Vec<Refused> {
        std::mem::take(&mut self.typing.refused)
    }

    /// The typing machines took since the last call, the newest per terminal: (node, terminal, request). Reading it clears it.
    pub fn take_typing_taken(&mut self) -> Vec<(String, String, u64)> {
        std::mem::take(&mut self.typing.taken)
    }

    /// The number of the last request sent: a [`Refused`] with a number up to it was typed before now.
    pub fn last_request(&self) -> u64 {
        self.requests
    }

    /// How much typing still waits for its answer (a caller that must know every refusal has been read waits for 0).
    pub fn typing_unanswered(&self) -> usize {
        self.typing.sent.len()
    }

    /// Asks for the terminal's size (the hub keeps the smallest its watchers asked for). Never sent for a viewer (ADR-0106): the hub
    /// refuses it, writes the refusal down, and a viewer's window does not size the agent's terminal.
    pub fn terminal_resize(&mut self, node: &str, terminal_id: &str, cols: u16, rows: u16) {
        if self.person_role == PersonRole::Viewer {
            return;
        }
        self.send_nowait(
            node,
            command_request::Command::TerminalResize(agora_protocol::v1::TerminalResize {
                terminal_id: terminal_id.to_string(),
                cols: u32::from(cols),
                rows: u32::from(rows),
            }),
        );
    }

    /// Tells the hub we stopped watching this stream.
    pub fn terminal_close(&mut self, stream: u32) {
        self.history.clear(stream);
        let conn = &mut self.conn;
        let _ = self.rt.block_on(conn.send(&Frame {
            stream_id: stream,
            kind: STREAM_CLOSE,
            seq: None,
            payload: Vec::new(),
        }));
        self.term_frames.retain(|f| f.stream_id != stream);
    }

    fn send_nowait(&mut self, node: &str, command: command_request::Command) -> u64 {
        self.requests += 1;
        let req = CommandRequest {
            request_id: format!("t{}", self.requests),
            idempotency_key: String::new(),
            target_node: node.to_string(),
            command: Some(command),
        };
        let conn = &mut self.conn;
        let _ = self.rt.block_on(conn.send(&Frame {
            stream_id: STREAM_COMMANDS,
            kind: 0,
            seq: None,
            payload: req.encode_to_vec(),
        }));
        self.requests
    }
}

fn command_error(code: &str) -> ErrorInfo {
    ErrorInfo {
        code: code.to_string(),
        ..Default::default()
    }
}

/// ADR-0101: what a waiting request offers beyond a single answer, as the machine announced it (after its policy).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlwaysOffer {
    /// The agent's own name for its "always" option: untrusted text, shown quoted, may be empty.
    pub label: String,
    /// The exact option the answer is bound to (signed, never shown).
    pub option_id: String,
    /// The request offers no single answer: only one the agent remembers.
    pub only: bool,
}

/// ADR-0101: the "always" offer of `a`, as the web window reads it: ALWAYS in its `scopes` with the option's id, never for a request the
/// machine itself makes (`agent.start`, `agent.skip_permissions`, any `agent.` action).
pub fn always_offer(a: &agora_protocol::v1::ApprovalSnapshot) -> Option<AlwaysOffer> {
    use agora_protocol::v1::ApprovalScope;
    let has = |s: ApprovalScope| a.scopes.contains(&(s as i32));
    if a.action.starts_with("agent.")
        || !has(ApprovalScope::Always)
        || a.always_option_id.is_empty()
    {
        return None;
    }
    Some(AlwaysOffer {
        label: a.always_label.clone(),
        option_id: a.always_option_id.clone(),
        only: !has(ApprovalScope::Once) && !has(ApprovalScope::Unspecified),
    })
}

/// ADR-0101 and Codex's review of it: "for this session" is said only for a kind known to drop an "always" answer when its session ends. ACP
/// lets the agent decide what the answer covers and for how long, and no machine reports a kind as checked yet (no manifest or protocol
/// field says so), so every kind gets the words that say the agent keeps it (`-kept`). The ADR's session words keep their own ids for the
/// day a kind can be marked as checked (`session_limited`).
pub fn always_text(id: &'static str, session_limited: bool) -> &'static str {
    if session_limited {
        return id;
    }
    match id {
        "tui-request-answer-always" => key!("tui-request-answer-always-kept"),
        "tui-request-answer-always-only" => key!("tui-request-answer-always-only-kept"),
        "tui-chat-answer-always" => key!("tui-chat-answer-always-kept"),
        "tui-chat-answer-always-only" => key!("tui-chat-answer-always-only-kept"),
        "tui-tab-answer-always" => key!("tui-tab-answer-always-kept"),
        "tui-tab-answer-always-only" => key!("tui-tab-answer-always-only-kept"),
        "tui-allowed-always" => key!("tui-allowed-always-kept"),
        "tui-app-cmd-allow-always" => key!("tui-app-cmd-allow-always-kept"),
        "tui-allow-once-not-offered" => key!("tui-allow-once-not-offered-kept"),
        "tui-always-warning" => key!("tui-always-warning-kept"),
        other => other,
    }
}

/// ADR-0101: the hint that says the keys answering `a`: `plain` when it offers no "always", else the `always` or `only` one (`only`: no
/// single answer), in the words [`always_text`] picks.
pub fn answer_hint(
    a: &agora_protocol::v1::ApprovalSnapshot,
    plain: &'static str,
    always: &'static str,
    only: &'static str,
) -> &'static str {
    match always_offer(a) {
        None => plain,
        Some(o) if o.only => always_text(only, false),
        Some(_) => always_text(always, false),
    }
}

/// How a request is answered: `y`, ADR-0101's `A` (Alt+A in a tab), `n`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Allow,
    AllowAlways,
    Deny,
}

/// ADR-0101: the answer a key typed in the box that shows a request gives: `y` once, `A` always, `n` denies; any other key (a lowercase
/// `a`, a `Y`) leaves the request waiting, so a one-letter case difference is never the gap between a single and a remembered answer.
pub fn box_answer(key: Option<u8>) -> Option<Answer> {
    match key {
        Some(b'y') => Some(Answer::Allow),
        Some(b'A') => Some(Answer::AllowAlways),
        Some(b'n') => Some(Answer::Deny),
        _ => None,
    }
}

/// ADR-0101: the line (a text id) that refuses an answer `a` does not offer, before anything is signed or sent: "always" on a request that
/// offers only a single answer, a plain Allow on one that offers only "always". `None` when the answer may go.
pub fn not_offered(
    a: &agora_protocol::v1::ApprovalSnapshot,
    answer: Answer,
) -> Option<&'static str> {
    let offer = always_offer(a);
    match answer {
        Answer::AllowAlways if offer.is_none() => Some(key!("tui-allow-always-not-offered")),
        Answer::Allow if offer.is_some_and(|o| o.only) => {
            Some(always_text(key!("tui-allow-once-not-offered"), false))
        }
        _ => None,
    }
}

/// The bytes this device signs to answer `a` and the scope the answer carries, or the code it is refused with before anything is sent:
/// `approval.scope_unavailable` for an "always" answer the request does not offer, or a plain Allow on a request that offers only
/// "always" (the agent would remember it). A Deny is always a single answer.
pub fn answer_message(
    a: &agora_protocol::v1::ApprovalSnapshot,
    answer: Answer,
) -> Result<(Vec<u8>, agora_protocol::v1::ApprovalScope), &'static str> {
    use agora_protocol::v1::ApprovalScope;
    let (node, local) = a
        .approval_id
        .split_once('/')
        .unwrap_or(("", a.approval_id.as_str()));
    let offer = always_offer(a);
    match answer {
        Answer::Deny => Ok((
            decision_message(node, local, &a.action, &a.detail, false),
            ApprovalScope::Unspecified,
        )),
        Answer::Allow if offer.as_ref().is_some_and(|o| o.only) => {
            Err("approval.scope_unavailable")
        }
        Answer::Allow => Ok((
            decision_message(node, local, &a.action, &a.detail, true),
            ApprovalScope::Unspecified,
        )),
        Answer::AllowAlways => {
            // The node's own id of the agent: the part of the hub-wide key after `<node>/`.
            let agent = a
                .agent_id
                .strip_prefix(node)
                .and_then(|r| r.strip_prefix('/'))
                .filter(|r| !node.is_empty() && !r.is_empty());
            match (offer, agent) {
                (Some(o), Some(agent)) => Ok((
                    crate::device::always_message(
                        node,
                        local,
                        agent,
                        &a.action,
                        &a.detail,
                        &o.label,
                        &o.option_id,
                    ),
                    ApprovalScope::Always,
                )),
                _ => Err("approval.scope_unavailable"),
            }
        }
    }
}

/// The request an agent's tab answers: exactly `approval`, and only while it is still `agent`'s and waiting. Never another request.
pub fn exact_approval<'a>(
    snapshot: &'a Snapshot,
    agent: &str,
    approval: &str,
) -> Option<&'a agora_protocol::v1::ApprovalSnapshot> {
    snapshot
        .approvals
        .iter()
        .find(|a| a.approval_id == approval && a.agent_id == agent)
}

/// The request `y` / `n` answer: `agent`'s when it has one, else the first waiting.
pub fn answered_approval<'a>(
    snapshot: &'a Snapshot,
    agent: Option<&str>,
) -> Option<&'a agora_protocol::v1::ApprovalSnapshot> {
    agent
        .and_then(|id| snapshot.approvals.iter().find(|a| a.agent_id == id))
        .or_else(|| snapshot.approvals.first())
}

fn refused_for_grant(r: &CommandResponse) -> bool {
    matches!(&r.result, Some(command_response::Result::Err(e)) if e.code == "approval.grant_required")
}

/// Where an agent works, for asking its node about it ([`Session::diff`], [`Session::files_list`], [`Session::files_read`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    /// A folder the agent was started in (a registered workspace), by the node's id for it.
    Folder(String),
    /// The agent's worktree, by the node's id for it.
    Worktree(String),
}

/// Where to ask about `agent`'s work (its changes, its files), as the web does: the folder it is bound to when there is one, else its worktree; the
/// machine and the node's own id. `None` when it has neither.
pub fn place_of(snapshot: &Snapshot, agent: &str) -> Option<(String, Place)> {
    let a = snapshot.agents.iter().find(|a| a.agent_id == agent)?;
    let local = |id: &str| id.split_once('/').map_or(id, |(_, l)| l).to_string();
    if let Some(w) = snapshot
        .agent_workspaces
        .iter()
        .find(|b| b.agent_id == agent)
        .filter(|b| {
            snapshot
                .workspaces
                .iter()
                .any(|w| w.workspace_id == b.workspace_id)
        })
    {
        return Some((a.node_id.clone(), Place::Folder(local(&w.workspace_id))));
    }
    (!a.worktree_id.is_empty()).then(|| (a.node_id.clone(), Place::Worktree(local(&a.worktree_id))))
}

/// What a watched terminal said.
#[derive(Debug, PartialEq, Eq)]
pub enum TermEvent {
    /// Output to write as it is.
    Data(Vec<u8>),
    /// A repaint of the whole screen (first, and after the viewer fell behind).
    Screen(Vec<u8>),
    Resized(u16, u16),
    /// Output was dropped because nobody read it for long: watch the terminal again to get its screen.
    Behind,
    /// The program ended with this status (negative: a signal).
    Exit(i32),
    /// The hub ended the relay.
    Ended,
    Quiet,
    /// The link to the hub is gone.
    Lost,
}

/// The most history held for one screen: one replay, as the machine caps it (512 KiB, docs/specs/protocol.md, kind 4).
pub const HISTORY_HELD_MAX: usize = 512 * 1024;

/// History frames (kind 4, ADR-0100) per terminal stream, held for the screen (kind 1) of the same subscription that follows them.
/// Held history belongs to that subscription only: anything else on the stream (the relay ended, output was dropped, any other frame)
/// or the connection's loss drops it, so it never reaches a later subscription's screen (Codex review of #964). More than one replay
/// ([`HISTORY_HELD_MAX`]) is never held: past it the history is dropped, and the rest of it until the screen is ignored.
#[derive(Default)]
pub struct HeldHistory {
    /// The bytes held, and whether they went past the cap (then nothing more is held until the screen).
    streams: std::collections::HashMap<u32, (Vec<u8>, bool)>,
}

impl HeldHistory {
    /// What a frame of a terminal stream means: `None` for a history frame (held), the event for any other (unknown kinds are
    /// [`TermEvent::Quiet`]). A screen leaves the held history for [`HeldHistory::take`]; every other frame drops it.
    pub fn route(&mut self, f: &Frame) -> Option<TermEvent> {
        let id = f.stream_id;
        if f.kind == KIND_HISTORY {
            let (held, over) = self.streams.entry(id).or_default();
            if *over || held.len() + f.payload.len() > HISTORY_HELD_MAX {
                held.clear();
                *over = true;
            } else {
                held.extend_from_slice(&f.payload);
            }
            return None;
        }
        let e = term_event(f);
        if !matches!(e, TermEvent::Screen(_)) {
            self.clear(id);
        }
        Some(e)
    }

    /// The history held for `stream`'s screen, taken (empty when none).
    pub fn take(&mut self, stream: u32) -> Vec<u8> {
        self.streams
            .remove(&stream)
            .map(|(held, _)| held)
            .unwrap_or_default()
    }

    pub fn clear(&mut self, stream: u32) {
        self.streams.remove(&stream);
    }

    pub fn clear_all(&mut self) {
        self.streams.clear();
    }
}

/// The event a terminal stream frame carries; an unknown kind or a malformed payload is [`TermEvent::Quiet`].
fn term_event(f: &Frame) -> TermEvent {
    if f.kind == STREAM_CLOSE {
        return TermEvent::Ended;
    }
    let p = &f.payload;
    match f.kind {
        KIND_BEHIND => TermEvent::Behind,
        0 => TermEvent::Data(p.clone()),
        1 => TermEvent::Screen(p.clone()),
        2 if p.len() == 4 => TermEvent::Resized(
            u16::from_le_bytes([p[0], p[1]]),
            u16::from_le_bytes([p[2], p[3]]),
        ),
        3 if p.len() == 4 => TermEvent::Exit(i32::from_le_bytes([p[0], p[1], p[2], p[3]])),
        _ => TermEvent::Quiet,
    }
}

/// What an agent's conversation said.
pub enum ChatEvent {
    /// Entries to show; `true` means they replace what is held (a snapshot), `false` that they follow it.
    Lines(Vec<agora_protocol::v1::ChatEntry>, bool),
    Ended,
    Quiet,
    Lost,
}

#[cfg(test)]
mod name_tests {
    use super::server_names;

    #[test]
    fn an_ip_address_tries_itself_then_localhost_and_a_host_only_itself() {
        let n = |a: &str| {
            server_names(a)
                .iter()
                .map(|s| format!("{s:?}"))
                .collect::<Vec<_>>()
        };
        assert_eq!(n("192.168.1.2:7444").len(), 2);
        assert_eq!(n("[::1]:7444").len(), 2);
        assert_eq!(n("hub.example:7444").len(), 1);
    }
}

#[cfg(test)]
mod answer_tests {
    use super::answered_approval;
    use agora_protocol::v1::{ApprovalSnapshot, Snapshot};

    #[test]
    fn y_and_n_answer_the_selected_agents_request_or_else_the_first() {
        let req = |id: &str, agent: &str| ApprovalSnapshot {
            approval_id: id.into(),
            agent_id: agent.into(),
            ..Default::default()
        };
        let s = Snapshot {
            approvals: vec![req("m/1", "m/agent-1"), req("m/2", "m/agent-2")],
            ..Default::default()
        };
        let id = |a: Option<&str>| answered_approval(&s, a).map(|x| x.approval_id.clone());
        assert_eq!(id(Some("m/agent-2")).as_deref(), Some("m/2"));
        assert_eq!(id(Some("m/agent-9")).as_deref(), Some("m/1"));
        assert_eq!(id(None).as_deref(), Some("m/1"));
        assert_eq!(
            answered_approval(&Snapshot::default(), Some("m/agent-1")),
            None
        );
    }
}

/// Whether `f` is an event saying that a request for the user's answer arrived.
fn asks_the_user(f: &Frame) -> bool {
    agora_protocol::v1::PositionedEvent::decode(f.payload.as_slice())
        .ok()
        .and_then(|p| p.event)
        .is_some_and(|e| {
            matches!(
                e.body,
                Some(agora_protocol::v1::event::Body::ApprovalRequested(_))
            )
        })
}

/// A frame that arrived while something else was waiting for its own: an event marks the hub's state changed (and `asked` when it
/// is a request for the user's answer, as [`Session::next`] does), a snapshot is kept for [`Session::next`] (dropping it lost the
/// hub's first one, which is not sent again: the console stayed at "Connecting…"), and a frame of a terminal or chat stream is
/// queued for its reader. Anything else is not for a waiting caller.
fn set_aside(
    f: Frame,
    changed: &mut bool,
    asked: &mut bool,
    kept: &mut Option<Box<Snapshot>>,
    queue: &mut std::collections::VecDeque<Frame>,
    seen: &mut Seen,
) {
    if f.stream_id == STREAM_EVENTS && f.kind == kinds::POSITIONED_EVENT {
        if seen.note(&f) {
            *changed = true;
        }
        if asks_the_user(&f) {
            *asked = true;
        }
    } else if f.stream_id == STREAM_EVENTS && f.kind == kinds::SNAPSHOT {
        if let Ok(s) = Snapshot::decode(f.payload.as_slice()) {
            seen.snapshot_pos = Some(s.pos);
            *kept = Some(Box::new(s));
            // The snapshot is newer than any event before it.
            *changed = false;
        }
    } else if f.stream_id == STREAM_EVENTS && is_pair_frame(&f) {
        keep_pair_frame(f, queue);
    } else if f.stream_id >= agora_protocol::frame::FIRST_DYNAMIC_STREAM {
        queue_stream_frame(queue, f);
    }
}

/// Most bytes of one stream's frames kept for a reader that is busy elsewhere (ADR-0022: a slow reader never makes memory grow).
const MAX_QUEUED_STREAM_BYTES: usize = 2 * 1024 * 1024;
/// The queue is only measured once it is this long, so the usual push stays cheap.
const MEASURE_QUEUE_AT: usize = 128;
/// A frame the client makes itself: frames of this stream were dropped, the reader must watch it again (a fresh repaint).
const KIND_BEHIND: u8 = 200;
/// A terminal history frame (ADR-0100 decision 2).
const KIND_HISTORY: u8 = 4;

/// Queues a terminal or chat frame for its reader. Past [`MAX_QUEUED_STREAM_BYTES`] for its stream, that stream's queued frames are
/// dropped and a [`KIND_BEHIND`] marker stands in for them (and for what comes after, until the reader takes it).
fn queue_stream_frame(queue: &mut std::collections::VecDeque<Frame>, f: Frame) {
    if queue.len() >= MEASURE_QUEUE_AT {
        let id = f.stream_id;
        if queue
            .iter()
            .any(|q| q.stream_id == id && q.kind == KIND_BEHIND)
        {
            return;
        }
        let bytes: usize = queue
            .iter()
            .filter(|q| q.stream_id == id)
            .map(|q| q.payload.len())
            .sum::<usize>()
            + f.payload.len();
        if bytes > MAX_QUEUED_STREAM_BYTES {
            queue.retain(|q| q.stream_id != id);
            queue.push_back(Frame {
                stream_id: id,
                kind: KIND_BEHIND,
                seq: None,
                payload: Vec::new(),
            });
            return;
        }
    }
    queue.push_back(f);
}

/// ADR-0065: the hub's pairing notices to an owner's window (`PAIR_REQUESTED`, `PAIR_DECIDED`).
fn is_pair_frame(f: &Frame) -> bool {
    f.kind == kinds::PAIR_REQUESTED || f.kind == kinds::PAIR_DECIDED
}

/// How many pairing notices a session keeps for a caller that reads them: the hub shows at most 4 cards and batches its pushes, so
/// this is generous; a session whose caller never reads them does not grow.
const KEPT_PAIR_FRAMES: usize = 64;

/// Keeps a pairing notice in the queue (they are not terminal frames: their stream is the events stream, which no terminal reader
/// takes), dropping the oldest notice when too many are waiting.
fn keep_pair_frame(f: Frame, queue: &mut std::collections::VecDeque<Frame>) {
    queue.push_back(f);
    while queue.iter().filter(|q| is_pair_frame(q)).count() > KEPT_PAIR_FRAMES {
        if let Some(pos) = queue.iter().position(is_pair_frame) {
            queue.remove(pos);
        }
    }
}

/// ADR-0101: the bytes and scope of each answer, and what is refused before anything is sent.
#[cfg(test)]
mod always_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::{always_offer, answer_message, Answer};
    use crate::device::{always_message, approval_message};
    use agora_protocol::v1::{ApprovalScope, ApprovalSnapshot};

    fn ask(scopes: &[ApprovalScope]) -> ApprovalSnapshot {
        ApprovalSnapshot {
            approval_id: "laptop/ap-1".into(),
            agent_id: "laptop/agent-1".into(),
            action: "run_command".into(),
            detail: "{\"command\":\"git status\"}".into(),
            scopes: scopes.iter().map(|s| *s as i32).collect(),
            always_label: if scopes.contains(&ApprovalScope::Always) {
                "Always allow git status".into()
            } else {
                String::new()
            },
            always_option_id: if scopes.contains(&ApprovalScope::Always) {
                "allow-always-1".into()
            } else {
                String::new()
            },
            ..Default::default()
        }
    }

    #[test]
    fn the_offer_is_read_as_the_window_reads_it() {
        use ApprovalScope::{Always, Once};
        assert_eq!(always_offer(&ask(&[])), None);
        assert_eq!(always_offer(&ask(&[Once])), None);
        assert!(!always_offer(&ask(&[Once, Always])).unwrap().only);
        assert!(always_offer(&ask(&[Always])).unwrap().only);
        let mut start = ask(&[Once, Always]);
        start.action = "agent.start".into();
        assert_eq!(always_offer(&start), None);
        let mut no_id = ask(&[Always]);
        no_id.always_option_id.clear();
        assert_eq!(always_offer(&no_id), None);
    }

    #[test]
    fn always_signs_the_always_bytes_with_scope_always_and_nothing_else_does() {
        use ApprovalScope::{Always, Once};
        let a = ask(&[Once, Always]);
        let (m, scope) = answer_message(&a, Answer::AllowAlways).unwrap();
        assert_eq!(scope, ApprovalScope::Always);
        assert_eq!(
            m,
            always_message(
                "laptop",
                "ap-1",
                "agent-1",
                &a.action,
                &a.detail,
                "Always allow git status",
                "allow-always-1"
            )
        );
        assert_eq!(
            answer_message(&a, Answer::Allow).unwrap(),
            (
                approval_message("laptop", "ap-1", true),
                ApprovalScope::Unspecified
            )
        );
        assert_eq!(
            answer_message(&a, Answer::Deny).unwrap(),
            (
                approval_message("laptop", "ap-1", false),
                ApprovalScope::Unspecified
            )
        );
    }

    #[test]
    fn the_request_box_reads_a_capital_a_as_always_and_nothing_else_as_it() {
        use super::box_answer;
        assert_eq!(box_answer(Some(b'A')), Some(Answer::AllowAlways));
        assert_eq!(box_answer(Some(b'y')), Some(Answer::Allow));
        assert_eq!(box_answer(Some(b'n')), Some(Answer::Deny));
        for k in [b'a', b'Y', b'N', b'\r', 0x1b] {
            assert_eq!(box_answer(Some(k)), None, "{k}");
        }
        assert_eq!(box_answer(None), None);
    }

    #[test]
    fn an_answer_that_is_not_offered_is_named_before_anything_is_signed() {
        use super::not_offered;
        use ApprovalScope::{Always, Once};
        assert_eq!(
            not_offered(&ask(&[Once]), Answer::AllowAlways),
            Some("tui-allow-always-not-offered")
        );
        assert_eq!(not_offered(&ask(&[Once]), Answer::Allow), None);
        assert_eq!(
            not_offered(&ask(&[Always]), Answer::Allow),
            Some("tui-allow-once-not-offered-kept")
        );
        assert_eq!(not_offered(&ask(&[Always]), Answer::AllowAlways), None);
        assert_eq!(not_offered(&ask(&[Always]), Answer::Deny), None);
        assert_eq!(
            not_offered(&ask(&[Once, Always]), Answer::AllowAlways),
            None
        );
    }

    #[test]
    fn what_a_request_does_not_offer_is_refused_before_anything_is_sent() {
        use ApprovalScope::{Always, Once};
        // No offer: never ALWAYS.
        assert_eq!(
            answer_message(&ask(&[Once]), Answer::AllowAlways),
            Err("approval.scope_unavailable")
        );
        assert_eq!(
            answer_message(&ask(&[]), Answer::AllowAlways),
            Err("approval.scope_unavailable")
        );
        // Only "always": no plain Allow (the agent would remember it); a Deny stays a single answer.
        let only = ask(&[Always]);
        assert_eq!(
            answer_message(&only, Answer::Allow),
            Err("approval.scope_unavailable")
        );
        assert_eq!(
            answer_message(&only, Answer::Deny).unwrap().1,
            ApprovalScope::Unspecified
        );
        assert_eq!(
            answer_message(&only, Answer::AllowAlways).unwrap().1,
            ApprovalScope::Always
        );
        // An agent that is not the machine's own: nothing is signed for it.
        let mut other = ask(&[Once, Always]);
        other.agent_id = "box/agent-1".into();
        assert_eq!(
            answer_message(&other, Answer::AllowAlways),
            Err("approval.scope_unavailable")
        );
    }
}

#[cfg(test)]
mod queue_tests {
    use super::*;

    fn frame(stream_id: u32, bytes: usize) -> Frame {
        Frame {
            stream_id,
            kind: 0,
            seq: None,
            payload: vec![0; bytes],
        }
    }

    /// A reader busy elsewhere does not make a stream's queue grow past its bound: the frames go, a marker says the reader is behind,
    /// and another stream's frames are untouched.
    #[test]
    fn a_stream_nobody_reads_is_bounded_and_marked() {
        let first = agora_protocol::frame::FIRST_DYNAMIC_STREAM;
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(frame(first + 1, 10));
        for _ in 0..(MAX_QUEUED_STREAM_BYTES / 4096 + 2 * MEASURE_QUEUE_AT) {
            queue_stream_frame(&mut queue, frame(first, 4096));
        }
        let bytes: usize = queue
            .iter()
            .filter(|f| f.stream_id == first)
            .map(|f| f.payload.len())
            .sum();
        assert!(bytes <= MAX_QUEUED_STREAM_BYTES);
        assert_eq!(
            queue
                .iter()
                .filter(|f| f.stream_id == first && f.kind == KIND_BEHIND)
                .count(),
            1
        );
        assert_eq!(queue.iter().filter(|f| f.stream_id == first + 1).count(), 1);
    }
}

#[cfg(test)]
mod typing_tests {
    use super::*;

    fn answer(n: u64, code: Option<&str>) -> CommandResponse {
        CommandResponse {
            request_id: format!("t{n}"),
            result: code.map(|c| {
                command_response::Result::Err(ErrorInfo {
                    code: c.into(),
                    ..Default::default()
                })
            }),
        }
    }
    const GRANT: Option<&str> = Some("approval.grant_required");

    /// Codex review of #796: two terminals' refusals before the console looks are both kept; a terminal keeps only its newest.
    #[test]
    fn each_terminal_keeps_its_newest_refusal() {
        let mut t = Typing::default();
        t.sent(1, "laptop", "t-b");
        t.sent(2, "laptop", "t-a");
        t.sent(3, "laptop", "t-a");
        t.sent(4, "mac", "t-a");
        t.reply(&answer(1, GRANT));
        t.reply(&answer(2, GRANT));
        t.reply(&answer(3, GRANT));
        t.reply(&answer(4, Some("node.offline")));
        let kept: Vec<_> = t
            .refused
            .iter()
            .map(|r| (r.node.as_str(), r.terminal_id.as_str(), r.request))
            .collect();
        assert_eq!(kept, [("laptop", "t-b", 1), ("laptop", "t-a", 3)]);
        assert!(t.sent.is_empty(), "every answer read forgets its typing");
        // An answer to nothing typed (another reader's, or forgotten) asks nothing.
        t.reply(&answer(9, GRANT));
        assert_eq!(t.refused.len(), 2);
        // Codex review of #796: a focus report or pointer motion the console sent by itself is answered, and its refusal asks nothing.
        let mut t = Typing::default();
        t.track(7, "laptop", "t-a", false);
        t.reply(&answer(7, GRANT));
        assert!(t.refused.is_empty() && t.sent.is_empty());
        // Typing a machine took is told too, the newest per terminal (the console's notice goes after the next accepted key).
        let mut t = Typing::default();
        t.sent(10, "laptop", "t-a");
        t.sent(11, "laptop", "t-a");
        t.track(12, "laptop", "t-a", false);
        t.reply(&answer(11, None));
        t.reply(&answer(10, None));
        t.reply(&answer(12, None));
        assert_eq!(t.taken, [("laptop".to_string(), "t-a".to_string(), 11)]);
        assert!(t.refused.is_empty());
        // Answers out of order: the older typing's refusal, read last, does not replace the newer one.
        let mut t = Typing::default();
        t.sent(5, "laptop", "t-a");
        t.sent(6, "laptop", "t-a");
        t.reply(&answer(6, GRANT));
        t.reply(&answer(5, GRANT));
        assert_eq!(t.refused.len(), 1);
        assert_eq!(t.refused[0].request, 6);
    }

    /// Codex review of #796: a long paste without markers is one request per key; the first key's refusal still names its terminal,
    /// and past the hard cap the forgotten one fails closed (no refusal, so no question).
    #[test]
    fn a_long_paste_keeps_its_first_keys_terminal_and_the_cap_fails_closed() {
        let mut t = Typing::default();
        for n in 1..=300 {
            t.sent(n, "laptop", "t-a");
        }
        t.reply(&answer(1, GRANT));
        assert!(t
            .refused
            .iter()
            .any(|r| r.is_for("laptop", "t-a") && r.request == 1));
        let mut t = Typing::default();
        for n in 1..=(Typing::KEPT as u64 + 1) {
            t.sent(n, "laptop", "t-a");
        }
        assert_eq!(t.sent.len(), Typing::KEPT);
        t.reply(&answer(1, GRANT));
        assert!(t.refused.is_empty());
        t.reply(&answer(2, GRANT));
        assert_eq!(t.refused.len(), 1);
    }
}

#[cfg(test)]
mod set_aside_tests {
    use super::*;

    #[test]
    fn only_a_welcome_on_the_control_stream_names_a_role() {
        let frame = |stream_id, kind, payload: Vec<u8>| Frame {
            stream_id,
            kind,
            seq: None,
            payload,
        };
        let mut w = agora_protocol::v1::Welcome::default();
        w.set_person_role(PersonRole::Viewer);
        assert_eq!(
            welcome_role(&frame(STREAM_CONTROL, kinds::WELCOME, w.encode_to_vec())),
            Some(PersonRole::Viewer)
        );
        // An older hub's first frame: the snapshot, which is no Welcome.
        assert_eq!(
            welcome_role(&frame(
                STREAM_EVENTS,
                kinds::SNAPSHOT,
                Snapshot::default().encode_to_vec()
            )),
            None
        );
        // A published Welcome (a node's) reads as unspecified.
        assert_eq!(
            welcome_role(&frame(
                STREAM_CONTROL,
                kinds::WELCOME,
                agora_protocol::v1::Welcome::default().encode_to_vec()
            )),
            Some(PersonRole::Unspecified)
        );
    }

    #[test]
    fn a_snapshot_that_arrives_while_something_waits_is_kept_and_events_and_streams_are_not_lost() {
        let (mut changed, mut kept, mut queue) = (false, None, std::collections::VecDeque::new());
        let mut asked = false;
        let mut seen = Seen::default();
        let frame = |stream_id, kind, payload: Vec<u8>| Frame {
            stream_id,
            kind,
            seq: None,
            payload,
        };
        set_aside(
            frame(STREAM_EVENTS, kinds::POSITIONED_EVENT, vec![]),
            &mut changed,
            &mut asked,
            &mut kept,
            &mut queue,
            &mut seen,
        );
        assert!(changed && kept.is_none());
        let s = Snapshot {
            pos: 7,
            nodes: vec![agora_protocol::v1::NodeSnapshot {
                node_id: "mac".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        set_aside(
            frame(STREAM_EVENTS, kinds::SNAPSHOT, s.encode_to_vec()),
            &mut changed,
            &mut asked,
            &mut kept,
            &mut queue,
            &mut seen,
        );
        assert_eq!(kept.as_deref(), Some(&s));
        assert!(!changed, "a snapshot supersedes the events before it");
        // ADR-0090: the config options the hub replays right after the snapshot, at its position, are kept and want no fresh
        // snapshot (they would make the console refresh again and again).
        let config = |pos: u64, options: Vec<agora_protocol::v1::SessionConfigOption>| {
            agora_protocol::v1::PositionedEvent {
                pos,
                event: Some(agora_protocol::v1::Event {
                    producer_id: "mac".into(),
                    body: Some(agora_protocol::v1::event::Body::AgentSessionConfig(
                        agora_protocol::v1::AgentSessionConfig {
                            agent_id: "agent-1".into(),
                            options,
                        },
                    )),
                    ..Default::default()
                }),
            }
            .encode_to_vec()
        };
        let skip = agora_protocol::v1::SessionConfigOption {
            id: "yolo".into(),
            r#type: agora_protocol::v1::SessionConfigType::Boolean as i32,
            current: "true".into(),
            skips_permissions: true,
            ..Default::default()
        };
        set_aside(
            frame(
                STREAM_EVENTS,
                kinds::POSITIONED_EVENT,
                config(7, vec![skip.clone()]),
            ),
            &mut changed,
            &mut asked,
            &mut kept,
            &mut queue,
            &mut seen,
        );
        assert!(!changed);
        assert_eq!(
            seen.configs.options.get("mac/agent-1"),
            Some(&vec![skip.clone()])
        );
        assert!(
            std::mem::take(&mut seen.configs_changed),
            "a view draws again"
        );
        // The same report again changes nothing a view shows.
        set_aside(
            frame(
                STREAM_EVENTS,
                kinds::POSITIONED_EVENT,
                config(7, vec![skip]),
            ),
            &mut changed,
            &mut asked,
            &mut kept,
            &mut queue,
            &mut seen,
        );
        assert!(!seen.configs_changed);
        // A later report replaces it; an empty one forgets it.
        set_aside(
            frame(
                STREAM_EVENTS,
                kinds::POSITIONED_EVENT,
                config(7, Vec::new()),
            ),
            &mut changed,
            &mut asked,
            &mut kept,
            &mut queue,
            &mut seen,
        );
        assert!(seen.configs.options.is_empty() && !changed && seen.configs_changed);
        // A live mode change after the snapshot is kept for the agent and wants a redraw; an older one is in the snapshot already.
        let options = |pos: u64, mode: &str| {
            agora_protocol::v1::PositionedEvent {
                pos,
                event: Some(agora_protocol::v1::Event {
                    producer_id: "mac".into(),
                    body: Some(agora_protocol::v1::event::Body::AgentSessionOptions(
                        agora_protocol::v1::AgentSessionOptions {
                            agent_id: "agent-1".into(),
                            current_mode: mode.into(),
                            ..Default::default()
                        },
                    )),
                    ..Default::default()
                }),
            }
            .encode_to_vec()
        };
        seen.configs_changed = false;
        set_aside(
            frame(STREAM_EVENTS, kinds::POSITIONED_EVENT, options(6, "old")),
            &mut changed,
            &mut asked,
            &mut kept,
            &mut queue,
            &mut seen,
        );
        assert!(seen.configs.modes.is_empty() && !seen.configs_changed && !changed);
        set_aside(
            frame(STREAM_EVENTS, kinds::POSITIONED_EVENT, options(9, "bypass")),
            &mut changed,
            &mut asked,
            &mut kept,
            &mut queue,
            &mut seen,
        );
        assert_eq!(
            seen.configs.modes.get("mac/agent-1"),
            Some(&(Vec::new(), "bypass".to_string()))
        );
        assert!(seen.configs_changed && changed);
        changed = false;
        // An event after the snapshot is newer than it: it stays marked, so the caller asks again after taking the snapshot.
        set_aside(
            frame(
                STREAM_EVENTS,
                kinds::POSITIONED_EVENT,
                agora_protocol::v1::PositionedEvent {
                    pos: 8,
                    event: None,
                }
                .encode_to_vec(),
            ),
            &mut changed,
            &mut asked,
            &mut kept,
            &mut queue,
            &mut seen,
        );
        assert!(changed && kept.is_some());
        changed = false;
        set_aside(
            frame(
                agora_protocol::frame::FIRST_DYNAMIC_STREAM,
                0,
                b"x".to_vec(),
            ),
            &mut changed,
            &mut asked,
            &mut kept,
            &mut queue,
            &mut seen,
        );
        assert_eq!(queue.len(), 1);
        // A frame that is not for a waiting caller changes nothing.
        set_aside(
            frame(STREAM_COMMANDS, 1, vec![]),
            &mut changed,
            &mut asked,
            &mut kept,
            &mut queue,
            &mut seen,
        );
        assert!(!changed && queue.len() == 1 && kept.is_some());
        assert!(!asked, "no request for the user's answer came");
    }

    #[test]
    fn a_request_for_the_users_answer_that_arrives_while_something_waits_is_still_said() {
        let (mut changed, mut asked, mut kept, mut queue) =
            (false, false, None, std::collections::VecDeque::new());
        let mut seen = Seen::default();
        let event = |body| Frame {
            stream_id: STREAM_EVENTS,
            kind: kinds::POSITIONED_EVENT,
            seq: None,
            payload: agora_protocol::v1::PositionedEvent {
                pos: 1,
                event: Some(agora_protocol::v1::Event {
                    body: Some(body),
                    ..Default::default()
                }),
            }
            .encode_to_vec(),
        };
        // Another event changes the state but asks nothing.
        set_aside(
            event(agora_protocol::v1::event::Body::ApprovalResolved(
                Default::default(),
            )),
            &mut changed,
            &mut asked,
            &mut kept,
            &mut queue,
            &mut seen,
        );
        assert!(changed && !asked);
        set_aside(
            event(agora_protocol::v1::event::Body::ApprovalRequested(
                Default::default(),
            )),
            &mut changed,
            &mut asked,
            &mut kept,
            &mut queue,
            &mut seen,
        );
        assert!(changed && asked, "a full-screen view says the request");
    }
}

/// Milliseconds since the Unix epoch (0 before it).
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(0))
}

/// ADR-0059: the options a hub relayed that this window may offer: ids outside `[a-z0-9-]{1,32}` (a `!` or `,` in one, which a hub
/// could use to make a signed id look like a confirmation or like two ids) are dropped before anything is shown or signed. The list
/// is a display hint; the node decides which options skip permissions.
pub fn usable_options(
    options: Vec<agora_protocol::v1::LaunchOption>,
) -> Vec<agora_protocol::v1::LaunchOption> {
    options
        .into_iter()
        .filter(|o| {
            !o.id.is_empty()
                && o.id.len() <= 32
                && o.id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        })
        .collect()
}

#[cfg(test)]
mod launch_option_tests {
    use super::usable_options;
    use agora_protocol::v1::LaunchOption;

    #[test]
    fn ids_a_hub_could_abuse_are_never_offered() {
        let o = |id: &str, skips| LaunchOption {
            id: id.into(),
            label: "option-skip-permissions".into(),
            skips_permissions: skips,
        };
        let kept = usable_options(vec![
            o("skip-permissions!", false),
            o("a,b", false),
            o("Skip", true),
            o("", true),
            o("skip-permissions", true),
        ]);
        assert_eq!(kept, vec![o("skip-permissions", true)]);
    }
}

/// The bytes this device signs to answer the request `action` / `detail` (hub-wide id `<node>/<local>` split by the caller). ADR-0057: a
/// start is decided over the request as shown (one start; grants are given in the web window). ADR-0066: a start whose detail lists launch
/// settings is decided only over `agora-approval-start-v2`, which names them, so the answer covers what the card showed; one without keeps
/// `agora-approval-start-v1` byte for byte. There is no other message for an `agent.start`.
pub fn decision_message(
    node: &str,
    local: &str,
    action: &str,
    detail: &str,
    approved: bool,
) -> Vec<u8> {
    if action != "agent.start" {
        return approval_message(node, local, approved);
    }
    let settings = start_settings(detail);
    // ADR-0059 decision 5: a start that lists launch options is decided only over `agora-approval-start-v3`, which names them.
    let options: Vec<String> = start_option_rows(detail)
        .into_iter()
        .map(|(_, id, _)| id)
        .collect();
    if !options.is_empty() {
        crate::device::start_message_v3(
            node, local, approved, 0, false, &settings, &options, detail,
        )
    } else if settings.is_empty() {
        crate::device::start_message(node, local, approved, 0, false, detail)
    } else {
        crate::device::start_message_v2(node, local, approved, 0, false, &settings, detail)
    }
}

/// ADR-0066: the settings an `agent.start` detail lists as `(label, id, value)`, read as the node reads them (strings only; anything else
/// is empty). The card shows exactly these rows and the signature covers exactly their `(id, value)` pairs.
pub(crate) fn start_setting_rows(detail: &str) -> Vec<(String, String, String)> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(detail) else {
        return Vec::new();
    };
    let Some(list) = v.get("settings").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .map(|x| {
            let get = |k: &str| {
                x.get(k)
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            };
            (get("label"), get("id"), get("value"))
        })
        .collect()
}

/// ADR-0059 decision 5: the launch options an `agent.start` detail lists as `(label, id, skips_permissions)`, read as the node reads them
/// (strings only; anything else is empty, and only `true` skips). The card shows exactly these and the v3 message signs their ids.
pub(crate) fn start_option_rows(detail: &str) -> Vec<(String, String, bool)> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(detail) else {
        return Vec::new();
    };
    let Some(list) = v
        .get("launch_options")
        .and_then(serde_json::Value::as_array)
    else {
        return Vec::new();
    };
    list.iter()
        .map(|x| {
            let get = |k: &str| {
                x.get(k)
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            };
            let skips = x
                .get("skips_permissions")
                .and_then(serde_json::Value::as_bool)
                == Some(true);
            (get("label"), get("id"), skips)
        })
        .collect()
}

/// The `(id, value)` pairs of [`start_setting_rows`]: what the v2 message signs.
fn start_settings(detail: &str) -> Vec<(String, String)> {
    start_setting_rows(detail)
        .into_iter()
        .map(|(_, id, value)| (id, value))
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod decision_tests {
    use super::decision_message;
    use crate::device::{approval_message, start_message, start_message_v2, start_message_v3};

    const PLAIN: &str = r#"{"kind":"codex","folder":"/w","prompt":"x","settings":[]}"#;
    const WITH: &str = r#"{"kind":"codex","folder":"/w","prompt":"x","settings":[{"id":"model","label":"setting-model","value":"gpt-6.1-sol"},{"id":"effort","label":"setting-effort","value":"high"}]}"#;

    /// Without settings the answer is the v1 message byte for byte (and a detail with no `settings` key at all is the same).
    #[test]
    fn a_start_without_settings_keeps_the_v1_message() {
        for detail in [PLAIN, r#"{"kind":"codex","prompt":"x"}"#, "not json"] {
            for approved in [true, false] {
                assert_eq!(
                    decision_message("laptop", "7", "agent.start", detail, approved),
                    start_message("laptop", "7", approved, 0, false, detail)
                );
            }
        }
        assert!(decision_message("laptop", "7", "agent.start", PLAIN, true)
            .starts_with(b"agora-approval-start-v1\0"));
    }

    /// A start that lists settings is never answered over a message that does not name them: allow or deny, only v2 is signed.
    #[test]
    fn a_start_with_settings_is_only_ever_signed_over_v2() {
        let settings = vec![
            ("model".to_string(), "gpt-6.1-sol".to_string()),
            ("effort".to_string(), "high".to_string()),
        ];
        for approved in [true, false] {
            let m = decision_message("laptop", "7", "agent.start", WITH, approved);
            assert_eq!(
                m,
                start_message_v2("laptop", "7", approved, 0, false, &settings, WITH)
            );
            assert!(m.starts_with(b"agora-approval-start-v2\0"));
            assert_ne!(m, start_message("laptop", "7", approved, 0, false, WITH));
        }
        // Other requests are not starts: their message is the plain one.
        assert_eq!(
            decision_message("laptop", "7", "files.write", WITH, true),
            approval_message("laptop", "7", true)
        );
    }

    /// The shared vectors (`schema/fixtures/signing/approval-start-v2.json`, also checked by the node and the web window): the message
    /// this terminal signs for each case is the fixture's bytes, with the settings read out of the detail as the card shows them.
    #[test]
    fn the_shared_start_v2_vectors_are_what_this_terminal_signs() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../schema/fixtures/signing/approval-start-v2.json");
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let hex = |m: &[u8]| -> String { m.iter().map(|b| format!("{b:02x}")).collect() };
        for c in v["cases"].as_array().unwrap() {
            let listed: Vec<(String, String)> = c["settings"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| {
                    (
                        p[0].as_str().unwrap().to_string(),
                        p[1].as_str().unwrap().to_string(),
                    )
                })
                .collect();
            let whole = start_message_v2(
                c["node"].as_str().unwrap(),
                c["approval_id"].as_str().unwrap(),
                c["approved"].as_bool().unwrap(),
                u32::try_from(c["helper_grant"].as_u64().unwrap()).unwrap(),
                c["helpers_may_start"].as_bool().unwrap(),
                &listed,
                c["detail"].as_str().unwrap(),
            );
            assert_eq!(hex(&whole), c["hex"].as_str().unwrap(), "{}", c["name"]);
            // This terminal signs no grant and no nested permission: only the cases without either apply to it.
            if c["helper_grant"].as_u64() != Some(0)
                || c["helpers_may_start"].as_bool() != Some(false)
            {
                continue;
            }
            let m = decision_message(
                c["node"].as_str().unwrap(),
                c["approval_id"].as_str().unwrap(),
                "agent.start",
                c["detail"].as_str().unwrap(),
                c["approved"].as_bool().unwrap(),
            );
            assert_eq!(hex(&m), c["hex"].as_str().unwrap(), "{}", c["name"]);
        }
    }
    /// ADR-0059 decision 5: a start that lists launch options is only ever answered over v3, which names them (allow or deny).
    #[test]
    fn a_start_with_launch_options_is_only_ever_signed_over_v3() {
        const OPTS: &str = r#"{"kind":"codex","prompt":"x","settings":[{"id":"effort","label":"setting-effort","value":"high"}],"launch_options":[{"id":"skip-permissions","label":"option-skip-permissions","skips_permissions":true}]}"#;
        let settings = vec![("effort".to_string(), "high".to_string())];
        let options = vec!["skip-permissions".to_string()];
        for approved in [true, false] {
            let m = decision_message("laptop", "7", "agent.start", OPTS, approved);
            assert_eq!(
                m,
                start_message_v3("laptop", "7", approved, 0, false, &settings, &options, OPTS)
            );
            assert!(m.starts_with(b"agora-approval-start-v3\0"));
            assert_ne!(
                m,
                start_message_v2("laptop", "7", approved, 0, false, &settings, OPTS)
            );
        }
    }

    /// The shared v3 vectors (`schema/fixtures/signing/approval-start-v3.json`, also checked by the node and the web window).
    #[test]
    fn the_shared_start_v3_vectors_are_what_this_terminal_signs() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../schema/fixtures/signing/approval-start-v3.json");
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let hex = |m: &[u8]| -> String { m.iter().map(|b| format!("{b:02x}")).collect() };
        for c in v["cases"].as_array().unwrap() {
            let settings: Vec<(String, String)> = c["settings"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| {
                    (
                        p[0].as_str().unwrap().to_string(),
                        p[1].as_str().unwrap().to_string(),
                    )
                })
                .collect();
            let options: Vec<String> = c["options"]
                .as_array()
                .unwrap()
                .iter()
                .map(|o| o.as_str().unwrap().to_string())
                .collect();
            let whole = start_message_v3(
                c["node"].as_str().unwrap(),
                c["approval_id"].as_str().unwrap(),
                c["approved"].as_bool().unwrap(),
                u32::try_from(c["helper_grant"].as_u64().unwrap()).unwrap(),
                c["helpers_may_start"].as_bool().unwrap(),
                &settings,
                &options,
                c["detail"].as_str().unwrap(),
            );
            assert_eq!(hex(&whole), c["hex"].as_str().unwrap(), "{}", c["name"]);
            if c["helper_grant"].as_u64() != Some(0)
                || c["helpers_may_start"].as_bool() != Some(false)
            {
                continue;
            }
            let m = decision_message(
                c["node"].as_str().unwrap(),
                c["approval_id"].as_str().unwrap(),
                "agent.start",
                c["detail"].as_str().unwrap(),
                c["approved"].as_bool().unwrap(),
            );
            assert_eq!(hex(&m), c["hex"].as_str().unwrap(), "{}", c["name"]);
        }
    }
}

#[cfg(test)]
mod history_tests {
    use super::{HeldHistory, TermEvent, HISTORY_HELD_MAX, KIND_BEHIND, STREAM_CLOSE};
    use agora_protocol::Frame;

    fn frame(stream: u32, kind: u8, payload: &[u8]) -> Frame {
        Frame {
            stream_id: stream,
            kind,
            seq: None,
            payload: payload.to_vec(),
        }
    }

    #[test]
    fn history_travels_with_the_screen_of_its_own_subscription() {
        let mut h = HeldHistory::default();
        assert_eq!(h.route(&frame(3, 4, b"ab")), None);
        assert_eq!(h.route(&frame(3, 4, b"c")), None);
        assert_eq!(
            h.route(&frame(3, 1, b"s")),
            Some(TermEvent::Screen(b"s".to_vec()))
        );
        assert_eq!(h.take(3), b"abc");
        assert_eq!(h.take(3), b"", "taken once");
    }

    // Codex review of #964: history whose screen never came (the relay ended, output was dropped, the connection was lost, the
    // view stopped watching) is never handed to a later screen.
    #[test]
    fn history_whose_screen_never_came_is_dropped() {
        for end in [STREAM_CLOSE, KIND_BEHIND, 0] {
            let mut h = HeldHistory::default();
            assert_eq!(h.route(&frame(3, 4, b"stale")), None);
            assert!(h.route(&frame(3, end, b"x")).is_some());
            assert_eq!(
                h.route(&frame(3, 1, b"s")),
                Some(TermEvent::Screen(b"s".to_vec()))
            );
            assert_eq!(h.take(3), b"", "after kind {end}");
        }
        let mut h = HeldHistory::default();
        h.route(&frame(3, 4, b"stale"));
        h.route(&frame(4, 4, b"other"));
        h.clear_all();
        assert_eq!((h.take(3), h.take(4)), (Vec::new(), Vec::new()));
        h.route(&frame(5, 4, b"stale"));
        h.clear(5);
        assert_eq!(h.take(5), b"");
    }

    #[test]
    fn never_more_than_one_replay_is_held() {
        let mut h = HeldHistory::default();
        h.route(&frame(3, 4, &vec![b'a'; HISTORY_HELD_MAX - 1]));
        h.route(&frame(3, 4, b"xy"));
        h.route(&frame(3, 4, b"z"));
        h.route(&frame(3, 1, b"s"));
        assert_eq!(h.take(3), b"");
        h.route(&frame(3, 4, b"ok"));
        h.route(&frame(3, 1, b"s"));
        assert_eq!(h.take(3), b"ok");
    }
}

#[cfg(test)]
mod same_process_tests {
    use super::*;
    use agora_protocol::v1::{
        event::Body, AgentSessionConfig, AgentStarted, Event, PositionedEvent,
    };

    fn at(pos: u64, body: Body) -> Frame {
        Frame {
            stream_id: STREAM_EVENTS,
            kind: kinds::POSITIONED_EVENT,
            seq: None,
            payload: PositionedEvent {
                pos,
                event: Some(Event {
                    producer_id: "mac".into(),
                    body: Some(body),
                    ..Default::default()
                }),
            }
            .encode_to_vec(),
        }
    }

    /// ADR-0104 (#930): the terminal client sums no usage of its own; it draws the hub's snapshot, which keeps a kept agent's counters.
    /// So the `AgentStarted` (`same_process`) of an agent its restarted node took up asks for a fresh snapshot, and leaves the options
    /// it holds for that agent as they are (the node re-states them right after, and each report replaces the last).
    #[test]
    fn a_taken_up_agent_asks_for_the_hubs_snapshot_and_keeps_its_held_options() {
        let mut seen = Seen {
            snapshot_pos: Some(7),
            ..Seen::default()
        };
        let option = agora_protocol::v1::SessionConfigOption {
            id: "effort".into(),
            current: "high".into(),
            ..Default::default()
        };
        seen.note(&at(
            7,
            Body::AgentSessionConfig(AgentSessionConfig {
                agent_id: "agent-1".into(),
                options: vec![option.clone()],
            }),
        ));
        seen.configs_changed = false;
        let taken_up = Body::AgentStarted(AgentStarted {
            agent_id: "agent-1".into(),
            node_id: "mac".into(),
            same_process: true,
            ..Default::default()
        });
        assert!(seen.note(&at(8, taken_up)), "a fresh snapshot is wanted");
        assert_eq!(seen.configs.options.get("mac/agent-1"), Some(&vec![option]));
        assert!(!seen.configs_changed);
    }
}

#[cfg(test)]
mod start_kinds_tests {
    use super::{is_registry_kind, start_kinds, Kind};

    fn k(kind: &str, installed: bool) -> Kind {
        Kind {
            kind: kind.into(),
            name: kind.into(),
            installed,
            program: String::new(),
        }
    }

    /// #1134: installed hand-made kinds, then installed registry ones, each in the machine's order; missing registry ones are left out.
    #[test]
    fn registry_kinds_follow_the_hand_made_ones_and_missing_ones_are_left_out() {
        let got = start_kinds(vec![
            k("registry-gemini", true),
            k("claude", true),
            k("registry-goose", false),
            k("codex", false),
            k("registry-qwen-code", true),
            k("shell", true),
        ]);
        let ids = |v: &[Kind]| v.iter().map(|k| k.kind.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&got.installed), ["claude", "shell"]);
        assert_eq!(
            ids(&got.untested),
            ["registry-gemini", "registry-qwen-code"]
        );
        assert_eq!(ids(&got.missing), ["codex"]);
        assert_eq!(start_kinds(Vec::new()), super::StartKinds::default());
    }

    #[test]
    fn a_registry_kind_is_told_by_its_prefix_only() {
        assert!(is_registry_kind("registry-gemini"));
        assert!(!is_registry_kind("gemini"));
        assert!(!is_registry_kind("my-registry-gemini"));
        assert!(!is_registry_kind("registry"));
    }
}

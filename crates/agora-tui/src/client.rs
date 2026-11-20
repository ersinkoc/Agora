//! The hub client of the terminal console (ADR-0043): sign in with a one-time code, then follow the hub's snapshot. It speaks the
//! same protocol as the browser (ADR-0027) and trusts only the hub's own CA.
//!
//! Every call blocks the calling thread (a small runtime is built per call), so the terminal loop stays plain code.

use std::sync::Arc;
use std::time::Duration;

use crate::device::{approval_message, grant_message, new_grant_id, Device};
use agora_protocol::frame::{kinds, STREAM_CLOSE, STREAM_COMMANDS, STREAM_EVENTS};
use agora_protocol::v1::{
    command_ok, command_request, command_response, CommandOk, CommandRequest, CommandResponse,
    ErrorInfo, Snapshot,
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
            Ok::<_, ClientError>(conn)
        })?;
        Ok(Session {
            rt,
            conn,
            requests: 0,
            term_frames: std::collections::VecDeque::new(),
            next_stream: agora_protocol::frame::FIRST_DYNAMIC_STREAM,
            changed: false,
            asked: false,
            snapshot: None,
            typing: Typing::default(),
        })
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
        self.rt.block_on(async {
            match tokio::time::timeout(wait, conn.recv()).await {
                Err(_) => Update::Quiet,
                // The answer to something typed without waiting: only a missing grant matters.
                Ok(Ok(Some(f))) if typing.answer(&f) => Update::Quiet,
                // A terminal being watched alongside: kept for `terminal_next`, never dropped here.
                Ok(Ok(Some(f))) if f.stream_id >= agora_protocol::frame::FIRST_DYNAMIC_STREAM => {
                    queue.push_back(f);
                    Update::Quiet
                }
                Ok(Ok(Some(f))) if f.stream_id == STREAM_EVENTS && f.kind == kinds::SNAPSHOT => {
                    match Snapshot::decode(f.payload.as_slice()) {
                        Ok(s) => Update::Snapshot(Box::new(s)),
                        Err(_) => Update::Closed,
                    }
                }
                Ok(Ok(Some(f)))
                    if f.stream_id == STREAM_EVENTS && f.kind == kinds::POSITIONED_EVENT =>
                {
                    if asks_the_user(&f) {
                        *asked = true;
                    }
                    Update::Changed
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
                set_aside(f, changed, asked, kept, term_frames);
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
        let Some(a) = answered_approval(snapshot, agent) else {
            return Ok(false);
        };
        // Hub-wide ids are `<node>/<local id>`; the machine knows the local one.
        let (node, local) = a
            .approval_id
            .split_once('/')
            .unwrap_or(("", a.approval_id.as_str()));
        let message = decision_message(node, local, &a.action, &a.detail, approved);
        let signature = device
            .sign(&message)
            .ok_or_else(|| "client.sign".to_string())?;
        let cmd = command_request::Command::ApprovalResolve(agora_protocol::v1::ApprovalResolve {
            approval_id: local.to_string(),
            approved,
            signer_device_id: device.id.clone(),
            signature,
            helper_grant: 0,
            helpers_may_start: false,
        });
        self.command(node, cmd, Duration::from_secs(15))
            .map(|()| true)
    }

    /// Opens a terminal of a machine (ADR-0029): the hub relays its screen and output on a stream of ours. `terminal_id` is the
    /// machine's own id. Returns the stream; read it with [`Session::terminal_next`].
    pub fn terminal_open(&mut self, node: &str, terminal_id: &str) -> Result<u32, String> {
        self.open_stream(
            node,
            command_request::Command::TerminalSubscribe(agora_protocol::v1::TerminalSubscribe {
                terminal_id: terminal_id.to_string(),
                format: String::new(),
            }),
        )
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
                    set_aside(f, changed, asked, kept, term_frames);
                }
            }
        })
    }

    /// The next thing on a terminal stream, waiting up to `wait`.
    pub fn terminal_next(&mut self, stream: u32, wait: Duration) -> TermEvent {
        fn event(f: &Frame) -> Option<TermEvent> {
            if f.kind == STREAM_CLOSE {
                return Some(TermEvent::Ended);
            }
            let p = &f.payload;
            match f.kind {
                0 => Some(TermEvent::Data(p.clone())),
                1 => Some(TermEvent::Screen(p.clone())),
                2 if p.len() == 4 => Some(TermEvent::Resized(
                    u16::from_le_bytes([p[0], p[1]]),
                    u16::from_le_bytes([p[2], p[3]]),
                )),
                3 if p.len() == 4 => Some(TermEvent::Exit(i32::from_le_bytes([
                    p[0], p[1], p[2], p[3],
                ]))),
                _ => None,
            }
        }
        if let Some(pos) = self.term_frames.iter().position(|f| f.stream_id == stream) {
            if let Some(f) = self.term_frames.remove(pos) {
                return event(&f).unwrap_or(TermEvent::Quiet);
            }
        }
        let conn = &mut self.conn;
        let changed = &mut self.changed;
        let kept = &mut self.snapshot;
        let queue = &mut self.term_frames;
        let typing = &mut self.typing;
        let asked = &mut self.asked;
        self.rt.block_on(async {
            match tokio::time::timeout(wait, conn.recv()).await {
                Err(_) => TermEvent::Quiet,
                Ok(Ok(Some(f))) if f.stream_id == stream => event(&f).unwrap_or(TermEvent::Quiet),
                Ok(Ok(Some(f))) => {
                    // The answer to something typed without waiting: only a missing grant matters.
                    if !typing.answer(&f) {
                        set_aside(f, changed, asked, kept, queue);
                    }
                    TermEvent::Quiet
                }
                Ok(_) => TermEvent::Lost,
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
        let typing = &mut self.typing;
        self.rt.block_on(async {
            match tokio::time::timeout(wait, conn.recv()).await {
                Err(_) => ChatEvent::Quiet,
                Ok(Ok(Some(f))) if f.stream_id == stream => take(&f).unwrap_or(ChatEvent::Quiet),
                Ok(Ok(Some(f))) => {
                    if !typing.answer(&f) {
                        set_aside(f, changed, asked, kept, queue);
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

    /// What the agents used so far, as the hub estimates it: total in micro-USD (the hub answers this itself, ADR-0033).
    pub fn usage_total(&mut self) -> Result<u64, String> {
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
    /// signature for exactly that folder.
    pub fn list_folder(
        &mut self,
        node: &str,
        path: &str,
        authorization: agora_protocol::v1::DirectoryAuthorization,
    ) -> Result<agora_protocol::v1::DirectoryEntries, String> {
        let ok = self.command_ok(
            node,
            command_request::Command::DirectoryList(agora_protocol::v1::DirectoryList {
                path: path.to_string(),
                authorization: Some(authorization),
            }),
            Duration::from_secs(30),
        )?;
        match ok.result {
            Some(command_ok::Result::DirectoryEntries(e)) => Ok(e),
            _ => Err("protocol.invalid_message".into()),
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

    /// What `node` makes of a folder before an agent starts there (ADR-0052 `directory.inspect`): its canonical path and project.
    pub fn inspect_folder(
        &mut self,
        node: &str,
        path: &str,
        authorization: agora_protocol::v1::DirectoryAuthorization,
    ) -> Result<agora_protocol::v1::DirectoryInfo, String> {
        let ok = self.command_ok(
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
            _ => Err("protocol.invalid_message".into()),
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

    /// Starts an agent of `kind` in a project (its hub-wide id) on a machine; returns the agent's own id.
    pub fn start_agent(
        &mut self,
        node: &str,
        project_id: &str,
        kind: &str,
    ) -> Result<String, String> {
        let ok = self.command_ok(
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

    /// Keys typed into the terminal (an ordinary routed command; the machine runs them in the order sent). Fire and forget.
    pub fn terminal_input(&mut self, node: &str, terminal_id: &str, data: &[u8]) {
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
    /// a machine's refusal of it asks for no grant.
    pub fn terminal_report(&mut self, node: &str, terminal_id: &str, data: &[u8]) {
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

    pub fn terminal_resize(&mut self, node: &str, terminal_id: &str, cols: u16, rows: u16) {
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
pub enum TermEvent {
    /// Output to write as it is.
    Data(Vec<u8>),
    /// A repaint of the whole screen (first, and after the viewer fell behind).
    Screen(Vec<u8>),
    Resized(u16, u16),
    /// The program ended with this status (negative: a signal).
    Exit(i32),
    /// The hub ended the relay.
    Ended,
    Quiet,
    /// The link to the hub is gone.
    Lost,
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
) {
    if f.stream_id == STREAM_EVENTS && f.kind == kinds::POSITIONED_EVENT {
        *changed = true;
        if asks_the_user(&f) {
            *asked = true;
        }
    } else if f.stream_id == STREAM_EVENTS && f.kind == kinds::SNAPSHOT {
        if let Ok(s) = Snapshot::decode(f.payload.as_slice()) {
            *kept = Some(Box::new(s));
            // The snapshot is newer than any event before it.
            *changed = false;
        }
    } else if f.stream_id == STREAM_EVENTS && is_pair_frame(&f) {
        keep_pair_frame(f, queue);
    } else if f.stream_id >= agora_protocol::frame::FIRST_DYNAMIC_STREAM {
        queue.push_back(f);
    }
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
    fn a_snapshot_that_arrives_while_something_waits_is_kept_and_events_and_streams_are_not_lost() {
        let (mut changed, mut kept, mut queue) = (false, None, std::collections::VecDeque::new());
        let mut asked = false;
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
        );
        assert!(changed && kept.is_none());
        let s = Snapshot {
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
        );
        assert_eq!(kept.as_deref(), Some(&s));
        assert!(!changed, "a snapshot supersedes the events before it");
        // An event after the snapshot is newer than it: it stays marked, so the caller asks again after taking the snapshot.
        set_aside(
            frame(STREAM_EVENTS, kinds::POSITIONED_EVENT, vec![]),
            &mut changed,
            &mut asked,
            &mut kept,
            &mut queue,
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
        );
        assert_eq!(queue.len(), 1);
        // A frame that is not for a waiting caller changes nothing.
        set_aside(
            frame(STREAM_COMMANDS, 1, vec![]),
            &mut changed,
            &mut asked,
            &mut kept,
            &mut queue,
        );
        assert!(!changed && queue.len() == 1 && kept.is_some());
        assert!(!asked, "no request for the user's answer came");
    }

    #[test]
    fn a_request_for_the_users_answer_that_arrives_while_something_waits_is_still_said() {
        let (mut changed, mut asked, mut kept, mut queue) =
            (false, false, None, std::collections::VecDeque::new());
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

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
        let Ok(Ok(tcp)) =
            tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(&hub.addr)).await
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
    /// A machine refused something that was sent without waiting for the answer (typing) because it wants a grant first.
    grant_wanted: bool,
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
            grant_wanted: false,
        })
    }

    /// Waits up to `wait` for the hub to say something.
    pub fn next(&mut self, wait: Duration) -> Update {
        if std::mem::take(&mut self.changed) {
            return Update::Changed;
        }
        let conn = &mut self.conn;
        let queue = &mut self.term_frames;
        self.rt.block_on(async {
            match tokio::time::timeout(wait, conn.recv()).await {
                Err(_) => Update::Quiet,
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
                    Update::Changed
                }
                Ok(Ok(Some(_))) => Update::Quiet,
                Ok(_) => Update::Closed,
            }
        })
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
    fn command_info(
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
        let term_frames = &mut self.term_frames;
        let grant_wanted = &mut self.grant_wanted;
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
                        if refused_for_grant(&r) {
                            *grant_wanted = true;
                        }
                        continue;
                    }
                    return match r.result {
                        Some(command_response::Result::Err(e)) => Err(e),
                        Some(command_response::Result::Ok(ok)) => Ok(ok),
                        None => Ok(CommandOk::default()),
                    };
                }
                if f.stream_id == STREAM_EVENTS && f.kind == kinds::POSITIONED_EVENT {
                    *changed = true;
                } else if f.stream_id >= agora_protocol::frame::FIRST_DYNAMIC_STREAM {
                    term_frames.push_back(f);
                }
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
        // ADR-0057: a start is decided over the request as shown (one start; grants are given in the web window).
        let message = if a.action == "agent.start" {
            crate::device::start_message(node, local, approved, 0, false, &a.detail)
        } else {
            approval_message(node, local, approved)
        };
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
        let term_frames = &mut self.term_frames;
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
                } else if f.stream_id == STREAM_EVENTS && f.kind == kinds::POSITIONED_EVENT {
                    *changed = true;
                } else if f.stream_id >= agora_protocol::frame::FIRST_DYNAMIC_STREAM {
                    term_frames.push_back(f);
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
        let queue = &mut self.term_frames;
        let grant_wanted = &mut self.grant_wanted;
        self.rt.block_on(async {
            match tokio::time::timeout(wait, conn.recv()).await {
                Err(_) => TermEvent::Quiet,
                Ok(Ok(Some(f))) if f.stream_id == stream => event(&f).unwrap_or(TermEvent::Quiet),
                Ok(Ok(Some(f))) => {
                    if f.stream_id == STREAM_EVENTS && f.kind == kinds::POSITIONED_EVENT {
                        *changed = true;
                    } else if (f.stream_id, f.kind) == (STREAM_COMMANDS, 1) {
                        // The answer to something typed without waiting: only a missing grant matters.
                        if CommandResponse::decode(f.payload.as_slice())
                            .is_ok_and(|r| refused_for_grant(&r))
                        {
                            *grant_wanted = true;
                        }
                    } else if f.stream_id >= agora_protocol::frame::FIRST_DYNAMIC_STREAM {
                        queue.push_back(f);
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
        let queue = &mut self.term_frames;
        self.rt.block_on(async {
            match tokio::time::timeout(wait, conn.recv()).await {
                Err(_) => ChatEvent::Quiet,
                Ok(Ok(Some(f))) if f.stream_id == stream => take(&f).unwrap_or(ChatEvent::Quiet),
                Ok(Ok(Some(f))) => {
                    if f.stream_id == STREAM_EVENTS && f.kind == kinds::POSITIONED_EVENT {
                        *changed = true;
                    } else if f.stream_id >= agora_protocol::frame::FIRST_DYNAMIC_STREAM {
                        queue.push_back(f);
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
            .filter(|k| k.installed)
            .map(|k| {
                let name = if k.display_name.is_empty() {
                    k.kind.clone()
                } else {
                    k.display_name
                };
                (k.kind, name)
            })
            .collect())
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

    /// Keys typed into the terminal (an ordinary routed command; the machine runs them in the order sent). Fire and forget.
    pub fn terminal_input(&mut self, node: &str, terminal_id: &str, data: &[u8]) {
        self.send_nowait(
            node,
            command_request::Command::TerminalInput(agora_protocol::v1::TerminalInput {
                terminal_id: terminal_id.to_string(),
                data: data.to_vec(),
            }),
        );
    }

    /// Was something typed refused because the machine wants a grant first (ADR-0041)? Reading it clears it.
    pub fn take_grant_wanted(&mut self) -> bool {
        std::mem::take(&mut self.grant_wanted)
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

    fn send_nowait(&mut self, node: &str, command: command_request::Command) {
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

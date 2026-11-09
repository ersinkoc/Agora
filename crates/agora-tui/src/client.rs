//! The hub client of the terminal console (ADR-0043): sign in with a one-time code, then follow the hub's snapshot. It speaks the
//! same protocol as the browser (ADR-0027) and trusts only the hub's own CA.
//!
//! Every call blocks the calling thread (a small runtime is built per call), so the terminal loop stays plain code.

use std::sync::Arc;
use std::time::Duration;

use crate::device::{approval_message, Device};
use agora_protocol::frame::{kinds, STREAM_CLOSE, STREAM_COMMANDS, STREAM_EVENTS};
use agora_protocol::v1::{
    command_request, command_response, CommandRequest, CommandResponse, Snapshot,
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

/// The name the hub's certificate must carry: its host, or `localhost` when the address is an IP number (the hub's certificate
/// always names `localhost`).
fn server_name(addr: &str) -> Result<ServerName<'static>, ClientError> {
    let host = addr
        .rsplit_once(':')
        .map_or(addr, |(h, _)| h)
        .trim_matches(['[', ']']);
    let name = if host.parse::<std::net::IpAddr>().is_ok() {
        "localhost"
    } else {
        host
    };
    ServerName::try_from(name.to_string()).map_err(|_| ClientError::Unreachable)
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
    let tcp = tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(&hub.addr))
        .await
        .map_err(|_| ClientError::Unreachable)?
        .map_err(|_| ClientError::Unreachable)?;
    tokio::time::timeout(
        Duration::from_secs(10),
        TlsConnector::from(Arc::new(cfg)).connect(server_name(&hub.addr)?, tcp),
    )
    .await
    .map_err(|_| ClientError::Unreachable)?
    .map_err(|_| ClientError::Unreachable)
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
        })
    }

    /// Waits up to `wait` for the hub to say something.
    pub fn next(&mut self, wait: Duration) -> Update {
        if std::mem::take(&mut self.changed) {
            return Update::Changed;
        }
        let conn = &mut self.conn;
        self.rt.block_on(async {
            match tokio::time::timeout(wait, conn.recv()).await {
                Err(_) => Update::Quiet,
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
                stream_id: STREAM_COMMANDS,
                kind: 0,
                seq: None,
                payload: req.encode_to_vec(),
            })
            .await
            .map_err(|_| "client.unreachable".to_string())?;
            let end = tokio::time::Instant::now() + wait;
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
                    if r.request_id != id {
                        continue;
                    }
                    return match r.result {
                        Some(command_response::Result::Err(e)) => Err(e.code),
                        _ => Ok(()),
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
        let Some(a) = snapshot.approvals.first() else {
            return Ok(false);
        };
        // Hub-wide ids are `<node>/<local id>`; the machine knows the local one.
        let (node, local) = a
            .approval_id
            .split_once('/')
            .unwrap_or(("", a.approval_id.as_str()));
        let signature = device
            .sign(&approval_message(node, local, approved))
            .ok_or_else(|| "client.sign".to_string())?;
        let cmd = command_request::Command::ApprovalResolve(agora_protocol::v1::ApprovalResolve {
            approval_id: local.to_string(),
            approved,
            signer_device_id: device.id.clone(),
            signature,
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
        self.rt.block_on(async {
            match tokio::time::timeout(wait, conn.recv()).await {
                Err(_) => TermEvent::Quiet,
                Ok(Ok(Some(f))) if f.stream_id == stream => event(&f).unwrap_or(TermEvent::Quiet),
                Ok(Ok(Some(f))) => {
                    if f.stream_id == STREAM_EVENTS && f.kind == kinds::POSITIONED_EVENT {
                        *changed = true;
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

    /// Stops an agent on a machine.
    pub fn stop_agent(&mut self, node: &str, agent_id: &str) -> Result<(), String> {
        self.command(
            node,
            command_request::Command::AgentStop(agora_protocol::v1::AgentStop {
                agent_id: agent_id.to_string(),
            }),
            Duration::from_secs(15),
        )
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

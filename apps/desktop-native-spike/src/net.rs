//! The hub connection of the spike, on its own thread (ADR-0099 decision 3 says a background thread that posts to GPUI's main
//! thread). Sign-in is `agora_tui::client::sign_in`; the WebSocket is `agora_protocol::ws` over TLS that trusts only the hub's CA,
//! the way `agora-tui`'s client does it (its `tls` is private, so the same few lines are repeated here; stage 1 moves both into the
//! shared crate).
//!
//! Terminal output is fed into each terminal's emulator **on this thread**, under a short lock, so the main thread only draws: a
//! 10 MB/s flood never parses on the UI thread. The main thread is woken once per batch (a coalesced flag), not once per frame.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agora_protocol::frame::{
    kinds, FIRST_DYNAMIC_STREAM, STREAM_CLOSE, STREAM_COMMANDS, STREAM_EVENTS,
};
use agora_protocol::v1::{
    command_request, command_response, CommandRequest, CommandResponse, Snapshot,
};
use agora_protocol::ws::connect_with_token;
use agora_protocol::Frame;
use agora_tui::termio::ModeScanner;
use agora_vt::{AlacrittyTerminal, Size, Terminal};
use prost::Message;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName};
use tokio::sync::mpsc;
use tokio_rustls::TlsConnector;

/// The node's "fell behind" kind on a terminal stream (`agora-tui` keeps the same private constant).
const KIND_BEHIND: u8 = 200;
/// History kept per terminal: the default scrollback of the web client.
pub const SCROLLBACK: usize = 10_000;

/// One agent from the snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentRow {
    /// `<node>/<local id>`.
    pub agent_id: String,
    pub kind: String,
    pub running: bool,
}

/// The emulator and what the spike tracks beside it, shared by the network thread (writer) and the UI (reader).
pub struct TermState {
    pub term: AlacrittyTerminal,
    pub scanner: ModeScanner,
    /// Bumped on every chunk fed.
    pub generation: u64,
    pub bytes: u64,
    /// When the first chunk after the last [`TermState::mark_sent`] arrived (echo measurement).
    pub first_after_send: Option<Instant>,
    pub send_mark: bool,
    pub got_screen: bool,
    pub ended: bool,
    /// Times the node said this viewer fell behind (output dropped) and the terminal was watched again.
    pub behind: u64,
}

impl TermState {
    pub fn mark_sent(&mut self) {
        self.send_mark = true;
        self.first_after_send = None;
    }
}

pub struct TermShared {
    pub state: Mutex<TermState>,
    /// A wake-up for the UI is already on its way.
    pub dirty: AtomicBool,
    pub stream: AtomicU64,
    pub node: String,
    pub terminal_id: String,
    pub agent_id: String,
    /// Time spent parsing on the network thread (ns), for the report.
    pub parse_ns: AtomicU64,
}

impl TermShared {
    pub fn lock(&self) -> std::sync::MutexGuard<'_, TermState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// What the UI asks of the network thread.
pub enum Cmd {
    Open(Arc<TermShared>),
    Input {
        node: String,
        terminal_id: String,
        data: Vec<u8>,
    },
    Resize {
        node: String,
        terminal_id: String,
        cols: u16,
        rows: u16,
    },
}

/// What the network thread tells the UI.
pub enum UiEvent {
    Agents(Vec<AgentRow>),
    /// A terminal has new content to draw (sent once until the UI clears the flag).
    Dirty(Arc<TermShared>),
    /// A terminal's stream opened (or failed to: the error code).
    Opened(Arc<TermShared>, Result<(), String>),
    Lost(String),
}

pub struct Hub {
    pub addr: String,
    pub ca_pem: String,
}

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

async fn tls(hub: &Hub) -> Result<tokio_rustls::client::TlsStream<tokio::net::TcpStream>, String> {
    let mut roots = rustls::RootCertStore::empty();
    for c in CertificateDer::pem_slice_iter(hub.ca_pem.as_bytes()) {
        roots
            .add(c.map_err(|_| "ca".to_string())?)
            .map_err(|_| "ca".to_string())?;
    }
    let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|_| "tls".to_string())?
    .with_root_certificates(roots)
    .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(cfg));
    for name in server_names(&hub.addr) {
        let tcp = agora_protocol::dial::connect(&hub.addr)
            .await
            .map_err(|_| "unreachable".to_string())?;
        let _ = tcp.set_nodelay(true);
        if let Ok(s) = connector.connect(name, tcp).await {
            return Ok(s);
        }
    }
    Err("tls".to_string())
}

fn rows_of(s: &Snapshot) -> Vec<AgentRow> {
    let mut v: Vec<AgentRow> = s
        .agents
        .iter()
        .map(|a| AgentRow {
            agent_id: a.agent_id.clone(),
            kind: a.kind.clone(),
            running: a.running,
        })
        .collect();
    v.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
    v
}

/// Makes a terminal record for an agent (`<node>/<local>`, terminal `t-<local>`, as `agora-tui`'s preview does).
pub fn new_term(agent_id: &str) -> Arc<TermShared> {
    let (node, local) = agent_id.split_once('/').unwrap_or(("", agent_id));
    Arc::new(TermShared {
        state: Mutex::new(TermState {
            term: AlacrittyTerminal::with_scrollback(
                Size {
                    cols: 100,
                    rows: 30,
                },
                SCROLLBACK,
            ),
            scanner: ModeScanner::default(),
            generation: 0,
            bytes: 0,
            first_after_send: None,
            send_mark: false,
            got_screen: false,
            ended: false,
            behind: 0,
        }),
        dirty: AtomicBool::new(false),
        stream: AtomicU64::new(0),
        node: node.to_string(),
        terminal_id: format!("t-{local}"),
        agent_id: agent_id.to_string(),
        parse_ns: AtomicU64::new(0),
    })
}

/// Rows of a repaint (`ESC [ n ; 1 H` at the start of each row), as `agora-tui`'s preview reads them.
fn rows_in(screen: &[u8]) -> u16 {
    let mut rows = 0u16;
    let mut i = 0;
    while let Some(off) = screen[i..].windows(2).position(|w| w == b"\x1b[") {
        let start = i + off + 2;
        let digits = screen[start..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        if screen[start + digits..].starts_with(b";1H") {
            if let Some(n) = std::str::from_utf8(&screen[start..start + digits])
                .ok()
                .and_then(|s| s.parse::<u16>().ok())
            {
                rows = rows.max(n);
            }
        }
        i = start;
    }
    rows
}

/// Runs the connection until the hub closes it. `wake` posts an event to the UI thread.
pub fn run(
    hub: Hub,
    token: String,
    mut cmds: mpsc::UnboundedReceiver<Cmd>,
    wake: impl Fn(UiEvent) + Send + 'static,
) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(_) => {
            wake(UiEvent::Lost("runtime".into()));
            return;
        }
    };
    rt.block_on(async move {
        let host = hub.addr.rsplit_once(':').map_or(hub.addr.as_str(), |(h, _)| h).to_string();
        let stream = match tls(&hub).await {
            Ok(s) => s,
            Err(e) => {
                wake(UiEvent::Lost(e));
                return;
            }
        };
        let mut conn = match connect_with_token(stream, &format!("wss://{host}/"), &token).await {
            Ok(c) => c,
            Err(_) => {
                wake(UiEvent::Lost("refused".into()));
                return;
            }
        };
        let subscribe = Frame {
            stream_id: STREAM_EVENTS,
            kind: kinds::SUBSCRIBE,
            seq: None,
            payload: agora_protocol::v1::Subscribe { after_pos: None }.encode_to_vec(),
        };
        if conn.send(&subscribe).await.is_err() {
            wake(UiEvent::Lost("unreachable".into()));
            return;
        }
        let mut requests: u64 = 0;
        let mut next_stream = FIRST_DYNAMIC_STREAM;
        let mut by_stream: HashMap<u32, Arc<TermShared>> = HashMap::new();
        // Open requests waiting for their answer: request id -> terminal.
        let mut opening: HashMap<String, Arc<TermShared>> = HashMap::new();
        let mut last_resub = Instant::now();
        let mut changed = false;
        loop {
            // Input first: everything the UI queued is sent before the next frame from the hub is read (input outranks output).
            tokio::select! {
                biased;
                c = cmds.recv() => {
                    let Some(c) = c else { break };
                    let mut batch = vec![c];
                    while let Ok(more) = cmds.try_recv() { batch.push(more); }
                    for c in batch {
                        let (stream_id, node, command) = match c {
                            Cmd::Input { node, terminal_id, data } => (STREAM_COMMANDS, node, command_request::Command::TerminalInput(
                                agora_protocol::v1::TerminalInput { terminal_id, data })),
                            Cmd::Resize { node, terminal_id, cols, rows } => (STREAM_COMMANDS, node, command_request::Command::TerminalResize(
                                agora_protocol::v1::TerminalResize { terminal_id, cols: u32::from(cols), rows: u32::from(rows) })),
                            Cmd::Open(t) => {
                                let s = next_stream;
                                next_stream += 1;
                                t.stream.store(u64::from(s), Ordering::SeqCst);
                                by_stream.insert(s, t.clone());
                                requests += 1;
                                opening.insert(format!("n{requests}"), t.clone());
                                (s, t.node.clone(), command_request::Command::TerminalSubscribe(
                                    agora_protocol::v1::TerminalSubscribe { terminal_id: t.terminal_id.clone(), format: String::new() }))
                            }
                        };
                        if !matches!(command, command_request::Command::TerminalSubscribe(_)) {
                            requests += 1;
                        }
                        let req = CommandRequest {
                            request_id: format!("n{requests}"),
                            idempotency_key: String::new(),
                            target_node: node,
                            command: Some(command),
                        };
                        if conn.send(&Frame { stream_id, kind: 0, seq: None, payload: req.encode_to_vec() }).await.is_err() {
                            wake(UiEvent::Lost("unreachable".into()));
                            return;
                        }
                    }
                }
                f = conn.recv() => {
                    let f = match f {
                        Ok(Some(f)) => f,
                        _ => { wake(UiEvent::Lost("closed".into())); return; }
                    };
                    if f.stream_id >= FIRST_DYNAMIC_STREAM {
                        let Some(t) = by_stream.get(&f.stream_id).cloned() else { continue };
                        if f.kind == KIND_BEHIND {
                            // The node dropped output for this viewer: watch again (a fresh screen follows), as agora-tui does.
                            t.lock().behind += 1;
                            by_stream.remove(&f.stream_id);
                            let _ = conn.send(&Frame { stream_id: f.stream_id, kind: STREAM_CLOSE, seq: None, payload: Vec::new() }).await;
                            let s = next_stream;
                            next_stream += 1;
                            t.stream.store(u64::from(s), Ordering::SeqCst);
                            by_stream.insert(s, t.clone());
                            requests += 1;
                            let req = CommandRequest {
                                request_id: format!("n{requests}"),
                                idempotency_key: String::new(),
                                target_node: t.node.clone(),
                                command: Some(command_request::Command::TerminalSubscribe(agora_protocol::v1::TerminalSubscribe {
                                    terminal_id: t.terminal_id.clone(),
                                    format: String::new(),
                                })),
                            };
                            let _ = conn.send(&Frame { stream_id: s, kind: 0, seq: None, payload: req.encode_to_vec() }).await;
                            continue;
                        }
                        let started = Instant::now();
                        {
                            let mut st = t.lock();
                            match f.kind {
                                0 => {
                                    st.scanner.feed(&f.payload);
                                    st.term.feed(&f.payload);
                                    st.bytes += f.payload.len() as u64;
                                }
                                1 => {
                                    let rows = rows_in(&f.payload);
                                    if rows > 0 && rows != st.term.size().rows {
                                        let cols = st.term.size().cols;
                                        st.term.resize(Size { cols, rows });
                                    }
                                    st.scanner.feed(&f.payload);
                                    st.term.feed(&f.payload);
                                    st.got_screen = true;
                                }
                                2 if f.payload.len() == 4 => {
                                    let p = &f.payload;
                                    let size = Size { cols: u16::from_le_bytes([p[0], p[1]]), rows: u16::from_le_bytes([p[2], p[3]]) };
                                    st.term.resize(size);
                                }
                                3 | STREAM_CLOSE => st.ended = true,
                                _ => {}
                            }
                            st.generation += 1;
                            if st.send_mark && st.first_after_send.is_none() {
                                st.first_after_send = Some(Instant::now());
                            }
                        }
                        t.parse_ns.fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
                        if !t.dirty.swap(true, Ordering::AcqRel) {
                            wake(UiEvent::Dirty(t));
                        }
                        continue;
                    }
                    if f.stream_id == STREAM_COMMANDS && f.kind == 1 {
                        if let Ok(r) = CommandResponse::decode(f.payload.as_slice()) {
                            if let Some(t) = opening.remove(&r.request_id) {
                                let res = match r.result {
                                    Some(command_response::Result::Err(e)) => Err(e.code),
                                    _ => Ok(()),
                                };
                                wake(UiEvent::Opened(t, res));
                            }
                        }
                        continue;
                    }
                    if f.stream_id == STREAM_EVENTS && f.kind == kinds::SNAPSHOT {
                        if let Ok(s) = Snapshot::decode(f.payload.as_slice()) {
                            wake(UiEvent::Agents(rows_of(&s)));
                        }
                    } else if f.stream_id == STREAM_EVENTS && f.kind == kinds::POSITIONED_EVENT {
                        changed = true;
                    }
                    // A change is picked up by asking for a fresh snapshot at most once a second.
                    if changed && last_resub.elapsed() > Duration::from_secs(1) {
                        changed = false;
                        last_resub = Instant::now();
                        let _ = conn.send(&subscribe).await;
                    }
                }
            }
        }
    });
}

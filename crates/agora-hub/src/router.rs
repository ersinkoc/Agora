//! Routing of commands from clients to the node that owns the target (ADR-0029).
//!
//! A node's connection registers here for as long as it lives. A client's command is forwarded unchanged except for
//! its request id (the hub picks a fresh one so two clients cannot collide) and the answer is mapped back. The hub
//! never edits what a command says; the node runs it through the same handler as a local command, with the same policy.
//! The one exception is a terminal's size: several windows may watch one terminal, and the hub sends the node the smallest
//! size any of them asked for, so no window is clipped (ADR-0072, which amends ADR-0029 here; see [`Router::resize`] and
//! docs/specs/protocol.md, "Shared terminal size").

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use agora_protocol::v1::{self, command_request};
use agora_protocol::{Error, ErrorCode};
use tokio::sync::{mpsc, oneshot};

/// What the hub asks a node connection to do.
pub enum ToNode {
    /// A command with where its answer goes. `stream` is 1 (commands) or, for a terminal subscription, the dynamic
    /// stream the node must write that terminal's frames on.
    Command {
        /// Already carries the hub-chosen request id.
        request: Box<v1::CommandRequest>,
        reply: oneshot::Sender<v1::CommandResponse>,
        stream: u32,
    },
    /// End a relayed stream.
    Close { stream: u32 },
}

/// A terminal frame on its way from a node to the client that subscribed.
pub struct RelayFrame {
    pub stream: u32,
    pub kind: u8,
    pub payload: Vec<u8>,
}

/// Frames a client may have queued before its relay is closed (it resubscribes and gets a fresh snapshot).
const RELAY_QUEUE: usize = 512;

/// The most commands queued for one node connection and not yet written to it (architecture review R5). The connection's loop
/// writes them as they come, so the queue only grows while a write to the node waits (it gives up after 25 s); this bounds what
/// every client together can pile up behind such a node. A command refused for it was never sent: retrying it is safe.
pub const NODE_QUEUE_MAX: usize = 1024;

struct Link {
    owner: Arc<()>,
    capabilities: Vec<String>,
    tx: mpsc::UnboundedSender<ToNode>,
    load: Arc<LinkLoad>,
}

/// How much one node connection has to do (`GET /api/metrics`, architecture review R8). Kept with the connection, so a connection
/// that ended or was replaced stops counting at once, whatever it still held.
#[derive(Debug, Default)]
pub struct LinkLoad {
    /// Commands queued for the node and not yet written to it. A node that does not read its connection grows it, up to
    /// `NODE_QUEUE_MAX`: the next command is refused (`node.resource_limit`) before it is queued.
    pub queued: AtomicUsize,
    /// Commands written to the node and not answered yet.
    pub unanswered: AtomicUsize,
}

struct Relay {
    client: ClientId,
    client_stream: u32,
    /// The terminal this relay shows; `None` for a chat.
    terminal: Option<String>,
    to_client: mpsc::Sender<RelayFrame>,
    closed: Arc<Mutex<Vec<u32>>>,
}

#[derive(Clone)]
struct ClientId(Arc<()>);

impl PartialEq for ClientId {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for ClientId {}

impl Hash for ClientId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::ptr::hash(Arc::as_ptr(&self.0), state);
    }
}

/// A terminal size: `(cols, rows)`.
pub type Size = (u16, u16);

/// The largest terminal the node accepts (`agora-node` control, `TerminalResize`); a bigger window asks for this.
pub const MAX_COLS: u16 = 1000;
pub const MAX_ROWS: u16 = 1000;

/// What the hub does with a client's `TerminalResize`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resize {
    /// Forward it with this size (the smallest of the windows watching the terminal).
    Forward(Size),
    /// The terminal already has the size the windows watching it need: answered by the hub, nothing is sent.
    Unchanged,
    /// Nobody watches the terminal through the hub: forwarded as it is, as before shared sizes.
    PassThrough,
}

/// The sizes windows asked for one terminal, and the size the hub last knew it to have.
#[derive(Default)]
struct Shared {
    /// Client connection → the size it last asked for. Counts only while that client has a relay for the terminal.
    asked: HashMap<ClientId, Size>,
    /// The size the hub last sent the node or saw in the terminal's own resize frame; `None` when unknown.
    applied: Option<Size>,
}

/// The smallest size over the clients in `viewers` that asked for one: the minimum of the columns and, separately, of the rows.
fn smallest(asked: &HashMap<ClientId, Size>, viewers: &HashSet<ClientId>) -> Option<Size> {
    asked
        .iter()
        .filter(|(c, _)| viewers.contains(c))
        .map(|(_, s)| *s)
        .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1)))
}

pub struct Router {
    nodes: Mutex<HashMap<String, Link>>,
    /// `(node, stream on the node's connection)` → the client stream it feeds. Locked before `sizes` when both are held.
    relays: Mutex<HashMap<(String, u32), Relay>>,
    /// `(node, terminal id)` → the sizes the windows watching it asked for.
    sizes: Mutex<HashMap<(String, String), Shared>>,
    next: AtomicU64,
    request_epoch: Mutex<u64>,
    next_stream: AtomicU32,
    /// Terminal relays ended because the window's queue was full or gone (`GET /api/metrics`). Chat relays are not counted.
    relays_dropped: AtomicU64,
    /// Bytes of terminal frames passed on to windows (`GET /api/metrics`; a rate of it is the terminals' bytes per second). Chat
    /// frames are not counted.
    terminal_bytes: AtomicU64,
}

impl Default for Router {
    fn default() -> Self {
        Self {
            nodes: Mutex::default(),
            relays: Mutex::default(),
            sizes: Mutex::default(),
            next: AtomicU64::new(0),
            request_epoch: Mutex::new(0),
            next_stream: AtomicU32::new(agora_protocol::frame::FIRST_DYNAMIC_STREAM),
            relays_dropped: AtomicU64::new(0),
            terminal_bytes: AtomicU64::new(0),
        }
    }
}

/// One client connection's end of the relays: terminal frames arrive on `frames`, and `closed` lists client streams the
/// hub ended (the node left, the client was too slow, the terminal exited). Dropping it ends all its relays.
pub struct ClientRelays {
    router: Arc<Router>,
    id: ClientId,
    tx: mpsc::Sender<RelayFrame>,
    closed: Arc<Mutex<Vec<u32>>>,
}

impl ClientRelays {
    /// Client streams the hub has ended since the last call.
    pub fn take_closed(&self) -> Vec<u32> {
        std::mem::take(&mut *self.closed.lock().unwrap_or_else(|e| e.into_inner()))
    }

    /// The client ended one of its streams.
    pub fn close(&self, client_stream: u32) {
        self.router
            .close_client_stream(&self.id, Some(client_stream));
    }
}

impl Drop for ClientRelays {
    fn drop(&mut self) {
        self.router.close_client_stream(&self.id, None);
    }
}

/// Keeps a node registered; dropping it (the connection ended) unregisters it, unless a newer connection of the same
/// node has replaced it in the meantime.
pub struct Registration {
    router: Arc<Router>,
    node: String,
    owner: Arc<()>,
    load: Arc<LinkLoad>,
}

impl Registration {
    /// This connection's load, for the connection's own loop to keep up to date.
    pub fn load(&self) -> Arc<LinkLoad> {
        Arc::clone(&self.load)
    }

    /// Ends the registration now. Returns whether this was still the node's current connection: `false` when a newer
    /// connection of the same node replaced it, which then stays registered.
    pub fn end(self) -> bool {
        self.release()
    }

    /// The node this registration is for.
    pub fn node(&self) -> &str {
        &self.node
    }

    fn release(&self) -> bool {
        let ours = {
            let mut g = self.router.nodes.lock().unwrap_or_else(|e| e.into_inner());
            let ours = g
                .get(&self.node)
                .is_some_and(|l| Arc::ptr_eq(&l.owner, &self.owner));
            if ours {
                g.remove(&self.node);
            }
            ours
        };
        // The terminals this node was relaying end with it.
        if ours {
            self.router.end_node_relays(&self.node);
        }
        ours
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        // After `end` this finds nothing of its own left and does nothing.
        self.release();
    }
}

impl Router {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Registers a connected node. The receiver yields the commands to forward to it.
    pub fn register(
        self: &Arc<Self>,
        node: &str,
    ) -> (Registration, mpsc::UnboundedReceiver<ToNode>) {
        self.register_with_capabilities(node, vec![])
    }

    /// Registers the capabilities negotiated on this particular node connection.
    pub fn register_with_capabilities(
        self: &Arc<Self>,
        node: &str,
        capabilities: Vec<String>,
    ) -> (Registration, mpsc::UnboundedReceiver<ToNode>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let owner = Arc::new(());
        let load = Arc::new(LinkLoad::default());
        let replaced = self
            .nodes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                node.to_string(),
                Link {
                    owner: Arc::clone(&owner),
                    tx,
                    capabilities,
                    load: Arc::clone(&load),
                },
            )
            .is_some();
        // The terminals the replaced connection was relaying end now: the new connection never writes on their streams,
        // and the clients resubscribe through it.
        if replaced {
            self.end_node_relays(node);
        }
        (
            Registration {
                router: Arc::clone(self),
                node: node.to_string(),
                owner,
                load,
            },
            rx,
        )
    }

    /// A client connection's relay endpoint, and the queue its terminal frames arrive on.
    pub fn client(self: &Arc<Self>) -> (ClientRelays, mpsc::Receiver<RelayFrame>) {
        let (tx, rx) = mpsc::channel(RELAY_QUEUE);
        (
            ClientRelays {
                router: Arc::clone(self),
                id: ClientId(Arc::new(())),
                tx,
                closed: Arc::default(),
            },
            rx,
        )
    }

    /// Reserves the stream a node will write a client's terminal (or, with `terminal` `None`, chat) on. Frames the node writes
    /// on it are queued for the client under `client_stream`. A window that comes back to a terminal counts for its size again.
    pub fn open_relay(
        &self,
        node: &str,
        client: &ClientRelays,
        client_stream: u32,
        terminal: Option<&str>,
    ) -> u32 {
        let mut relays = self.relays.lock().unwrap_or_else(|e| e.into_inner());
        let first = agora_protocol::frame::FIRST_DYNAMIC_STREAM;
        let mut stream = self.next_stream.load(Ordering::Relaxed).max(first);
        while relays.keys().any(|(n, s)| n == node && *s == stream) {
            stream = if stream == u32::MAX {
                first
            } else {
                stream + 1
            };
        }
        self.next_stream.store(
            if stream == u32::MAX {
                first
            } else {
                stream + 1
            },
            Ordering::Relaxed,
        );
        relays.insert(
            (node.to_string(), stream),
            Relay {
                client: client.id.clone(),
                client_stream,
                terminal: terminal.map(str::to_string),
                to_client: client.tx.clone(),
                closed: Arc::clone(&client.closed),
            },
        );
        drop(relays);
        if let Some(t) = terminal {
            self.reshape(node, t);
        }
        stream
    }

    /// A client's `TerminalResize` for `terminal` on `node` (tmux's "smallest" policy). The hub remembers the size of a client
    /// that watches the terminal (has a relay for it) and forwards the smallest columns and the smallest rows over the clients
    /// watching it, only when that differs from what the terminal has. A client that does not watch the terminal has no say
    /// while others watch it (it does not see the screen), and is passed through as before when nobody does (a script).
    /// A zero size is passed through for the node to refuse, as before; a size beyond the node's limits asks for the limit.
    pub fn resize(
        &self,
        client: &ClientRelays,
        node: &str,
        terminal: &str,
        cols: u32,
        rows: u32,
    ) -> Resize {
        if cols == 0 || rows == 0 {
            return Resize::PassThrough;
        }
        let size = (
            u16::try_from(cols).unwrap_or(u16::MAX).min(MAX_COLS),
            u16::try_from(rows).unwrap_or(u16::MAX).min(MAX_ROWS),
        );
        let relays = self.relays.lock().unwrap_or_else(|e| e.into_inner());
        let viewers = viewers(&relays, node, terminal);
        let mut sizes = self.sizes.lock().unwrap_or_else(|e| e.into_inner());
        let key = (node.to_string(), terminal.to_string());
        if !viewers.contains(&client.id) {
            let watched = sizes
                .get(&key)
                .and_then(|s| smallest(&s.asked, &viewers))
                .is_some();
            if watched {
                return Resize::Unchanged;
            }
            // The node gets a size the hub did not choose: from now on it is unknown.
            if let Some(s) = sizes.get_mut(&key) {
                s.applied = None;
            }
            return Resize::PassThrough;
        }
        let shared = sizes.entry(key).or_default();
        shared.asked.insert(client.id.clone(), size);
        match smallest(&shared.asked, &viewers) {
            Some(min) if shared.applied != Some(min) => {
                shared.applied = Some(min);
                Resize::Forward(min)
            }
            _ => Resize::Unchanged,
        }
    }

    /// The set of windows watching `terminal` changed: sends the node the new smallest size when it differs from what the
    /// terminal has. Nothing is sent when no watching window has asked for a size (the terminal keeps the one it has).
    fn reshape(&self, node: &str, terminal: &str) {
        let send = {
            let relays = self.relays.lock().unwrap_or_else(|e| e.into_inner());
            let viewers = viewers(&relays, node, terminal);
            let mut sizes = self.sizes.lock().unwrap_or_else(|e| e.into_inner());
            match sizes.get_mut(&(node.to_string(), terminal.to_string())) {
                Some(shared) => match smallest(&shared.asked, &viewers) {
                    Some(min) if shared.applied != Some(min) => {
                        shared.applied = Some(min);
                        Some(min)
                    }
                    _ => None,
                },
                None => None,
            }
        };
        if let Some((cols, rows)) = send {
            let req = v1::CommandRequest {
                request_id: String::new(),
                idempotency_key: String::new(),
                target_node: String::new(),
                command: Some(command_request::Command::TerminalResize(
                    v1::TerminalResize {
                        terminal_id: terminal.to_string(),
                        cols: cols.into(),
                        rows: rows.into(),
                    },
                )),
            };
            // Nobody waits for this answer: a terminal that is gone stays gone, and the windows see the node's resize frame.
            let _ = self.send(node, req);
        }
    }

    /// How many terminal relays ended because the window could not keep up (or was gone).
    pub fn relays_dropped(&self) -> u64 {
        self.relays_dropped.load(Ordering::Relaxed)
    }

    /// Commands queued for nodes and not yet written, and commands written and not answered yet, over every node connected now.
    pub fn node_load(&self) -> (usize, usize) {
        let nodes = self.nodes.lock().unwrap_or_else(|e| e.into_inner());
        nodes.values().fold((0, 0), |(q, u), l| {
            (
                q.saturating_add(l.load.queued.load(Ordering::Relaxed)),
                u.saturating_add(l.load.unanswered.load(Ordering::Relaxed)),
            )
        })
    }

    /// Bytes of terminal frames passed on to windows so far.
    pub fn terminal_bytes(&self) -> u64 {
        self.terminal_bytes.load(Ordering::Relaxed)
    }

    /// A frame from a node on a dynamic stream: queued for the client. A client whose queue is full is not waited for;
    /// its relay is closed on both sides and it resubscribes. A terminal exit ends the relay after the frame.
    pub fn relay_frame(&self, node: &str, stream: u32, kind: u8, payload: Vec<u8>) {
        let key = (node.to_string(), stream);
        let mut relays = self.relays.lock().unwrap_or_else(|e| e.into_inner());
        let Some(r) = relays.get(&key) else {
            return;
        };
        // The terminal's own size (kind 2, rare) and its end (kind 3) keep the shared-size book right: a size set by a local
        // client is known, so the next change of watchers sends what they need. Output (kind 0) does not get here.
        if kind == 2 || kind == 3 {
            if let Some(t) = &r.terminal {
                let mut sizes = self.sizes.lock().unwrap_or_else(|e| e.into_inner());
                let k = (node.to_string(), t.clone());
                if kind == 3 {
                    sizes.remove(&k);
                } else if let ([c0, c1, r0, r1], Some(s)) = (payload.as_slice(), sizes.get_mut(&k))
                {
                    s.applied = Some((
                        u16::from_le_bytes([*c0, *c1]),
                        u16::from_le_bytes([*r0, *r1]),
                    ));
                }
            }
        }
        if kind == agora_protocol::frame::STREAM_CLOSE {
            if let Some(r) = relays.remove(&key) {
                r.closed
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(r.client_stream);
                drop(relays);
                if let Some(t) = r.terminal {
                    self.reshape(node, &t);
                }
            }
            return;
        }
        let bytes = payload.len() as u64;
        // Only terminal relays count for the terminal metrics (#574 review: a chat snapshot is not terminal output).
        let terminal = r.terminal.is_some();
        let sent = r.to_client.try_send(RelayFrame {
            stream: r.client_stream,
            kind,
            payload,
        });
        if sent.is_ok() && terminal {
            crate::metrics::saturating_add(&self.terminal_bytes, bytes);
        }
        let exited = kind == 3;
        if sent.is_err() || exited {
            if sent.is_err() && terminal {
                crate::metrics::saturating_add(&self.relays_dropped, 1);
            }
            if let Some(r) = relays.remove(&key) {
                if sent.is_err() {
                    r.closed
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(r.client_stream);
                    drop(relays);
                    self.tell_node(node, ToNode::Close { stream });
                    if let Some(t) = r.terminal {
                        self.reshape(node, &t);
                    }
                }
            }
        }
    }

    /// Ends a relay whose subscription the node refused, or whose command could not be delivered.
    pub fn drop_relay(&self, node: &str, stream: u32) {
        let gone = self
            .relays
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&(node.to_string(), stream));
        if let Some(t) = gone.and_then(|r| r.terminal) {
            self.reshape(node, &t);
        }
    }

    fn tell_node(&self, node: &str, msg: ToNode) {
        if let Some(l) = self
            .nodes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(node)
        {
            let _ = l.tx.send(msg);
        }
    }

    /// Ends one client stream (or all of a client's) and tells the nodes to stop writing on them. A window that stopped watching
    /// a terminal (or left) no longer counts for its size: the others get the size they need, which may be bigger.
    fn close_client_stream(&self, client: &ClientId, only: Option<u32>) {
        let (ended, left) = {
            let mut relays = self.relays.lock().unwrap_or_else(|e| e.into_inner());
            let keys: Vec<_> = relays
                .iter()
                .filter(|(_, r)| &r.client == client && only.is_none_or(|s| s == r.client_stream))
                .map(|(k, _)| k.clone())
                .collect();
            let mut left: Vec<(String, String)> = Vec::new();
            for k in &keys {
                if let Some(t) = relays.remove(k).and_then(|r| r.terminal) {
                    left.push((k.0.clone(), t));
                }
            }
            left.sort();
            left.dedup();
            // Its size is forgotten where it no longer watches (a second view of the same terminal keeps it), or everywhere when
            // the connection ended.
            let mut sizes = self.sizes.lock().unwrap_or_else(|e| e.into_inner());
            match only {
                None => {
                    for s in sizes.values_mut() {
                        s.asked.remove(client);
                    }
                }
                Some(_) => {
                    for (node, t) in &left {
                        if !viewers(&relays, node, t).contains(client) {
                            if let Some(s) = sizes.get_mut(&(node.clone(), t.clone())) {
                                s.asked.remove(client);
                            }
                        }
                    }
                }
            }
            // A terminal nobody asked a size for is forgotten (the book stays as small as the terminals being watched).
            sizes.retain(|_, s| !s.asked.is_empty());
            (keys, left)
        };
        for (node, stream) in ended {
            self.tell_node(&node, ToNode::Close { stream });
        }
        for (node, t) in left {
            self.reshape(&node, &t);
        }
    }

    fn end_node_relays(&self, node: &str) {
        let mut relays = self.relays.lock().unwrap_or_else(|e| e.into_inner());
        let keys: Vec<_> = relays.keys().filter(|(n, _)| n == node).cloned().collect();
        for k in keys {
            if let Some(r) = relays.remove(&k) {
                r.closed
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(r.client_stream);
            }
        }
        // A node that comes back may have new terminals of any size: what the hub knew of them is not true any more. The sizes
        // windows asked for stay, and count again when they subscribe through the new connection.
        for (k, s) in self
            .sizes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter_mut()
        {
            if k.0 == node {
                s.applied = None;
            }
        }
    }

    /// The connected nodes that negotiated `capability`, sorted.
    pub fn nodes_with(&self, capability: &str) -> Vec<String> {
        let mut out: Vec<String> = self
            .nodes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(_, l)| l.capabilities.iter().any(|c| c == capability))
            .map(|(n, _)| n.clone())
            .collect();
        out.sort();
        out
    }

    pub(crate) fn has_capability(&self, node: &str, capability: &str) -> bool {
        self.nodes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(node)
            .is_some_and(|link| link.capabilities.iter().any(|cap| cap == capability))
    }

    pub fn is_connected(&self, node: &str) -> bool {
        self.nodes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(node)
    }

    /// Forwards `request` to `node` under a fresh request id; returns the id and where the answer will arrive.
    /// A node that is not connected (or whose connection just ended) is `node.offline`.
    pub fn send(
        &self,
        node: &str,
        request: v1::CommandRequest,
    ) -> Result<oneshot::Receiver<v1::CommandResponse>, Error> {
        self.send_on(node, request, agora_protocol::frame::STREAM_COMMANDS)
    }

    /// Like [`send`](Self::send), with the stream the node answers a terminal subscription on.
    pub fn send_on(
        &self,
        node: &str,
        mut request: v1::CommandRequest,
        stream: u32,
    ) -> Result<oneshot::Receiver<v1::CommandResponse>, Error> {
        let offline = || Error::new(ErrorCode::NodeOffline).with("node_id", node);
        let mut epoch = self.request_epoch.lock().unwrap_or_else(|e| e.into_inner());
        let sequence = self.next.fetch_add(1, Ordering::Relaxed);
        request.request_id = if *epoch == 0 {
            format!("h{sequence}")
        } else {
            format!("h{}-{sequence}", *epoch)
        };
        if sequence == u64::MAX {
            *epoch = epoch.saturating_add(1);
        }
        drop(epoch);
        request.target_node.clear();
        let (reply, rx) = oneshot::channel();
        let g = self.nodes.lock().unwrap_or_else(|e| e.into_inner());
        let link = g.get(node).ok_or_else(offline)?;
        if matches!(
            request.command,
            Some(
                command_request::Command::DirectoryInspect(_)
                    | command_request::Command::AgentLaunch(_)
                    | command_request::Command::WorkspaceQuery(_)
            )
        ) && !link.capabilities.iter().any(|c| c == "workspace.directory")
        {
            return Err(Error::new(ErrorCode::ProtocolCapabilityMissing)
                .with("capability", "workspace.directory"));
        }
        // ADR-0053: an older node would answer `capability_missing` itself; the window shows the typed-path form instead.
        if matches!(
            request.command,
            Some(command_request::Command::DirectoryList(_))
        ) && !link.capabilities.iter().any(|c| c == "directory.list")
        {
            return Err(Error::new(ErrorCode::ProtocolCapabilityMissing)
                .with("capability", "directory.list"));
        }
        // ADR-0058: a node that cannot switch an agent's mode or model is never sent the command.
        let switch = match request.command {
            Some(command_request::Command::AgentSetMode(_)) => Some("agent.set_mode"),
            Some(command_request::Command::AgentSetModel(_)) => Some("agent.set_model"),
            // ADR-0060: a node that cannot resume agents is never sent the command.
            Some(command_request::Command::AgentResume(_)) => Some("agent.resume"),
            // ADR-0090: a node that cannot set an option or cancel a turn is never sent the command.
            Some(command_request::Command::AgentSetConfig(_)) => Some("agent.set_config"),
            Some(command_request::Command::AgentCancelTurn(_)) => Some("agent.cancel_turn"),
            // ADR-0095: a node that cannot dispatch a task is never sent the command.
            Some(command_request::Command::TaskDispatch(_)) => Some("task.dispatch"),
            // ADR-0108: only the hub passes a forget on, and only to a node that removes what it keeps of the agent.
            Some(command_request::Command::AgentForget(_)) => Some("agent.forget.local"),
            _ => None,
        };
        if let Some(cap) = switch.filter(|cap| !link.capabilities.iter().any(|c| c == cap)) {
            return Err(Error::new(ErrorCode::ProtocolCapabilityMissing).with("capability", cap));
        }
        // ADR-0061: endorsements and revocations go only to a node that verifies them.
        if matches!(
            request.command,
            Some(
                command_request::Command::DeviceEndorse(_)
                    | command_request::Command::DeviceRevoke(_)
            )
        ) && !link.capabilities.iter().any(|c| c == "device.trust")
        {
            return Err(
                Error::new(ErrorCode::ProtocolCapabilityMissing).with("capability", "device.trust")
            );
        }
        // ADR-0059: an older node would start the agent without the options it does not know (it would ask after all): refused here
        // instead, so the window says so rather than showing an agent that behaves otherwise than chosen.
        let options = match &request.command {
            Some(command_request::Command::AgentLaunch(a)) => !a.launch_options.is_empty(),
            Some(command_request::Command::AgentStart(a)) => !a.launch_options.is_empty(),
            _ => false,
        };
        if options
            && !link
                .capabilities
                .iter()
                .any(|c| c == "agent.launch_options")
        {
            return Err(Error::new(ErrorCode::ProtocolCapabilityMissing)
                .with("capability", "agent.launch_options"));
        }
        // ADR-0066: a start with settings (model, effort) goes only to a node that verifies a signature over them. An older node would
        // start the agent with its own defaults (or, before ADR-0066, ignore the field), so it is refused here instead: never
        // silently dropped.
        let settings = match &request.command {
            Some(command_request::Command::AgentLaunch(a)) => !a.settings.is_empty(),
            Some(command_request::Command::AgentStart(a)) => !a.settings.is_empty(),
            _ => false,
        };
        if settings
            && !link
                .capabilities
                .iter()
                .any(|c| c == "agent.launch_settings")
        {
            return Err(Error::new(ErrorCode::ProtocolCapabilityMissing)
                .with("capability", "agent.launch_settings"));
        }
        if matches!(
            request.command,
            Some(command_request::Command::BoardJoin(_))
        ) && !link.capabilities.iter().any(|c| c == "board.join")
        {
            return Err(
                Error::new(ErrorCode::ProtocolCapabilityMissing).with("capability", "board.join")
            );
        }
        // ADR-0109: a delivery goes only to a node that negotiated the capability; an older one keeps the best-effort path.
        if matches!(
            request.command,
            Some(command_request::Command::DeliverTaskHandoff(_))
        ) && !link
            .capabilities
            .iter()
            .any(|c| c == agora_protocol::handshake::TASK_HANDOFF_V1)
        {
            return Err(Error::new(ErrorCode::ProtocolCapabilityMissing)
                .with("capability", agora_protocol::handshake::TASK_HANDOFF_V1));
        }
        // Counted before it is queued, so the connection's loop never takes one off before it was counted; refused, without
        // queueing it, when the node already has NODE_QUEUE_MAX waiting (R5).
        if link
            .load
            .queued
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |q| {
                (q < NODE_QUEUE_MAX).then_some(q + 1)
            })
            .is_err()
        {
            return Err(Error::new(ErrorCode::NodeResourceLimit)
                .with("resource", "requests")
                .with("limit", NODE_QUEUE_MAX as i64));
        }
        if link
            .tx
            .send(ToNode::Command {
                request: Box::new(request),
                reply,
                stream,
            })
            .is_err()
        {
            link.load.queued.fetch_sub(1, Ordering::Relaxed);
            return Err(offline());
        }
        Ok(rx)
    }
}

/// The client connections that have a relay for `terminal` on `node`.
fn viewers(
    relays: &HashMap<(String, u32), Relay>,
    node: &str,
    terminal: &str,
) -> HashSet<ClientId> {
    relays
        .iter()
        .filter(|((n, _), r)| n == node && r.terminal.as_deref() == Some(terminal))
        .map(|(_, r)| r.client.clone())
        .collect()
}

/// What a client may ask the hub to forward: everything except enrollment, which only an unauthenticated peer may do
/// and which the hub answers itself.
pub fn forwardable(req: &v1::CommandRequest) -> Result<&str, Error> {
    match &req.command {
        None => Err(Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "command")),
        // `MeshPost` and `BoardCall` are what a node sends the hub (the hub writes `origin` from the certificate); a client
        // forwarding one would put its own `origin` on a message, so they are not client commands. `BoardJoin` only the hub
        // sends (ADR-0039 amendment B): from a client it would make a node hand over its local work.
        Some(
            command_request::Command::Enroll(_)
            | command_request::Command::MeshPost(_)
            | command_request::Command::BoardCall(_)
            | command_request::Command::BoardJoin(_)
            // ADR-0109: the hub's own durable handoff delivery; a client forwarding one would place work in an agent's inbox.
            | command_request::Command::DeliverTaskHandoff(_)
            | command_request::Command::AcknowledgeTaskHandoff(_)
            // ADR-0061: answered or carried by the hub itself, never forwarded as a client's command.
            | command_request::Command::DeviceRegister(_)
            | command_request::Command::DeviceList(_)
            | command_request::Command::DeviceEndorse(_)
            | command_request::Command::DeviceRevoke(_)
            | command_request::Command::DeviceAskApproval(_)
            // ADR-0065: the owner's switch and a machine's pairing messages are the hub's own, never a node's.
            | command_request::Command::PairOpen(_)
            | command_request::Command::PairClose(_)
            | command_request::Command::PairStatus(_)
            | command_request::Command::PairApprove(_)
            | command_request::Command::PairDeny(_)
            | command_request::Command::PairRequest(_)
            | command_request::Command::PairReveal(_)
            // ADR-0085: a machine asks the hub for its own certificate on its own connection; a client has no such thing to forward.
            | command_request::Command::Renew(_)
            // ADR-0122: shared items are the hub's (and a node asks for one on its own connection); never forwarded to a node.
            | command_request::Command::ShareItemPublish(_)
            | command_request::Command::ShareItemList(_)
            | command_request::Command::ShareItemRemove(_)
            | command_request::Command::ShareItemGet(_),
        ) => Err(Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "command")),
        Some(_) if req.target_node.is_empty() => {
            Err(Error::new(ErrorCode::NodeUnknown).with("node_id", ""))
        }
        Some(_) => Ok(&req.target_node),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn req(target: &str) -> v1::CommandRequest {
        v1::CommandRequest {
            request_id: "client-1".into(),
            idempotency_key: "k".into(),
            target_node: target.into(),
            command: Some(command_request::Command::AgentList(v1::AgentList::default())),
        }
    }

    #[test]
    fn pairing_commands_are_the_hubs_own_and_never_forwarded_to_a_node() {
        use command_request::Command as C;
        for c in [
            C::PairOpen(v1::PairOpen {}),
            C::PairClose(v1::PairClose {}),
            C::PairStatus(v1::PairStatus {}),
            C::PairApprove(v1::PairApprove::default()),
            C::PairDeny(v1::PairDeny::default()),
            C::PairRequest(v1::PairRequest::default()),
            C::PairReveal(v1::PairReveal::default()),
        ] {
            let mut r = req("laptop");
            r.command = Some(c);
            assert!(forwardable(&r).is_err());
        }
    }

    #[test]
    fn a_folder_listing_goes_only_to_a_node_that_can_list() {
        let r = Router::new();
        let (_old, mut old_rx) =
            r.register_with_capabilities("old", vec!["workspace.directory".into()]);
        let (_new, mut new_rx) = r.register_with_capabilities("new", vec!["directory.list".into()]);
        let mut request = req("old");
        request.command = Some(command_request::Command::DirectoryList(
            v1::DirectoryList::default(),
        ));
        assert!(matches!(
            r.send("old", request.clone()),
            Err(e) if e.code == ErrorCode::ProtocolCapabilityMissing
        ));
        assert!(old_rx.try_recv().is_err());
        assert!(r.send("new", request).is_ok());
        assert!(matches!(new_rx.try_recv(), Ok(ToNode::Command { .. })));
    }

    #[test]
    fn a_resume_goes_only_to_a_node_that_offers_it() {
        let r = Router::new();
        let (_old, mut old_rx) = r.register("old");
        let (_new, mut new_rx) = r.register_with_capabilities("new", vec!["agent.resume".into()]);
        let resume = Some(command_request::Command::AgentResume(
            v1::AgentResume::default(),
        ));
        let mut request = req("old");
        request.command = resume.clone();
        let refused = r.send("old", request).err().unwrap();
        assert_eq!(refused.code, ErrorCode::ProtocolCapabilityMissing);
        assert!(old_rx.try_recv().is_err());
        let mut request = req("new");
        request.command = resume;
        assert!(r.send("new", request).is_ok());
        assert!(matches!(new_rx.try_recv(), Ok(ToNode::Command { .. })));
    }

    /// ADR-0108: the hub passes a forget only to a node that offers `agent.forget.local`; an older one is never sent it.
    #[test]
    fn a_forget_goes_only_to_a_node_that_offers_it() {
        let r = Router::new();
        let (_old, mut old_rx) = r.register_with_capabilities("old", vec!["agent.resume".into()]);
        let (_new, mut new_rx) =
            r.register_with_capabilities("new", vec!["agent.forget.local".into()]);
        let forget = Some(command_request::Command::AgentForget(v1::AgentForget {
            agent_id: "agent-3".into(),
        }));
        let mut request = req("");
        request.command = forget.clone();
        let refused = r.send("old", request.clone()).err().unwrap();
        assert_eq!(refused.code, ErrorCode::ProtocolCapabilityMissing);
        assert!(old_rx.try_recv().is_err());
        request.command = forget;
        assert!(r.send("new", request).is_ok());
        assert!(matches!(new_rx.try_recv(), Ok(ToNode::Command { .. })));
    }

    #[test]
    fn a_start_with_launch_options_goes_only_to_a_node_that_knows_them() {
        let r = Router::new();
        let (_old, mut old_rx) =
            r.register_with_capabilities("old", vec!["workspace.directory".into()]);
        let (_new, mut new_rx) = r.register_with_capabilities(
            "new",
            vec!["workspace.directory".into(), "agent.launch_options".into()],
        );
        let launch = |options: Vec<String>| {
            let mut request = req("old");
            request.command = Some(command_request::Command::AgentLaunch(v1::AgentLaunch {
                launch_options: options,
                ..Default::default()
            }));
            request
        };
        let refused = r
            .send("old", launch(vec!["skip-permissions".into()]))
            .err()
            .unwrap();
        assert_eq!(refused.code, ErrorCode::ProtocolCapabilityMissing);
        assert!(old_rx.try_recv().is_err());
        // Without options an older node still gets the start.
        assert!(r.send("old", launch(Vec::new())).is_ok());
        assert!(matches!(old_rx.try_recv(), Ok(ToNode::Command { .. })));
        assert!(r
            .send("new", launch(vec!["skip-permissions".into()]))
            .is_ok());
        assert!(matches!(new_rx.try_recv(), Ok(ToNode::Command { .. })));
    }

    #[test]
    fn a_start_with_settings_goes_only_to_a_node_that_verifies_them() {
        // ADR-0066: an older node (with or without `agent.launch_options`) is refused, never sent a start whose model it would drop.
        let r = Router::new();
        let (_old, mut old_rx) = r.register_with_capabilities(
            "old",
            vec!["workspace.directory".into(), "agent.launch_options".into()],
        );
        let (_new, mut new_rx) = r.register_with_capabilities(
            "new",
            vec!["workspace.directory".into(), "agent.launch_settings".into()],
        );
        let setting = vec![v1::LaunchSetting {
            id: "model".into(),
            value: "gpt-6.1-sol".into(),
        }];
        let launch = |settings: Vec<v1::LaunchSetting>| {
            let mut request = req("old");
            request.command = Some(command_request::Command::AgentLaunch(v1::AgentLaunch {
                settings,
                ..Default::default()
            }));
            request
        };
        let refused = r.send("old", launch(setting.clone())).err().unwrap();
        assert_eq!(refused.code, ErrorCode::ProtocolCapabilityMissing);
        assert!(old_rx.try_recv().is_err());
        let mut start = req("old");
        start.command = Some(command_request::Command::AgentStart(v1::AgentStart {
            settings: setting.clone(),
            ..Default::default()
        }));
        assert!(r.send("old", start).is_err());
        // Without settings an older node still gets the launch; a node that offers the capability gets one with them.
        assert!(r.send("old", launch(Vec::new())).is_ok());
        assert!(matches!(old_rx.try_recv(), Ok(ToNode::Command { .. })));
        assert!(r.send("new", launch(setting)).is_ok());
        assert!(matches!(new_rx.try_recv(), Ok(ToNode::Command { .. })));
    }

    #[test]
    fn a_mode_or_model_switch_goes_only_to_a_node_that_offers_it() {
        let r = Router::new();
        let (_old, mut old_rx) = r.register("old");
        let (_new, mut new_rx) = r.register_with_capabilities(
            "new",
            vec!["agent.set_mode".into(), "agent.set_model".into()],
        );
        for (command, cap) in [
            (
                command_request::Command::AgentSetMode(v1::AgentSetMode::default()),
                "agent.set_mode",
            ),
            (
                command_request::Command::AgentSetModel(v1::AgentSetModel::default()),
                "agent.set_model",
            ),
        ] {
            let mut request = req("old");
            request.command = Some(command);
            let refused = r.send("old", request.clone()).err().unwrap();
            assert_eq!(refused.code, ErrorCode::ProtocolCapabilityMissing);
            assert_eq!(
                refused.params.get("capability"),
                Some(&agora_protocol::ParamValue::Str(cap.into()))
            );
            assert!(old_rx.try_recv().is_err());
            assert!(r.send("new", request).is_ok());
            assert!(matches!(new_rx.try_recv(), Ok(ToNode::Command { .. })));
        }
    }

    /// ADR-0090: the hub forwards each of the two commands only to a node that offers its capability, and refuses it for any other.
    #[test]
    fn a_config_or_cancel_turn_command_goes_only_to_a_node_that_offers_it() {
        let r = Router::new();
        let (_old, mut old_rx) = r.register("old");
        let (_new, mut new_rx) = r.register_with_capabilities(
            "new",
            vec!["agent.set_config".into(), "agent.cancel_turn".into()],
        );
        for (command, cap) in [
            (
                command_request::Command::AgentSetConfig(v1::AgentSetConfig::default()),
                "agent.set_config",
            ),
            (
                command_request::Command::AgentCancelTurn(v1::AgentCancelTurn::default()),
                "agent.cancel_turn",
            ),
        ] {
            let mut request = req("old");
            request.command = Some(command);
            let refused = r.send("old", request.clone()).err().unwrap();
            assert_eq!(refused.code, ErrorCode::ProtocolCapabilityMissing);
            assert_eq!(
                refused.params.get("capability"),
                Some(&agora_protocol::ParamValue::Str(cap.into()))
            );
            assert!(old_rx.try_recv().is_err());
            assert!(r.send("new", request).is_ok());
            assert!(matches!(new_rx.try_recv(), Ok(ToNode::Command { .. })));
        }
    }

    /// ADR-0095: a dispatch goes only to a node that offers `task.dispatch`; to any other the hub refuses it and forwards nothing.
    #[test]
    fn a_task_dispatch_goes_only_to_a_node_that_offers_it() {
        let r = Router::new();
        let (_old, mut old_rx) = r.register("old");
        let (_new, mut new_rx) = r.register_with_capabilities("new", vec!["task.dispatch".into()]);
        let mut request = req("old");
        request.command = Some(command_request::Command::TaskDispatch(
            v1::TaskDispatch::default(),
        ));
        let refused = r.send("old", request.clone()).err().unwrap();
        assert_eq!(refused.code, ErrorCode::ProtocolCapabilityMissing);
        assert_eq!(
            refused.params.get("capability"),
            Some(&agora_protocol::ParamValue::Str("task.dispatch".into()))
        );
        assert!(old_rx.try_recv().is_err());
        assert!(r.send("new", request).is_ok());
        assert!(matches!(new_rx.try_recv(), Ok(ToNode::Command { .. })));
    }

    #[test]
    fn directory_commands_require_the_negotiated_capability_without_a_legacy_fallback() {
        let r = Router::new();
        let (_old, mut old_rx) = r.register("old");
        let (_new, mut new_rx) =
            r.register_with_capabilities("new", vec!["workspace.directory".into()]);
        for command in [
            command_request::Command::DirectoryInspect(v1::DirectoryInspect::default()),
            command_request::Command::AgentLaunch(v1::AgentLaunch::default()),
            command_request::Command::WorkspaceQuery(v1::WorkspaceQuery::default()),
        ] {
            let mut request = req("old");
            request.command = Some(command);
            assert!(
                matches!(r.send("old", request.clone()), Err(e) if e.code == ErrorCode::ProtocolCapabilityMissing)
            );
            assert!(old_rx.try_recv().is_err());
            assert!(r.send("new", request).is_ok());
            assert!(matches!(new_rx.try_recv(), Ok(ToNode::Command { .. })));
        }
        assert!(
            r.send("old", req("old")).is_ok(),
            "legacy requests still work"
        );
    }

    #[test]
    fn a_handoff_delivery_goes_only_to_a_node_with_the_capability() {
        let r = Router::new();
        let (_old, mut old_rx) = r.register("old");
        let (_new, mut new_rx) = r.register_with_capabilities(
            "new",
            vec![agora_protocol::handshake::TASK_HANDOFF_V1.into()],
        );
        let mut request = req("old");
        request.command = Some(command_request::Command::DeliverTaskHandoff(
            v1::DeliverTaskHandoff::default(),
        ));
        assert!(
            matches!(r.send("old", request.clone()), Err(e) if e.code == ErrorCode::ProtocolCapabilityMissing
                && e.to_info().params["capability"].value
                    == Some(v1::param_value::Value::Str(agora_protocol::handshake::TASK_HANDOFF_V1.into())))
        );
        assert!(old_rx.try_recv().is_err());
        assert!(r.send("new", request).is_ok());
        assert!(matches!(new_rx.try_recv(), Ok(ToNode::Command { .. })));
    }

    #[tokio::test]
    async fn a_command_reaches_the_registered_node_under_a_hub_id_and_the_answer_returns() {
        let r = Router::new();
        let (_reg, mut rx) = r.register("laptop");
        let answer = r.send("laptop", req("laptop")).unwrap();
        let Some(ToNode::Command { request, reply, .. }) = rx.recv().await else {
            panic!("expected a command");
        };
        assert_ne!(request.request_id, "client-1");
        assert_eq!(request.idempotency_key, "k", "passed through");
        assert_eq!(request.target_node, "", "the node does not need it");
        reply
            .send(v1::CommandResponse {
                request_id: request.request_id.clone(),
                result: None,
            })
            .unwrap();
        assert_eq!(answer.await.unwrap().request_id, request.request_id);
    }

    #[test]
    fn two_requests_never_share_an_id_and_an_unknown_or_gone_node_is_offline() {
        let r = Router::new();
        let (reg, _rx) = r.register("a");
        drop(r.send("a", req("a")).unwrap());
        assert!(matches!(r.send("b", req("b")), Err(e) if e.code == ErrorCode::NodeOffline));
        drop(reg);
        assert!(!r.is_connected("a"));
        assert!(r.send("a", req("a")).is_err());
    }

    #[test]
    fn a_nodes_queued_commands_are_counted_until_its_connection_takes_them_or_ends() {
        // `GET /api/metrics` (R8): commands waiting for a node that does not read its connection are counted, per connection.
        let r = Router::new();
        let (a, mut rx_a) = r.register("a");
        let (_b, _rx_b) = r.register("b");
        let _keep: Vec<_> = (0..3)
            .map(|i| r.send("a", req(&format!("a{i}"))).unwrap())
            .collect();
        let _keep_b = r.send("b", req("b")).unwrap();
        assert_eq!(r.node_load(), (4, 0));
        // The connection's loop takes one and counts it as written and waiting for its answer (server.rs does this).
        let load = a.load();
        assert!(matches!(rx_a.try_recv(), Ok(ToNode::Command { .. })));
        load.queued.fetch_sub(1, Ordering::Relaxed);
        load.unanswered.store(1, Ordering::Relaxed);
        assert_eq!(r.node_load(), (3, 1));
        // A connection that ends stops counting at once, whatever it still held; a refused send is not counted.
        drop(a);
        assert_eq!(r.node_load(), (1, 0));
        drop(rx_a);
        assert!(r.send("a", req("late")).is_err());
        assert_eq!(r.node_load(), (1, 0));
    }

    #[test]
    fn a_node_that_does_not_take_its_commands_refuses_more_than_the_queue_holds() {
        // R5: a node whose connection is not written to (its loop waits on a write) stops piling up commands at NODE_QUEUE_MAX;
        // the refused one was never queued, so retrying is safe, and taking one makes room again.
        let r = Router::new();
        let (a, mut rx) = r.register("a");
        let _keep: Vec<_> = (0..NODE_QUEUE_MAX)
            .map(|i| r.send("a", req(&format!("q{i}"))).unwrap())
            .collect();
        let refused = r.send("a", req("one-too-many")).unwrap_err();
        assert_eq!(refused.code, ErrorCode::NodeResourceLimit);
        assert_eq!(r.node_load(), (NODE_QUEUE_MAX, 0));
        assert!(matches!(rx.try_recv(), Ok(ToNode::Command { .. })));
        a.load().queued.fetch_sub(1, Ordering::Relaxed);
        assert!(r.send("a", req("again")).is_ok());
        // Another node is not held up by this one.
        let (_b, _rx_b) = r.register("b");
        assert!(r.send("b", req("b")).is_ok());
    }

    #[test]
    fn an_old_connection_ending_does_not_unregister_its_replacement() {
        let r = Router::new();
        let (old, _rx1) = r.register("a");
        let (_new, _rx2) = r.register("a");
        drop(old);
        assert!(r.is_connected("a"));
    }

    #[test]
    fn relays_of_a_replaced_connection_end_when_the_node_reconnects() {
        let r = Router::new();
        let (old, _rx1) = r.register("n");
        let (client, _frames) = r.client();
        r.open_relay("n", &client, 9, None);
        // The new connection never writes on the old one's streams: the client is told at once and resubscribes.
        let (new, _rx2) = r.register("n");
        assert_eq!(client.take_closed(), vec![9]);
        let s = r.open_relay("n", &client, 11, None);
        assert!(!old.end(), "replaced");
        assert!(r.is_connected("n"));
        assert!(client.take_closed().is_empty(), "the new relay lives on");
        r.relay_frame("n", s, 0, vec![1]);
        assert!(new.end(), "current");
        assert_eq!(client.take_closed(), vec![11]);
    }

    #[test]
    fn terminal_frames_reach_the_client_and_a_slow_client_loses_its_relay() {
        let r = Router::new();
        let (_reg, mut node_rx) = r.register("n");
        let (client, mut frames) = r.client();
        let stream = r.open_relay("n", &client, 7, None);
        r.relay_frame("n", stream, 1, b"snap".to_vec());
        let f = frames.try_recv().unwrap();
        assert_eq!(
            (f.stream, f.kind, f.payload.as_slice()),
            (7, 1, &b"snap"[..])
        );
        // Never reading: the queue fills, the relay ends on both sides.
        for _ in 0..(RELAY_QUEUE + 1) {
            r.relay_frame("n", stream, 0, vec![0]);
        }
        assert_eq!(client.take_closed(), vec![7]);
        assert!(matches!(node_rx.try_recv(), Ok(ToNode::Close { stream: s }) if s == stream));
        // Frames for a relay that is gone are ignored.
        r.relay_frame("n", stream, 0, vec![0]);
    }

    #[test]
    fn only_terminal_relays_count_for_the_terminal_metrics() {
        // #574 review: `GET /api/metrics` counts the bytes of terminal frames that reached a window and the terminal relays let go;
        // a chat relay (snapshot, events) and a frame that was not delivered count for neither.
        let r = Router::new();
        let (_reg, _node_rx) = r.register("n");
        let (client, mut frames) = r.client();
        let chat = r.open_relay("n", &client, 5, None);
        let term = r.open_relay("n", &client, 6, Some("t-1"));
        r.relay_frame("n", chat, 10, vec![7; 4104]);
        r.relay_frame("n", chat, 11, vec![7; 30]);
        r.relay_frame("n", term, 1, b"snap".to_vec());
        r.relay_frame("n", term, 0, vec![1, 2, 3, 4]);
        while frames.try_recv().is_ok() {}
        assert_eq!(r.terminal_bytes(), 8);
        assert_eq!(r.relays_dropped(), 0);

        // A chat relay whose window does not keep up is let go, but is not a terminal relay dropped.
        for _ in 0..=RELAY_QUEUE {
            r.relay_frame("n", chat, 11, vec![0; 3]);
        }
        assert_eq!(client.take_closed(), vec![5]);
        assert_eq!((r.terminal_bytes(), r.relays_dropped()), (8, 0));
        while frames.try_recv().is_ok() {}

        // A terminal relay that does not keep up: RELAY_QUEUE frames of 2 bytes reach the queue, the next one ends the relay.
        for _ in 0..=RELAY_QUEUE {
            r.relay_frame("n", term, 0, vec![0; 2]);
        }
        assert_eq!(client.take_closed(), vec![6]);
        assert_eq!(r.relays_dropped(), 1);
        assert_eq!(r.terminal_bytes(), 8 + 2 * RELAY_QUEUE as u64);
    }

    #[test]
    fn f373_terminal_bytes_do_not_wrap_after_the_counter_saturates() {
        let r = Router::new();
        r.terminal_bytes.store(u64::MAX, Ordering::Relaxed);
        let (_reg, _node_rx) = r.register("n");
        let (client, _frames) = r.client();
        let stream = r.open_relay("n", &client, 1, Some("terminal"));
        r.relay_frame("n", stream, 0, vec![1]);
        assert_eq!(r.terminal_bytes(), u64::MAX);
    }

    #[test]
    fn terminal_bytes_still_reach_the_counter_maximum() {
        let r = Router::new();
        r.terminal_bytes.store(u64::MAX - 1, Ordering::Relaxed);
        let (_reg, _node_rx) = r.register("n");
        let (client, _frames) = r.client();
        let stream = r.open_relay("n", &client, 1, Some("terminal"));
        r.relay_frame("n", stream, 0, vec![1]);
        assert_eq!(r.terminal_bytes(), u64::MAX);
    }

    #[test]
    fn f374_dropped_terminal_relays_do_not_wrap_after_the_counter_saturates() {
        let r = Router::new();
        r.relays_dropped.store(u64::MAX, Ordering::Relaxed);
        let (_reg, _node_rx) = r.register("n");
        let (client, _frames) = r.client();
        let stream = r.open_relay("n", &client, 1, Some("terminal"));
        for _ in 0..=RELAY_QUEUE {
            r.relay_frame("n", stream, 0, vec![1]);
        }
        assert_eq!(r.relays_dropped(), u64::MAX);
    }

    #[test]
    fn dropped_terminal_relays_still_reach_the_counter_maximum() {
        let r = Router::new();
        r.relays_dropped.store(u64::MAX - 1, Ordering::Relaxed);
        let (_reg, _node_rx) = r.register("n");
        let (client, _frames) = r.client();
        let stream = r.open_relay("n", &client, 1, Some("terminal"));
        for _ in 0..=RELAY_QUEUE {
            r.relay_frame("n", stream, 0, vec![1]);
        }
        assert_eq!(r.relays_dropped(), u64::MAX);
    }

    #[test]
    fn verifies_f373_terminal_byte_saturation_for_single_multi_and_empty_payloads() {
        for (starting, payload, expected) in [
            (u64::MAX, vec![1], u64::MAX),
            (u64::MAX - 2, vec![1, 2, 3], u64::MAX),
            (17, Vec::new(), 17),
        ] {
            let r = Router::new();
            r.terminal_bytes.store(starting, Ordering::Relaxed);
            let (_reg, _node_rx) = r.register("n");
            let (client, _frames) = r.client();
            let stream = r.open_relay("n", &client, 1, Some("terminal"));
            r.relay_frame("n", stream, 0, payload);
            assert_eq!(r.terminal_bytes(), expected);
        }
    }

    #[test]
    fn verifies_f374_drop_saturation_for_max_near_max_and_non_terminal_relays() {
        for (starting, terminal, expected) in [
            (u64::MAX, Some("terminal"), u64::MAX),
            (u64::MAX - 1, Some("terminal"), u64::MAX),
            (9, None, 9),
        ] {
            let r = Router::new();
            r.relays_dropped.store(starting, Ordering::Relaxed);
            let (_reg, _node_rx) = r.register("n");
            let (client, _frames) = r.client();
            let stream = r.open_relay("n", &client, 1, terminal);
            for _ in 0..=RELAY_QUEUE {
                r.relay_frame("n", stream, 0, vec![1]);
            }
            assert_eq!(r.relays_dropped(), expected);
        }
    }

    #[test]
    fn f376_dynamic_stream_ids_never_wrap_into_reserved_streams() {
        let control = Router::new();
        let (_reg, _node_rx) = control.register("n");
        let (client, _frames) = control.client();
        control.next_stream.store(u32::MAX - 1, Ordering::Relaxed);
        assert_eq!(control.open_relay("n", &client, 1, None), u32::MAX - 1);
        assert_eq!(control.open_relay("n", &client, 2, None), u32::MAX);
        println!("CONTROL OK");

        let target = Router::new();
        let (_reg, _node_rx) = target.register("n");
        let (client, _frames) = target.client();
        target.next_stream.store(u32::MAX, Ordering::Relaxed);
        assert_eq!(target.open_relay("n", &client, 1, None), u32::MAX);
        let actual = target.open_relay("n", &client, 2, None);
        let expected = agora_protocol::frame::FIRST_DYNAMIC_STREAM;
        println!("EXPECTED: {expected}");
        println!("ACTUAL: {actual}");
        if actual == expected {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert_eq!(actual, expected);
    }

    #[test]
    fn verifies_f376_dynamic_stream_allocation_boundaries() {
        let r = Router::new();
        let (_reg, _node_rx) = r.register("n");
        let (client, _frames) = r.client();
        let first = agora_protocol::frame::FIRST_DYNAMIC_STREAM;

        r.next_stream.store(0, Ordering::Relaxed);
        assert_eq!(r.open_relay("n", &client, 1, None), first);

        r.next_stream.store(u32::MAX, Ordering::Relaxed);
        assert_eq!(r.open_relay("n", &client, 2, None), u32::MAX);
        assert_eq!(r.open_relay("n", &client, 3, None), first + 1);

        r.next_stream.store(first, Ordering::Relaxed);
        assert_eq!(r.open_relay("n", &client, 4, None), first + 2);
        println!("FIX VERIFIED");
    }

    #[test]
    fn f377_a_wrapped_connection_id_cannot_let_an_old_registration_end_its_replacement() {
        let control = Router::new();
        let (old, _old_rx) = control.register("n");
        let (_current, _current_rx) = control.register("n");
        drop(old);
        assert!(control.is_connected("n"));
        println!("CONTROL OK");

        let target = Router::new();
        let (old, _old_rx) = target.register("n");
        target.next.store(u64::MAX, Ordering::Relaxed);
        let (_at_max, _max_rx) = target.register("n");
        let (_current, _current_rx) = target.register("n");
        drop(old);
        let actual = target.is_connected("n");
        println!("EXPECTED: true");
        println!("ACTUAL: {actual}");
        if actual {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert!(actual);
    }

    #[test]
    fn verifies_f377_registration_ownership_is_unique_across_boundaries() {
        let ordinary = Router::new();
        let (old, _old_rx) = ordinary.register("n");
        let (_current, _current_rx) = ordinary.register("n");
        drop(old);
        assert!(ordinary.is_connected("n"));

        let wrapped = Router::new();
        let (old, _old_rx) = wrapped.register("n");
        wrapped.next.store(u64::MAX, Ordering::Relaxed);
        let (_at_max, _max_rx) = wrapped.register("n");
        let (_current, _current_rx) = wrapped.register("n");
        drop(old);
        assert!(wrapped.is_connected("n"));

        let reordered = Router::new();
        let (oldest, _oldest_rx) = reordered.register("n");
        let (older, _older_rx) = reordered.register("n");
        let (_current, _current_rx) = reordered.register("n");
        drop(older);
        drop(oldest);
        assert!(reordered.is_connected("n"));
        println!("FIX VERIFIED");
    }

    #[test]
    fn f378_a_wrapped_client_id_cannot_let_an_old_client_close_its_replacements_relay() {
        let control = Router::new();
        let (_reg, _node_rx) = control.register("n");
        let (old, _old_frames) = control.client();
        let (current, mut current_frames) = control.client();
        let stream = control.open_relay("n", &current, 1, None);
        drop(old);
        control.relay_frame("n", stream, 0, vec![1]);
        assert!(current_frames.try_recv().is_ok());
        println!("CONTROL OK");

        let target = Router::new();
        let (_reg, _node_rx) = target.register("n");
        target.next.store(0, Ordering::Relaxed);
        let (old, _old_frames) = target.client();
        target.next.store(u64::MAX, Ordering::Relaxed);
        let (_at_max, _max_frames) = target.client();
        let (current, mut current_frames) = target.client();
        let stream = target.open_relay("n", &current, 1, None);
        drop(old);
        target.relay_frame("n", stream, 0, vec![1]);
        let actual = current_frames.try_recv().is_ok();
        println!("EXPECTED: true");
        println!("ACTUAL: {actual}");
        if actual {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert!(actual);
    }

    #[test]
    fn verifies_f378_client_ownership_cannot_alias() {
        let relay_survives_old_client = |force_boundary: bool, close_one: bool| {
            let r = Router::new();
            let (_reg, _node_rx) = r.register("n");
            let (old, _old_frames) = r.client();
            let old_stream = r.open_relay("n", &old, 7, None);
            if force_boundary {
                r.next.store(u64::MAX, Ordering::Relaxed);
                let (_boundary, _boundary_frames) = r.client();
            }
            let (current, mut frames) = r.client();
            let current_stream = r.open_relay("n", &current, 8, None);
            if close_one {
                old.close(7);
            } else {
                drop(old);
            }
            r.relay_frame("n", current_stream, 0, vec![1]);
            assert!(frames.try_recv().is_ok());
            assert!(!r
                .relays
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key(&("n".to_string(), old_stream)));
        };

        relay_survives_old_client(false, false);
        relay_survives_old_client(true, false);
        relay_survives_old_client(true, true);
        println!("FIX VERIFIED");
    }

    #[test]
    fn f379_request_ids_remain_fresh_when_the_u64_sequence_wraps() {
        let r = Router::new();
        let (_reg, mut node_rx) = r.register("n");
        r.next.store(10, Ordering::Relaxed);
        let _first_reply = r.send("n", req("n")).unwrap();
        let first = match node_rx.try_recv().unwrap() {
            ToNode::Command { request, .. } => request.request_id,
            ToNode::Close { .. } => panic!("unexpected close"),
        };
        let _control_reply = r.send("n", req("n")).unwrap();
        let control = match node_rx.try_recv().unwrap() {
            ToNode::Command { request, .. } => request.request_id,
            ToNode::Close { .. } => panic!("unexpected close"),
        };
        assert_ne!(first, control);
        println!("CONTROL OK");

        r.next.store(0, Ordering::Relaxed);
        let _baseline_reply = r.send("n", req("n")).unwrap();
        let baseline = match node_rx.try_recv().unwrap() {
            ToNode::Command { request, .. } => request.request_id,
            ToNode::Close { .. } => panic!("unexpected close"),
        };
        r.next.store(u64::MAX, Ordering::Relaxed);
        let _at_max_reply = r.send("n", req("n")).unwrap();
        let _ = node_rx.try_recv().unwrap();
        let _wrapped_reply = r.send("n", req("n")).unwrap();
        let actual = match node_rx.try_recv().unwrap() {
            ToNode::Command { request, .. } => request.request_id,
            ToNode::Close { .. } => panic!("unexpected close"),
        };
        println!("EXPECTED: a request id different from {baseline}");
        println!("ACTUAL: {actual}");
        if actual != baseline {
            println!("PROBLEM NOT REPRODUCED");
        } else {
            println!("PROBLEM CONFIRMED");
        }
        assert_ne!(actual, baseline);
    }

    #[test]
    fn verifies_f379_request_ids_are_unique_across_sequence_epochs() {
        let r = Router::new();
        let (_reg, mut node_rx) = r.register("n");
        let send_id = |r: &Router, node_rx: &mut mpsc::UnboundedReceiver<ToNode>| {
            let _reply = r.send("n", req("n")).unwrap();
            match node_rx.try_recv().unwrap() {
                ToNode::Command { request, .. } => request.request_id,
                ToNode::Close { .. } => panic!("unexpected close"),
            }
        };

        assert_eq!(send_id(&r, &mut node_rx), "h0");
        assert_eq!(send_id(&r, &mut node_rx), "h1");

        r.next.store(u64::MAX - 1, Ordering::Relaxed);
        let before_wrap = send_id(&r, &mut node_rx);
        let at_wrap = send_id(&r, &mut node_rx);
        let after_wrap = send_id(&r, &mut node_rx);
        let next_epoch = send_id(&r, &mut node_rx);
        assert_eq!(before_wrap, format!("h{}", u64::MAX - 1));
        assert_eq!(at_wrap, format!("h{}", u64::MAX));
        assert_eq!(after_wrap, "h1-0");
        assert_eq!(next_epoch, "h1-1");
        assert_eq!(
            HashSet::from([before_wrap, at_wrap, after_wrap, next_epoch]).len(),
            4
        );
        println!("FIX VERIFIED");
    }

    #[test]
    fn a_client_going_away_or_a_node_leaving_ends_the_relays() {
        let r = Router::new();
        let (reg, mut node_rx) = r.register("n");
        let (client, _frames) = r.client();
        let s1 = r.open_relay("n", &client, 3, None);
        let s2 = r.open_relay("n", &client, 4, None);
        client.close(3);
        assert!(matches!(node_rx.try_recv(), Ok(ToNode::Close { stream }) if stream == s1));
        drop(reg); // the node leaves: the other relay ends for the client
        assert_eq!(client.take_closed(), vec![4]);
        let _ = s2;
        let (reg2, mut node_rx2) = r.register("n");
        let s3 = r.open_relay("n", &client, 9, None);
        drop(client);
        assert!(matches!(node_rx2.try_recv(), Ok(ToNode::Close { stream }) if stream == s3));
        drop(reg2);
    }

    #[test]
    fn a_terminal_exit_ends_the_relay_after_delivering_it() {
        let r = Router::new();
        let (_reg, _rx) = r.register("n");
        let (client, mut frames) = r.client();
        let s = r.open_relay("n", &client, 5, None);
        r.relay_frame("n", s, 3, vec![0, 0, 0, 0]);
        assert_eq!(frames.try_recv().unwrap().kind, 3);
        r.relay_frame("n", s, 0, vec![1]);
        assert!(frames.try_recv().is_err(), "nothing after the exit");
    }

    /// The sizes the node was sent since the last call (closes are skipped).
    fn sent_sizes(rx: &mut mpsc::UnboundedReceiver<ToNode>) -> Vec<(String, u32, u32)> {
        let mut out = Vec::new();
        while let Ok(m) = rx.try_recv() {
            if let ToNode::Command { request, .. } = m {
                if let Some(command_request::Command::TerminalResize(t)) = request.command {
                    out.push((t.terminal_id, t.cols, t.rows));
                }
            }
        }
        out
    }

    fn sz(t: &str, cols: u32, rows: u32) -> Vec<(String, u32, u32)> {
        vec![(t.to_string(), cols, rows)]
    }

    #[test]
    fn two_windows_on_one_terminal_get_the_smallest_size_and_only_changes_are_sent() {
        let r = Router::new();
        let (_reg, mut rx) = r.register("n");
        let (small, _f1) = r.client();
        let (big, _f2) = r.client();
        r.open_relay("n", &small, 3, Some("t-a"));
        r.open_relay("n", &big, 3, Some("t-a"));
        assert!(sent_sizes(&mut rx).is_empty(), "nobody asked yet");
        assert_eq!(
            r.resize(&big, "n", "t-a", 200, 50),
            Resize::Forward((200, 50))
        );
        assert_eq!(
            r.resize(&small, "n", "t-a", 100, 60),
            Resize::Forward((100, 50))
        );
        // The same again, or a bigger window growing, changes nothing the terminal needs.
        assert_eq!(r.resize(&small, "n", "t-a", 100, 60), Resize::Unchanged);
        assert_eq!(
            r.resize(&big, "n", "t-a", 220, 70),
            Resize::Forward((100, 60))
        );
        assert_eq!(r.resize(&big, "n", "t-a", 230, 80), Resize::Unchanged);
        // Another terminal is its own book.
        r.open_relay("n", &small, 5, Some("t-b"));
        assert_eq!(
            r.resize(&small, "n", "t-b", 80, 24),
            Resize::Forward((80, 24))
        );
        assert!(
            sent_sizes(&mut rx).is_empty(),
            "the client's own command carries the size"
        );
    }

    #[test]
    fn when_the_small_window_stops_watching_or_leaves_the_others_grow_back() {
        let r = Router::new();
        let (_reg, mut rx) = r.register("n");
        let (small, _f1) = r.client();
        let (big, _f2) = r.client();
        r.open_relay("n", &small, 3, Some("t-a"));
        r.open_relay("n", &big, 4, Some("t-a"));
        r.resize(&big, "n", "t-a", 200, 50);
        r.resize(&small, "n", "t-a", 100, 40);
        small.close(3);
        assert_eq!(sent_sizes(&mut rx), sz("t-a", 200, 50));
        // It comes back: its size was forgotten with its view, so nothing changes until it says it again.
        r.open_relay("n", &small, 6, Some("t-a"));
        assert!(sent_sizes(&mut rx).is_empty());
        assert_eq!(
            r.resize(&small, "n", "t-a", 100, 40),
            Resize::Forward((100, 40))
        );
        // The window closes (its connection ends): the other grows back once.
        drop(small);
        assert_eq!(sent_sizes(&mut rx), sz("t-a", 200, 50));
        // The last one leaving sends nothing: the terminal keeps its size.
        drop(big);
        assert!(sent_sizes(&mut rx).is_empty());
    }

    #[test]
    fn when_a_lagging_small_window_is_dropped_the_others_grow_back() {
        let r = Router::new();
        let (_reg, mut rx) = r.register("n");
        let (small, _f1) = r.client();
        let (big, _f2) = r.client();
        let s_stream = r.open_relay("n", &small, 3, Some("t-a"));
        let _b_stream = r.open_relay("n", &big, 4, Some("t-a"));
        assert_eq!(
            r.resize(&big, "n", "t-a", 200, 50),
            Resize::Forward((200, 50))
        );
        assert_eq!(
            r.resize(&small, "n", "t-a", 100, 40),
            Resize::Forward((100, 40))
        );
        let _ = sent_sizes(&mut rx);
        for _ in 0..=RELAY_QUEUE {
            r.relay_frame("n", s_stream, 0, vec![b'x']);
        }
        assert_eq!(r.relays_dropped(), 1);
        assert_eq!(sent_sizes(&mut rx), sz("t-a", 200, 50));
    }

    #[test]
    fn a_second_view_of_the_same_terminal_in_one_window_keeps_its_size_when_the_first_closes() {
        let r = Router::new();
        let (_reg, mut rx) = r.register("n");
        let (one, _f1) = r.client();
        let (other, _f2) = r.client();
        r.open_relay("n", &one, 3, Some("t-a"));
        r.open_relay("n", &one, 4, Some("t-a"));
        r.open_relay("n", &other, 3, Some("t-a"));
        r.resize(&other, "n", "t-a", 200, 50);
        r.resize(&one, "n", "t-a", 90, 30);
        one.close(3);
        assert!(
            sent_sizes(&mut rx).is_empty(),
            "still watching through stream 4"
        );
        one.close(4);
        assert_eq!(sent_sizes(&mut rx), sz("t-a", 200, 50));
    }

    #[test]
    fn a_window_whose_relay_the_hub_ended_counts_again_when_it_resubscribes() {
        let r = Router::new();
        let (reg, mut rx) = r.register("n");
        let (small, _f1) = r.client();
        let (big, _f2) = r.client();
        r.open_relay("n", &small, 3, Some("t-a"));
        r.open_relay("n", &big, 3, Some("t-a"));
        r.resize(&big, "n", "t-a", 200, 50);
        r.resize(&small, "n", "t-a", 100, 40);
        // The node reconnects: every relay ends and the terminal's size is unknown.
        drop(reg);
        let (_reg2, mut rx2) = r.register("n");
        assert!(sent_sizes(&mut rx).is_empty());
        r.open_relay("n", &big, 7, Some("t-a"));
        assert_eq!(sent_sizes(&mut rx2), sz("t-a", 200, 50));
        r.open_relay("n", &small, 7, Some("t-a"));
        assert_eq!(sent_sizes(&mut rx2), sz("t-a", 100, 40));
    }

    #[test]
    fn a_window_that_does_not_watch_the_terminal_has_no_say_while_others_do() {
        let r = Router::new();
        let (_reg, _rx) = r.register("n");
        let (script, _f1) = r.client();
        let (viewer, _f2) = r.client();
        // Nobody watches through the hub: as before, the size goes as asked (and a zero size is the node's to refuse).
        assert_eq!(r.resize(&script, "n", "t-a", 120, 40), Resize::PassThrough);
        assert_eq!(r.resize(&script, "n", "t-a", 0, 40), Resize::PassThrough);
        r.open_relay("n", &viewer, 3, Some("t-a"));
        assert_eq!(
            r.resize(&script, "n", "t-a", 120, 40),
            Resize::PassThrough,
            "the viewer has not asked yet"
        );
        assert_eq!(
            r.resize(&viewer, "n", "t-a", 100, 30),
            Resize::Forward((100, 30))
        );
        assert_eq!(r.resize(&script, "n", "t-a", 50, 10), Resize::Unchanged);
        assert_eq!(r.resize(&viewer, "n", "t-a", 100, 30), Resize::Unchanged);
    }

    #[test]
    fn a_size_beyond_the_nodes_limits_asks_for_the_limit() {
        let r = Router::new();
        let (_reg, _rx) = r.register("n");
        let (c, _f) = r.client();
        r.open_relay("n", &c, 3, Some("t-a"));
        assert_eq!(
            r.resize(&c, "n", "t-a", 5000, 70_000),
            Resize::Forward((MAX_COLS, MAX_ROWS))
        );
        assert_eq!(r.resize(&c, "n", "t-a", 0, 10), Resize::PassThrough);
        assert_eq!(r.resize(&c, "n", "t-a", 1, 1), Resize::Forward((1, 1)));
    }

    #[test]
    fn a_size_set_elsewhere_is_seen_in_the_terminals_frames_and_corrected_on_the_next_change() {
        let r = Router::new();
        let (_reg, mut rx) = r.register("n");
        let (c, _f) = r.client();
        let s = r.open_relay("n", &c, 3, Some("t-a"));
        assert_eq!(
            r.resize(&c, "n", "t-a", 100, 30),
            Resize::Forward((100, 30))
        );
        // A terminal client on the node's own socket made it bigger: the hub sees the resize frame it relays.
        let mut frame = 140u16.to_le_bytes().to_vec();
        frame.extend_from_slice(&45u16.to_le_bytes());
        r.relay_frame("n", s, 2, frame);
        assert_eq!(
            r.resize(&c, "n", "t-a", 100, 30),
            Resize::Forward((100, 30))
        );
        // The terminal ends: its book goes with it.
        r.relay_frame("n", s, 3, vec![0; 4]);
        assert!(r.sizes.lock().unwrap().is_empty());
        assert!(sent_sizes(&mut rx).is_empty());
    }

    #[test]
    fn a_refused_subscription_no_longer_counts() {
        let r = Router::new();
        let (_reg, mut rx) = r.register("n");
        let (a, _f1) = r.client();
        let (b, _f2) = r.client();
        r.open_relay("n", &a, 3, Some("t-a"));
        r.resize(&a, "n", "t-a", 200, 50);
        let s = r.open_relay("n", &b, 3, Some("t-a"));
        r.resize(&b, "n", "t-a", 80, 20);
        r.drop_relay("n", s);
        assert_eq!(sent_sizes(&mut rx), sz("t-a", 200, 50));
    }

    #[test]
    fn only_commands_with_a_target_are_forwarded() {
        assert!(forwardable(&req("a")).is_ok());
        assert!(forwardable(&req("")).is_err());
        let mut e = req("a");
        e.command = Some(command_request::Command::Enroll(
            v1::EnrollRequest::default(),
        ));
        assert!(forwardable(&e).is_err());
        e.command = None;
        assert!(forwardable(&e).is_err());
        // What a node says to the hub is not something a client may forward (it would choose its own `origin`).
        e.command = Some(command_request::Command::MeshPost(v1::MeshPost::default()));
        assert!(forwardable(&e).is_err());
        e.command = Some(command_request::Command::BoardCall(v1::BoardCall::default()));
        assert!(forwardable(&e).is_err());
        e.command = Some(command_request::Command::BoardJoin(v1::BoardJoin::default()));
        assert!(forwardable(&e).is_err());
        e.command = Some(command_request::Command::DeliverTaskHandoff(
            v1::DeliverTaskHandoff::default(),
        ));
        assert!(forwardable(&e).is_err());
    }

    /// #574 review: the relay path before and after the terminal counters (`cargo test --release -p agora-hub --lib
    /// relay_path_cost -- --ignored --nocapture`). Each window has a terminal relay; every round the node writes one frame to each.
    #[test]
    #[ignore = "measurement"]
    fn relay_path_cost() {
        for (clients, size) in [(1usize, 64usize), (1, 4096), (8, 64), (8, 4096), (32, 4096)] {
            let r = Router::new();
            let (_reg, _node_rx) = r.register("n");
            let mut windows = Vec::new();
            for c in 0..clients {
                let (client, frames) = r.client();
                let s = r.open_relay("n", &client, 7, Some(&format!("t-{c}")));
                windows.push((client, frames, s));
            }
            let frames_total = 2_000_000 / clients.max(1);
            let mut best = f64::MAX;
            for _ in 0..5 {
                let start = std::time::Instant::now();
                for i in 0..frames_total {
                    for (_, _, s) in &windows {
                        r.relay_frame("n", *s, 0, vec![0u8; size]);
                    }
                    if i % 256 == 255 {
                        for (_, rx, _) in &mut windows {
                            while rx.try_recv().is_ok() {}
                        }
                    }
                }
                let ns = start.elapsed().as_nanos() as f64 / (frames_total * clients) as f64;
                best = best.min(ns);
            }
            println!("relay_path clients={clients} payload={size}B best_of_5={best:.1} ns/frame");
        }
    }
}

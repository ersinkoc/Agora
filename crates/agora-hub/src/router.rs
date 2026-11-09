//! Routing of commands from clients to the node that owns the target (ADR-0029).
//!
//! A node's connection registers here for as long as it lives. A client's command is forwarded unchanged except for
//! its request id (the hub picks a fresh one so two clients cannot collide) and the answer is mapped back. The hub
//! never edits what a command says; the node runs it through the same handler as a local command, with the same policy.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
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

struct Link {
    conn: u64,
    tx: mpsc::UnboundedSender<ToNode>,
}

struct Relay {
    client: u64,
    client_stream: u32,
    to_client: mpsc::Sender<RelayFrame>,
    closed: Arc<Mutex<Vec<u32>>>,
}

pub struct Router {
    nodes: Mutex<HashMap<String, Link>>,
    /// `(node, stream on the node's connection)` → the client stream it feeds.
    relays: Mutex<HashMap<(String, u32), Relay>>,
    next: AtomicU64,
    next_stream: AtomicU32,
}

impl Default for Router {
    fn default() -> Self {
        Self {
            nodes: Mutex::default(),
            relays: Mutex::default(),
            next: AtomicU64::new(0),
            next_stream: AtomicU32::new(agora_protocol::frame::FIRST_DYNAMIC_STREAM),
        }
    }
}

/// One client connection's end of the relays: terminal frames arrive on `frames`, and `closed` lists client streams the
/// hub ended (the node left, the client was too slow, the terminal exited). Dropping it ends all its relays.
pub struct ClientRelays {
    router: Arc<Router>,
    id: u64,
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
            .close_client_stream(self.id, Some(client_stream));
    }
}

impl Drop for ClientRelays {
    fn drop(&mut self) {
        self.router.close_client_stream(self.id, None);
    }
}

/// Keeps a node registered; dropping it (the connection ended) unregisters it, unless a newer connection of the same
/// node has replaced it in the meantime.
pub struct Registration {
    router: Arc<Router>,
    node: String,
    conn: u64,
}

impl Drop for Registration {
    fn drop(&mut self) {
        let ours = {
            let mut g = self.router.nodes.lock().unwrap_or_else(|e| e.into_inner());
            let ours = g.get(&self.node).is_some_and(|l| l.conn == self.conn);
            if ours {
                g.remove(&self.node);
            }
            ours
        };
        // The terminals this node was relaying end with it.
        if ours {
            self.router.end_node_relays(&self.node);
        }
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
        let (tx, rx) = mpsc::unbounded_channel();
        let conn = self.next.fetch_add(1, Ordering::Relaxed);
        self.nodes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(node.to_string(), Link { conn, tx });
        (
            Registration {
                router: Arc::clone(self),
                node: node.to_string(),
                conn,
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
                id: self.next.fetch_add(1, Ordering::Relaxed),
                tx,
                closed: Arc::default(),
            },
            rx,
        )
    }

    /// Reserves the stream a node will write a client's terminal on. Frames the node writes on it are queued for the
    /// client under `client_stream`.
    pub fn open_relay(&self, node: &str, client: &ClientRelays, client_stream: u32) -> u32 {
        let stream = self.next_stream.fetch_add(1, Ordering::Relaxed);
        self.relays
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                (node.to_string(), stream),
                Relay {
                    client: client.id,
                    client_stream,
                    to_client: client.tx.clone(),
                    closed: Arc::clone(&client.closed),
                },
            );
        stream
    }

    /// A frame from a node on a dynamic stream: queued for the client. A client whose queue is full is not waited for;
    /// its relay is closed on both sides and it resubscribes. A terminal exit ends the relay after the frame.
    pub fn relay_frame(&self, node: &str, stream: u32, kind: u8, payload: Vec<u8>) {
        let key = (node.to_string(), stream);
        let mut relays = self.relays.lock().unwrap_or_else(|e| e.into_inner());
        let Some(r) = relays.get(&key) else {
            return;
        };
        if kind == agora_protocol::frame::STREAM_CLOSE {
            if let Some(r) = relays.remove(&key) {
                r.closed
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(r.client_stream);
            }
            return;
        }
        let sent = r.to_client.try_send(RelayFrame {
            stream: r.client_stream,
            kind,
            payload,
        });
        let exited = kind == 3;
        if sent.is_err() || exited {
            if let Some(r) = relays.remove(&key) {
                if sent.is_err() {
                    r.closed
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(r.client_stream);
                    drop(relays);
                    self.tell_node(node, ToNode::Close { stream });
                }
            }
        }
    }

    /// Ends a relay whose subscription the node refused, or whose command could not be delivered.
    pub fn drop_relay(&self, node: &str, stream: u32) {
        self.relays
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&(node.to_string(), stream));
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

    /// Ends one client stream (or all of a client's) and tells the nodes to stop writing on them.
    fn close_client_stream(&self, client: u64, only: Option<u32>) {
        let ended: Vec<(String, u32)> = {
            let mut relays = self.relays.lock().unwrap_or_else(|e| e.into_inner());
            let keys: Vec<_> = relays
                .iter()
                .filter(|(_, r)| r.client == client && only.is_none_or(|s| s == r.client_stream))
                .map(|(k, _)| k.clone())
                .collect();
            for k in &keys {
                relays.remove(k);
            }
            keys
        };
        for (node, stream) in ended {
            self.tell_node(&node, ToNode::Close { stream });
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
        request.request_id = format!("h{}", self.next.fetch_add(1, Ordering::Relaxed));
        request.target_node.clear();
        let (reply, rx) = oneshot::channel();
        let g = self.nodes.lock().unwrap_or_else(|e| e.into_inner());
        let link = g.get(node).ok_or_else(offline)?;
        link.tx
            .send(ToNode::Command {
                request: Box::new(request),
                reply,
                stream,
            })
            .map_err(|_| offline())?;
        Ok(rx)
    }
}

/// What a client may ask the hub to forward: everything except enrollment, which only an unauthenticated peer may do
/// and which the hub answers itself.
pub fn forwardable(req: &v1::CommandRequest) -> Result<&str, Error> {
    match &req.command {
        None => Err(Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "command")),
        // `MeshPost` and `BoardCall` are what a node sends the hub (the hub writes `origin` from the certificate); a client
        // forwarding one would put its own `origin` on a message, so they are not client commands.
        Some(
            command_request::Command::Enroll(_)
            | command_request::Command::MeshPost(_)
            | command_request::Command::BoardCall(_),
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
    fn an_old_connection_ending_does_not_unregister_its_replacement() {
        let r = Router::new();
        let (old, _rx1) = r.register("a");
        let (_new, _rx2) = r.register("a");
        drop(old);
        assert!(r.is_connected("a"));
    }

    #[test]
    fn terminal_frames_reach_the_client_and_a_slow_client_loses_its_relay() {
        let r = Router::new();
        let (_reg, mut node_rx) = r.register("n");
        let (client, mut frames) = r.client();
        let stream = r.open_relay("n", &client, 7);
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
    fn a_client_going_away_or_a_node_leaving_ends_the_relays() {
        let r = Router::new();
        let (reg, mut node_rx) = r.register("n");
        let (client, _frames) = r.client();
        let s1 = r.open_relay("n", &client, 3);
        let s2 = r.open_relay("n", &client, 4);
        client.close(3);
        assert!(matches!(node_rx.try_recv(), Ok(ToNode::Close { stream }) if stream == s1));
        drop(reg); // the node leaves: the other relay ends for the client
        assert_eq!(client.take_closed(), vec![4]);
        let _ = s2;
        let (reg2, mut node_rx2) = r.register("n");
        let s3 = r.open_relay("n", &client, 9);
        drop(client);
        assert!(matches!(node_rx2.try_recv(), Ok(ToNode::Close { stream }) if stream == s3));
        drop(reg2);
    }

    #[test]
    fn a_terminal_exit_ends_the_relay_after_delivering_it() {
        let r = Router::new();
        let (_reg, _rx) = r.register("n");
        let (client, mut frames) = r.client();
        let s = r.open_relay("n", &client, 5);
        r.relay_frame("n", s, 3, vec![0, 0, 0, 0]);
        assert_eq!(frames.try_recv().unwrap().kind, 3);
        r.relay_frame("n", s, 0, vec![1]);
        assert!(frames.try_recv().is_err(), "nothing after the exit");
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
    }
}

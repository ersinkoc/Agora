//! Routing of commands from clients to the node that owns the target (ADR-0029).
//!
//! A node's connection registers here for as long as it lives. A client's command is forwarded unchanged except for
//! its request id (the hub picks a fresh one so two clients cannot collide) and the answer is mapped back. The hub
//! never edits what a command says; the node runs it through the same handler as a local command, with the same policy.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use agora_protocol::v1::{self, command_request};
use agora_protocol::{Error, ErrorCode};
use tokio::sync::{mpsc, oneshot};

/// One command on its way to a node, with where its answer goes.
pub struct Routed {
    /// Already carries the hub-chosen request id.
    pub request: v1::CommandRequest,
    pub reply: oneshot::Sender<v1::CommandResponse>,
}

struct Link {
    conn: u64,
    tx: mpsc::UnboundedSender<Routed>,
}

#[derive(Default)]
pub struct Router {
    nodes: Mutex<HashMap<String, Link>>,
    next: AtomicU64,
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
        let mut g = self.router.nodes.lock().unwrap_or_else(|e| e.into_inner());
        if g.get(&self.node).is_some_and(|l| l.conn == self.conn) {
            g.remove(&self.node);
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
    ) -> (Registration, mpsc::UnboundedReceiver<Routed>) {
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
        mut request: v1::CommandRequest,
    ) -> Result<oneshot::Receiver<v1::CommandResponse>, Error> {
        let offline = || Error::new(ErrorCode::NodeOffline).with("node_id", node);
        request.request_id = format!("h{}", self.next.fetch_add(1, Ordering::Relaxed));
        request.target_node.clear();
        let (reply, rx) = oneshot::channel();
        let g = self.nodes.lock().unwrap_or_else(|e| e.into_inner());
        let link = g.get(node).ok_or_else(offline)?;
        link.tx
            .send(Routed { request, reply })
            .map_err(|_| offline())?;
        Ok(rx)
    }
}

/// What a client may ask the hub to forward: everything except enrollment, which only an unauthenticated peer may do
/// and which the hub answers itself.
pub fn forwardable(req: &v1::CommandRequest) -> Result<&str, Error> {
    match &req.command {
        None => Err(Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "command")),
        Some(command_request::Command::Enroll(_)) => {
            Err(Error::new(ErrorCode::ProtocolInvalidMessage).with("field", "command"))
        }
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
        let got = rx.recv().await.unwrap();
        assert_ne!(got.request.request_id, "client-1");
        assert_eq!(got.request.idempotency_key, "k", "passed through");
        assert_eq!(got.request.target_node, "", "the node does not need it");
        got.reply
            .send(v1::CommandResponse {
                request_id: got.request.request_id.clone(),
                result: None,
            })
            .unwrap();
        assert_eq!(answer.await.unwrap().request_id, got.request.request_id);
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
    }
}

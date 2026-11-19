//! Stops a bridge connected to the repository fake ACP agent and checks process ownership
//! at the moment its acknowledgement is published.
#![allow(clippy::unwrap_used)]

use std::io::{self, Cursor, Read, Write};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

use agora_platform::{process_children, process_identity, process_still_running};
use serde_json::{json, Value};

const GUARD: Duration = Duration::from_secs(10);

#[test]
fn acp_shutdown_order_child() {
    if std::env::var_os("AGORA_TEST_ACP_SHUTDOWN_ORDER").is_none()
        || !std::env::args().any(|arg| arg == "acp_shutdown_order_child")
    {
        return;
    }
    println!();
    agora_fake_agent::acp::run(std::io::stdin().lock(), &mut std::io::stdout().lock()).unwrap();
}

struct NodeInput(Receiver<Vec<u8>>, Cursor<Vec<u8>>);

impl Read for NodeInput {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.1.read(buf)?;
        if n != 0 || buf.is_empty() {
            return Ok(n);
        }
        let Ok(bytes) = self.0.recv() else {
            return Ok(0);
        };
        self.1 = Cursor::new(bytes);
        self.1.read(buf)
    }
}

struct NodeOutput {
    buffer: Vec<u8>,
    replies: Sender<Value>,
    release_stop: Receiver<()>,
    stop_id: Option<Value>,
}

impl Write for NodeOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let reply: Value = serde_json::from_slice(&std::mem::take(&mut self.buffer))?;
        let is_stop = self
            .stop_id
            .as_ref()
            .is_some_and(|id| reply.get("id") == Some(id));
        self.replies.send(reply).unwrap();
        if is_stop {
            // Keep the bridge paused at publication until the node side has checked the child's identity.
            self.release_stop.recv_timeout(GUARD).unwrap();
        }
        Ok(())
    }
}

fn send(input: &Sender<Vec<u8>>, request: Value) {
    input.send(format!("{request}\n").into_bytes()).unwrap();
}

fn stop_order(method: Option<&str>, stop_id: Option<Value>) -> Option<bool> {
    let (input, requests) = channel();
    let (replies, output) = channel();
    let (release, release_stop) = channel();
    let gate_id = stop_id.clone();
    let bridge = std::thread::spawn(move || {
        let child = vec![
            std::env::current_exe()
                .unwrap()
                .to_str()
                .unwrap()
                .to_string(),
            "--exact".into(),
            "acp_shutdown_order_child".into(),
            "--nocapture".into(),
            "--test-threads=1".into(),
        ];
        agora_adapters::acp::run(
            &child,
            NodeInput(requests, Cursor::default()),
            NodeOutput {
                buffer: Vec::new(),
                replies,
                release_stop,
                stop_id: gate_id,
            },
        )
    });
    send(
        &input,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize"}),
    );
    let init = output.recv_timeout(GUARD).unwrap();
    assert_eq!(init["id"], 1);
    assert_eq!(init["result"]["adapterName"], "acp-bridge");
    // This dedicated test binary has one child. Initialize proves it has started and answered.
    let children = process_children(std::process::id()).unwrap();
    assert_eq!(children.len(), 1);
    let child = process_identity(children[0]).unwrap();
    assert!(process_still_running(&child));

    if let Some(method) = method {
        let mut request = json!({"jsonrpc":"2.0","method":method});
        if let Some(id) = &stop_id {
            request["id"] = id.clone();
        }
        send(&input, request);
    } else {
        drop(input);
    }

    let alive_at_reply = stop_id.map(|id| {
        let reply = output.recv_timeout(GUARD).unwrap();
        let alive = process_still_running(&child);
        release.send(()).unwrap();
        assert_eq!(reply["id"], id);
        assert_eq!(reply["result"], json!({}));
        alive
    });
    bridge.join().unwrap().unwrap();
    assert!(
        !process_still_running(&child),
        "child must be gone when the bridge returns"
    );
    assert!(matches!(
        output.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Disconnected)
    ));
    alive_at_reply
}

#[test]
fn acp_stop_reply_is_sent_only_after_the_child_is_reaped() {
    if process_identity(std::process::id())
        .is_err_and(|err| err.kind() == io::ErrorKind::Unsupported)
    {
        return;
    }
    std::env::set_var("AGORA_TEST_ACP_SHUTDOWN_ORDER", "1");
    for method in ["session/stop", "shutdown"] {
        assert_eq!(
            stop_order(Some(method), Some(json!(2))),
            Some(false),
            "{method}"
        );
    }
    assert_eq!(stop_order(None, None), None);
}

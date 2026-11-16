//! Copy into crates/agora-hub/tests/review_574_chat_bytes.rs in a throwaway worktree.
#![allow(clippy::unwrap_used)]

use agora_hub::router::Router;
use agora_protocol::v1::{ChatEntry, ChatSnapshot};
use prost::Message;

#[test]
fn chat_snapshots_must_not_increase_terminal_bytes() {
    let router = Router::new();
    let (_node, _commands) = router.register("review-node");
    let (client, mut frames) = router.client();
    // Production server.rs passes None for chat.subscribe, Some(id) for terminal.subscribe.
    let chat = router.open_relay("review-node", &client, 7, None);
    let snapshot = ChatSnapshot {
        entries: vec![ChatEntry {
            entry_id: 1,
            text: "x".repeat(4096),
            ..Default::default()
        }],
    }
    .encode_to_vec();
    let chat_len = snapshot.len();
    router.relay_frame("review-node", chat, 10, snapshot);
    let got = frames.try_recv().unwrap();
    assert_eq!((got.kind, got.payload.len()), (10, chat_len));

    let terminal = router.open_relay("review-node", &client, 9, Some("terminal"));
    router.relay_frame("review-node", terminal, 0, b"abcd".to_vec());
    let got = frames.try_recv().unwrap();
    assert_eq!((got.kind, got.payload.as_slice()), (0, b"abcd".as_slice()));
    println!(
        "REVIEW_574 chat_payload_bytes={chat_len} terminal_payload_bytes=4 observed_terminal_bytes={}",
        router.terminal_bytes()
    );
    assert_eq!(
        router.terminal_bytes(),
        4,
        "chat transcript bytes counted as terminal output"
    );
}

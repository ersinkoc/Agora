// Include in the real server integration test to reuse fake auth/socket helpers.
#[tokio::test]
async fn review629_legacy_replay_is_as_safe_as_the_rebuilt_snapshot() {
    let store = SqliteStore::open_in_memory().unwrap();
    let seed = |producer: &str, seq, body| {
        let bytes = event(producer, seq, body);
        store.append(producer, seq, seq, &bytes).unwrap();
    };
    seed("n1", 1, online("n1"));
    seed("n1", 2, agent("n1", "a"));
    seed("n2", 1, online("n2"));
    let (prefix, checkpoint) = agora_hub::projection::View::rebuild(&store).unwrap();
    assert_eq!(checkpoint, 3);
    let prefix_snapshot = server::snapshot_of(&prefix, checkpoint);
    // These bytes were accepted by the pre-fix node session and persisted in its log.
    // Seed through storage, since the new session rightly refuses them.
    seed(
        "n2",
        2,
        Body::AgentForgotten(v1::AgentForgotten {
            agent_id: "n1/a".into(),
        }),
    );
    seed(
        "n2",
        3,
        Body::NodeResources(v1::NodeResources {
            node_id: "n1".into(),
            agents_running: 99,
            ..Default::default()
        }),
    );
    seed(
        "n2",
        4,
        Body::CheckoutDiscovered(v1::CheckoutDiscovered {
            checkout_id: "forged".into(),
            node_id: "n1".into(),
            project_id: "p".into(),
            path: "/fixture".into(),
        }),
    );
    let hub = Arc::new(Hub::open_at(store, 100).unwrap());
    let (view, end) = hub.view();
    assert!(
        !view.agents["n1/a"].forgotten,
        "snapshot projection ignores forged forget"
    );
    assert_eq!(view.nodes["n1"].agents_running, 0);
    assert_eq!(view.checkouts["n2/forged"].0, "n2");
    let st = Arc::new(ServerState {
        hub,
        support: support(),
        ca: None,
        lockout: Arc::new(Lockout::new()),
        auth: Box::new(Tokens),
        assets: None,
        sessions: None,
    });
    let addr = start(Arc::clone(&st)).await;
    let mut fresh = dial(addr, Some("good-token")).await;
    fresh
        .send(&f(
            STREAM_EVENTS,
            kinds::SUBSCRIBE,
            v1::Subscribe::default().encode_to_vec(),
        ))
        .await
        .unwrap();
    let fr = recv(&mut fresh).await;
    assert_eq!(fr.kind, kinds::SNAPSHOT);
    let snap = v1::Snapshot::decode(fr.payload.as_slice()).unwrap();
    assert!(snap
        .agents
        .iter()
        .any(|a| a.agent_id == "n1/a" && a.running));
    let output = std::env::var_os("REVIEW_629_FRAMES").map(std::path::PathBuf::from);
    if let Some(path) = &output {
        std::fs::create_dir_all(path).unwrap();
        std::fs::write(path.join("prefix.bin"), prefix_snapshot.encode_to_vec()).unwrap();
        std::fs::write(path.join("snapshot.bin"), &fr.payload).unwrap();
    }
    let mut replay = dial(addr, Some("good-token")).await;
    replay
        .send(&f(
            STREAM_EVENTS,
            kinds::SUBSCRIBE,
            v1::Subscribe {
                after_pos: Some(checkpoint),
            }
            .encode_to_vec(),
        ))
        .await
        .unwrap();
    let mut unsafe_events = Vec::new();
    let mut i = 0;
    loop {
        let fr = recv(&mut replay).await;
        assert_eq!(
            fr.kind,
            kinds::POSITIONED_EVENT,
            "current head replays instead of a safe snapshot"
        );
        let positioned = v1::PositionedEvent::decode(fr.payload.as_slice()).unwrap();
        if let Some(path) = &output {
            std::fs::write(path.join(format!("frame-{i}.bin")), &fr.payload).unwrap();
        }
        i += 1;
        let ev = positioned.event.unwrap();
        if ev.producer_id == "n2" {
            unsafe_events.push(format!("pos {}: {:?}", positioned.pos, ev.body));
        }
        if positioned.pos == end {
            break;
        }
    }
    if let Some(path) = &output {
        std::fs::write(path.join("count.txt"), i.to_string()).unwrap();
    }
    println!(
        "Fresh snapshot: n1/a running and visible, n1 count 0, forged checkout belongs to n2."
    );
    println!(
        "Reconnect after pos3 received {} unsafe stored events: {unsafe_events:?}",
        unsafe_events.len()
    );
    assert!(
        unsafe_events.is_empty(),
        "old forged bodies must not reach a reconnecting client unchanged"
    );
}
